# Agency Phase 3 Implementation Plan — Per-Worktree Git Panel

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a per-worktree source-control panel to Agency: see a task's changed files, view per-file diffs, stage/unstage, commit, push, and read the recent commit log — all scoped to that task's git worktree.

**Architecture:** A new Tauri-free git module `agency_core::git` provides functions that shell out to `git` against a worktree path (mirrors the existing `worktree` module's approach). `agency-app` exposes thin `#[tauri::command]` wrappers that resolve a `task_id` to its worktree path via `AppState` and call the git module. The frontend adds a `GitPanel` React component shown beside the terminal in the task board.

**Tech Stack:** Rust (std `Command` → `git`), Tauri v2, React + TypeScript.

## Global Constraints

- **Local-only / zero first-party data collection:** no network calls in Agency's own code. `git push` is a user-initiated git-remote operation (allowed); nothing else touches the network.
- **Git module is Tauri-free and testable** in `agency-core` (tested against temp repos). `#[tauri::command]`s stay thin.
- **Scope (Phase 3):** file-level stage/unstage, full-file diffs, recent commit log, commit, push. **Out of scope (deferred):** hunk-level staging; the approve→merge-to-main action (Phase 4, with the conflict-resolver); branch-only log filtering (Phase 3 shows recent commits).
- **Worktree/branch conventions** are owned by `agency_core::worktree` (`<repo>/.agency/worktrees/<task-id>`, branch `agent/<task-id>`). The git panel operates on an existing task's worktree (the session must exist).
- **Frontend↔Rust naming:** `agency-app` DTOs use `#[serde(rename_all = "camelCase")]`. Tauri maps camelCase JS args to snake_case Rust params.
- TDD for the Rust git module; commit after each green task.

---

## File Structure

```
crates/agency-core/
├── src/git.rs            # NEW: status/diff/log/stage/unstage/commit/push
├── src/lib.rs            # MODIFY: add `pub mod git;`
└── tests/git.rs          # NEW: integration tests against temp repos

crates/agency-app/
├── src/state.rs          # MODIFY: add worktree_path(task_id) resolver
├── src/commands.rs       # MODIFY: add git_* command wrappers + DTOs
└── src/lib.rs            # MODIFY: register the new commands

ui/src/
├── api.ts                # MODIFY: typed wrappers for git_* commands
├── components/GitPanel.tsx   # NEW
├── components/TaskBoard.tsx  # MODIFY: show GitPanel beside the terminal
└── styles.css            # MODIFY: git panel layout
```

---

### Task 1: `agency_core::git` — read operations (status, diff, log)

**Files:**
- Create: `crates/agency-core/src/git.rs`
- Modify: `crates/agency-core/src/lib.rs` (add `pub mod git;`)
- Test: `crates/agency-core/tests/git.rs`

**Interfaces:**
- Produces:
  - `struct FileChange { pub path: String, pub index: String, pub worktree: String }` (derives `Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize`) — `index`/`worktree` are the one-char git porcelain status codes (e.g. `"M"`, `"A"`, `"?"`, `" "`).
  - `struct CommitInfo { pub hash: String, pub summary: String }` (same derives).
  - `fn status(worktree: &Path) -> anyhow::Result<Vec<FileChange>>`
  - `fn diff(worktree: &Path, path: &str, staged: bool) -> anyhow::Result<String>`
  - `fn log(worktree: &Path, limit: usize) -> anyhow::Result<Vec<CommitInfo>>`

- [ ] **Step 1: Write the failing test**

Create `crates/agency-core/tests/git.rs`:

```rust
use agency_core::git;
use std::path::Path;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) {
    assert!(
        Command::new("git").args(args).current_dir(dir).status().unwrap().success(),
        "git {:?}",
        args
    );
}

fn init_repo(dir: &Path) {
    run(dir, &["init", "-q"]);
    run(dir, &["config", "user.email", "t@e.com"]);
    run(dir, &["config", "user.name", "T"]);
    std::fs::write(dir.join("tracked.txt"), "one\n").unwrap();
    run(dir, &["add", "-A"]);
    run(dir, &["commit", "-q", "-m", "initial"]);
}

#[test]
fn status_reports_modified_and_untracked() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "hi\n").unwrap();

    let changes = git::status(dir.path()).unwrap();
    let modified = changes.iter().find(|c| c.path == "tracked.txt").unwrap();
    assert_eq!(modified.worktree, "M");
    let untracked = changes.iter().find(|c| c.path == "new.txt").unwrap();
    assert_eq!(untracked.index, "?");
}

#[test]
fn diff_shows_unstaged_changes() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();

    let d = git::diff(dir.path(), "tracked.txt", false).unwrap();
    assert!(d.contains("+two"));
}

#[test]
fn log_lists_recent_commits() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let commits = git::log(dir.path(), 10).unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].summary, "initial");
    assert!(!commits[0].hash.is_empty());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core --test git`
Expected: FAIL — module `git` not found.

- [ ] **Step 3: Implement the read operations**

Create `crates/agency-core/src/git.rs`:

```rust
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    /// Staged (index) status code: one of M A D R C ? ! or space.
    pub index: String,
    /// Unstaged (worktree) status code.
    pub worktree: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitInfo {
    pub hash: String,
    pub summary: String,
}

/// Run a git command in `worktree`, returning stdout on success.
fn git(worktree: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree)
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

pub fn status(worktree: &Path) -> Result<Vec<FileChange>> {
    let out = git(worktree, &["status", "--porcelain"])?;
    let mut changes = Vec::new();
    for line in out.lines() {
        if line.len() < 4 {
            continue;
        }
        let index = line[0..1].to_string();
        let work = line[1..2].to_string();
        // Path begins at column 3 (after "XY ").
        let mut path = line[3..].to_string();
        // Renames are "orig -> new"; keep the new path.
        if let Some(idx) = path.find(" -> ") {
            path = path[idx + 4..].to_string();
        }
        changes.push(FileChange {
            path,
            index,
            worktree: work,
        });
    }
    Ok(changes)
}

pub fn diff(worktree: &Path, path: &str, staged: bool) -> Result<String> {
    if staged {
        git(worktree, &["diff", "--cached", "--", path])
    } else {
        git(worktree, &["diff", "--", path])
    }
}

pub fn log(worktree: &Path, limit: usize) -> Result<Vec<CommitInfo>> {
    let limit_arg = format!("-n{limit}");
    // Tab-separated hash\tsummary, one commit per line.
    let out = git(
        worktree,
        &["log", &limit_arg, "--format=%H%x09%s"],
    )?;
    let mut commits = Vec::new();
    for line in out.lines() {
        if let Some((hash, summary)) = line.split_once('\t') {
            commits.push(CommitInfo {
                hash: hash.to_string(),
                summary: summary.to_string(),
            });
        }
    }
    Ok(commits)
}
```

Add to `crates/agency-core/src/lib.rs` (alongside the other `pub mod` lines):

```rust
pub mod git;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agency-core --test git`
Expected: PASS (3 passed), no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/git.rs crates/agency-core/src/lib.rs crates/agency-core/tests/git.rs
git commit -m "Add git read operations (status, diff, log)"
```

---

### Task 2: `agency_core::git` — write operations (stage, unstage, commit, push)

**Files:**
- Modify: `crates/agency-core/src/git.rs`
- Test: `crates/agency-core/tests/git.rs`

**Interfaces:**
- Consumes: the `git()` helper from Task 1.
- Produces:
  - `fn stage(worktree: &Path, path: &str) -> anyhow::Result<()>`
  - `fn unstage(worktree: &Path, path: &str) -> anyhow::Result<()>`
  - `fn commit(worktree: &Path, message: &str) -> anyhow::Result<()>`
  - `fn push(worktree: &Path) -> anyhow::Result<()>` — pushes the current branch to `origin`, setting upstream.

- [ ] **Step 1: Write the failing test**

Append to `crates/agency-core/tests/git.rs`:

```rust
#[test]
fn stage_commit_then_log_and_push_to_local_remote() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    // Make a change, stage it, commit it.
    std::fs::write(repo.join("tracked.txt"), "one\ntwo\n").unwrap();
    git::stage(&repo, "tracked.txt").unwrap();

    let staged = git::status(&repo).unwrap();
    let entry = staged.iter().find(|c| c.path == "tracked.txt").unwrap();
    assert_eq!(entry.index, "M"); // staged modification

    git::unstage(&repo, "tracked.txt").unwrap();
    let unstaged = git::status(&repo).unwrap();
    let entry = unstaged.iter().find(|c| c.path == "tracked.txt").unwrap();
    assert_eq!(entry.index, " "); // no longer staged
    assert_eq!(entry.worktree, "M");

    git::stage(&repo, "tracked.txt").unwrap();
    git::commit(&repo, "add two").unwrap();
    let commits = git::log(&repo, 10).unwrap();
    assert_eq!(commits[0].summary, "add two");

    // Set up a bare remote and push to it.
    let remote = dir.path().join("remote.git");
    assert!(std::process::Command::new("git")
        .args(["init", "--bare", "-q", remote.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    run(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);

    git::push(&repo).unwrap();

    // The remote now has our branch with the commit.
    let ls = std::process::Command::new("git")
        .args(["log", "--format=%s", "-n1", "--all"])
        .current_dir(&remote)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&ls.stdout).contains("add two"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core --test git`
Expected: FAIL — `stage`/`unstage`/`commit`/`push` not found.

- [ ] **Step 3: Implement the write operations**

Append to `crates/agency-core/src/git.rs`:

```rust
pub fn stage(worktree: &Path, path: &str) -> Result<()> {
    git(worktree, &["add", "--", path])?;
    Ok(())
}

pub fn unstage(worktree: &Path, path: &str) -> Result<()> {
    // `restore --staged` requires git >= 2.23; available on all supported setups.
    git(worktree, &["restore", "--staged", "--", path])?;
    Ok(())
}

pub fn commit(worktree: &Path, message: &str) -> Result<()> {
    git(worktree, &["commit", "-m", message])?;
    Ok(())
}

pub fn push(worktree: &Path) -> Result<()> {
    let branch = git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    git(worktree, &["push", "-u", "origin", &branch])?;
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agency-core --test git`
Expected: PASS (4 passed), no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/git.rs crates/agency-core/tests/git.rs
git commit -m "Add git write operations (stage, unstage, commit, push)"
```

---

### Task 3: `agency-app` git command wrappers

**Files:**
- Modify: `crates/agency-app/src/state.rs` (add worktree-path resolver)
- Modify: `crates/agency-app/src/commands.rs` (git command wrappers + DTOs)
- Modify: `crates/agency-app/src/lib.rs` (register commands)

**Interfaces:**
- Consumes: `agency_core::git`, and `AppState` (sessions hold `repo_path`).
- Produces:
  - On `AppState`: `pub fn worktree_path(&self, task_id: &str) -> anyhow::Result<std::path::PathBuf>` — looks up the session by `task_id`, returns `repo_path/.agency/worktrees/<task_id>`. Errors if the task is unknown.
  - Commands (thin; map `anyhow::Error` → `String`): `git_status(task_id) -> Vec<FileChange>`, `git_diff(task_id, path, staged) -> String`, `git_log(task_id) -> Vec<CommitInfo>`, `git_stage(task_id, path)`, `git_unstage(task_id, path)`, `git_commit(task_id, message)`, `git_push(task_id)`. (`FileChange`/`CommitInfo` from `agency_core::git` are returned directly; they already derive `Serialize`.)
  - `git_log` uses a fixed limit of 100.

- [ ] **Step 1: Write the failing test (worktree-path resolver)**

Append to `crates/agency-app/tests/state.rs`:

```rust
#[test]
fn worktree_path_resolves_for_active_task() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo); // helper already defined earlier in this test file

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    state.register_profile(AgentProfile {
        name: "fake".into(),
        command: fake_agent_command(),
        args: vec!["{{prompt}}".into()],
        env: vec![],
    });
    let project = state.add_project("demo", &repo).unwrap();
    let info = state
        .start_task(&project.id, "p", "fake", "HEAD", |_| {})
        .unwrap();

    let wt = state.worktree_path(&info.task_id).unwrap();
    assert!(wt.ends_with(format!(".agency/worktrees/{}", info.task_id)));
    assert!(wt.exists());

    assert!(state.worktree_path("does-not-exist").is_err());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-app --test state`
Expected: FAIL — `worktree_path` not found.

- [ ] **Step 3: Implement the resolver on AppState**

Add to `impl AppState` in `crates/agency-app/src/state.rs`:

```rust
    pub fn worktree_path(&self, task_id: &str) -> anyhow::Result<std::path::PathBuf> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(task_id)
            .ok_or_else(|| anyhow::anyhow!("unknown task: {task_id}"))?;
        Ok(session
            .repo_path
            .join(".agency")
            .join("worktrees")
            .join(task_id))
    }
```

- [ ] **Step 4: Run the resolver test**

Run: `cargo test -p agency-app --test state`
Expected: PASS, no warnings.

- [ ] **Step 5: Add the git command wrappers**

Append to `crates/agency-app/src/commands.rs`:

```rust
use agency_core::git::{self, CommitInfo, FileChange};

#[tauri::command]
pub fn git_status(state: State<'_, AppState>, task_id: String) -> Result<Vec<FileChange>, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::status(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_diff(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    staged: bool,
) -> Result<String, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::diff(&wt, &path, staged).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_log(state: State<'_, AppState>, task_id: String) -> Result<Vec<CommitInfo>, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::log(&wt, 100).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_stage(state: State<'_, AppState>, task_id: String, path: String) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::stage(&wt, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_unstage(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::unstage(&wt, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_commit(
    state: State<'_, AppState>,
    task_id: String,
    message: String,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::commit(&wt, &message).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_push(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    git::push(&wt).map_err(|e| e.to_string())
}
```

- [ ] **Step 6: Register the commands**

In `crates/agency-app/src/lib.rs`, extend the `generate_handler!` list with the seven new commands:

```rust
        .invoke_handler(tauri::generate_handler![
            commands::list_projects,
            commands::add_project,
            commands::remove_project,
            commands::start_task,
            commands::send_input,
            commands::task_status,
            commands::stop_task,
            commands::git_status,
            commands::git_diff,
            commands::git_log,
            commands::git_stage,
            commands::git_unstage,
            commands::git_commit,
            commands::git_push,
        ])
```

- [ ] **Step 7: Build and test**

Run: `cargo build -p agency-app` (warning-free) and `cargo test -p agency-app`.
Expected: builds clean; existing + new state tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/agency-app/src/state.rs crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs crates/agency-app/tests/state.rs
git commit -m "Add git command wrappers and worktree-path resolver"
```

---

### Task 4: Frontend — git API layer + GitPanel component

**Files:**
- Modify: `ui/src/api.ts`
- Create: `ui/src/components/GitPanel.tsx`

**Interfaces:**
- Consumes: the Task 3 commands.
- Produces:
  - In `api.ts`: `FileChange { path; index; worktree }`, `CommitInfo { hash; summary }`, and wrappers `gitStatus(taskId)`, `gitDiff(taskId, path, staged)`, `gitLog(taskId)`, `gitStage(taskId, path)`, `gitUnstage(taskId, path)`, `gitCommit(taskId, message)`, `gitPush(taskId)`.
  - `GitPanel({ taskId }: { taskId: string })` — lists changed files (staged/unstaged), shows the selected file's diff, stage/unstage buttons, a commit message box + Commit, a Push button, a Refresh button, and the recent commit log.

- [ ] **Step 1: Add the API wrappers**

Append to `ui/src/api.ts`:

```ts
export interface FileChange {
  path: string;
  index: string; // staged status code
  worktree: string; // unstaged status code
}

export interface CommitInfo {
  hash: string;
  summary: string;
}

export const gitStatus = (taskId: string) =>
  invoke<FileChange[]>("git_status", { taskId });

export const gitDiff = (taskId: string, path: string, staged: boolean) =>
  invoke<string>("git_diff", { taskId, path, staged });

export const gitLog = (taskId: string) => invoke<CommitInfo[]>("git_log", { taskId });

export const gitStage = (taskId: string, path: string) =>
  invoke<void>("git_stage", { taskId, path });

export const gitUnstage = (taskId: string, path: string) =>
  invoke<void>("git_unstage", { taskId, path });

export const gitCommit = (taskId: string, message: string) =>
  invoke<void>("git_commit", { taskId, message });

export const gitPush = (taskId: string) => invoke<void>("git_push", { taskId });
```

- [ ] **Step 2: Create the GitPanel component**

Create `ui/src/components/GitPanel.tsx`:

```tsx
import { useCallback, useEffect, useState } from "react";
import {
  CommitInfo,
  FileChange,
  gitCommit,
  gitDiff,
  gitLog,
  gitPush,
  gitStage,
  gitStatus,
  gitUnstage,
} from "../api";

export default function GitPanel({ taskId }: { taskId: string }) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [commits, setCommits] = useState<CommitInfo[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState("");
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");

  const refresh = useCallback(async () => {
    try {
      setChanges(await gitStatus(taskId));
      setCommits(await gitLog(taskId));
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }, [taskId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function showDiff(file: FileChange) {
    setSelected(file.path);
    try {
      const staged = file.index !== " " && file.index !== "?";
      setDiff(await gitDiff(taskId, file.path, staged));
    } catch (e) {
      setDiff(String(e));
    }
  }

  async function act(fn: () => Promise<unknown>) {
    try {
      await fn();
      setError("");
    } catch (e) {
      setError(String(e));
    }
    await refresh();
  }

  const staged = changes.filter((c) => c.index !== " " && c.index !== "?");
  const unstaged = changes.filter((c) => c.index === " " || c.index === "?");

  return (
    <div className="git-panel">
      <div className="git-header">
        <h3>Changes</h3>
        <button onClick={refresh}>Refresh</button>
      </div>
      {error && <div className="git-error">{error}</div>}

      <div className="git-section">
        <h4>Staged</h4>
        {staged.length === 0 && <div className="git-empty">none</div>}
        {staged.map((c) => (
          <div key={c.path} className="git-file">
            <span className="git-status">{c.index}</span>
            <span className="git-path" onClick={() => showDiff(c)}>
              {c.path}
            </span>
            <button onClick={() => act(() => gitUnstage(taskId, c.path))}>−</button>
          </div>
        ))}
      </div>

      <div className="git-section">
        <h4>Unstaged</h4>
        {unstaged.length === 0 && <div className="git-empty">none</div>}
        {unstaged.map((c) => (
          <div key={c.path} className="git-file">
            <span className="git-status">{c.index === "?" ? "?" : c.worktree}</span>
            <span className="git-path" onClick={() => showDiff(c)}>
              {c.path}
            </span>
            <button onClick={() => act(() => gitStage(taskId, c.path))}>+</button>
          </div>
        ))}
      </div>

      <div className="git-commit">
        <textarea
          placeholder="Commit message"
          value={message}
          onChange={(e) => setMessage(e.target.value)}
        />
        <div className="git-actions">
          <button
            onClick={() =>
              act(async () => {
                await gitCommit(taskId, message);
                setMessage("");
              })
            }
          >
            Commit
          </button>
          <button onClick={() => act(() => gitPush(taskId))}>Push</button>
        </div>
      </div>

      {selected && (
        <div className="git-diff">
          <h4>{selected}</h4>
          <pre>{diff || "(no diff)"}</pre>
        </div>
      )}

      <div className="git-section">
        <h4>Recent commits</h4>
        {commits.map((c) => (
          <div key={c.hash} className="git-commit-row">
            <code>{c.hash.slice(0, 7)}</code> <span>{c.summary}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Verify the frontend builds**

Run: `pnpm --dir ui build`
Expected: `tsc && vite build` completes with no type errors. (`GitPanel` is not yet mounted; this verifies types/imports.)

- [ ] **Step 4: Commit**

```bash
git add ui/src/api.ts ui/src/components/GitPanel.tsx
git commit -m "Add git API wrappers and GitPanel component"
```

---

### Task 5: Integrate GitPanel into the task board + styles

**Files:**
- Modify: `ui/src/components/TaskBoard.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- Consumes: `GitPanel`, and the `TaskInfo.taskId` that `TerminalPane` already obtains from `startTask`.
- Produces: when a task is running, the board shows the terminal and the `GitPanel` side by side. This requires the running task's `taskId` to be lifted out of `TerminalPane` so `TaskBoard` can pass it to `GitPanel`.

- [ ] **Step 1: Lift the started taskId up via a callback**

In `ui/src/components/TerminalPane.tsx`, add an optional `onStarted` prop and call it once the task starts. Change the `Props` interface and the `.then(...)`:

```tsx
interface Props {
  projectId: string;
  prompt: string;
  onStatus: (label: string) => void;
  onStarted?: (taskId: string) => void;
}
```

Inside the `startTask(...).then((i) => { ... })` callback, after `info = i;`, add:

```tsx
      onStarted?.(i.taskId);
```

And add `onStarted` to the effect dependency array (it is optional and stable when passed a `useCallback`/setter):

```tsx
  }, [projectId, prompt, onStatus, onStarted]);
```

- [ ] **Step 2: Render GitPanel beside the terminal**

Replace the body of `ui/src/components/TaskBoard.tsx` with:

```tsx
import { useState } from "react";
import { Project } from "../api";
import TerminalPane from "./TerminalPane";
import GitPanel from "./GitPanel";

interface Props {
  project: Project;
}

export default function TaskBoard({ project }: Props) {
  const [prompt, setPrompt] = useState("");
  const [activePrompt, setActivePrompt] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const [taskId, setTaskId] = useState<string | null>(null);

  function closeTask() {
    setActivePrompt(null);
    setTaskId(null);
    setStatus("");
  }

  return (
    <main className="board">
      <header className="board-header">
        <h2>{project.name}</h2>
        <code>{project.repo_path}</code>
      </header>
      {activePrompt === null ? (
        <div className="new-task">
          <textarea
            placeholder="Task prompt (the shell profile ignores it for now; real agents will use it)"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
          />
          <button onClick={() => setActivePrompt(prompt)}>Start task</button>
        </div>
      ) : (
        <div className="task-running">
          <div className="task-status">status: {status || "starting…"}</div>
          <div className="task-split">
            <TerminalPane
              key={`${project.id}:${activePrompt}`}
              projectId={project.id}
              prompt={activePrompt}
              onStatus={setStatus}
              onStarted={setTaskId}
            />
            {taskId && <GitPanel taskId={taskId} />}
          </div>
          <button onClick={closeTask}>Close terminal</button>
        </div>
      )}
    </main>
  );
}
```

- [ ] **Step 3: Add layout styles**

Append to `ui/src/styles.css`:

```css
.task-split { display: flex; gap: 12px; flex: 1; min-height: 0; }
.task-split .terminal { flex: 2; }
.git-panel { flex: 1; min-width: 280px; max-width: 420px; overflow: auto; background: #1b1f27; border: 1px solid #2a2e36; border-radius: 8px; padding: 10px; display: flex; flex-direction: column; gap: 10px; }
.git-header { display: flex; justify-content: space-between; align-items: center; }
.git-header h3 { margin: 0; font-size: 13px; text-transform: uppercase; letter-spacing: 0.06em; color: #9aa3b2; }
.git-section h4 { margin: 4px 0; font-size: 12px; color: #8893a5; }
.git-empty { color: #5b6472; font-size: 12px; }
.git-file { display: flex; align-items: center; gap: 6px; font-size: 13px; padding: 2px 0; }
.git-status { width: 14px; color: #d8a657; font-family: ui-monospace, monospace; }
.git-path { flex: 1; cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.git-path:hover { text-decoration: underline; }
.git-file button { padding: 0 8px; }
.git-commit { display: flex; flex-direction: column; gap: 6px; }
.git-commit textarea { background: #14161a; border: 1px solid #2a2e36; color: #e6e6e6; border-radius: 6px; padding: 6px; min-height: 56px; resize: vertical; }
.git-actions { display: flex; gap: 6px; }
.git-error { color: #e06c75; font-size: 12px; white-space: pre-wrap; }
.git-diff pre { background: #0e1014; border-radius: 6px; padding: 8px; overflow: auto; max-height: 240px; font-size: 12px; }
.git-commit-row { font-size: 12px; padding: 1px 0; }
.git-commit-row code { color: #7da3c9; }
```

- [ ] **Step 4: Verify the frontend builds and the unit tests pass**

Run: `pnpm --dir ui build` and `pnpm --dir ui test`.
Expected: clean build, vitest still green.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/TerminalPane.tsx ui/src/components/TaskBoard.tsx ui/src/styles.css
git commit -m "Show git panel beside the terminal in the task board"
```

---

## Self-Review

**Spec coverage (Phase 3 scope):**
- Per-worktree status + per-file diff → Tasks 1, 3, 4. ✓
- Recent commit log → Tasks 1, 3, 4. ✓
- Stage/unstage (file-level) → Tasks 2, 3, 4. ✓
- Commit → Tasks 2, 3, 4. ✓
- Push → Tasks 2, 3, 4 (tested against a local bare remote). ✓
- Panel shown beside the terminal → Task 5. ✓
- Deferred (documented): hunk-level staging; approve→merge (Phase 4); branch-only log filtering; untracked-file diffs (untracked files appear in the list and can be staged, but `git diff` shows their content only after staging — acceptable v1).

**Placeholder scan:** No TBD/TODO; every code step has complete code. The Task 2 test's `remote_parent_init` helper is explicitly flagged as optional scaffolding with a simpler alternative.

**Type consistency:** `FileChange`/`CommitInfo` are defined once in `agency_core::git` (Serialize) and returned directly through the Tauri commands; the TS interfaces mirror them. `git_*` command names and camelCase args match `api.ts`. `worktree_path` is defined in Task 3 and consumed by every git command. `onStarted` added to `TerminalPane` in Task 5 matches the call in `TaskBoard`.

**Notes for the executor:**
- Requires `git` on PATH (present).
- The git panel operates on an **active** task (a live session). Reviewing a stopped task's changes is out of scope for Phase 3.
- Do not modify `agency-core`'s existing modules beyond adding `pub mod git;`.
