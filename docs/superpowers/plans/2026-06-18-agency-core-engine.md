# Agency Core Engine Implementation Plan (Phase 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the headless Rust core engine for Agency — a project registry, git worktree manager, agent profiles, and a PTY-based agent supervisor — fully tested without any UI or real model.

**Architecture:** A single Rust library crate `agency-core` in a Cargo workspace. Storage is local SQLite (rusqlite, bundled). Git operations shell out to the `git` CLI. Agents run in real pseudo-terminals via `portable-pty`; output is delivered through a callback and input is injected through the PTY master. Errors use `anyhow` (this is an application core, not a published library, so a unified error type is fine).

**Tech Stack:** Rust, Cargo workspace, rusqlite (bundled SQLite), portable-pty, serde, uuid, anyhow; tempfile for tests.

## Global Constraints

- **Local-only:** no network calls anywhere in this crate. The engine never reaches the network; only the agent processes it spawns may, per their own configuration.
- **Name:** the product is **Agency**. Crate is `agency-core`.
- **Worktree layout:** worktrees live at `<repo>/.agency/worktrees/<task-id>` on branch `agent/<task-id>`. The repo's `.git/info/exclude` must contain `.agency/` so worktrees never appear as untracked files.
- **Base ref:** worktrees branch off a caller-supplied base ref (default in production is `main`; tests pass `HEAD` to stay branch-name agnostic).
- **TDD:** every behavior gets a failing test first. Commit after each green task.
- **No `unwrap()` in non-test code** except where a lock is known-poison-free and documented; prefer `?` with `anyhow`.

---

## File Structure

```
Cargo.toml                              # workspace root
crates/agency-core/
├── Cargo.toml                          # crate manifest + deps
├── src/
│   ├── lib.rs                          # re-exports modules
│   ├── registry.rs                     # Project + Registry (SQLite)
│   ├── worktree.rs                     # Worktree + WorktreeManager (git CLI)
│   ├── profile.rs                      # AgentProfile + arg/env rendering
│   └── supervisor.rs                   # AgentStatus, AgentHandle, spawn_agent
└── tests/
    ├── fixtures/
    │   └── fake_agent.sh               # scripted CLI used as a stand-in agent
    ├── registry.rs
    ├── worktree.rs
    ├── profile.rs
    └── supervisor.rs
```

Each module owns one responsibility and is independently testable. Integration tests live under `tests/` (one file per module) so they exercise the public API exactly as the app will.

---

### Task 1: Scaffold the workspace and core crate

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/agency-core/Cargo.toml`
- Create: `crates/agency-core/src/lib.rs`
- Create: `.gitignore`

**Interfaces:**
- Consumes: nothing.
- Produces: a compiling `agency-core` crate that exposes (initially empty) modules `registry`, `worktree`, `profile`, `supervisor`, plus a `version()` function returning `&'static str`.

- [ ] **Step 1: Write the failing test**

Create `crates/agency-core/tests/smoke.rs`:

```rust
#[test]
fn crate_exposes_version() {
    assert!(!agency_core::version().is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agency-core --test smoke`
Expected: FAIL — `agency-core` is not yet a package / `version` undefined.

- [ ] **Step 3: Create the workspace and crate manifests**

Root `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/agency-core"]
```

`crates/agency-core/Cargo.toml`:

```toml
[package]
name = "agency-core"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = "1"
rusqlite = { version = "0.31", features = ["bundled"] }
portable-pty = "0.8"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }

[dev-dependencies]
tempfile = "3"
```

`.gitignore`:

```
/target
```

- [ ] **Step 4: Create the crate root with module stubs**

`crates/agency-core/src/lib.rs`:

```rust
pub mod profile;
pub mod registry;
pub mod supervisor;
pub mod worktree;

/// Returns the crate version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
```

Create empty module files so the crate compiles:

`crates/agency-core/src/registry.rs`:

```rust
// Project registry — implemented in Task 2.
```

`crates/agency-core/src/worktree.rs`:

```rust
// Worktree manager — implemented in Task 3.
```

`crates/agency-core/src/profile.rs`:

