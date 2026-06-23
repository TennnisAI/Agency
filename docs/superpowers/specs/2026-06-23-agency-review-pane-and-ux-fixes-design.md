# Agency — Review Pane Stacking + UX Fixes — Design

Date: 2026-06-23

## Overview

Five related UI/pane fixes for the agency app. The largest is reworking the
git review pane so Changes and History are visible together (no tab switch),
VSCode-style. The rest are smaller UX fixes plus one cross-cutting backend
feature (LLM-summarized agent titles).

Work items:

1. Git review pane — stacked Changes-over-History with a movable divider and
   foldable History (replaces the Changes/History tab toggle).
2. Projects pane — keep the `+` button visible at every sidebar width.
3. Focus-view agents rail — add a `+` button to create an agent.
4. Focus-view agents rail — show a vertical "AGENTS · N" spine when folded.
5. Agent naming — give each run a mutable, LLM-summarized title derived from
   its first prompt.

Items 1–4 are frontend/CSS only. Item 5 touches Rust core, Tauri commands, and
the UI.

---

## 1. Git review pane — stacked sections with movable divider

### Current state

`ui/src/components/git/GitPanel.tsx` is one component with two separate `return`
blocks selected by a `layout: "compact" | "full"` prop:

- `full` — the dedicated review board (left column = tabs + Changes/History +
  `ReviewComments`; right column = `DiffViewer`/`CommitDetail`).
- `compact` — a 360px sidebar beside the agent terminal (vertical stack: tabs →
  panel → `ReviewComments` → diff pinned at the bottom).

Both branches duplicate a `tab` toggle (`"changes" | "history"`) and its
`git-tabs` buttons. The shared pieces today are only the data fetching, the
`sel` selection union, `refresh`/`act`, and the two panel elements.

### Design

Eliminate the tab toggle. Extract a new `GitSections` component that stacks the
two panels vertically with a draggable horizontal divider and a collapsible
History section. Both layouts render `<GitSections>`, removing the duplicated
tab logic.

**New component: `ui/src/components/git/GitSections.tsx`**

Props:

```ts
{
  changesPanel: React.ReactNode;  // built by GitPanel (already is)
  historyPanel: React.ReactNode;  // built by GitPanel (already is)
}
```

Structure:

- **Changes section** — a header label ("Changes") + the `changesPanel`. Takes
  the remaining flex space (`flex: 1`, scrollable).
- **Horizontal divider** — `<Resizer orientation="horizontal" ...>`. Active only
  while History is expanded.
- **History section** — a VSCode-style header with a chevron (▾/▸) that toggles
  fold. When expanded it has a persisted pixel height (scrollable
  `historyPanel` inside). When collapsed only the header bar shows and Changes
  fills the freed space.

**Resizer generalization (`ui/src/components/Resizer.tsx`)**

Add an `orientation?: "vertical" | "horizontal"` prop (default `"vertical"`,
preserving all current call sites). Vertical reads `clientX`→width (today's
behavior); horizontal reads `clientY`→height and sets `cursor: row-resize`.
The drag math mirrors the existing pointer handler.

**State & persistence**

