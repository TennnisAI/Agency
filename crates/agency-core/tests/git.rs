use agency_core::git;
use agency_core::git::parse_diff;
use std::path::Path;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) {
    assert!(
        Command::new("git").args(args).current_dir(dir).status().unwrap().success(),
        "git {:?}",
        args
    );
}

fn init_repo(dir: &Path) {
    run(dir, &["init", "-q"]);
    run(dir, &["config", "user.email", "t@e.com"]);
    run(dir, &["config", "user.name", "T"]);
    std::fs::write(dir.join("tracked.txt"), "one\n").unwrap();
    run(dir, &["add", "-A"]);
    run(dir, &["commit", "-q", "-m", "initial"]);
}

#[test]
fn status_reports_modified_and_untracked() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "hi\n").unwrap();

    let changes = git::status(dir.path()).unwrap();
    let modified = changes.iter().find(|c| c.path == "tracked.txt").unwrap();
    assert_eq!(modified.worktree, "M");
    let untracked = changes.iter().find(|c| c.path == "new.txt").unwrap();
    assert_eq!(untracked.index, "?");
}

#[test]
fn diff_shows_unstaged_changes() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();

    let d = git::diff(dir.path(), "tracked.txt", false).unwrap();
    assert!(d.contains("+two"));
}

#[test]
fn stage_commit_then_log_and_push_to_local_remote() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    // Make a change, stage it, commit it.
    std::fs::write(repo.join("tracked.txt"), "one\ntwo\n").unwrap();
    git::stage(&repo, "tracked.txt").unwrap();

    let staged = git::status(&repo).unwrap();
    let entry = staged.iter().find(|c| c.path == "tracked.txt").unwrap();
    assert_eq!(entry.index, "M"); // staged modification

    git::unstage(&repo, "tracked.txt").unwrap();
    let unstaged = git::status(&repo).unwrap();
    let entry = unstaged.iter().find(|c| c.path == "tracked.txt").unwrap();
    assert_eq!(entry.index, " "); // no longer staged
    assert_eq!(entry.worktree, "M");

    git::stage(&repo, "tracked.txt").unwrap();
    git::commit(&repo, "add two").unwrap();
    let commits = git::log_graph(&repo, 10).unwrap();
    assert_eq!(commits[0].subject, "add two");

    // Set up a bare remote and push to it.
    let remote = dir.path().join("remote.git");
    assert!(std::process::Command::new("git")
        .args(["init", "--bare", "-q", remote.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    run(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);

    git::push(&repo).unwrap();

    // The remote now has our branch with the commit.
    let ls = std::process::Command::new("git")
        .args(["log", "--format=%s", "-n1", "--all"])
        .current_dir(&remote)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&ls.stdout).contains("add two"));
}

#[test]
fn diff_stat_counts_added_deleted_files() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()); // existing helper: repo on a branch with tracked.txt = "one\n"
    // Branch off and make changes: modify tracked.txt, add new file.
    run(dir.path(), &["checkout", "-q", "-b", "feat"]);
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "a\nb\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-q", "-m", "changes"]);

    let stat = git::diff_stat(dir.path(), "master").unwrap_or_else(|_| {
        git::diff_stat(dir.path(), "main").unwrap()
    });
    assert_eq!(stat.files, 2);
    assert_eq!(stat.added, 4); // +two +three (tracked) + a + b (new)
    assert_eq!(stat.deleted, 0);
}

