# One-Stop Phase 8 — Time Closes the Loop

> **For agentic workers:** Use superpowers:executing-plans style — implement
> task-by-task, failing-test-first where practical. Steps use checkbox
> (`- [x]`) syntax for tracking.

**Goal:** the app answers "what happened," not just "what now." Notes gain
YAML-style frontmatter (parsed, rendered as a property table, filterable in
docs search as `key:value`); `- [ ]` checkboxes across the workspace and all
projects aggregate into a Tasks section on Home where checking a box edits
the source file in place and a task can be promoted to an issue; a "Generate
weekly note" command assembles `journal/weekly/2026-W31.md` from local data
(merges, issues closed, runs archived) and can optionally dispatch a
workspace agent to narrate it.

**Ship gate (master plan, verbatim):** a week of normal use produces a weekly
note with real content; checking a task on Home edits the underlying
markdown; frontmatter renders and filters.

## Global constraints

- pnpm, direct `node_modules/.bin` binaries when pnpm exec misbehaves.
- Verification gate: `cargo test`, `tsc --noEmit`, `vite build`, `vitest run`.
- No network calls. No emoji; monochrome glyphs only. No em dashes in UI copy.
- SQLite changes additive via `column_exists` only (this phase needs **none**).
- Frontend file access stays inside `FileRoot` + `resolve_within`.
- New API-facing Rust structs use `#[serde(rename_all = "camelCase")]`.
- commands.rs async policy: read-only/polled commands async, mutating
  commands sync (main-thread serialization is the write lock).

## Decisions pinned here

- **Frontmatter parsing is frontend-only, flat, and forgiving.** No YAML dep
  exists in either half of the repo and none is added. A note's frontmatter
  is a `---` fence on line 1, `key: value` lines, closing `---`; values are
  raw strings (surrounding single/double quotes stripped); no nesting, no
  lists, no multi-line values. Anything malformed (no closing fence within
  100 lines, a line without `:`) means "this note has no frontmatter" — the
  text is body, never an error. This mirrors `issuefs.rs`'s line-parser
  philosophy but *forgiving* where issuefs is strict: notes are freeform,
  issue files are a format we own. Parsing lands in `parseDoc`
  (`docsIndex.ts:91`) as a preamble: `DocMeta` gains
  `frontmatter: [string, string][]` (ordered pairs, duplicates kept for
  display) and `fmEnd: number` (0 when absent = first body line), and the
  heading/tag/link scans **skip lines `< fmEnd`** — a `#` inside frontmatter
  must not become a tag or title. `title:` frontmatter does NOT override the
  H1 title (non-goal; keeps ranking and quick-switcher behavior stable).
- **Property-table rendering is line styling, not a block widget.** CM6 view
  plugins may not provide layout-changing block decorations, so
  `LivePreviewPlugin.build` gets a frontmatter pass ahead of the syntax-tree
  walk: when the doc starts with a frontmatter fence and the selection is
  outside it, fence lines collapse (inline replace of the `---` text; the
  opening fence renders a small "properties" label widget), each
  `key: value` line gets a `.lp-fm-line` line class plus `.lp-fm-key` /
  `.lp-fm-value` marks, and CSS draws the table skin. Selection inside the
  range reveals raw text (the existing `revealed` contract). The generic
  handlers must skip nodes that fall inside the frontmatter range —
  today the opening `---` renders as an HRWidget and the closing fence turns
  the keys into a SetextHeading2, which is the bug this fixes. Backgrounds
  ride `::before` at `z-index:-3` per the `drawSelection` note in
  `DocsEditor.tsx:201`.
- **`key:value` search grammar, new `ui/src/lib/docsQuery.ts`.**
  `parseDocsQuery(q)` → `{ filters: [key, value][], text: string }`: a token
  matching `/^[A-Za-z][A-Za-z0-9_-]*:\S*$/` (value quotable as
  `key:"two words"`) is a filter, everything else re-joins as free text.
  Filter match = case-insensitive substring over that key's frontmatter
  value; multiple filters AND. `searchLocal` applies filters first to build
  the candidate doc set: filters + no text → matching docs as title hits
  (line -1); filters + text → normal matching restricted to candidates.
  `DocsTree`'s backend call sends only the free-text remainder and
  `mergeBodyHits` drops hits whose path failed the filters (pass the
  candidate set in). Filter-only queries short-circuit the backend exactly
  like the `#tag` branch at `DocsTree.tsx:253`. `#tag` syntax unchanged.
  A bare `key:` (empty value) filters on "has this key."
