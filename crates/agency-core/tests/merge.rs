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
fn detect_base_finds_main() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    assert_eq!(merge::detect_base(dir.path()).unwrap(), "main");
}
