//! The tracker briefing as it actually lands: in a real worktree, against real
//! git. The splice rules themselves are unit-tested in the module.

use agency_core::briefing;
use agency_core::worktree::WorktreeManager;
use std::process::Command;
use tempfile::tempdir;

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git").args(args).current_dir(dir).status().unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q"]);
    git(p, &["config", "user.email", "t@t.t"]);
    git(p, &["config", "user.name", "t"]);
    std::fs::write(p.join("README.md"), "hi").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "init"]);
    dir
}

/// `git status --porcelain` in a directory, as lines.
fn status(dir: &std::path::Path) -> String {
    let out =
        Command::new("git").args(["status", "--porcelain"]).current_dir(dir).output().unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn briefing_lands_in_the_worktree_and_git_never_offers_it() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-1", "HEAD").unwrap();

    assert!(briefing::emit_agents_md(&wt.path, repo.path(), "AGE").unwrap());
    let text = std::fs::read_to_string(wt.path.join("AGENTS.md")).unwrap();
    // The two things the briefing exists to say: the tracker has a name, and
    // it is at an absolute path outside this worktree.
    assert!(text.contains("tracked in Agency"), "{text}");
    let issues = repo.path().join(".agency/issues");
    assert!(text.contains(&issues.display().to_string()), "{text}");

    // The point of the exclude: an agent's reflexive `git add -A` must not
    // sweep the briefing onto the branch, where merging would land it in the
    // project. Nothing generated may show up as work.
    assert_eq!(status(&wt.path), "", "briefing is visible to git in the worktree");
    git(&wt.path, &["add", "-A"]);
    assert_eq!(status(&wt.path), "", "briefing was staged by `git add -A`");

    // Re-emitting on a later run refreshes in place, never duplicates.
    assert!(briefing::emit_agents_md(&wt.path, repo.path(), "AGE").unwrap());
    let again = std::fs::read_to_string(wt.path.join("AGENTS.md")).unwrap();
    assert_eq!(again, text);
    assert_eq!(again.matches(briefing::SECTION_START).count(), 1, "{again}");
}

#[test]
fn a_repos_own_agents_file_is_left_alone() {
    let repo = init_repo();
    let theirs = "# House rules\n\nRun the tests before you commit.\n";
    std::fs::write(repo.path().join("AGENTS.md"), theirs).unwrap();
    git(repo.path(), &["add", "AGENTS.md"]);
    git(repo.path(), &["commit", "-q", "-m", "agents"]);

    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-2", "HEAD").unwrap();

    // Tracked: appending would show in the run's diff and ride into the
    // project's main branch at merge. Skipped, and the file is untouched.
    assert!(!briefing::emit_agents_md(&wt.path, repo.path(), "AGE").unwrap());
    assert_eq!(std::fs::read_to_string(wt.path.join("AGENTS.md")).unwrap(), theirs);
    assert_eq!(status(&wt.path), "");
}

#[test]
fn an_untracked_agents_file_keeps_its_content() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-3", "HEAD").unwrap();
    // Untracked but present — something else in the toolchain wrote it.
    std::fs::write(wt.path.join("AGENTS.md"), "# Notes\n\nkeep me\n").unwrap();

    assert!(briefing::emit_agents_md(&wt.path, repo.path(), "AGE").unwrap());
    let text = std::fs::read_to_string(wt.path.join("AGENTS.md")).unwrap();
    assert!(text.starts_with("# Notes\n\nkeep me\n"), "{text}");
    assert!(text.contains(briefing::SECTION_START), "{text}");
}
