# One-Stop Phase 6 — Tracker Depth

> **For agentic workers:** Use superpowers:executing-plans style — implement task-by-task, failing-test-first where practical. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** the cross-project board becomes the primary working view; the issue
model gains just enough structure — `due`/`scheduled` dates, manual `rank`
ordering, index-backed text search. This is the first phase designed
file-first on top of Phase 5: the new fields are frontmatter keys before they
are columns.

**Frontmatter after this phase (new keys optional, omitted when unset):**

```markdown
---
key: AGE-14
status: in_progress
priority: 2
due: 2026-08-01
scheduled: 2026-07-30
rank: 1.5
created: 2026-07-27T09:30:00Z
updated: 2026-07-27T14:02:00Z
---
# Fix terminal resize on reattach
```

## Global constraints

- pnpm, direct `node_modules/.bin` binaries when pnpm exec misbehaves.
- Verification gate: `cargo test`, `tsc --noEmit`, `vite build`, `vitest run`.
- SQLite changes are **additive migrations** guarded by `column_exists(...)`.
- No network calls. No emoji — new glyphs are monochrome text (◷ for dates).
- Frontend file access stays inside the `resolve_within` jail (this phase adds
  no new file IO paths at all).

## Decisions pinned here

- **Dates are civil dates, stored as `YYYY-MM-DD` strings** — frontmatter
  value, `TEXT` index columns, `Option<String>` on `Issue`, `string | null` in
  TS. An issue is due on a *day*, not an instant; date strings compare
  lexicographically, sort in SQL and TS alike, and dodge every timezone trap
  (overdue is computed in the UI against the user's local today via the
  existing `dateStamp` in `dailyNote.ts`). Parse strictly: shape *and*
  calendar validity (`2026-02-30` rejects the file, same ethos as Phase 5 —
  strict frontmatter keeps agents honest).
- **`rank` is an `Option<f64>`, ascending, global to the issue.** Lower rank
  sorts higher within its status group. Must be finite (NaN/inf rejects the
  file). Not scoped per status: an issue keeps its rank across status moves,
  which is what you want when dragging something back to todo.
- **Canonical key order** grows to: `key`, `status`, `priority`, `due`,
  `scheduled`, `rank`, `created`, `updated` — new keys sit with the other
  mutable planning fields, before the timestamps. Phase 5's parser already
  round-trips these as unknown keys, so files written by this build survive an
  older build's write (that forward-compat was the point of `extra`).
  Consequence of the keys becoming known: a hand-written `due: whenever`
  that Phase 5 carried as an opaque extra line now **rejects the file**
  (row kept, per the corruption rule) — strictness is the feature.
- **`IssuePatch` learns clear-vs-untouched** via double-`Option`:
  `due`/`scheduled`: `Option<Option<String>>`, `rank`: `Option<Option<f64>>`,
  with the standard `#[serde(default, deserialize_with = "double_option")]`
  pattern (absent = untouched, `null` = clear). TS side: `due?: string | null`
  etc. State-level `update_issue` validates dates (shape + calendar) and rank
  (finite) so the API can't write a file the parser would reject.
- **`Registry::update_issue` is removed.** It's the pre-Phase-5 mutation path,
  now reachable only from its own unit test, and COALESCE can't express
  "clear to null" — extending it would create a half-supporting trap. The
  real patch application lives in `state::update_issue`; the registry test
  moves to `upsert_issue_row`. `Registry::create_issue` stays (index-layer
  helper used across tests) and inserts NULL for the new columns.
- **No new Tauri commands, no api.ts signature changes.** The cross-project
  board already fans out `list_issues` per project (HomeView does today);
  bodies ride along in the index rows. Board text search and the palette `@`
  provider therefore filter **client-side over index-backed rows** — the
  master plan's aside about routing `@` through `search_files` over
  `.agency/issues/` buys nothing over data the palette already fetches, and
  is deviated from here, documented. (`/` content search still hits issue
  files naturally once you're in a project, since they're tracked markdown.)
- **`compareIssues` becomes rank-first within a status**: rank ascending,
  ranked before unranked, then priority descending, then `createdAt`/`seq` as
  today. Group order (status) is unchanged.
- **Drag-to-reorder lives in the per-project IssuesView only.** The
  cross-project board mixes projects inside a status group; reordering there
  has no stable meaning. Mechanics: pointer-based drag (mousedown → 5px
  threshold → `elementFromPoint` tracking → commit on mouseup) — **not HTML5
  DnD**, because Tauri's native drag-drop layer intercepts drops at the
  NSView level on macOS and the page's `drop` event never fires — plus a
  pure planner in `ui/src/lib/issueRank.ts` —
  `planReorder(group, fromIdx, toIdx) -> {id, rank}[]`:
  - If the group isn't fully ranked yet, **materialize** ranks 1..n in the
    current visual order (first drag pays the batch of updates).
  - Otherwise the dragged issue gets the midpoint of its new neighbors'
    ranks; when the gap collapses (< 1e-6), **renormalize** the whole group
    back to 1..n. That's the "fractional ranks, periodic renormalize" from
    the master plan, in one testable function returning the minimal update
    set.
