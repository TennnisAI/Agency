# Workspace Lifecycle — Plan 3: Archive

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an "archive" state between keep-running and destroy: archiving a run removes its worktree but keeps the branch + record, declutters the active list, and is restorable.

**Architecture:** A nullable `archived_at` timestamp on the `runs` row marks a run archived. Archiving kills both sessions, runs the optional `[scripts].archive` cleanup, removes the worktree **but keeps the branch** (`agent/<id>`), and stamps `archived_at`. `list_runs` hides archived rows; `list_archived_runs` surfaces them. Restore re-creates the worktree from the kept branch and clears `archived_at`. The UI gains an Archive action and an "Archived" section (Restore / Discard) in the focus rail.

**Tech Stack:** Rust (agency-core, agency-app/Tauri), `rusqlite`, `git`, React/TS.

**Spec:** `docs/superpowers/specs/2026-06-22-workspace-lifecycle-design.md` (Feature 3). **Builds on Plans 1–2** (merged to `main`).

## Global Constraints

- Rust edition 2021. No AI/Claude attribution in any commit message (hard rule).
- Archive = kill sessions + run optional archive script (best-effort) + `git worktree remove --force` **keeping** the branch + set `archived_at = now`. The run record is kept (not deleted).
- Restore = `git worktree add <path> agent/<id>` (existing branch, **no** `-b`) + clear `archived_at`. A fresh agent session is the user's next step (not auto-started here).
- Terminal scrollback / agent chat is NOT preserved across archive (accepted limitation). Branch, commits, diff, and metadata are.
- `list_runs` returns only non-archived (`archived_at IS NULL`); `list_archived_runs` returns only archived.
- Now that `archived_at` exists, `list_port_bases` (Plan 2) must exclude archived runs so an archived run's port is reclaimable.
- Discarding an archived run must still delete the kept branch (no leak).
- UI scope for this plan: Archive button + Archived section live in the **focus view**. Adding the same Archive button to the grid `AgentTile` is an explicit follow-up, not part of this plan.

## File Structure

**Backend**
- **Modify** `crates/agency-core/src/registry.rs` — `Run.archived_at`; schema column + migration; filter `list_runs`; `list_archived_runs`; `set_archived`; narrow `list_port_bases`.
- **Modify** `crates/agency-core/src/worktree.rs` — `remove_keep_branch`; `restore`; make `remove` tolerant (best-effort + prune) so archived-discard deletes the branch.
- **Modify** `crates/agency-core/src/scripts.rs` — `run_blocking` (one-off script to completion).
- **Modify** `crates/agency-app/src/state.rs` — `archive_run`, `restore_run`, `list_archived_runs`; `create_run` literal stopgap.
- **Modify** `crates/agency-app/src/commands.rs` — `archive_run`, `restore_run`, `list_archived_runs`.
- **Modify** `crates/agency-app/src/lib.rs` — register 3 commands.

**Frontend**
- **Modify** `ui/src/api.ts` — `RunInfo.archivedAt`; `archiveRun`, `restoreRun`, `listArchivedRuns`.
- **Create** `ui/src/components/ArchivedSection.tsx` — collapsible archived list with Restore / Discard.
- **Modify** `ui/src/components/AgentFocus.tsx` — Archive button in the head; render `ArchivedSection` in the rail.
- **Modify** `ui/src/styles.css` — archived section styles.

---

### Task 1: `archived_at` schema + registry queries

**Files:**
- Modify: `crates/agency-core/src/registry.rs`
- Modify: `crates/agency-app/src/state.rs` (the `Run { … }` literal in `create_run` — add `archived_at: None`)

**Interfaces:**
- Produces:
  - `Run` gains `pub archived_at: Option<i64>`
  - `Registry::list_archived_runs(&self, project_id: &str) -> Result<Vec<Run>>`
  - `Registry::set_archived(&self, id: &str, archived_at: Option<i64>) -> Result<()>`
  - `list_runs` excludes archived; `list_port_bases` excludes archived

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod tests` in `registry.rs` (it already has a `sample_run` helper from Plan 2 — extend that helper to set the new field, and add tests):

First, update the existing `sample_run` helper to include the new field:

```rust
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
            archived_at: None,
        }
    }
