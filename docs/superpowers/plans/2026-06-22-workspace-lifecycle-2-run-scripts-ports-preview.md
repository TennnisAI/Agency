# Workspace Lifecycle — Plan 2: Run Scripts + Ports + Preview

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let each workspace launch its app/dev-server from the configured `[scripts].run` command on an allocated port, in a dedicated terminal, with an in-app preview and an open-in-browser button.

**Architecture:** Each run gets a contiguous port block (`base + slot*block_size`) allocated at creation and persisted on the run record (`port_base`), exposed as `AGENCY_PORT`. The run command executes in a **separate** tmux session `agency-run-<id>` (independent of the agent session `agency-<id>`), streamed to a new "Run" tab via the same attach/Channel path the agent terminal uses. The tab also embeds an `<iframe>` to `http://localhost:$AGENCY_PORT` and an "Open in browser" button (Tauri opener plugin).

**Tech Stack:** Rust (agency-core, agency-app/Tauri), `rusqlite`, `tmux`, `tauri-plugin-opener` (new), React/TypeScript, `@tauri-apps/plugin-opener` (new), `xterm.js`.

**Spec:** `docs/superpowers/specs/2026-06-22-workspace-lifecycle-design.md` (Feature 2). **Builds on Plan 1** (config + scripts modules are merged to `main`).

## Global Constraints

- Rust edition 2021. No AI/Claude attribution in any commit message (hard rule).
- Default port base = `5200`, block_size = `10` (already in `agency_core::config::PortsConfig`; do not redefine).
- `AGENCY_PORT = ports.base + slot*ports.block_size`, where `slot` is the **lowest free** non-negative integer across **all** runs that currently hold a `port_base` (port space is machine-global).
- Port is allocated in `create_run` (so the setup, agent, and run processes all see the same `AGENCY_PORT`), persisted on the run record, and freed when the run record is deleted (discard). `rerun` reuses the run's stored `port_base`.
- The run command runs in tmux session `agency-run-<id>` via `sh -lc '<run>'`. Agent session naming (`agency-<id>`) is unchanged.
- `run_mode = "nonconcurrent"` stops every other run-script session before starting this one; `"concurrent"` (default) does not.
- All new streaming commands mirror the existing agent pattern exactly: `Channel<TerminalChunk>` with base64 chunks, `detach`/`input`/`resize` helpers, no-op resize when unattached.
- Embedded preview is an `<iframe>` to `http://localhost:<port>`; the open-in-browser button uses the opener plugin. Both are hidden when the run has no port.

## File Structure

**Backend**
- **Modify** `crates/agency-core/src/registry.rs` — `Run.port_base: Option<u16>`; schema column + migration; `list_port_bases()`.
- **Modify** `crates/agency-app/src/state.rs` — `pick_port`/`allocate_port`; wire port into `create_run`/`rerun`; `RunInfo.port`; `run_attaches` map + `run_session_name`; run-script lifecycle methods; cleanup in discard/stop/close/delete.
- **Modify** `crates/agency-app/src/commands.rs` — 9 run-script commands.
- **Modify** `crates/agency-app/src/lib.rs` — register commands; init opener plugin.
- **Modify** `crates/agency-app/Cargo.toml` — `tauri-plugin-opener = "2"`.
- **Modify** `crates/agency-app/capabilities/default.json` — add `opener:allow-open-url`.

**Frontend**
- **Modify** `ui/src/api.ts` — `RunInfo.port`; 9 run-script wrappers.
- **Modify** `ui/src/components/FocusTerminal.tsx` — extract `TerminalStream`; export `agentStream` + `runStream`; accept optional `stream`.
- **Create** `ui/src/components/RunPanel.tsx` — run controls + terminal + preview.
- **Modify** `ui/src/components/AgentFocus.tsx` — Agent/Run tab toggle.
- **Modify** `ui/src/styles.css` — tab + run-panel styles.
- **Modify** `ui/package.json` — `@tauri-apps/plugin-opener`.

---

### Task 1: Persist `port_base` on runs (schema + registry)

**Files:**
- Modify: `crates/agency-core/src/registry.rs`
- Modify: `crates/agency-app/src/state.rs` (the `Run { … }` literal in `create_run` only — set `port_base: None` as a temporary stopgap; Task 2 fills it in)

**Interfaces:**
- Produces:
  - `agency_core::registry::Run` gains `pub port_base: Option<u16>`
  - `Registry::list_port_bases(&self) -> Result<Vec<u16>>`

- [ ] **Step 1: Write the failing tests**