#[test]
fn parse_diff_splits_header_and_hunks() {
    let dir = tempfile::tempdir().unwrap();
    // Write enough context lines so top and bottom additions end up in separate hunks.
    std::fs::write(
        dir.path().join("tracked.txt"),
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    )
    .unwrap();
    run(dir.path(), &["init", "-q"]);
    run(dir.path(), &["config", "user.email", "t@e.com"]);
    run(dir.path(), &["config", "user.name", "T"]);
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-q", "-m", "initial"]);
    // Two separated changes → two hunks.
    std::fs::write(
        dir.path().join("tracked.txt"),
        "ADDED-TOP\none\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\nADDED-BOTTOM\n",
    )
    .unwrap();
    let raw = git_raw_diff(dir.path(), "tracked.txt");
    let fd = parse_diff(&raw);

    // Header captured (the diff --git / --- / +++ lines).
    assert!(fd.header.contains("diff --git"));
    assert!(fd.header.contains("+++ b/tracked.txt"));
    // Two hunks, each starting with @@.
    assert_eq!(fd.hunks.len(), 2, "hunks: {:#?}", fd.hunks);
    assert!(fd.hunks[0].header.starts_with("@@"));
    // First hunk has the top addition, second the bottom.
    assert!(fd.hunks[0].lines.iter().any(|l| l == "+ADDED-TOP"));
    assert!(fd.hunks[1].lines.iter().any(|l| l == "+ADDED-BOTTOM"));
}

#[test]
fn parse_diff_empty_for_no_changes() {
    let fd = parse_diff("");
    assert_eq!(fd.header, "");
    assert!(fd.hunks.is_empty());
}

// Helper: raw unstaged diff for a path.
fn git_raw_diff(dir: &std::path::Path, path: &str) -> String {
    let out = std::process::Command::new("git")
        .args(["diff", "--", path])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn log_graph_returns_parents_and_subject() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    run(dir.path(), &["commit", "-aqm", "second commit"]);
    let items = git::log_graph(dir.path(), 10).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].subject, "second commit");
    assert_eq!(items[0].parents.len(), 1, "second has one parent");
    assert_eq!(items[0].parents[0], items[1].hash);
    assert!(items[1].parents.is_empty(), "root has no parent");
    assert_eq!(items[0].author, "T");
}

#[test]
fn log_graph_captures_refs() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let items = git::log_graph(dir.path(), 10).unwrap();
    assert!(items[0].refs.iter().any(|r| r.contains("HEAD") || r.contains("master") || r.contains("main")),
        "head commit carries a ref: {:?}", items[0].refs);
}

#[test]
fn branch_info_reports_branch_and_no_upstream() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let info = git::branch_info(dir.path()).unwrap();
    assert!(info.branch == "master" || info.branch == "main");
    assert!(info.upstream.is_none());
    // Nothing is published in a repo with no remote, so the initial commit counts
    // as ahead — see branch_info_counts_all_commits_when_there_is_no_origin.
    assert_eq!(info.ahead, 1);
    assert_eq!(info.behind, 0);
    assert!(!info.has_remote);
}

/// A brand-new repo has no `origin`, so there is no base to measure the default
/// branch against. Counting from nothing left `ahead` at 0, which hid the one
/// button that case needs: "Add Remote & Publish…".
#[test]
fn branch_info_counts_all_commits_when_there_is_no_origin() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    commit_file(dir.path(), "second.txt", "b", "second");

    let info = git::branch_info(dir.path()).unwrap();
    assert!(!info.has_remote);
    assert!(info.upstream.is_none());
    assert_eq!(info.ahead, 2, "every commit is unpublished when there is no remote");

    // A branch off the local default still measures from its own fork point,
    // so it reports its own work rather than the whole history.
    run(dir.path(), &["checkout", "-q", "-b", "feature"]);
    commit_file(dir.path(), "third.txt", "c", "branch work");
    assert_eq!(git::branch_info(dir.path()).unwrap().ahead, 1);
}

/// What "Add Remote & Publish…" actually does: set origin, then push. Afterwards
/// the branch has an upstream and nothing left to publish.
#[test]
fn publishing_a_repo_with_no_origin_sets_upstream_and_clears_ahead() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let remote_parent = tempfile::tempdir().unwrap();
    let remote = remote_parent.path().join("remote.git");
    run(remote_parent.path(), &["init", "-q", "--bare", remote.to_str().unwrap()]);

    git::set_origin(dir.path(), remote.to_str().unwrap()).unwrap();
    git::push(dir.path()).unwrap();

    let info = git::branch_info(dir.path()).unwrap();
    assert!(info.has_remote);
    assert_eq!(info.upstream.as_deref(), Some(format!("origin/{}", info.branch).as_str()));
    assert_eq!(info.ahead, 0, "everything is published now");
}

