# Agency Phase 7a Implementation Plan — Multi-Agent Shell, Grid/Focus & tmux Sessions

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Rebuild Agency's UI into the designed multi-agent dashboard (shell + project tree + Agents Grid/Focus + add-agent flow) and back each agent with a durable **tmux** session.

**Architecture:** A new `agency_core::tmux` module manages durable `agency-<task-id>` tmux sessions (start/status/capture/kill); live Focus terminals reuse the existing `supervisor::spawn_agent` to run `tmux attach` in a PTY. Runs persist in SQLite. `agency-app` exposes run commands; the frontend is a new shell (TitleBar/StatusBar/ProjectTree/AgentsGrid/AgentFocus) reusing the existing git/settings/merge components.

**Tech Stack:** Rust (std `Command`→tmux/git, portable-pty via supervisor, rusqlite), Tauri v2, React + TypeScript, xterm.js.

## Global Constraints

- **Local-only / zero first-party data collection:** Agency's own code makes no network calls.
- **tmux session per run:** named `agency-<task-id>`, durable (survives app quit), `remain-on-exit on`. The module resolves the tmux binary via a **configurable path** (`Tmux::new(bin: PathBuf)`); in dev it falls back to `"tmux"` on PATH. Bundling the binary is a later packaging task.
- **Worktree/branch conventions** owned by `agency_core::worktree`: `<repo>/.agency/worktrees/<task-id>`, branch `agent/<task-id>`, base default `main`.
- **Agent-type badge colors:** Claude=peach, Pi=teal, Hermes=mauve, shell/other=default. Profile name decides the badge.
- **Design authority:** `docs/design-handoff/Agency-Design-Handoff.html` (§) and `docs/design-handoff/Agency v2.dc.html` (mockup). Use the Catppuccin tokens already in `ui/src/theme.css`; no literal hexes in components.
- **Frontend↔Rust:** agency-app/agency-core DTOs serialize with their field names; tagged enums use `#[serde(tag=…)]` as specified. Tauri maps camelCase JS args to snake_case Rust params.
- TDD for Rust; commit after each green task. Frontend tasks are build-verified (visual fidelity is a human check).

---

## File Structure

```
crates/agency-core/
├── src/tmux.rs        # NEW: Tmux backend (start/status/capture/attach/kill)
├── src/git.rs         # MODIFY: diff_stat
├── src/registry.rs    # MODIFY: runs table + CRUD
└── src/lib.rs         # MODIFY: pub mod tmux;

crates/agency-app/
├── src/state.rs       # MODIFY: run model (create/list/status/discard/rerun, attach), drop old sessions/start_task
├── src/commands.rs    # MODIFY: run commands; adapt merge commands
└── src/lib.rs         # MODIFY: register commands

ui/src/
├── api.ts                       # MODIFY: run types + wrappers
├── store/runs.tsx               # NEW: RunStore context (runs, selection, view)
├── App.tsx                      # REPLACE: shell (TitleBar+Body+StatusBar)
├── components/TitleBar.tsx      # NEW
├── components/StatusBar.tsx     # NEW
├── components/ProjectTree.tsx   # NEW (replaces ProjectSidebar in the shell)
├── components/AgentsView.tsx    # NEW (Grid|Focus switch + header)
├── components/AgentTile.tsx     # NEW
├── components/AgentFocus.tsx    # NEW (rail + FocusTerminal)
├── components/FocusTerminal.tsx # NEW (refactor of TerminalPane → attach_run)
├── components/NewTaskForm.tsx   # NEW
└── styles.css                   # MODIFY: shell/grid/tile/focus styles (tokens)
```

Reused unchanged: `GitPanel`, `Settings`, `MergeModal`, all of `agency-core` except the additions above. `ProjectSidebar`/`TaskBoard`/`TerminalPane` are superseded (delete in Task 12).

---

### Task 1: `agency_core::tmux` — session lifecycle

**Files:**
- Create: `crates/agency-core/src/tmux.rs`
- Modify: `crates/agency-core/src/lib.rs` (`pub mod tmux;`)
- Test: `crates/agency-core/tests/tmux.rs`

**Interfaces:**
- Produces:
  - `enum SessionStatus { Running, Exited { code: i32 }, Gone }` (derives `Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize`; `#[serde(tag = "state", rename_all = "camelCase")]`).
  - `struct Tmux { bin: PathBuf }`; `Tmux::new(bin: PathBuf) -> Tmux`; `Tmux::resolved() -> Tmux` (returns `Tmux::new("tmux".into())` for now — bundled-path resolution is a packaging task).
  - `Tmux::start_session(&self, name: &str, cwd: &Path, command: &str, args: &[String], env: &[(String, String)]) -> Result<()>`
  - `Tmux::session_exists(&self, name: &str) -> Result<bool>`
  - `Tmux::session_status(&self, name: &str) -> Result<SessionStatus>`
  - `Tmux::capture(&self, name: &str, lines: usize) -> Result<String>`
  - `Tmux::kill_session(&self, name: &str) -> Result<()>`
- `attach` is Task 2.

- [ ] **Step 1: Write the failing test**

Create `crates/agency-core/tests/tmux.rs`:

```rust
use agency_core::tmux::{SessionStatus, Tmux};
use std::path::Path;

fn unique(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    format!("{prefix}-{n}")
}

#[test]
fn start_capture_status_kill() {
    let t = Tmux::resolved();
    let name = unique("agencytest");
    let cwd = std::env::temp_dir();

    // A command that prints then exits 0 quickly.
    t.start_session(&name, &cwd, "sh", &["-c".into(), "echo HELLO; exit 0".into()], &[])
        .unwrap();
    assert!(t.session_exists(&name).unwrap());

    // Wait for output to appear in the pane.
    let mut captured = String::new();
    for _ in 0..50 {
        captured = t.capture(&name, 50).unwrap();
        if captured.contains("HELLO") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(captured.contains("HELLO"), "capture was: {captured:?}");

    // remain-on-exit keeps the session; status becomes Exited(0).
    let mut status = SessionStatus::Running;
    for _ in 0..50 {
        status = t.session_status(&name).unwrap();
        if matches!(status, SessionStatus::Exited { .. }) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert_eq!(status, SessionStatus::Exited { code: 0 });

    t.kill_session(&name).unwrap();
    assert!(!t.session_exists(&name).unwrap());
    assert_eq!(t.session_status(&name).unwrap(), SessionStatus::Gone);
}

#[test]
fn start_session_runs_in_cwd_with_env() {
    let t = Tmux::resolved();
    let name = unique("agencyenv");
    let dir = tempfile::tempdir().unwrap();

    t.start_session(
        &name,
        dir.path(),
        "sh",
        &["-c".into(), "echo CWD=$(pwd); echo VAR=$MYVAR; sleep 2".into()],
        &[("MYVAR".into(), "xyz".into())],
    )
    .unwrap();

    let mut cap = String::new();
    for _ in 0..50 {
        cap = t.capture(&name, 50).unwrap();
        if cap.contains("VAR=xyz") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(cap.contains("VAR=xyz"), "cap: {cap:?}");
    // cwd path may be /private-symlinked on macOS; just assert the temp dir's final component.
    let leaf = dir.path().file_name().unwrap().to_string_lossy().to_string();
    assert!(cap.contains(&leaf), "cap: {cap:?}");

    t.kill_session(&name).unwrap();
}
```

Add `Path` import note: the test uses `std::path::Path` indirectly via `&Path` params — keep the `use std::path::Path;` only if the compiler needs it; remove if unused to stay warning-free.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core --test tmux`
Expected: FAIL — module `tmux` not found.

- [ ] **Step 3: Implement**

Create `crates/agency-core/src/tmux.rs`:

```rust
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SessionStatus {
    Running,
    Exited { code: i32 },
    Gone,
}

pub struct Tmux {
    bin: PathBuf,
}

impl Tmux {
    pub fn new(bin: PathBuf) -> Tmux {
        Tmux { bin }
    }

    /// Resolve the tmux binary. For now this is system `tmux`; a packaging task
    /// will point this at a binary bundled inside the app.
    pub fn resolved() -> Tmux {
        Tmux::new(PathBuf::from("tmux"))
    }

    fn cmd(&self, args: &[&str]) -> Result<std::process::Output> {
        Ok(Command::new(&self.bin).args(args).output()?)
    }