`registry.rs` has no test module yet. Add at the bottom of `crates/agency-core/src/registry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn sample_run(id: &str, port: Option<u16>) -> Run {
        Run {
            id: id.to_string(),
            project_id: "proj".to_string(),
            agent: "claude".to_string(),
            prompt: "do a thing".to_string(),
            base: "HEAD".to_string(),
            branch: format!("agent/{id}"),
            created_at: 42,
            port_base: port,
        }
    }

    #[test]
    fn run_roundtrips_port_base() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", Some(5200))).unwrap();
        let got = reg.get_run("x-1").unwrap().unwrap();
        assert_eq!(got.port_base, Some(5200));
    }

    #[test]
    fn list_port_bases_returns_only_assigned() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", Some(5200))).unwrap();
        reg.insert_run(&sample_run("x-2", None)).unwrap();
        reg.insert_run(&sample_run("x-3", Some(5210))).unwrap();
        let mut bases = reg.list_port_bases().unwrap();
        bases.sort();
        assert_eq!(bases, vec![5200, 5210]);
    }

    #[test]
    fn migrates_legacy_runs_table_without_port_base() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("legacy.db");
        // Create a runs table WITHOUT port_base, as older builds did.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, agent TEXT NOT NULL,
                    prompt TEXT NOT NULL, base TEXT NOT NULL, branch TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at)
                 VALUES ('old-1','proj','claude','p','HEAD','agent/old-1',1)",
                [],
            )
            .unwrap();
        }
        // Opening through Registry must add the column and preserve the row.
        let reg = Registry::open(&db).unwrap();
        let got = reg.get_run("old-1").unwrap().unwrap();
        assert_eq!(got.port_base, None);
        reg.insert_run(&sample_run("new-1", Some(5200))).unwrap();
        assert_eq!(reg.list_port_bases().unwrap(), vec![5200]);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p agency-core registry::`
Expected: FAIL to compile — `Run` has no `port_base` field; `list_port_bases` undefined.

- [ ] **Step 3: Add the field to `Run`**

In `crates/agency-core/src/registry.rs`, add to the `Run` struct (after `created_at: i64,`):

```rust
    pub port_base: Option<u16>,
```

- [ ] **Step 4: Add the column to the schema and a migration**

In `Registry::open`, the `runs` `CREATE TABLE IF NOT EXISTS` currently ends with `created_at INTEGER NOT NULL`. Change that line to add the column for fresh DBs:

```rust
                created_at INTEGER NOT NULL,
                port_base INTEGER
```

Then, immediately after the `execute_batch(...)?;` call (still inside `open`, before `Ok(Registry { conn })`), add the migration for pre-existing DBs:

```rust
        // Migrate older DBs whose `runs` table predates `port_base`.
        if !column_exists(&conn, "runs", "port_base")? {
            conn.execute("ALTER TABLE runs ADD COLUMN port_base INTEGER", [])?;
        }
```

And add this free function near the bottom of the file (next to `row_to_run`):

```rust
fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}
```

- [ ] **Step 5: Persist and read the column**

Update `insert_run` — change the SQL and params to include `port_base` (bound as `i64` for a clean SQLite type):

```rust
    pub fn insert_run(&self, run: &Run) -> Result<()> {
        self.conn.execute(
            "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at, port_base)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                run.id, run.project_id, run.agent, run.prompt, run.base, run.branch,
                run.created_at, run.port_base.map(|p| p as i64)
            ],
        )?;
        Ok(())
    }
```

Update the two `SELECT` statements (`get_run` and `list_runs`) to fetch `port_base` as the last column:

- `get_run`: `"SELECT id, project_id, agent, prompt, base, branch, created_at, port_base FROM runs WHERE id = ?1"`
- `list_runs`: `"SELECT id, project_id, agent, prompt, base, branch, created_at, port_base FROM runs WHERE project_id = ?1 ORDER BY created_at DESC"`

Update `row_to_run` to read it (index 7), converting `i64` → `u16`:

```rust
fn row_to_run(row: &rusqlite::Row) -> Result<Run> {
    let port_base: Option<i64> = row.get(7)?;
    Ok(Run {
        id: row.get(0)?,
        project_id: row.get(1)?,
        agent: row.get(2)?,
        prompt: row.get(3)?,
        base: row.get(4)?,
        branch: row.get(5)?,
        created_at: row.get(6)?,
        port_base: port_base.map(|p| p as u16),
    })
}
```

Add the query method (inside `impl Registry`, near `list_runs`):

