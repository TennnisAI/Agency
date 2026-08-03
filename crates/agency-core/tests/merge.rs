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

    merge::abort_merge(dir.path(), None).unwrap();
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
    // Empty string is treated as "no target" → falls back to detect_base.
    assert_eq!(
        agency_core::merge::resolve_target(Some(""), repo).unwrap(),
        "main"
    );
    // Whitespace-only likewise.
    assert_eq!(
        agency_core::merge::resolve_target(Some("  "), repo).unwrap(),
        "main"
    );
}

#[test]
fn commits_behind_counts_base_advance() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    run(dir.path(), &["checkout", "-q", "-b", "agent/b"]);
    // Base moves ahead by two commits after the branch was cut.
    run(dir.path(), &["checkout", "-q", "main"]);
    std::fs::write(dir.path().join("m1.txt"), "1\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-q", "-m", "m1"]);
    std::fs::write(dir.path().join("m2.txt"), "2\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-q", "-m", "m2"]);
    assert_eq!(merge::commits_behind(dir.path(), "agent/b", "main").unwrap(), 2);
    assert_eq!(merge::commits_ahead(dir.path(), "agent/b", "main").unwrap(), 0);
}

#[test]
fn clean_merge_restores_original_checkout() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    run(dir.path(), &["checkout", "-q", "-b", "agent/r"]);
    std::fs::write(dir.path().join("new.txt"), "hi\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-q", "-m", "add new"]);
    // The user is parked on an unrelated branch when the merge runs.
    run(dir.path(), &["checkout", "-q", "-b", "dev", "main"]);

    let outcome = merge::merge(dir.path(), "agent/r", "main").unwrap();
    assert!(matches!(outcome, MergeOutcome::Clean { .. }));

    // Merge landed on main…
    let head_of_main = Command::new("git")
        .args(["rev-list", "--count", "dev..main"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_ne!(String::from_utf8_lossy(&head_of_main.stdout).trim(), "0");
    // …and the checkout is back where the user left it.
    let head = Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "dev");
}

/// The whole point of `merge_state`/`finish_merge`: the three states a
/// resolver can leave behind are told apart, and the merge is completed
/// without re-running it.
#[test]
fn resolved_conflicts_are_finished_not_re_merged() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    run(dir.path(), &["checkout", "-q", "-b", "agent/f"]);
    std::fs::write(dir.path().join("f.txt"), "branch-change\n").unwrap();
    run(dir.path(), &["commit", "-qam", "branch edit"]);
    run(dir.path(), &["checkout", "-q", "main"]);
    std::fs::write(dir.path().join("f.txt"), "main-change\n").unwrap();
    run(dir.path(), &["commit", "-qam", "main edit"]);
    // The user was parked elsewhere when they hit merge.
    run(dir.path(), &["checkout", "-q", "-b", "dev"]);

    assert!(matches!(
        merge::merge(dir.path(), "agent/f", "main").unwrap(),
        MergeOutcome::Conflicts { .. }
    ));

    // Mid-merge: unresolved, and nothing has landed.
    let s = merge::merge_state(dir.path(), "agent/f", "main").unwrap();
    assert!(s.merging && !s.merged);
    assert_eq!(s.unresolved, vec!["f.txt".to_string()]);
    // Finishing now refuses rather than committing conflict markers.
    let err = merge::finish_merge(dir.path(), None).unwrap_err();
    assert!(err.to_string().contains("still conflicted"), "got: {err}");

    // A resolver edits the file but forgets to `git add`: still unmerged.
    std::fs::write(dir.path().join("f.txt"), "reconciled\n").unwrap();
    assert_eq!(
        merge::merge_state(dir.path(), "agent/f", "main").unwrap().unresolved,
        vec!["f.txt".to_string()]
    );

    run(dir.path(), &["add", "f.txt"]);
    let s = merge::merge_state(dir.path(), "agent/f", "main").unwrap();
    assert!(s.merging && s.unresolved.is_empty() && !s.merged);

    let commit = merge::finish_merge(dir.path(), Some("dev")).unwrap();
    assert!(!commit.is_empty());
    assert!(!merge::is_merging(dir.path()).unwrap());
    // The merge landed, and the checkout is back where the user left it.
    let s = merge::merge_state(dir.path(), "agent/f", "main").unwrap();
    assert!(s.merged && !s.merging);
    let head = Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "dev");
    assert_eq!(std::fs::read_to_string(dir.path().join("f.txt")).unwrap(), "main-change\n");
}