/// A repo with no commits at all: HEAD is unborn, so `rev-parse HEAD` and `log`
/// both fail. The panel still needs a branch name and an empty history instead
/// of a raw "fatal: ambiguous argument 'HEAD'" banner.
#[test]
fn branch_info_and_log_survive_a_repo_with_no_commits() {
    let dir = tempfile::tempdir().unwrap();
    run(dir.path(), &["init", "-q"]);

    let info = git::branch_info(dir.path()).unwrap();
    assert!(info.branch == "master" || info.branch == "main", "branch: {}", info.branch);
    assert!(info.upstream.is_none());
    assert_eq!(info.ahead, 0, "no commits means nothing to publish");
    assert_eq!(info.behind, 0);
    assert!(info.base.is_none());

    assert!(git::log_graph(dir.path(), 10).unwrap().is_empty());
}

use agency_core::git::{stage_hunk, unstage_hunk};

fn staged_diff(dir: &std::path::Path, path: &str) -> String {
    let out = std::process::Command::new("git")
        .args(["diff", "--cached", "--", path])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Set up a repo whose initial commit has 10 lines so that adding one line at
/// the top and one at the bottom produces two separate hunks.
fn init_repo_10(dir: &Path) {
    run(dir, &["init", "-q"]);
    run(dir, &["config", "user.email", "t@e.com"]);
    run(dir, &["config", "user.name", "T"]);
    std::fs::write(
        dir.join("tracked.txt"),
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
    )
    .unwrap();
    run(dir, &["add", "-A"]);
    run(dir, &["commit", "-q", "-m", "initial"]);
}

#[test]
fn stage_hunk_stages_only_that_hunk() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_10(dir.path());
    std::fs::write(
        dir.path().join("tracked.txt"),
        "ADDED-TOP\none\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\nADDED-BOTTOM\n",
    )
    .unwrap();

    // Stage the first hunk only.
    stage_hunk(dir.path(), "tracked.txt", 0).unwrap();

    let staged = staged_diff(dir.path(), "tracked.txt");
    assert!(staged.contains("+ADDED-TOP"), "staged: {staged}");
    assert!(!staged.contains("+ADDED-BOTTOM"), "staged: {staged}");

    // The bottom change is still unstaged.
    let out = std::process::Command::new("git")
        .args(["diff", "--", "tracked.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let unstaged = String::from_utf8_lossy(&out.stdout);
    assert!(unstaged.contains("+ADDED-BOTTOM"), "unstaged: {unstaged}");

    // Unstage it back.
    unstage_hunk(dir.path(), "tracked.txt", 0).unwrap();
    let staged2 = staged_diff(dir.path(), "tracked.txt");
    assert!(!staged2.contains("+ADDED-TOP"), "staged2: {staged2}");
}

#[test]
fn stage_all_stages_everything() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "hi\n").unwrap();
    git::stage_all(dir.path()).unwrap();
    let changes = git::status(dir.path()).unwrap();
    assert!(changes.iter().all(|c| c.index != " " && c.index != "?"),
        "all changes staged: {changes:?}");
}

#[test]
fn unstage_all_clears_index() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    git::stage_all(dir.path()).unwrap();
    git::unstage_all(dir.path()).unwrap();
    let changes = git::status(dir.path()).unwrap();
    let m = changes.iter().find(|c| c.path == "tracked.txt").unwrap();
    assert_eq!(m.worktree, "M");
    assert_eq!(m.index, " ");
}

#[test]
fn discard_reverts_tracked_and_deletes_untracked() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\nchanged\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "hi\n").unwrap();
    git::discard(dir.path(), "tracked.txt", false).unwrap();
    git::discard(dir.path(), "new.txt", true).unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join("tracked.txt")).unwrap(), "one\n");
    assert!(!dir.path().join("new.txt").exists());
}

