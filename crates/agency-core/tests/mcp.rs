//! The emitted MCP config as it actually lands: in a real worktree, against
//! real git. The file formats themselves are unit-tested in the module.

use agency_core::mcp::{self, McpServer};
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

/// `-uall` so an untracked file inside an untracked folder is named, rather
/// than collapsed into a bare `?? packages/` row.
fn status(dir: &Path) -> String {
    let out = Command::new("git")
        .args(["status", "--porcelain", "-uall"])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn kg() -> McpServer {
    McpServer { name: "kg".into(), command: Some("graphify".into()), ..Default::default() }
}

/// AGE-115: the config used to be the one file drop with no exclude, so an
/// agent's reflexive `git add -A` staged it and merging the run put Agency's
/// MCP config in the project.
#[test]
fn every_agents_config_is_written_and_git_never_offers_it() {
    for (agent, rel) in
        [("claude", ".mcp.json"), ("cursor", ".cursor/mcp.json"), ("opencode", "opencode.json")]
    {
        let repo = init_repo();
        let mgr = WorktreeManager::new(repo.path().to_path_buf());
        let wt = mgr.create(&format!("task-{agent}"), "HEAD").unwrap();

        assert!(mcp::emit_for_agent(agent, &wt.path, repo.path(), &[kg()]).unwrap(), "{agent}");
        assert!(wt.path.join(rel).is_file(), "{agent}: {rel} not written");

        assert_eq!(status(&wt.path), "", "{agent}: {rel} is visible to git");
        git(&wt.path, &["add", "-A"]);
        assert_eq!(status(&wt.path), "", "{agent}: {rel} was staged by `git add -A`");
    }
}

/// The exclude is anchored, so a `.mcp.json` the agent writes somewhere down
/// the tree is still the user's own work and still shows up as a change.
#[test]
fn the_exclude_only_hides_the_repo_roots_copy() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-nested", "HEAD").unwrap();
    mcp::emit_for_agent("claude", &wt.path, repo.path(), &[kg()]).unwrap();

    let nested = wt.path.join("packages/thing");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join(".mcp.json"), "{}\n").unwrap();
    assert!(status(&wt.path).contains("packages/thing/.mcp.json"), "{}", status(&wt.path));
}

/// A repo that commits its own `.mcp.json` keeps it byte for byte: an exclude
/// has no say over a tracked file, so upserting into it would put a diff on the
/// branch and ride into the project at merge.
#[test]
fn a_config_the_repo_tracks_is_left_alone() {
    let repo = init_repo();
    std::fs::write(repo.path().join(".mcp.json"), "{\"mcpServers\":{}}\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "mcp"]);

    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-tracked", "HEAD").unwrap();

    assert!(!mcp::emit_for_agent("claude", &wt.path, repo.path(), &[kg()]).unwrap());
    assert_eq!(
        std::fs::read_to_string(wt.path.join(".mcp.json")).unwrap(),
        "{\"mcpServers\":{}}\n",
        "the repo's own config was rewritten"
    );
    assert_eq!(status(&wt.path), "");
}

/// An untracked config already sitting in the worktree is still merged into,
/// so a config a previous run or the user left there keeps its entries.
#[test]
fn an_untracked_config_is_upserted_around() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-untracked", "HEAD").unwrap();
    std::fs::write(wt.path.join(".mcp.json"), r#"{"mcpServers":{"theirs":{"command":"x"}}}"#)
        .unwrap();

    assert!(mcp::emit_for_agent("claude", &wt.path, repo.path(), &[kg()]).unwrap());
    let root: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(wt.path.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(root["mcpServers"]["theirs"]["command"], "x");
    assert_eq!(root["mcpServers"]["kg"]["command"], "graphify");
    assert_eq!(status(&wt.path), "");
}
