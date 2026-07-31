# One-Stop Agency — Master Implementation Plan

Companion to `docs/one-stop-assessment.md`. That doc says *what and why*; this
one says *in what order and how*. Each phase below is independently shippable
and gets its own detailed task-by-task plan (in the style of
`docs/superpowers/plans/`) when we start it — this document pins the sequence,
the architecture decisions, and the per-phase scope so those detailed plans
don't re-litigate anything.

## Decisions (locked 2026-07-27)

- **D1 — Issues become files.** Canonical storage is markdown + YAML
  frontmatter under `.agency/issues/`; SQLite becomes a derived, rebuildable
  index. An MCP layer may come later as ergonomics, not as the source of truth.
- **D2 — One workspace root.** A single user-chosen folder (default
  `~/Agency`) is the home for journaling, planning, and cross-project notes.
  Modeled as a *pinned project with relaxed requirements*: git offered and
  default-on at creation, never required.
- **D3 — Execution style.** Sequenced phases, Claude driving, verify each
  phase before starting the next.

## Global constraints (apply to every phase)

- UI package manager is **pnpm**, never npm; run tests/builds via direct
  `node_modules/.bin` binaries when pnpm exec misbehaves (known pnpm 11 gotcha).
- Verification gate per phase: `cargo test` green, `tsc --noEmit` exit 0,
  `vite build` exit 0, `vitest run` green.
- SQLite schema changes are **additive migrations** guarded by
  `column_exists(...)` (`registry.rs`). Never rewrite a `CREATE TABLE`.
- Every git call goes through the `Command::new("git")` helper pattern in
  `agency-core/src/git.rs`. Match it.
- New API-facing Rust structs that cross to TS use
  `#[serde(rename_all = "camelCase")]`.
- **No emoji in the UI.** Icons are monochrome text glyphs (◈ ⇋ ▤ ▧) or
  stroked SVGs matching the existing convention.
- All file access from the frontend goes through a `FileRoot` and
  `resolve_within` — no new path-taking commands that bypass the jail.
- Nothing in these phases adds network calls. The zero-first-party-data-
  collection positioning is a hard invariant.

---

## Phase 1 — Workspace root + daily note

**Goal:** journaling, planning, and personal notes get a home; the daily-note
habit works end to end. This is the highest-leverage phase because it unblocks
the entire writing half of the use case with mostly-existing machinery.

**Architecture.** The workspace is a **project row** with a new nullable
`kind` column (`NULL`/`"repo"` = normal, `"workspace"` = the pinned one).
Rationale: every per-project subsystem (docs detection, files, issues, runs,
palette, sidebar) already keys off a project id — a parallel concept would
mean branching in a dozen call sites; a flagged project means branching in
three:

1. **Add flow** (`repoSetupView.ts`, `RepoSetupDialog`): a "Create workspace"
   path that offers `git init` (default on) but proceeds without it.
2. **Readiness gating**: with no git, the Source Control tab and agent spawn
   are hidden for the workspace; with git (the default), they work exactly as
   in any project — which means **agents can be dispatched on writing tasks in
   the workspace**, in worktrees, with the full merge flow. This falls out for
   free and is a headline feature, not an accident.
3. **Docs root**: for `kind = "workspace"`, `detect_docs_dir` returns `""` —
   the *whole folder* is the vault, not a `docs/` subfolder. Verify
   `resolve_within(root, "")` and `read_markdown_corpus(root, "")` handle the
   empty relative path (they resolve to the root; add tests).

**Daily note** rides on this: `journal/YYYY-MM-DD.md` in the workspace,
⌘⇧D opens (creating from an optional `templates/daily.md` if present),
prev/next-day navigation in the editor header, palette entry "Today's note".

**Scope.**
- `registry.rs`: `projects.kind` migration; `ensure_workspace()` no-op if
  present; workspace excluded from issue-key backfill until Phase 5.
- `setup.rs` / `state.rs` / `commands.rs`: workspace-aware add/inspect paths.
- `ProjectTree.tsx`: pinned workspace entry above the project list, distinct
  glyph, not part of the active-filter logic.
- `DocsView.tsx` / `useDocs.ts`: root-as-vault; skip `journal/` special-casing
  beyond sort order (newest first within that folder).
- New `ui/src/lib/dailyNote.ts` (pure date/path logic, unit-tested) +
  shortcut wiring in `useShortcuts.ts` + menu item in `menu.rs`.
- First-run: Settings gains "Workspace location" (choose/move); the workspace
  is created lazily on first use, not at app launch — no surprise folders.

**Non-goals for this phase:** cross-project search, frontmatter, task
aggregation. Resist.