```rust
// Agent profiles — implemented in Task 4.
```

`crates/agency-core/src/supervisor.rs`:

```rust
// Agent supervisor — implemented in Task 5.
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p agency-core --test smoke`
Expected: PASS (1 passed).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml .gitignore crates/agency-core
git commit -m "Scaffold agency-core crate and workspace"
```

---

### Task 2: Project registry (SQLite)

**Files:**
- Modify: `crates/agency-core/src/registry.rs`
- Test: `crates/agency-core/tests/registry.rs`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `struct Project { pub id: String, pub name: String, pub repo_path: PathBuf, pub default_agent: Option<String>, pub default_provider: Option<String> }` (derives `Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize`)
  - `struct Registry`
  - `Registry::open(db_path: &Path) -> anyhow::Result<Registry>`
  - `Registry::add_project(&self, name: &str, repo_path: &Path) -> anyhow::Result<Project>`
  - `Registry::get_project(&self, id: &str) -> anyhow::Result<Option<Project>>`
  - `Registry::list_projects(&self) -> anyhow::Result<Vec<Project>>`
  - `Registry::remove_project(&self, id: &str) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing test**

Create `crates/agency-core/tests/registry.rs`:

```rust
use agency_core::registry::Registry;
use std::path::Path;

#[test]
fn add_then_get_and_list_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(&dir.path().join("agency.db")).unwrap();

    let p = reg.add_project("demo", Path::new("/tmp/demo-repo")).unwrap();
    assert_eq!(p.name, "demo");
    assert_eq!(p.repo_path, Path::new("/tmp/demo-repo"));
    assert!(!p.id.is_empty());

    let fetched = reg.get_project(&p.id).unwrap().unwrap();
    assert_eq!(fetched, p);

    let all = reg.list_projects().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0], p);
}

#[test]
fn remove_project_deletes_it() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(&dir.path().join("agency.db")).unwrap();

    let p = reg.add_project("demo", Path::new("/tmp/demo-repo")).unwrap();
    reg.remove_project(&p.id).unwrap();

    assert!(reg.get_project(&p.id).unwrap().is_none());
    assert_eq!(reg.list_projects().unwrap().len(), 0);
}

#[test]
fn data_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agency.db");

    let id = {
        let reg = Registry::open(&db).unwrap();
        reg.add_project("demo", Path::new("/tmp/demo-repo")).unwrap().id
    };

    let reg2 = Registry::open(&db).unwrap();
    assert!(reg2.get_project(&id).unwrap().is_some());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agency-core --test registry`
Expected: FAIL — `Registry` not found.

- [ ] **Step 3: Implement the registry**

Replace `crates/agency-core/src/registry.rs` with:

```rust
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub repo_path: PathBuf,
    pub default_agent: Option<String>,
    pub default_provider: Option<String>,
}

pub struct Registry {
    conn: Connection,
}

impl Registry {
    pub fn open(db_path: &Path) -> Result<Registry> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(db_path)
            .with_context(|| format!("opening db at {}", db_path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                repo_path TEXT NOT NULL,
                default_agent TEXT,
                default_provider TEXT
            );",
        )?;
        Ok(Registry { conn })
    }

    pub fn add_project(&self, name: &str, repo_path: &Path) -> Result<Project> {
        let project = Project {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            repo_path: repo_path.to_path_buf(),
            default_agent: None,
            default_provider: None,
        };
        self.conn.execute(
            "INSERT INTO projects (id, name, repo_path, default_agent, default_provider)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                project.id,
                project.name,
                project.repo_path.to_string_lossy(),
                project.default_agent,
                project.default_provider,
            ],
        )?;
        Ok(project)
    }

    pub fn get_project(&self, id: &str) -> Result<Option<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, default_agent, default_provider
             FROM projects WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_project(row)?)),
            None => Ok(None),
        }
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, default_agent, default_provider
             FROM projects ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| Ok(row_to_project(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    pub fn remove_project(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn row_to_project(row: &rusqlite::Row) -> Result<Project> {
    let repo_path: String = row.get(2)?;
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        repo_path: PathBuf::from(repo_path),
        default_agent: row.get(3)?,
        default_provider: row.get(4)?,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agency-core --test registry`
