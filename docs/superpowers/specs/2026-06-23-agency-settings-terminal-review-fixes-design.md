# Agency — Settings, Review-Pane, Terminal & UX Fixes — Design

Date: 2026-06-23

## Overview

Six UI/UX fixes for the agency app. Four are small (CSS / one-line layout
moves); two are substantive — reworking how the compact Review pane shows diffs,
and adding a standalone shell terminal as a session type in the agents rail.

Work items:

1. Settings — fix notifications layout overflow and replace the raw browser
   checkboxes with a sleek custom toggle.
2. Review pane — stop rendering diffs inline in the cramped 360px compact pane;
   make it a navigator that jumps the selection to the full-width Source Control
   tab. Fixes the crash, the horizontal cut-off, and the broken History collapse.
3. Disable text selection on chrome (buttons/headers/labels), keep it on
   diffs/terminals/inputs/code.
4. Move the agents-rail `+` button to sit directly after the "Agents" label.
5. Make all panes drag-resizeable (the compact Review pane is the main gap).
6. Standalone terminal — a `kind: "terminal"` session in the agents rail that
   runs the user's `$SHELL` in the project repo root, reusing the existing
   tmux/PTY + streaming + FocusTerminal pipeline.

Items 1–5 are frontend-only. Item 6 touches Rust core, Tauri commands, state,
and the UI.

---

## 1. Settings — notifications layout + sleek toggles

### Bug

In `ui/src/components/Settings.tsx` the `NOTIFICATIONS` section maps over toggle
rows as `<label className="settings-notif-row">` containing a raw
`<input type="checkbox">` + label text. In `styles.css`,
`.settings-notif-row { display: flex; align-items: center; gap: 8px; }` has no
width constraint, so the long label text overflows past the right edge of the
620px modal (visible in the screenshot, labels clipped off-screen). The raw
checkbox renders as the unstyled red browser control.

### Design (CSS only + tiny markup tweak)

- `.settings-notif-row`: `display: flex; align-items: center; gap: 10px;
  justify-content: space-between;` and let the label text take remaining width
  (`flex: 1`, normal wrapping). For the toggle rows the control sits on the left
  and the descriptive text fills the rest — restructure the row so the text is
  in its own `<span className="settings-notif-label">` (so layout is explicit
  rather than relying on raw text node flex behavior).
- Replace the checkbox with a **custom toggle switch**: keep the real
  `<input type="checkbox">` for accessibility/state but visually hide it
  (`.toggle input { position:absolute; opacity:0; }`) and render a styled
  `<span className="toggle-track"><span className="toggle-thumb"/></span>`.
  Track is a rounded pill; off = `--s1` background, on = `--green` (or `--blue`);
  thumb is a circle that slides via `transform: translateX(...)` with a 0.15s
  transition. Wrap each row's control in `<span className="toggle">`.
- The "Idle after (seconds)" row keeps its number input but uses the same
  `space-between` row layout so it lines up with the toggle rows.
- Apply the same `.toggle` treatment to the existing `setup-gitignore` checkbox
  for visual consistency.

A small presentational `Toggle` component (`ui/src/components/Toggle.tsx`) wraps
the hidden-input + track/thumb markup so it is reused by both Settings and the
gitignore checkbox.

### Testing

None (presentational). Verified manually.

---

## 2. Review pane — navigator that jumps to full Source Control

### Bug

`ui/src/components/git/GitPanel.tsx` `layout="compact"` is a fixed 360px
`<aside>`. When a commit is selected it renders a full `<CommitDetail>` (which
has its own 240px file-list column + side-by-side `DiffViewer`) crammed into
360px → the diff gets ~120px, the side-by-side grid (`44px 44px 1fr 1fr`) is
crushed with no horizontal scroll, the stacked Changes/History sections collapse
and overlap ("CHANGESHISTORY"), and History collapse breaks. Selecting a commit
also leaves no way to deselect.

### Design

The compact pane becomes a **navigator only**: Changes list + History list. It
no longer renders `DiffViewer`/`CommitDetail` inline. Selecting a file or commit
in compact mode:

1. Sets the shared selection, and
2. Switches the workspace `tab` to `"source"` (the existing full-width Source
   Control view, `GitPanel layout="full"`), which already has the correct
   two-column layout (navigator left, diff/commit-detail right).