**Ship gate:** create workspace → decline git → Docs/Files work, Source/Agents
hidden. Recreate with git → dispatch an agent on a writing prompt → merge its
branch. ⌘⇧D on a fresh day creates from template. All four verification
commands green.

---

## Phase 2 — The search primitive

**Goal:** one backend search command that Files, Docs, the palette, and (later)
issues all consume. Also fixes docs-index scaling while we're in the area.

**Architecture.** New `crates/agency-core/src/search.rs`:

```
search_files(root: &Path, q: SearchQuery) -> Result<Vec<SearchHit>>
  SearchQuery { query, regex: bool, case: bool, globs: Vec<String>, max_hits }
  SearchHit  { path, line, col, text, context_before/after? }
```

- Shell out to `rg --json` when present on PATH (probe via the existing
  `pathenv.rs` login-shell PATH logic — same mechanism as agent detection);
  fall back to a bounded walk + line scan in Rust so search works everywhere.
- Hard caps: hits, per-file hits, total bytes scanned, and a wall-clock
  timeout — a search must never wedge the IPC thread. Async command
  (`commands.rs`), like the Phase-14 loop-gate fix.
- Respect `.gitignore` in repo roots (rg does natively; fallback walks
  tracked+untracked-not-ignored via `git ls-files` when in a repo, plain walk
  otherwise).

**Docs index scaling (same phase, same files):** replace the 2s full-corpus
re-read in `useDocs.ts`/`read_markdown_corpus` with an mtime-signature pass —
backend returns `(path, mtime, size)` cheaply; the corpus body is re-read only
for changed files. Swap `searchDocs`'s substring scan to `search_files` for
body hits, keeping title/tag matching in the TS index (it's already cheap and
semantic).

**Scope:** `search.rs` (+ unit tests with a fixture tree), `commands.rs`,
`lib.rs` registration, `api.ts`, `useDocs.ts`, `docsIndex.ts` (search path
only). No UI beyond wiring Docs search to the new backend.

**Ship gate:** search a 10k-file repo without rg installed and with it; both
return within the timeout with identical shapes. Docs tab on a 500-note corpus
idles without re-reading bodies (verify via log counters in dev).

---

## Phase 3 — Editor gaps: quick-open, find-in-files, tabs

**Goal:** the three VS Code gaps, all riding on Phase 2.

- **Quick-open (⌘P outside Docs):** file-name fuzzy match over the current
  root. Backend: cheap `list_files(root, limit)` using `git ls-files` when
  possible (fast, ignores noise), walk fallback. Frontend: reuse the
  `DocsQuickSwitcher` interaction pattern; one shared fuzzy-filter util with
  `paletteFilter.ts`.
- **Find-in-files:** a search input atop the Files tab tree; results grouped
  by file (path → matching lines), click lands in the editor **at the line**.
  `FileEditor` gains a `scrollToLine` handle (mirror `scrollToHeading` in
  `DocsEditor.tsx`).
- **Editor tabs:** `FilesView` holds `open: string[]` + `active` instead of one
  `selected` (`FilesView.tsx:10`). Tab strip above the editor; middle-click /
  ⌘W closes; dirty dot per tab; per-root persistence in `localStorage` like
  `usePaneWidth`. Cap open tabs (~10, LRU-evict clean ones) to bound memory —
  each tab is a live CodeMirror instance, so evicted tabs keep only
  `{path, scrollPos}`.

**Ship gate:** open 5 files in tabs, edit two, switch roots and back — dirty
state and active tab survive; find-in-files jumps to the exact line; ⌘P works
from every tab of a project, and still does the Docs switcher inside Docs.

---

## Phase 4 — Command palette v2

**Goal:** the palette becomes the app's front door.

**Architecture.** Rebuild `CommandPalette.tsx` around a **provider registry**:
each provider contributes entries for a query mode, sync from local state or
async debounced from the backend:

| Prefix | Provider | Source |
| --- | --- | --- |
| *(none)* | projects, runs, recent notes/files, top commands | in-memory |
| `>` | commands (new agent, new terminal, toggle sidebar, settings, today's note, fetch, …) | static registry, reusing the `onMenu` action ids in `App.tsx` so palette and native menu can't drift |
| `#` | tags → notes | docs index |
| `@` | issues | `list_issues` (SQLite now, files after Phase 5 — provider interface hides the swap) |
| `/` | file contents | `search_files`, debounced 150ms |

Scope-aware: inside a project, that project's entries rank first; at home,
cross-project. Recency-weighted ranking (persist last-N activations in
`localStorage`). Extract `useModalKeys`-based list navigation shared with the
quick switchers.

**Ship gate:** every native-menu action is reachable via `>`; `@` finds an
issue in a non-selected project and jumps to it; `/` content search lands in
the editor at the line.

---

## Phase 5 — Issues as files

**Goal:** the tracker becomes agent-native, versioned, and portable. The
riskiest phase — do it before the issue model grows, and land the migration
behind a compatibility layer so the UI barely notices.

**Format.** One file per issue, `.agency/issues/AGE-14.md`:

```markdown
---
key: AGE-14
status: in_progress        # backlog|todo|in_progress|in_review|done|cancelled
priority: 2                # 0-4
created: 2026-07-27T09:30:00Z
updated: 2026-07-27T14:02:00Z
---
# Fix terminal resize on reattach

Body markdown, wikilinks allowed.
```

Filename is the key; title is the H1. `id` (uuid) stays DB-internal only —
files are keyed by `key`, which is already stable and never reused.

**Architecture.**

- New `crates/agency-core/src/issuefs.rs`: parse/serialize (strict frontmatter,
  forgiving body), `read_issue_dir`, atomic write (temp + rename). Unit-tested
  round-trip.
- **SQLite reduces to (a) the seq high-water mark** (`issue_seqs` stays — files
  can be deleted, numbers must never be reused) **and (b) a rebuildable index**
  for fast list/joins with runs. On project open and on a poll signature
  (mtime pass, same trick as Phase 2 docs), reconcile: files are truth, DB rows
  follow; a DB row with no file is dropped.
- **Migration:** one-shot, per project, on first open at the new version:
  export every SQLite issue to `.agency/issues/`, then flip that project to
  file-mode (a `projects.issues_migrated` column). `.agency/issues/` must NOT
  be gitignored — check `.gitignore` handling around `agency.local.toml` and
  ensure only `*.local.toml` is excluded.
- **Write path:** all existing mutations (`create_issue`, `update_issue`,
  `advance_issue_status`, delete) write the file first, then refresh the index
  row. The Tauri command surface and `api.ts` types stay **unchanged** — the
  UI does not participate in this phase beyond a conflict toast.
- **Agent flow property (document it in-app):** an agent editing issue files in
  its worktree stages tracker changes *on its branch* — they land at merge,
  exactly like code. The app's board reads the main checkout. Merging a run
  that edits `AGE-14.md` therefore both closes the issue via the existing hook
  *and* merges any body edits. `advance_issue_status` keeps its
  forward-only rank guard so a merge never regresses a manually-moved issue.
- **Workspace issues:** the workspace project gets an issue key at this point
  (extend the backfill), giving personal/planning tasks the same tracker.
- **Conflict policy:** last-writer-wins at file level (same as docs), with the
  `updated` field bumping on every app write; the app's poll picks up external
  edits within a tick.

**Ship gate:** migrate a project with existing issues; `git log` shows the
export commit-able; create/edit/dispatch/merge flows unchanged in the UI; an
agent instructed to "mark AGE-7 done and file a follow-up issue" succeeds by
editing/creating files, and the board reflects it after merge. Full test suite
green including new `issuefs` round-trip and reconciliation tests.

---

## Phase 6 — Tracker depth

**Goal:** the cross-project board becomes the primary working view; the model
gains just enough structure (dates, manual order, search).

- **Dates:** `due`, `scheduled` in frontmatter + index columns (additive
  migration). Shown in `IssueRow`/`IssueDetail`; overdue glyph; sortable.
- **Manual rank:** `rank: float` in frontmatter; drag to reorder within a
  status group (fractional ranks, periodic renormalize). `compareIssues`
  becomes rank-first within a status, priority as tiebreak.
- **Cross-project board (`HomeView` issues mode → real view):** filter by
  status/priority/project, text search (index-backed), inline status/priority
  edits, dispatch-to-agent from the row, and a **Today** section on top:
  due/scheduled ≤ today + everything `in_progress`. This view is the answer to
  "what should I do today."
- Issue search joins the palette `@` provider properly (title + body via
  `search_files` over `.agency/issues/`).

**Ship gate:** across ≥3 projects: filter, reorder within a status by drag,
set a due date, see it in Today, dispatch from the board without entering the
project.

---

## Phase 7 — Links between the three domains

**Goal:** the reason to have one app instead of three.

- **Universal wikilink resolution:** extend `resolveLink` (`docsIndex.ts:120`)
  with typed targets — `[[AGE-14]]` (matches any project's key pattern; issue
  files are markdown so they're *already in a corpus* once Phase 5 lands, this
  is mostly making Docs-corpus links resolve *across* corpora) and
  `[[run:<id>]]`. Completion (`docsCompletion`) offers issues after `[[`.
- **Cross-domain backlinks:** issue detail shows notes linking to it; note
  side panel shows issues linking to the note. One `links` index table keyed
  `(from_kind, from_id, to_kind, to_id)` rebuilt with the corpora.
- **Context panels:** `IssueDetail` lists linked runs (`run.issueId` exists —
  `registry.rs:53`) with jump-to-focus; `AgentFocus` header shows its issue
  chip; `DocsSidePanel` gains a "Mentions" section.

**Ship gate:** write `[[AGE-14]]` in a workspace journal note → it resolves,
the issue's detail pane lists the journal entry, the issue's run chip jumps to
the agent.

---

## Phase 8 — Time closes the loop

**Goal:** the app answers "what happened," not just "what now."

- **Note frontmatter:** parse YAML frontmatter into `DocMeta`; render as a
  property table in the editor; filterable in docs search (`key:value`).
- **Task aggregation:** collect `- [ ]` / `- [x]` across the workspace + open
  projects into a Tasks section on Home; checking a box edits the source file
  in place. Deliberately *not* synced to issues — checkboxes are for thoughts,
  issues are for work; promotion is one explicit "promote to issue" action.
- **Weekly review, generated:** a command ("Generate weekly note") that
  assembles `journal/weekly/2026-W31.md` from local data — merges (`git log`
  across projects), issues closed (frontmatter `updated` + status), agent runs
  archived — then, optionally, dispatches a workspace agent to narrate it.
  Pure local reads; the agent step reuses the ordinary dispatch flow.

**Ship gate:** a week of normal use produces a weekly note with real content;
checking a task on Home edits the underlying markdown; frontmatter renders and
filters.

---

## Sequencing rationale & risk register

```
P1 workspace ──► P2 search ──► P3 editor ──► P4 palette ──► P5 issues-as-files ──► P6 tracker ──► P7 links ──► P8 time
   (value now)     (primitive)    (rides P2)    (rides P2)      (structural)          (rides P5)     (rides P5)   (rides P1+P5)
```

- **P1 first**, not P2: it delivers user-visible value immediately and derisks
  D2 while every later phase still works if D2's modeling needed adjustment.
- **P5 sits mid-sequence deliberately:** late enough that search and the
  palette (its consumers) exist to prove it, early enough that dates/rank/
  labels (P6) are designed file-first instead of migrated twice. **Nothing in
  P1–P4 may add issue-model fields** — that's the discipline that keeps P5
  cheap.
- **Riskiest items:** (1) P5 reconciliation — mitigated by files-as-truth,
  rebuildable index, and the migration flag per project; (2) P1 empty-rel-path
  handling in the file jail — mitigated by explicit tests on
  `resolve_within(root, "")`; (3) P3 tab memory — mitigated by the LRU cap.
- **Each phase ends with a working app** — no phase leaves a tab half-migrated.

## Process per phase (D3)

1. Write the detailed task-by-task plan for the phase (same format as
   `docs/superpowers/plans/`, checkboxes, failing-test-first where practical).
2. Implement task by task; run the four-command verification gate.
3. Manual ship-gate pass (the phase's gate above) in the dev app.
4. Update this document's status line below; commit; next phase.

## Status

- [x] Phase 1 — Workspace root + daily note — *shipped 2026-07-28: verification
  gate green, manual ship gate passed
  (plan: `docs/superpowers/plans/2026-07-28-one-stop-phase1-workspace-daily-note.md`)*
- [x] Phase 2 — Search primitive — *shipped 2026-07-28: verification gate green,
  ship gates passed (10k files: 304ms fallback / 144ms rg; idle corpus does zero
  body reads; plan: `docs/superpowers/plans/2026-07-28-one-stop-phase2-search-primitive.md`)*
- [x] Phase 3 — Quick-open, find-in-files, tabs — *shipped 2026-07-30:
  verification gate green, ship gate passed
  (plan: `docs/superpowers/plans/2026-07-28-one-stop-phase3-editor-gaps.md`)*
- [x] Phase 4 — Command palette v2 — *shipped 2026-07-30: verification gate
  green, ship gate passed
  (plan: `docs/superpowers/plans/2026-07-30-one-stop-phase4-command-palette.md`)*
- [x] Phase 5 — Issues as files — *shipped 2026-07-30: verification gate green,
  ship gate passed
  (plan: `docs/superpowers/plans/2026-07-30-one-stop-phase5-issues-as-files.md`)*
- [x] Phase 6 — Tracker depth — *shipped 2026-07-31: verification gate green,
  ship gate passed
  (plan: `docs/superpowers/plans/2026-07-30-one-stop-phase6-tracker-depth.md`)*
- [ ] Phase 7 — Cross-domain links — *implemented 2026-07-31, verification
  gate green; manual ship-gate pass pending
  (plan: `docs/superpowers/plans/2026-07-31-one-stop-phase7-cross-domain-links.md`)*
- [ ] Phase 8 — Time / review