    fn ok(&self, args: &[&str]) -> Result<String> {
        let out = self.cmd(args)?;
        if !out.status.success() {
            bail!(
                "tmux {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    pub fn start_session(
        &self,
        name: &str,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<()> {
        let cwd_s = cwd.to_string_lossy().to_string();
        let mut a: Vec<String> = vec![
            "new-session".into(),
            "-d".into(),
            "-s".into(),
            name.into(),
            "-c".into(),
            cwd_s,
            "-x".into(),
            "220".into(),
            "-y".into(),
            "50".into(),
        ];
        for (k, v) in env {
            a.push("-e".into());
            a.push(format!("{k}={v}"));
        }
        // Terminate options; the rest is the command to run.
        a.push(command.to_string());
        a.extend(args.iter().cloned());
        let aref: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
        self.ok(&aref)?;
        // Keep the pane (and its exit code + final output) after the process exits.
        self.ok(&["set-option", "-t", name, "remain-on-exit", "on"])?;
        Ok(())
    }

    pub fn session_exists(&self, name: &str) -> Result<bool> {
        Ok(self.cmd(&["has-session", "-t", name])?.status.success())
    }

    pub fn session_status(&self, name: &str) -> Result<SessionStatus> {
        if !self.session_exists(name)? {
            return Ok(SessionStatus::Gone);
        }
        let out = self.ok(&[
            "list-panes",
            "-t",
            name,
            "-F",
            "#{pane_dead} #{pane_dead_status}",
        ])?;
        let first = out.lines().next().unwrap_or("");
        let mut parts = first.split_whitespace();
        let dead = parts.next().unwrap_or("0");
        if dead == "1" {
            let code = parts.next().unwrap_or("0").parse::<i32>().unwrap_or(0);
            Ok(SessionStatus::Exited { code })
        } else {
            Ok(SessionStatus::Running)
        }
    }

    pub fn capture(&self, name: &str, lines: usize) -> Result<String> {
        if !self.session_exists(name)? {
            return Ok(String::new());
        }
        let start = format!("-{lines}");
        self.ok(&["capture-pane", "-p", "-t", name, "-S", &start])
    }

    pub fn kill_session(&self, name: &str) -> Result<()> {
        if self.session_exists(name)? {
            self.ok(&["kill-session", "-t", name])?;
        }
        Ok(())
    }
}
```

Add to `crates/agency-core/src/lib.rs`:

```rust
pub mod tmux;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-core --test tmux`
Expected: PASS (2 passed). Requires `tmux` on PATH.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/tmux.rs crates/agency-core/src/lib.rs crates/agency-core/tests/tmux.rs
git commit -m "Add tmux session lifecycle module"
```

---

### Task 2: `agency_core::tmux` — live attach

**Files:**
- Modify: `crates/agency-core/src/tmux.rs`
- Test: `crates/agency-core/tests/tmux.rs`

**Interfaces:**
- Consumes: `crate::supervisor::{spawn_agent, AgentHandle}`, `crate::profile::AgentProfile`.
- Produces: `Tmux::attach<F>(&self, name: &str, on_output: F) -> Result<AgentHandle>` where `F: Fn(Vec<u8>) + Send + 'static` — runs `tmux attach-session -t <name>` in a PTY (reusing `spawn_agent`); the returned `AgentHandle` streams pane output and `write_input` sends keystrokes into the session. Dropping the handle detaches (session stays alive).

- [ ] **Step 1: Write the failing test**

Append to `crates/agency-core/tests/tmux.rs`:

```rust
use std::sync::{Arc, Mutex};

#[test]
fn attach_streams_output_and_input() {
    let t = Tmux::resolved();
    let name = unique("agencyattach");
    let cwd = std::env::temp_dir();
    // A shell that prints READY, reads a line, echoes it.
    t.start_session(&name, &cwd, "sh", &["-c".into(), "echo READY; read x; echo GOT:$x; sleep 2".into()], &[])
        .unwrap();

    let buf = Arc::new(Mutex::new(String::new()));
    let b = buf.clone();
    let handle = t.attach(&name, move |bytes| {
        b.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes));
    }).unwrap();

    let wait = |needle: &str| {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            if buf.lock().unwrap().contains(needle) { return true; }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        false
    };

    assert!(wait("READY"), "buf: {:?}", buf.lock().unwrap());
    handle.write_input(b"ping\n").unwrap();
    assert!(wait("GOT:ping"), "buf: {:?}", buf.lock().unwrap());

    drop(handle); // detach
    // session still alive right after detach
    assert!(t.session_exists(&name).unwrap());
    t.kill_session(&name).unwrap();
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core --test tmux`
Expected: FAIL — `attach` not found.

- [ ] **Step 3: Implement**

Add to `crates/agency-core/src/tmux.rs` (imports at top):

```rust
use crate::profile::AgentProfile;
use crate::supervisor::{spawn_agent, AgentHandle};
```

Add the method to `impl Tmux`:

```rust
    pub fn attach<F>(&self, name: &str, on_output: F) -> Result<AgentHandle>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        let profile = AgentProfile {
            name: "tmux-attach".to_string(),
            command: self.bin.to_string_lossy().to_string(),
            args: vec!["attach-session".to_string(), "-t".to_string(), name.to_string()],
            env: vec![],
        };
        // cwd is irrelevant for an attach; use the temp dir which always exists.
        spawn_agent(&profile, &std::env::temp_dir(), "", on_output)
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-core --test tmux`
Expected: PASS (3 passed).

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/tmux.rs crates/agency-core/tests/tmux.rs
git commit -m "Add tmux live attach reusing the PTY supervisor"
```

---

### Task 3: `agency_core::git` — diff stat

**Files:**
- Modify: `crates/agency-core/src/git.rs`
- Test: `crates/agency-core/tests/git.rs`

**Interfaces:**
- Consumes: the private `git()` helper in `git.rs`.
- Produces: `struct DiffStat { pub added: u32, pub deleted: u32, pub files: u32 }` (derives `Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize`); `fn diff_stat(worktree: &Path, base: &str) -> anyhow::Result<DiffStat>` via `git diff --numstat <base>...HEAD`.

- [ ] **Step 1: Write the failing test**

Append to `crates/agency-core/tests/git.rs`:

```rust
#[test]
fn diff_stat_counts_added_deleted_files() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()); // existing helper: repo on a branch with tracked.txt = "one\n"
    // Branch off and make changes: modify tracked.txt, add new file.
    run(dir.path(), &["checkout", "-q", "-b", "feat"]);
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "a\nb\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-q", "-m", "changes"]);

    let stat = git::diff_stat(dir.path(), "master").unwrap_or_else(|_| {
        git::diff_stat(dir.path(), "main").unwrap()
    });
    assert_eq!(stat.files, 2);
    assert_eq!(stat.added, 4); // +two +three (tracked) + a + b (new)
    assert_eq!(stat.deleted, 0);
}
```

Note: `init_repo` in `tests/git.rs` may create `master` or `main` depending on git defaults — the test tries `master` then `main`. If your `init_repo` pins a branch, pass that name directly instead of the fallback.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core --test git`
Expected: FAIL — `diff_stat` not found.

- [ ] **Step 3: Implement**

Append to `crates/agency-core/src/git.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffStat {
    pub added: u32,
    pub deleted: u32,
    pub files: u32,
}

pub fn diff_stat(worktree: &Path, base: &str) -> Result<DiffStat> {
    let range = format!("{base}...HEAD");
    let out = git(worktree, &["diff", "--numstat", &range])?;
    let mut stat = DiffStat { added: 0, deleted: 0, files: 0 };
    for line in out.lines() {
        let mut parts = line.split('\t');
        let a = parts.next().unwrap_or("0");
        let d = parts.next().unwrap_or("0");
        // Binary files show "-" for counts; treat as 0 but still count the file.
        stat.added += a.parse::<u32>().unwrap_or(0);
        stat.deleted += d.parse::<u32>().unwrap_or(0);
        stat.files += 1;
    }
    Ok(stat)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-core --test git`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/git.rs crates/agency-core/tests/git.rs
