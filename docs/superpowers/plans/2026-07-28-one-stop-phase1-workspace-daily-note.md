# One-Stop Phase 1 — Workspace Root + Daily Note Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans style — implement task-by-task, failing-test-first where practical. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Journaling, planning, and personal notes get a home. A pinned "Workspace" project (default `~/Agency`) with relaxed git requirements: git offered and default-on at creation, never required. The whole folder is the docs vault. `⌘⇧D` opens today's `journal/YYYY-MM-DD.md` (created from `templates/daily.md` when present), with prev/next-day navigation and a palette entry.

**Architecture (locked in `docs/one-stop-plan.md`):** the workspace is a **project row** with a new nullable `projects.kind` column (`NULL` = normal, `"workspace"` = the pinned one). Every per-project subsystem keys off a project id, so the workspace rides through runs/files/docs/issues untouched. Branching happens in exactly three places: the add flow, readiness gating, and the docs root.

**Tech stack:** Rust (`agency-core`, `agency-app`/Tauri, rusqlite), TypeScript + React 18 (Vite), Vitest, cargo test.

## Global constraints

- UI package manager is **pnpm**; run tests via direct `node_modules/.bin` binaries if `pnpm exec` misbehaves (pnpm 11 gotcha).
- SQLite changes are **additive migrations** guarded by `column_exists(...)`. Never rewrite `CREATE TABLE`.
- New API-facing Rust structs crossing to TS use `#[serde(rename_all = "camelCase")]`. (`Project` stays snake_case — it already crosses as snake_case and the UI types match.)
- **No emoji in the UI** — monochrome text glyphs only (workspace glyph: ◈).
- All frontend file access goes through `FileRoot` + `resolve_within`. No new path-taking commands that bypass the jail. (`create_workspace` / `move_workspace` take absolute paths like the existing `add_project`/`inspect_repo` — they are project-management, not file access.)
- No new network calls.
- **No new issue-model fields** (discipline for Phase 5).

## File structure

**Backend:**
- `crates/agency-core/src/registry.rs` — `Project.kind`, migration, `ensure_workspace`, `get_workspace`, backfill exclusion.
- `crates/agency-core/src/files.rs` — tests for empty-rel-path (`resolve_within(root, "")`, `read_markdown_corpus(root, "")`).
- `crates/agency-app/src/state.rs` — `get_workspace`, `create_workspace`, `move_workspace`, `default_workspace_location`, workspace-aware `detect_docs_dir` support (`project_kind`).
- `crates/agency-app/src/commands.rs` — `get_workspace`, `create_workspace`, `move_workspace`, `default_workspace_location` commands; `detect_docs_dir` returns `""` for the workspace.
- `crates/agency-app/src/lib.rs` — register new commands.
- `crates/agency-app/src/menu.rs` — File ▸ "Today's Note" (`CmdOrCtrl+Shift+D`, action `daily-note`).

**Frontend:**
- `ui/src/api.ts` — `Project.kind`, `getWorkspace`, `createWorkspace`, `moveWorkspace`, `defaultWorkspaceLocation`.
- `ui/src/lib/dailyNote.ts` (new) + `dailyNote.test.ts` (new) — pure date/path logic.
- `ui/src/components/WorkspaceCreateDialog.tsx` (new) — location + git toggle (default on).
- `ui/src/components/ProjectTree.tsx` — pinned workspace entry above the list, distinct glyph, excluded from the active filter.
- `ui/src/components/AgentsView.tsx` — hide Source Control + agent spawn for a git-less workspace.
- `ui/src/components/DocsView.tsx` / `ui/src/components/DocsTree.tsx` — root-as-vault (`docsDir === ""`), journal newest-first, open-note event.
- `ui/src/components/DocsEditor.tsx` — prev/next-day navigation in the header for journal notes.
- `ui/src/components/CommandPalette.tsx` — "Today's Note" entry.
- `ui/src/components/Settings.tsx` — "Workspace" section (location, choose/move).
- `ui/src/App.tsx` — `daily-note` menu/palette/shortcut routing, open-daily-note flow.
- `ui/src/hooks/useShortcuts.ts` — `⌘⇧D`.

---

### Task 1: registry — `projects.kind` + workspace row helpers

- [x] Failing tests in `registry.rs::tests`: kind round-trips (default None), `ensure_workspace` creates once and is idempotent (revives a closed row), workspace gets **no issue key** and `backfill_issue_keys` skips it.
- [x] `Project.kind: Option<String>`; `ALTER TABLE projects ADD COLUMN kind TEXT` behind `column_exists`; thread through INSERT/SELECTs/`row_to_project`.
- [x] `ensure_workspace(name, path)` — returns the existing workspace row if present (un-closing it), else inserts with `kind='workspace'`, a palette color, `issue_key = NULL`.
- [x] `get_workspace()` — the workspace row regardless of closed state.
- [x] `backfill_issue_keys` gains `AND (kind IS NULL OR kind <> 'workspace')` (workspace joins the tracker in Phase 5).
- [x] `set_project_repo_path(id, path)` for move.
- [x] `cargo test -p agency-core` green.

### Task 2: files — empty-rel-path is the root

