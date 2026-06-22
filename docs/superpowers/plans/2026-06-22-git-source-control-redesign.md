# Git Source-Control Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the duplicated `SourceControl.tsx` + `GitReviewPanel.tsx` with one shared, VSCode-parity git component (`GitPanel`) rendered in two layouts (compact sidebar / full tab), adding side-by-side syntax-highlighted diffs, line-level staging, and a commit-history graph.

**Architecture:** A Tauri app — React/Vite SPA in the webview talks to Rust `#[tauri::command]` functions via `invoke()`. The graph, diff parsing, and syntax highlighting run entirely in the frontend; Rust grows only to surface richer git data (parents, refs, author/date, ahead/behind, commit contents) and the missing write actions (discard, stage-all, line staging, amend). Tested logic lives in pure `.ts` modules (`status.ts`, `diffModel.ts`, `graph.ts`) per the codebase's existing pattern; `.tsx` components stay thin.

**Tech Stack:** Rust + `git` CLI (`agency-core/src/git.rs`); React 18 + TypeScript + Vite; Vitest (node env, no DOM); Shiki (highlighting, Catppuccin Mocha theme); jsdiff (intra-line word diff).

## Global Constraints

- Frontend tests run in Vitest **node environment** — no DOM, no `@testing-library`. Test pure functions only; keep `.tsx` thin and untested (matches existing `repoSetupView.ts` / `RepoSetupDialog.tsx` split).
- All git write/read goes through Rust commands resolved by `state.worktree_path(&task_id)`; never shell out from the frontend.
- Every new Tauri command must be registered in `crates/agency-app/src/lib.rs` inside `generate_handler![...]`.
- Theme: use existing CSS variables from `ui/src/theme.css` (`--green`, `--red`, `--yellow`, `--teal`, `--peach`, `--pink`, `--mauve`, `--blue`, `--o0`, etc.). No raw hex colors in new CSS.
- Rust git helpers return `anyhow::Result<T>`; commands map errors with `.map_err(|e| e.to_string())` and return `Result<T, String>`.
- Commit message rule: do NOT add AI attribution / `Co-Authored-By` trailers to commits.
- Spec: `docs/superpowers/specs/2026-06-22-git-source-control-redesign-design.md`.

---

## File Structure

**Rust (modify):**
- `crates/agency-core/src/git.rs` — add status/graph/commit/line-staging helpers + new structs.
- `crates/agency-app/src/commands.rs` — add `#[tauri::command]` wrappers.
- `crates/agency-app/src/lib.rs` — register new commands.
- `crates/agency-core/tests/git.rs` — add helper + new tests.

**Frontend (create):**
- `ui/src/components/git/status.ts` + `status.test.ts`
- `ui/src/components/git/diffModel.ts` + `diffModel.test.ts`
- `ui/src/components/git/graph.ts` + `graph.test.ts`
- `ui/src/components/git/highlight.ts`
- `ui/src/components/git/FileRow.tsx`
- `ui/src/components/git/ResourceGroup.tsx`
- `ui/src/components/git/CommitBox.tsx`
- `ui/src/components/git/BranchBar.tsx`
- `ui/src/components/git/DiffViewer.tsx`
- `ui/src/components/git/Graph.tsx`
- `ui/src/components/git/CommitRow.tsx`
- `ui/src/components/git/HistoryPanel.tsx`
- `ui/src/components/git/CommitDetail.tsx`
- `ui/src/components/git/ChangesPanel.tsx`
- `ui/src/components/git/GitPanel.tsx`

**Frontend (modify):**
- `ui/src/api.ts` — new types + `invoke` wrappers; remove `projectLog`.
- `ui/src/components/AgentsView.tsx` — render `GitPanel` in both surfaces.
- `ui/src/styles.css` — `git-*` namespace rules; remove stale `sc-*`/`hist-*`/`git-status-*`.

**Frontend (delete):** `ui/src/components/SourceControl.tsx`, `ui/src/components/GitReviewPanel.tsx`, `ui/src/components/DiffView.tsx`.

---

## PHASE 1 — Backend (Rust git helpers + commands)

### Task 1: Stage-all / unstage-all / discard helpers

**Files:**
- Modify: `crates/agency-core/src/git.rs`
- Modify: `crates/agency-app/src/commands.rs`
- Modify: `crates/agency-app/src/lib.rs`
- Modify: `ui/src/api.ts`
- Test: `crates/agency-core/tests/git.rs`

**Interfaces:**
- Produces (Rust): `git::stage_all(&Path) -> Result<()>`, `git::unstage_all(&Path) -> Result<()>`, `git::discard(&Path, path: &str, untracked: bool) -> Result<()>`, `git::discard_all(&Path) -> Result<()>`.
- Produces (commands): `git_stage_all`, `git_unstage_all`, `git_discard`, `git_discard_all` (all take `task_id`, return `Result<(), String>`; `git_discard` also takes `path: String`, `untracked: bool`).
- Produces (TS): `gitStageAll(taskId)`, `gitUnstageAll(taskId)`, `gitDiscard(taskId, path, untracked)`, `gitDiscardAll(taskId)`.

- [ ] **Step 1: Write failing tests**

Append to `crates/agency-core/tests/git.rs`:
```rust
#[test]
fn stage_all_stages_everything() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "hi\n").unwrap();
    git::stage_all(dir.path()).unwrap();
    let changes = git::status(dir.path()).unwrap();
    assert!(changes.iter().all(|c| c.index != " " && c.index != "?"),
        "all changes staged: {changes:?}");
}

#[test]
fn unstage_all_clears_index() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    git::stage_all(dir.path()).unwrap();
    git::unstage_all(dir.path()).unwrap();
    let changes = git::status(dir.path()).unwrap();
    let m = changes.iter().find(|c| c.path == "tracked.txt").unwrap();
    assert_eq!(m.worktree, "M");
    assert_eq!(m.index, " ");
}

#[test]
fn discard_reverts_tracked_and_deletes_untracked() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\nchanged\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "hi\n").unwrap();
    git::discard(dir.path(), "tracked.txt", false).unwrap();
    git::discard(dir.path(), "new.txt", true).unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join("tracked.txt")).unwrap(), "one\n");
    assert!(!dir.path().join("new.txt").exists());
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p agency-core --test git stage_all_stages_everything unstage_all_clears_index discard_reverts`
Expected: FAIL — `no function or associated item named 'stage_all'`.

- [ ] **Step 3: Implement helpers**

Append to `crates/agency-core/src/git.rs`:
```rust
pub fn stage_all(worktree: &Path) -> Result<()> {
    git(worktree, &["add", "-A"])?;
    Ok(())
}

pub fn unstage_all(worktree: &Path) -> Result<()> {
    git(worktree, &["reset", "-q"])?;
    Ok(())
}

/// Discard one file's changes. Untracked files are deleted; tracked files are
/// restored from the index (mirrors VSCode "Discard Changes" on the Changes group).
pub fn discard(worktree: &Path, path: &str, untracked: bool) -> Result<()> {
    if untracked {
        git(worktree, &["clean", "-f", "--", path])?;
    } else {
        git(worktree, &["restore", "--", path])?;
    }
    Ok(())
}

pub fn discard_all(worktree: &Path) -> Result<()> {
    git(worktree, &["restore", "--", "."])?;
    Ok(())
}
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test -p agency-core --test git stage_all_stages_everything unstage_all_clears_index discard_reverts`
Expected: PASS (3 tests).

- [ ] **Step 5: Add Tauri commands**

In `crates/agency-app/src/commands.rs`, after `git_unstage`:
```rust
#[tauri::command]
pub fn git_stage_all(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stage_all(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_unstage_all(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::unstage_all(&wt).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_discard(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    untracked: bool,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::discard(&wt, &path, untracked).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_discard_all(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::discard_all(&wt).map_err(|e| e.to_string())
}
```

- [ ] **Step 6: Register commands**

In `crates/agency-app/src/lib.rs` `generate_handler![...]`, after `commands::git_unstage,`:
```rust
            commands::git_stage_all,
            commands::git_unstage_all,
            commands::git_discard,
            commands::git_discard_all,
```

- [ ] **Step 7: Add TS wrappers**

In `ui/src/api.ts`, after the `gitUnstage` export:
```typescript
export const gitStageAll = (taskId: string) => invoke<void>("git_stage_all", { taskId });
export const gitUnstageAll = (taskId: string) => invoke<void>("git_unstage_all", { taskId });
export const gitDiscard = (taskId: string, path: string, untracked: boolean) =>
  invoke<void>("git_discard", { taskId, path, untracked });
export const gitDiscardAll = (taskId: string) => invoke<void>("git_discard_all", { taskId });
```

- [ ] **Step 8: Build check + commit**

Run: `cargo build -p agency-app && (cd ui && npx tsc --noEmit)`
Expected: both succeed.
```bash
git add crates/agency-core/src/git.rs crates/agency-core/tests/git.rs crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs ui/src/api.ts
git commit -m "Add stage-all/unstage-all/discard git commands"
```

---

### Task 2: Rich commit log (`log_graph`) + branch info

**Files:**
- Modify: `crates/agency-core/src/git.rs`, `crates/agency-app/src/commands.rs`, `crates/agency-app/src/lib.rs`, `ui/src/api.ts`
- Test: `crates/agency-core/tests/git.rs`

**Interfaces:**
- Produces (Rust structs): `HistoryItem { hash: String, parents: Vec<String>, author: String, email: String, date: i64, subject: String, refs: Vec<String> }`, `BranchInfo { branch: String, upstream: Option<String>, ahead: u32, behind: u32, base: Option<String> }`.
- Produces (Rust fns): `git::log_graph(&Path, limit: usize) -> Result<Vec<HistoryItem>>`, `git::branch_info(&Path) -> Result<BranchInfo>`.
- Produces (commands): `git_log_graph(task_id, limit) -> Result<Vec<HistoryItem>, String>`, `git_branch_info(task_id) -> Result<BranchInfo, String>`.
- Produces (TS types): `HistoryItem { hash; parents: string[]; author; email; date; subject; refs: string[] }`, `BranchInfo { branch; upstream: string | null; ahead; behind; base: string | null }`.
- Produces (TS fns): `gitLogGraph(taskId, limit)`, `gitBranchInfo(taskId)`.

- [ ] **Step 1: Write failing tests**

Append to `crates/agency-core/tests/git.rs`:
```rust
#[test]
fn log_graph_returns_parents_and_subject() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    run(dir.path(), &["commit", "-aqm", "second commit"]);
    let items = git::log_graph(dir.path(), 10).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].subject, "second commit");
    assert_eq!(items[0].parents.len(), 1, "second has one parent");
    assert_eq!(items[0].parents[0], items[1].hash);
    assert!(items[1].parents.is_empty(), "root has no parent");
    assert_eq!(items[0].author, "T");
}

#[test]
fn log_graph_captures_refs() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let items = git::log_graph(dir.path(), 10).unwrap();
    assert!(items[0].refs.iter().any(|r| r.contains("HEAD") || r.contains("master") || r.contains("main")),
        "head commit carries a ref: {:?}", items[0].refs);
}

#[test]
fn branch_info_reports_branch_and_no_upstream() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let info = git::branch_info(dir.path()).unwrap();
    assert!(info.branch == "master" || info.branch == "main");
    assert!(info.upstream.is_none());
    assert_eq!(info.ahead, 0);
    assert_eq!(info.behind, 0);
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p agency-core --test git log_graph branch_info`
Expected: FAIL — `log_graph` / `branch_info` not found.

- [ ] **Step 3: Implement structs + helpers**

Append to `crates/agency-core/src/git.rs`:
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub date: i64,
    pub subject: String,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BranchInfo {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub base: Option<String>,
}

