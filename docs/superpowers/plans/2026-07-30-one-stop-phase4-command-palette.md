# One-Stop Phase 4 — Command Palette v2

> **For agentic workers:** Use superpowers:executing-plans style — implement task-by-task, failing-test-first where practical. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** the palette becomes the app's front door. Rebuild `CommandPalette.tsx` around a provider registry keyed by query prefix — *(none)* projects/runs/recents/top-commands, `>` commands, `#` tags → notes, `@` issues (cross-project), `/` file contents — with recency-weighted ranking and shared list navigation.

**Architecture (locked in `docs/one-stop-plan.md`):** providers contribute entries per query mode, sync from local state or async debounced from the backend; the `>` registry reuses the `onMenu` action ids in `App.tsx` so palette and native menu can't drift; `@` uses `list_issues` behind a provider interface that hides the Phase-5 storage swap; `/` uses `search_files` debounced 150 ms.

**Tech stack:** TS/React only — no Rust changes this phase. Vitest for the pure libs.

## Global constraints

- pnpm, direct `node_modules/.bin` binaries when pnpm exec misbehaves.
- Verification gate: `cargo test`, `tsc --noEmit`, `vite build`, `vitest run`.
- No network calls. No emoji — glyphs are text or stroked SVGs. **No issue-model fields** (P5 discipline).

## Decisions pinned here

