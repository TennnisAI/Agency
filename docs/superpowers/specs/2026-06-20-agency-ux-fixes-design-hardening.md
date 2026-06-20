# Agency — UX Fixes + Design Hardening

_Spec · 2026-06-20_

## Context

Agency is a local multi-agent terminal orchestrator (Tauri + Rust core + React/TS UI).
The functional shell exists but diverges from the design handoff
(`docs/design-handoff/Agency v2.dc.html` + `Agency-Design-Handoff.html`) and has several
UX gaps. This spec captures the fixes and a polish pass to close the gap with the design.

Persistence decision: **keep the existing global SQLite registry** (`~/.config/agency/agency.db`)
as the source of truth. We are **not** introducing a per-project `.agency/agency.json` save file
at this time (the `.agency/worktrees/<id>/` layout is unchanged).

## Goals

1. Projects pane: `+` button in the header; remove the bottom add-project form.
2. Adding an agent spawns a terminal instantly (no prompt modal).
3. Closing a project stops its agents; reopening restores them re-runnable.
4. Ability to stop and to discard an individual agent, with a destructive confirm.
5. Drag-to-resize the sidebar, focus rail, source-control pane, and review panel.
6. Polish pass to match the design handoff (tokens, fonts, dimensions, states).

Non-goals: per-project save file, project rename UI, multi-machine portability, theme swapping.

## Design

### 1. Projects pane header

Replace `<h2 class="tree-head">Projects</h2>` and the bottom `.add-project` form
(`ui/src/components/ProjectTree.tsx`) with a header row:

- `PROJECTS` eyebrow label — 10.5px / 600 / letter-spacing .13em / `#6c7086`.
- A 22×22 `+` button on the right — `bg #1e1e2e`, `border 1px #313244`, radius 6,
  hover `bg #313244`.

The `+` button opens the native folder picker (`@tauri-apps/plugin-dialog`) directly.
Project name defaults to the chosen folder's basename, then calls `add_project(name, path)`.
The inline name/path/Browse inputs are removed.

Tree styling matches the spec: chevron (▾/▸), 24×24 colored initial icon, name + dim mono
path, per-project status dots, nested agent rows, selected row with inset accent bar.

### 2. Add agent = instant spawn

Remove the `NewTaskForm` prompt modal. The Agents header gets a `+ Agent ▾` menu listing
the three agent types. Selecting one creates a worktree off current `HEAD` and spawns the
agent immediately; the user interacts via the live terminal.

Backend:
- Seed three agent profiles: Claude Code (`claude`, peach `#fab387`), Pi (`pi`, teal
  `#94e2d5`), Hermes (`hermes`, mauve `#cba6f7`). Profiles run interactively — drop the
  required `{{prompt}}` arg so the bare command runs.
- `create_run` accepts an empty prompt (worktree + tmux session created as today; the
  prompt is no longer required to launch).

### 3. Project lifecycle: close vs. remove

Split today's single `remove_project` (which orphans tmux sessions — the reported bug):

- **`close_project(id)`** — kills every tmux session for the project's runs; **keeps** the
  project and run records in the DB. Invoked by the project row `X` with a confirm.
  On reopen, runs report tmux status `Gone` and the UI shows "Exited — re-run"; the existing
  `rerun(id)` command re-spawns.
- **`delete_project(id)`** — destructive: kills sessions, removes worktrees, deletes the
  project and its runs from the DB. Separate affordance with a red confirm.

The agents pane already filters runs by `selectedProjectId`; we keep that so other/closed
projects' agents never linger in the view.

### 4. Stop / discard an agent

Surface existing backend capability in the UI on the agent tile, focus header, and rail row:

- **■ Stop** — kills the tmux session, keeps the run (becomes "Exited — re-run").
  Backend: add `stop_run(id)` (kill session, keep record) if not already separable from
  `discard_run`.
- **Discard agent** — destructive: `discard_run(id)` (stop + remove worktree + delete
  record). Requires a red confirm warning.

### 5. Persistence

No change. Global SQLite remains authoritative. Close/reopen behavior derives from DB records
plus tmux liveness (`run_status` → `Gone` after a session is killed).

### 6. Resizable panes

Add drag splitters between: sidebar/main, focus rail/terminal, source-control left/right,
and the review panel/content. Each splitter updates a CSS variable live on drag, clamps to a
min/max, and persists the chosen width to `localStorage`. Defaults match the spec dimensions
(sidebar 266, rail 312, sc-left 336, review 360).

### 7. Design hardening

Styling pass against `Agency v2`:

- Full Catppuccin Mocha token set as CSS variables (crust/mantle/base/line/s0/s1/s2,
  overlays, subtexts, text, and accents blue/lav/mauve/pink/green/teal/yellow/peach/red).
- Bundle Geist (UI) + JetBrains Mono (code) locally (offline, not CDN).
- Fixed dimensions: title bar 40, status bar 27, sidebar 266, focus rail 312 (→38 collapsed),
  source-control left 336, review panel 360, grid tile min 340 / row 262.
- Semantic status states: running (pulsing green dot + glow), awaiting (yellow dot + tile ring),
  idle/review (blue), exited (gray dot, tile dimmed ~.82), crashed (red dot + red border).
- Type-colored agent badges (peach/teal/mauve), JetBrains Mono, radius 5.
- xterm terminal theme matched to the palette.
- Status bar: branch · project · task counts (running/awaiting/idle/stopped) · spacer ·
  shortcuts · version.
- Diff add/del/hunk styling with per-hunk stage; segmented controls; button variants
  (primary blue, secondary, ghost, destructive red); modal styling (incl. merge resolver).

## Delivery phases

1. **Backend** — `close_project` / `delete_project`, `stop_run` vs `discard_run`,
   instant-spawn `create_run`, seeded profiles. Rust tests.
2. **Frontend behavior** — header `+`, `+ Agent ▾` menu, stop/discard/close/remove controls
   with confirms, pane resizers.
3. **Design hardening** — the §7 polish pass.

## Testing

- Rust unit/integration tests:
  - `close_project` kills sessions and keeps project + run records.
  - `delete_project` removes worktrees and DB records.
  - `create_run` succeeds with an empty prompt.
  - seeded profiles present with expected commands/colors.
- Frontend vitest for store + resizer logic (selection filtering, width clamping/persistence).
- Manual run-through verifying: add project via `+`, instant agent spawn, stop/discard with
  confirm, close project then reopen and re-run, drag-resize each pane.

## Risks

- Killing tmux sessions on close must not delete worktrees (needed for reopen/re-run).
- `discard_run` already removes worktrees; ensure `stop_run` and `close_project` do **not**.
- Resizer persistence must clamp to avoid unusable pane widths.