/// HEAD ancestry, newest first. Fields are unit-separated (\x1f); parents and
/// refs are space/comma lists. %D yields "HEAD -> main, origin/main, tag: v1".
pub fn log_graph(worktree: &Path, limit: usize) -> Result<Vec<HistoryItem>> {
    let limit_arg = format!("-n{limit}");
    let format = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D";
    let out = git(worktree, &["log", &limit_arg, format])?;
    let mut items = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split('\u{1f}').collect();
        if f.len() < 7 {
            continue;
        }
        let parents = f[1].split_whitespace().map(str::to_string).collect();
        let refs = f[6]
            .split(',')
            .map(|r| r.trim().trim_start_matches("HEAD -> ").to_string())
            .filter(|r| !r.is_empty())
            .collect();
        items.push(HistoryItem {
            hash: f[0].to_string(),
            parents,
            author: f[2].to_string(),
            email: f[3].to_string(),
            date: f[4].parse().unwrap_or(0),
            subject: f[5].to_string(),
            refs,
        });
    }
    Ok(items)
}

pub fn branch_info(worktree: &Path) -> Result<BranchInfo> {
    let branch = git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    let upstream = git(worktree, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let (mut ahead, mut behind) = (0, 0);
    if upstream.is_some() {
        if let Ok(counts) = git(worktree, &["rev-list", "--left-right", "--count", "@{u}...HEAD"]) {
            let mut p = counts.split_whitespace();
            behind = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            ahead = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        }
    }
    // Base = merge-base with the first reachable default branch.
    let base = ["origin/HEAD", "main", "master"].iter().find_map(|cand| {
        git(worktree, &["merge-base", "HEAD", cand])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    });
    Ok(BranchInfo { branch, upstream, ahead, behind, base })
}
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test -p agency-core --test git log_graph branch_info`
Expected: PASS (3 tests).

- [ ] **Step 5: Add Tauri commands**

In `crates/agency-app/src/commands.rs`, after the line-staging/hunk commands area (after `git_unstage_hunk`):
```rust
#[tauri::command]
pub fn git_log_graph(
    state: State<'_, AppState>,
    task_id: String,
    limit: usize,
) -> Result<Vec<agency_core::git::HistoryItem>, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::log_graph(&wt, limit).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_branch_info(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<agency_core::git::BranchInfo, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::branch_info(&wt).map_err(|e| e.to_string())
}
```

- [ ] **Step 6: Register commands**

In `lib.rs` `generate_handler![...]`:
```rust
            commands::git_log_graph,
            commands::git_branch_info,
```

- [ ] **Step 7: Add TS types + wrappers**

In `ui/src/api.ts`, replace the `projectLog` export with:
```typescript
export interface HistoryItem {
  hash: string;
  parents: string[];
  author: string;
  email: string;
  date: number;
  subject: string;
  refs: string[];
}

export interface BranchInfo {
  branch: string;
  upstream: string | null;
  ahead: number;
  behind: number;
  base: string | null;
}

export const gitLogGraph = (taskId: string, limit: number) =>
  invoke<HistoryItem[]>("git_log_graph", { taskId, limit });
export const gitBranchInfo = (taskId: string) =>
  invoke<BranchInfo>("git_branch_info", { taskId });
```
Also remove the `project_log` command from `lib.rs` and the `project_log` Tauri command from `commands.rs` (no longer used).

- [ ] **Step 8: Build check + commit**

Run: `cargo build -p agency-app && (cd ui && npx tsc --noEmit)`
Note: `tsc` will flag `projectLog` usages in `SourceControl.tsx` — those files are deleted in Task 13. If blocking, temporarily comment the `histProject`/`projectLog` lines in `SourceControl.tsx`; they are removed wholesale later.
Expected: `cargo build` succeeds.
```bash
git add -A
git commit -m "Add git_log_graph and git_branch_info commands; drop project_log"
```

---

### Task 3: Commit contents (`commit_files`, `commit_diff`) + amend

**Files:**
- Modify: `crates/agency-core/src/git.rs`, `crates/agency-app/src/commands.rs`, `crates/agency-app/src/lib.rs`, `ui/src/api.ts`
- Test: `crates/agency-core/tests/git.rs`

**Interfaces:**
- Produces (Rust struct): `CommitFile { path: String, status: String }`.
- Produces (Rust fns): `git::commit_files(&Path, hash: &str) -> Result<Vec<CommitFile>>`, `git::commit_diff(&Path, hash: &str, path: &str) -> Result<String>`, `git::commit_amend(&Path, message: &str) -> Result<()>`.
- Produces (commands): `git_commit_files(task_id, hash)`, `git_commit_diff(task_id, hash, path)`, `git_commit_amend(task_id, message)`.
- Produces (TS): `CommitFile { path; status }`; `gitCommitFiles(taskId, hash)`, `gitCommitDiff(taskId, hash, path)`, `gitCommitAmend(taskId, message)`.

- [ ] **Step 1: Write failing tests**

Append to `crates/agency-core/tests/git.rs`:
```rust
#[test]
fn commit_files_lists_changed_paths() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.path().join("added.txt"), "x\n").unwrap();
    run(dir.path(), &["add", "-A"]);
    run(dir.path(), &["commit", "-qm", "c2"]);
    let head = git::log_graph(dir.path(), 1).unwrap()[0].hash.clone();
    let files = git::commit_files(dir.path(), &head).unwrap();
    assert!(files.iter().any(|f| f.path == "tracked.txt" && f.status == "M"));
    assert!(files.iter().any(|f| f.path == "added.txt" && f.status == "A"));
}

#[test]
fn commit_diff_shows_file_diff() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\n").unwrap();
    run(dir.path(), &["commit", "-aqm", "c2"]);
    let head = git::log_graph(dir.path(), 1).unwrap()[0].hash.clone();
    let diff = git::commit_diff(dir.path(), &head, "tracked.txt").unwrap();
    assert!(diff.contains("+two"), "diff shows added line: {diff}");
}
```

- [ ] **Step 2: Run tests, verify fail**

Run: `cargo test -p agency-core --test git commit_files commit_diff`
Expected: FAIL — not found.

- [ ] **Step 3: Implement**

Append to `crates/agency-core/src/git.rs`:
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitFile {
    pub path: String,
    pub status: String,
}

pub fn commit_files(worktree: &Path, hash: &str) -> Result<Vec<CommitFile>> {
    // --format= strips commit metadata; root commits are handled by `show`.
    let out = git(worktree, &["show", "--name-status", "--format=", hash])?;
    let mut files = Vec::new();
    for line in out.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let status = parts.next().unwrap_or("").chars().next().unwrap_or(' ').to_string();
        // Renames have two paths "R100 old new"; keep the last.
        let path = parts.last().unwrap_or("").to_string();
        if !path.is_empty() {
            files.push(CommitFile { path, status });
        }
    }
    Ok(files)
}

pub fn commit_diff(worktree: &Path, hash: &str, path: &str) -> Result<String> {
    git(worktree, &["show", "--format=", hash, "--", path])
}

pub fn commit_amend(worktree: &Path, message: &str) -> Result<()> {
    git(worktree, &["commit", "--amend", "-m", message])?;
    Ok(())
}
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test -p agency-core --test git commit_files commit_diff`
Expected: PASS.

- [ ] **Step 5: Commands**

In `commands.rs`:
```rust
#[tauri::command]
pub fn git_commit_files(
    state: State<'_, AppState>,
    task_id: String,
    hash: String,
) -> Result<Vec<agency_core::git::CommitFile>, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::commit_files(&wt, &hash).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_commit_diff(
    state: State<'_, AppState>,
    task_id: String,
    hash: String,
    path: String,
) -> Result<String, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::commit_diff(&wt, &hash, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_commit_amend(
    state: State<'_, AppState>,
    task_id: String,
    message: String,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::commit_amend(&wt, &message).map_err(|e| e.to_string())
}
```

- [ ] **Step 6: Register** in `lib.rs`:
```rust
            commands::git_commit_files,
            commands::git_commit_diff,
            commands::git_commit_amend,
```

- [ ] **Step 7: TS wrappers** in `ui/src/api.ts`:
```typescript
export interface CommitFile {
  path: string;
  status: string;
}

export const gitCommitFiles = (taskId: string, hash: string) =>
  invoke<CommitFile[]>("git_commit_files", { taskId, hash });
export const gitCommitDiff = (taskId: string, hash: string, path: string) =>
  invoke<string>("git_commit_diff", { taskId, hash, path });
export const gitCommitAmend = (taskId: string, message: string) =>
  invoke<void>("git_commit_amend", { taskId, message });
```

- [ ] **Step 8: Build + commit**

Run: `cargo build -p agency-app && (cd ui && npx tsc --noEmit)`
```bash
git add -A
git commit -m "Add commit contents (files/diff) and amend commands"
```

---

### Task 4: Line/selection-level staging

**Files:**
- Modify: `crates/agency-core/src/git.rs`, `crates/agency-app/src/commands.rs`, `crates/agency-app/src/lib.rs`, `ui/src/api.ts`
- Test: `crates/agency-core/tests/git.rs`

**Interfaces:**
- Consumes: `parse_diff`, `diff`, `git_stdin`, `FileDiff`, `Hunk` (existing in `git.rs`).
- Produces (Rust): `git::build_partial_patch(fd: &FileDiff, hunk_index: usize, selected: &[usize], reverse: bool) -> Result<String>`; `git::stage_lines(&Path, path, hunk_index, selected: &[usize])`, `git::unstage_lines(&Path, path, hunk_index, selected: &[usize])`, `git::revert_lines(&Path, path, hunk_index, selected: &[usize])`.
- Produces (commands/TS): `git_stage_lines` / `git_unstage_lines` / `git_revert_lines` (args `task_id, path, hunk_index: usize, lines: Vec<usize>`); `gitStageLines(taskId, path, hunkIndex, lines)`, `gitUnstageLines(...)`, `gitRevertLines(...)`.

Note: `selected` indices are positions within `hunk.lines` (0-based, body lines only, excluding the `@@` header).

- [ ] **Step 1: Write failing tests**

Append to `crates/agency-core/tests/git.rs`:
```rust
#[test]
fn build_partial_patch_keeps_selected_add_drops_others() {
    // Hunk adds two lines after context; select only the first added line (index 1).
    let fd = parse_diff(
        "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,3 @@\n one\n+two\n+three\n",
    );
    let patch = git::build_partial_patch(&fd, 0, &[1], false).unwrap();
    assert!(patch.contains("+two"));
    assert!(!patch.contains("+three"), "unselected add dropped: {patch}");
    assert!(patch.contains("@@ -1,1 +1,2 @@"), "recomputed header: {patch}");
}

#[test]
fn stage_lines_stages_only_selection() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), "one\ntwo\nthree\n").unwrap();
    // working diff: hunk index 0 adds "two" (idx 1) and "three" (idx 2) after "one".
    let raw = git::diff(dir.path(), "tracked.txt", false).unwrap();
    let fd = parse_diff(&raw);
    // Select the first added body line only.
    let add_idx = fd.hunks[0].lines.iter().position(|l| l.starts_with("+two")).unwrap();
    git::stage_lines(dir.path(), "tracked.txt", 0, &[add_idx]).unwrap();
    let staged = git::diff(dir.path(), "tracked.txt", true).unwrap();
    assert!(staged.contains("+two"));
    assert!(!staged.contains("+three"), "only selection staged: {staged}");
}
```

- [ ] **Step 2: Run tests, verify fail**

Run: `cargo test -p agency-core --test git build_partial_patch stage_lines`
Expected: FAIL — not found.

- [ ] **Step 3: Implement builder + wrappers**

