use agency_core::setup::{
    clone_repo, init_repo, initial_commit, initial_commit_with_progress, repo_name_from_url,
    repo_readiness, write_default_gitignore, RepoReadiness,
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
    initial_commit_with_progress(dir.path(), false, |p| seen.push((p.phase, p.detail))).unwrap();

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
    initial_commit_with_progress(dir.path(), false, |p| seen.push((p.phase, p.detail))).unwrap();

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