```rust
    pub fn list_port_bases(&self) -> Result<Vec<u16>> {
        let mut stmt = self
            .conn
            .prepare("SELECT port_base FROM runs WHERE port_base IS NOT NULL")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r? as u16);
        }
        Ok(out)
    }
```

- [ ] **Step 6: Fix the one construction site so the workspace compiles**

In `crates/agency-app/src/state.rs`, the `create_run` method builds a `Run { … }` literal. Add a temporary field (Task 2 replaces it):

```rust
            created_at: now_secs(),
            port_base: None,
```

- [ ] **Step 7: Run tests + build**

Run: `cargo test -p agency-core registry::`
Expected: PASS (3 tests).
Run: `cargo build`
Expected: workspace compiles (the `port_base: None` stopgap keeps `state.rs` valid).

- [ ] **Step 8: Commit**

```bash
git add crates/agency-core/src/registry.rs crates/agency-app/src/state.rs
git commit -m "Persist port_base on runs with schema migration"
```

---

### Task 2: Port allocation wired into run creation

**Files:**
- Modify: `crates/agency-app/src/state.rs`

**Interfaces:**
- Consumes: `Registry::list_port_bases` (Task 1), `agency_core::config::load` + `PortsConfig` (Plan 1).
- Produces:
  - free fn `pick_port(used: &HashSet<u16>, base: u16, block_size: u16) -> Option<u16>`
  - `RunInfo` gains `pub port: Option<u16>`

- [ ] **Step 1: Write the failing test for the pure allocator**

Add to the existing `#[cfg(test)] mod tests` in `crates/agency-app/src/state.rs` (it currently tests `slugify`/`new_task_id`):

```rust
    use super::pick_port;
    use std::collections::HashSet;

    #[test]
    fn pick_port_returns_base_when_unused() {
        let used = HashSet::new();
        assert_eq!(pick_port(&used, 5200, 10), Some(5200));
    }

    #[test]
    fn pick_port_skips_used_blocks_lowest_first() {
        let used: HashSet<u16> = [5200, 5210].into_iter().collect();
        assert_eq!(pick_port(&used, 5200, 10), Some(5220));
    }

    #[test]
    fn pick_port_fills_lowest_gap() {
        let used: HashSet<u16> = [5200, 5220].into_iter().collect();
        assert_eq!(pick_port(&used, 5200, 10), Some(5210));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p agency-app pick_port`
Expected: FAIL to compile — `pick_port` undefined.

- [ ] **Step 3: Implement `pick_port` and `allocate_port`**

Add the free function near `new_task_id` in `state.rs`:

```rust
/// Lowest free port-block base: the first `base + slot*block_size` (slot = 0,1,2…)
/// not already in `used`. Returns `None` only if the `u16` space overflows first.
fn pick_port(used: &std::collections::HashSet<u16>, base: u16, block_size: u16) -> Option<u16> {
    let mut slot: u16 = 0;
    loop {
        let candidate = base.checked_add(slot.checked_mul(block_size)?)?;
        if !used.contains(&candidate) {
            return Some(candidate);
        }
        slot = slot.checked_add(1)?;
    }
}
```

Add a method to `impl AppState` (near `create_run`):

```rust
    fn allocate_port(&self, base: u16, block_size: u16) -> Result<u16> {
        let used: std::collections::HashSet<u16> =
            self.registry.lock().unwrap().list_port_bases()?.into_iter().collect();
        pick_port(&used, base, block_size).ok_or_else(|| anyhow!("no free port block available"))
    }
```

- [ ] **Step 4: Wire the port into `create_run`**

In `create_run`, just after `let config = agency_core::config::load(&repo);` add:

```rust
        let port = self.allocate_port(config.ports.base, config.ports.block_size)?;
```

Change the `script_env` call's last argument from `None` to `Some(port)`:

```rust
        env.extend(agency_core::scripts::script_env(&worktree.path, &repo, &id, Some(port)));
```

Change the `Run { … }` literal's `port_base: None,` (the Task 1 stopgap) to:

```rust
            port_base: Some(port),
```

- [ ] **Step 5: Wire the stored port into `rerun`**

In `rerun`, change the `script_env` call's last argument from `None` to the run's stored port:

```rust
        env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));
```

- [ ] **Step 6: Expose the port on `RunInfo`**

Add to the `RunInfo` struct (after `files: u32,`):

```rust
    pub port: Option<u16>,
```

In `run_info`, add to the returned `RunInfo { … }`:

```rust
            port: run.port_base,
```

- [ ] **Step 7: Run tests + build**

Run: `cargo test -p agency-app pick_port`
Expected: PASS (3 tests).
Run: `cargo build && cargo test -p agency-core`
Expected: compiles; core suite still green.

- [ ] **Step 8: Commit**

```bash
git add crates/agency-app/src/state.rs
git commit -m "Allocate a port block per run and expose it on RunInfo"
```

---

### Task 3: Run-script tmux sessions (state)

**Files:**
- Modify: `crates/agency-app/src/state.rs`

**Interfaces:**
- Consumes: `agency_core::config::{load, RunMode}`, `agency_core::scripts::script_env`, `Tmux`.
- Produces (all on `AppState`):
  - `run_script_configured(&self, id: &str) -> Result<bool>`
  - `start_run_script(&self, id: &str) -> Result<()>`
  - `stop_run_script(&self, id: &str) -> Result<()>`
  - `run_script_status(&self, id: &str) -> Result<SessionStatus>`
  - `run_script_preview(&self, id: &str, lines: usize) -> Result<String>`
  - `attach_run_script<F>(&self, id: &str, on_output: F) -> Result<()>`
  - `detach_run_script(&self, id: &str)`
  - `run_script_input(&self, id: &str, data: &[u8]) -> Result<()>`
  - `resize_run_script(&self, id: &str, cols: u16, rows: u16) -> Result<()>`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `state.rs`:

```rust
    #[test]
    fn run_session_name_is_namespaced() {
        assert_eq!(super::run_session_name("fix-login-a3k2"), "agency-run-fix-login-a3k2");
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p agency-app run_session_name`
Expected: FAIL to compile — `run_session_name` undefined.

- [ ] **Step 3: Add the run-attaches map and session name**

In `AppState`, add a field:

```rust
    run_attaches: Mutex<HashMap<String, AgentHandle>>,
```

In `AppState::new`, add to the constructed struct:

```rust
            run_attaches: Mutex::new(HashMap::new()),
```

Add the free function next to `session_name`:

```rust
fn run_session_name(id: &str) -> String {
    format!("agency-run-{id}")
}
```

- [ ] **Step 4: Implement the lifecycle methods**

Add a new `impl AppState` block (or append to the existing one) with:

```rust
    pub fn run_script_configured(&self, id: &str) -> Result<bool> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        Ok(agency_core::config::load(&repo).scripts.run.is_some())
    }

    pub fn start_run_script(&self, id: &str) -> Result<()> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let config = agency_core::config::load(&repo);
        let run_cmd = config
            .scripts
            .run
            .clone()
            .ok_or_else(|| anyhow!("no run script configured in .agency/agency.toml"))?;
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);

        // nonconcurrent: stop every other run-script session first.
        if config.scripts.run_mode == agency_core::config::RunMode::Nonconcurrent {
            let others = self.registry.lock().unwrap().list_runs(&run.project_id)?;
            for other in others {
                if other.id != run.id {
                    self.run_attaches.lock().unwrap().remove(&other.id);
                    self.tmux.kill_session(&run_session_name(&other.id)).ok();
                }
            }
        }

        let mut env = self.provider_env()?;
        env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));

        // Restart cleanly if a previous run session is still around.
        self.tmux.kill_session(&run_session_name(id)).ok();
        self.tmux.start_session(
            &run_session_name(id),
            &worktree,
            "sh",
            &["-lc".to_string(), run_cmd],
            &env,
        )
    }

    pub fn stop_run_script(&self, id: &str) -> Result<()> {
        self.run_attaches.lock().unwrap().remove(id);
        self.tmux.kill_session(&run_session_name(id)).ok();
        Ok(())
    }

    pub fn run_script_status(&self, id: &str) -> Result<SessionStatus> {
        Ok(self
            .tmux
            .session_status(&run_session_name(id))
            .unwrap_or(SessionStatus::Gone))
    }

    pub fn run_script_preview(&self, id: &str, lines: usize) -> Result<String> {
        self.tmux.capture(&run_session_name(id), lines)
    }

    pub fn attach_run_script<F>(&self, id: &str, on_output: F) -> Result<()>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        let handle = self.tmux.attach(&run_session_name(id), on_output)?;
        self.run_attaches.lock().unwrap().insert(id.to_string(), handle);
        Ok(())
    }

    pub fn detach_run_script(&self, id: &str) {
        self.run_attaches.lock().unwrap().remove(id);
    }

    pub fn run_script_input(&self, id: &str, data: &[u8]) -> Result<()> {
        let attaches = self.run_attaches.lock().unwrap();
        let handle = attaches
            .get(id)
            .ok_or_else(|| anyhow!("run script not attached: {id}"))?;
        handle.write_input(data)
    }

    pub fn resize_run_script(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        let attaches = self.run_attaches.lock().unwrap();
        if let Some(handle) = attaches.get(id) {
            handle.resize(rows, cols)?;
        }
        Ok(())
    }
```