```

Then add these tests:

```rust
    #[test]
    fn archiving_hides_from_list_runs_and_shows_in_archived() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", Some(5200))).unwrap();
        reg.insert_run(&sample_run("x-2", Some(5210))).unwrap();
        reg.set_archived("x-1", Some(1000)).unwrap();

        let active: Vec<String> = reg.list_runs("proj").unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(active, vec!["x-2"]);
        let archived: Vec<String> = reg.list_archived_runs("proj").unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(archived, vec!["x-1"]);
    }

    #[test]
    fn archived_run_port_is_not_listed_as_used() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", Some(5200))).unwrap();
        reg.insert_run(&sample_run("x-2", Some(5210))).unwrap();
        reg.set_archived("x-1", Some(1000)).unwrap();
        let mut bases = reg.list_port_bases().unwrap();
        bases.sort();
        assert_eq!(bases, vec![5210]); // 5200 freed by archiving x-1
    }

    #[test]
    fn set_archived_none_restores_to_active() {
        let dir = tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("a.db")).unwrap();
        reg.insert_run(&sample_run("x-1", Some(5200))).unwrap();
        reg.set_archived("x-1", Some(1000)).unwrap();
        reg.set_archived("x-1", None).unwrap();
        let active: Vec<String> = reg.list_runs("proj").unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(active, vec!["x-1"]);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p agency-core registry::`
Expected: FAIL to compile — `archived_at` field, `list_archived_runs`, `set_archived` undefined.

- [ ] **Step 3: Add the field and schema column**

Add to the `Run` struct (after `port_base: Option<u16>,`):

```rust
    pub archived_at: Option<i64>,
```

In `Registry::open`, add `archived_at INTEGER` to the `runs` `CREATE TABLE` (after `port_base INTEGER`):

```rust
                port_base INTEGER,
                archived_at INTEGER
```

Add the migration right after the `port_base` migration block (reusing `column_exists` from Plan 2):

```rust
        if !column_exists(&conn, "runs", "archived_at")? {
            conn.execute("ALTER TABLE runs ADD COLUMN archived_at INTEGER", [])?;
        }
```

- [ ] **Step 4: Update insert/select/queries**

`insert_run` — add `archived_at` to the column list, the `VALUES (?…, ?9)`, and params:

```rust
    pub fn insert_run(&self, run: &Run) -> Result<()> {
        self.conn.execute(
            "INSERT INTO runs (id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                run.id, run.project_id, run.agent, run.prompt, run.base, run.branch,
                run.created_at, run.port_base.map(|p| p as i64), run.archived_at
            ],
        )?;
        Ok(())
    }
```

`get_run` SELECT → add `archived_at` as the last (9th) column:
`"SELECT id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at FROM runs WHERE id = ?1"`

`list_runs` SELECT → add `archived_at` AND filter:
`"SELECT id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at FROM runs WHERE project_id = ?1 AND archived_at IS NULL ORDER BY created_at DESC"`

`row_to_run` — read index 8:

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
        archived_at: row.get(8)?,
    })
}
```

Narrow `list_port_bases`:
`"SELECT port_base FROM runs WHERE port_base IS NOT NULL AND archived_at IS NULL"`

Add the two new methods (inside `impl Registry`, near `list_runs`):

```rust
    pub fn list_archived_runs(&self, project_id: &str) -> Result<Vec<Run>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, agent, prompt, base, branch, created_at, port_base, archived_at
             FROM runs WHERE project_id = ?1 AND archived_at IS NOT NULL ORDER BY archived_at DESC",
        )?;
        let rows = stmt.query_map([project_id], |row| Ok(row_to_run(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    pub fn set_archived(&self, id: &str, archived_at: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET archived_at = ?2 WHERE id = ?1",
            rusqlite::params![id, archived_at],
        )?;
        Ok(())
    }
```

- [ ] **Step 5: Stopgap in state.rs**

In `crates/agency-app/src/state.rs`, the `create_run` `Run { … }` literal: add after `port_base: Some(port),`:

```rust
            archived_at: None,
```

Also update any existing `Run { … }` literals in `tests/registry.rs` if present (add `archived_at: None`) — the compiler will point them out.

- [ ] **Step 6: Run tests + build**

Run: `cargo test -p agency-core registry::`
Expected: PASS (existing + 3 new).
Run: `cargo build`
Expected: workspace compiles.

