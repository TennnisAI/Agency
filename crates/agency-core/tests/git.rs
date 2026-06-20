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
fn log_lists_recent_commits() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let commits = git::log(dir.path(), 10).unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].summary, "initial");
    assert!(!commits[0].hash.is_empty());
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
    let commits = git::log(&repo, 10).unwrap();
    assert_eq!(commits[0].summary, "add two");

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
