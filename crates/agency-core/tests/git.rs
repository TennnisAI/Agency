use agency_core::git;
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