Append to `crates/agency-core/src/git.rs`:
```rust
/// Build a patch applying only `selected` body lines of one hunk.
/// Unselected '+' lines are dropped; unselected '-' lines become context so
/// they are preserved. The hunk header counts are recomputed. When `reverse`
/// is true (unstaging the cached diff) the roles of +/- are swapped for counting.
pub fn build_partial_patch(
    fd: &FileDiff,
    hunk_index: usize,
    selected: &[usize],
    reverse: bool,
) -> Result<String> {
    let hunk = fd
        .hunks
        .get(hunk_index)
        .ok_or_else(|| anyhow::anyhow!("hunk index {hunk_index} out of range"))?;
    // Parse "@@ -old_start,old_len +new_start,new_len @@".
    let (old_start, new_start) = parse_hunk_starts(&hunk.header)?;
    let mut body: Vec<String> = Vec::new();
    let (mut old_len, mut new_len) = (0u32, 0u32);
    for (i, line) in hunk.lines.iter().enumerate() {
        let kind = line.chars().next().unwrap_or(' ');
        let sel = selected.contains(&i);
        match kind {
            '+' => {
                if sel {
                    body.push(line.clone());
                    new_len += 1;
                }
                // unselected add: drop entirely
            }
            '-' => {
                if sel {
                    body.push(line.clone());
                    old_len += 1;
                } else {
                    // keep as context
                    body.push(format!(" {}", &line[1..]));
                    old_len += 1;
                    new_len += 1;
                }
            }
            _ => {
                body.push(line.clone());
                old_len += 1;
                new_len += 1;
            }
        }
    }
    let _ = reverse; // counts are symmetric for our construction
    let mut patch = fd.header.clone();
    patch.push_str(&format!(
        "@@ -{old_start},{old_len} +{new_start},{new_len} @@\n"
    ));
    for l in body {
        patch.push_str(&l);
        patch.push('\n');
    }
    Ok(patch)
}

fn parse_hunk_starts(header: &str) -> Result<(u32, u32)> {
    // header like "@@ -1,1 +1,3 @@ optional"
    let core = header.trim_start_matches("@@").trim();
    let mut parts = core.split_whitespace();
    let old = parts.next().unwrap_or("");
    let new = parts.next().unwrap_or("");
    let old_start = old.trim_start_matches('-').split(',').next().unwrap_or("0").parse().unwrap_or(0);
    let new_start = new.trim_start_matches('+').split(',').next().unwrap_or("0").parse().unwrap_or(0);
    Ok((old_start, new_start))
}

pub fn stage_lines(worktree: &Path, path: &str, hunk_index: usize, selected: &[usize]) -> Result<()> {
    let fd = parse_diff(&diff(worktree, path, false)?);
    let patch = build_partial_patch(&fd, hunk_index, selected, false)?;
    git_stdin(worktree, &["apply", "--cached", "-"], &patch)
}

pub fn unstage_lines(worktree: &Path, path: &str, hunk_index: usize, selected: &[usize]) -> Result<()> {
    let fd = parse_diff(&diff(worktree, path, true)?);
    let patch = build_partial_patch(&fd, hunk_index, selected, true)?;
    git_stdin(worktree, &["apply", "--cached", "--reverse", "-"], &patch)
}

pub fn revert_lines(worktree: &Path, path: &str, hunk_index: usize, selected: &[usize]) -> Result<()> {
    let fd = parse_diff(&diff(worktree, path, false)?);
    let patch = build_partial_patch(&fd, hunk_index, selected, false)?;
    git_stdin(worktree, &["apply", "--reverse", "-"], &patch)
}
```

- [ ] **Step 4: Run tests, verify pass**

Run: `cargo test -p agency-core --test git build_partial_patch stage_lines`
Expected: PASS.

- [ ] **Step 5: Commands** in `commands.rs`:
```rust
#[tauri::command]
pub fn git_stage_lines(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
    lines: Vec<usize>,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stage_lines(&wt, &path, hunk_index, &lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_unstage_lines(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
    lines: Vec<usize>,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::unstage_lines(&wt, &path, hunk_index, &lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_revert_lines(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
    lines: Vec<usize>,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::revert_lines(&wt, &path, hunk_index, &lines).map_err(|e| e.to_string())
}
```

- [ ] **Step 6: Register** in `lib.rs`:
```rust
            commands::git_stage_lines,
            commands::git_unstage_lines,
            commands::git_revert_lines,
```

- [ ] **Step 7: TS wrappers** in `ui/src/api.ts`:
```typescript
export const gitStageLines = (taskId: string, path: string, hunkIndex: number, lines: number[]) =>
  invoke<void>("git_stage_lines", { taskId, path, hunkIndex, lines });
export const gitUnstageLines = (taskId: string, path: string, hunkIndex: number, lines: number[]) =>
  invoke<void>("git_unstage_lines", { taskId, path, hunkIndex, lines });
export const gitRevertLines = (taskId: string, path: string, hunkIndex: number, lines: number[]) =>
  invoke<void>("git_revert_lines", { taskId, path, hunkIndex, lines });
```

- [ ] **Step 8: Build + run full Rust test suite + commit**

Run: `cargo test -p agency-core --test git && cargo build -p agency-app && (cd ui && npx tsc --noEmit)`
Expected: all pass.
```bash
git add -A
git commit -m "Add line/selection-level staging git commands"
```

---

## PHASE 2 — Frontend pure logic (tested modules)

### Task 5: `status.ts` — status decoration + grouping

**Files:**
- Create: `ui/src/components/git/status.ts`, `ui/src/components/git/status.test.ts`

**Interfaces:**
- Consumes: `FileChange` from `../../api`.
- Produces: `type Decoration = { letter: string; varName: string }`; `decorate(index: string, worktree: string): Decoration`; `type GitGroup = "merge" | "index" | "workingTree" | "untracked"`; `groupOf(c: FileChange): GitGroup`; `partition(changes: FileChange[]): Record<GitGroup, FileChange[]>`; `isStaged(c: FileChange): boolean`.

- [ ] **Step 1: Write failing test** `ui/src/components/git/status.test.ts`:
```typescript
import { describe, expect, it } from "vitest";
import { decorate, groupOf, partition } from "./status";
import type { FileChange } from "../../api";

const fc = (index: string, worktree: string, path = "f"): FileChange => ({ path, index, worktree });

describe("decorate", () => {
  it("modified is M / yellow", () => {
    expect(decorate(" ", "M")).toEqual({ letter: "M", varName: "--yellow" });
  });
  it("added is A / green", () => {
    expect(decorate("A", " ")).toEqual({ letter: "A", varName: "--green" });
  });
  it("untracked is U / green", () => {
    expect(decorate("?", "?")).toEqual({ letter: "U", varName: "--green" });
  });
  it("deleted is D / red", () => {
    expect(decorate(" ", "D")).toEqual({ letter: "D", varName: "--red" });
  });
  it("conflict (UU) is ! / peach", () => {
    expect(decorate("U", "U")).toEqual({ letter: "!", varName: "--peach" });
  });
});

describe("groupOf / partition", () => {
  it("routes conflict to merge, staged to index, modified to workingTree, untracked to untracked", () => {
    const groups = partition([
      fc("U", "U", "conflict"),
      fc("M", " ", "staged"),
      fc(" ", "M", "modified"),
      fc("?", "?", "new"),
    ]);
    expect(groups.merge.map((c) => c.path)).toEqual(["conflict"]);
    expect(groups.index.map((c) => c.path)).toEqual(["staged"]);
    expect(groups.workingTree.map((c) => c.path)).toEqual(["modified"]);
    expect(groups.untracked.map((c) => c.path)).toEqual(["new"]);
  });
  it("a file staged AND modified appears in both index and workingTree", () => {
    const groups = partition([fc("M", "M", "both")]);
    expect(groups.index.map((c) => c.path)).toEqual(["both"]);
    expect(groups.workingTree.map((c) => c.path)).toEqual(["both"]);
  });
});
```

- [ ] **Step 2: Run, verify fail**

Run: `cd ui && npx vitest run src/components/git/status.test.ts`
Expected: FAIL — cannot find `./status`.

- [ ] **Step 3: Implement** `ui/src/components/git/status.ts`:
```typescript
import type { FileChange } from "../../api";

export type Decoration = { letter: string; varName: string };
export type GitGroup = "merge" | "index" | "workingTree" | "untracked";

const CONFLICT = new Set(["DD", "AU", "UD", "UA", "DU", "AA", "UU"]);

export function isStaged(c: FileChange): boolean {
  return c.index !== " " && c.index !== "?";
}

function isConflict(c: FileChange): boolean {
  return CONFLICT.has(`${c.index}${c.worktree}`);
}

/** Map a status code pair to a single decoration. `index` wins when staged. */
export function decorate(index: string, worktree: string): Decoration {
  if (isConflict({ path: "", index, worktree })) return { letter: "!", varName: "--peach" };
  if (index === "?" || worktree === "?") return { letter: "U", varName: "--green" };
  const code = index !== " " ? index : worktree;
  switch (code) {
    case "A": return { letter: "A", varName: "--green" };
    case "D": return { letter: "D", varName: "--red" };
    case "R": return { letter: "R", varName: "--teal" };
    case "C": return { letter: "C", varName: "--teal" };
    case "!": return { letter: "I", varName: "--o0" };
    case "M": default: return { letter: "M", varName: "--yellow" };
  }
}

export function groupOf(c: FileChange): GitGroup {
  if (isConflict(c)) return "merge";
  if (c.index === "?" || c.worktree === "?") return "untracked";
  return isStaged(c) ? "index" : "workingTree";
}

export function partition(changes: FileChange[]): Record<GitGroup, FileChange[]> {
  const groups: Record<GitGroup, FileChange[]> = { merge: [], index: [], workingTree: [], untracked: [] };
  for (const c of changes) {
    if (isConflict(c)) { groups.merge.push(c); continue; }
    if (c.index === "?" || c.worktree === "?") { groups.untracked.push(c); continue; }
    if (isStaged(c)) groups.index.push(c);
    if (c.worktree !== " " && c.worktree !== "?") groups.workingTree.push(c);
  }
  return groups;
}
```

- [ ] **Step 4: Run, verify pass**

Run: `cd ui && npx vitest run src/components/git/status.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add ui/src/components/git/status.ts ui/src/components/git/status.test.ts
git commit -m "Add git status decoration + grouping module"
```

---

### Task 6: `diffModel.ts` — aligned rows + word diff (jsdiff)

**Files:**
- Create: `ui/src/components/git/diffModel.ts`, `ui/src/components/git/diffModel.test.ts`
- Modify: `ui/package.json` (add `diff` dependency)

**Interfaces:**
- Consumes: `FileDiff`, `Hunk` from `../../api`.
- Produces: `type Span = { text: string; changed: boolean }`; `type DiffRow = { kind: "ctx" | "add" | "del" | "spacer"; hunkIndex: number; lineIndex: number; oldNo: number | null; newNo: number | null; oldSpans: Span[] | null; newSpans: Span[] | null }`; `buildRows(fd: FileDiff): DiffRow[]` (side-by-side aligned); `wordSpans(oldText: string, newText: string): { old: Span[]; new: Span[] }`.

- [ ] **Step 1: Add dependency**

Run: `cd ui && npm install diff@^5 && npm install -D @types/diff`
Expected: `diff` in `dependencies`, `@types/diff` in `devDependencies`.

