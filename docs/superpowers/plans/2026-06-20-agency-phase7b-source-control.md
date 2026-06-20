# Agency Phase 7b Implementation Plan — Source Control Redesign + Per-Hunk Staging

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Add real per-hunk git staging and rebuild Agency's git surface into a VSCode-style Source Control screen + a Git Review side-panel, matching the design handoff.

**Architecture:** New hunk-parsing + per-hunk apply functions in `agency_core::git` (reconstruct a patch, `git apply --cached [--reverse]`). Thin `agency-app` commands expose them. The frontend replaces `GitPanel` with `DiffView` (per-hunk), `SourceControl` (full screen + history pills), and `GitReviewPanel` (Agents side panel).

**Tech Stack:** Rust (std `Command` → git, with stdin piping), Tauri v2, React + TypeScript.

## Global Constraints

- **Local-only / zero first-party data collection:** no network in Agency's own code (`git push` is user-initiated).
- **Reuse, don't duplicate:** existing `git::{status,diff,commit,push,log,diff_stat,stage,unstage}` and the Phase-7a `git_*` commands (which take a run id as `task_id`) stay; add only what's new.
- **Per-hunk addressing:** `hunk_index` is an index into the FRESHLY recomputed diff at apply time (backend recomputes before applying) — never a stale offset.
- **Apply mechanism:** stage = `git apply --cached --unidiff-zero -` with patch on stdin; unstage = add `--reverse`.
- **Design authority:** `docs/design-handoff/Agency-Design-Handoff.html` (§03 Source Control, §07 diff/file row) + `docs/design-handoff/Agency v2.dc.html`. Use Catppuccin tokens in `ui/src/theme.css`; no literal hexes in components.
- **Frontend↔Rust:** DTOs serialize with field names; Tauri maps camelCase JS args to snake_case Rust params.
- TDD for Rust; commit after each green task. Frontend tasks build-verified.

---

## File Structure

```
crates/agency-core/
├── src/git.rs          # MODIFY: Hunk, FileDiff, parse_diff/parse_file_diff, stage_hunk/unstage_hunk, git_stdin
└── tests/git.rs        # MODIFY: parse + per-hunk stage/unstage tests

crates/agency-app/
├── src/commands.rs     # MODIFY: git_parse_diff, git_stage_hunk, git_unstage_hunk, project_log
└── src/lib.rs          # MODIFY: register the 4 commands

ui/src/
├── api.ts                       # MODIFY: Hunk/FileDiff types + 4 wrappers
├── components/DiffView.tsx      # NEW: per-hunk diff renderer + stage/unstage hunk
├── components/SourceControl.tsx # NEW: full Source Control screen + history pills
├── components/GitReviewPanel.tsx# NEW: compact Agents-view review panel
├── components/AgentsView.tsx    # MODIFY: Source tab → SourceControl; Review toggle → GitReviewPanel
└── styles.css                   # MODIFY: diff/source-control/review styles
```

Deleted at the end: `ui/src/components/GitPanel.tsx` (superseded).

---

### Task 1: `agency_core::git` — hunk parsing

**Files:**
- Modify: `crates/agency-core/src/git.rs`
- Test: `crates/agency-core/tests/git.rs`

**Interfaces:**
- Produces:
  - `struct Hunk { pub header: String, pub lines: Vec<String> }` (derives `Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize`). `header` = the `@@ … @@` line; `lines` = body lines WITH their leading `+`/`-`/` ` (space) prefix, excluding the `@@` line.
  - `struct FileDiff { pub header: String, pub hunks: Vec<Hunk> }` (same derives). `header` = all lines before the first `@@` (the `diff --git`, `index`, `---`, `+++`), joined with `\n` and trailing `\n` (empty string if the diff has no hunks).
  - `fn parse_diff(diff: &str) -> FileDiff` — splits a single-file unified diff into header + hunks.

- [ ] **Step 1: Write the failing test**

Append to `crates/agency-core/tests/git.rs`:

```rust
use agency_core::git::{parse_diff, Hunk};

#[test]
fn parse_diff_splits_header_and_hunks() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path()); // tracked.txt = "one\n"
    // Two separated changes → two hunks.
    std::fs::write(
        dir.path().join("tracked.txt"),
        "ADDED-TOP\none\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\nADDED-BOTTOM\n",
    )
    .unwrap();
    let raw = git_raw_diff(dir.path(), "tracked.txt");
    let fd = parse_diff(&raw);

    // Header captured (the diff --git / --- / +++ lines).
    assert!(fd.header.contains("diff --git"));
    assert!(fd.header.contains("+++ b/tracked.txt"));
    // Two hunks, each starting with @@.
    assert_eq!(fd.hunks.len(), 2, "hunks: {:#?}", fd.hunks);
    assert!(fd.hunks[0].header.starts_with("@@"));
    // First hunk has the top addition, second the bottom.
    assert!(fd.hunks[0].lines.iter().any(|l| l == "+ADDED-TOP"));
    assert!(fd.hunks[1].lines.iter().any(|l| l == "+ADDED-BOTTOM"));
}

#[test]
fn parse_diff_empty_for_no_changes() {
    let fd = parse_diff("");
    assert_eq!(fd.header, "");
    assert!(fd.hunks.is_empty());
}

// Helper: raw unstaged diff for a path.
fn git_raw_diff(dir: &std::path::Path, path: &str) -> String {
    let out = std::process::Command::new("git")
        .args(["diff", "--", path])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}
```

Note: `init_repo` is the existing helper in `tests/git.rs`. If it writes a single short line, the two-hunk split requires enough separation (≥ 4 context lines between changes); the test data above (top + bottom of a 10-line body) guarantees two hunks.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core --test git`
Expected: FAIL — `parse_diff`/`Hunk` not found.

- [ ] **Step 3: Implement**

Append to `crates/agency-core/src/git.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub header: String,
    pub hunks: Vec<Hunk>,
}

