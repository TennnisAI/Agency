use agency_core::setup::{
    clone_destination, clone_repo, clone_repo_with_progress, folder_missing, init_repo,
    initial_commit, initial_commit_with_progress, inside_work_tree, repo_name_from_url,
    repo_readiness, scan_large_files, write_default_gitignore, CancelToken, CommitOptions,
    RepoReadiness, CANCELLED, LARGE_FILE_BYTES,
};
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    assert!(
        Command::new("git").args(args).current_dir(dir).status().unwrap().success(),
        "git {:?}",
        args
    );
}

fn init_bare_repo(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "t@e.com"]);
    git(dir, &["config", "user.name", "T"]);
}

#[test]
fn plain_folder_is_not_a_repo() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::NotARepo);
}

#[test]
fn inside_work_tree_answers_yes_no_or_dont_know() {
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(
        inside_work_tree(plain.path()),
        Some(false),
        "a plain folder is definitively not one"
    );

    let repo = tempfile::tempdir().unwrap();
    init_bare_repo(repo.path());
    assert_eq!(inside_work_tree(repo.path()), Some(true));

    // git can't even be spawned against a folder that isn't there. That is not
    // "no repository here" — callers that skip a run's isolated worktree on a
    // gitless folder must not act on it, so it has to stay distinguishable.
    let gone = plain.path().join("removed");
    assert_eq!(inside_work_tree(&gone), None);
    // And `repo_readiness` says so in its own right: it used to fold this into
    // NotARepo, which every caller read as "a plain folder, so run the agent
    // in it" against a folder that was not there (AGE-203).
    assert_eq!(repo_readiness(&gone), RepoReadiness::Missing);
}

#[test]
fn a_moved_or_deleted_folder_reads_as_missing() {
    let parent = tempfile::tempdir().unwrap();
    let repo = parent.path().join("proj");
    std::fs::create_dir(&repo).unwrap();
    init_bare_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hi\n").unwrap();
    initial_commit(&repo, false).unwrap();
    assert_eq!(repo_readiness(&repo), RepoReadiness::Ready { dirty: false });
    assert!(!folder_missing(&repo));

    // Moved out from under us, exactly as a drag in Finder does.
    let moved = parent.path().join("proj-elsewhere");
    std::fs::rename(&repo, &moved).unwrap();
    assert!(folder_missing(&repo));
    assert_eq!(repo_readiness(&repo), RepoReadiness::Missing);
    // The folder at its new home is untouched, so a reconnect has something to
    // point at.
    assert_eq!(repo_readiness(&moved), RepoReadiness::Ready { dirty: false });

    // A path taken over by a file is as unusable as one that is not there.
    let file = parent.path().join("a-file");
    std::fs::write(&file, "not a folder\n").unwrap();
    assert!(folder_missing(&file));
    assert_eq!(repo_readiness(&file), RepoReadiness::Missing);
}

#[test]
fn repo_with_no_commits_and_files_is_stageable() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::NoCommits { stageable: true });
}

#[test]
fn empty_repo_with_no_commits_is_not_stageable() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::NoCommits { stageable: false });
}

#[test]
fn committed_clean_repo_is_ready_not_dirty() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "init"]);
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: false });
}

#[test]
fn committed_repo_with_changes_is_ready_dirty() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "init"]);
    std::fs::write(dir.path().join("b.txt"), "new\n").unwrap();
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: true });
}

#[test]
fn init_repo_makes_a_repo_with_no_commits() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()).unwrap();
    assert!(matches!(repo_readiness(dir.path()), RepoReadiness::NoCommits { .. }));
}

#[test]
fn initial_commit_with_files_makes_repo_ready() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()).unwrap();
    git(dir.path(), &["config", "user.email", "t@e.com"]);
    git(dir.path(), &["config", "user.name", "T"]);
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    initial_commit(dir.path(), false).unwrap();
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: false });
}