- [ ] **Step 2: Write failing test** `ui/src/components/git/diffModel.test.ts`:
```typescript
import { describe, expect, it } from "vitest";
import { buildRows, wordSpans } from "./diffModel";
import type { FileDiff } from "../../api";

const fd = (lines: string[]): FileDiff => ({
  header: "diff --git a/f b/f\n--- a/f\n+++ b/f\n",
  hunks: [{ header: "@@ -1,2 +1,2 @@", lines }],
});

describe("buildRows", () => {
  it("pairs a delete with the following add on one row", () => {
    const rows = buildRows(fd([" ctx", "-old", "+new"]));
    // ctx row, then a paired modify row
    expect(rows[0].kind).toBe("ctx");
    const mod = rows[1];
    expect(mod.oldSpans?.map((s) => s.text).join("")).toBe("old");
    expect(mod.newSpans?.map((s) => s.text).join("")).toBe("new");
    expect(mod.oldNo).toBe(2);
    expect(mod.newNo).toBe(2);
  });
  it("emits spacer on the side without content for pure add", () => {
    const rows = buildRows(fd([" ctx", "+added"]));
    const add = rows[1];
    expect(add.kind).toBe("add");
    expect(add.oldSpans).toBeNull();
    expect(add.newSpans?.map((s) => s.text).join("")).toBe("added");
  });
});

describe("wordSpans", () => {
  it("marks only the changed word", () => {
    const { old, new: nw } = wordSpans("the cat sat", "the dog sat");
    expect(old.filter((s) => s.changed).map((s) => s.text).join("")).toContain("cat");
    expect(nw.filter((s) => s.changed).map((s) => s.text).join("")).toContain("dog");
  });
});
```

- [ ] **Step 3: Run, verify fail**

Run: `cd ui && npx vitest run src/components/git/diffModel.test.ts`
Expected: FAIL — cannot find `./diffModel`.

- [ ] **Step 4: Implement** `ui/src/components/git/diffModel.ts`:
```typescript
import { diffWordsWithSpace } from "diff";
import type { FileDiff } from "../../api";

export type Span = { text: string; changed: boolean };
export type DiffRow = {
  kind: "ctx" | "add" | "del" | "spacer";
  hunkIndex: number;
  lineIndex: number; // index within hunk.lines (body), for staging selection
  oldNo: number | null;
  newNo: number | null;
  oldSpans: Span[] | null;
  newSpans: Span[] | null;
};

export function wordSpans(oldText: string, newText: string): { old: Span[]; new: Span[] } {
  const parts = diffWordsWithSpace(oldText, newText);
  const oldS: Span[] = [];
  const newS: Span[] = [];
  for (const p of parts) {
    if (p.added) newS.push({ text: p.value, changed: true });
    else if (p.removed) oldS.push({ text: p.value, changed: true });
    else { oldS.push({ text: p.value, changed: false }); newS.push({ text: p.value, changed: false }); }
  }
  return { old: oldS, new: newS };
}

const plain = (text: string): Span[] => [{ text, changed: false }];

function parseStarts(header: string): { oldStart: number; newStart: number } {
  const m = header.match(/@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
  return { oldStart: m ? +m[1] : 1, newStart: m ? +m[2] : 1 };
}

/** Build side-by-side aligned rows. Consecutive del/add runs are paired up so
 *  modified lines sit on one row (enabling intra-line word diff). */
export function buildRows(fd: FileDiff): DiffRow[] {
  const rows: DiffRow[] = [];
  fd.hunks.forEach((hunk, hunkIndex) => {
    let { oldStart: oldNo, newStart: newNo } = parseStarts(hunk.header);
    const lines = hunk.lines;
    let i = 0;
    while (i < lines.length) {
      const line = lines[i];
      const kind = line[0] ?? " ";
      if (kind === " ") {
        rows.push({ kind: "ctx", hunkIndex, lineIndex: i, oldNo, newNo, oldSpans: plain(line.slice(1)), newSpans: plain(line.slice(1)) });
        oldNo++; newNo++; i++;
        continue;
      }
      // collect a run of deletes then a run of adds
      const dels: number[] = [];
      const adds: number[] = [];
      while (i < lines.length && lines[i][0] === "-") { dels.push(i); i++; }
      while (i < lines.length && lines[i][0] === "+") { adds.push(i); i++; }
      const pairs = Math.max(dels.length, adds.length);
      for (let p = 0; p < pairs; p++) {
        const d = dels[p];
        const a = adds[p];
        if (d !== undefined && a !== undefined) {
          const { old, new: nw } = wordSpans(lines[d].slice(1), lines[a].slice(1));
          rows.push({ kind: "del", hunkIndex, lineIndex: d, oldNo, newNo: null, oldSpans: old, newSpans: null });
          rows[rows.length - 1].newNo = null;
          rows.push({ kind: "add", hunkIndex, lineIndex: a, oldNo: null, newNo, oldSpans: null, newSpans: nw });
          // pair them visually: represent as a single modify row pair (del row carries new side too)
          rows.splice(rows.length - 2, 2, {
            kind: "del", hunkIndex, lineIndex: d, oldNo, newNo,
            oldSpans: old, newSpans: nw,
          });
          oldNo++; newNo++;
        } else if (d !== undefined) {
          rows.push({ kind: "del", hunkIndex, lineIndex: d, oldNo, newNo: null, oldSpans: plain(lines[d].slice(1)), newSpans: null });
          oldNo++;
        } else if (a !== undefined) {
          rows.push({ kind: "add", hunkIndex, lineIndex: a, oldNo: null, newNo, oldSpans: null, newSpans: plain(lines[a].slice(1)) });
          newNo++;
        }
      }
    }
  });
  return rows;
}
```

Note for implementer: a paired modify renders as one row with both `oldSpans` and `newSpans` set and `kind: "del"` carrying both sides; the `DiffViewer` treats a row with both span sides present as a side-by-side modify. The test asserts exactly this shape (`rows[1].oldSpans` and `rows[1].newSpans` both populated).

- [ ] **Step 5: Run, verify pass**

Run: `cd ui && npx vitest run src/components/git/diffModel.test.ts`
Expected: PASS.

- [ ] **Step 6: Commit**
```bash
git add ui/package.json ui/package-lock.json ui/src/components/git/diffModel.ts ui/src/components/git/diffModel.test.ts
git commit -m "Add diff model (aligned rows + word diff) with jsdiff"
```

---

### Task 7: `graph.ts` — commit-graph swimlanes (port of VSCode)

**Files:**
- Create: `ui/src/components/git/graph.ts`, `ui/src/components/git/graph.test.ts`

**Interfaces:**
- Produces: `type Lane = { target: string; color: string }`; `type GraphRow = { hash: string; input: Lane[]; output: Lane[]; circleIndex: number; color: string }`; `computeGraph(commits: { hash: string; parents: string[] }[]): GraphRow[]`; `PALETTE: string[]` (CSS var names).

Algorithm (single forward pass, newest→oldest): a lane is identified by the commit hash it points toward; row N input = row N−1 output (cloned). For the current commit: the first input lane targeting it is replaced by a lane targeting `parents[0]` (keeping its color); further lanes targeting it are dropped; unrelated lanes pass through preserving order. Each extra parent (`parents[1..]`) appends a new lane (palette color). If no input lane targeted the commit (a tip), `parents[0]` is appended. `circleIndex` = index of the first input lane targeting the commit, else `input.length`. Palette index advances via `(i+1) % 5` shared across the whole graph.

- [ ] **Step 1: Write failing test** `ui/src/components/git/graph.test.ts`:
```typescript
import { describe, expect, it } from "vitest";
import { computeGraph } from "./graph";

describe("computeGraph", () => {
  it("linear history uses one lane", () => {
    const rows = computeGraph([
      { hash: "c", parents: ["b"] },
      { hash: "b", parents: ["a"] },
      { hash: "a", parents: [] },
    ]);
    expect(rows[0].circleIndex).toBe(0);
    expect(rows[0].output).toEqual([{ target: "b", color: rows[0].color }]);
    expect(rows[1].output).toEqual([{ target: "a", color: rows[1].color }]);
    expect(rows[2].output).toEqual([]); // root: no parent lane
  });

  it("a merge commit adds a second lane for its second parent", () => {
    const rows = computeGraph([
      { hash: "m", parents: ["a", "b"] },
      { hash: "b", parents: ["root"] },
      { hash: "a", parents: ["root"] },
    ]);
    expect(rows[0].output.map((l) => l.target)).toEqual(["a", "b"]);
    // by the time we reach 'a', its lane is consumed and continues to root
    expect(rows[2].circleIndex).toBe(0);
  });

  it("lanes converging on the same commit collapse to one", () => {
    const rows = computeGraph([
      { hash: "a", parents: ["root"] },
      { hash: "root", parents: [] },
    ]);
    // 'a' is a tip -> appends lane to root; root consumes it and has no parents
    expect(rows[0].output).toEqual([{ target: "root", color: rows[0].color }]);
    expect(rows[1].output).toEqual([]);
  });
});
```

- [ ] **Step 2: Run, verify fail**

Run: `cd ui && npx vitest run src/components/git/graph.test.ts`
Expected: FAIL — cannot find `./graph`.

- [ ] **Step 3: Implement** `ui/src/components/git/graph.ts`:
```typescript
export type Lane = { target: string; color: string };
export type GraphRow = {
  hash: string;
  input: Lane[];
  output: Lane[];
  circleIndex: number;
  color: string;
};

export const PALETTE = ["--peach", "--pink", "--yellow", "--teal", "--mauve"];

export function computeGraph(commits: { hash: string; parents: string[] }[]): GraphRow[] {
  const rows: GraphRow[] = [];
  let colorIndex = 0;
  const nextColor = () => {
    const c = PALETTE[colorIndex];
    colorIndex = (colorIndex + 1) % PALETTE.length;
    return c;
  };

  let input: Lane[] = [];
  for (const commit of commits) {
    const output: Lane[] = [];
    let circleIndex = -1;
    let firstParentPlaced = false;

    input.forEach((lane, i) => {
      if (lane.target === commit.hash) {
        if (circleIndex === -1) {
          circleIndex = i;
          // first lane targeting commit continues as first parent
          if (commit.parents[0]) {
            output.push({ target: commit.parents[0], color: lane.color });
            firstParentPlaced = true;
          }
        }
        // additional lanes targeting commit collapse (dropped)
      } else {
        output.push({ ...lane });
      }
    });

    const commitColor = circleIndex === -1 ? nextColor() : input[circleIndex].color;
    if (circleIndex === -1) circleIndex = input.length;

    // tip (no incoming lane): place first parent now
    if (!firstParentPlaced && commit.parents[0]) {
      output.push({ target: commit.parents[0], color: commitColor });
    }
    // extra parents (merge) each add a new lane
    for (let p = 1; p < commit.parents.length; p++) {
      output.push({ target: commit.parents[p], color: nextColor() });
    }

    rows.push({ hash: commit.hash, input, output, circleIndex, color: commitColor });
    input = output.map((l) => ({ ...l }));
  }
  return rows;
}
```

- [ ] **Step 4: Run, verify pass**

Run: `cd ui && npx vitest run src/components/git/graph.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add ui/src/components/git/graph.ts ui/src/components/git/graph.test.ts
git commit -m "Add commit-graph swimlane algorithm (VSCode port)"
```

---

### Task 8: `highlight.ts` — Shiki singleton

**Files:**
- Create: `ui/src/components/git/highlight.ts`
- Modify: `ui/package.json` (add `shiki`)

**Interfaces:**
- Produces: `langForPath(path: string): string`; `highlightLine(code: string, lang: string): Promise<string>` returns HTML string of one line's tokens (Catppuccin Mocha); `ensureHighlighter(): Promise<void>`. If highlighting fails or language unknown, callers fall back to plain text.

- [ ] **Step 1: Add dependency**

Run: `cd ui && npm install shiki@^1`
Expected: `shiki` in `dependencies`.

