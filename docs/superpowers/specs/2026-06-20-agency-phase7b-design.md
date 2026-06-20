# Agency Phase 7b — Source Control Redesign + Per-Hunk Staging + Git Review Panel (Design)

**Date:** 2026-06-20
**Status:** Approved design, pre-planning

## 1. Summary

Phase 7b redesigns Agency's git surface to match the handoff mockup (`docs/design-handoff/`):
a VSCode-style **Source Control** screen with per-file AND **per-hunk staging**, a unified
diff viewer with "Stage hunk" buttons, a history section with project-switch pills, plus a
**Git Review side-panel** in the Agents view scoped to the focused run. The headline new
capability is real **per-hunk staging** via constructed patches applied with `git apply`.

Builds on Phase 7a (multi-agent shell, runs, tmux). 7c follows (modals, ⌘K, shortcuts,
settings reskin).

## 2. Goals / Non-goals

### Goals
- **Per-hunk staging backend:** parse a file diff into hunks; stage/unstage an individual
  hunk by reconstructing a minimal patch and applying it with `git apply --cached`
  (`--reverse` to unstage).
- **Source Control screen** (the existing segmented "Source Control" tab): commit message
  box; staged vs unstaged file lists with per-file stage/unstage; a unified diff viewer for
  the selected file with per-hunk "Stage hunk" / "Unstage hunk" buttons and add/del/hunk
  coloring; a history section listing recent commits.