- History height: reuse `usePaneWidth` (it is just "a persisted, clamped pixel
  number keyed by a string" — dimension-agnostic). Key `git-history-h`, default
  ~220px, min ~120px, max derived from a sensible cap.
- Fold state: persisted boolean in `localStorage` under `pane:git-history-folded`.
  Default **expanded** (both panels visible — the whole point of the change).
- Both keys are **shared across the two layouts** (one user preference). A small
  helper (e.g. `loadFold`/`saveFold`) wraps read/write so it is unit-testable,
  mirroring `loadWidth`/`saveWidth`.

**GitPanel changes**

- Drop `tab`/`setTab` state and the `git-tabs` buttons from both branches.
- Keep `changesPanel` and `historyPanel` element construction.
- `full` left column: render `<GitSections changesPanel={...} historyPanel={...} />`
  above `ReviewComments`. Right column unchanged (diff/commit detail driven by
  the existing single `sel` union — selecting a file or a commit both work).
- `compact`: render `<GitSections .../>` above `ReviewComments`, with the diff
  still pinned at the bottom (`git-compact-diff`).

`ReviewComments` stays exactly where it is today (below the stack) in both
layouts.

**CSS (`ui/src/styles.css`)**

- `.resizer.horizontal { height: 5px; width: auto; cursor: row-resize; }`
  (existing `.resizer` keeps `width: 5px; cursor: col-resize`).
- Section wrappers and a foldable History header styled to match the existing
  `.git-tabs`/section look (header bar with chevron, hover affordance).
- Remove now-unused `.git-tabs` rules only if no other component references them.

### Testing

- Add a unit test for the fold-state helper (`loadFold`/`saveFold`) — load
  default, round-trip, and bad/missing values — mirroring the existing
  `usePaneWidth.test.ts`.
- History-height clamping is already covered by `clampWidth` tests.
- Drag interaction mirrors the existing (untested) `Resizer`; no brittle
  pointer-event tests added.

---

## 2. Projects pane — `+` button always visible

### Bug

In `ui/src/styles.css`, `.tree` has `min-width: var(--sidebar-w)` plus
`overflow: hidden`. The wrapper in `App.tsx` sets the actual width from the
`sidebar` `usePaneWidth` (drag floor 200px). When the wrapper is narrower than
`--sidebar-w`, the tree keeps its larger min width and clips its own right
edge — where the right-aligned `+` (`.icon-add`) in `.tree-head` lives — so the
button vanishes.

### Fix (CSS only)

- `.tree { min-width: 0; }` — let the tree follow its wrapper width; the 200px
  drag floor in `App.tsx`/`usePaneWidth` remains the real minimum.
- `.tree-head .eyebrow { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }`
  so the "PROJECTS" label truncates instead of pushing the button off-screen.
- `.icon-add { flex-shrink: 0; }` so the button keeps its size and stays pinned
  to the right edge at every width.

No JS changes.

---

## 3. Focus-view agents rail — `+` button

### Current state

`ui/src/components/AgentFocus.tsx` renders the foldable `rail` (the agents list
in Focus view). Its head shows `Agents` + a `«` collapse button. The only way
to add an agent today is the `+ Agent ▾` dropdown in the main `AgentsView`
header (`AGENT_TYPES` → `createAgent`).

### Design

Add a `+` button to the rail head, opening the same agent-type menu.

Because the agent-type dropdown now lives in two places (the `AgentsView`
header and the rail head), extract a shared component:

**New component: `ui/src/components/AgentAddMenu.tsx`**

Encapsulates the open/close state, the `AGENT_TYPES` list, and the menu markup.
Props let the caller choose the trigger style and supply the spawn handler:

```ts
{
  onSpawn: (agentId: string) => void;   // AgentsView passes its spawn(); rail passes createAgent
  variant?: "button" | "icon";          // "button" = "+ Agent ▾"; "icon" = compact "+" for the rail
}
```

- `AgentsView` replaces its inline menu with `<AgentAddMenu variant="button" onSpawn={spawn} />`,
  preserving the repo-readiness `spawn()` flow.
- `AgentFocus` rail head renders `<AgentAddMenu variant="icon" onSpawn={createAgent} />`
  next to the `«` button.

(The rail uses `createAgent` directly, matching the existing keyboard-shortcut
path; repo-readiness gating already happens for the primary add flow in
`AgentsView`. Keeping the rail's add as a direct `createAgent` is acceptable
because an agent can only be focused within an already-set-up project.)

### Testing

Light — this is composition of existing behavior. No new unit tests unless the
menu extraction exposes testable logic.

---

## 4. Folded agents rail — vertical "AGENTS · N" spine

### Current state

When `railOpen` is false, `AgentFocus` renders a bare `rail-stub` button with
`»`.

### Design

Per the design handoff (`docs/design-handoff/Agency v2.dc.html`, ~line 340),
the folded rail is a 38px-wide spine:

- The `»` expand button on top.
- Below it, vertical text **"AGENTS · {runs.length}"** using
  `writing-mode: vertical-rl` with the handoff's letter-spacing/weight/color.

Update the `else` branch in `AgentFocus.tsx` to render this structure (button +
vertical label) and add `.rail-stub` styles to `ui/src/styles.css` matching the
handoff (mantle background, right border, centered column, padding-top).

### Testing

None (presentational).

---

## 5. Agent naming — mutable, LLM-summarized title

### Constraint

`prompt` is immutable and foundational: `new_task_id(prompt)` derives the task
id, which drives the branch name and tmux sessions. It cannot be repurposed as
a display name. The "+ Agent" flow currently creates runs with `prompt = ""`,
so rows fall back to showing the branch.

### Design

Add a separate, mutable `title` that is generated from the run's first prompt
and shown in place of the branch once available. The id/branch/sessions are
untouched.

**Schema (`crates/agency-core/src/registry.rs`)**

- Add a nullable `title TEXT` column to the `runs` table via an additive
  migration (`ALTER TABLE runs ADD COLUMN title TEXT` guarded so existing DBs
  upgrade in place; new DBs include it in `CREATE TABLE`).
- Extend the run struct and all `SELECT`/`INSERT` mappings to carry `title:
  Option<String>`.
- Add a registry method to update a run's title by id.

**Tauri command (`crates/agency-app/src/commands.rs` + `state.rs`)**

- `RunInfo` DTO gains `title: Option<String>` (serialized camelCase → TS
  `title: string | null`).
- New command `set_run_title(id: String, first_prompt: String)`:
  1. No-op if the run already has a non-empty `title`.
  2. Asynchronously produce a 3–5 word title from `first_prompt` using the
     **already-configured provider** in `ProviderSettings` (Anthropic API key,
     else LM Studio `lmStudioBaseUrl`). Prompt the model for a terse,
     filename-like task title.
  3. On success, persist via the registry update method.
  4. On no configured provider or any failure, fall back to the trimmed,
     truncated first line of `first_prompt` (e.g. first ~6 words / 48 chars).
- The async work must not block the command's return; the title appears on the
  next runs poll. (Spawn onto the Tauri async runtime; persist when done.)

**Capture (UI — `ui/src/components/FocusTerminal.tsx`)**

Tee the existing input stream. Today: `onData = term.onData((d) => stream.input(runId, d))`.

- Add a per-run "first prompt not yet captured" guard, active only while the
  focused run's `title` is empty.
- Buffer printable input, handling backspace (`\x7f`/`\b`) and paste chunks,
  until the first carriage return (`\r`). Trim; ignore if empty/whitespace or
  control-only.
- On the first non-trivial submitted line, call `set_run_title(runId, line)`
  once, then disable the guard for that run.
- This is **best-effort**: agent TUIs, arrow-key editing, and multi-line input
  can make the captured line imperfect. The LLM summary smooths it; the
  fallback keeps it working offline. A wrong capture only yields a slightly-off
  title — id/branch are unaffected.

**API (`ui/src/api.ts`)**

- `RunInfo` gains `title: string | null`.
- `export const setRunTitle = (id, firstPrompt) => invoke<void>("set_run_title", { id, firstPrompt });`

**Display order**

Everywhere a run name is shown, use `title || prompt || branch`:

- `ProjectTree.tsx` — `tree-child-name`
- `AgentFocus.tsx` — `rail-name`
- `AgentTile.tsx` and `AgentFocus` focus head — wherever the name/branch shows.

(Introduce a tiny `runName(run)` helper in a shared place — e.g. `agents.ts` —
so the precedence lives in one spot.)

### Testing

- Rust: registry test for adding/reading `title` and the update method;
  migration upgrades an existing DB.
- TS: unit test for the input-buffer/first-line extraction logic (backspace,
  paste, empty/whitespace, control-only) — factor the buffering into a pure
  helper so it is testable without a real terminal.
- The LLM call itself is not unit-tested (network/provider dependent); the
  fallback path is covered by the helper test.

---

## Build / verification

- Rust: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build` and `cargo test`.
- UI: `cd ui && npx tsc --noEmit && npx vitest run`.

## Out of scope

- Renaming an agent's branch/task id (immutable by design).
- Manual title editing (could be added later; not required here).
- Per-layout independent fold/height preferences for the git pane (shared by
  decision).