- [ ] **Step 2: Implement** `ui/src/components/git/highlight.ts`:
```typescript
import { createHighlighter, type Highlighter } from "shiki";

const LANGS = ["typescript", "tsx", "javascript", "jsx", "json", "rust", "css", "html", "markdown", "python", "bash", "toml", "yaml"];
const EXT: Record<string, string> = {
  ts: "typescript", tsx: "tsx", js: "javascript", jsx: "jsx", json: "json",
  rs: "rust", css: "css", html: "html", md: "markdown", py: "python",
  sh: "bash", toml: "toml", yml: "yaml", yaml: "yaml",
};

export function langForPath(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return EXT[ext] ?? "text";
}

let hl: Highlighter | null = null;
let loading: Promise<void> | null = null;

export function ensureHighlighter(): Promise<void> {
  if (hl) return Promise.resolve();
  if (!loading) {
    loading = createHighlighter({ themes: ["catppuccin-mocha"], langs: LANGS }).then((h) => {
      hl = h;
    });
  }
  return loading;
}

/** Returns inner HTML (a sequence of <span style>) for one line, or the
 *  HTML-escaped plain text if highlighting is unavailable. */
export async function highlightLine(code: string, lang: string): Promise<string> {
  try {
    await ensureHighlighter();
    if (!hl || lang === "text" || !LANGS.includes(lang)) return escapeHtml(code);
    const html = hl.codeToHtml(code, { lang, theme: "catppuccin-mocha" });
    // Extract inner spans of the single <span class="line">…</span>.
    const m = html.match(/<span class="line">(.*)<\/span>/s);
    return m ? m[1] : escapeHtml(code);
  } catch {
    return escapeHtml(code);
  }
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
```

- [ ] **Step 3: Typecheck + commit**

Run: `cd ui && npx tsc --noEmit`
Expected: succeeds.
```bash
git add ui/package.json ui/package-lock.json ui/src/components/git/highlight.ts
git commit -m "Add Shiki highlighter singleton (Catppuccin Mocha)"
```

---

## PHASE 3 — Components

### Task 9: `FileRow` + `ResourceGroup`

**Files:**
- Create: `ui/src/components/git/FileRow.tsx`, `ui/src/components/git/ResourceGroup.tsx`
- Modify: `ui/src/styles.css` (add `git-row`, `git-group` rules)

**Interfaces:**
- Consumes: `FileChange` (api), `decorate` (status.ts).
- Produces (FileRow): `FileRow({ change, group, selected, onSelect, onPrimary, onDiscard })` where `group: GitGroup`, `onPrimary` stages (workingTree/untracked/merge) or unstages (index), `onDiscard?` shows discard for workingTree/untracked.
- Produces (ResourceGroup): `ResourceGroup({ id, label, changes, selectedPath, onSelectFile, onFilePrimary, onFileDiscard, onStageAll, onUnstageAll, onDiscardAll })`.

- [ ] **Step 1: Implement `FileRow.tsx`**
```tsx
import { FileChange } from "../../api";
import { decorate, type GitGroup } from "./status";

export default function FileRow({
  change, group, selected, onSelect, onPrimary, onDiscard,
}: {
  change: FileChange;
  group: GitGroup;
  selected: boolean;
  onSelect: () => void;
  onPrimary: () => void;
  onDiscard?: () => void;
}) {
  const code = group === "index" ? change.index : (change.worktree === "?" || change.index === "?" ? "?" : change.worktree);
  const dec = decorate(group === "index" ? change.index : " ", group === "index" ? " " : change.worktree);
  const slash = change.path.lastIndexOf("/");
  const dir = slash >= 0 ? change.path.slice(0, slash + 1) : "";
  const name = slash >= 0 ? change.path.slice(slash + 1) : change.path;
  const primaryLabel = group === "index" ? "Unstage" : "Stage";
  const primaryGlyph = group === "index" ? "−" : "+";
  return (
    <div className={`git-row ${selected ? "sel" : ""}`} onClick={onSelect} title={change.path}>
      <span className="git-letter" style={{ color: `var(${dec.varName})` }}>{dec.letter}</span>
      <span className="git-name"><span className="git-dir">{dir}</span>{name}</span>
      <span className="git-row-actions">
        {onDiscard && (
          <button className="git-iconbtn" title="Discard Changes"
            onClick={(e) => { e.stopPropagation(); onDiscard(); }}>↩</button>
        )}
        <button className="git-iconbtn" title={primaryLabel}
          onClick={(e) => { e.stopPropagation(); onPrimary(); }}>{primaryGlyph}</button>
      </span>
    </div>
  );
}
```

- [ ] **Step 2: Implement `ResourceGroup.tsx`**
```tsx
import { useState } from "react";
import { FileChange } from "../../api";
import FileRow from "./FileRow";
import type { GitGroup } from "./status";

export default function ResourceGroup({
  id, label, changes, selectedPath, onSelectFile, onFilePrimary, onFileDiscard,
  onStageAll, onUnstageAll, onDiscardAll,
}: {
  id: GitGroup;
  label: string;
  changes: FileChange[];
  selectedPath: string | null;
  onSelectFile: (c: FileChange) => void;
  onFilePrimary: (c: FileChange) => void;
  onFileDiscard?: (c: FileChange) => void;
  onStageAll?: () => void;
  onUnstageAll?: () => void;
  onDiscardAll?: () => void;
}) {
  const [open, setOpen] = useState(true);
  if (changes.length === 0) return null;
  return (
    <div className="git-group">
      <div className="git-group-head">
        <button className="git-twisty" onClick={() => setOpen((o) => !o)}>{open ? "▾" : "▸"}</button>
        <span className="git-group-label">{label}</span>
        <span className="git-count">{changes.length}</span>
        <span className="git-group-actions">
          {onDiscardAll && <button className="git-iconbtn" title="Discard All" onClick={onDiscardAll}>↩</button>}
          {onUnstageAll && <button className="git-iconbtn" title="Unstage All" onClick={onUnstageAll}>−</button>}
          {onStageAll && <button className="git-iconbtn" title="Stage All" onClick={onStageAll}>+</button>}
        </span>
      </div>
      {open && changes.map((c) => (
        <FileRow key={`${id}:${c.path}`} change={c} group={id}
          selected={selectedPath === c.path}
          onSelect={() => onSelectFile(c)}
          onPrimary={() => onFilePrimary(c)}
          onDiscard={onFileDiscard ? () => onFileDiscard(c) : undefined} />
      ))}
    </div>
  );
}
```

- [ ] **Step 3: Add CSS** to `ui/src/styles.css`:
```css
.git-group { display: flex; flex-direction: column; }
.git-group-head { display: flex; align-items: center; gap: 6px; padding: 4px 6px; }
.git-group-label { font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--o1); font-weight: 600; }
.git-count { background: var(--s0); color: var(--sub0); border-radius: 9px; font-size: 10px; padding: 0 6px; }
.git-group-actions { margin-left: auto; display: flex; gap: 2px; opacity: 0; }
.git-group-head:hover .git-group-actions { opacity: 1; }
.git-twisty { background: none; border: none; color: var(--o0); cursor: pointer; padding: 0; font-size: 11px; }
.git-row { display: flex; align-items: center; gap: 8px; padding: 3px 6px 3px 20px; border-radius: 4px; cursor: pointer; font-size: 13px; }
.git-row:hover { background: var(--s0); }
.git-row.sel { background: var(--s1); }
.git-letter { width: 12px; text-align: center; font-family: var(--mono); font-weight: 600; flex-shrink: 0; }
.git-name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text); }
.git-dir { color: var(--o0); }
.git-row-actions { display: flex; gap: 2px; opacity: 0; }
.git-row:hover .git-row-actions { opacity: 1; }
.git-iconbtn { background: none; border: none; color: var(--sub0); cursor: pointer; font-size: 13px; line-height: 1; padding: 2px 4px; border-radius: 4px; }
.git-iconbtn:hover { background: var(--s2); color: var(--text); }
```

- [ ] **Step 4: Typecheck + commit**

Run: `cd ui && npx tsc --noEmit`
Expected: succeeds.
```bash
git add ui/src/components/git/FileRow.tsx ui/src/components/git/ResourceGroup.tsx ui/src/styles.css
git commit -m "Add FileRow and ResourceGroup components"
```

---

### Task 10: `CommitBox` + `BranchBar`

**Files:**
- Create: `ui/src/components/git/CommitBox.tsx`, `ui/src/components/git/BranchBar.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- Consumes: `BranchInfo` (api).
- Produces (CommitBox): `CommitBox({ branch, hasUpstream, ahead, behind, onCommit, onCommitPush, onAmend, onSync, onPublish })` — `onCommit(message)`, `onCommitPush(message)`, `onAmend(message)` receive the textarea content; primary button label adapts (Commit / Publish Branch / Sync Changes).
- Produces (BranchBar): `BranchBar({ info, onSync, onRefresh })`.

- [ ] **Step 1: Implement `CommitBox.tsx`**
```tsx
import { useState } from "react";