- **Today's definition** (master plan, verbatim semantics): open issues with
  `due ≤ today` or `scheduled ≤ today`, plus everything `in_progress`,
  across all projects. Sorted overdue-first (due ascending), then the normal
  compare. Pure selector `todayIssues(...)` + `isOverdue(...)` in
  `lib/issues.ts`, unit-tested with a fixed `today` string — no `Date.now()`
  in the selector.
- **The board is a new `HomeIssues.tsx`**, extracted from HomeView's issues
  mode (HomeView keeps the agents overview + blank state and delegates).
  It gets: the Today section on top, a filter bar (text search + status +
  priority + project single-select dropdowns + a sort selector: Board order /
  Due date / Recently updated), and **real `IssueRow` rows** — the same
  component as the project board, so inline status/priority menus, delete,
  and the AgentAddMenu dispatch caret all work without entering the project.
  Below Today, issues stay grouped per project (existing fold state and
  "busiest first" order); filters and sort apply within groups.
- **Dispatch from the board** reuses the AgentsView flow shape in HomeIssues:
  `agentInstalled` check → `InstallAgentDialog` if missing; `inspectRepo` →
  `RepoSetupDialog(context: "spawn")` if not ready; else `startIssueRun` and
  jump straight into focus via the existing `onOpenRun(project, runId)`.
  Default-agent plain ▶ uses `pickDefaultAgent`, same as IssuesView.
- **Dates in the UI:** `IssueDetail` gains Due and Scheduled rows (native
  `<input type="date">` styled to match, clearable — its value string is
  already the storage format). `IssueRow` shows a due chip (`◷ Aug 1`) when
  set, `overdue` styling when past and the issue is open. Scheduled shows
  only in the detail pane — it's a planning field, the row stays quiet.
- **Palette `@` search deepens** via a new optional `haystack` field on
  `PaletteEntry`: `filterEntries` matches it as a tier between sublabel and
  fuzzy. Issue entries put body + due date in the haystack, so `@` finds
  issues by content without polluting the visible label/sublabel.

## File structure

**Backend:**
- `crates/agency-core/src/issuefs.rs` — date validation, `due`/`scheduled`/
  `rank` on `IssueFile`, KNOWN_KEYS + parse/serialize + tests.
- `crates/agency-core/src/registry.rs` — guarded `due TEXT`, `scheduled TEXT`,
  `rank REAL` columns; `Issue` fields; `row_to_issue`/selects/upsert; remove
  `update_issue`; `IssuePatch` new fields + `double_option`.
- `crates/agency-app/src/state.rs` — `update_issue` patch application +
  validation; `write_issue_file`/`create_issue` carry the new fields.
- `crates/agency-app/tests/` — state-level date/rank patch coverage.

**Frontend:**
- `ui/src/api.ts` — `Issue`/`IssuePatch` types.
- `ui/src/lib/issues.ts` — `compareIssues` rank-first, `todayIssues`,
  `isOverdue`, `fmtDue`; new `ui/src/lib/issues.test.ts`.
- `ui/src/lib/issueRank.ts` (+ test) — reorder planner.
- `ui/src/lib/paletteFilter.ts` (+ test) — `haystack` tier.
- `ui/src/components/IssueRow.tsx` — due chip, drag handle props.
- `ui/src/components/IssueDetail.tsx` — date editors.
- `ui/src/components/IssuesView.tsx` — DnD wiring → `planReorder` → batched
  `updateIssue` calls.
- `ui/src/components/HomeIssues.tsx` (new) + `HomeView.tsx` (delegates).
- `ui/src/components/CommandPalette.tsx` — issue haystack.
- `ui/src/styles.css` — chips, filter bar, drag indicator.

---

### Task 1: issuefs — dates + rank in the file format

- [x] Failing tests: `parse_date` accepts `2026-08-01`, rejects `2026-02-30`,
      `26-8-1`, `2026/08/01`, empty; full-file parse round-trips due/
      scheduled/rank; canonical serialization emits the pinned key order and
      omits unset keys; a Phase-5-shaped file (no new keys) parses to `None`s
      and serializes byte-identically to before; rejections: bad date, `rank:
      abc`, `rank: NaN`, duplicates; trailing comments on new keys tolerated.
- [x] Implement: `IssueFile.{due, scheduled, rank}`, KNOWN_KEYS → 8, parse
      arms (date shape + `days_in_month` validity; `f64` finite), serialize in
      canonical order.

### Task 2: registry — columns, row mapping, patch type

- [x] Guarded migrations: `issues.due TEXT`, `issues.scheduled TEXT`,
      `issues.rank REAL`.
