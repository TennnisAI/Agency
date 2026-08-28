//! The skills kit as it actually lands: in a real worktree, against real git.
//! The generated copy and the resolver's arithmetic are unit-tested in the
//! module.

use agency_core::skills::{self, Workspace};
use agency_core::worktree::WorktreeManager;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

fn git(dir: &Path, args: &[&str]) {
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

fn status(dir: &Path) -> String {
    let out =
        Command::new("git").args(["status", "--porcelain"]).current_dir(dir).output().unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn workspace(wt: &agency_core::worktree::Worktree, repo: &Path) -> Workspace {
    Workspace {
        worktree: wt.path.clone(),
        repo_root: repo.to_path_buf(),
        issue_key: "AGE".into(),
        branch: wt.branch.clone(),
        ..Default::default()
    }
}

fn skill_md(wt: &Path, name: &str) -> std::path::PathBuf {
    wt.join(".claude").join("skills").join(name).join("SKILL.md")
}

#[test]
fn the_kit_lands_in_the_worktree_and_git_never_offers_it() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-1", "HEAD").unwrap();
    let ws = workspace(&wt, repo.path());

    assert!(skills::emit_for_agent("claude", &ws).unwrap());
    for name in [skills::DATE_SKILL, skills::WORKSPACE_SKILL] {
        assert!(skill_md(&wt.path, name).is_file(), "{name} not written");
    }
    // The resolver runs where it was written, under a plain `sh`.
    let script =
        wt.path.join(".claude/skills").join(skills::DATE_SKILL).join(skills::RESOLVER_FILE);
    let out = Command::new("sh")
        .arg(&script)
        .args(["last-month", "--today", "2026-01-15"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("start=2025-12-01"), "{text}");

    // The point of the exclude: an agent's reflexive `git add -A` must not
    // sweep the kit onto the branch, where merging would land it in the
    // project. Nothing generated may show up as work.
    assert_eq!(status(&wt.path), "", "the kit is visible to git in the worktree");
    git(&wt.path, &["add", "-A"]);
    assert_eq!(status(&wt.path), "", "the kit was staged by `git add -A`");

    // Re-emitting on a later run rewrites in place, never duplicates.
    let before = std::fs::read_to_string(skill_md(&wt.path, skills::WORKSPACE_SKILL)).unwrap();
    assert!(skills::emit_for_agent("claude", &ws).unwrap());
    assert_eq!(
        std::fs::read_to_string(skill_md(&wt.path, skills::WORKSPACE_SKILL)).unwrap(),
        before
    );
    assert_eq!(status(&wt.path), "");
}

#[test]
fn a_repos_own_skills_are_upserted_around_never_replaced() {
    let repo = init_repo();
    // The repo's own skill, tracked, in the same directory we write into.
    let theirs = repo.path().join(".claude/skills/house-style");
    std::fs::create_dir_all(&theirs).unwrap();
    std::fs::write(theirs.join("SKILL.md"), "# House style\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "skills"]);

    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-2", "HEAD").unwrap();
    assert!(skills::emit_for_agent("claude", &workspace(&wt, repo.path())).unwrap());

    assert_eq!(
        std::fs::read_to_string(skill_md(&wt.path, "house-style")).unwrap(),
        "# House style\n",
        "the repo's own skill was rewritten"
    );
    assert!(skill_md(&wt.path, skills::DATE_SKILL).is_file());
    assert_eq!(status(&wt.path), "");
}

#[test]
fn a_skill_the_repo_tracks_under_our_own_name_is_left_alone() {
    let repo = init_repo();
    // Someone committed a skill in Agency's namespace: an exclude has no say
    // over a tracked file, so rewriting it would put a diff on the branch and
    // ride into the project at merge.
    let theirs = repo.path().join(".claude/skills").join(skills::DATE_SKILL);
    std::fs::create_dir_all(&theirs).unwrap();
    std::fs::write(theirs.join("SKILL.md"), "# Ours, not Agency's\n").unwrap();
    git(repo.path(), &["add", "-A", "-f"]);
    git(repo.path(), &["commit", "-q", "-m", "skills"]);

    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-3", "HEAD").unwrap();
    // The other half of the kit still lands: one tracked directory is not a
    // reason to withhold the rest.
    assert!(skills::emit_for_agent("claude", &workspace(&wt, repo.path())).unwrap());

    assert_eq!(
        std::fs::read_to_string(skill_md(&wt.path, skills::DATE_SKILL)).unwrap(),
        "# Ours, not Agency's\n"
    );
    assert!(skill_md(&wt.path, skills::WORKSPACE_SKILL).is_file());
    assert_eq!(status(&wt.path), "");
}

#[test]
fn dsh_reads_the_vendor_neutral_root_and_git_never_offers_it() {
    // DeepSeek Harness discovers `<projectRoot>/.agents/skills` on its own, so
    // the kit lands there — and the exclude for that root has to exist too, or
    // a dsh run's reflexive `git add -A` sweeps the kit onto the branch.
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-5", "HEAD").unwrap();

    assert!(skills::emit_for_agent("dsh", &workspace(&wt, repo.path())).unwrap());
    for name in [skills::DATE_SKILL, skills::WORKSPACE_SKILL] {
        let md = wt.path.join(".agents/skills").join(name).join("SKILL.md");
        assert!(md.is_file(), "{name} not written under .agents/skills");
    }
    assert!(!wt.path.join(".claude").exists(), "dsh's kit leaked into claude's root");
    assert_eq!(status(&wt.path), "", "the kit is visible to git in the worktree");
    git(&wt.path, &["add", "-A"]);
    assert_eq!(status(&wt.path), "", "the kit was staged by `git add -A`");
}

#[test]
fn an_agent_with_no_known_convention_gets_nothing() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-4", "HEAD").unwrap();

    assert!(!skills::emit_for_agent("codex", &workspace(&wt, repo.path())).unwrap());
    assert!(!wt.path.join(".claude").exists(), "wrote a kit for an agent that can't read it");
    assert_eq!(status(&wt.path), "");
}