#[test]
fn initial_commit_on_empty_folder_uses_allow_empty() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()).unwrap();
    git(dir.path(), &["config", "user.email", "t@e.com"]);
    git(dir.path(), &["config", "user.name", "T"]);
    initial_commit(dir.path(), false).unwrap();
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: false });
}

#[test]
fn initial_commit_reports_staging_progress_then_commit() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()).unwrap();
    git(dir.path(), &["config", "user.email", "t@e.com"]);
    git(dir.path(), &["config", "user.name", "T"]);
    // Enough files to cross the every-100 staging update threshold twice.
    for i in 0..250 {
        std::fs::write(dir.path().join(format!("f{i}.txt")), "x\n").unwrap();
    }

    let mut seen: Vec<(String, String)> = Vec::new();
    initial_commit_with_progress(dir.path(), &CommitOptions::default(), |p| {
        seen.push((p.phase, p.detail))
    })
    .unwrap();

    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: false });
    // Staging counts up as git hashes files, then the commit phase closes it out.
    let staging: Vec<&String> =
        seen.iter().filter(|(ph, _)| ph == "Staging files").map(|(_, d)| d).collect();
    assert!(staging.contains(&&"100 files".to_string()), "got {seen:?}");
    assert!(staging.contains(&&"200 files".to_string()), "got {seen:?}");
    assert_eq!(seen.last().unwrap().0, "Writing commit");
}

#[test]
fn initial_commit_on_empty_folder_reports_no_staged_files() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()).unwrap();
    git(dir.path(), &["config", "user.email", "t@e.com"]);
    git(dir.path(), &["config", "user.name", "T"]);

    let mut seen: Vec<(String, String)> = Vec::new();
    initial_commit_with_progress(dir.path(), &CommitOptions::default(), |p| {
        seen.push((p.phase, p.detail))
    })
    .unwrap();

    // Nothing to stage, but the phases still fire so the dialog isn't left blank.
    assert_eq!(seen.last().unwrap(), &("Writing commit".to_string(), "0 files".to_string()));
}

#[test]
fn initial_commit_with_gitignore_writes_and_excludes() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()).unwrap();
    git(dir.path(), &["config", "user.email", "t@e.com"]);
    git(dir.path(), &["config", "user.name", "T"]);
    std::fs::create_dir(dir.path().join("node_modules")).unwrap();
    std::fs::write(dir.path().join("node_modules/x.js"), "x\n").unwrap();
    std::fs::write(dir.path().join("keep.txt"), "k\n").unwrap();
    initial_commit(dir.path(), true).unwrap();

    let gi = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert!(gi.contains("node_modules/"));
    // node_modules excluded → tree clean, keep.txt + .gitignore committed.
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: false });
}

#[test]
fn initial_commit_excludes_chosen_paths_even_when_already_staged() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    std::fs::create_dir(dir.path().join("models")).unwrap();
    std::fs::write(dir.path().join("models/w.gguf"), "weights\n").unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
    // A first attempt the user cancelled after it had staged the very file they
    // then chose to leave out.
    git(dir.path(), &["add", "-A"]);

    let opts = CommitOptions {
        add_gitignore: true,
        ignore_paths: vec!["models/".to_string()],
        ..Default::default()
    };
    initial_commit_with_progress(dir.path(), &opts, |_| {}).unwrap();

    let out = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let files: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
    assert!(files.contains(&"main.rs"), "{files:?}");
    assert!(!files.contains(&"models/w.gguf"), "{files:?}");
    let gi = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    // Appended to the defaults, not instead of them.
    assert!(gi.contains("/models/"), "{gi}");
    assert!(gi.contains("node_modules/"), "{gi}");
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: false });
}

