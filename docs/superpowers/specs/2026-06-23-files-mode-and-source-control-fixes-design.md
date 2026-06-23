# Design: Files mode, resizable source-control panes, and terminal newline fix

Date: 2026-06-23
Status: Approved (pending implementation plan)

## Overview

Three changes to the `agency` app (Tauri + Rust backend in `crates/agency-app`,
React + Vite frontend in `ui/`):

1. **Bug fix** — opening an agent injects extra blank lines into its terminal.
2. **Enhancement** — make the Source Control tab's panes resizable.
3. **Feature** — a new top-level **Files** mode: a file/folder tree plus an
   editable file viewer, keyed to the focused agent's worktree.

The three are independent and can land in any order. They share this spec
because they were requested together; the implementation plan may split them
into separate commits/PRs.

---

## 1. Terminal newline bug

### Symptom
Opening an agent (focusing it so `FocusTerminal` mounts) shows extra blank
lines in the terminal that the user did not type.

### Root cause (hypothesis — to be confirmed under systematic-debugging)
`FocusTerminal` (`ui/src/components/FocusTerminal.tsx`) writes two overlapping
sources for the same on-screen content when it mounts:

- `stream.preview(runId, 200)` calls the `run_preview` command, which runs
  `tmux capture-pane -p -S -200` (`crates/agency-core/src/tmux.rs:123`). That
  capture includes the **current visible screen** at its tail, not just
  scrollback. The frontend then **force-appends a newline**:
  `term.write(seed.endsWith("\n") ? seed : seed + "\n")` (`FocusTerminal.tsx:57`).
- `stream.attach(runId, …)` spawns `tmux attach-session`
  (`crates/agency-core/src/tmux.rs:155`). On attach, tmux **redraws the full
  current screen** to the new client.

Result: the visible screen is painted twice (once from the capture seed, once
from the attach redraw), and the forced trailing `\n` adds blank space.

### Fix approach
Develop the fix under the **systematic-debugging** skill: reproduce the extra
lines against a live agent, confirm the exact cause, then apply the minimal
fix. The expected change is small and confined to `FocusTerminal.tsx`:

- Remove the unconditional `+ "\n"` appended to the preview seed.
- Reconcile the seed-vs-attach-redraw overlap so the visible screen is not
  painted twice (e.g. only seed the scrollback that precedes the redraw, or
  let the attach redraw own the screen).

The exact mechanism is finalized only after the repro confirms the cause; the
backend tmux behavior is not expected to change.

### Acceptance
Focusing an agent shows its existing terminal content once, with no extra
blank lines beyond what the agent actually produced. Scrollback context
remains visible.

---

## 2. Resizable source-control panes

### Current state
`GitPanel` (`ui/src/components/git/GitPanel.tsx`) `full` layout splits into:

- `.git-full-left` — fixed `width: 360px; min-width: 300px`
  (`ui/src/styles.css:747`)
- `.git-full-right` — `flex: 1` (`ui/src/styles.css:748`)

There is no resizer between them, so the left/right split and the diff pane
width are fixed. (The inner Changes/History horizontal split inside
`GitSections` already has a working `Resizer` and is unchanged by this work.)

### Fix approach
Add a **vertical `Resizer`** between `.git-full-left` and `.git-full-right`,
reusing existing primitives:

- `ui/src/components/Resizer.tsx` (`orientation="vertical"`, the default).
- `ui/src/hooks/usePaneWidth.ts` for width state + `localStorage` persistence,
  with a dedicated pane key (e.g. `git-full-left`), sensible `min`/`max`
  (min ≈ 300 to match current `min-width`).

CSS change: `.git-full-left` width becomes driven by the hook's `style.width`
rather than the hard-coded `360px`; `.git-full-right` stays `flex: 1` to absorb
remaining space.

### Acceptance
The user can drag the boundary between the file/history list and the diff
viewer in the full Source Control view; the chosen width persists across app
restarts (localStorage), matching the existing History-pane behavior.

---

## 3. Files mode

### Placement and keying
- A **third top-level mode** alongside Agents and Source Control. The `Tab`
  type in `ui/src/store/runs.tsx` (currently `"agents" | "source"`) gains
  `"files"`. The tab buttons live in `ui/src/components/AgentsView.tsx`.
- Keyed to the **focused agent's worktree**: the mode reads `focusedRunId`
  from the run store and passes it as `task_id` to the backend. The backend
  resolves the directory via the existing `state.worktree_path(task_id)`
  (the same resolution Source Control uses).
