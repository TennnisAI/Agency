# Agency Phase 7c — Command Palette, Shortcuts, Modal & Settings Polish (Design)

**Date:** 2026-06-20
**Status:** Approved design, pre-planning

## 1. Summary

Phase 7c is the final Phase-7 slice: it makes the app chrome's promises real and polishes
the remaining surfaces to the design handoff. A **⌘K command palette** (fuzzy over projects
+ agent runs), **keyboard shortcuts** (⌘N / ⌘G / ⌘↵ / ⌘K), the **New-task** and **Merge**
modals' full visual treatment, a **Settings reskin**, and **sidebar/rail collapse** polish.
All frontend; no backend changes.

Builds on 7a (multi-agent shell + tmux runs) and 7b (source control). After 7c, the full
mockup is realized.

## 2. Goals / Non-goals

### Goals
- **⌘K command palette:** an overlay opened by ⌘K (and clicking the title-bar search box).
  Fuzzy-filters the user's projects and their agent runs; arrow keys + Enter select; picking
  a project selects it, picking a run focuses it (switch to Agents/Focus). Esc closes.
- **Keyboard shortcuts (global):** ⌘N opens New task; ⌘G switches the Agents view to the
  Source Control tab; ⌘↵ triggers Approve→ (merge) on the focused run; ⌘K opens the palette.
- **New-task modal polish:** present the New-task form as the designed centered modal
  (mockup) with project (current), base branch, agent profile, and a **live preview** of the
  worktree path (`<repo>/.agency/worktrees/<id-preview>`) and `agent/<id>` branch that will
  be created.
- **Merge modal polish:** wrap the existing MergeModal flow in the designed framing — a
  checkout → rebase → resolve → summary **step timeline** around the resolver terminal, with
  the Discard / Keep worktree / Confirm-merge footer styling. (Behavior unchanged from
  Phase 5; this is presentation + the timeline state indicator.)
- **Settings reskin:** restyle the Settings overlay to the design system (sectioned layout,
  tokenized inputs, the back/close affordance), keeping all existing fields/behavior.
- **Collapse polish:** the sidebar toggle (exists) and the focus agent-rail collapse (exists)
  reflow neighbors smoothly to match the mockup's slim-rail behavior; ensure the title-bar
  toggle and the «/» rail control are consistent and the content reflows to full width.

### Non-goals (deferred / out of scope)
- Searching repo file contents in ⌘K (projects + runs only, per the 7c brainstorming).
- Real fuzzy-ranking libraries — a simple case-insensitive substring + light scoring is
  enough; no new dependency.
- Multi-key chord shortcuts; shortcut customization UI.
- Any backend/Rust change (7c is frontend-only).
- Animated transitions beyond simple CSS (no animation library).

## 3. Architecture (all frontend, `ui/src`)

### 3.1 Keyboard + palette
- **`store/runs.tsx` (modify):** add UI-intent signals the shortcuts/palette drive —
  `openNewTask: boolean` + `setOpenNewTask`, and reuse existing `setSelectedProject`,
  `setFocusedRun`, `setView`. (Keeps a single source of truth the shell already consumes.)
- **`hooks/useShortcuts.ts` (new):** a hook that registers a `keydown` listener on `window`
  for ⌘/Ctrl + N/G/K/Enter, calling injected callbacks; ignores events when the target is an
  input/textarea EXCEPT ⌘K and ⌘Enter (which should work everywhere). Cleans up on unmount.