#[test]
fn cancelled_commit_stops_and_leaves_no_lock() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();

    let cancel = CancelToken::new();
    // Already cancelled: staging must not outlive the token, whatever it hashed.
    cancel.cancel();
    let opts = CommitOptions { cancel, ..Default::default() };
    let err = initial_commit_with_progress(dir.path(), &opts, |_| {}).unwrap_err();

    assert_eq!(err.to_string(), CANCELLED);
    // No commit, and nothing left behind to block the next attempt.
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::NoCommits { stageable: true });
    assert!(!dir.path().join(".git/index.lock").exists());
}

#[test]
fn ignore_rules_hold_for_awkward_file_names() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    // Glob characters and a space: the rule written for this file has to match
    // it and nothing else.
    std::fs::write(dir.path().join("[raw] set*.bin"), "x\n").unwrap();
    std::fs::write(dir.path().join("keep.bin"), "k\n").unwrap();

    let opts =
        CommitOptions { ignore_paths: vec!["[raw] set*.bin".to_string()], ..Default::default() };
    initial_commit_with_progress(dir.path(), &opts, |_| {}).unwrap();

    let out = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let files: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
    assert!(files.contains(&"keep.bin"), "{files:?}");
    assert!(!files.iter().any(|f| f.contains("raw")), "{files:?}");
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::Ready { dirty: false });
}

#[test]
fn cancelling_mid_staging_stops_the_commit() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    // Enough files that the first progress update (every 100) lands with most of
    // the work still to do, which is where the user hits Cancel.
    for i in 0..3000 {
        std::fs::write(dir.path().join(format!("f{i}.txt")), "x\n").unwrap();
    }

    let cancel = CancelToken::new();
    let opts = CommitOptions { cancel: cancel.clone(), ..Default::default() };
    let err = initial_commit_with_progress(dir.path(), &opts, |_| cancel.cancel()).unwrap_err();

    assert_eq!(err.to_string(), CANCELLED);
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::NoCommits { stageable: true });
    assert!(!dir.path().join(".git/index.lock").exists());
}

#[test]
fn cancelling_after_staging_still_writes_no_commit() {
    let dir = tempfile::tempdir().unwrap();
    init_bare_repo(dir.path());
    std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();

    let cancel = CancelToken::new();
    let opts = CommitOptions { cancel: cancel.clone(), ..Default::default() };
    // The click lands once staging is done, in the window before the commit is
    // written. The dialog has already closed, so no commit may appear.
    let err = initial_commit_with_progress(dir.path(), &opts, |p| {
        if p.phase == "Writing commit" {
            cancel.cancel();
        }
    })
    .unwrap_err();

    assert_eq!(err.to_string(), CANCELLED);
    assert_eq!(repo_readiness(dir.path()), RepoReadiness::NoCommits { stageable: true });
}

#[test]
fn scan_reports_only_files_over_the_threshold() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("small.txt"), "hi\n").unwrap();
    // Nothing here is anywhere near 100 MB, so the setup dialog stays quiet.
    let scan = scan_large_files(dir.path());
    assert_eq!(scan.count, 0);
    assert!(scan.files.is_empty());
    assert!(scan.ignore_paths.is_empty());
    assert!(!scan.truncated);
    // The dialog words its warning from this, so it has to be the real threshold.
    assert_eq!(scan.threshold_bytes, LARGE_FILE_BYTES);
}

#[test]
fn write_default_gitignore_does_not_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".gitignore"), "custom\n").unwrap();
    write_default_gitignore(dir.path()).unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join(".gitignore")).unwrap(), "custom\n");
}

#[test]
fn repo_name_from_url_handles_common_forms() {
    assert_eq!(repo_name_from_url("https://github.com/owner/repo.git"), "repo");
    assert_eq!(repo_name_from_url("https://github.com/owner/repo"), "repo");
    assert_eq!(repo_name_from_url("git@github.com:owner/repo.git"), "repo");
    assert_eq!(repo_name_from_url("https://github.com/owner/repo/"), "repo");
    assert_eq!(repo_name_from_url("  https://github.com/owner/repo.git  "), "repo");
}

