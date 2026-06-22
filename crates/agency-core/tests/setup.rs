use agency_core::setup::{repo_readiness, RepoReadiness};
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