**State lift.** The `Selection` currently lives inside each `GitPanel` instance,
so the compact and full panels have independent selections. Lift selection into
a shared owner so a selection made in compact mode is shown by the full panel:

- Add `selectedFocusSel` (the `Selection` union: file or commit) to the run
  store (`ui/src/store/runs.tsx`) alongside `tab`, OR pass it down from
  `AgentsView` as state shared between the compact `GitPanel` and the `source`
  tab's `GitPanel`. Chosen approach: keep it in `AgentsView` local state and
  pass `selection` + `onSelect` into both `GitPanel` instances; `GitPanel`
  becomes controlled for selection (props instead of internal `useState`).
- Compact `onSelect(sel)` → `setSelection(sel); setTab("source")`.
- Full `onSelect(sel)` → `setSelection(sel)` (no tab change; it's already full).

**GitPanel changes.**
- `compact` branch: render only `GitSections` (Changes + History) +
  `ReviewComments`. Remove the `git-compact-diff` block and its
  `DiffViewer`/`CommitDetail`.
- `full` branch: unchanged behavior, but selection is now driven by props.
- This deletes the crashing/cramped code path entirely.

**CommitDetail** stays as-is (still used by the full panel's right column).

### Testing

- Manual: expand a commit and a file from compact Review → confirm it jumps to
  Source Control with the right diff and no crash; confirm History fold works.
- If selection logic moves to a small pure helper, add a unit test; otherwise
  this is wiring verified manually.

---

## 3. Disable text selection on chrome

### Design (global CSS)

In `ui/src/styles.css`:

- Global default: set `user-select: none; -webkit-user-select: none;` on
  `.shell` (the app root) so it inherits to all chrome.
- Re-enable selection where it matters:
  `.diff-body, .diff-cell, .terminal, .xterm, input, textarea, code,
  .settings-meta-val, .git-commit-hash { user-select: text;
  -webkit-user-select: text; }`
- Verify the diff line-selection click behavior (`toggleLine`) is unaffected —
  it relies on click handlers, not text selection, so disabling selection on the
  gutter is fine; the `.diff-cell` keeps `user-select: text` so users can still
  copy code.

### Testing

Manual — confirm buttons/headers/labels no longer highlight, diffs/terminal/
inputs still allow copy.

---

## 4. Agents-rail `+` button placement

### Bug

`ui/src/components/AgentFocus.tsx` rail head order is
`<span>Agents</span> <span className="spacer"/> <AgentAddMenu/> <button>«</button>`,
so the spacer shoves `+` to the right next to the collapse button.

### Fix

Reorder to `<span>Agents</span> <AgentAddMenu/> <span className="spacer"/>
<button>«</button>` — `+` sits immediately after the label; `«` stays pinned
right. One-line move.

### Testing

None (presentational).

---

## 5. Resizeable panes

### Current state

`ui/src/components/Resizer.tsx` + `usePaneWidth` already power the sidebar
(`App.tsx`), the focus rail (`AgentFocus.tsx`), and the git History section
(`GitSections.tsx`). The compact Review pane (`git-panel.compact`, fixed 360px
in CSS) and any other fixed-width panes lack a handle.

### Design

- Compact Review pane: wrap in `usePaneWidth("review", 360, 280, 640)` and add a
  `<Resizer side="right" .../>` on its left edge (it docks on the right side of
  the workspace, so dragging its left border resizes it). Drive its width via
  inline style instead of the hard-coded `width: 360px` in CSS (keep a CSS
  `min-width`/fallback).
- Audit remaining panes (e.g. `git-full-left` fixed 360px) and add handles where
  a fixed width limits usability; persist each with its own `usePaneWidth` key.

### Testing

`usePaneWidth`/`clampWidth` already unit-tested. Manual drag check per pane.

---

## 6. Standalone terminal — `kind: "terminal"` session in the rail

### Constraint / current model

`create_run` (`crates/agency-app/src/state.rs`) always:
- creates a git worktree + `agent/<id>` branch (`WorktreeManager::create`),
- starts a tmux session running the agent profile's command in that worktree.

`RunInfo`/`registry::Run` carry branch + worktree assumptions, and review/merge/
title-gen all assume a worktree. A plain terminal must skip all of that and run
`$SHELL` in the project repo root.

### Design — add a `kind` discriminator

**Schema (`crates/agency-core/src/registry.rs`)**
- Add a `kind TEXT NOT NULL DEFAULT 'agent'` column to `runs` via an additive
  migration (`ALTER TABLE runs ADD COLUMN kind ...` guarded so existing DBs
  upgrade; new DBs include it in `CREATE TABLE`). Values: `"agent"` |
  `"terminal"`.
- Extend the run struct + `SELECT`/`INSERT` mappings to carry `kind`.

**State (`crates/agency-app/src/state.rs`)**
- New method `create_terminal(project_id) -> RunInfo` that shares a private
  session-start helper with `create_run` (rather than overloading `create_run`
  with a `kind` param, keeping the agent path untouched). For the terminal:
  - Skip `WorktreeManager::create`; no branch.
  - cwd = project **repo root** (`repo` path), not a worktree.
  - command = the user's login shell: `std::env::var("SHELL")` →
    fallback `"/bin/zsh"` (macOS) / `"/bin/bash"`. Run interactively
    (`-l`/`-i` as appropriate) so the prompt/profile loads.
  - Start the tmux session exactly like agents (`tmux.start_session`) so
    `attach/input/resize/preview/status` all work unchanged.
- `RunInfo` DTO gains `kind: String` (camelCase → TS).
- Guard worktree/git paths on kind:
  - `worktree_path(id)` for a terminal returns an error / is never called
    (Review/Source Control is not offered for terminals).
  - `discard_run` for a terminal: kill the tmux session + remove the registry
    row; **no worktree to remove** (gate the worktree-removal branch on
    `kind == "agent"`).
  - `archive_run`/`merge`/`set_run_title`/`rerun` are not offered for terminals
    (gated in the UI; backend stays safe if called by no-oping or erroring
    clearly).

**Tauri command (`crates/agency-app/src/commands.rs`)**
- `create_terminal(project_id: String) -> RunInfo` wrapping the state method.
- Register it in the Tauri builder's command list.

**API (`ui/src/api.ts`)**
- `RunInfo` gains `kind: "agent" | "terminal"`.
- `export const createTerminal = (projectId) =>
  invoke<RunInfo>("create_terminal", { projectId });`

**Store (`ui/src/store/runs.tsx`)**
- Add `createTerminal()` mirroring `createAgent` (create → refresh → focus →
  `setView("focus")`).

**Rail / focus UI**
- Rail `+` menu (`AgentAddMenu`): add a "New terminal" entry below the agent
  profiles that calls `createTerminal`.
- Rail row (`AgentFocus.tsx`): for `kind === "terminal"`, show a distinct
  label/icon (e.g. `≳ terminal` instead of `agent: name`) and a neutral dot.
- Focus header for a terminal: show **only Stop / Close** (Close = discard,
  relabeled). Hide branch `<code>`, the Agent/Run tabs, Approve, and Archive.
- `FocusTerminal` is reused verbatim (it only needs a run id + the agent stream).
  Title-capture (`onFirstPrompt`) is **disabled** for terminals.
- Source Control / Review is not offered for a focused terminal (it has no
  worktree); `AgentsView` gates the Review button + `source` tab on
  `focused.kind === "agent"`.

### Testing

- Rust: registry test for the `kind` column (default + round-trip); a
  `create_terminal` test asserting no worktree is created and the session runs
  in the repo root.
- TS: type-level wiring; manual run-through (create terminal, run a command,
  close it; confirm no worktree dir is created under `.agency/worktrees`).

---

## Build / verification

- Rust: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build && cargo test`.
- UI: `cd ui && npx tsc --noEmit && npx vitest run`.
- Manual: walk each of the six surfaces (settings toggles, review jump, no chrome
  selection, rail `+` position, drag-resize review pane, create/use/close a
  terminal).

## Out of scope

- Multiple terminal tabs / a VSCode-style bottom dock (chosen: rail entry).
- Manual title editing for agents.
- Terminals participating in review/merge (they have no worktree by design).
- Per-pane independent persistence beyond a `usePaneWidth` key each.