#[test]
fn clone_repo_clones_into_named_subfolder() {
    // A local source repo with one commit stands in for a remote.
    let src = tempfile::tempdir().unwrap();
    init_bare_repo(src.path());
    std::fs::write(src.path().join("hello.txt"), "hi\n").unwrap();
    git(src.path(), &["add", "-A"]);
    git(src.path(), &["commit", "-q", "-m", "init"]);

    let parent = tempfile::tempdir().unwrap();
    let url = format!("file://{}", src.path().display());
    let dest = clone_repo(&url, parent.path()).unwrap();

    // A file:// URL ending in the temp dir's name yields that name as the folder.
    assert_eq!(dest.parent().unwrap(), parent.path());
    assert!(dest.join("hello.txt").exists());
    assert_eq!(repo_readiness(&dest), RepoReadiness::Ready { dirty: false });
}

#[test]
fn clone_destination_names_the_folder_the_clone_will_create() {
    let parent = Path::new("/tmp/parent");
    // The frontend cancels a clone by its inputs, and the backend turns them
    // back into this path — so it has to agree with what the clone creates.
    assert_eq!(
        clone_destination("https://github.com/owner/repo.git", parent),
        Some(parent.join("repo"))
    );
    assert_eq!(clone_destination("git@github.com:owner/repo", parent), Some(parent.join("repo")));
    // No name in the URL means no clone and nothing to cancel.
    assert_eq!(clone_destination("", parent), None);
}

#[test]
fn cancelled_clone_leaves_no_half_downloaded_folder() {
    let src = tempfile::tempdir().unwrap();
    init_bare_repo(src.path());
    std::fs::write(src.path().join("a.txt"), "a\n").unwrap();
    git(src.path(), &["add", "-A"]);
    git(src.path(), &["commit", "-q", "-m", "init"]);

    let parent = tempfile::tempdir().unwrap();
    let url = format!("file://{}", src.path().display());
    let dest = clone_destination(&url, parent.path()).unwrap();

    // The click lands on the first progress line, which for a repo this small
    // can arrive after git has already written the folder — the case that most
    // needs cleaning up, since the leftover would block the retry.
    let cancel = CancelToken::new();
    let err =
        clone_repo_with_progress(&url, parent.path(), &cancel, |_| cancel.cancel()).unwrap_err();

    assert_eq!(err.to_string(), CANCELLED);
    assert!(!dest.exists(), "{} survived a cancelled clone", dest.display());
}

#[test]
fn clone_cancelled_before_it_starts_reports_cancelled() {
    let src = tempfile::tempdir().unwrap();
    init_bare_repo(src.path());
    std::fs::write(src.path().join("a.txt"), "a\n").unwrap();
    git(src.path(), &["add", "-A"]);
    git(src.path(), &["commit", "-q", "-m", "init"]);

    let parent = tempfile::tempdir().unwrap();
    let url = format!("file://{}", src.path().display());
    let cancel = CancelToken::new();
    // Already cancelled: the clone must not outlive the token, and must not
    // report the killed git as a credentials problem.
    cancel.cancel();
    let err = clone_repo_with_progress(&url, parent.path(), &cancel, |_| {}).unwrap_err();

    assert_eq!(err.to_string(), CANCELLED);
    assert!(!clone_destination(&url, parent.path()).unwrap().exists());
}

#[test]
fn clone_repo_refuses_existing_destination() {
    let src = tempfile::tempdir().unwrap();
    init_bare_repo(src.path());
    std::fs::write(src.path().join("a.txt"), "a\n").unwrap();
    git(src.path(), &["add", "-A"]);
    git(src.path(), &["commit", "-q", "-m", "init"]);

    let parent = tempfile::tempdir().unwrap();
    let name = src.path().file_name().unwrap();
    // Pre-create the folder the clone would land in.
    std::fs::create_dir(parent.path().join(name)).unwrap();
    let url = format!("file://{}", src.path().display());
    assert!(clone_repo(&url, parent.path()).is_err());
}