git commit -m "Add git diff_stat (numstat) for tile diff badges"
```

---

### Task 4: `Registry` — runs persistence

**Files:**
- Modify: `crates/agency-core/src/registry.rs`
- Test: `crates/agency-core/tests/registry.rs`

**Interfaces:**
- Produces:
  - `struct Run { pub id: String, pub project_id: String, pub agent: String, pub prompt: String, pub base: String, pub branch: String, pub created_at: i64 }` (derives `Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize`).
  - On `Registry`: `insert_run(&self, run: &Run) -> Result<()>`, `list_runs(&self, project_id: &str) -> Result<Vec<Run>>` (newest first), `get_run(&self, id: &str) -> Result<Option<Run>>`, `delete_run(&self, id: &str) -> Result<()>`.
  - New table created in `open()` via `CREATE TABLE IF NOT EXISTS runs(id TEXT PRIMARY KEY, project_id TEXT NOT NULL, agent TEXT NOT NULL, prompt TEXT NOT NULL, base TEXT NOT NULL, branch TEXT NOT NULL, created_at INTEGER NOT NULL)`.

- [ ] **Step 1: Write the failing test**

Append to `crates/agency-core/tests/registry.rs`:

```rust
use agency_core::registry::Run;

#[test]
fn runs_persist_list_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agency.db");
    let run = Run {
        id: "task-1".into(),
        project_id: "proj-1".into(),
        agent: "claude".into(),
        prompt: "do it".into(),
        base: "main".into(),
        branch: "agent/task-1".into(),
        created_at: 1000,
    };
    {
        let reg = Registry::open(&db).unwrap();
        reg.insert_run(&run).unwrap();
    }
    let reg = Registry::open(&db).unwrap();
    assert_eq!(reg.get_run("task-1").unwrap().unwrap(), run);
    assert_eq!(reg.list_runs("proj-1").unwrap(), vec![run.clone()]);
    assert_eq!(reg.list_runs("other").unwrap().len(), 0);

    reg.delete_run("task-1").unwrap();
    assert!(reg.get_run("task-1").unwrap().is_none());
}