#[test]
fn commit_files_lists_changed_paths() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.path().join("added.txt"), "x\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-qm", "c2"]);
    let head = git::log_graph(dir.path(), 1).unwrap()[0].hash.clone();
    let files = git::commit_files(dir.path(), &head).unwrap();
    assert!(files.iter().any(|f| f.path == "tracked.txt" && f.status == "M"));
    assert!(files.iter().any(|f| f.path == "added.txt" && f.status == "A"));
}

#[test]
fn commit_diff_shows_file_diff() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    run(dir.path(), &["commit", "-aqm", "c2"]);
    let head = git::log_graph(dir.path(), 1).unwrap()[0].hash.clone();
    let diff = git::commit_diff(dir.path(), &head, "tracked.txt").unwrap();
    assert!(diff.contains("+two"), "diff shows added line: {diff}");
}

// Bytes with a NUL so git calls the file binary and diffs it as "Binary files
// … differ" — the case the text diff viewer has nothing to show for.
fn binary_bytes(tag: u8) -> Vec<u8> {
    vec![0x89, b'P', b'N', b'G', 0x00, 0x1a, 0x0a, tag]
}

#[test]
fn blob_sides_reads_both_sides_of_an_unstaged_binary_change() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("i.png"), binary_bytes(1)).unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-qm", "add image"]);
    std::fs::write(dir.path().join("i.png"), binary_bytes(2)).unwrap();

    // The text diff has no hunks at all — that's what this replaces.
    let fd = parse_diff(&git::diff(dir.path(), "i.png", false).unwrap());
    assert!(fd.hunks.is_empty(), "binary diff carries no hunks");

    let (old, new) = git::blob_sides(dir.path(), "i.png", &git::BlobMode::Unstaged).unwrap();
    assert_eq!(old.unwrap().bytes, binary_bytes(1));
    assert_eq!(new.unwrap().bytes, binary_bytes(2));
}

#[test]
fn blob_sides_staged_compares_head_with_the_index() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("i.png"), binary_bytes(1)).unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-qm", "add image"]);
    std::fs::write(dir.path().join("i.png"), binary_bytes(2)).unwrap();
    run(dir.path(), &["add", "i.png"]);
    // Staged holds tag 2; the working tree has moved on to tag 3.
    std::fs::write(dir.path().join("i.png"), binary_bytes(3)).unwrap();

    let (old, new) = git::blob_sides(dir.path(), "i.png", &git::BlobMode::Staged).unwrap();
    assert_eq!(old.unwrap().bytes, binary_bytes(1));
    assert_eq!(new.unwrap().bytes, binary_bytes(2));
}

#[test]
fn blob_sides_has_no_old_side_for_an_untracked_file() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("new.png"), binary_bytes(7)).unwrap();

    let (old, new) = git::blob_sides(dir.path(), "new.png", &git::BlobMode::Unstaged).unwrap();
    assert!(old.is_none(), "an untracked file has no index side");
    let new = new.unwrap();
    assert_eq!(new.bytes, binary_bytes(7));
    assert_eq!(new.size, 8);
}

#[test]
fn blob_sides_has_no_new_side_for_a_deleted_file() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("i.png"), binary_bytes(1)).unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-qm", "add image"]);
    std::fs::remove_file(dir.path().join("i.png")).unwrap();

    let (old, new) = git::blob_sides(dir.path(), "i.png", &git::BlobMode::Unstaged).unwrap();
    assert!(old.is_some());
    assert!(new.is_none(), "the file is gone from the working tree");
}