- **Routing through Shell, not the store.** Today's palette calls `setSelectedProject` directly, which leaves Shell's `project` state stale (every other navigation path goes through Shell's `selectProject`/`openRun`). The rebuilt palette takes `onAction(actionId)`, `onOpenProject(p)`, `onOpenRun(p, runId)` props wired to Shell's existing handlers — fixing that latent bug and making `>` literally invoke the same `onMenu` switch the native menu uses.
- **Command registry** (`ui/src/lib/paletteCommands.ts`): static list `{ id, label, sublabel, when }` where `when` ∈ `always | project | focusedAgent | workspaceVisible`. Ids are the `menu:` action names minus `palette` (the palette is already open) and `quit` (routed backend-side via `lifecycle::request_quit`; it never reaches `onMenu`). Palette-only additions handled by new `onMenu` cases: `new-issue` (moves out of the palette's hardcoded action into the registry) and `go-agents`/`go-issues`/`go-docs`/`go-files` tab navigation. Anti-drift is enforced by tests that scan `menu.rs` for `menu:<id>` ids and `App.tsx` for `case "<id>"` — every registry id must be routed, every menu id (minus the two exclusions) must be in the registry.
- **Query modes** (`ui/src/lib/paletteQuery.ts`): first character selects the provider — `>` commands, `@` issues, `#` tags, `/` content; anything else is default mode. The prefix is stripped, remainder trimmed → `{ mode, term }`. Pure, tested.
- **Recency store** (`ui/src/lib/recency.ts`): `localStorage["palette:recency:v1"]`, an array of `{ key, label, sub, t }` capped at 100, newest first, deduped by key. Keys are namespaced: `cmd:<id>`, `project:<id>`, `run:<id>`, `issue:<id>`, `note:<projectId>:<path>`, `file:<projectId>:<path>`. Recording points: every palette activation, DocsView's user-driven `setSelected` (the restore path uses `setSelectedState` and stays silent), and FilesView's user-driven opens (FilesView gains a `projectId` prop so file keys can be reopened from the palette later; opens under a run root record under the owning project's id).
- **Default mode:** empty query = sections in order — top commands (Today's Note + New Issue as today, plus up to 3 recency-top commands), recent notes/files (up to 8 from the store; a recent whose project vanished is dropped at render), projects, then the selected project's runs; capped at 50 rows. Non-empty query = `filterEntries` over all default entries with input pre-sorted by recency, so within a match tier recent entries rank first. Scope-aware for free: `runs` is already the selected project's list.
- **`>` commands:** `availableCommands({ hasProject, hasFocusedAgent, workspaceVisible })` filters by context (mirroring the native menu's `set_context` gating); term filters via `filterEntries`. Activation calls `onAction(id)` — Shell's `onMenu` does the rest.
- **`@` issues:** on first entry into the mode, fan out `listIssues` over **all** projects (SQLite reads, small lists), cached for the palette's lifetime. Row label `KEY-seq title`, sublabel `project · status`. Ranking: selected project first, open statuses before done/cancelled, then match quality. Activation: stash `PENDING_ISSUE_KEY` (the existing HomeView→IssuesView handoff), `onOpenProject(project)`, `setTab("issues")` — IssuesView selects it once its list loads. This is the ship-gate "issue in a non-selected project" path.
- **`#` tags:** scope corpus = selected project's docs, else the workspace at home (no cross-project corpus reads — that's Phase 7 territory). On first entry: `detectDocsDir` + `readDocsCorpus` + `buildIndex`, cached for the palette's lifetime. Empty term → one row per tag (with note count); activating a tag rewrites the query to `#<tag>` instead of closing. Non-empty term → notes whose tags prefix-match, sublabel `#tag · path`. Activation: stamp `docs:last:<pid>`, `onOpenProject(scope)`, `setTab("docs")`, dispatch `agency:open-note` (the daily-note flow's exact recipe).
- **`/` content search:** root = focused run's worktree else selected project (AgentsView's `filesRoot` derivation), else the workspace at home; no root → a hint row. Term ≥ 2 chars, debounced 150 ms with a stale-response token (the `DocsTree` pattern), display capped at 50. Row label = trimmed line text, sublabel `path:line`. Activation: `onOpenProject` if the scope isn't selected, `setTab("files")`, `requestOpenFile({ path, line })` — FilesView's pending-open slot covers the mount race, `scrollToLine` lands it (ship gate).
- **Shared list navigation** (`ui/src/hooks/useListNav.ts`): the highlight/ArrowUp/ArrowDown/Enter logic currently copy-pasted across CommandPalette, QuickOpen, and DocsQuickSwitcher, extracted once; the two switchers are refactored onto it. Escape stays with `useModalKeys`.
- **Async-provider UX:** each async mode renders `loading…`/error rows like QuickOpen does; results replace them in place. Mode switches reset the highlight to 0. A dim one-line footer hints the prefixes when the query is empty (`>` commands · `@` issues · `#` tags · `/` content) — text glyphs, no emoji.

## File structure

**Frontend (no backend changes):**
- `ui/src/lib/paletteQuery.ts` (new, tested) — prefix → `{ mode, term }`.
- `ui/src/lib/paletteCommands.ts` (new, tested incl. menu.rs/App.tsx drift scans) — command registry + context filter.
- `ui/src/lib/recency.ts` (new, tested) — activation store.
- `ui/src/hooks/useListNav.ts` (new) — shared list keyboard navigation.
- `ui/src/components/CommandPalette.tsx` — rebuilt around providers.
- `ui/src/App.tsx` — new `onMenu` cases (`new-issue`, `go-*`); palette props (`onAction`, `onOpenProject`, `onOpenRun`).
- `ui/src/components/QuickOpen.tsx`, `DocsQuickSwitcher.tsx` — refactor onto `useListNav`.
- `ui/src/components/DocsView.tsx` — record note activations.
- `ui/src/components/FilesView.tsx` (+ `AgentsView.tsx` passing `projectId`) — record file activations.
- `ui/src/styles.css` — palette footer hint + section label styles.

---

### Task 1: query modes + command registry + Shell routing

- [x] Failing Vitest specs for `paletteQuery.ts`: `""`/plain → default; `>`/`@`/`#`/`/` prefixes strip and trim; lone prefix → empty term.
- [x] Implement `paletteQuery.ts`.
- [x] Failing specs for `paletteCommands.ts`: `when` filtering (no project hides project commands; no focused agent hides agent commands; hidden workspace hides Today's Note); drift scans — every `menu:` id in `crates/agency-app/src/menu.rs` except `palette`/`quit` is a registry id, and every registry id has a `case "<id>"` in `ui/src/App.tsx`.
- [x] Implement `paletteCommands.ts` (registry: settings, new-agent, new-terminal, daily-note, add-project, clone-project, source, toggle-sidebar, home, approve, archive, discard, report-issue, github, new-issue, go-agents, go-issues, go-docs, go-files).
- [x] `App.tsx`: add `onMenu` cases — `new-issue` (PENDING_QUICKADD_KEY + `setTab("issues")`, moved from the palette) and `go-*` (`setTab`); pass `onAction`/`onOpenProject`/`onOpenRun` to `CommandPalette`.

### Task 2: recency store + shared list nav

- [x] Failing specs for `recency.ts`: record dedupes by key and bumps to front; cap at 100 evicts oldest; `recentByPrefix("note:", n)`; storage errors are swallowed (quota/unavailable → no throw).
- [x] Implement `recency.ts`.
- [x] `useListNav.ts`: `{ hi, setHi, onKey }` over a row count + activate callback (ArrowUp/Down clamp, Enter activates, highlight resets when the count source changes).
- [x] Refactor `QuickOpen.tsx` and `DocsQuickSwitcher.tsx` onto `useListNav` — behavior identical.
- [x] Recording points: DocsView `setSelected`, FilesView user-driven opens (new `projectId` prop threaded from AgentsView).

### Task 3: palette rebuild — default mode + `>` commands

- [x] Rebuild `CommandPalette.tsx`: query state → `parsePaletteQuery`; unified `Row { key, glyph, label, sublabel, activate }`; `useListNav` + `useModalKeys`; every activation records recency and routes via the new props.
- [x] Default mode per the pinned decision (sections, recency pre-sort, 50-row cap); footer prefix hint on empty query.
- [x] `>` mode: `availableCommands` (context from `useRuns` + `workspaceHidden` + projects) filtered by term; activation → `onAction`.
- [x] Manual check: ⌘K → type `>` → every native-menu custom action listed (context permitting) and firing identically to the menu.

### Task 4: `@` issues provider

- [x] On mode entry: fan-out `listIssues` across `listProjects()` results (cache per palette open; per-project failures drop that project silently); loading/error rows.
- [x] Ranking + labels per the pinned decision; term filters over `KEY-seq` + title.
- [x] Activation: `PENDING_ISSUE_KEY` stash → `onOpenProject` → `setTab("issues")`.

### Task 5: `#` tags provider

- [x] On mode entry: scope = selected project else workspace; `detectDocsDir` + `readDocsCorpus` + `buildIndex` once per open; no docs → hint row.
- [x] Empty term → tag rows (`#tag`, note count) that rewrite the query; term → note rows via tag prefix match.
- [x] Activation: `docs:last` stamp → `onOpenProject` → `setTab("docs")` → `agency:open-note`.

### Task 6: `/` content provider

- [x] Debounced 150 ms `searchFiles(root, "", { query: term })` with stale-token guard; ≥ 2 chars; root per the pinned derivation; no-root and error hint rows.
- [x] Activation: select scope project if needed → `setTab("files")` → `requestOpenFile({ path, line })`.

### Task 7: verification + ship gate

- [x] `cargo test`, `tsc --noEmit`, `vite build`, `vitest run` all green.
- [x] Ship gate (dev app): every native-menu action reachable via `>`; `@` finds an issue in a non-selected project and jumps to it (detail pane opens on the right issue); `/` content search lands in the editor at the line; `#` opens a tagged note in Docs; recents appear after using them; ⌘P quick-open and the Docs switcher are untouched.
- [x] Update `docs/one-stop-plan.md` status lines (Phase 3 — shipped, and Phase 4 on completion).

**Non-goals (resist):** issue-model fields (dates/rank/labels — P5/P6), cross-project docs corpora (P7), palette access to PR/source-control actions beyond the existing menu set, fuzzy-highlighting match spans, provider plugins beyond the five modes, virtualized palette list.