Expected: PASS (3 passed).

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/registry.rs crates/agency-core/tests/registry.rs
git commit -m "Add SQLite project registry"
```

---

### Task 3: Worktree manager (git CLI)

**Files:**
- Modify: `crates/agency-core/src/worktree.rs`
- Test: `crates/agency-core/tests/worktree.rs`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `struct Worktree { pub task_id: String, pub path: PathBuf, pub branch: String }` (derives `Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize`)
  - `struct WorktreeManager`
  - `WorktreeManager::new(repo_path: PathBuf) -> WorktreeManager`
  - `WorktreeManager::create(&self, task_id: &str, base: &str) -> anyhow::Result<Worktree>`
  - `WorktreeManager::list(&self) -> anyhow::Result<Vec<Worktree>>`
  - `WorktreeManager::remove(&self, task_id: &str) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing test**

Create `crates/agency-core/tests/worktree.rs`:

```rust
use agency_core::worktree::WorktreeManager;
use std::path::Path;
use std::process::Command;

/// Create a real git repo with one commit; return its path (kept alive by `dir`).
fn init_repo(dir: &Path) {
    let run = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {:?} failed", args);
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    std::fs::write(dir.join("README.md"), "hi").unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "init"]);
}

#[test]
fn create_makes_worktree_and_branch() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let mgr = WorktreeManager::new(dir.path().to_path_buf());

    let wt = mgr.create("task-1", "HEAD").unwrap();

    assert_eq!(wt.task_id, "task-1");
    assert_eq!(wt.branch, "agent/task-1");
    assert!(wt.path.ends_with(".agency/worktrees/task-1"));
    assert!(wt.path.join("README.md").exists());

    // The exclude file should keep .agency/ untracked-invisible.
    let exclude = std::fs::read_to_string(dir.path().join(".git/info/exclude")).unwrap();
    assert!(exclude.contains(".agency/"));
}

#[test]
fn list_returns_created_worktrees() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let mgr = WorktreeManager::new(dir.path().to_path_buf());

    mgr.create("task-1", "HEAD").unwrap();
    mgr.create("task-2", "HEAD").unwrap();

    let mut ids: Vec<String> = mgr.list().unwrap().into_iter().map(|w| w.task_id).collect();
    ids.sort();
    assert_eq!(ids, vec!["task-1".to_string(), "task-2".to_string()]);
}

#[test]
fn remove_deletes_worktree() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let mgr = WorktreeManager::new(dir.path().to_path_buf());

    let wt = mgr.create("task-1", "HEAD").unwrap();
    mgr.remove("task-1").unwrap();

    assert!(!wt.path.exists());
    assert!(mgr.list().unwrap().is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agency-core --test worktree`
Expected: FAIL — `WorktreeManager` not found.

- [ ] **Step 3: Implement the worktree manager**

Replace `crates/agency-core/src/worktree.rs` with:

```rust
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Worktree {
    pub task_id: String,
    pub path: PathBuf,
    pub branch: String,
}

pub struct WorktreeManager {
    repo_path: PathBuf,
}

impl WorktreeManager {
    pub fn new(repo_path: PathBuf) -> WorktreeManager {
        WorktreeManager { repo_path }
    }

    fn worktrees_root(&self) -> PathBuf {
        self.repo_path.join(".agency").join("worktrees")
    }

    fn branch_for(task_id: &str) -> String {
        format!("agent/{task_id}")
    }

    /// Run a git command in the repo, returning stdout on success.
    fn git(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.repo_path)
            .output()?;
        if !output.status.success() {
            bail!(
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Ensure `.agency/` is excluded so worktrees never show as untracked.
    fn ensure_excluded(&self) -> Result<()> {
        let exclude = self.repo_path.join(".git").join("info").join("exclude");
        let current = std::fs::read_to_string(&exclude).unwrap_or_default();
        if !current.lines().any(|l| l.trim() == ".agency/") {
            if let Some(parent) = exclude.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut updated = current;
            if !updated.is_empty() && !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(".agency/\n");
            std::fs::write(&exclude, updated)?;
        }
        Ok(())
    }

    pub fn create(&self, task_id: &str, base: &str) -> Result<Worktree> {
        self.ensure_excluded()?;
        let path = self.worktrees_root().join(task_id);
        let branch = Self::branch_for(task_id);
        let path_str = path.to_string_lossy().to_string();
        self.git(&["worktree", "add", &path_str, "-b", &branch, base])?;
        Ok(Worktree {
            task_id: task_id.to_string(),
            path,
            branch,
        })
    }

    pub fn list(&self) -> Result<Vec<Worktree>> {
        let out = self.git(&["worktree", "list", "--porcelain"])?;
        let root = self.worktrees_root();
        let mut worktrees = Vec::new();
        let mut cur_path: Option<PathBuf> = None;
        let mut cur_branch: Option<String> = None;

        let mut flush = |path: &mut Option<PathBuf>, branch: &mut Option<String>, acc: &mut Vec<Worktree>| {
            if let (Some(p), Some(b)) = (path.take(), branch.take()) {
                if p.starts_with(&root) {
                    if let Some(task_id) = p.file_name().and_then(|s| s.to_str()) {
                        acc.push(Worktree {
                            task_id: task_id.to_string(),
                            path: p.clone(),
                            branch: b,
                        });
                    }
                }
            }
        };

        for line in out.lines() {
            if let Some(rest) = line.strip_prefix("worktree ") {
                flush(&mut cur_path, &mut cur_branch, &mut worktrees);
                cur_path = Some(PathBuf::from(rest));
            } else if let Some(rest) = line.strip_prefix("branch ") {
                cur_branch = Some(rest.trim_start_matches("refs/heads/").to_string());
            }
        }
        flush(&mut cur_path, &mut cur_branch, &mut worktrees);
        Ok(worktrees)
    }

    pub fn remove(&self, task_id: &str) -> Result<()> {
        let path = self.worktrees_root().join(task_id);
        let path_str = path.to_string_lossy().to_string();
        self.git(&["worktree", "remove", &path_str, "--force"])?;
        // Branch deletion is best-effort; ignore failure (e.g. already merged/gone).
        let branch = Self::branch_for(task_id);
        let _ = self.git(&["branch", "-D", &branch]);
        Ok(())
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agency-core --test worktree`
Expected: PASS (3 passed).

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/worktree.rs crates/agency-core/tests/worktree.rs
git commit -m "Add git worktree manager"
```

---

### Task 4: Agent profiles (launch config + rendering)

**Files:**
- Modify: `crates/agency-core/src/profile.rs`
- Create: `crates/agency-core/tests/fixtures/fake_agent.sh`
- Test: `crates/agency-core/tests/profile.rs`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `struct AgentProfile { pub name: String, pub command: String, pub args: Vec<String>, pub env: Vec<(String, String)> }` (derives `Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize`)
  - `AgentProfile::render_args(&self, prompt: &str) -> Vec<String>` — replaces the literal token `{{prompt}}` in each arg with `prompt`.

- [ ] **Step 1: Create the fake-agent fixture**

Create `crates/agency-core/tests/fixtures/fake_agent.sh`:

```bash
#!/usr/bin/env bash
# A stand-in terminal agent for tests: announce readiness, echo the prompt arg,
# read one line of input and echo it back, then exit cleanly.
echo "AGENT_READY"
echo "PROMPT:$1"
read -r line
echo "GOT:$line"
echo "AGENT_DONE"
```

Make it executable:

```bash
chmod +x crates/agency-core/tests/fixtures/fake_agent.sh
```

- [ ] **Step 2: Write the failing test**

Create `crates/agency-core/tests/profile.rs`:

```rust
use agency_core::profile::AgentProfile;

#[test]
fn render_args_substitutes_prompt_token() {
    let profile = AgentProfile {
        name: "claude".into(),
        command: "claude".into(),
        args: vec!["-p".into(), "{{prompt}}".into()],
        env: vec![],
    };

    let rendered = profile.render_args("fix the bug");
    assert_eq!(rendered, vec!["-p".to_string(), "fix the bug".to_string()]);
}