- [ ] **Step 5: Kill run sessions on cleanup**

In `discard_run`, after `self.tmux.kill_session(&session_name(id)).ok();` add:

```rust
        self.run_attaches.lock().unwrap().remove(id);
        self.tmux.kill_session(&run_session_name(id)).ok();
```

In `stop_run`, after `self.tmux.kill_session(&session_name(id)).ok();` add the same two lines.

In `close_project` and `delete_project`, inside the `for run in &runs` loop, after the existing `self.tmux.kill_session(&session_name(&run.id)).ok();`, add:

```rust
            self.run_attaches.lock().unwrap().remove(&run.id);
            self.tmux.kill_session(&run_session_name(&run.id)).ok();
```

- [ ] **Step 6: Run tests + build**

Run: `cargo test -p agency-app run_session_name`
Expected: PASS.
Run: `cargo build`
Expected: compiles.

- [ ] **Step 7: Commit**

```bash
git add crates/agency-app/src/state.rs
git commit -m "Manage per-workspace run-script tmux sessions"
```

---

### Task 4: Run-script commands + opener plugin

**Files:**
- Modify: `crates/agency-app/src/commands.rs`
- Modify: `crates/agency-app/src/lib.rs`
- Modify: `crates/agency-app/Cargo.toml`
- Modify: `crates/agency-app/capabilities/default.json`

**Interfaces:**
- Consumes: Task 3 `AppState` methods; existing `TerminalChunk`, `STANDARD`, `Channel`.
- Produces: tauri commands `start_run_script`, `stop_run_script`, `run_script_status`, `run_script_configured`, `run_script_preview`, `attach_run_script`, `detach_run_script`, `run_script_input`, `resize_run_script`.

- [ ] **Step 1: Add the dependency**

In `crates/agency-app/Cargo.toml`, under `[dependencies]` after `tauri-plugin-dialog = "2"`:

```toml
tauri-plugin-opener = "2"
```

- [ ] **Step 2: Add the commands**

Append to `crates/agency-app/src/commands.rs` (these mirror `attach_run`/`run_input`/`resize_run`/`run_status`/`run_preview` exactly):

```rust
#[tauri::command]
pub fn run_script_configured(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    state.run_script_configured(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_run_script(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.start_run_script(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_run_script(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.stop_run_script(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn run_script_status(state: State<'_, AppState>, id: String) -> Result<SessionStatus, String> {
    state.run_script_status(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn run_script_preview(
    state: State<'_, AppState>,
    id: String,
    lines: usize,
) -> Result<String, String> {
    state.run_script_preview(&id, lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn attach_run_script(
    state: State<'_, AppState>,
    id: String,
    on_chunk: Channel<TerminalChunk>,
) -> Result<(), String> {
    state
        .attach_run_script(&id, move |bytes| {
            let _ = on_chunk.send(TerminalChunk { b64: STANDARD.encode(&bytes) });
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn detach_run_script(state: State<'_, AppState>, id: String) {
    state.detach_run_script(&id);
}

#[tauri::command]
pub fn run_script_input(state: State<'_, AppState>, id: String, data: String) -> Result<(), String> {
    state.run_script_input(&id, data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resize_run_script(
    state: State<'_, AppState>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    state.resize_run_script(&id, cols, rows).map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Register commands + opener plugin in `lib.rs`**

In `crates/agency-app/src/lib.rs`, add the opener plugin after the dialog plugin:

```rust
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
```

Add the nine commands to the `tauri::generate_handler![ … ]` list (anywhere in the list, e.g. after `commands::rerun,`):

```rust
            commands::run_script_configured,
            commands::start_run_script,
            commands::stop_run_script,
            commands::run_script_status,
            commands::run_script_preview,
            commands::attach_run_script,
            commands::detach_run_script,
            commands::run_script_input,
            commands::resize_run_script,
```

- [ ] **Step 4: Grant the opener permission**

In `crates/agency-app/capabilities/default.json`, add `"opener:allow-open-url"` to the `permissions` array:

```json
  "permissions": ["core:default", "dialog:allow-open", "opener:allow-open-url"]