- **`components/CommandPalette.tsx` (new):** overlay; an input + a filtered list of entries
  (`{kind:'project'|'run', id, label, sublabel}`) built from projects (`listProjects`) and,
  for each project, its runs (`listRuns`) — or just the selected project's runs to stay cheap
  (decision: include all projects' runs is heavier; **scope: the loaded projects + the
  selected project's runs**, plus every project as a jump target). Keyboard: ↑/↓ move
  selection, Enter activates, Esc closes. Activating a project → `setSelectedProject(id)`;
  a run → select its project + `setFocusedRun(id)` + `setView('focus')`.

### 3.2 Wiring (`App.tsx` / `TitleBar.tsx`)
- `App` mounts `useShortcuts({ onNewTask, onSource, onApprove, onPalette })` and renders
  `<CommandPalette>` when open. `onNewTask` → `setOpenNewTask(true)`; `onSource` → a signal
  AgentsView reads to switch its tab to source (add `sourceTabSignal`/a callback via the
  store, or lift the Agents tab into the store — see 3.4); `onApprove` → open the merge modal
  for the focused run (a store flag `approveSignal`); `onPalette` → open the palette.
- `TitleBar` search box becomes a button that opens the palette (`onOpenPalette`).

### 3.3 Modals & settings
- **`NewTaskForm` (modify):** add a base-branch input (default "HEAD") and a live preview line
  showing the computed worktree path + `agent/<preview-id>` branch (preview id is a short
  placeholder like `<new>` since the real id is server-assigned). Present it inside the
  designed modal shell.
- **`MergeModal` (modify):** add a step-timeline header (Checkout → Rebase → Resolve →
  Summary) whose active step reflects the modal's current phase (initial merge attempt =
  Checkout/Rebase; resolving = Resolve; done = Summary). Restyle the footer. No flow change.
- **`Settings` (modify):** reskin to the design tokens/sectioned layout; keep all fields.

### 3.4 Agents tab in the store (small refactor)
To let ⌘G and the palette drive the Agents view's tab/focus, lift the Agents-view `tab`
('agents'|'source') and the `review`/`approve` intents into `store/runs.tsx` (currently local
to `AgentsView`). `AgentsView` reads `tab/setTab` from the store. This keeps shortcuts and the
palette as thin dispatchers into one store, avoiding prop-drilling or ad-hoc event buses.

## 4. Data flow (examples)
- **⌘K → jump to a run:** ⌘K opens palette → type "fix" → matches run "claude: fix the bug"
  → Enter → store sets selectedProject + focusedRun + view='focus' → shell renders that run's
  Focus terminal.
- **⌘N:** sets `openNewTask=true` → AgentsView shows the New-task modal.
- **⌘G:** store `tab='source'` → AgentsView shows Source Control for the focused run.
- **⌘↵:** store `approveSignal` for the focused run → AgentsView/AgentFocus opens MergeModal.

## 5. Error handling
- Palette list fetches (`listProjects`/`listRuns`) wrapped in try/catch → empty list on error
  (the palette still opens; no crash).
- Shortcuts are inert when there's no focused run (⌘↵) or no project (⌘G/⌘N just no-op or
  open with the empty state) — guarded, never throw.
- All modals keep their existing error banners.

## 6. Testing
- **Pure helpers (vitest):** the palette's fuzzy filter/scoring function (`filterEntries(query,
  entries) -> entries`) gets unit tests (case-insensitive substring, ordering). This is the
  one piece of real logic; the rest is wiring + presentation.
- **Build under TS-strict;** component behavior (keyboard nav, modal timeline) and visual
  fidelity vs `Agency v2.dc.html` are human checks.
- No backend tests (no backend change); `cargo test --workspace` must stay green (unchanged).

## 7. Key decisions
| Decision | Choice |
|---|---|
| ⌘K scope | Projects + runs (no repo file-content search) |
| Fuzzy match | Simple case-insensitive substring + light scoring, no new dep |
| Shortcuts | ⌘N new · ⌘G source · ⌘↵ approve · ⌘K palette |
| New-task | Designed modal + live worktree/branch preview |
| Merge modal | Step timeline (Checkout→Rebase→Resolve→Summary), behavior unchanged |
| Settings | Reskin to tokens, fields unchanged |
| State | Lift Agents tab + new-task/approve intents into store/runs.tsx |
| Backend | None — 7c is frontend-only |