#[test]
fn list_runs_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(&dir.path().join("agency.db")).unwrap();
    for (id, ts) in [("a", 1), ("b", 3), ("c", 2)] {
        reg.insert_run(&Run {
            id: id.into(), project_id: "p".into(), agent: "shell".into(),
            prompt: "".into(), base: "main".into(), branch: format!("agent/{id}"), created_at: ts,
        }).unwrap();
    }
    let ids: Vec<String> = reg.list_runs("p").unwrap().into_iter().map(|r| r.id).collect();
    assert_eq!(ids, vec!["b", "c", "a"]); // created_at desc
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core --test registry`
Expected: FAIL — `Run`/`insert_run` not found.

- [ ] **Step 3: Implement**

In `crates/agency-core/src/registry.rs`, add the `runs` table to the `execute_batch` in `open()`:

```rust
            CREATE TABLE IF NOT EXISTS runs (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                agent TEXT NOT NULL,
                prompt TEXT NOT NULL,
                base TEXT NOT NULL,
                branch TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
```

Add the struct (near `Project`):

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub project_id: String,
    pub agent: String,
    pub prompt: String,
    pub base: String,
    pub branch: String,
    pub created_at: i64,
}
```

Add methods to `impl Registry`:

```rust
    pub fn insert_run(&self, run: &Run) -> Result<()> {
        self.conn.execute(
            "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                run.id, run.project_id, run.agent, run.prompt, run.base, run.branch, run.created_at
            ],
        )?;
        Ok(())
    }

    pub fn get_run(&self, id: &str) -> Result<Option<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, agent, prompt, base, branch, created_at FROM runs WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_run(row)?)),
            None => Ok(None),
        }
    }

    pub fn list_runs(&self, project_id: &str) -> Result<Vec<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, agent, prompt, base, branch, created_at
             FROM runs WHERE project_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([project_id], |row| Ok(row_to_run(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    pub fn delete_run(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM runs WHERE id = ?1", [id])?;
        Ok(())
    }
```

Add the free helper near `row_to_project`:

```rust
fn row_to_run(row: &rusqlite::Row) -> Result<Run> {
    Ok(Run {
        id: row.get(0)?,
        project_id: row.get(1)?,
        agent: row.get(2)?,
        prompt: row.get(3)?,
        base: row.get(4)?,
        branch: row.get(5)?,
        created_at: row.get(6)?,
    })
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-core --test registry`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/registry.rs crates/agency-core/tests/registry.rs
git commit -m "Persist runs in the registry"
```

---

### Task 5: `AppState` — run model (create/list/status/discard/rerun)

**Files:**
- Modify: `crates/agency-app/src/state.rs`
- Test: `crates/agency-app/tests/state.rs`

This task replaces the old in-memory `sessions: Mutex<HashMap<String, Session>>` + `start_task`/`send_input`/`task_status`/`stop_task` with the persisted run model backed by tmux. `worktree_path`, `merge_task`, `abort_merge_task` are re-pointed at run records. (Attach + resolver adapt in Task 6.)

**Interfaces:**
- Produces on `AppState`:
  - Fields: `registry: Mutex<Registry>`, `attaches: Mutex<HashMap<String, AgentHandle>>` (used in Task 6), `tmux: agency_core::tmux::Tmux`.
  - `struct RunInfo { pub id, pub project_id, pub agent, pub prompt, pub branch: String, pub status: agency_core::tmux::SessionStatus, pub added: u32, pub deleted: u32, pub files: u32 }` (derive `Debug, Clone, serde::Serialize`; `#[serde(rename_all = "camelCase")]`).
  - `fn new(db_path: &Path) -> Result<AppState>` — opens registry (seeding profiles as before), `tmux: Tmux::resolved()`, empty `attaches`.
  - `fn create_run(&self, project_id: &str, prompt: &str, agent: &str, base: &str) -> Result<RunInfo>`
  - `fn list_runs(&self, project_id: &str) -> Result<Vec<RunInfo>>`
  - `fn run_status(&self, id: &str) -> Result<agency_core::tmux::SessionStatus>`
  - `fn discard_run(&self, id: &str) -> Result<()>`
  - `fn rerun(&self, id: &str) -> Result<RunInfo>`
  - keep: `provider_env`, profile/settings methods, `repo_path_for`/`worktree_path` (re-pointed at runs), `merge_task`/`abort_merge_task`.
- Session name helper: `fn session_name(id: &str) -> String { format!("agency-{id}") }`.

- [ ] **Step 1: Update existing tests + add new ones**

Existing tests reference `start_task`, `send_input`, `task_status`, `stop_task`, `worktree_path`, `merge_task`. Replace the `start_task(...)`-based tests with `create_run(...)` equivalents, and delete tests for the removed `send_input`/`task_status`/`stop_task` (their behavior moves to attach/run_status/discard in Task 6 + this task). Keep/repoint `merge_task` and `worktree_path` tests to use `create_run`.

Add to `crates/agency-app/tests/state.rs`:

```rust
use agency_core::tmux::SessionStatus;

fn session_gone_or_cleanup(state: &AppState, id: &str) {
    // ensure no lingering tmux session after a test
    let _ = state.discard_run(id);
}

#[test]
fn create_run_persists_starts_session_and_lists() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo); // repo on `main` with a commit

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    // a profile that stays alive so the session is Running
    state.register_profile(AgentProfile {
        name: "stay".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "echo HI; sleep 3".into()],
        env: vec![],
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    let info = state.create_run(&project.id, "do it", "stay", "HEAD").unwrap();
    assert_eq!(info.agent, "stay");
    assert_eq!(info.branch, format!("agent/{}", info.id));

    // worktree exists
    let wt = state.worktree_path(&info.id).unwrap();
    assert!(wt.exists());

    // listed for the project, status Running
    let runs = state.list_runs(&project.id).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, info.id);
    // poll status until Running observed (session just started)
    let mut ok = false;
    for _ in 0..50 {
        if matches!(state.run_status(&info.id).unwrap(), SessionStatus::Running | SessionStatus::Exited{..}) { ok = true; break; }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(ok);

    // discard cleans up worktree + record + session
    state.discard_run(&info.id).unwrap();
    assert!(!wt.exists());
    assert_eq!(state.list_runs(&project.id).unwrap().len(), 0);
    session_gone_or_cleanup(&state, &info.id);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-app --test state`
Expected: FAIL — `create_run`/`run_status`/`discard_run` not found (and old removed-method references won't compile until you finish editing).

- [ ] **Step 3: Implement**

In `crates/agency-app/src/state.rs`:

Replace the struct + `new` to drop `sessions` and add `attaches` + `tmux`:

```rust
use agency_core::supervisor::AgentHandle;
use agency_core::tmux::{SessionStatus, Tmux};

pub struct AppState {
    registry: Mutex<Registry>,
    attaches: Mutex<HashMap<String, AgentHandle>>,
    tmux: Tmux,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub id: String,
    pub project_id: String,
    pub agent: String,
    pub prompt: String,
    pub branch: String,
    pub status: SessionStatus,
    pub added: u32,
    pub deleted: u32,
    pub files: u32,
}

fn session_name(id: &str) -> String {
    format!("agency-{id}")
}
```

In `new()`, keep the existing registry-open + profile seeding, and build the state as:

```rust
        Ok(AppState {
            registry: Mutex::new(registry),
            attaches: Mutex::new(HashMap::new()),
            tmux: Tmux::resolved(),
        })
```

Add the run methods (and a private `run_info` assembler). `repo_path_for` now reads the run's project; `worktree_path` derives from it:

```rust
    fn project_repo(&self, project_id: &str) -> Result<std::path::PathBuf> {
        let reg = self.registry.lock().unwrap();
        Ok(reg
            .get_project(project_id)?
            .ok_or_else(|| anyhow!("unknown project: {project_id}"))?
            .repo_path)
    }

    fn run_record(&self, id: &str) -> Result<agency_core::registry::Run> {
        let reg = self.registry.lock().unwrap();
        reg.get_run(id)?.ok_or_else(|| anyhow!("unknown run: {id}"))
    }

    pub fn worktree_path(&self, id: &str) -> Result<std::path::PathBuf> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        Ok(repo.join(".agency").join("worktrees").join(id))
    }

    fn run_info(&self, run: &agency_core::registry::Run) -> RunInfo {
        let name = session_name(&run.id);
        let status = self.tmux.session_status(&name).unwrap_or(SessionStatus::Gone);
        let wt = self
            .project_repo(&run.project_id)
            .ok()
            .map(|repo| repo.join(".agency").join("worktrees").join(&run.id));
        let stat = wt
            .filter(|p| p.exists())
            .and_then(|p| agency_core::git::diff_stat(&p, &run.base).ok())
            .unwrap_or(agency_core::git::DiffStat { added: 0, deleted: 0, files: 0 });
        RunInfo {
            id: run.id.clone(),
            project_id: run.project_id.clone(),
            agent: run.agent.clone(),
            prompt: run.prompt.clone(),
            branch: run.branch.clone(),
            status,
            added: stat.added,
            deleted: stat.deleted,
            files: stat.files,
        }
    }

    pub fn create_run(&self, project_id: &str, prompt: &str, agent: &str, base: &str) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {agent}"))?
        };
        let id = crate::state::new_task_id();
        let worktree = agency_core::worktree::WorktreeManager::new(repo).create(&id, base)?;

        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        let args = profile.render_args(prompt);

        self.tmux
            .start_session(&session_name(&id), &worktree.path, &profile.command, &args, &env)?;

        let run = agency_core::registry::Run {
            id: id.clone(),
            project_id: project_id.to_string(),
            agent: agent.to_string(),
            prompt: prompt.to_string(),
            base: base.to_string(),
            branch: worktree.branch.clone(),
            created_at: now_secs(),
        };
        self.registry.lock().unwrap().insert_run(&run)?;
        Ok(self.run_info(&run))
    }

    pub fn list_runs(&self, project_id: &str) -> Result<Vec<RunInfo>> {
        let runs = self.registry.lock().unwrap().list_runs(project_id)?;
        Ok(runs.iter().map(|r| self.run_info(r)).collect())
    }

    pub fn run_status(&self, id: &str) -> Result<SessionStatus> {
        Ok(self.tmux.session_status(&session_name(id)).unwrap_or(SessionStatus::Gone))
    }

    pub fn discard_run(&self, id: &str) -> Result<()> {
        self.attaches.lock().unwrap().remove(id);
        let run = self.run_record(id)?;
        self.tmux.kill_session(&session_name(id)).ok();
        if let Ok(repo) = self.project_repo(&run.project_id) {
            let _ = agency_core::worktree::WorktreeManager::new(repo).remove(id);
        }
        self.registry.lock().unwrap().delete_run(id)?;
        Ok(())
    }

    pub fn rerun(&self, id: &str) -> Result<RunInfo> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);
        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(&run.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {}", run.agent))?
        };
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        let args = profile.render_args(&run.prompt);
        self.tmux.kill_session(&session_name(id)).ok();
        self.tmux.start_session(&session_name(id), &worktree, &profile.command, &args, &env)?;
        Ok(self.run_info(&run))
    }
```

Add a `now_secs()` free helper near `new_task_id`:

```rust
fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
```

Update `merge_task`/`abort_merge_task`: they previously used `repo_path_for` (session-based). Re-point them to the run record:

```rust
    pub fn merge_task(&self, id: &str) -> Result<agency_core::merge::MergeOutcome> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let base = agency_core::merge::detect_base(&repo)?;
        agency_core::merge::merge(&repo, &run.branch, &base)
    }

    pub fn abort_merge_task(&self, id: &str) -> Result<()> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        agency_core::merge::abort_merge(&repo)
    }
```

Delete the old `start_task`, `send_input`, `task_status`, `stop_task`, `Session` struct, and the old `repo_path_for` (replaced by `project_repo`). Ensure `new_task_id` is still present (used by `create_run`); if it was previously private to this module keep it.

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-app --test state`
Expected: PASS (the new create_run test + repointed merge/worktree tests), zero warnings. Remove any tests referencing deleted methods.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app/src/state.rs crates/agency-app/tests/state.rs
git commit -m "Replace task model with persisted tmux-backed runs"
```

---

### Task 6: `AppState` — attach, input, preview; adapt resolver

**Files:**
- Modify: `crates/agency-app/src/state.rs`
- Test: `crates/agency-app/tests/state.rs`

**Interfaces:**
- Produces on `AppState`:
  - `fn attach_run<F>(&self, id: &str, on_output: F) -> Result<()>` where `F: Fn(Vec<u8>) + Send + 'static` — `tmux.attach(session_name(id), …)`, store handle in `attaches[id]`.
  - `fn detach_run(&self, id: &str)` — drop the attach handle.
  - `fn run_input(&self, id: &str, data: &[u8]) -> Result<()>` — write to `attaches[id]` (error if not attached).
  - `fn run_preview(&self, id: &str, lines: usize) -> Result<String>` — `tmux.capture(session_name(id), lines)`.
- `resolve_merge`/`resolver_input`/`resolver_status` adapt to the run record for repo/branch (the resolver itself keeps using the direct-PTY `spawn_agent` in the repo — acceptable for 7a since it's short-lived and in-focus; document this).

- [ ] **Step 1: Write the failing test**

Append to `crates/agency-app/tests/state.rs`:

```rust
#[test]
fn attach_streams_and_input_reaches_agent() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    state.register_profile(AgentProfile {
        name: "echoer".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "echo READY; read x; echo GOT:$x; sleep 3".into()],
        env: vec![],
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "echoer", "HEAD").unwrap();

    let buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let b = buf.clone();
    state.attach_run(&info.id, move |bytes| {
        b.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes));
    }).unwrap();

    let wait = |needle: &str| {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            if buf.lock().unwrap().contains(needle) { return true; }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        false
    };
    assert!(wait("READY"), "buf: {:?}", buf.lock().unwrap());
    state.run_input(&info.id, b"ping\n").unwrap();
    assert!(wait("GOT:ping"), "buf: {:?}", buf.lock().unwrap());

    // preview also reflects the pane
    let prev = state.run_preview(&info.id, 50).unwrap();
    assert!(prev.contains("READY"));

    state.detach_run(&info.id);
    state.discard_run(&info.id).unwrap();
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-app --test state`
Expected: FAIL — `attach_run`/`run_input`/`run_preview` not found.

- [ ] **Step 3: Implement**

Add to `impl AppState`:

```rust
    pub fn attach_run<F>(&self, id: &str, on_output: F) -> Result<()>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        let handle = self.tmux.attach(&session_name(id), on_output)?;
        self.attaches.lock().unwrap().insert(id.to_string(), handle);
        Ok(())
    }

    pub fn detach_run(&self, id: &str) {
        self.attaches.lock().unwrap().remove(id);
    }

    pub fn run_input(&self, id: &str, data: &[u8]) -> Result<()> {
        let attaches = self.attaches.lock().unwrap();
        let handle = attaches.get(id).ok_or_else(|| anyhow!("run not attached: {id}"))?;
        handle.write_input(data)
    }

    pub fn run_preview(&self, id: &str, lines: usize) -> Result<String> {
        self.tmux.capture(&session_name(id), lines)
    }
```

Adapt `resolve_merge` (and keep `resolver_input`/`resolver_status` as-is, they use the `resolvers` map): change the repo/branch/conflicts resolution to use the run record instead of the removed session lookup. Replace the body's repo/branch acquisition with:

```rust
        let run = self.run_record(task_id)?;
        let repo = self.project_repo(&run.project_id)?;
        let branch = run.branch.clone();
        let base = agency_core::merge::detect_base(&repo).unwrap_or_else(|_| "main".to_string());
```

(The rest of `resolve_merge` — building the prompt from the skill, spawning the resolver via `spawn_agent` in `repo`, storing in `resolvers` — is unchanged.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-app` (full crate) and `cargo build -p agency-app`.
Expected: PASS, zero warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app/src/state.rs crates/agency-app/tests/state.rs
git commit -m "Add run attach/input/preview and adapt the resolver to runs"
```

---

### Task 7: Tauri command layer for runs

**Files:**
- Modify: `crates/agency-app/src/commands.rs`, `crates/agency-app/src/lib.rs`

**Interfaces:**
- Replace the old `start_task`/`send_input`/`task_status`/`stop_task` commands with: `create_run(project_id, prompt, agent, base) -> RunInfo`, `list_runs(project_id) -> Vec<RunInfo>`, `run_preview(id, lines) -> String`, `attach_run(id, on_chunk: Channel<TerminalChunk>) -> ()`, `detach_run(id) -> ()`, `run_input(id, data: String) -> ()`, `run_status(id) -> SessionStatus`, `discard_run(id) -> ()`, `rerun(id) -> RunInfo`.
- Keep `merge_task`/`abort_merge_task`/`resolve_merge`/`resolver_input`/`resolver_status`/git_*/profile/settings commands; `merge_task`/`abort_merge_task`/`resolve_merge` now take an `id` (the run id) — same param name `task_id` is fine.
- Register the new set; remove the deleted commands from `generate_handler!`.

- [ ] **Step 1: Implement the command wrappers**

In `crates/agency-app/src/commands.rs`, remove `start_task`/`send_input`/`task_status`/`stop_task` and add:

```rust
use crate::state::RunInfo;
use agency_core::tmux::SessionStatus;

#[tauri::command]
pub fn create_run(
    state: State<'_, AppState>,
    project_id: String,
    prompt: String,
    agent: String,
    base: String,
) -> Result<RunInfo, String> {
    let base = if base.is_empty() { "HEAD".to_string() } else { base };
    state.create_run(&project_id, &prompt, &agent, &base).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_runs(state: State<'_, AppState>, project_id: String) -> Result<Vec<RunInfo>, String> {
    state.list_runs(&project_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn run_preview(state: State<'_, AppState>, id: String, lines: usize) -> Result<String, String> {
    state.run_preview(&id, lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn attach_run(
    state: State<'_, AppState>,
    id: String,
    on_chunk: Channel<TerminalChunk>,
) -> Result<(), String> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    state
        .attach_run(&id, move |bytes| {
            let _ = on_chunk.send(TerminalChunk { b64: STANDARD.encode(&bytes) });
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn detach_run(state: State<'_, AppState>, id: String) {
    state.detach_run(&id);
}

#[tauri::command]
pub fn run_input(state: State<'_, AppState>, id: String, data: String) -> Result<(), String> {
    state.run_input(&id, data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn run_status(state: State<'_, AppState>, id: String) -> Result<SessionStatus, String> {
    state.run_status(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn discard_run(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.discard_run(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rerun(state: State<'_, AppState>, id: String) -> Result<RunInfo, String> {
    state.rerun(&id).map_err(|e| e.to_string())
}
```

Update `merge_task`/`abort_merge_task`/`resolve_merge` signatures only if they referenced removed types — they take `task_id: String` and call the (re-pointed) `AppState` methods; no body change beyond what Task 5/6 did.

- [ ] **Step 2: Update registration**

In `crates/agency-app/src/lib.rs` `generate_handler!`: remove `start_task, send_input, task_status, stop_task`; add `create_run, list_runs, run_preview, attach_run, detach_run, run_input, run_status, discard_run, rerun`. Keep all git_*/merge/resolver/profile/settings commands.

- [ ] **Step 3: Build + test**

Run: `cargo build -p agency-app` (warning-free) and `cargo test --workspace`.
Expected: clean; all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs
git commit -m "Add run command layer; retire task commands"
```

---

### Task 8: Frontend — run API + RunStore

**Files:**
- Modify: `ui/src/api.ts`
- Create: `ui/src/store/runs.tsx`

**Interfaces:**
- `api.ts`: types `SessionStatus = { state: "running" } | { state: "exited"; code: number } | { state: "gone" }`, `RunInfo { id; projectId; agent; prompt; branch; status: SessionStatus; added; deleted; files }`; wrappers `createRun(projectId, prompt, agent, base)`, `listRuns(projectId)`, `runPreview(id, lines)`, `attachRun(id, onBytes)` (Channel like the old startTask), `detachRun(id)`, `runInput(id, data)`, `runStatus(id)`, `discardRun(id)`, `rerun(id)`. Remove the old `startTask`/`sendInput`/`taskStatus`/`stopTask`.
- `store/runs.tsx`: a React context exposing `{ runs, selectedProjectId, setSelectedProject, view, setView ('grid'|'focus'), focusedRunId, setFocusedRun, refreshRuns() }`. Polls `listRuns(selectedProjectId)` on an interval (~1.5s) and on demand.

- [ ] **Step 1: API wrappers**

In `ui/src/api.ts`, remove `startTask`/`sendInput`/`taskStatus`/`stopTask`, and add:

```ts
export type SessionStatus =
  | { state: "running" }
  | { state: "exited"; code: number }
  | { state: "gone" };

export interface RunInfo {
  id: string;
  projectId: string;
  agent: string;
  prompt: string;
  branch: string;
  status: SessionStatus;
  added: number;
  deleted: number;
  files: number;
}

export const createRun = (projectId: string, prompt: string, agent: string, base: string) =>
  invoke<RunInfo>("create_run", { projectId, prompt, agent, base });
export const listRuns = (projectId: string) => invoke<RunInfo[]>("list_runs", { projectId });
export const runPreview = (id: string, lines: number) =>
  invoke<string>("run_preview", { id, lines });
export const detachRun = (id: string) => invoke<void>("detach_run", { id });
export const runInput = (id: string, data: string) => invoke<void>("run_input", { id, data });
export const runStatus = (id: string) => invoke<SessionStatus>("run_status", { id });
export const discardRun = (id: string) => invoke<void>("discard_run", { id });
export const rerun = (id: string) => invoke<RunInfo>("rerun", { id });

export function attachRun(id: string, onBytes: (b: Uint8Array) => void): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (m) => onBytes(b64ToBytes(m.b64));
  return invoke<void>("attach_run", { id, onChunk });
}
```

- [ ] **Step 2: RunStore**

Create `ui/src/store/runs.tsx`:

```tsx
import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { RunInfo, listRuns } from "../api";

type View = "grid" | "focus";

interface RunStore {
  runs: RunInfo[];
  selectedProjectId: string | null;
  setSelectedProject: (id: string | null) => void;
  view: View;
  setView: (v: View) => void;
  focusedRunId: string | null;
  setFocusedRun: (id: string | null) => void;
  refreshRuns: () => Promise<void>;
}

const Ctx = createContext<RunStore | null>(null);

export function RunStoreProvider({ children }: { children: React.ReactNode }) {
  const [runs, setRuns] = useState<RunInfo[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [view, setView] = useState<View>("grid");
  const [focusedRunId, setFocusedRun] = useState<string | null>(null);
  const projectRef = useRef<string | null>(null);
  projectRef.current = selectedProjectId;

  const refreshRuns = useCallback(async () => {
    const pid = projectRef.current;
    if (!pid) {
      setRuns([]);
      return;
    }
    try {
      setRuns(await listRuns(pid));
    } catch {
      /* ignore transient errors */
    }
  }, []);

  function setSelectedProject(id: string | null) {
    setSelectedProjectId(id);
    setView("grid");
    setFocusedRun(null);
  }

  useEffect(() => {
    refreshRuns();
    const t = window.setInterval(refreshRuns, 1500);
    return () => window.clearInterval(t);
  }, [selectedProjectId, refreshRuns]);

  return (
    <Ctx.Provider
      value={{ runs, selectedProjectId, setSelectedProject, view, setView, focusedRunId, setFocusedRun, refreshRuns }}
    >
      {children}
    </Ctx.Provider>
  );
}

export function useRuns(): RunStore {
  const v = useContext(Ctx);
  if (!v) throw new Error("useRuns outside RunStoreProvider");
  return v;
}
```

- [ ] **Step 3: Build**

Run: `pnpm --dir ui build`
Expected: type-checks (store not yet mounted). Note: removing old api functions will break imports in `TaskBoard`/`TerminalPane`; those are deleted in Task 12. To keep this task building in isolation, you may temporarily leave `TaskBoard`/`TerminalPane` referencing removed APIs **only if** the build still passes — if it does not, proceed directly to the frontend tasks (8→12) as a group and run the build at the end of Task 12. (Recommended: treat Tasks 8–12 as a frontend group; commit each but run the full `pnpm build` green at Task 12.)

- [ ] **Step 4: Commit**

```bash
git add ui/src/api.ts ui/src/store/runs.tsx
git commit -m "Add run API wrappers and RunStore"
```

---

### Task 9: Frontend — app shell (TitleBar, StatusBar, ProjectTree)

**Files:**
- Create: `ui/src/components/TitleBar.tsx`, `ui/src/components/StatusBar.tsx`, `ui/src/components/ProjectTree.tsx`
- Replace: `ui/src/App.tsx`
- Modify: `ui/src/styles.css`

**Design reference:** mockup top chrome (lines ~29–46), sidebar/project tree (~51–96), status bar. Use tokens from `theme.css`.

**Interfaces:**
- `TitleBar({ onToggleSidebar, onOpenSettings })` — 40px bar: sidebar-toggle icon, blue rotated-square logo, "Agency", a visual-only ⌘K search box (non-functional in 7a), Settings cog.
- `StatusBar({ projectName })` — 27px: left project name (or "no project"), right `⌘N new · ⌘G source · ⌘↵ approve` hint text.
- `ProjectTree({ selectedId, onSelect })` — collapsible project list; each project row has a chevron to expand its runs (via `listRuns` per expanded project), status dots, click selects the project; the existing add-project form (name + Browse + Add) at the bottom; reuse `addProject`/`listProjects`/`removeProject` from api.
- `App` — wraps everything in `RunStoreProvider`; layout = `<TitleBar/>` + `<div class="body"><ProjectTree/> <main>…</main></div>` + `<StatusBar/>`; holds `sidebarOpen`, `showSettings`; `main` renders `AgentsView` (Task 10/11) when a project is selected else the empty state; `Settings`/`MergeModal` overlays as today.

- [ ] **Step 1: Create the three shell components**

`ui/src/components/TitleBar.tsx`:

```tsx
export default function TitleBar({
  onToggleSidebar,
  onOpenSettings,
}: {
  onToggleSidebar: () => void;
  onOpenSettings: () => void;
}) {
  return (
    <header className="titlebar">
      <div className="titlebar-left">
        <button className="icon-btn no-drag" title="Toggle sidebar" onClick={onToggleSidebar}>
          ☰
        </button>
        <span className="logo-mark" />
        <span className="app-name">Agency</span>
      </div>
      <div className="search-box no-drag" aria-disabled>
        <span>⌕</span>
        <span className="search-ph">Search projects, tasks, files…</span>
        <span className="kbd">⌘K</span>
      </div>
      <div className="titlebar-right">
        <button className="icon-btn no-drag" title="Settings" onClick={onOpenSettings}>
          ⚙
        </button>
      </div>
    </header>
  );
}
```

`ui/src/components/StatusBar.tsx`:

```tsx
export default function StatusBar({ projectName }: { projectName: string | null }) {
  return (
    <footer className="statusbar">
      <span>{projectName ?? "no project"}</span>
      <span className="kbd-hints">⌘N new · ⌘G source · ⌘↵ approve</span>
    </footer>
  );
}
```

`ui/src/components/ProjectTree.tsx`:

```tsx
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Project, RunInfo, addProject, listProjects, listRuns, removeProject } from "../api";

function statusClass(s: RunInfo["status"]): string {
  if (s.state === "running") return "running";
  if (s.state === "crashed" as unknown) return "crashed";
  return "exited";
}

export default function ProjectTree({
  selectedId,
  onSelect,
}: {
  selectedId: string | null;
  onSelect: (p: Project) => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [expanded, setExpanded] = useState<Record<string, RunInfo[] | undefined>>({});
  const [name, setName] = useState("");
  const [repoPath, setRepoPath] = useState("");

  async function refresh() {
    setProjects(await listProjects());
  }
  useEffect(() => {
    refresh();
  }, []);

  async function toggle(p: Project) {
    setExpanded((e) => ({ ...e, [p.id]: e[p.id] ? undefined : [] }));
    if (!expanded[p.id]) {
      try {
        const runs = await listRuns(p.id);
        setExpanded((e) => ({ ...e, [p.id]: runs }));
      } catch {
        /* ignore */
      }
    }
  }

  async function handleBrowse() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel === "string") {
      setRepoPath(sel);
      if (!name.trim()) setName(sel.split("/").filter(Boolean).pop() ?? "");
    }
  }
  async function handleAdd() {
    if (!name.trim() || !repoPath.trim()) return;
    await addProject(name.trim(), repoPath.trim());
    setName("");
    setRepoPath("");
    await refresh();
  }

  return (
    <aside className="tree">
      <h2 className="tree-head">Projects</h2>
      <ul className="tree-list">
        {projects.map((p) => (
          <li key={p.id}>
            <div className={`tree-row ${p.id === selectedId ? "selected" : ""}`} onClick={() => onSelect(p)}>
              <span className="chev" onClick={(e) => { e.stopPropagation(); toggle(p); }}>
                {expanded[p.id] !== undefined ? "▾" : "▸"}
              </span>
              <span className="tree-name">{p.name}</span>
              <button className="ghost-x" onClick={(e) => { e.stopPropagation(); removeProject(p.id).then(refresh); }}>×</button>
            </div>
            {expanded[p.id] !== undefined && (
              <ul className="tree-children">
                {(expanded[p.id] ?? []).map((r) => (
                  <li key={r.id} className="tree-child">
                    <span className={`dot ${statusClass(r.status)}`} />
                    <span className="tree-child-name">{r.agent}: {r.prompt || r.branch}</span>
                  </li>
                ))}
              </ul>
            )}
          </li>
        ))}
      </ul>
      <div className="add-project">
        <input placeholder="name" value={name} onChange={(e) => setName(e.target.value)} />
        <div className="repo-row">
          <input placeholder="/path/to/repo" value={repoPath} onChange={(e) => setRepoPath(e.target.value)} />
          <button className="browse" onClick={handleBrowse}>Browse…</button>
        </div>
        <button onClick={handleAdd}>Add project</button>
      </div>
    </aside>
  );
}
```

(Note: `statusClass` only needs `running`/`exited` — `gone` maps to `exited`. Simplify the helper to avoid the cast: `return s.state === "running" ? "running" : "exited";`.)

- [ ] **Step 2: Replace App**

`ui/src/App.tsx`:

```tsx
import { useState } from "react";
import { RunStoreProvider, useRuns } from "./store/runs";
import TitleBar from "./components/TitleBar";
import StatusBar from "./components/StatusBar";
import ProjectTree from "./components/ProjectTree";
import AgentsView from "./components/AgentsView";
import Settings from "./components/Settings";
import { Project } from "./api";

function Shell() {
  const { selectedProjectId, setSelectedProject } = useRuns();
  const [project, setProject] = useState<Project | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [showSettings, setShowSettings] = useState(false);

  function selectProject(p: Project) {
    setProject(p);
    setSelectedProject(p.id);
  }

  return (
    <div className="shell">
      <TitleBar onToggleSidebar={() => setSidebarOpen((s) => !s)} onOpenSettings={() => setShowSettings(true)} />
      <div className="body">
        {sidebarOpen && <ProjectTree selectedId={selectedProjectId} onSelect={selectProject} />}
        {project ? <AgentsView project={project} /> : <main className="board empty">Select or add a project to begin.</main>}
      </div>
      <StatusBar projectName={project?.name ?? null} />
      {showSettings && <Settings onClose={() => setShowSettings(false)} />}
    </div>
  );
}

export default function App() {
  return (
    <RunStoreProvider>
      <Shell />
    </RunStoreProvider>
  );
}
```

- [ ] **Step 3: Styles + build**

Append shell/tree styles to `ui/src/styles.css` using tokens (heights `var(--titlebar-h)`/`var(--statusbar-h)`, sidebar `var(--sidebar-w)`, surfaces `--mantle`/`--crust`, accents). Match the mockup's top chrome + tree. Key rules: `.shell{display:flex;flex-direction:column;height:100%}`, `.titlebar{height:var(--titlebar-h);…;-webkit-app-region:drag}`, `.no-drag{-webkit-app-region:no-drag}`, `.logo-mark{width:15px;height:15px;background:var(--blue);border-radius:4px;transform:rotate(45deg)}`, `.body{flex:1;display:flex;min-height:0}`, `.tree{width:var(--sidebar-w);…}`, `.statusbar{height:var(--statusbar-h);…;justify-content:space-between}`, `.dot.running{…pulse}` (reuse existing dot rules).

Run: `pnpm --dir ui build` — it will fail to resolve `AgentsView` until Task 10. That's expected within the frontend group; continue.

- [ ] **Step 4: Commit**

```bash
git add ui/src/App.tsx ui/src/components/TitleBar.tsx ui/src/components/StatusBar.tsx ui/src/components/ProjectTree.tsx ui/src/styles.css
git commit -m "Add app shell: title bar, status bar, project tree"
```

---

### Task 10: Frontend — Agents Grid + tile + New task

**Files:**
- Create: `ui/src/components/AgentsView.tsx`, `ui/src/components/AgentTile.tsx`, `ui/src/components/NewTaskForm.tsx`
- Modify: `ui/src/styles.css`

**Design reference:** mockup content header (~99–124), grid tiles (~131–308). Tokens.

**Interfaces:**
- `AgentsView({ project })` — content header (Agents | Source Control segmented nav; in Agents: Grid/Focus toggle + "+ New task"); body switches on `useRuns().view`: Grid (Task 10), Focus (Task 11), or `GitPanel` when the Source-Control nav is active (local `tab` state `'agents'|'source'`). Hosts the `NewTaskForm` (shown when "+ New task" clicked).
- `AgentTile({ run })` — status dot · prompt title · agent badge (`badge.<agent>`) · branch · diff stat (`+added −deleted · files`) · text preview (poll `runPreview(run.id, 12)` ~1.5s) · footer (status text + Approve→ when there are changes / Discard). Click opens Focus (`setFocusedRun(run.id); setView('focus')`).
- `NewTaskForm({ project, onDone })` — profile `<select>` (from `listProfiles`) + prompt textarea + Start → `createRun(project.id, prompt, profile, "HEAD")` then `refreshRuns()` + `onDone()`.

- [ ] **Step 1: NewTaskForm**

```tsx
import { useEffect, useState } from "react";
import { AgentProfile, Project, createRun, listProfiles } from "../api";
import { useRuns } from "../store/runs";

export default function NewTaskForm({ project, onDone }: { project: Project; onDone: () => void }) {
  const { refreshRuns } = useRuns();
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [agent, setAgent] = useState("claude");
  const [prompt, setPrompt] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    listProfiles().then((ps) => {
      setProfiles(ps);
      if (!ps.find((p) => p.name === "claude") && ps[0]) setAgent(ps[0].name);
    });
  }, []);

  async function start() {
    try {
      await createRun(project.id, prompt, agent, "HEAD");
      await refreshRuns();
      onDone();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="new-task card">
      <h3>New agent</h3>
      {error && <div className="git-error">{error}</div>}
      <label>Agent type</label>
      <select value={agent} onChange={(e) => setAgent(e.target.value)}>
        {profiles.map((p) => <option key={p.name} value={p.name}>{p.name}</option>)}
      </select>
      <label>Prompt</label>
      <textarea value={prompt} onChange={(e) => setPrompt(e.target.value)} placeholder="What should this agent do?" />
      <div className="row-actions">
        <button onClick={start}>Start agent</button>
        <button className="ghost" onClick={onDone}>Cancel</button>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: AgentTile**

```tsx
import { useEffect, useState } from "react";
import { RunInfo, runPreview } from "../api";
import { useRuns } from "../store/runs";

function badgeClass(agent: string): string {
  if (agent === "claude") return "badge claude";
  if (agent === "pi") return "badge pi";
  if (agent === "hermes") return "badge hermes";
  return "badge";
}
function statusLabel(s: RunInfo["status"]): { cls: string; text: string } {
  if (s.state === "running") return { cls: "running", text: "running" };
  if (s.state === "exited") return { cls: "exited", text: `exited (${s.code})` };
  return { cls: "exited", text: "gone" };
}

export default function AgentTile({ run }: { run: RunInfo }) {
  const { setFocusedRun, setView } = useRuns();
  const [preview, setPreview] = useState("");

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const p = await runPreview(run.id, 12);
        if (alive) setPreview(p);
      } catch {
        /* ignore */
      }
    };
    tick();
    const t = window.setInterval(tick, 1500);
    return () => { alive = false; window.clearInterval(t); };
  }, [run.id]);

  const st = statusLabel(run.status);
  return (
    <div className="tile" onClick={() => { setFocusedRun(run.id); setView("focus"); }}>
      <div className="tile-head">
        <span className={`dot ${st.cls}`} />
        <span className="tile-title">{run.prompt || run.branch}</span>
        <span className={badgeClass(run.agent)}>{run.agent}</span>
      </div>
      <div className="tile-meta">
        <code>{run.branch}</code>
        <span className="diffstat"><span className="add">+{run.added}</span> <span className="del">−{run.deleted}</span> · {run.files}f</span>
      </div>
      <pre className="tile-preview">{preview}</pre>
      <div className="tile-foot">{st.text}</div>
    </div>
  );
}
```

- [ ] **Step 3: AgentsView**

```tsx
import { useState } from "react";
import { Project } from "../api";
import { useRuns } from "../store/runs";
import AgentTile from "./AgentTile";
import AgentFocus from "./AgentFocus";
import NewTaskForm from "./NewTaskForm";
import GitPanel from "./GitPanel";

export default function AgentsView({ project }: { project: Project }) {
  const { runs, view, setView, focusedRunId } = useRuns();
  const [tab, setTab] = useState<"agents" | "source">("agents");
  const [newOpen, setNewOpen] = useState(false);

  return (
    <main className="agents">
      <div className="content-head">
        <div className="seg">
          <button className={tab === "agents" ? "on" : ""} onClick={() => setTab("agents")}>▦ Agents</button>
          <button className={tab === "source" ? "on" : ""} onClick={() => setTab("source")}>⎇ Source Control</button>
        </div>
        {tab === "agents" && (
          <div className="seg">
            <button className={view === "grid" ? "on" : ""} onClick={() => setView("grid")}>▦ Grid</button>
            <button className={view === "focus" ? "on" : ""} onClick={() => setView("focus")}>▭ Focus</button>
          </div>
        )}
        <div className="spacer" />
        {tab === "agents" && <button onClick={() => setNewOpen(true)}>+ New task</button>}
      </div>

      {tab === "source" && <div className="source-wrap">{focusedRunId ? <GitPanel taskId={focusedRunId} /> : <div className="board empty">Open an agent to review its changes.</div>}</div>}

      {tab === "agents" && view === "grid" && (
        <div className="grid">
          {runs.length === 0 && <div className="board empty">No agents yet — start one with “+ New task”.</div>}
          {runs.map((r) => <AgentTile key={r.id} run={r} />)}
        </div>
      )}
      {tab === "agents" && view === "focus" && <AgentFocus />}

      {newOpen && <div className="settings-overlay"><NewTaskForm project={project} onDone={() => setNewOpen(false)} /></div>}
    </main>
  );
}
```

- [ ] **Step 4: Styles** — add `.agents`, `.content-head`, `.seg` (segmented control per §07), `.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(340px,1fr));gap:14px;…}`, `.tile` (card, border by status), `.tile-preview{font-family:var(--mono);…;mask/ fade at bottom}`, `.badge`/`.diffstat` colors, to `ui/src/styles.css`, using tokens and the mockup §07 tile spec.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/AgentsView.tsx ui/src/components/AgentTile.tsx ui/src/components/NewTaskForm.tsx ui/src/styles.css
git commit -m "Add Agents grid, tiles, and new-task form"
```

---

### Task 11: Frontend — Agents Focus + FocusTerminal + rail

**Files:**
- Create: `ui/src/components/AgentFocus.tsx`, `ui/src/components/FocusTerminal.tsx`
- Modify: `ui/src/styles.css`

**Design reference:** mockup focus (~311–445).

**Interfaces:**
- `FocusTerminal({ runId })` — xterm.js bound to `attachRun(runId, …)`; on mount seeds from `runPreview(runId, 200)` then attaches live; input → `runInput`; uses the `xtermTheme`; on unmount `detachRun(runId)` + dispose. (This is the `TerminalPane` logic refactored to the run API; reuse its fit/ResizeObserver/focus and listener-dispose patterns.)
- `AgentFocus()` — reads `useRuns()`; left **agent rail** lists `runs` (click sets `focusedRunId`); main = header (badge/branch/status + Approve→ opening `MergeModal` for the focused run + Discard) + `<FocusTerminal runId={focusedRunId}/>`. If no `focusedRunId`, show a prompt to pick one.

- [ ] **Step 1: FocusTerminal** (refactor of TerminalPane)

```tsx
import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { attachRun, detachRun, runInput, runPreview } from "../api";
import { xtermTheme } from "../lib/xtermTheme";

export default function FocusTerminal({ runId }: { runId: string }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const container = ref.current;
    if (!container) return;
    const term = new Terminal({ convertEol: true, fontSize: 13, cursorBlink: true, theme: xtermTheme });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(container);
    const doFit = () => { try { fit.fit(); } catch { /* not laid out */ } };
    requestAnimationFrame(() => { doFit(); term.focus(); });
    const ro = new ResizeObserver(doFit);
    ro.observe(container);

    let disposed = false;
    let onData: { dispose(): void } | undefined;
    runPreview(runId, 200).then((seed) => { if (!disposed && seed) term.write(seed.endsWith("\n") ? seed : seed + "\n"); });
    attachRun(runId, (bytes) => term.write(bytes)).then(() => {
      if (disposed) return;
      onData = term.onData((d) => runInput(runId, d));
    });

    return () => {
      disposed = true;
      ro.disconnect();
      onData?.dispose();
      detachRun(runId);
      term.dispose();
    };
  }, [runId]);

  return <div className="terminal focus-term" ref={ref} />;
}
```

- [ ] **Step 2: AgentFocus**

```tsx
import { useState } from "react";
import { useRuns } from "../store/runs";
import FocusTerminal from "./FocusTerminal";
import MergeModal from "./MergeModal";

function badgeClass(a: string) {
  return ["claude", "pi", "hermes"].includes(a) ? `badge ${a}` : "badge";
}

export default function AgentFocus() {
  const { runs, focusedRunId, setFocusedRun } = useRuns();
  const [showMerge, setShowMerge] = useState(false);
  const [railOpen, setRailOpen] = useState(true);
  const focused = runs.find((r) => r.id === focusedRunId) ?? null;

  return (
    <div className="focus">
      {railOpen ? (
        <div className="rail">
          <div className="rail-head">
            <span>Agents</span>
            <button className="icon-btn" onClick={() => setRailOpen(false)}>«</button>
          </div>
          {runs.map((r) => (
            <button key={r.id} className={`rail-row ${r.id === focusedRunId ? "on" : ""}`} onClick={() => setFocusedRun(r.id)}>
              <span className={`dot ${r.status.state === "running" ? "running" : "exited"}`} />
              <span className="rail-name">{r.agent}: {r.prompt || r.branch}</span>
            </button>
          ))}
        </div>
      ) : (
        <button className="rail-stub icon-btn" onClick={() => setRailOpen(true)}>»</button>
      )}

      <div className="focus-main">
        {focused ? (
          <>
            <div className="focus-head">
              <span className={badgeClass(focused.agent)}>{focused.agent}</span>
              <code>{focused.branch}</code>
              <span className="spacer" />
              <button onClick={() => setShowMerge(true)}>Approve →</button>
            </div>
            <FocusTerminal key={focused.id} runId={focused.id} />
            {showMerge && <MergeModal taskId={focused.id} onClose={() => setShowMerge(false)} />}
          </>
        ) : (
          <div className="board empty">Select an agent from the rail.</div>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Styles** — add `.focus{display:flex;flex:1;min-height:0}`, `.rail{width:var(--focus-rail-w,312px);…}`, `.rail-row`, `.focus-main{flex:1;display:flex;flex-direction:column;min-height:0}`, `.focus-head`, `.focus-term{flex:1;min-height:0}` to `ui/src/styles.css` (tokens; mockup focus spec). Add `--focus-rail-w:312px` to `theme.css` if you prefer a token.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/AgentFocus.tsx ui/src/components/FocusTerminal.tsx ui/src/styles.css
git commit -m "Add Agents focus view and live focus terminal"
```

---

### Task 12: Frontend — wire-up, delete dead components, green build

**Files:**
- Delete: `ui/src/components/ProjectSidebar.tsx`, `ui/src/components/TaskBoard.tsx`, `ui/src/components/TerminalPane.tsx`
- Modify: `ui/src/styles.css` (remove now-dead rules for deleted components if obviously unused — optional)

- [ ] **Step 1: Delete superseded components**

```bash
git rm ui/src/components/ProjectSidebar.tsx ui/src/components/TaskBoard.tsx ui/src/components/TerminalPane.tsx
```

Grep for any remaining imports of them and remove (there should be none after Tasks 9–11):

Run: `grep -rn "ProjectSidebar\|TaskBoard\|TerminalPane" ui/src` — expect no matches.

- [ ] **Step 2: Full green build + tests**

Run: `pnpm --dir ui build` — `tsc && vite build` must pass with no TS errors.
Run: `pnpm --dir ui test` — vitest green (the `b64` test still passes).
Run: `cargo test --workspace` — all green.

If `tsc` flags unused exports or types from the API refactor, fix them (e.g., remove leftover `StatusDto` if no longer used, or keep if MergeModal/resolver still use it).

- [ ] **Step 3: Manual smoke (record, human-run)**

`cd crates/agency-app && cargo tauri dev`: add a project, expand it in the tree, **+ New task** → pick `shell` → a tile appears and its terminal preview fills; click the tile → Focus shows a live terminal you can type into; quit & relaunch → the run is still listed (Running if its tmux session survived). Note results in the report; visual fidelity vs. `Agency v2.dc.html` is a human check.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "Wire up multi-agent shell; remove superseded components"
```

---

## Self-Review

**Spec coverage (7a):**
- tmux session module (start/status/capture/attach/kill, configurable bin) → Tasks 1–2. ✓
- diff stat → Task 3. ✓
- runs persistence → Task 4. ✓
- run model (create/list/status/discard/rerun) + RunInfo + adapt merge/worktree → Task 5. ✓
- attach/input/preview + adapt resolver → Task 6. ✓
- run command layer → Task 7. ✓
- run API + store → Task 8. ✓
- shell (title/status/tree) → Task 9. ✓
- grid + tile + new-task (pick agent → spins up) → Task 10. ✓
- focus + live terminal + rail → Task 11. ✓
- Source Control routes to GitPanel; Settings/MergeModal reused; dead components removed → Tasks 10/12. ✓
- Deferred (7b/7c): source-control redesign, per-hunk, git-review panel, polished modals, settings reskin, ⌘K/shortcuts, tmux binary bundling. Documented.

**Placeholder scan:** Backend tasks have complete code + tests. Frontend tasks have complete component code; styling steps reference the authoritative mockup/tokens (as in Phase 6) rather than restating every rule — each names the exact selectors/values to add. The Task 8 build note explicitly frames Tasks 8–12 as a frontend group that goes green at Task 12 (so "build fails until Task 10/11" is expected, not a gap).

**Type consistency:** `SessionStatus` (tagged `state`/camelCase) is identical across tmux.rs ↔ RunInfo ↔ api.ts ↔ components. `RunInfo`/`Run` field names match Rust↔TS. `create_run/list_runs/run_preview/attach_run/detach_run/run_input/run_status/discard_run/rerun` names match across state ↔ commands ↔ api. `worktree_path`/`merge_task` re-pointed consistently. `MergeModal` still takes `taskId` (a run id) — unchanged.

**Notes for the executor:**
- Requires `tmux` and `git` on PATH (present). tmux tests poll with timeouts to absorb startup jitter.
- The riskiest tasks are 5–7 (engine refactor); they have the most test coverage and should be reviewed carefully.
- Treat Tasks 8–12 as a frontend group: commit each, but the full `pnpm build` is expected to go green at Task 12.
- The resolver intentionally stays on the direct-PTY `spawn_agent` for 7a (short-lived, in-focus); moving it to tmux is a later option.