```

- [ ] **Step 5: Build**

Run: `cargo build`
Expected: compiles; `tauri-plugin-opener` resolves and the handler list is valid.

- [ ] **Step 6: Commit**

```bash
git add crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs crates/agency-app/Cargo.toml crates/agency-app/capabilities/default.json Cargo.lock
git commit -m "Expose run-script commands and the Tauri opener plugin"
```

---

### Task 5: Frontend API + terminal stream generalization

**Files:**
- Modify: `ui/src/api.ts`
- Modify: `ui/src/components/FocusTerminal.tsx`
- Modify: `ui/package.json`

**Interfaces:**
- Produces:
  - `RunInfo.port: number | null`
  - api wrappers: `startRunScript`, `stopRunScript`, `runScriptStatus`, `runScriptConfigured`, `runScriptPreview`, `attachRunScript`, `detachRunScript`, `runScriptInput`, `resizeRunScript`
  - `FocusTerminal` exports `TerminalStream`, `agentStream`, `runStream`; accepts optional `stream` prop (defaults to `agentStream`)

- [ ] **Step 1: Add the opener npm dependency**

In `ui/package.json`, add to `dependencies` (alphabetical, after `@tauri-apps/plugin-dialog`):

```json
    "@tauri-apps/plugin-opener": "^2",
```

Then run: `cd ui && npm install`
Expected: lockfile/node_modules updated, no errors.

- [ ] **Step 2: Extend `RunInfo` and add run-script wrappers**

In `ui/src/api.ts`, add `port` to `RunInfo` (after `files: number;`):

```ts
  port: number | null;
```

Add after the `attachRun` function (around line 69):

```ts
export const runScriptConfigured = (id: string) =>
  invoke<boolean>("run_script_configured", { id });
export const startRunScript = (id: string) => invoke<void>("start_run_script", { id });
export const stopRunScript = (id: string) => invoke<void>("stop_run_script", { id });
export const runScriptStatus = (id: string) =>
  invoke<SessionStatus>("run_script_status", { id });
export const runScriptPreview = (id: string, lines: number) =>
  invoke<string>("run_script_preview", { id, lines });
export const detachRunScript = (id: string) => invoke<void>("detach_run_script", { id });
export const runScriptInput = (id: string, data: string) =>
  invoke<void>("run_script_input", { id, data });
export const resizeRunScript = (id: string, cols: number, rows: number) =>
  invoke<void>("resize_run_script", { id, cols, rows });

export function attachRunScript(id: string, onBytes: (b: Uint8Array) => void): Promise<void> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (m) => onBytes(b64ToBytes(m.b64));
  return invoke<void>("attach_run_script", { id, onChunk });
}
```

- [ ] **Step 3: Generalize `FocusTerminal` over a stream**

Replace the imports and component signature in `ui/src/components/FocusTerminal.tsx`. Change the top import line:

```ts
import { attachRun, detachRun, resizeRun, runInput, runPreview,
  attachRunScript, detachRunScript, resizeRunScript, runScriptInput, runScriptPreview } from "../api";
```

Add, just below the imports:

```ts
export interface TerminalStream {
  attach(id: string, onBytes: (b: Uint8Array) => void): Promise<void>;
  detach(id: string): void;
  resize(id: string, cols: number, rows: number): Promise<void>;
  input(id: string, data: string): Promise<void>;
  preview(id: string, lines: number): Promise<string>;
}

export const agentStream: TerminalStream = {
  attach: attachRun, detach: detachRun, resize: resizeRun, input: runInput, preview: runPreview,
};