export default function CommitBox({
  branch, hasUpstream, ahead, behind, onCommit, onCommitPush, onAmend, onSync, onPublish,
}: {
  branch: string;
  hasUpstream: boolean;
  ahead: number;
  behind: number;
  onCommit: (m: string) => void;
  onCommitPush: (m: string) => void;
  onAmend: (m: string) => void;
  onSync: () => void;
  onPublish: () => void;
}) {
  const [message, setMessage] = useState("");
  const [menu, setMenu] = useState(false);
  const send = (fn: (m: string) => void) => { fn(message); setMessage(""); setMenu(false); };
  return (
    <div className="git-commit">
      <textarea className="git-commit-input" placeholder={`Message (commit on ${branch})`}
        value={message} onChange={(e) => setMessage(e.target.value)} />
      <div className="git-commit-bar">
        <div className="git-split">
          <button className="git-primary" onClick={() => send(onCommit)}>✓ Commit</button>
          <button className="git-primary git-caret" onClick={() => setMenu((o) => !o)}>▾</button>
          {menu && (
            <div className="git-menu" onMouseLeave={() => setMenu(false)}>
              <button onClick={() => send(onCommitPush)}>Commit &amp; Push</button>
              <button onClick={() => send(onAmend)}>Commit (Amend)</button>
            </div>
          )}
        </div>
        {!hasUpstream
          ? <button className="git-secondary" onClick={onPublish}>☁ Publish Branch</button>
          : (ahead > 0 || behind > 0) &&
            <button className="git-secondary" onClick={onSync}>⟳ Sync {behind ? `↓${behind}` : ""} {ahead ? `↑${ahead}` : ""}</button>}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Implement `BranchBar.tsx`**
```tsx
import { BranchInfo } from "../../api";

export default function BranchBar({ info, onSync, onRefresh }: {
  info: BranchInfo | null;
  onSync: () => void;
  onRefresh: () => void;
}) {
  if (!info) return null;
  return (
    <div className="git-branchbar">
      <span className="git-branch">⎇ {info.branch}</span>
      {(info.ahead > 0 || info.behind > 0) && (
        <span className="git-aheadbehind">
          {info.behind > 0 && <span title="behind">↓{info.behind}</span>}
          {info.ahead > 0 && <span title="ahead">↑{info.ahead}</span>}
        </span>
      )}
      <span className="spacer" style={{ flex: 1 }} />
      {info.upstream && <button className="git-iconbtn" title="Sync" onClick={onSync}>⟳</button>}
      <button className="git-iconbtn" title="Refresh" onClick={onRefresh}>⟲</button>
    </div>
  );
}
```

- [ ] **Step 3: Add CSS** to `ui/src/styles.css`:
```css
.git-commit { display: flex; flex-direction: column; gap: 6px; padding: 8px; }
.git-commit-input { background: var(--crust); border: 1px solid var(--s1); color: var(--text); border-radius: 6px; padding: 7px; min-height: 54px; resize: vertical; font-family: var(--sans); font-size: 13px; }
.git-commit-input:focus { outline: none; border-color: var(--blue); }
.git-commit-bar { display: flex; gap: 6px; align-items: center; }
.git-split { position: relative; display: flex; }
.git-primary { background: var(--blue); color: var(--crust); border: none; font-weight: 600; padding: 6px 12px; cursor: pointer; font-size: 12.5px; }
.git-primary:hover { background: var(--lav); }
.git-split .git-primary:first-child { border-radius: 7px 0 0 7px; }
.git-caret { border-radius: 0 7px 7px 0; padding: 6px 8px; border-left: 1px solid rgba(0,0,0,.25); }
.git-secondary { background: var(--s0); color: var(--text); border: none; border-radius: 7px; padding: 6px 10px; cursor: pointer; font-size: 12.5px; }
.git-secondary:hover { background: var(--s1); }
.git-menu { position: absolute; top: calc(100% + 4px); left: 0; z-index: 25; background: var(--mantle); border: 1px solid var(--s1); border-radius: 8px; box-shadow: 0 12px 32px rgba(0,0,0,.4); padding: 4px; min-width: 160px; display: flex; flex-direction: column; }
.git-menu button { background: none; border: none; color: var(--text); text-align: left; padding: 7px 9px; border-radius: 5px; cursor: pointer; font-size: 12.5px; }
.git-menu button:hover { background: var(--s0); }
.git-branchbar { display: flex; align-items: center; gap: 8px; padding: 6px 10px; border-bottom: 1px solid var(--line); background: var(--mantle); font-size: 12.5px; }
.git-branch { color: var(--blue); font-weight: 600; }
.git-aheadbehind { display: flex; gap: 6px; color: var(--sub0); font-family: var(--mono); font-size: 12px; }
```

- [ ] **Step 4: Typecheck + commit**

Run: `cd ui && npx tsc --noEmit`
```bash
git add ui/src/components/git/CommitBox.tsx ui/src/components/git/BranchBar.tsx ui/src/styles.css
git commit -m "Add CommitBox and BranchBar components"
```

---

### Task 11: `DiffViewer`

**Files:**
- Create: `ui/src/components/git/DiffViewer.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- Consumes: `gitParseDiff`, `gitCommitDiff`, `gitStageLines`, `gitUnstageLines`, `gitRevertLines`, `gitStageHunk`, `gitUnstageHunk` (api); `buildRows`, `DiffRow`, `Span` (diffModel); `highlightLine`, `langForPath` (highlight); `FileDiff` (api).
- Produces: `DiffViewer({ taskId, path, mode, hash, onChanged })` where `mode: "working-unstaged" | "working-staged" | "commit"`; for `"commit"`, `hash` is required and the view is read-only (no staging gutter). Renders side-by-side, collapsing to inline below container width 900px (via `ResizeObserver`), with a toolbar toggle.

- [ ] **Step 1: Implement `DiffViewer.tsx`**
```tsx
import { useCallback, useEffect, useRef, useState } from "react";
import {
  FileDiff, gitParseDiff, gitCommitDiff, gitStageHunk, gitUnstageHunk,
  gitStageLines, gitUnstageLines, gitRevertLines,
} from "../../api";
import { buildRows, type DiffRow, type Span } from "./diffModel";
import { highlightLine, langForPath } from "./highlight";

type Mode = "working-unstaged" | "working-staged" | "commit";

function spansToText(spans: Span[] | null): string {
  return spans ? spans.map((s) => s.text).join("") : "";
}

export default function DiffViewer({
  taskId, path, mode, hash, onChanged,
}: {
  taskId: string;
  path: string;
  mode: Mode;
  hash?: string;
  onChanged: () => void;
}) {
  const [fd, setFd] = useState<FileDiff | null>(null);
  const [rows, setRows] = useState<DiffRow[]>([]);
  const [highlighted, setHighlighted] = useState<Record<string, string>>({});
  const [error, setError] = useState("");
  const [sideBySide, setSideBySide] = useState(true);
  const [sel, setSel] = useState<{ hunk: number; lines: Set<number> } | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const staged = mode === "working-staged";
  const readonly = mode === "commit";

  const load = useCallback(async () => {
    try {
      const parsed = readonly && hash
        ? parseInto(await gitCommitDiff(taskId, hash, path))
        : await gitParseDiff(taskId, path, staged);
      setFd(parsed);
      setRows(buildRows(parsed));
      setError("");
    } catch (e) { setError(String(e)); }
  }, [taskId, path, staged, readonly, hash]);

  useEffect(() => { load(); }, [load]);

  // collapse to inline when narrow
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => setSideBySide(entry.contentRect.width >= 900));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // syntax highlight (async, best-effort)
  useEffect(() => {
    let cancelled = false;
    const lang = langForPath(path);
    (async () => {
      const out: Record<string, string> = {};
      for (const r of rows) {
        for (const [side, spans] of [["o", r.oldSpans], ["n", r.newSpans]] as const) {
          const text = spansToText(spans);
          if (text && out[`${side}:${r.lineIndex}`] === undefined) {
            out[`${side}:${r.lineIndex}`] = await highlightLine(text, lang);
          }
        }
      }
      if (!cancelled) setHighlighted(out);
    })();
    return () => { cancelled = true; };
  }, [rows, path]);

  async function hunkAction(hunkIndex: number) {
    try {
      if (staged) await gitUnstageHunk(taskId, path, hunkIndex);
      else await gitStageHunk(taskId, path, hunkIndex);
      await load(); onChanged();
    } catch (e) { setError(String(e)); }
  }

  async function applySelection(action: "stage" | "unstage" | "revert") {
    if (!sel || sel.lines.size === 0) return;
    const lines = [...sel.lines];
    try {
      if (action === "stage") await gitStageLines(taskId, path, sel.hunk, lines);
      else if (action === "unstage") await gitUnstageLines(taskId, path, sel.hunk, lines);
      else await gitRevertLines(taskId, path, sel.hunk, lines);
      setSel(null); await load(); onChanged();
    } catch (e) { setError(String(e)); }
  }

  function toggleLine(hunk: number, lineIndex: number) {
    if (readonly) return;
    setSel((prev) => {
      const lines = new Set(prev && prev.hunk === hunk ? prev.lines : []);
      lines.has(lineIndex) ? lines.delete(lineIndex) : lines.add(lineIndex);
      return { hunk, lines };
    });
  }

  if (error) return <div className="git-error">{error}</div>;
  if (!fd) return <div className="diff-empty">loading…</div>;
  if (rows.length === 0) return <div className="diff-empty">no textual changes</div>;

  return (
    <div className="diffviewer" ref={wrapRef}>
      <div className="diff-toolbar">
        <span className="diff-path">{path}</span>
        <span className="spacer" style={{ flex: 1 }} />
        {sel && sel.lines.size > 0 && !readonly && (
          <span className="diff-sel-actions">
            {!staged && <button className="git-iconbtn" onClick={() => applySelection("stage")}>Stage selection</button>}
            {staged && <button className="git-iconbtn" onClick={() => applySelection("unstage")}>Unstage selection</button>}
            {!staged && <button className="git-iconbtn" onClick={() => applySelection("revert")}>Revert selection</button>}
          </span>
        )}
        <button className="git-iconbtn" onClick={() => setSideBySide((s) => !s)}>
          {sideBySide ? "Inline" : "Side by side"}
        </button>
      </div>
      <div className={`diff-body ${sideBySide ? "sxs" : "inline"}`}>
        {rows.map((r, idx) => (
          <DiffLineRow key={idx} row={r} sideBySide={sideBySide} highlighted={highlighted}
            selected={!!sel && sel.hunk === r.hunkIndex && sel.lines.has(r.lineIndex)}
            onToggle={() => toggleLine(r.hunkIndex, r.lineIndex)} />
        ))}
      </div>
      {!readonly && (
        <div className="diff-hunks">
          {fd.hunks.map((h, i) => (
            <button key={i} className="git-iconbtn" onClick={() => hunkAction(i)}>
              {staged ? "Unstage" : "Stage"} hunk {i + 1}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function DiffLineRow({ row, sideBySide, highlighted, selected, onToggle }: {
  row: DiffRow; sideBySide: boolean; highlighted: Record<string, string>;
  selected: boolean; onToggle: () => void;
}) {
  const oldHtml = highlighted[`o:${row.lineIndex}`];
  const newHtml = highlighted[`n:${row.lineIndex}`];
  const cell = (spans: Span[] | null, html: string | undefined, side: "old" | "new") => {
    if (!spans) return <div className={`diff-cell empty`} />;
    return (
      <div className={`diff-cell ${side}`}>
        {html !== undefined
          ? <span dangerouslySetInnerHTML={{ __html: wrapChanged(html, spans) }} />
          : <span>{spans.map((s, i) => <span key={i} className={s.changed ? "wd" : ""}>{s.text}</span>)}</span>}
      </div>
    );
  };
  return (
    <div className={`diff-line ${row.kind} ${selected ? "sel" : ""}`} onClick={onToggle}>
      <span className="diff-gutter">{row.oldNo ?? ""}</span>
      <span className="diff-gutter">{row.newNo ?? ""}</span>
      {sideBySide
        ? <>{cell(row.oldSpans, oldHtml, "old")}{cell(row.newSpans, newHtml, "new")}</>
        : cell(row.newSpans ?? row.oldSpans, newHtml ?? oldHtml, row.newSpans ? "new" : "old")}
    </div>
  );
}

// When highlighted HTML is present we cannot easily inject word-diff spans;
// fall back to a line-level wash (handled by .diff-line.add/.del CSS). The word
// overlay only applies in the non-highlighted span path above (class "wd").
function wrapChanged(html: string, _spans: Span[]): string { return html; }

function parseInto(raw: string): FileDiff {
  // Local mirror of git.parse_diff for commit_diff output.
  const lines = raw.split("\n");
  const header: string[] = [];
  const hunks: FileDiff["hunks"] = [];
  let current: FileDiff["hunks"][number] | null = null;
  let seen = false;
  for (const line of lines) {
    if (line.startsWith("@@")) {
      seen = true;
      if (current) hunks.push(current);
      current = { header: line, lines: [] };
    } else if (current) current.lines.push(line);
    else if (!seen) header.push(line);
  }
  if (current) hunks.push(current);
  return { header: header.length ? header.join("\n") + "\n" : "", hunks };
}
```

- [ ] **Step 2: Add CSS** to `ui/src/styles.css`:
```css
.diffviewer { display: flex; flex-direction: column; min-height: 0; flex: 1; }
.diff-toolbar { display: flex; align-items: center; gap: 8px; padding: 6px 10px; border-bottom: 1px solid var(--line); }
.diff-path { font-family: var(--mono); font-size: 12px; color: var(--sub1); }
.diff-body { overflow: auto; flex: 1; font-family: var(--mono); font-size: 12.5px; line-height: 1.55; }
.diff-line { display: grid; grid-template-columns: 44px 44px 1fr 1fr; align-items: stretch; cursor: pointer; }
.diff-body.inline .diff-line { grid-template-columns: 44px 44px 1fr; }
.diff-line:hover { background: var(--s0); }
.diff-line.sel { outline: 1px solid var(--blue); outline-offset: -1px; }
.diff-gutter { color: var(--o0); text-align: right; padding: 0 6px; user-select: none; background: var(--mantle); }
.diff-cell { padding: 0 8px; white-space: pre-wrap; word-break: break-word; }
.diff-cell.empty { background: rgba(108,112,134,.06); }
.diff-line.add .diff-cell.new { background: color-mix(in srgb, var(--green) 12%, transparent); }
.diff-line.del .diff-cell.old { background: color-mix(in srgb, var(--red) 12%, transparent); }
.diff-line.del .diff-cell.new { background: color-mix(in srgb, var(--green) 12%, transparent); }
.diff-cell .wd { border-radius: 2px; }
.diff-line.del .diff-cell.old .wd { background: color-mix(in srgb, var(--red) 30%, transparent); }
.diff-line.add .diff-cell.new .wd, .diff-line.del .diff-cell.new .wd { background: color-mix(in srgb, var(--green) 30%, transparent); }
.diff-hunks { display: flex; flex-wrap: wrap; gap: 6px; padding: 8px 10px; border-top: 1px solid var(--line); }
.diff-sel-actions { display: flex; gap: 4px; }
```

- [ ] **Step 3: Typecheck + commit**

Run: `cd ui && npx tsc --noEmit`
Expected: succeeds.
```bash
git add ui/src/components/git/DiffViewer.tsx ui/src/styles.css
git commit -m "Add DiffViewer (side-by-side, highlight, line staging)"
```

---

### Task 12: `Graph` + `CommitRow` + `HistoryPanel` + `CommitDetail`

**Files:**
- Create: `ui/src/components/git/Graph.tsx`, `ui/src/components/git/CommitRow.tsx`, `ui/src/components/git/HistoryPanel.tsx`, `ui/src/components/git/CommitDetail.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- Consumes: `gitLogGraph`, `gitCommitFiles`, `HistoryItem`, `BranchInfo`, `CommitFile` (api); `computeGraph`, `GraphRow`, `PALETTE` (graph.ts); `DiffViewer`; `decorate` (status.ts).
- Produces (Graph): `Graph({ row })` — renders one row's swimlane SVG (11×22 geometry).
- Produces (CommitRow): `CommitRow({ item, graphRow, aheadOfBase, selected, onSelect })`.
- Produces (HistoryPanel): `HistoryPanel({ taskId, base, onSelectCommit, selectedHash })`.
- Produces (CommitDetail): `CommitDetail({ taskId, item })` — header + file list → `DiffViewer` mode "commit".

- [ ] **Step 1: Implement `Graph.tsx`**
```tsx
import type { GraphRow } from "./graph";

const W = 11, H = 22, R = 4;
const x = (i: number) => W * (i + 1);

export default function Graph({ row }: { row: GraphRow }) {
  const lanes = Math.max(row.input.length, row.output.length, 1);
  const width = W * (lanes + 1);
  const cx = x(row.circleIndex);
  return (
    <svg className="git-graph" width={width} height={H} viewBox={`0 0 ${width} ${H}`}>
      {row.output.map((lane, i) => (
        <line key={`o${i}`} x1={x(i)} y1={H / 2} x2={x(i)} y2={H}
          stroke={`var(${lane.color})`} strokeWidth={2} />
      ))}
      {row.input.map((lane, i) => (
        <line key={`i${i}`} x1={x(i)} y1={0} x2={x(i)} y2={H / 2}
          stroke={`var(${lane.color})`} strokeWidth={2} />
      ))}
      <circle cx={cx} cy={H / 2} r={R} fill={`var(${row.color})`} />
    </svg>
  );
}
```
Note: this is the simplified straight-lane renderer (vertical rails + node). The S-curve connectors described in the spec are a visual refinement; straight rails are correct and legible for the common linear/branch cases and keep the first pass shippable. Curve refinement can be a follow-up.

- [ ] **Step 2: Implement `CommitRow.tsx`**
```tsx
import { HistoryItem } from "../../api";
import type { GraphRow } from "./graph";
import Graph from "./Graph";

function rel(ts: number): string {
  const s = Math.floor(Date.now() / 1000 - ts);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

export default function CommitRow({ item, graphRow, aheadOfBase, selected, onSelect }: {
  item: HistoryItem;
  graphRow: GraphRow;
  aheadOfBase: boolean;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <div className={`git-commitrow ${selected ? "sel" : ""} ${aheadOfBase ? "ahead" : "base"}`} onClick={onSelect}>
      <Graph row={graphRow} />
      <div className="git-commit-main">
        <div className="git-commit-subject">{item.subject}</div>
        <div className="git-commit-meta">
          {item.refs.map((r) => <span key={r} className="git-ref">{r}</span>)}
          <span className="git-commit-author">{item.author}</span>
          <span className="git-commit-hash">{item.hash.slice(0, 7)}</span>
          <span className="git-commit-date">{rel(item.date)}</span>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Implement `HistoryPanel.tsx`**
```tsx
import { useCallback, useEffect, useState } from "react";
import { HistoryItem, gitLogGraph } from "../../api";
import { computeGraph } from "./graph";
import CommitRow from "./CommitRow";

export default function HistoryPanel({ taskId, base, onSelectCommit, selectedHash }: {
  taskId: string;
  base: string | null;
  onSelectCommit: (item: HistoryItem) => void;
  selectedHash: string | null;
}) {
  const [items, setItems] = useState<HistoryItem[]>([]);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    try { setItems(await gitLogGraph(taskId, 80)); setError(""); }
    catch (e) { setError(String(e)); }
  }, [taskId]);
  useEffect(() => { load(); }, [load]);

  const graph = computeGraph(items.map((i) => ({ hash: i.hash, parents: i.parents })));
  // commits before the base hash (newest-first) are "ahead of base"
  const baseIdx = base ? items.findIndex((i) => i.hash.startsWith(base) || base.startsWith(i.hash)) : -1;

  if (error) return <div className="git-error">{error}</div>;
  return (
    <div className="git-history">
      {items.map((item, i) => (
        <CommitRow key={item.hash} item={item} graphRow={graph[i]}
          aheadOfBase={baseIdx < 0 ? false : i < baseIdx}
          selected={selectedHash === item.hash}
          onSelect={() => onSelectCommit(item)} />
      ))}
    </div>
  );
}
```

- [ ] **Step 4: Implement `CommitDetail.tsx`**
```tsx
import { useCallback, useEffect, useState } from "react";
import { CommitFile, HistoryItem, gitCommitFiles } from "../../api";
import { decorate } from "./status";
import DiffViewer from "./DiffViewer";

export default function CommitDetail({ taskId, item }: { taskId: string; item: HistoryItem }) {
  const [files, setFiles] = useState<CommitFile[]>([]);
  const [path, setPath] = useState<string | null>(null);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    try { const f = await gitCommitFiles(taskId, item.hash); setFiles(f); setPath(f[0]?.path ?? null); setError(""); }
    catch (e) { setError(String(e)); }
  }, [taskId, item.hash]);
  useEffect(() => { load(); }, [load]);

  return (
    <div className="git-commitdetail">
      <div className="git-commitdetail-head">
        <div className="git-commitdetail-subject">{item.subject}</div>
        <div className="git-commit-meta">
          <span>{item.author}</span><span className="git-commit-hash">{item.hash.slice(0, 9)}</span>
          <span>{new Date(item.date * 1000).toLocaleString()}</span>
        </div>
      </div>
      {error && <div className="git-error">{error}</div>}
      <div className="git-commitdetail-body">
        <div className="git-commitdetail-files">
          {files.map((f) => {
            const dec = decorate(f.status, " ");
            return (
              <div key={f.path} className={`git-row ${path === f.path ? "sel" : ""}`} onClick={() => setPath(f.path)} title={f.path}>
                <span className="git-letter" style={{ color: `var(${dec.varName})` }}>{dec.letter}</span>
                <span className="git-name">{f.path}</span>
              </div>
            );
          })}
        </div>
        <div className="git-commitdetail-diff">
          {path ? <DiffViewer taskId={taskId} path={path} mode="commit" hash={item.hash} onChanged={() => {}} />
                : <div className="diff-empty">Select a file.</div>}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 5: Add CSS** to `ui/src/styles.css`:
```css
.git-history { overflow: auto; flex: 1; }
.git-commitrow { display: flex; align-items: center; gap: 8px; padding: 4px 8px; cursor: pointer; }
.git-commitrow:hover { background: var(--s0); }
.git-commitrow.sel { background: var(--s1); }
.git-commitrow.base { opacity: 0.7; }
.git-graph { flex-shrink: 0; }
.git-commit-main { min-width: 0; flex: 1; }
.git-commit-subject { color: var(--text); font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.git-commit-meta { display: flex; gap: 8px; align-items: center; font-size: 11px; color: var(--o0); }
.git-commit-hash { font-family: var(--mono); }
.git-ref { background: color-mix(in srgb, var(--blue) 20%, transparent); color: var(--blue); border-radius: 9px; padding: 0 7px; font-size: 10px; }
.git-commitdetail { display: flex; flex-direction: column; min-height: 0; flex: 1; }
.git-commitdetail-head { padding: 10px; border-bottom: 1px solid var(--line); }
.git-commitdetail-subject { font-size: 14px; color: var(--text); margin-bottom: 4px; }
.git-commitdetail-body { display: flex; min-height: 0; flex: 1; }
.git-commitdetail-files { width: 240px; border-right: 1px solid var(--line); overflow: auto; padding: 6px 0; }
.git-commitdetail-diff { flex: 1; display: flex; min-width: 0; }
```

- [ ] **Step 6: Typecheck + commit**

Run: `cd ui && npx tsc --noEmit`
Expected: succeeds.
```bash
git add ui/src/components/git/Graph.tsx ui/src/components/git/CommitRow.tsx ui/src/components/git/HistoryPanel.tsx ui/src/components/git/CommitDetail.tsx ui/src/styles.css
git commit -m "Add commit graph, history panel, and commit detail components"
```

---

### Task 13: `ChangesPanel` + `GitPanel` + wire layouts; remove old files

**Files:**
- Create: `ui/src/components/git/ChangesPanel.tsx`, `ui/src/components/git/GitPanel.tsx`
- Modify: `ui/src/components/AgentsView.tsx`, `ui/src/styles.css`
- Delete: `ui/src/components/SourceControl.tsx`, `ui/src/components/GitReviewPanel.tsx`, `ui/src/components/DiffView.tsx`

**Interfaces:**
- Consumes: all of the above + `gitStatus`, `gitBranchInfo`, `gitStage`, `gitUnstage`, `gitStageAll`, `gitUnstageAll`, `gitDiscard`, `gitDiscardAll`, `gitCommit`, `gitCommitAmend`, `gitPush`, `FileChange`, `BranchInfo`, `HistoryItem` (api); `partition`, `groupOf` (status).
- Produces (ChangesPanel): `ChangesPanel({ taskId, changes, branch, onAct, selectedPath, onSelectFile })`.
- Produces (GitPanel): `GitPanel({ taskId, layout })` where `layout: "compact" | "full"`.

- [ ] **Step 1: Implement `ChangesPanel.tsx`**
```tsx
import {
  FileChange, BranchInfo, gitStage, gitUnstage, gitStageAll, gitUnstageAll,
  gitDiscard, gitDiscardAll, gitCommit, gitCommitAmend, gitPush,
} from "../../api";
import { partition } from "./status";
import ResourceGroup from "./ResourceGroup";
import CommitBox from "./CommitBox";

export default function ChangesPanel({
  taskId, changes, branch, onAct, selectedPath, onSelectFile,
}: {
  taskId: string;
  changes: FileChange[];
  branch: BranchInfo | null;
  onAct: (fn: () => Promise<unknown>) => void;
  selectedPath: string | null;
  onSelectFile: (path: string, group: "index" | "workingTree" | "merge" | "untracked") => void;
}) {
  const g = partition(changes);
  const untrackedOf = (c: FileChange) => c.index === "?" || c.worktree === "?";
  return (
    <div className="git-changes">
      <CommitBox
        branch={branch?.branch ?? "?"}
        hasUpstream={!!branch?.upstream}
        ahead={branch?.ahead ?? 0}
        behind={branch?.behind ?? 0}
        onCommit={(m) => onAct(() => gitCommit(taskId, m))}
        onCommitPush={(m) => onAct(async () => { await gitCommit(taskId, m); await gitPush(taskId); })}
        onAmend={(m) => onAct(() => gitCommitAmend(taskId, m))}
        onSync={() => onAct(() => gitPush(taskId))}
        onPublish={() => onAct(() => gitPush(taskId))}
      />
      <ResourceGroup id="merge" label="Merge Changes" changes={g.merge}
        selectedPath={selectedPath} onSelectFile={(c) => onSelectFile(c.path, "merge")}
        onFilePrimary={(c) => onAct(() => gitStage(taskId, c.path))} />
      <ResourceGroup id="index" label="Staged Changes" changes={g.index}
        selectedPath={selectedPath} onSelectFile={(c) => onSelectFile(c.path, "index")}
        onFilePrimary={(c) => onAct(() => gitUnstage(taskId, c.path))}
        onUnstageAll={() => onAct(() => gitUnstageAll(taskId))} />
      <ResourceGroup id="workingTree" label="Changes" changes={g.workingTree}
        selectedPath={selectedPath} onSelectFile={(c) => onSelectFile(c.path, "workingTree")}
        onFilePrimary={(c) => onAct(() => gitStage(taskId, c.path))}
        onFileDiscard={(c) => onAct(() => gitDiscard(taskId, c.path, false))}
        onStageAll={() => onAct(() => gitStageAll(taskId))}
        onDiscardAll={() => onAct(() => gitDiscardAll(taskId))} />
      <ResourceGroup id="untracked" label="Untracked Changes" changes={g.untracked}
        selectedPath={selectedPath} onSelectFile={(c) => onSelectFile(c.path, "untracked")}
        onFilePrimary={(c) => onAct(() => gitStage(taskId, c.path))}
        onFileDiscard={(c) => onAct(() => gitDiscard(taskId, c.path, true))}
        onStageAll={() => onAct(() => gitStageAll(taskId))} />
    </div>
  );
}
```

- [ ] **Step 2: Implement `GitPanel.tsx`**
```tsx
import { useCallback, useEffect, useRef, useState } from "react";
import { FileChange, BranchInfo, HistoryItem, gitStatus, gitBranchInfo, gitPush } from "../../api";
import ChangesPanel from "./ChangesPanel";
import HistoryPanel from "./HistoryPanel";
import CommitDetail from "./CommitDetail";
import DiffViewer from "./DiffViewer";
import BranchBar from "./BranchBar";

type Selection =
  | { kind: "file"; path: string; group: "index" | "workingTree" | "merge" | "untracked" }
  | { kind: "commit"; item: HistoryItem }
  | null;

export default function GitPanel({ taskId, layout }: { taskId: string; layout: "compact" | "full" }) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [branch, setBranch] = useState<BranchInfo | null>(null);
  const [error, setError] = useState("");
  const [tab, setTab] = useState<"changes" | "history">("changes");
  const [sel, setSel] = useState<Selection>(null);
  const visible = useRef(true);

  const refresh = useCallback(async () => {
    try {
      setChanges(await gitStatus(taskId));
      setBranch(await gitBranchInfo(taskId));
      setError("");
    } catch (e) { setError(String(e)); }
  }, [taskId]);

  useEffect(() => { refresh(); }, [refresh]);
  useEffect(() => {
    const id = setInterval(() => { if (visible.current) refresh(); }, 2000);
    return () => clearInterval(id);
  }, [refresh]);

  const act = useCallback((fn: () => Promise<unknown>) => {
    (async () => {
      try { await fn(); setError(""); } catch (e) { setError(String(e)); }
      await refresh();
    })();
  }, [refresh]);

  const onSelectFile = (path: string, group: "index" | "workingTree" | "merge" | "untracked") =>
    setSel({ kind: "file", path, group });

  const diffMode = (group: string): "working-unstaged" | "working-staged" =>
    group === "index" ? "working-staged" : "working-unstaged";

  const changesPanel = (
    <ChangesPanel taskId={taskId} changes={changes} branch={branch} onAct={act}
      selectedPath={sel?.kind === "file" ? sel.path : null} onSelectFile={onSelectFile} />
  );
  const historyPanel = (
    <HistoryPanel taskId={taskId} base={branch?.base ?? null}
      selectedHash={sel?.kind === "commit" ? sel.item.hash : null}
      onSelectCommit={(item) => setSel({ kind: "commit", item })} />
  );

  if (layout === "compact") {
    return (
      <aside className="git-panel compact">
        {error && <div className="git-error">{error}</div>}
        <div className="git-tabs">
          <button className={tab === "changes" ? "on" : ""} onClick={() => setTab("changes")}>Changes</button>
          <button className={tab === "history" ? "on" : ""} onClick={() => setTab("history")}>History</button>
        </div>
        {tab === "changes" ? changesPanel : historyPanel}
        {sel?.kind === "file" && (
          <div className="git-compact-diff">
            <DiffViewer taskId={taskId} path={sel.path} mode={diffMode(sel.group)} onChanged={refresh} />
          </div>
        )}
        {sel?.kind === "commit" && <div className="git-compact-diff"><CommitDetail taskId={taskId} item={sel.item} /></div>}
      </aside>
    );
  }

  return (
    <div className="git-panel full">
      <BranchBar info={branch} onSync={() => act(() => gitPush(taskId))} onRefresh={refresh} />
      {error && <div className="git-error">{error}</div>}
      <div className="git-full-body">
        <div className="git-full-left">
          <div className="git-tabs">
            <button className={tab === "changes" ? "on" : ""} onClick={() => setTab("changes")}>Changes</button>
            <button className={tab === "history" ? "on" : ""} onClick={() => setTab("history")}>History</button>
          </div>
          {tab === "changes" ? changesPanel : historyPanel}
        </div>
        <div className="git-full-right">
          {sel?.kind === "file" && <DiffViewer taskId={taskId} path={sel.path} mode={diffMode(sel.group)} onChanged={refresh} />}
          {sel?.kind === "commit" && <CommitDetail taskId={taskId} item={sel.item} />}
          {!sel && <div className="diff-empty">Select a file or commit.</div>}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Wire into `AgentsView.tsx`**

Replace imports `SourceControl` and `GitReviewPanel` with:
```tsx
import GitPanel from "./git/GitPanel";
```
Replace the `tab === "source"` block body:
```tsx
      {tab === "source" && (
        <div className="source-wrap">
          {focusedRunId ? <GitPanel taskId={focusedRunId} layout="full" /> : <div className="board empty">Open an agent to review its changes.</div>}
        </div>
      )}
```
Replace the review sidebar render:
```tsx
          {review && focusedRunId && (
            <GitPanel taskId={focusedRunId} layout="compact" />
          )}
```

- [ ] **Step 4: Delete old files**

Run:
```bash
git rm ui/src/components/SourceControl.tsx ui/src/components/GitReviewPanel.tsx ui/src/components/DiffView.tsx
```

- [ ] **Step 5: Add layout CSS** to `ui/src/styles.css`:
```css
.git-panel.compact { display: flex; flex-direction: column; width: 360px; min-width: 360px; border-left: 1px solid var(--line); background: var(--mantle); overflow: hidden; }
.git-panel.compact .git-compact-diff { border-top: 1px solid var(--line); min-height: 200px; display: flex; overflow: hidden; }
.git-panel.full { display: flex; flex-direction: column; flex: 1; min-height: 0; }
.git-full-body { display: flex; flex: 1; min-height: 0; }
.git-full-left { width: 360px; min-width: 300px; border-right: 1px solid var(--line); display: flex; flex-direction: column; overflow: auto; }
.git-full-right { flex: 1; display: flex; min-width: 0; min-height: 0; }
.git-tabs { display: flex; gap: 4px; padding: 6px; border-bottom: 1px solid var(--line); }
.git-tabs button { background: none; border: none; color: var(--o1); cursor: pointer; padding: 5px 10px; border-radius: 6px; font-size: 12.5px; }
.git-tabs button.on { background: var(--s0); color: var(--text); }
.git-changes { overflow: auto; }
```

- [ ] **Step 6: Verify build, typecheck, tests**

Run: `cd ui && npx tsc --noEmit && npx vitest run && npm run build`
Expected: typecheck clean (no remaining `SourceControl`/`projectLog` references), all vitest pass, vite build succeeds.

- [ ] **Step 7: Commit**
```bash
git add -A
git commit -m "Wire GitPanel into both surfaces; remove duplicated git components"
```

---

### Task 14: Manual verification + cleanup pass

**Files:**
- Modify: `ui/src/styles.css` (remove dead rules)

- [ ] **Step 1: Remove stale CSS**

In `ui/src/styles.css`, delete now-unused rules for the old components: `.source-control`, `.sc-left`, `.sc-right`, `.sc-commit`, `.sc-file`, `.hist-pills`, `.hist-row`, `.pill`, `.review-panel`, `.review-head`, `.git-panel` (old), `.git-section`, `.git-file`, `.git-status*`, `.git-path`, `.git-diff`, `.diffview`, `.hunk*`. Keep `.git-error` and `.diff-empty` (still used). Grep first to confirm none are referenced by surviving components:

Run: `cd ui && grep -rn "source-control\|sc-left\|sc-file\|hist-row\|review-panel\|git-section\|class=\"git-file\|diffview\|hunk-" src/components | grep -v "/git/"`
Expected: no matches (all such markup was deleted). Remove the corresponding CSS blocks.

- [ ] **Step 2: Launch the app and verify**

Use the `run` skill (or `cd <home>/agency && cargo tauri dev`). Verify against the spec:
1. Open a project, spawn/focus an agent that has made changes.
2. **Source Control tab (full):** branch bar shows branch + ahead/behind; Changes tab shows grouped files with status letters/colors; hover reveals stage/unstage/discard; commit box commits; selecting a file shows a side-by-side, syntax-highlighted diff; selecting a line range and "Stage selection" stages only those lines; History tab shows commits with the graph rail and refs; clicking a commit shows its files + read-only diff.
3. **Review sidebar (compact):** toggle "Review"; confirm it now has both Changes AND History tabs, commit box works, and selecting a file shows an inline diff.
4. Confirm colors match the Catppuccin theme (no foreign colors).

- [ ] **Step 3: Run full test suites**

Run: `cargo test -p agency-core && (cd ui && npx vitest run && npx tsc --noEmit)`
Expected: all green.

- [ ] **Step 4: Commit any cleanup**
```bash
git add -A
git commit -m "Remove stale git component CSS"
```

---

## Self-Review notes (addressed)

- **Spec coverage:** §1 architecture → Tasks 9–13; §2 backend → Tasks 1–4; §3 changes view → Tasks 9,10,13; §4 diff viewer → Tasks 6,8,11; §5 history+graph → Tasks 2,3,7,12; §6 layouts+theming → Tasks 9–14 CSS; §7 refresh/errors → Task 13 (`GitPanel` interval + `act`); §8 testing → Tasks 1–7 tests.
- **Known simplifications (intentional, noted in-task):** the graph renderer draws straight vertical lane rails + node rather than VSCode's S-curve connectors (Task 12 Step 1); the word-diff overlay applies in the non-highlighted render path, with highlighted lines falling back to a line-level wash (Task 11). Both are legible and shippable; curve/overlay-merge are follow-ups if desired.
- **Type consistency:** `taskId`/`task_id`, `hunkIndex`/`hunk_index`, `lines: number[]`/`Vec<usize>` align across api.ts ↔ commands.rs. `DiffViewer` `mode` union is used consistently by `GitPanel` and `CommitDetail`. `GitGroup` values match between `status.ts`, `ResourceGroup`, `ChangesPanel`, and `GitPanel`.