- **Task scanning is a backend read command; toggling is a backend write
  command.** Scanning frontend-side would mean shipping every corpus body to
  Home on a poll; instead `crates/agency-core/src/files.rs` gains
  `scan_tasks(root, rel_dir) -> Vec<TaskHit>` reusing `walk_markdown` — the
  task universe is *exactly* the docs corpus (same skips, same caps), not
  the search walk. `TaskHit { path, line /*0-based*/, checked, text }`
  (camelCase). Fence-aware: task lines inside code fences are skipped
  (match the `docsIndex.ts` fence discipline). Toggling:
  `toggle_task(root, rel_dir, path, line, checked) -> bool` re-reads the
  file, verifies the addressed line still parses as a task with the
  *opposite* state, splices the 3-char `[ ]`/`[x]` marker, and writes via
  `issuefs::atomic_write`; returns `false` (no write) when the line moved
  or changed — the caller refreshes and re-renders. Tauri: `scan_tasks`
  async, `toggle_task` sync, both jailed through the same
  `resolve_root` + `resolve_within` path as the corpus commands.
- **Home task data comes from a stats-gated fan-out hook,**
  `useAllTasks(active)`, modeled on `useCrossRefs`: every 5s while active,
  `listProjects()` (workspace included — it is a project row), per project
  `detectDocsDir` then `docsCorpusStats`; a per-project mtime signature
  (the `useDocs.ts:51` trick) gates `scanTasks`, so an idle Home costs one
  readdir per project per tick and zero body reads. Per-project failures
  drop that project for the tick. Returns
  `{ groups: TaskGroup[], toggle, refresh }` where a `TaskGroup` is
  `{ project, docsDir, path, title, tasks: TaskHit[] }` (title = basename
  sans extension; note titles would need corpus bodies we deliberately
  don't have).
- **Tasks section lives in `HomeIssues`, below Today.** Phase 6 made the
  issues-mode Home "what should I do today"; checkboxes join that view, not
  the agents overview. Display: unchecked tasks only, grouped per note
  (project name + note title header, click header opens the note via
  `agency:navigate`), collapsible, total count in the section head, capped
  at 8 groups shown with a "N more notes" line (no silent truncation).
  Checking a box: optimistic strikethrough, `toggleTask`, then refresh;
  a `false` reply (stale line) skips the strike and refreshes. Checked
  tasks disappear on the next poll. `- [x]` hits are still collected
  (the scanner reports both) — Home just filters; the promote flow and
  future views get the full set.
- **Promote-to-issue writes provenance into the issue body, not the note.**
  Row action `▧ promote` → `createIssue(projectId, title = task text,
  body = "From [[<note base sans .md>]]", "todo")` → navigate to the issue.
  The source checkbox is left untouched: the `[[note]]` link makes the note
  appear in the issue's Mentions and the issue in the note's Mentions
  (Phase 7 machinery, zero new code), and checking the box off stays the
  user's call once the work is done. No frontmatter is added to the issue
  model — the P5 format is unchanged.
- **Weekly note assembly is frontend-pure over existing reads; the only
  backend change is exposing run timestamps.** `RunInfo` gains
  `createdAt: i64` and `archivedAt: Option<i64>` (both already on the
  registry `Run`, threaded through `state.rs:891 run_info` — additive,
  camelCase). Everything else reuses shipped commands:
  - *Merges:* `gitLogGraph("project:<id>", 300)` per project, client-filter
    `parents.length > 1 && date >= weekStart` (a new `--since` git helper is
    not worth it for a weekly window; 300 commits/week/project is generous
    headroom, and a project that exceeds it just truncates that section).
  - *Issues closed:* per-project `listIssues` (which triggers reconcile, so
    file edits are fresh), filter `status ∈ {done, cancelled} &&
    updatedAt >= weekStart`. Documented fuzziness per the master plan:
    `updated` is the only timestamp — a done issue edited later re-appears,
    one edited after an old close is missed. Accepted; no `closed:` field
    is added (P5 format discipline).
  - *Runs archived:* per-project `listArchivedRuns`, filter
    `archivedAt >= weekStart`.
- **Week math is pure TS in new `ui/src/lib/weeklyNote.ts`.** ISO weeks
  (Monday start): `isoWeekStamp(d) → "2026-W31"`, `weeklyNotePath(d) →
  "journal/weekly/2026-W31.md"`, `isoWeekStart(d) → Date` (Monday 00:00
  local; the report window is week start → generation moment).
  `buildWeeklyNote(stamp, sections) → string` renders markdown: an H1
  (`# Week 2026-W31`), per-project **Merged** lists (`subject` lines),
  **Issues closed** as `[[AGE-14]]` + title lines, **Agent runs archived**
  as `[[run:<id>|<title>]]` lines — Phase 7 makes every one of these
  resolve and backlink for free — then an empty `## Notes` section for
  narration. Empty data sections are omitted; an empty week still produces
  the header + Notes.
- **Command flow mirrors `openDailyNote` (`App.tsx:64`).**
  `generateWeeklyNote()`: workspace guard (create-workspace intent
  `"weekly-note"` reuses the daily-note dialog path) → if
  `journal/weekly/<stamp>.md` exists, open it and toast that it already
  exists (**never overwrite** — narration and hand edits live there) →
  else gather (per-project fetches with `.catch` dropping that project),
  build, `createDir("journal")` + `createDir("journal/weekly")`,
  `createFile` + `writeFile`, stamp `docs:last`, select workspace, docs
  tab, `agency:open-note`. Registered as palette command `weekly-note`
  (`when: "workspaceVisible"`), File-menu item `menu:weekly-note` beside
  Today's Note (always enabled, same rationale), `case "weekly-note"` in
  `App.tsx` `onMenu` — the paletteCommands drift tests enforce all three.
- **Narration is an offer, not a step.** After a *fresh* generation, if the
  workspace is a git repo (`gitBranchInfo("project:<ws.id>")` succeeds), a
  toast/action offers "Narrate with an agent"; accepting calls
  `createRun(ws.id, narratePrompt, agent, base)` with agent = workspace
  `default_agent` (fallback: first detected agent, same resolution the
  issue dispatch uses) and base from `gitBranchInfo`. The prompt names the
  file: rewrite/summarize `journal/weekly/<stamp>.md`'s Notes section from
  the listed facts, keep links intact. Declining does nothing. A no-git
  workspace never sees the offer. Reuse the existing toast-with-action
  pattern if one exists; else a minimal confirm dialog matching the
  app's dialog skin.

## File structure

**Backend:**
- `crates/agency-core/src/files.rs` — `TaskHit`, `scan_tasks`,
  `toggle_task` (+ inline test mods).
- `crates/agency-app/src/commands.rs` — `scan_tasks` (async),
  `toggle_task` (sync); registration in `lib.rs`.
- `crates/agency-app/src/state.rs` — `RunInfo.created_at` /
  `RunInfo.archived_at`.
- `crates/agency-app/src/menu.rs` — `menu:weekly-note`.

**Frontend:**
- `ui/src/lib/docsIndex.ts` — frontmatter parse in `parseDoc`,
  `DocMeta.frontmatter` / `fmEnd`, scan-skip, filter-aware `searchLocal` /
  `mergeBodyHits`.
- `ui/src/lib/docsQuery.ts` (new, pure) + `docsQuery.test.ts`.
- `ui/src/lib/livePreview.ts` — frontmatter pass + node skip.
- `ui/src/lib/tasks.ts` (new, pure) — grouping, promote body builder.
- `ui/src/lib/weeklyNote.ts` (new, pure) + `weeklyNote.test.ts`.
- `ui/src/hooks/useAllTasks.ts` (new).
- `ui/src/components/DocsTree.tsx` — filter-aware search effect.
- `ui/src/components/DocsSidePanel.tsx` — Properties section (frontmatter
  pairs, read-only).
- `ui/src/components/HomeIssues.tsx` — Tasks section.
- `ui/src/App.tsx` — `generateWeeklyNote`, `onMenu` case, narration offer.
- `ui/src/lib/paletteCommands.ts` — `weekly-note` entry.
- `ui/src/api.ts` — `scanTasks`, `toggleTask`, `TaskHit`, `RunInfo` fields.
- `ui/src/styles.css` — `.lp-fm-*`, tasks section, properties section.

---

### Task 1: frontmatter in the doc model

- [x] Failing tests (`docsIndex.test.ts`, `describe("frontmatter")`):
      fence on line 1 + `key: value` lines → ordered `frontmatter` pairs,
      quotes stripped, `fmEnd` past the closing fence; no fence / fence not
      on line 1 / unclosed fence / `---` mid-document → no frontmatter,
      `fmEnd === 0`; `# Title` *after* frontmatter is the title; `#tag` and
      `[[link]]` inside frontmatter are NOT collected; headings inside
      frontmatter not collected; duplicate keys both kept; empty value ok.
- [x] Implement in `docsIndex.ts`: `parseFrontmatter(lines)` helper,
      `DocMeta.frontmatter: [string, string][]` + `fmEnd: number`,
      `parseDoc` skips `< fmEnd` for headings/tags, `extractWikilinks`
      callers unaffected for issue bodies (issue files never reach a
      corpus) but `parseDoc`'s own link collection skips the fm range.

### Task 2: key:value search

- [x] Failing tests (`docsQuery.test.ts`): `parseDocsQuery` — plain text,
      one filter, filter + text, quoted value, bare `key:`, `#tag`
      untouched (stays in text), colon inside a word with leading digit is
      text. (`docsIndex.test.ts`): `searchLocal` with `status:draft` →
      docs whose frontmatter matches, title hits; filter + text restricts;
      no-match filter → empty; `mergeBodyHits` drops body hits from
      filtered-out paths.
- [x] Implement `docsQuery.ts`; extend `searchLocal(index, query)` and
      `mergeBodyHits(index, local, body, allowed?)`.
- [x] `DocsTree.tsx` search effect: parse once; filters-only → local only;
      filters + text → backend gets the remainder, merge with the allowed
      set; placeholder becomes `"Search notes…  (#tag, key:value)"`.

### Task 3: frontmatter rendering

- [x] `livePreview.ts`: compute the frontmatter range (share the fence
      rule with `docsIndex` via an exported helper); when unrevealed,
      fence-line replaces + `.lp-fm-line` line decos + key/value marks;
      generic tree walk skips nodes inside the range (kills the
      HRWidget/Setext artifacts). Revealed → raw text, no decos.
- [x] `DocsSidePanel.tsx`: read-only **Properties** section above Outline
      when `doc.frontmatter.length > 0` (key/value rows, `.docs-side`
      skin); value click sets the search query to `key:value` via a new
      optional `onFilter` prop wired from `DocsView` (same channel as
      `onTagClick`).
- [x] `styles.css`: `.lp-fm-*` table skin under `.docs-editor-host`,
      properties rows.

### Task 4: task scanning + toggling (backend)

- [x] Failing tests (`files.rs` inline mod `task_tests`): fixture tree →
      `scan_tasks` finds `- [ ]` / `- [x]` / `* [ ]` with 0-based lines
      and trimmed text, skips fenced blocks, skips non-corpus files
      (hidden dirs, non-md), workspace root (`rel_dir = ""`) works;
      `toggle_task` flips `[ ]`→`[x]` and back, preserves the rest of the
      file byte-for-byte, returns `false` without writing when the line is
      not the expected task or out of range; atomic (file never truncated:
      assert content after a toggle on a multi-KB file).
- [x] Implement `TaskHit` + `scan_tasks` + `toggle_task` in `files.rs`
      (reuse `walk_markdown`, `issuefs::atomic_write`).
- [x] `commands.rs`: `scan_tasks` (async) + `toggle_task` (sync) through
      `resolve_root`; register in `lib.rs`; wrappers + `TaskHit` type in
      `api.ts`.

### Task 5: Home tasks

- [x] Failing tests (`tasks.test.ts`): `groupTasks(project, docsDir, hits)`
      → per-note groups, unchecked-first ordering by path then line,
      checked excluded from `openCount`; `promoteBody(notePath)` →
      `From [[base]]` with `.md` stripped and folders dropped.
- [x] Implement `lib/tasks.ts`; `hooks/useAllTasks.ts` (5s poll, stats
      signature gate, stale-token guard, per-project catch).
- [x] `HomeIssues.tsx`: **Tasks** section under Today — section head with
      open count, per-note groups (project chip + note title, click →
      `agency:navigate` note), checkbox rows (`.lp-checkbox` skin),
      optimistic toggle via `useAllTasks().toggle`, promote action per row
      (`createIssue` + navigate), 8-group cap with explicit "N more" line,
      empty state omitted (no section when zero tasks).
- [x] `styles.css`: tasks section reusing `.today-group` / `.issue-row`
      conventions.

### Task 6: run timestamps

- [x] `state.rs`: add `created_at: i64`, `archived_at: Option<i64>` to
      `RunInfo` (camelCase serde) populated in `run_info` from the
      registry `Run`; mirror in `api.ts` `RunInfo`. Extend an existing
      app-tests assertion (`tests/state.rs`) to cover the new fields.

### Task 7: weekly note

- [x] Failing tests (`weeklyNote.test.ts`): `isoWeekStamp` — year
      boundaries (2026-01-01 → W53 of 2025? assert real ISO values for a
      few pinned dates incl. today's week), `weeklyNotePath`,
      `isoWeekStart` returns the Monday; `buildWeeklyNote` — full fixture
      (merges two projects, one closed issue, one archived run) →
      sections in order, `[[AGE-1]]` / `[[run:id|title]]` link syntax,
      empty sections omitted, empty week → header + Notes only.
- [x] Implement `lib/weeklyNote.ts`.
- [x] `App.tsx`: `generateWeeklyNote()` per the pinned flow (guard, exists
      → open + toast, gather with per-project catch, write, open);
      `case "weekly-note"` in `onMenu`; narration offer post-generation
      (git-gated, `createRun` on the workspace).
- [x] `paletteCommands.ts`: `weekly-note` (`when: "workspaceVisible"`,
      label "Generate Weekly Note"); `menu.rs`: `menu:weekly-note` in the
      File menu beside Today's Note. Drift tests must pass unmodified.

### Task 8: verification + ship gate

- [x] `cargo test`, `tsc --noEmit`, `vite build`, `vitest run` all green.
- [x] Ship gate (dev app): add `status: draft` frontmatter to a note → it
      renders as a property table, raw on click-in; search `status:draft`
      finds it, `status:draft foo` restricts body search; add `- [ ] call
      the bank` to a journal note → it appears on Home (issues mode),
      checking it edits the file (verify in the editor), the note title
      opens the note; promote a task → issue created with the note in its
      Mentions; run "Generate Weekly Note" → `journal/weekly/<this week>.md`
      opens with real merges/issues/runs content and resolving links;
      re-running opens the same file without overwriting; narration offer
      appears (git workspace) and dispatches a workspace agent when
      accepted.
- [x] Update `docs/one-stop-plan.md` status line for Phase 8.

## Post-gate fixes (2026-07-31, from first manual pass)

- [x] Tasks section drowned by plan-doc checkboxes (3000+ hits): groups now
      start collapsed with a count badge, Expand/Collapse all in the section
      head, right-click (or ⋯) on a group offers Open note / Hide note /
      Hide folder / Hide all project tasks; exclusions persist in
      localStorage with an "N hidden ✕" reset button; group cap raised to 30.
- [x] "Generate Weekly Note" unfindable in the palette: default (no-`>`)
      mode only searched the staple + recent commands; with a query it now
      searches every available command.
- [x] Workspace tab diet: Files tab hidden for the workspace (every file is
      a note; Docs covers it with autosave, Files was the one manual-save
      editor there), tab memory redirects files → docs, quick-open disabled
      for the workspace. Issues tab stays: promoted checkboxes and personal
      planning land there.
- [x] Seeded starter note: new workspaces get `Welcome.md`
      (`agency-core/src/guide.rs`), written before the initial commit, which
      demos properties (it has frontmatter), tasks (it has checkboxes),
      wikilinks, the journal, and search in place; a fresh workspace's Docs
      tab opens it automatically (only note in the corpus). Deleting it
      sticks; the palette's "Workspace Guide" command
      (`ensure_workspace_guide`) re-seeds and opens it on demand, including
      for workspaces created before this shipped.

- [x] Tasks card UX round two: the whole card folds to its header line
      (chevron + count, persisted); the "N more notes" line is now a
      "Show N more notes" / "Show fewer" toggle; the "N hidden" button
      opens a menu listing every hidden source with per-rule unhide plus
      "Unhide everything".
- [x] move_workspace guards against a destination inside the current
      workspace (the picker made `…/Workspace/New/Workspace` easy; rename
      into a subfolder of the source can never succeed) with a clear error.
- [x] Hover-pill padding: the global `button:hover` background hugged the
      new tasks buttons; `.task-collapse` gets padding with negative
      margins, and every tasks button now overrides its hover background.
- [x] Settings ▸ Workspace gains "Switch…" beside "Move…": pick a
      different folder to become the workspace (confirm dialog), old folder
      stays on disk, switching back is choosing it again. Rides the
      idempotent create_workspace, so a fresh folder is seeded with the
      guide; git preference carries over.
- [x] Live-preview reveal is now focus-gated: on mount CM parks the cursor
      at position 0 (line 1), which counted as "selection inside the
      frontmatter" and showed a fresh note's properties as raw fences until
      the first click. An unfocused editor now reveals nothing (it has no
      visible cursor); rebuilds ride `focusChanged`.

**Non-goals (resist):** YAML lists/nesting/multiline values, `title:`
override of H1, frontmatter *editing* UI beyond the raw text (the table is
read-only chrome), task sync to issues (promotion stays explicit),
checkbox scanning of issue bodies, a `closed:` issue timestamp, new git
helpers, weekly-note regeneration/overwrite, scheduling the weekly note,
notifications.