#[test]
fn blob_sides_for_a_commit_compares_against_its_parent() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("i.png"), binary_bytes(1)).unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-qm", "add image"]);
    std::fs::write(dir.path().join("i.png"), binary_bytes(2)).unwrap();
    run(dir.path(), &["commit", "-aqm", "change image"]);

    let log = git::log_graph(dir.path(), 3).unwrap();
    let head = log[0].hash.clone();
    let (old, new) = git::blob_sides(dir.path(), "i.png", &git::BlobMode::Commit(head)).unwrap();
    assert_eq!(old.unwrap().bytes, binary_bytes(1));
    assert_eq!(new.unwrap().bytes, binary_bytes(2));

    // The commit that introduced the file: nothing on the old side. The root
    // commit exercises the same path with no parent to resolve at all.
    let added = log[1].hash.clone();
    let (old, new) = git::blob_sides(dir.path(), "i.png", &git::BlobMode::Commit(added)).unwrap();
    assert!(old.is_none(), "an added file has no parent-side blob");
    assert_eq!(new.unwrap().bytes, binary_bytes(1));
    let root = log[2].hash.clone();
    let (old, _) = git::blob_sides(dir.path(), "tracked.txt", &git::BlobMode::Commit(root)).unwrap();
    assert!(old.is_none(), "the root commit has no parent");
}

#[test]
fn blob_sides_reads_a_file_in_a_subdirectory() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::create_dir_all(dir.path().join("assets/img")).unwrap();
    std::fs::write(dir.path().join("assets/img/i.png"), binary_bytes(1)).unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-qm", "add image"]);
    std::fs::write(dir.path().join("assets/img/i.png"), binary_bytes(2)).unwrap();

    let (old, new) =
        git::blob_sides(dir.path(), "assets/img/i.png", &git::BlobMode::Unstaged).unwrap();
    assert_eq!(old.unwrap().bytes, binary_bytes(1));
    assert_eq!(new.unwrap().bytes, binary_bytes(2));
}

#[test]
fn build_partial_patch_keeps_selected_add_drops_others() {
    // Hunk adds two lines after context; select only the first added line (index 1).
    let fd = parse_diff(
        "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,3 @@\n one\n+two\n+three\n",
    );
    let patch = git::build_partial_patch(&fd, 0, &[1], false).unwrap();
    assert!(patch.contains("+two"));
    assert!(!patch.contains("+three"), "unselected add dropped: {patch}");
    assert!(patch.contains("@@ -1,1 +1,2 @@"), "recomputed header: {patch}");
}

#[test]
fn stage_lines_stages_only_selection() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\nthree\n").unwrap();
    // working diff: hunk index 0 adds "two" (idx 1) and "three" (idx 2) after "one".
    let raw = git::diff(dir.path(), "tracked.txt", false).unwrap();
    let fd = parse_diff(&raw);
    // Select the first added body line only.
    let add_idx = fd.hunks[0].lines.iter().position(|l| l.starts_with("+two")).unwrap();
    git::stage_lines(dir.path(), "tracked.txt", 0, &[add_idx]).unwrap();
    let staged = git::diff(dir.path(), "tracked.txt", true).unwrap();
    assert!(staged.contains("+two"));
    assert!(!staged.contains("+three"), "only selection staged: {staged}");
}

#[test]
fn staging_all_hunks_equals_staging_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    init_repo_10(dir.path());
    std::fs::write(
        dir.path().join("tracked.txt"),
        "ADDED-TOP\none\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\nADDED-BOTTOM\n",
    )
    .unwrap();

    // Stage hunk 0, then the (now-only-remaining) hunk 0 again.
    stage_hunk(dir.path(), "tracked.txt", 0).unwrap();
    stage_hunk(dir.path(), "tracked.txt", 0).unwrap();

    // Nothing left unstaged for the file.
    let out = std::process::Command::new("git")
        .args(["diff", "--", "tracked.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
}

#[test]
fn unstage_lines_unstages_only_selection() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\nthree\n").unwrap();
    git::stage_all(dir.path()).unwrap();
    // Cached diff adds "two" and "three"; unstage only "two".
    let fd = parse_diff(&git::diff(dir.path(), "tracked.txt", true).unwrap());
    let idx = fd.hunks[0].lines.iter().position(|l| l.starts_with("+two")).unwrap();
    git::unstage_lines(dir.path(), "tracked.txt", 0, &[idx]).unwrap();
    let staged = git::diff(dir.path(), "tracked.txt", true).unwrap();
    assert!(staged.contains("+three"), "three still staged: {staged}");
    assert!(!staged.contains("+two"), "two should be unstaged: {staged}");
}