- [x] Failing tests: `upsert_issue_row` stores and returns the new fields
      (set and cleared); `list_issues`/`get_issue` surface them; reconcile
      carries a file's due/rank into the row (issuefs test).
- [x] Implement: `Issue.{due, scheduled, rank}` (`Option<String>`,
      `Option<String>`, `Option<f64>`), all issue selects + `row_to_issue` +
      `upsert_issue_row`; `create_issue` inserts NULLs.
- [x] `IssuePatch.{due, scheduled, rank}` as double-`Option` with the
      `double_option` deserializer (absent ≠ null); unit test the serde
      behavior (missing field vs explicit null).
- [x] Remove `Registry::update_issue`; rewrite its test against
      `upsert_issue_row`.

### Task 3: state — file-first writes carry the new fields

- [x] Failing state-level tests: `update_issue` sets due/scheduled/rank
      (file shows the keys, row matches), clears them with explicit null
      (keys vanish from the file), rejects malformed date and non-finite
      rank; reconcile round-trip: external edit adding `due:` shows up in
      `list_issues` next poll.
- [x] Implement: extend `state::update_issue` patch application +
      validation; `write_issue_file` and `create_issue` populate the new
      `IssueFile` fields from the row (new issues: all `None`).

### Task 4: TS model — types, compare, today

- [x] `api.ts`: `Issue.{due, scheduled, rank}`, `IssuePatch` optionals
      (`string | null`, `number | null`).
- [x] Failing tests (`ui/src/lib/issues.test.ts`): `compareIssues` rank
      ascending first, ranked before unranked, then priority/created as
      before; `isOverdue` (open + past due only); `todayIssues` picks
      due/scheduled ≤ today + in_progress, excludes closed, sorts
      overdue-first; `fmtDue` renders `Aug 1` / with year when not current.
- [x] Implement in `lib/issues.ts` (selectors take `today: string` — no
      clock reads).

### Task 5: reorder planner + DnD in IssuesView

- [x] Failing tests (`ui/src/lib/issueRank.test.ts`): unranked group →
      materialize 1..n with the dragged item in place; ranked group → single
      midpoint update; collapsed gap → full renormalize; no-op drop → empty
      plan; drop at head/tail.
- [x] Implement `planReorder`.
- [x] Wire IssuesView: rows draggable within their status group (HTML5 DnD,
      drop indicator via CSS class), drop → `planReorder` → `Promise.all`
      of `updateIssue({rank})` → `refresh()`. Cross-group drag is ignored
      (status changes stay on the pill).

### Task 6: dates in IssueRow + IssueDetail

- [x] IssueDetail: Due / Scheduled date inputs (clearable, commit on change,
      `patch({due: v || null})`).
- [x] IssueRow: `◷` due chip with `fmtDue`, `overdue` class via `isOverdue`;
      title tooltip carries the full date.
- [x] Styles: chip, overdue color (theme red/orange var), date input skin.

### Task 7: HomeIssues — the cross-project board

- [x] Extract HomeView's issues mode into `HomeIssues.tsx` (same poll,
      fold state, project ordering); HomeView delegates.
- [x] Today section on top from `todayIssues` across projects, rows labeled
      with project key.
- [x] Filter bar: text search (title+body), status / priority / project
      dropdowns, sort selector (Board order / Due date / Recently updated);
      filters compose; applied within project groups.
- [x] Real `IssueRow`s: inline status/priority/delete via `updateIssue`/
      `deleteIssue` + poll refresh; ▶ and AgentAddMenu dispatch through a
      local spawn flow (`agentInstalled` → InstallAgentDialog; `inspectRepo`
      → RepoSetupDialog context "spawn"; `startIssueRun` → `onOpenRun`).
- [x] Empty states: no Today section when empty; "no issues match" under
      active filters.

### Task 8: palette haystack

- [x] Failing test: `filterEntries` matches `haystack` after sublabel,
      before fuzzy; entries without haystack unaffected.
- [x] Implement `haystack?` in `PaletteEntry` + CommandPalette issue entries
      (body + due), so `@` finds issues by body text.

### Task 9: verification + ship gate

- [x] `cargo test`, `tsc --noEmit`, `vite build`, `vitest run` all green.
- [x] Ship gate (dev app, ≥3 projects): filter by status/priority/project and
      text on the home board; drag-reorder within a status in a project and
      watch `rank:` appear in the file; set a due date in the detail pane —
      chip appears, file gains `due:`, issue shows in Today once due; dispatch
      an issue from the home board without entering the project and land in
      focus; `@` in the palette finds an issue by a body-only word.
- [x] Update `docs/one-stop-plan.md` status lines (Phase 5 shipped, Phase 6).

**Non-goals (resist):** labels, cross-domain links and issue-corpus wikilinks
(P7), reordering on the cross-project board, drag-between-status-groups,
per-status rank scoping, recurring/relative dates, notifications, an MCP
layer, any new Tauri command or api.ts signature change.