- [ ] **Step 7: Commit**

```bash
git add crates/agency-core/src/registry.rs crates/agency-app/src/state.rs crates/agency-core/tests/registry.rs
git commit -m "Add archived_at to runs with archive/restore queries"
```

---

### Task 2: Worktree keep-branch removal + restore

**Files:**
- Modify: `crates/agency-core/src/worktree.rs`
- Create: `crates/agency-core/tests/worktree.rs`

**Interfaces:**
- Produces:
  - `WorktreeManager::remove_keep_branch(&self, task_id: &str) -> Result<()>`
  - `WorktreeManager::restore(&self, task_id: &str) -> Result<Worktree>`
  - `remove` becomes tolerant (best-effort worktree removal + prune, then delete branch)

- [ ] **Step 1: Write the failing integration test**

Create `crates/agency-core/tests/worktree.rs`:

```rust
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

fn branch_exists(dir: &std::path::Path, branch: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", branch])
        .current_dir(dir)
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
fn remove_keep_branch_keeps_the_branch_then_restore_recreates_worktree() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    let wt = mgr.create("task-1", "HEAD").unwrap();
    assert!(wt.path.exists());
    assert!(branch_exists(repo.path(), "agent/task-1"));

    // Archive: worktree gone, branch kept.
    mgr.remove_keep_branch("task-1").unwrap();
    assert!(!wt.path.exists(), "worktree dir removed");
    assert!(branch_exists(repo.path(), "agent/task-1"), "branch kept");

    // Restore: worktree recreated on the same branch.
    let restored = mgr.restore("task-1").unwrap();
    assert!(restored.path.exists(), "worktree recreated");
    assert_eq!(restored.branch, "agent/task-1");
}

#[test]
fn remove_after_keep_branch_deletes_the_branch_without_error() {
    let repo = init_repo();
    let mgr = WorktreeManager::new(repo.path().to_path_buf());
    mgr.create("task-2", "HEAD").unwrap();
    mgr.remove_keep_branch("task-2").unwrap();
    // Discarding an archived run: worktree already gone, but the branch must go.
    mgr.remove("task-2").unwrap();
    assert!(!branch_exists(repo.path(), "agent/task-2"), "branch deleted on discard");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p agency-core --test worktree`
Expected: FAIL to compile — `remove_keep_branch`/`restore` undefined.

- [ ] **Step 3: Implement the methods and make `remove` tolerant**

In `crates/agency-core/src/worktree.rs`, replace the existing `remove` method and add the two new methods:

```rust
    /// Remove the worktree and delete its branch. Tolerant: each git step is
    /// best-effort so it works whether or not the worktree still exists (e.g.
    /// discarding an already-archived run), and still deletes the branch.
    pub fn remove(&self, task_id: &str) -> Result<()> {
        let path = self.worktrees_root().join(task_id);
        let path_str = path.to_string_lossy().to_string();
        let _ = self.git(&["worktree", "remove", &path_str, "--force"]);
        let _ = self.git(&["worktree", "prune"]);
        let branch = Self::branch_for(task_id);
        let _ = self.git(&["branch", "-D", &branch]);
        Ok(())
    }

    /// Remove the worktree but KEEP the branch, so the work can be restored.
    pub fn remove_keep_branch(&self, task_id: &str) -> Result<()> {
        let path = self.worktrees_root().join(task_id);
        let path_str = path.to_string_lossy().to_string();
        self.git(&["worktree", "remove", &path_str, "--force"])?;
        self.git(&["worktree", "prune"])?;
        Ok(())
    }

    /// Re-create a worktree for `task_id` on its existing branch `agent/<id>`.
    pub fn restore(&self, task_id: &str) -> Result<Worktree> {
        self.ensure_excluded()?;
        let path = self.worktrees_root().join(task_id);
        let branch = Self::branch_for(task_id);
        let path_str = path.to_string_lossy().to_string();
        self.git(&["worktree", "add", &path_str, &branch])?;
        Ok(Worktree { task_id: task_id.to_string(), path, branch })
    }
```

- [ ] **Step 4: Run tests + build**