#[test]
fn revert_lines_reverts_only_selection() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\nthree\n").unwrap();
    // Working diff adds "two" and "three"; revert only "two".
    let fd = parse_diff(&git::diff(dir.path(), "tracked.txt", false).unwrap());
    let idx = fd.hunks[0].lines.iter().position(|l| l.starts_with("+two")).unwrap();
    git::revert_lines(dir.path(), "tracked.txt", 0, &[idx]).unwrap();
    let content = std::fs::read_to_string(dir.path().join("tracked.txt")).unwrap();
    assert_eq!(content, "one\nthree\n", "only 'two' should be reverted");
}

#[test]
fn fetch_branch_updates_local_from_origin_without_checkout() {
    let upstream = tempfile::tempdir().unwrap();
    init_repo(upstream.path());
    run(upstream.path(), &["branch", "feat/remote-work"]);

    // Clone; the clone's origin is the upstream.
    let clone_parent = tempfile::tempdir().unwrap();
    let clone = clone_parent.path().join("clone");
    let status = std::process::Command::new("git")
        .args(["clone", "-q", upstream.path().to_str().unwrap(), clone.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success());

    // Advance the branch upstream after the clone.
    run(upstream.path(), &["checkout", "-q", "feat/remote-work"]);
    std::fs::write(upstream.path().join("later.txt"), "x").unwrap();
    run(upstream.path(), &["add", "-A"]);
    run(upstream.path(), &["commit", "-q", "-m", "later"]);

    agency_core::git::fetch_branch(&clone, "feat/remote-work").unwrap();
    let out = std::process::Command::new("git")
        .args(["log", "-1", "--format=%s", "feat/remote-work"])
        .current_dir(&clone)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "later");
}

/// A local clone of a bare remote, checked out on the remote's default branch
/// with upstream tracking set — the shape `sync` operates on. Returns
/// (`_keep`, remote_path, clone_path); `_keep` holds the tempdirs alive.
fn clone_with_upstream() -> (Vec<tempfile::TempDir>, std::path::PathBuf, std::path::PathBuf) {
    let src = tempfile::tempdir().unwrap();
    init_repo(src.path());

    let remote_parent = tempfile::tempdir().unwrap();
    let remote = remote_parent.path().join("remote.git");
    run(src.path(), &["clone", "--bare", "-q", ".", remote.to_str().unwrap()]);

    let clone_parent = tempfile::tempdir().unwrap();
    let clone = clone_parent.path().join("clone");
    run(clone_parent.path(), &["clone", "-q", remote.to_str().unwrap(), clone.to_str().unwrap()]);
    run(&clone, &["config", "user.email", "t@e.com"]);
    run(&clone, &["config", "user.name", "T"]);
    (vec![src, remote_parent, clone_parent], remote, clone)
}

fn commit_file(dir: &Path, name: &str, contents: &str, msg: &str) {
    std::fs::write(dir.join(name), contents).unwrap();
    run(dir, &["add", "-A"]);
    run(dir, &["commit", "-q", "-m", msg]);
}