#[test]
fn render_args_leaves_other_args_untouched() {
    let profile = AgentProfile {
        name: "x".into(),
        command: "x".into(),
        args: vec!["--flag".into(), "value".into()],
        env: vec![],
    };

    assert_eq!(
        profile.render_args("anything"),
        vec!["--flag".to_string(), "value".to_string()]
    );
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p agency-core --test profile`
Expected: FAIL — `AgentProfile` not found.

- [ ] **Step 4: Implement the profile**

Replace `crates/agency-core/src/profile.rs` with:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentProfile {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl AgentProfile {
    /// Render `args`, replacing the literal token `{{prompt}}` with `prompt`.
    pub fn render_args(&self, prompt: &str) -> Vec<String> {
        self.args
            .iter()
            .map(|a| a.replace("{{prompt}}", prompt))
            .collect()
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p agency-core --test profile`
Expected: PASS (2 passed).

- [ ] **Step 6: Commit**

```bash
git add crates/agency-core/src/profile.rs crates/agency-core/tests/profile.rs crates/agency-core/tests/fixtures/fake_agent.sh
git commit -m "Add agent profiles and fake-agent test fixture"
```

---

### Task 5: Agent supervisor (PTY spawn, stream, input, status)

**Files:**
- Modify: `crates/agency-core/src/supervisor.rs`
- Test: `crates/agency-core/tests/supervisor.rs`

**Interfaces:**
- Consumes: `AgentProfile` from Task 4 (`render_args`, `command`, `env`).
- Produces:
  - `enum AgentStatus { Running, Idle, Exited(i32), Crashed }` (derives `Debug, Clone, PartialEq`)
  - `struct AgentHandle`
  - `AgentHandle::write_input(&self, data: &[u8]) -> anyhow::Result<()>`
  - `AgentHandle::status(&self) -> AgentStatus`
  - `fn spawn_agent(profile: &AgentProfile, cwd: &Path, prompt: &str, on_output: F) -> anyhow::Result<AgentHandle>` where `F: Fn(Vec<u8>) + Send + 'static`

Note: only `Running` and `Exited(code)` are produced in Phase 1. `Idle` (prompt-return + idle-timer heuristic) and `Crashed` (signal/abnormal exit classification) are defined now but wired up in a later phase; they are part of the public enum so consumers can match exhaustively from the start.

- [ ] **Step 1: Write the failing test**

Create `crates/agency-core/tests/supervisor.rs`:

```rust
use agency_core::profile::AgentProfile;
use agency_core::supervisor::{spawn_agent, AgentStatus};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn fixture_path() -> String {
    // tests/fixtures/fake_agent.sh relative to the crate manifest.
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("fake_agent.sh");
    p.to_string_lossy().to_string()
}

/// Poll `buf` until it contains `needle` or the timeout elapses.
fn wait_for(buf: &Arc<Mutex<String>>, needle: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if buf.lock().unwrap().contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn spawns_streams_input_and_exits() {
    let profile = AgentProfile {
        name: "fake".into(),
        command: fixture_path(),
        args: vec!["{{prompt}}".into()],
        env: vec![],
    };

    let buf = Arc::new(Mutex::new(String::new()));
    let buf_cb = buf.clone();
    let cwd = std::env::temp_dir();

    let handle = spawn_agent(&profile, &cwd, "do-the-thing", move |bytes| {
        buf_cb.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes));
    })
    .unwrap();

    // Banner + rendered prompt arg appear.
    assert!(wait_for(&buf, "AGENT_READY", Duration::from_secs(5)));
    assert!(wait_for(&buf, "PROMPT:do-the-thing", Duration::from_secs(5)));

    // Inject input; the agent echoes it back.
    handle.write_input(b"ping\n").unwrap();
    assert!(wait_for(&buf, "GOT:ping", Duration::from_secs(5)));
    assert!(wait_for(&buf, "AGENT_DONE", Duration::from_secs(5)));

    // Wait for clean exit and assert status.
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if matches!(handle.status(), AgentStatus::Exited(_)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(handle.status(), AgentStatus::Exited(0));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agency-core --test supervisor`
Expected: FAIL — `spawn_agent` / `AgentStatus` not found.

- [ ] **Step 3: Implement the supervisor**

Replace `crates/agency-core/src/supervisor.rs` with:

```rust
use crate::profile::AgentProfile;
use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    Running,
    Idle,
    Exited(i32),
    Crashed,
}

pub struct AgentHandle {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    status: Arc<Mutex<AgentStatus>>,
    // Keep the master alive so the PTY stays open for the lifetime of the handle.
    _master: Box<dyn MasterPty + Send>,
}

impl AgentHandle {
    pub fn write_input(&self, data: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn status(&self) -> AgentStatus {
        self.status.lock().unwrap().clone()
    }
}

/// Spawn `profile` in a PTY with working directory `cwd`, injecting `prompt`
/// into the rendered args. `on_output` is called with raw PTY bytes as they arrive.
pub fn spawn_agent<F>(
    profile: &AgentProfile,
    cwd: &Path,
    prompt: &str,
    on_output: F,
) -> Result<AgentHandle>
where
    F: Fn(Vec<u8>) + Send + 'static,
{
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut cmd = CommandBuilder::new(&profile.command);
    cmd.args(profile.render_args(prompt));
    cmd.cwd(cwd);
    for (k, v) in &profile.env {
        cmd.env(k, v);
    }

    let mut child = pair.slave.spawn_command(cmd)?;
    // The slave handle is no longer needed once the child holds it.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;

    let status = Arc::new(Mutex::new(AgentStatus::Running));

    // Reader thread: pump PTY output to the callback until EOF.
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => on_output(buf[..n].to_vec()),
            }
        }
    });

    // Wait thread: record the exit code when the child finishes.
    let status_for_wait = status.clone();
    std::thread::spawn(move || {
        let code = match child.wait() {
            Ok(es) => es.exit_code() as i32,
            Err(_) => {
                *status_for_wait.lock().unwrap() = AgentStatus::Crashed;
                return;
            }
        };
        *status_for_wait.lock().unwrap() = AgentStatus::Exited(code);
    });

    Ok(AgentHandle {
        writer: Arc::new(Mutex::new(writer)),
        status,
        _master: pair.master,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agency-core --test supervisor`
Expected: PASS (1 passed). If the input-echo assertion is flaky on a very slow machine, the 5s timeouts already absorb normal scheduling jitter.

- [ ] **Step 5: Run the full suite**

Run: `cargo test -p agency-core`
Expected: PASS — all tests across smoke, registry, worktree, profile, supervisor.

- [ ] **Step 6: Commit**

```bash
git add crates/agency-core/src/supervisor.rs crates/agency-core/tests/supervisor.rs
git commit -m "Add PTY agent supervisor"
```

---

## Self-Review

**Spec coverage (Phase 1 scope only):**
- Project registry (SQLite) → Task 2. ✓
- Worktree manager (`.agency/worktrees/<task-id>`, branch `agent/<task-id>`, exclude) → Task 3. ✓
- Agent profiles / adapter config → Task 4. ✓
- Agent supervisor (PTY, stream output, inject input, status) → Task 5. ✓
- Local-only constraint → no network code anywhere; asserted by absence. ✓
- Deferred to later phases (correctly out of scope here): Tauri shell + UI, git diff/stage/commit/push panel, merge-resolver skill, Idle/Crashed heuristic wiring, model-provider settings. These are Phases 2–5.

**Placeholder scan:** No TBD/TODO in steps; every code step contains complete code; commands have expected output. ✓

**Type consistency:** `Project`, `Worktree`, `AgentProfile`, `AgentStatus`, `AgentHandle`, `spawn_agent`, `render_args` names and signatures match between their "Produces" blocks and their usages in later tests. `render_args` is defined in Task 4 and consumed in Task 5's `spawn_agent`. ✓

**Notes for the executor:**
- Requires `git` and a POSIX shell (`bash`) on PATH; both are assumed present on the target dev machine (macOS).
- Crate versions are floors; if a listed version is yanked or unavailable, take the nearest compatible release and keep the same API usage.