- **History project pills** (mockup A/W/M/D): switch which managed project's commit log is
  shown (uses `git::log` on each project's repo).
- **Git Review side-panel** in the Agents view: a "Review" toggle in the content header;
  shows the focused run's branch, diff stat, a commit box (Commit/Push), staged & changed
  files (per-file stage/unstage), and "Open in Source Control →" that deep-links to the full
  Source Control screen for that run.
- Reuse the engine: existing `status`/`diff`/`commit`/`push`/`log`/`diff_stat`/`stage`/
  `unstage` and the Phase-7a `git_*` commands (which take a run id as `task_id`).

### Non-goals (deferred)
- 7c: modal polish, ⌘K palette, keyboard shortcuts, settings reskin, collapse animations.
- Word/character-level intra-line diffs; side-by-side (split) diff view (unified only).
- Staging individual lines (hunk granularity only).
- Merge-conflict editing UI (the resolver agent handles conflicts, Phase 5).

## 3. Architecture

### 3.1 `agency_core::git` — hunk model + per-hunk staging
- `struct Hunk { pub header: String, pub lines: Vec<String>, pub old_start: u32, pub new_start: u32 }`
  (derives Debug/Clone/PartialEq/Serialize/Deserialize). `header` is the `@@ … @@` line;
  `lines` are the body lines including their leading `+`/`-`/` ` prefix.
- `parse_file_diff(diff: &str) -> Vec<Hunk>` — splits a single-file unified diff (the output
  of `git diff [--cached] -- <path>`, including the `diff --git`/`---`/`+++` header) into
  hunks. Keeps the file header separately so a patch can be reconstructed.
- `struct FileDiff { pub header: String, pub hunks: Vec<Hunk> }` + `parse_diff(diff) ->
  FileDiff` — `header` = the lines before the first `@@` (the `diff --git`, `index`, `---`,
  `+++`); `hunks` = the `@@` sections.
- `stage_hunk(worktree: &Path, path: &str, hunk_index: usize) -> Result<()>` —
  recompute the unstaged diff for `path` (`git diff -- <path>`), parse it, take the hunk at
  `hunk_index`, rebuild a patch = `FileDiff.header` + that single hunk, and pipe it to
  `git apply --cached --unidiff-zero -` via stdin.
- `unstage_hunk(worktree: &Path, path: &str, hunk_index: usize) -> Result<()>` — recompute
  the **staged** diff (`git diff --cached -- <path>`), take the hunk, rebuild the patch, and
  apply `git apply --cached --reverse --unidiff-zero -`.
- Index-based (`hunk_index` into the freshly recomputed diff) avoids stale-hunk drift: the
  caller passes which hunk in the current diff, and the backend always recomputes before
  applying.
- A helper `git_stdin(worktree, args, input: &str) -> Result<()>` runs git with stdin
  (existing `git()` helper does not pipe stdin; add this sibling).

### 3.2 `agency-app` — commands
- `git_parse_diff(task_id, path, staged) -> FileDiff` — resolve worktree, `git::diff` then
  `git::parse_diff` (so the UI gets structured hunks; the existing `git_diff` string command
  stays for any plain use).
- `git_stage_hunk(task_id, path, hunk_index) -> ()`, `git_unstage_hunk(task_id, path,
  hunk_index) -> ()`.
- `project_log(project_id, limit) -> Vec<CommitInfo>` — `git::log` on a project's repo
  (for the history pills, which are per *project*, not per run).
- Register all four. Existing `git_*` run-scoped commands unchanged.

### 3.3 Frontend
- **`ui/src/api.ts`**: `Hunk`, `FileDiff` types; `gitParseDiff(taskId, path, staged)`,
  `gitStageHunk(taskId, path, hunkIndex)`, `gitUnstageHunk(taskId, path, hunkIndex)`,
  `projectLog(projectId, limit)`.
- **`components/DiffView.tsx`** (new): renders a `FileDiff` — per-hunk blocks, each with a
  `@@` header row (blue) + add/del/context lines colored, and a "Stage hunk"/"Unstage hunk"
  button per hunk (calls `gitStageHunk`/`gitUnstageHunk` then triggers a refresh). Reused by
  both the Source Control screen and the Review panel.
- **`components/SourceControl.tsx`** (new): the full screen — staged/unstaged file lists
  (per-file +/−), selected-file `DiffView`, commit box (Commit/Push), and a history section
  with project pills (initials, from `listProjects`) that load `projectLog` for the picked
  project. Replaces the in-Agents `GitPanel` usage under the "Source Control" tab.
- **`components/GitReviewPanel.tsx`** (new): the compact Agents-view side panel — branch,
  diff stat, commit box (Commit/Push), staged/changed files (per-file +/−), "Open in Source
  Control →" (switches the Agents tab to source for the focused run). Toggled by a "Review"
  button in the content header (Grid and Focus).
- **`components/AgentsView.tsx`** (modify): the "Source Control" tab now renders
  `SourceControl` (scoped to the focused run, or a project picker if none); add the "Review"
  toggle that shows `GitReviewPanel` beside Grid/Focus.
- **`GitPanel.tsx`**: superseded by `SourceControl` + `GitReviewPanel`; delete after wiring,
  updating `AgentsView` (the only consumer) accordingly.
- Styles: extend `ui/src/styles.css` with diff/hunk/source-control/review-panel rules using
  the existing Catppuccin tokens (mockup §07 diff + file row, §03 layouts).

## 4. Data flow (per-hunk stage)
1. Source Control shows unstaged file `f.rs`; `gitParseDiff(runId, "f.rs", false)` →
   `FileDiff{header, hunks}`.
2. User clicks "Stage hunk" on hunk index 1 → `gitStageHunk(runId, "f.rs", 1)`.
3. Backend recomputes `git diff -- f.rs`, rebuilds `header + hunks[1]`, pipes to
   `git apply --cached --unidiff-zero`.
4. UI refreshes status + both diffs (the staged hunk now appears under "staged").

## 5. Error handling
- `git apply` failure (e.g. the hunk no longer applies because the file changed underneath)
  → the command returns the git error; the UI shows it in an error banner and refreshes so
  the user sees current state. No partial corruption (`git apply` is atomic per invocation).
- Empty/last hunk, binary files (no hunks) → `parse_diff` yields zero hunks; the UI shows
  "no hunks / binary" and offers only file-level staging.
- Per-hunk on an untracked file: untracked files have no `git diff` until added; the UI
  offers file-level "stage" only for untracked (consistent with Phase 3).

## 6. Testing
- **`agency-core` (TDD):** `parse_diff`/`parse_file_diff` on real `git diff` output (one and
  multiple hunks, header captured); `stage_hunk` against a temp repo with a 2-hunk change —
  stage hunk 0, assert `git diff --cached` contains hunk 0 and `git diff` still contains hunk
  1; `unstage_hunk` reverses it. Round-trip: stage all hunks one-by-one == `git add` whole
  file.
- **`agency-app`:** `git_parse_diff`/`git_stage_hunk`/`git_unstage_hunk`/`project_log`
  thin-command coverage via a run + temp repo.
- **Frontend:** build under TS-strict; `DiffView` hunk parsing is backend-driven so logic is
  thin; visual fidelity vs `Agency v2.dc.html` is a human check.

## 7. Key decisions
| Decision | Choice |
|---|---|
| Per-hunk staging | Real: reconstruct patch + `git apply --cached [--reverse]` |
| Hunk addressing | Index into the freshly-recomputed diff (no stale offsets) |
| Diff view | Unified only (no split); hunk granularity (no per-line) |
| History pills | Per managed project (`project_log`), switch which log shows |
| Git Review panel | Compact side panel in Agents view, scoped to focused run |
| GitPanel | Replaced by SourceControl + GitReviewPanel |
