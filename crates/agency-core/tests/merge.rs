use agency_core::merge::{self, MergeOutcome};
use std::path::Path;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) {
    assert!(
        Command::new("git").args(args).current_dir(dir).status().unwrap().success(),
        "git {:?}",
        args
    );
}

/// Repo on `main` with one commit and a file `f.txt`.
fn init_repo(dir: &Path) {
    run(dir, &["init", "-q", "-b", "main"]);
    run(dir, &["config", "user.email", "t@e.com"]);
    run(dir, &["config", "user.name", "T"]);
    std::fs::write(dir.join("f.txt"), "line1\n").unwrap();
    run(dir, &["add", "-A"]);
    run(dir, &["commit", "-q", "-m", "init"]);
}

#[test]
fn clean_merge_commits_branch() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    // Branch adds a new file; no conflict.
    run(dir.path(), &["checkout", "-q", "-b", "agent/x"]);
    std::fs::write(dir.path().join("new.txt"), "hi\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-q", "-m", "add new"]);
    run(dir.path(), &["checkout", "-q", "main"]);

    let outcome = merge::merge(dir.path(), "agent/x", "main").unwrap();
    match outcome {
        MergeOutcome::Clean { commit } => assert!(!commit.is_empty()),
        other => panic!("expected clean, got {other:?}"),
    }
    assert!(dir.path().join("new.txt").exists());
    assert!(!merge::is_merging(dir.path()).unwrap());
}

#[test]
fn conflicting_merge_detected_and_abortable() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    // Branch changes f.txt line1.
    run(dir.path(), &["checkout", "-q", "-b", "agent/y"]);
    std::fs::write(dir.path().join("f.txt"), "branch-change\n").unwrap();
    run(dir.path(), &["commit", "-qam", "branch edit"]);
    // main changes the same line differently.
    run(dir.path(), &["checkout", "-q", "main"]);
    std::fs::write(dir.path().join("f.txt"), "main-change\n").unwrap();
    run(dir.path(), &["commit", "-qam", "main edit"]);

    let outcome = merge::merge(dir.path(), "agent/y", "main").unwrap();
    match outcome {
        MergeOutcome::Conflicts { files } => assert_eq!(files, vec!["f.txt".to_string()]),
        other => panic!("expected conflicts, got {other:?}"),
    }
    assert!(merge::is_merging(dir.path()).unwrap());

    merge::abort_merge(dir.path()).unwrap();
    assert!(!merge::is_merging(dir.path()).unwrap());
    // main's content restored.
    assert_eq!(std::fs::read_to_string(dir.path().join("f.txt")).unwrap(), "main-change\n");
}

#[test]
fn commits_ahead_counts_new_branch_work() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    // A fresh branch off main carries no new commits.
    run(dir.path(), &["checkout", "-q", "-b", "agent/a"]);
    assert_eq!(merge::commits_ahead(dir.path(), "agent/a", "main").unwrap(), 0);
    // Two commits on the branch put it two ahead of main.
    std::fs::write(dir.path().join("one.txt"), "1\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-q", "-m", "one"]);
    std::fs::write(dir.path().join("two.txt"), "2\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-q", "-m", "two"]);
    assert_eq!(merge::commits_ahead(dir.path(), "agent/a", "main").unwrap(), 2);
}

#[test]
fn detect_base_finds_main() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    assert_eq!(merge::detect_base(dir.path()).unwrap(), "main");
}

#[test]
fn merge_refuses_dirty_working_tree() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    run(dir.path(), &["checkout", "-q", "-b", "agent/z"]);
    std::fs::write(dir.path().join("new.txt"), "x\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-q", "-m", "feat"]);
    run(dir.path(), &["checkout", "-q", "main"]);
    // Dirty the main working tree.
    std::fs::write(dir.path().join("f.txt"), "dirty\n").unwrap();

    let err = merge::merge(dir.path(), "agent/z", "main").unwrap_err();
    assert!(err.to_string().contains("uncommitted changes"), "got: {err}");
}

#[test]
fn resolve_target_prefers_explicit_then_falls_back() {
    use std::process::Command;
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let run = |args: &[&str]| {
        assert!(Command::new("git").args(args).current_dir(repo).status().unwrap().success());
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "t@t"]);
    run(&["config", "user.name", "t"]);
    std::fs::write(repo.join("f"), "x").unwrap();
    run(&["add", "."]);
    run(&["commit", "-qm", "init"]);

    // Explicit wins.
    assert_eq!(
        agency_core::merge::resolve_target(Some("develop"), repo).unwrap(),
        "develop"
    );
    // None falls back to detect_base → "main".
    assert_eq!(
        agency_core::merge::resolve_target(None, repo).unwrap(),
        "main"
    );
}