Run: `cargo test -p agency-core --test worktree`
Expected: PASS (2 tests).
Run: `cargo test -p agency-core`
Expected: full suite green.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/worktree.rs crates/agency-core/tests/worktree.rs
git commit -m "Add keep-branch worktree removal and restore"
```

---

### Task 3: Archive script runner + archive/restore in state

**Files:**
- Modify: `crates/agency-core/src/scripts.rs`
- Modify: `crates/agency-app/src/state.rs`

**Interfaces:**
- Consumes: Task 1 (`set_archived`, `list_archived_runs`), Task 2 (`remove_keep_branch`, `restore`).
- Produces:
  - `agency_core::scripts::run_blocking(script: &str, cwd: &Path, env: &[(String, String)]) -> Result<()>`
  - `AppState::archive_run(&self, id: &str) -> Result<()>`
  - `AppState::restore_run(&self, id: &str) -> Result<RunInfo>`
  - `AppState::list_archived_runs(&self, project_id: &str) -> Result<Vec<RunInfo>>`

- [ ] **Step 1: Write the failing test for `run_blocking`**

Add to the `#[cfg(test)] mod tests` in `scripts.rs`:

```rust
    #[test]
    fn run_blocking_runs_script_with_env_and_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        run_blocking(
            "echo $AGENCY_WORKSPACE_NAME > marker.txt",
            dir.path(),
            &[("AGENCY_WORKSPACE_NAME".to_string(), "fix-login".to_string())],
        )
        .unwrap();
        let body = std::fs::read_to_string(dir.path().join("marker.txt")).unwrap();
        assert_eq!(body.trim(), "fix-login");

        // Non-zero exit surfaces as an error.
        assert!(run_blocking("exit 3", dir.path(), &[]).is_err());
    }
```

(`tempfile` is already a dev-dependency of agency-core.)

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p agency-core scripts::run_blocking`
Expected: FAIL to compile — `run_blocking` undefined.

- [ ] **Step 3: Implement `run_blocking`**

At the top of `crates/agency-core/src/scripts.rs`, ensure these imports exist:

```rust
use anyhow::{bail, Result};
use std::path::Path;
use std::process::Command;
```

Add the function:

```rust
/// Run a one-off lifecycle script (`sh -lc <script>`) to completion in `cwd`
/// with the given env. Returns an error if the script exits non-zero.
pub fn run_blocking(script: &str, cwd: &Path, env: &[(String, String)]) -> Result<()> {
    let mut cmd = Command::new("sh");
    cmd.arg("-lc").arg(script).current_dir(cwd);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.status()?;
    if !status.success() {
        bail!("archive script exited with {status}");
    }
    Ok(())
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p agency-core scripts::run_blocking`
Expected: PASS.

- [ ] **Step 5: Implement archive/restore in `state.rs`**

Add to `impl AppState` (near `discard_run`):

```rust
    /// Archive a run: stop its sessions, run the optional archive cleanup script,
    /// remove the worktree but KEEP the branch, and stamp `archived_at`. The run
    /// record is kept so it can be restored.
    pub fn archive_run(&self, id: &str) -> Result<()> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;

        // Stop both sessions and drop attach handles.
        self.attaches.lock().unwrap().remove(id);
        self.tmux.kill_session(&session_name(id)).ok();
        self.run_attaches.lock().unwrap().remove(id);
        self.tmux.kill_session(&run_session_name(id)).ok();

        // Best-effort archive cleanup script, before the worktree disappears.
        let config = agency_core::config::load(&repo);
        if let Some(script) = config.scripts.archive.as_deref() {
            let worktree = repo.join(".agency").join("worktrees").join(&run.id);
            if worktree.exists() {
                let env = agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base);
                let _ = agency_core::scripts::run_blocking(script, &worktree, &env);
            }
        }

        WorktreeManager::new(repo).remove_keep_branch(id).ok();
        self.registry.lock().unwrap().set_archived(id, Some(now_secs()))?;
        Ok(())
    }

    /// Restore an archived run: re-create its worktree on the kept branch and
    /// clear `archived_at`. The agent is not auto-started.
    pub fn restore_run(&self, id: &str) -> Result<RunInfo> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        WorktreeManager::new(repo).restore(id)?;
        self.registry.lock().unwrap().set_archived(id, None)?;
        let refreshed = self.run_record(id)?;
        Ok(self.run_info(&refreshed))
    }

    pub fn list_archived_runs(&self, project_id: &str) -> Result<Vec<RunInfo>> {
        let runs = self.registry.lock().unwrap().list_archived_runs(project_id)?;
        Ok(runs.iter().map(|r| self.run_info(r)).collect())
    }