- [x] Tests in `files.rs`: `resolve_within(root, "")` resolves to the root itself; `read_markdown_corpus(root, "")` reads markdown at the root and in subfolders while still skipping hidden dirs; `list_dir(root, "")` lists the root.
- [x] Fix anything the tests surface (expected: none — `root.join("")` is `root`).

### Task 3: app state + commands + menu

- [x] `state.rs`: `get_workspace()`, `create_workspace(path, use_git)` (create_dir_all → optional `init_repo` + `initial_commit` → `ensure_workspace`), `move_workspace(new_path)` (`fs::rename`, then update row; bail if destination exists), `default_workspace_location()` (`$HOME/Agency`), `project_kind(id)`.
- [x] `commands.rs`: expose the four commands; `detect_docs_dir` returns `Some("")` when the project is the workspace (whole folder = vault).
- [x] `lib.rs`: register commands.
- [x] `menu.rs`: File ▸ "Today's Note", `CmdOrCtrl+Shift+D`, id `menu:daily-note`, always enabled.
- [x] Workspace-lifecycle test in `crates/agency-app/tests/` — create without git (no repo, project listed, docs root ""), create with git (Ready readiness), idempotent second create.

### Task 4: api + pinned tree entry + creation dialog

- [x] `api.ts`: `kind` on `Project`; `getWorkspace`, `createWorkspace`, `moveWorkspace`, `defaultWorkspaceLocation`.
- [x] `WorkspaceCreateDialog.tsx`: shows location (default from backend, "Choose…" via dialog plugin), "Keep history with git" toggle default **on**, creates and hands the project back.
- [x] `ProjectTree.tsx`: partition `kind === "workspace"` out of the normal list; pinned row above the list with ◈ glyph; click selects (or opens the create dialog when absent); excluded from the active filter; no close button (closing the workspace is not a flow); listens for `agency:create-workspace` events (carrying an optional `intent`) and re-emits `agency:workspace-ready` after creation.
- [x] `vitest` + `tsc --noEmit` green.

### Task 5: readiness gating + root-as-vault docs

- [x] `AgentsView.tsx`: track the selected project's readiness; when `project.kind === "workspace"` and it's not a git repo, hide the Source Control tab button and the agent-spawn menu (terminals stay — they run in the checkout, no worktree needed); a workspace on the source/agents tab with git works untouched (agents dispatchable = headline feature).
- [x] `useDocs.ts`/`DocsView.tsx`: `docsDir === ""` works end to end (falsy-string guards!); workspace never shows the "create docs/" empty state.
- [x] `DocsTree.tsx`: notes inside the top-level `journal/` folder sort newest-first (descending name = descending date); everything else unchanged.

### Task 6: daily note

- [x] `lib/dailyNote.ts` (+ tests): `dailyNotePath(date)` → `journal/YYYY-MM-DD.md`; `isDailyNotePath`; `parseDailyDate`; `adjacentDailyPath(paths, current, dir)` → nearest existing prev/next journal note; `renderDailyTemplate(tpl, date)` (`{{date}}` substitution); `defaultDailyContent(date)`.
- [x] `App.tsx` `openDailyNote()`: workspace absent → dispatch `agency:create-workspace` with `intent: "daily-note"` (resumes via `agency:workspace-ready`); present → ensure `journal/`, create today's note if missing (from `templates/daily.md` when present), stamp `docs:last:<wsId>`, select workspace + docs tab, dispatch `agency:open-note`.
- [x] `DocsView.tsx`: listen for `agency:open-note` (guard on project id) → refresh + select.
- [x] `useShortcuts.ts`: `⌘⇧D` → `onDailyNote`. Route menu action `daily-note` in `App.onMenu`.
- [x] `DocsEditor.tsx`: for journal notes, header gains ‹ / › to the nearest existing prev/next daily note (disabled at the ends).

### Task 7: palette entry + Settings location

- [x] `CommandPalette.tsx`: global "Today's Note" action → same daily-note flow (window event `agency:daily-note`; App listens).
- [x] `Settings.tsx`: "Workspace" section — shows the location (or the default + "created on first use"); "Move…" for an existing workspace via `moveWorkspace` (errors surfaced); "Choose…" pre-creation stores nothing — creation dialog owns the choice pre-creation, so pre-creation the section just explains the default.
- [x] *(added during ship gate)* "Show the workspace" toggle in Settings ▸ Workspace for people who don't want it: hides the pinned ◈ row, the palette entry, and turns ⌘⇧D into a hint toast (`lib/workspacePref.ts`, localStorage-backed). Hiding a created workspace also closes its project row (sessions stop, records/files kept); re-enabling revives it via the idempotent `create_workspace`.

### Task 8: verification + ship gate

- [x] `cargo test` green; `tsc --noEmit` exit 0; `vite build` exit 0; `vitest run` green.
- [x] Manual ship gate (dev app): create workspace → decline git → Docs/Files work, Source/agent-spawn hidden. Recreate with git → dispatch an agent on a writing prompt → merge. `⌘⇧D` on a fresh day creates from template. *Passed 2026-07-28.*

**Non-goals (resist):** cross-project search, frontmatter, task aggregation, issue-model fields.