export const runStream: TerminalStream = {
  attach: attachRunScript, detach: detachRunScript, resize: resizeRunScript,
  input: runScriptInput, preview: runScriptPreview,
};
```

Change the component signature:

```ts
export default function FocusTerminal(
  { runId, stream = agentStream }: { runId: string; stream?: TerminalStream },
) {
```

Then inside the effect, replace each bare api call with the `stream` member:
- `resizeRun(runId, …)` → `stream.resize(runId, …)`
- `runPreview(runId, 200)` → `stream.preview(runId, 200)`
- `attachRun(runId, …)` → `stream.attach(runId, …)`
- `runInput(runId, d)` → `stream.input(runId, d)`
- `detachRun(runId)` → `stream.detach(runId)`

(The agent terminal in `AgentFocus` keeps working unchanged because `stream` defaults to `agentStream`.)

- [ ] **Step 4: Typecheck**

Run: `cd ui && npx tsc --noEmit`
Expected: no type errors.

- [ ] **Step 5: Commit**

```bash
git add ui/src/api.ts ui/src/components/FocusTerminal.tsx ui/package.json ui/package-lock.json
git commit -m "Add run-script frontend API and generalize FocusTerminal over a stream"
```

---

### Task 6: Run panel UI (tab + terminal + preview)

**Files:**
- Create: `ui/src/components/RunPanel.tsx`
- Modify: `ui/src/components/AgentFocus.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- Consumes: Task 5 api wrappers + `runStream`; `RunInfo` (with `port`).
- Produces: `RunPanel` (default export); a local Agent/Run tab in `AgentFocus`.

- [ ] **Step 1: Create `RunPanel.tsx`**

```tsx
import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { RunInfo, SessionStatus, runScriptStatus, runScriptConfigured, startRunScript, stopRunScript } from "../api";
import FocusTerminal, { runStream } from "./FocusTerminal";

export default function RunPanel({ run }: { run: RunInfo }) {
  const [status, setStatus] = useState<SessionStatus>({ state: "gone" });
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [started, setStarted] = useState(false);
  const [previewKey, setPreviewKey] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const url = run.port != null ? `http://localhost:${run.port}` : null;
  const running = status.state === "running";

  useEffect(() => {
    setStarted(false);
    setError(null);
    runScriptConfigured(run.id).then(setConfigured).catch(() => setConfigured(false));
  }, [run.id]);

  useEffect(() => {
    let alive = true;
    const tick = () => runScriptStatus(run.id).then((s) => { if (alive) setStatus(s); }).catch(() => {});
    tick();
    const t = setInterval(tick, 1500);
    return () => { alive = false; clearInterval(t); };
  }, [run.id]);

  const start = async () => {
    setError(null);
    try {
      await startRunScript(run.id);
      setStarted(true);
    } catch (e) {
      setError(String(e));
    }
  };
  const stop = async () => {
    await stopRunScript(run.id);
    setStarted(false);
  };

  if (configured === false) {
    return (
      <div className="run-panel empty">
        No run script configured. Add a <code>[scripts]</code> <code>run</code> line to
        <code> .agency/agency.toml</code> to launch this workspace's app.
      </div>
    );
  }

  return (
    <div className="run-panel">
      <div className="run-controls">
        <span className={`dot ${running ? "running" : "exited"}`} />
        {running ? (
          <button className="tile-act" onClick={stop}>■ Stop</button>
        ) : (
          <button className="tile-act" onClick={start}>▶ Run</button>
        )}
        {url && (
          <>
            <code className="run-url">{url}</code>
            <button className="tile-act" onClick={() => setPreviewKey((k) => k + 1)}>⟳ Refresh</button>
            <button className="tile-act" onClick={() => openUrl(url)}>Open in browser ↗</button>
          </>
        )}
        {error && <span className="run-error">{error}</span>}
      </div>
      <div className="run-body">
        {(running || started) && (
          <div className="run-term">
            <FocusTerminal key={`run-${run.id}`} runId={run.id} stream={runStream} />
          </div>
        )}
        {url && running && (
          <iframe
            key={previewKey}
            className="run-preview"
            src={url}
            title="workspace preview"
          />
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Add the Agent/Run tab to `AgentFocus`**

In `ui/src/components/AgentFocus.tsx`, add the import:

```ts
import RunPanel from "./RunPanel";
```

Add a panel state inside the component (next to the other `useState` calls):

```ts
  const [panel, setPanel] = useState<"agent" | "run">("agent");
```

Reset it when the focused run changes — extend the existing effect:

```ts
  useEffect(() => {
    setShowMerge(false);
    setPanel("agent");
  }, [focusedRunId]);
```

In the `focus-head` row, add a tab toggle right after the `<code>{focused.branch}</code>` element:

```tsx
              <div className="focus-tabs">
                <button className={panel === "agent" ? "on" : ""} onClick={() => setPanel("agent")}>Agent</button>
                <button className={panel === "run" ? "on" : ""} onClick={() => setPanel("run")}>Run</button>
              </div>
```

Replace the single `<FocusTerminal key={focused.id} runId={focused.id} />` line with a panel switch:

```tsx
            {panel === "agent"
              ? <FocusTerminal key={focused.id} runId={focused.id} />
              : <RunPanel key={`run-${focused.id}`} run={focused} />}
```

- [ ] **Step 3: Add styles**

Append to `ui/src/styles.css`:

```css
.focus-tabs { display: flex; gap: 4px; margin-left: 8px; }
.focus-tabs button { padding: 2px 10px; border-radius: 6px; background: transparent; color: var(--muted, #9aa); border: 1px solid transparent; }
.focus-tabs button.on { background: var(--panel, #1d1f26); color: var(--fg, #e6e6e6); border-color: var(--border, #2a2d36); }

.run-panel { display: flex; flex-direction: column; height: 100%; min-height: 0; }
.run-panel.empty { padding: 24px; color: var(--muted, #9aa); display: block; }
.run-panel.empty code { margin: 0 3px; }
.run-controls { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-bottom: 1px solid var(--border, #2a2d36); }
.run-url { color: var(--muted, #9aa); }
.run-error { color: #e06c75; font-size: 12px; }
.run-body { flex: 1; display: flex; min-height: 0; }
.run-term { flex: 1; min-width: 0; display: flex; }
.run-preview { flex: 1; min-width: 0; border: 0; border-left: 1px solid var(--border, #2a2d36); background: #fff; }
```

- [ ] **Step 4: Typecheck + build**

Run: `cd ui && npx tsc --noEmit && npm run build`
Expected: no type errors; vite build succeeds.

- [ ] **Step 5: Manual smoke test**

1. In a scratch repo with a dev server, add:

```toml
[scripts]
run = "python3 -m http.server $AGENCY_PORT"
```

2. Spawn an agent; open the workspace's **Run** tab; click **Run**.
3. Confirm: the run terminal streams `Serving HTTP on … port 52xx`, the embedded preview shows the served directory, **Open in browser ↗** opens the same URL, and **Stop** kills the server (status dot goes grey).
4. Spawn a second workspace and Run it too — confirm it gets a *different* port (concurrent default) and both previews work.
5. Set `run_mode = "nonconcurrent"`, Run workspace A then workspace B — confirm starting B stops A's run session.
6. Remove the `run` line; reopen the Run tab — confirm the "No run script configured" message.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/RunPanel.tsx ui/src/components/AgentFocus.tsx ui/src/styles.css
git commit -m "Add Run tab with streamed logs, embedded preview, and open-in-browser"
```

---

## Self-Review

**Spec coverage (Feature 2):**
- Port allocation `base + slot*block_size`, lowest free slot → Task 2 `pick_port`/`allocate_port`. ✓
- Allocated at create, persisted, freed on discard, reused on rerun → Tasks 1–2 (`port_base`, `create_run`/`rerun`, delete-frees-via-`list_port_bases`). ✓
- `AGENCY_PORT` available to setup/agent/run → Task 2 passes `Some(port)` to `script_env` in `create_run`; Task 3 passes `run.port_base` to the run session. ✓
- Separate `agency-run-<id>` session → Task 3. ✓
- `run_mode` concurrent/nonconcurrent → Task 3 `start_run_script`. ✓
- Commands mirror agent attach path (Channel/base64) → Task 4. ✓
- Run tab with streamed logs + embedded iframe preview + open-in-browser → Tasks 5–6. ✓
- Both preview surfaces hidden without a port; "no run script" guidance → Task 6 `RunPanel`. ✓
- Cleanup kills run sessions on discard/stop/close/delete → Task 3 Step 5. ✓

**Placeholder scan:** none — every step carries concrete code/commands.

**Type consistency:** `port_base: Option<u16>` is the single representation across `Run` (Task 1), `RunInfo.port` (Task 2 → api `port: number | null`), and `script_env(..., port: Option<u16>)` (Plan 1). `run_session_name` is defined in Task 3 Step 3 and used by every method in Task 3 Step 4. `TerminalStream` (Task 5) is consumed by `RunPanel` via `runStream` (Task 6). The nine command names match between `commands.rs` (Task 4 Step 2), `lib.rs` registration (Task 4 Step 3), and `api.ts` (Task 5 Step 2): `start_run_script`/`stop_run_script`/`run_script_status`/`run_script_configured`/`run_script_preview`/`attach_run_script`/`detach_run_script`/`run_script_input`/`resize_run_script`.

**Dependencies between tasks:** 1→2 (field before allocation), 2→3 (port on run before run session uses it), 3→4 (methods before commands), 4→5 (commands before api), 5→6 (stream + api before UI). Strictly sequential.

**Deferred (later plans):** archived runs excluded from port reuse — when Plan 3 adds `archived_at`, narrow `list_port_bases` to non-archived. Notifications on run-script crash → Plan 4.