- **Empty state:** when `focusedRunId` is null, the Files mode shows a prompt
  to select/focus an agent rather than an empty tree.
- Switching the focused agent re-roots the tree to that agent's worktree.

### Tree contents
Show **everything** under the worktree root, unfiltered (including
`node_modules`, `.git`, build artifacts). Because the tree is unfiltered and
can be enormous, the tree **must expand lazily** — one directory level is
fetched on demand when a folder is expanded. The whole tree is never walked
eagerly.

### Backend (new Tauri commands)
Added to `crates/agency-app/src/commands.rs` and registered in the
`generate_handler!` list in `crates/agency-app/src/lib.rs`. All resolve the
root via `state.worktree_path(&task_id)` and **validate that the resolved path
stays inside the worktree** (reject any `..` traversal or absolute escape).

- `list_dir(task_id, rel_path) -> Vec<DirEntry>` where
  `DirEntry { name: String, is_dir: bool }`. Lists a single directory level
  (the worktree root when `rel_path` is empty). Entries sorted
  directories-first, then by name.
- `read_file(task_id, rel_path) -> FileContents` where `FileContents` carries
  the text plus flags for binary / too-large content so the UI can show a
  placeholder instead of rendering garbage. (Exact size threshold and binary
  detection chosen during implementation.)
- `write_file(task_id, rel_path, contents) -> ()`. Writes the buffer back to
  disk within the worktree.

Corresponding typed wrappers are added to `ui/src/api.ts`.

### Frontend
A new `FilesView` component (mode container) composed of:

- **`FileTree`** (left pane): renders lazily-expanding nodes from `list_dir`.
  Tracks expanded folders and the selected file. Re-roots when `focusedRunId`
  changes.
- **Editor pane** (right): a **CodeMirror 6** editor instance. On file select,
  loads contents via `read_file` into an editable buffer with syntax
  highlighting (language inferred from file extension; the existing
  `ui/src/components/git/highlight.ts` `langForPath` helper can inform the
  mapping). Tracks a **dirty** state; **Cmd+S** (and a visible save affordance)
  calls `write_file` and clears dirty. Binary/too-large files show a
  read-only placeholder instead of an editor.
- A **vertical `Resizer`** between tree and editor, same `Resizer` +
  `usePaneWidth` pattern as item 2, with its own pane key.

### Dependency
Adds **CodeMirror 6** to `ui/package.json` as the editor component (approved).
Chosen over Monaco (too heavy for Tauri) and a textarea+Shiki overlay (fragile
scroll-sync / line-number / perf issues for real editing).

### Out of scope (YAGNI)
- File create / rename / delete / move from the tree.
- Multi-file tabs or split editors (one file open at a time).
- External-change conflict resolution beyond a basic reload — if the file
  changes on disk while open, behavior is defined during implementation but a
  full merge/conflict UI is out of scope.
- `.gitignore` filtering (explicitly chose unfiltered).
- Search across files.

### Acceptance
- A Files tab appears next to Agents and Source Control.
- With an agent focused, its worktree tree renders; folders expand lazily;
  `node_modules` does not freeze the UI on load.
- Clicking a text file opens it in an editable, syntax-highlighted editor;
  edits can be saved with Cmd+S and persist to the worktree on disk.
- Binary/oversized files show a placeholder, not garbage.
- The tree/editor split is resizable and the width persists.
- Path traversal outside the worktree is rejected by the backend.

---

## Affected files (summary)

Backend:
- `crates/agency-app/src/commands.rs` — new `list_dir`, `read_file`,
  `write_file` commands.
- `crates/agency-app/src/lib.rs` — register the new commands.

Frontend:
- `ui/src/components/FocusTerminal.tsx` — newline-bug fix.
- `ui/src/components/git/GitPanel.tsx` + `ui/src/styles.css` — left/right
  resizer.
- `ui/src/store/runs.tsx` — extend `Tab` with `"files"`.
- `ui/src/components/AgentsView.tsx` — Files tab button + routing.
- `ui/src/components/FilesView.tsx` (new), `ui/src/components/FileTree.tsx`
  (new), editor component (new).
- `ui/src/api.ts` — wrappers for the new commands.
- `ui/package.json` — add CodeMirror 6.

## Testing
- Rust: unit tests for the new commands, especially path-traversal rejection
  and lazy single-level listing, following the existing `crates/agency-*/tests`
  patterns.
- Frontend: tests for pure helpers (e.g. tree node sorting / path joining) in
  line with existing Vitest usage; the newline fix is verified against a live
  repro per systematic-debugging.