/// Worktrees share one object store and one local default branch, so "ahead"
/// has to be measured from each branch's own fork point. Measuring against
/// origin instead folds the default branch's unpushed commits into every
/// branch and shows the same count on all of them.
#[test]
fn branch_info_ahead_excludes_the_default_branchs_unpushed_commits() {
    let (_keep, _remote, clone) = clone_with_upstream();
    // The shared checkout accumulates commits that were never pushed…
    commit_file(&clone, "a.txt", "a", "main work 1");
    commit_file(&clone, "b.txt", "b", "main work 2");

    // …and an agent worktree branches off it with one commit of its own.
    let busy = clone.parent().unwrap().join("busy");
    run(&clone, &["worktree", "add", "-q", "-b", "agent/busy", busy.to_str().unwrap()]);
    commit_file(&busy, "c.txt", "c", "branch work");
    let info = git::branch_info(&busy).unwrap();
    assert_eq!(info.branch, "agent/busy");
    assert!(info.upstream.is_none());
    assert_eq!(info.ahead, 1, "1, not 3: main's unpushed commits aren't this branch's work");

    // A sibling that has committed nothing is 0 ahead, so Publish stays hidden.
    let idle = clone.parent().unwrap().join("idle");
    run(&clone, &["worktree", "add", "-q", "-b", "agent/idle", idle.to_str().unwrap()]);
    assert_eq!(git::branch_info(&idle).unwrap().ahead, 0);

    // The shared checkout still reports its own backlog against origin.
    assert_eq!(git::branch_info(&clone).unwrap().ahead, 2);
}

#[test]
fn sync_pushes_when_only_ahead() {
    let (_keep, remote, clone) = clone_with_upstream();
    commit_file(&clone, "local.txt", "a", "local commit");

    let outcome = git::sync(&clone, |_| {}).unwrap();
    assert_eq!(outcome, git::SyncOutcome::Synced);

    // The remote received the local commit.
    let out = std::process::Command::new("git")
        .args(["log", "--format=%s", "-n1", "--all"])
        .current_dir(&remote)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("local commit"));
}

#[test]
fn sync_fast_forwards_when_only_behind() {
    let (_keep, _remote, clone) = clone_with_upstream();
    // A second clone advances the remote, so `clone` falls behind.
    let other_parent = tempfile::tempdir().unwrap();
    let other = other_parent.path().join("other");
    run(other_parent.path(), &["clone", "-q", _remote.to_str().unwrap(), other.to_str().unwrap()]);
    run(&other, &["config", "user.email", "t@e.com"]);
    run(&other, &["config", "user.name", "T"]);
    commit_file(&other, "remote.txt", "r", "remote commit");
    git::push(&other).unwrap();

    let outcome = git::sync(&clone, |_| {}).unwrap();
    assert_eq!(outcome, git::SyncOutcome::Synced);
    // The incoming commit was fast-forwarded into the local branch.
    assert!(clone.join("remote.txt").exists());
    let (ahead, behind) = git::ahead_behind(&clone).unwrap();
    assert_eq!((ahead, behind), (0, 0));
}

#[test]
fn sync_reports_diverged_without_touching_anything() {
    let (_keep, remote, clone) = clone_with_upstream();
    // Remote advances via another clone…
    let other_parent = tempfile::tempdir().unwrap();
    let other = other_parent.path().join("other");
    run(other_parent.path(), &["clone", "-q", remote.to_str().unwrap(), other.to_str().unwrap()]);
    run(&other, &["config", "user.email", "t@e.com"]);
    run(&other, &["config", "user.name", "T"]);
    commit_file(&other, "remote.txt", "r", "remote commit");
    git::push(&other).unwrap();
    // …while our clone makes its own commit, so the two have diverged.
    commit_file(&clone, "local.txt", "a", "local commit");

    let head_before = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"]).current_dir(&clone).output().unwrap();

    let outcome = git::sync(&clone, |_| {}).unwrap();
    assert_eq!(outcome, git::SyncOutcome::Diverged);

    // Nothing changed locally: no merge, no rebase, no lost work.
    let head_after = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"]).current_dir(&clone).output().unwrap();
    assert_eq!(head_before.stdout, head_after.stdout);
    assert!(clone.join("local.txt").exists());
    assert!(!clone.join("remote.txt").exists());
    // And the remote's tip was not overwritten by a force-push.
    let out = std::process::Command::new("git")
        .args(["log", "--format=%s", "-n1", "HEAD"])
        .current_dir(&remote)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "remote commit");
}