/// A resolver that commits the merge itself leaves nothing to commit, and
/// finishing is still the right (idempotent) call.
#[test]
fn finish_accepts_a_merge_the_resolver_already_committed() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    run(dir.path(), &["checkout", "-q", "-b", "agent/g"]);
    std::fs::write(dir.path().join("f.txt"), "branch-change\n").unwrap();
    run(dir.path(), &["commit", "-qam", "branch edit"]);
    run(dir.path(), &["checkout", "-q", "main"]);
    std::fs::write(dir.path().join("f.txt"), "main-change\n").unwrap();
    run(dir.path(), &["commit", "-qam", "main edit"]);

    assert!(matches!(
        merge::merge(dir.path(), "agent/g", "main").unwrap(),
        MergeOutcome::Conflicts { .. }
    ));
    // What the resolver agent is told to do, done in full.
    std::fs::write(dir.path().join("f.txt"), "reconciled\n").unwrap();
    run(dir.path(), &["add", "f.txt"]);
    run(dir.path(), &["commit", "--no-edit", "-q"]);

    let s = merge::merge_state(dir.path(), "agent/g", "main").unwrap();
    assert!(!s.merging && s.merged && s.unresolved.is_empty());
    let commit = merge::finish_merge(dir.path(), None).unwrap();
    assert!(!commit.is_empty());
    assert!(merge::merge_state(dir.path(), "agent/g", "main").unwrap().merged);
}

/// An unfinished merge is a dirty checkout, so the dirty-tree guard used to
/// fire on it and send the user off to stash a half-done merge.
#[test]
fn merge_reports_an_in_progress_merge_as_such() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    run(dir.path(), &["checkout", "-q", "-b", "agent/h"]);
    std::fs::write(dir.path().join("f.txt"), "branch-change\n").unwrap();
    run(dir.path(), &["commit", "-qam", "branch edit"]);
    run(dir.path(), &["checkout", "-q", "main"]);
    std::fs::write(dir.path().join("f.txt"), "main-change\n").unwrap();
    run(dir.path(), &["commit", "-qam", "main edit"]);
    merge::merge(dir.path(), "agent/h", "main").unwrap();

    let err = merge::merge(dir.path(), "agent/h", "main").unwrap_err();
    assert!(err.to_string().contains("mid-merge of agent/h"), "got: {err}");
    merge::abort_merge(dir.path(), None).unwrap();
}

/// Every run merges in the one project checkout, so an unfinished merge is
/// visible from all of them. Only the run whose branch is actually being merged
/// may see it as its own — otherwise the next run to open Approve inherits the
/// conflict, and finishing it there commits the wrong merge under the wrong
/// run's name.
#[test]
fn an_unfinished_merge_belongs_to_one_branch_not_every_branch() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    // Two agents, each with a commit that collides with main's.
    run(dir.path(), &["checkout", "-q", "-b", "agent/one"]);
    std::fs::write(dir.path().join("f.txt"), "one\n").unwrap();
    run(dir.path(), &["commit", "-qam", "one"]);
    run(dir.path(), &["checkout", "-q", "main"]);
    run(dir.path(), &["checkout", "-q", "-b", "agent/two"]);
    std::fs::write(dir.path().join("g.txt"), "two\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-qm", "two"]);
    run(dir.path(), &["checkout", "-q", "main"]);
    std::fs::write(dir.path().join("f.txt"), "main\n").unwrap();
    run(dir.path(), &["commit", "-qam", "main edit"]);

    // Agent one conflicts and the user walks away without aborting.
    assert!(matches!(
        merge::merge(dir.path(), "agent/one", "main").unwrap(),
        MergeOutcome::Conflicts { .. }
    ));

    let one = merge::merge_state(dir.path(), "agent/one", "main").unwrap();
    assert!(one.merging, "the run being merged owns the conflict");
    assert_eq!(one.unresolved, vec!["f.txt".to_string()]);
    assert_eq!(one.blocked_by, None);

    // Approving agent two must not show it agent one's conflict…
    let two = merge::merge_state(dir.path(), "agent/two", "main").unwrap();
    assert!(!two.merging && !two.merged);
    assert!(two.unresolved.is_empty());
    // …but must say why it can't merge yet, naming the run that's holding it up.
    assert_eq!(two.blocked_by.as_deref(), Some("agent/one"));

    assert!(merge::owns_merge(dir.path(), "agent/one"));
    assert!(!merge::owns_merge(dir.path(), "agent/two"));

    // And its own merge is refused, rather than joining the one in flight.
    let err = merge::merge(dir.path(), "agent/two", "main").unwrap_err();
    assert!(err.to_string().contains("mid-merge of agent/one"), "got: {err}");

    merge::abort_merge(dir.path(), None).unwrap();
    // With the checkout free again, agent two is unblocked.
    let two = merge::merge_state(dir.path(), "agent/two", "main").unwrap();
    assert_eq!(two.blocked_by, None);
}

#[test]
fn conflicting_merge_stays_on_base_for_resolution() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    run(dir.path(), &["checkout", "-q", "-b", "agent/c"]);
    std::fs::write(dir.path().join("f.txt"), "branch-change\n").unwrap();
    run(dir.path(), &["commit", "-qam", "branch edit"]);
    run(dir.path(), &["checkout", "-q", "main"]);
    std::fs::write(dir.path().join("f.txt"), "main-change\n").unwrap();
    run(dir.path(), &["commit", "-qam", "main edit"]);
    run(dir.path(), &["checkout", "-q", "-b", "dev"]);

    let outcome = merge::merge(dir.path(), "agent/c", "main").unwrap();
    assert!(matches!(outcome, MergeOutcome::Conflicts { .. }));
    // Conflicts keep the checkout on base — that's where resolution happens.
    let head = Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "main");
    merge::abort_merge(dir.path(), None).unwrap();
}
