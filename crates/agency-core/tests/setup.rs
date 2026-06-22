use agency_core::setup::{repo_readiness, RepoReadiness, init_repo, initial_commit, write_default_gitignore};
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
fn write_default_gitignore_does_not_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".gitignore"), "custom\n").unwrap();
    write_default_gitignore(dir.path()).unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join(".gitignore")).unwrap(), "custom\n");
}