```

- [ ] **Step 6: Build + test**

Run: `cargo build && cargo test -p agency-core`
Expected: compiles; suite green.

- [ ] **Step 7: Commit**

```bash
git add crates/agency-core/src/scripts.rs crates/agency-app/src/state.rs
git commit -m "Archive and restore runs, with an optional archive cleanup script"
```

---

### Task 4: Archive/restore commands

**Files:**
- Modify: `crates/agency-app/src/commands.rs`
- Modify: `crates/agency-app/src/lib.rs`

**Interfaces:**
- Consumes: Task 3 `AppState` methods.
- Produces: tauri commands `archive_run`, `restore_run`, `list_archived_runs`.

- [ ] **Step 1: Add the commands**

Append to `crates/agency-app/src/commands.rs`:

```rust
#[tauri::command]
pub fn archive_run(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.archive_run(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn restore_run(state: State<'_, AppState>, id: String) -> Result<RunInfo, String> {
    state.restore_run(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_archived_runs(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<RunInfo>, String> {
    state.list_archived_runs(&project_id).map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Register them in `lib.rs`**

Add to the `tauri::generate_handler![ … ]` list (e.g. after `commands::rerun,`):

```rust
            commands::archive_run,
            commands::restore_run,
            commands::list_archived_runs,
```

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs
git commit -m "Expose archive/restore/list-archived commands"
```

---

### Task 5: Archive UI (button + Archived section)

**Files:**
- Modify: `ui/src/api.ts`
- Create: `ui/src/components/ArchivedSection.tsx`
- Modify: `ui/src/components/AgentFocus.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- Consumes: Task 4 commands.
- Produces: `archiveRun`, `restoreRun`, `listArchivedRuns`; `RunInfo.archivedAt`; `ArchivedSection`.

- [ ] **Step 1: Add the api wrappers**

In `ui/src/api.ts`, add to `RunInfo` (after `port: number | null;`):

```ts
  archivedAt: number | null;
```

Add (near the other run lifecycle wrappers):

```ts
export const archiveRun = (id: string) => invoke<void>("archive_run", { id });
export const restoreRun = (id: string) => invoke<RunInfo>("restore_run", { id });
export const listArchivedRuns = (projectId: string) =>
  invoke<RunInfo[]>("list_archived_runs", { projectId });
```

- [ ] **Step 2: Create `ArchivedSection.tsx`**

```tsx
import { useCallback, useEffect, useState } from "react";
import { RunInfo, listArchivedRuns, restoreRun, discardRun } from "../api";
import { useRuns } from "../store/runs";

export default function ArchivedSection() {
  const { selectedProjectId, refreshRuns, setFocusedRun } = useRuns();
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<RunInfo[]>([]);

  const load = useCallback(async () => {
    if (!selectedProjectId) return setItems([]);
    try {
      setItems(await listArchivedRuns(selectedProjectId));
    } catch {
      /* ignore */
    }
  }, [selectedProjectId]);

  useEffect(() => {
    if (open) load();
  }, [open, load]);

  if (!selectedProjectId) return null;

  return (
    <div className="archived">
      <button className="archived-head" onClick={() => setOpen((o) => !o)}>
        {open ? "▾" : "▸"} Archived{items.length ? ` (${items.length})` : ""}
      </button>
      {open &&
        items.map((r) => (
          <div key={r.id} className="archived-row">
            <span className="archived-name">{r.agent}: {r.prompt || r.branch}</span>
            <button
              className="icon-btn"
              title="Restore"
              onClick={async () => {
                const restored = await restoreRun(r.id);
                await refreshRuns();
                await load();
                setFocusedRun(restored.id);
              }}
            >↺</button>
            <button
              className="icon-btn danger"
              title="Discard permanently"
              onClick={async () => {
                await discardRun(r.id);
                await load();
              }}
            >✕</button>
          </div>
        ))}
      {open && items.length === 0 && <div className="archived-empty">Nothing archived.</div>}
    </div>
  );
}
```

- [ ] **Step 3: Wire Archive button + section into `AgentFocus`**

In `ui/src/components/AgentFocus.tsx`:

Add imports:

```ts
import { stopRun, discardRun, archiveRun } from "../api";
import ArchivedSection from "./ArchivedSection";
```

(That replaces the existing `import { stopRun, discardRun } from "../api";` line.)

Add an Archive button in the `focus-head`, between the Discard button and the Approve button:

```tsx
              <button className="tile-act" title="Archive agent" onClick={async () => {
                const id = focused.id;
                await archiveRun(id);
                setFocusedRun(null);
                await refreshRuns();
              }}>⌂ Archive</button>
```

Render `ArchivedSection` at the bottom of the rail — inside the `.rail` div, right after the `runs.map(…)` block (still inside the `railOpen` branch):

```tsx
            <ArchivedSection />
```

- [ ] **Step 4: Add styles**

Append to `ui/src/styles.css`:

```css
.archived { margin-top: 8px; border-top: 1px solid var(--border, #2a2d36); padding-top: 6px; }
.archived-head { width: 100%; text-align: left; background: transparent; color: var(--muted, #9aa); padding: 4px 8px; border: 0; }
.archived-row { display: flex; align-items: center; gap: 4px; padding: 2px 8px; }
.archived-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted, #9aa); font-size: 12px; }
.archived-empty { padding: 4px 8px; color: var(--muted, #9aa); font-size: 12px; }
```

- [ ] **Step 5: Typecheck + build**

Run: `cd ui && pnpm exec tsc --noEmit && pnpm build`
Expected: no type errors; build succeeds.

- [ ] **Step 6: Manual smoke test**

1. Spawn an agent, let it make a commit. Click **⌂ Archive** in the focus head.
2. Confirm: the run disappears from the active rail/grid; it appears under **Archived (1)** in the rail; the worktree dir under `.agency/worktrees/<id>` is gone but `git branch` still lists `agent/<id>`.
3. Click **↺ Restore** — confirm the worktree is re-created and the run returns to the active list, focused.
4. Archive it again, then **✕ Discard** from the Archived section — confirm the branch `agent/<id>` is gone.
5. With `[scripts].archive = "echo bye > /tmp/agency-archive-marker"` configured, archive a run and confirm the marker file was written (script ran before removal).
6. Archive a run holding a port, then spawn a new run — confirm the new run reclaims the archived run's port.

- [ ] **Step 7: Commit**

```bash
git add ui/src/api.ts ui/src/components/ArchivedSection.tsx ui/src/components/AgentFocus.tsx ui/src/styles.css
git commit -m "Add Archive action and Archived section in the focus view"
```

---

## Self-Review

**Spec coverage (Feature 3):**
- `archived_at` column + migration → Task 1. ✓
- Archive keeps branch, removes worktree, runs archive script, stamps timestamp → Tasks 2–3. ✓
- Restore re-creates worktree on kept branch, clears timestamp → Tasks 2–3. ✓
- `list_runs` hides archived; `list_archived_runs` added → Task 1. ✓
- Archived port reclaimable (narrowed `list_port_bases`) → Task 1. ✓
- Discarding an archived run deletes the branch (tolerant `remove`) → Task 2. ✓
- Archive button + Archived section (Restore/Discard) in focus view → Task 5. ✓
- Scrollback-not-preserved limitation honored (no attempt to persist it). ✓

**Placeholder scan:** none — every step carries concrete code/commands.

**Type consistency:** `archived_at: Option<i64>` on `Run` (Task 1) ↔ `RunInfo`/`run_info` carries it as `archivedAt: number | null` (Task 5). `set_archived(id, Option<i64>)` is used by `archive_run` (`Some(now_secs())`) and `restore_run` (`None`). `remove_keep_branch`/`restore` (Task 2) are consumed by `archive_run`/`restore_run` (Task 3). The 3 command names match across `commands.rs` (Task 4), `lib.rs` (Task 4), and `api.ts` (Task 5): `archive_run`/`restore_run`/`list_archived_runs`.

**Task dependencies:** 1→3 (queries), 2→3 (worktree ops), 3→4 (methods), 4→5 (commands). Strictly sequential.

**Deferred:** Archive button on the grid `AgentTile` (focus-view only here); notifications unaffected.