pub fn parse_diff(diff: &str) -> FileDiff {
    let mut header_lines: Vec<&str> = Vec::new();
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut seen_hunk = false;

    for line in diff.lines() {
        if line.starts_with("@@") {
            seen_hunk = true;
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(Hunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
        } else if let Some(h) = current.as_mut() {
            h.lines.push(line.to_string());
        } else if !seen_hunk {
            header_lines.push(line);
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }

    let header = if header_lines.is_empty() {
        String::new()
    } else {
        let mut s = header_lines.join("\n");
        s.push('\n');
        s
    };
    FileDiff { header, hunks }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-core --test git`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/git.rs crates/agency-core/tests/git.rs
git commit -m "Add unified-diff hunk parsing"
```

---

### Task 2: `agency_core::git` — per-hunk stage/unstage

**Files:**
- Modify: `crates/agency-core/src/git.rs`
- Test: `crates/agency-core/tests/git.rs`

**Interfaces:**
- Consumes: the private `git()` helper, `parse_diff`, and `diff` from Task 1/Phase 3.
- Produces:
  - `fn git_stdin(worktree: &Path, args: &[&str], input: &str) -> Result<()>` — runs git with `input` on stdin; bails with stderr on non-zero exit.
  - `fn stage_hunk(worktree: &Path, path: &str, hunk_index: usize) -> Result<()>` — recompute `git diff -- <path>`, `parse_diff`, build patch = `header + hunk[hunk_index].header + "\n" + lines…`, apply `git apply --cached --unidiff-zero -`.
  - `fn unstage_hunk(worktree: &Path, path: &str, hunk_index: usize) -> Result<()>` — recompute `git diff --cached -- <path>`, same patch build, apply `git apply --cached --reverse --unidiff-zero -`.
  - Patch must end with a trailing newline.

- [ ] **Step 1: Write the failing test**

Append to `crates/agency-core/tests/git.rs`:

```rust
use agency_core::git::{stage_hunk, unstage_hunk};

fn staged_diff(dir: &std::path::Path, path: &str) -> String {
    let out = std::process::Command::new("git")
        .args(["diff", "--cached", "--", path])
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn stage_hunk_stages_only_that_hunk() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(
        dir.path().join("tracked.txt"),
        "ADDED-TOP\none\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\nADDED-BOTTOM\n",
    )
    .unwrap();

    // Stage the first hunk only.
    stage_hunk(dir.path(), "tracked.txt", 0).unwrap();

    let staged = staged_diff(dir.path(), "tracked.txt");
    assert!(staged.contains("+ADDED-TOP"), "staged: {staged}");
    assert!(!staged.contains("+ADDED-BOTTOM"), "staged: {staged}");

    // The bottom change is still unstaged.
    let out = std::process::Command::new("git")
        .args(["diff", "--", "tracked.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let unstaged = String::from_utf8_lossy(&out.stdout);
    assert!(unstaged.contains("+ADDED-BOTTOM"), "unstaged: {unstaged}");

    // Unstage it back.
    unstage_hunk(dir.path(), "tracked.txt", 0).unwrap();
    let staged2 = staged_diff(dir.path(), "tracked.txt");
    assert!(!staged2.contains("+ADDED-TOP"), "staged2: {staged2}");
}

#[test]
fn staging_all_hunks_equals_staging_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(
        dir.path().join("tracked.txt"),
        "ADDED-TOP\none\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\nADDED-BOTTOM\n",
    )
    .unwrap();

    // Stage hunk 0, then the (now-only-remaining) hunk 0 again.
    stage_hunk(dir.path(), "tracked.txt", 0).unwrap();
    stage_hunk(dir.path(), "tracked.txt", 0).unwrap();

    // Nothing left unstaged for the file.
    let out = std::process::Command::new("git")
        .args(["diff", "--", "tracked.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core --test git`
Expected: FAIL — `stage_hunk`/`unstage_hunk` not found.

- [ ] **Step 3: Implement**

Append to `crates/agency-core/src/git.rs`:

```rust
use std::io::Write;
use std::process::Stdio;

pub fn git_stdin(worktree: &Path, args: &[&str], input: &str) -> Result<()> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("failed to open git stdin"))?
        .write_all(input.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

fn build_hunk_patch(file_diff: &FileDiff, hunk_index: usize) -> Result<String> {
    let hunk = file_diff
        .hunks
        .get(hunk_index)
        .ok_or_else(|| anyhow::anyhow!("hunk index {hunk_index} out of range"))?;
    let mut patch = file_diff.header.clone();
    patch.push_str(&hunk.header);
    patch.push('\n');
    for line in &hunk.lines {
        patch.push_str(line);
        patch.push('\n');
    }
    Ok(patch)
}

pub fn stage_hunk(worktree: &Path, path: &str, hunk_index: usize) -> Result<()> {
    let raw = diff(worktree, path, false)?;
    let fd = parse_diff(&raw);
    let patch = build_hunk_patch(&fd, hunk_index)?;
    git_stdin(worktree, &["apply", "--cached", "--unidiff-zero", "-"], &patch)
}

pub fn unstage_hunk(worktree: &Path, path: &str, hunk_index: usize) -> Result<()> {
    let raw = diff(worktree, path, true)?;
    let fd = parse_diff(&raw);
    let patch = build_hunk_patch(&fd, hunk_index)?;
    git_stdin(
        worktree,
        &["apply", "--cached", "--reverse", "--unidiff-zero", "-"],
        &patch,
    )
}
```

Note: if `--unidiff-zero` causes apply to reject a normal-context patch on your git version, the fallback is to drop it (plain `git apply --cached -`); the tests will tell you. Keep `--unidiff-zero` first since it makes non-zero-context single-hunk patches apply more reliably.

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-core --test git`
Expected: PASS. If a hunk patch is rejected, remove `--unidiff-zero` and re-run (document the change).

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/git.rs crates/agency-core/tests/git.rs
git commit -m "Add per-hunk stage/unstage via git apply --cached"
```

---

### Task 3: `agency-app` — hunk + project-log commands

**Files:**
- Modify: `crates/agency-app/src/commands.rs`, `crates/agency-app/src/lib.rs`

**Interfaces:**
- Consumes: `AppState::worktree_path(id)`, `agency_core::git::{parse_diff, stage_hunk, unstage_hunk, diff, log, FileDiff, CommitInfo}`, and a project-repo lookup. AppState already has `worktree_path`; add a public `project_repo_path(project_id) -> Result<PathBuf>` if not present (Phase 7a has private `project_repo`; expose a thin public method `pub fn project_repo_path(&self, project_id: &str) -> Result<PathBuf>` delegating to it).
- Produces commands (thin, map anyhow→String):
  - `git_parse_diff(task_id, path, staged) -> FileDiff`
  - `git_stage_hunk(task_id, path, hunk_index) -> ()`
  - `git_unstage_hunk(task_id, path, hunk_index) -> ()`
  - `project_log(project_id, limit) -> Vec<CommitInfo>`
- Register all four.

- [ ] **Step 1: Add the AppState project-repo accessor (if missing)**

In `crates/agency-app/src/state.rs`, add (if `project_repo` is private):

```rust
    pub fn project_repo_path(&self, project_id: &str) -> anyhow::Result<std::path::PathBuf> {
        self.project_repo(project_id)
    }
```

- [ ] **Step 2: Add the commands**

In `crates/agency-app/src/commands.rs`:

```rust
use agency_core::git::FileDiff;

#[tauri::command]
pub fn git_parse_diff(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    staged: bool,
) -> Result<FileDiff, String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    let raw = agency_core::git::diff(&wt, &path, staged).map_err(|e| e.to_string())?;
    Ok(agency_core::git::parse_diff(&raw))
}

#[tauri::command]
pub fn git_stage_hunk(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::stage_hunk(&wt, &path, hunk_index).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn git_unstage_hunk(
    state: State<'_, AppState>,
    task_id: String,
    path: String,
    hunk_index: usize,
) -> Result<(), String> {
    let wt = state.worktree_path(&task_id).map_err(|e| e.to_string())?;
    agency_core::git::unstage_hunk(&wt, &path, hunk_index).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn project_log(
    state: State<'_, AppState>,
    project_id: String,
    limit: usize,
) -> Result<Vec<agency_core::git::CommitInfo>, String> {
    let repo = state.project_repo_path(&project_id).map_err(|e| e.to_string())?;
    agency_core::git::log(&repo, limit).map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Register**

Add to `crates/agency-app/src/lib.rs` `generate_handler!`: `git_parse_diff, git_stage_hunk, git_unstage_hunk, project_log`.

- [ ] **Step 4: Build + test**

Run: `cargo build -p agency-app` (warning-free) and `cargo test --workspace` (green).

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs crates/agency-app/src/state.rs
git commit -m "Add hunk and project-log commands"
```

---

### Task 4: Frontend — diff/hunk API + DiffView

**Files:**
- Modify: `ui/src/api.ts`
- Create: `ui/src/components/DiffView.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- `api.ts`: `interface Hunk { header: string; lines: string[] }`, `interface FileDiff { header: string; hunks: Hunk[] }`; wrappers `gitParseDiff(taskId, path, staged) -> FileDiff`, `gitStageHunk(taskId, path, hunkIndex) -> void`, `gitUnstageHunk(taskId, path, hunkIndex) -> void`, `projectLog(projectId, limit) -> CommitInfo[]`.
- `DiffView({ taskId, path, staged, onChanged })` — fetches `gitParseDiff`, renders each hunk (header row + colored lines), and a "Stage hunk"/"Unstage hunk" button per hunk that calls `gitStageHunk`/`gitUnstageHunk(taskId, path, i)` then `onChanged()`.

- [ ] **Step 1: API wrappers**

Append to `ui/src/api.ts`:

```ts
export interface Hunk {
  header: string;
  lines: string[];
}
export interface FileDiff {
  header: string;
  hunks: Hunk[];
}

export const gitParseDiff = (taskId: string, path: string, staged: boolean) =>
  invoke<FileDiff>("git_parse_diff", { taskId, path, staged });
export const gitStageHunk = (taskId: string, path: string, hunkIndex: number) =>
  invoke<void>("git_stage_hunk", { taskId, path, hunkIndex });
export const gitUnstageHunk = (taskId: string, path: string, hunkIndex: number) =>
  invoke<void>("git_unstage_hunk", { taskId, path, hunkIndex });
export const projectLog = (projectId: string, limit: number) =>
  invoke<CommitInfo[]>("project_log", { projectId, limit });
```

(`CommitInfo` already exists in api.ts from Phase 3.)

- [ ] **Step 2: DiffView**

Create `ui/src/components/DiffView.tsx`:

```tsx
import { useCallback, useEffect, useState } from "react";
import { FileDiff, gitParseDiff, gitStageHunk, gitUnstageHunk } from "../api";

function lineClass(line: string): string {
  if (line.startsWith("+")) return "diff-add";
  if (line.startsWith("-")) return "diff-del";
  return "diff-ctx";
}

export default function DiffView({
  taskId,
  path,
  staged,
  onChanged,
}: {
  taskId: string;
  path: string;
  staged: boolean;
  onChanged: () => void;
}) {
  const [fd, setFd] = useState<FileDiff | null>(null);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    try {
      setFd(await gitParseDiff(taskId, path, staged));
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }, [taskId, path, staged]);

  useEffect(() => {
    load();
  }, [load]);

  async function act(i: number) {
    try {
      if (staged) await gitUnstageHunk(taskId, path, i);
      else await gitStageHunk(taskId, path, i);
      await load();
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  }

  if (error) return <div className="git-error">{error}</div>;
  if (!fd) return <div className="diff-empty">loading…</div>;
  if (fd.hunks.length === 0) return <div className="diff-empty">no textual changes</div>;

  return (
    <div className="diffview">
      {fd.hunks.map((h, i) => (
        <div className="hunk" key={i}>
          <div className="hunk-head">
            <code className="diff-hunk">{h.header}</code>
            <button onClick={() => act(i)}>{staged ? "Unstage hunk" : "Stage hunk"}</button>
          </div>
          <pre className="hunk-body">
            {h.lines.map((l, j) => (
              <div className={lineClass(l)} key={j}>
                {l || " "}
              </div>
            ))}
          </pre>
        </div>
      ))}
    </div>
  );
}
```

- [ ] **Step 3: Styles** — append diff/hunk rules to `ui/src/styles.css` (tokens): `.diffview`, `.hunk`, `.hunk-head{display:flex;justify-content:space-between;align-items:center}`, `.hunk-body{font-family:var(--mono);white-space:pre;…}`, reuse `.diff-add/.diff-del/.diff-ctx/.diff-hunk` (already exist from Phase 6).

- [ ] **Step 4: Build**

Run: `pnpm --dir ui build` (DiffView not yet mounted; types check) and `pnpm --dir ui test` (vitest green).

- [ ] **Step 5: Commit**

```bash
git add ui/src/api.ts ui/src/components/DiffView.tsx ui/src/styles.css
git commit -m "Add hunk API and per-hunk DiffView"
```

---

### Task 5: Frontend — SourceControl screen + GitReviewPanel, wire into AgentsView, delete GitPanel

**Files:**
- Create: `ui/src/components/SourceControl.tsx`, `ui/src/components/GitReviewPanel.tsx`
- Modify: `ui/src/components/AgentsView.tsx`, `ui/src/styles.css`
- Delete: `ui/src/components/GitPanel.tsx`

**Interfaces:**
- `SourceControl({ taskId })` — staged/unstaged file lists (`gitStatus`), per-file stage/unstage (`gitStage`/`gitUnstage`), selected-file `DiffView` (staged flag from which list the file is in), commit box (`gitCommit`/`gitPush`), and a history section: project pills from `listProjects` (initials) that load `projectLog(projectId, 50)`.
- `GitReviewPanel({ taskId, onOpenSource })` — compact: branch + diff stat (from the run, via `listRuns`/the focused RunInfo), commit box (Commit/Push), staged & changed file lists with per-file stage/unstage, and an "Open in Source Control →" button calling `onOpenSource()`.
- `AgentsView` — the "Source Control" tab renders `<SourceControl taskId={focusedRunId}/>` (or an empty state if none focused); add a "Review" toggle (header) that shows `<GitReviewPanel taskId={focusedRunId} onOpenSource={() => setTab("source")} />` beside Grid/Focus. Remove the old `GitPanel` import/usage.

- [ ] **Step 1: SourceControl**

Create `ui/src/components/SourceControl.tsx`:

```tsx
import { useCallback, useEffect, useState } from "react";
import {
  CommitInfo,
  FileChange,
  Project,
  gitCommit,
  gitPush,
  gitStage,
  gitStatus,
  gitUnstage,
  listProjects,
  projectLog,
} from "../api";
import DiffView from "./DiffView";

function isStaged(c: FileChange): boolean {
  return c.index !== " " && c.index !== "?";
}

export default function SourceControl({ taskId }: { taskId: string }) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [selected, setSelected] = useState<{ path: string; staged: boolean } | null>(null);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [projects, setProjects] = useState<Project[]>([]);
  const [histProject, setHistProject] = useState<string | null>(null);
  const [commits, setCommits] = useState<CommitInfo[]>([]);

  const refresh = useCallback(async () => {
    try {
      setChanges(await gitStatus(taskId));
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }, [taskId]);

  useEffect(() => {
    refresh();
    listProjects().then(setProjects);
  }, [refresh]);

  useEffect(() => {
    if (histProject) projectLog(histProject, 50).then(setCommits).catch(() => setCommits([]));
  }, [histProject]);

  async function act(fn: () => Promise<unknown>) {
    try {
      await fn();
      setError("");
    } catch (e) {
      setError(String(e));
    }
    await refresh();
  }

  const staged = changes.filter(isStaged);
  const unstaged = changes.filter((c) => !isStaged(c));

  return (
    <div className="source-control">
      <div className="sc-left">
        <div className="sc-commit">
          <textarea
            placeholder="Commit message"
            value={message}
            onChange={(e) => setMessage(e.target.value)}
          />
          <div className="row-actions">
            <button onClick={() => act(async () => { await gitCommit(taskId, message); setMessage(""); })}>
              Commit
            </button>
            <button className="ghost" onClick={() => act(() => gitPush(taskId))}>Push</button>
          </div>
        </div>
        {error && <div className="git-error">{error}</div>}

        <h4>Staged</h4>
        {staged.length === 0 && <div className="git-empty">none</div>}
        {staged.map((c) => (
          <div key={c.path} className="sc-file">
            <span className="git-status">{c.index}</span>
            <span className="git-path" onClick={() => setSelected({ path: c.path, staged: true })}>{c.path}</span>
            <button onClick={() => act(() => gitUnstage(taskId, c.path))}>−</button>
          </div>
        ))}

        <h4>Changes</h4>
        {unstaged.length === 0 && <div className="git-empty">none</div>}
        {unstaged.map((c) => (
          <div key={c.path} className="sc-file">
            <span className="git-status">{c.index === "?" ? "?" : c.worktree}</span>
            <span className="git-path" onClick={() => setSelected({ path: c.path, staged: false })}>{c.path}</span>
            <button onClick={() => act(() => gitStage(taskId, c.path))}>+</button>
          </div>
        ))}

        <h4>History</h4>
        <div className="hist-pills">
          {projects.map((p) => (
            <button
              key={p.id}
              className={`pill ${histProject === p.id ? "on" : ""}`}
              title={p.name}
              onClick={() => setHistProject(p.id)}
            >
              {p.name.slice(0, 1).toUpperCase()}
            </button>
          ))}
        </div>
        {commits.map((c) => (
          <div key={c.hash} className="hist-row">
            <code>{c.hash.slice(0, 7)}</code> <span>{c.summary}</span>
          </div>
        ))}
      </div>

      <div className="sc-right">
        {selected ? (
          <DiffView taskId={taskId} path={selected.path} staged={selected.staged} onChanged={refresh} />
        ) : (
          <div className="diff-empty">Select a file to view its diff.</div>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: GitReviewPanel**

Create `ui/src/components/GitReviewPanel.tsx`:

```tsx
import { useCallback, useEffect, useState } from "react";
import { FileChange, gitCommit, gitPush, gitStage, gitStatus, gitUnstage } from "../api";

function isStaged(c: FileChange): boolean {
  return c.index !== " " && c.index !== "?";
}

export default function GitReviewPanel({
  taskId,
  onOpenSource,
}: {
  taskId: string;
  onOpenSource: () => void;
}) {
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");

  const refresh = useCallback(async () => {
    try {
      setChanges(await gitStatus(taskId));
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }, [taskId]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function act(fn: () => Promise<unknown>) {
    try {
      await fn();
      setError("");
    } catch (e) {
      setError(String(e));
    }
    await refresh();
  }

  const staged = changes.filter(isStaged);
  const unstaged = changes.filter((c) => !isStaged(c));

  return (
    <aside className="review-panel">
      <div className="review-head">
        <h3>Review</h3>
        <button className="ghost" onClick={onOpenSource}>Open in Source Control →</button>
      </div>
      {error && <div className="git-error">{error}</div>}
      <div className="sc-commit">
        <textarea placeholder="Commit message" value={message} onChange={(e) => setMessage(e.target.value)} />
        <div className="row-actions">
          <button onClick={() => act(async () => { await gitCommit(taskId, message); setMessage(""); })}>Commit</button>
          <button className="ghost" onClick={() => act(() => gitPush(taskId))}>Push</button>
        </div>
      </div>
      <h4>Staged</h4>
      {staged.length === 0 && <div className="git-empty">none</div>}
      {staged.map((c) => (
        <div key={c.path} className="sc-file">
          <span className="git-status">{c.index}</span>
          <span className="git-path">{c.path}</span>
          <button onClick={() => act(() => gitUnstage(taskId, c.path))}>−</button>
        </div>
      ))}
      <h4>Changes</h4>
      {unstaged.length === 0 && <div className="git-empty">none</div>}
      {unstaged.map((c) => (
        <div key={c.path} className="sc-file">
          <span className="git-status">{c.index === "?" ? "?" : c.worktree}</span>
          <span className="git-path">{c.path}</span>
          <button onClick={() => act(() => gitStage(taskId, c.path))}>+</button>
        </div>
      ))}
    </aside>
  );
}
```

- [ ] **Step 3: Wire AgentsView + delete GitPanel**

In `ui/src/components/AgentsView.tsx`: replace `import GitPanel from "./GitPanel";` with `import SourceControl from "./SourceControl";` and `import GitReviewPanel from "./GitReviewPanel";`. Add `const [review, setReview] = useState(false);`. In the content header (when `tab === "agents"`), add a Review toggle button `onClick={() => setReview((r) => !r)}`. Render the source tab as:

```tsx
{tab === "source" && (
  <div className="source-wrap">
    {focusedRunId ? <SourceControl taskId={focusedRunId} /> : <div className="board empty">Open an agent to review its changes.</div>}
  </div>
)}
```

And in the agents tab, when `review && focusedRunId`, render `<GitReviewPanel taskId={focusedRunId} onOpenSource={() => setTab("source")} />` alongside the Grid/Focus content (e.g. in a flex row).

Delete the file:

```bash
git rm ui/src/components/GitPanel.tsx
```

Grep for stragglers: `grep -rn "GitPanel" ui/src` → none.

- [ ] **Step 4: Styles** — append `.source-control{display:flex;gap:14px}`, `.sc-left`, `.sc-right`, `.sc-file`, `.hist-pills`, `.pill`, `.hist-row`, `.review-panel{width:360px;…}`, `.review-head` to `ui/src/styles.css` (tokens; mockup §03 + v2 review panel ~447–489).

- [ ] **Step 5: Build + test green**

Run: `pnpm --dir ui build` (tsc + vite, no errors), `pnpm --dir ui test` (vitest green), `cargo test --workspace` (green).

- [ ] **Step 6: Manual smoke (human, record)**

`cargo tauri dev`: start a run that modifies a file; open the **Source Control** tab → see staged/unstaged + click a file → per-hunk **Stage hunk** moves just that hunk to staged; toggle **Review** in the Agents view → the side panel shows the same; history pills switch project logs. Visual fidelity vs `Agency v2.dc.html` is a human check.

- [ ] **Step 7: Commit**

```bash
git add ui/src/components/SourceControl.tsx ui/src/components/GitReviewPanel.tsx ui/src/components/AgentsView.tsx ui/src/styles.css
git commit -m "Add Source Control screen and Git Review panel; retire GitPanel"
```

---

## Self-Review

**Spec coverage (7b):**
- Hunk parsing → Task 1. ✓
- Per-hunk stage/unstage (real, `git apply --cached`) → Task 2. ✓
- Hunk + project-log commands → Task 3. ✓
- DiffView (per-hunk render + stage/unstage) → Task 4. ✓
- Source Control screen + history pills + Git Review panel; retire GitPanel → Task 5. ✓
- Deferred (7c, documented): modals, ⌘K, shortcuts, settings reskin, collapse polish. Split diff / per-line staging: out of scope (documented in spec).

**Placeholder scan:** None — every step contains complete, real code.

**Type consistency:** `Hunk`/`FileDiff` identical Rust↔TS; `git_parse_diff/git_stage_hunk/git_unstage_hunk/project_log` names match across git.rs ↔ commands.rs ↔ api.ts. `hunk_index: usize` ↔ `hunkIndex: number`. `FileChange`/`CommitInfo`/`gitStatus/gitStage/gitUnstage/gitCommit/gitPush` are the existing Phase-3 names. `worktree_path(taskId)` and the new `project_repo_path` are consistent.

**Notes for the executor:**
- Requires `git` (present). The per-hunk apply is the risk; Task 2 has a documented `--unidiff-zero` fallback.
- Task 5 deletes `GitPanel` — confirm `AgentsView` was its only consumer (Phase 7a) before deleting.
