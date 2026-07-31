# One-Stop Phase 7 — Links Between the Three Domains

> **For agentic workers:** Use superpowers:executing-plans style — implement
> task-by-task, failing-test-first where practical. Steps use checkbox
> (`- [x]`) syntax for tracking.

**Goal:** the reason to have one app instead of three. `[[AGE-14]]` in any
note resolves to the issue; `[[run:<id>]]` resolves to an agent run; the
issue's detail pane lists the notes (and issues) that mention it; a note's
side panel lists the issues that mention it; the focus header of a run
dispatched from an issue links back to that issue.

**Ship gate (master plan, verbatim):** write `[[AGE-14]]` in a workspace
journal note → it resolves, the issue's detail pane lists the journal entry,
the issue's run chip jumps to the agent.

## Global constraints

- pnpm, direct `node_modules/.bin` binaries when pnpm exec misbehaves.
- Verification gate: `cargo test`, `tsc --noEmit`, `vite build`, `vitest run`.
- No network calls. No emoji — glyphs stay in the existing monochrome set
  (▧ issues, ▥ notes, ▤ files, matching the palette).
- No em dashes in UI copy.
- Frontend file access stays inside the `resolve_within` jail (this phase adds
  no new file IO paths at all).

## Decisions pinned here

- **The links index is an in-memory frontend structure, not a SQLite table.**
  The master plan sketched "one `links` index table keyed
  `(from_kind, from_id, to_kind, to_id)` rebuilt with the corpora" — this
  phase keeps exactly that shape and rebuild discipline, but in TS, beside the
  existing backlinks map (`docsIndex.ts`), which is already frontend-only.
  Rationale: all three sources already reach the frontend fresh — note
  corpora via `useDocs` (mtime-signature polled), **issue bodies via
  `list_issues` index rows** (the `issues.body` column is kept current by
  reconcile), runs via `list_runs`. A backend table would mean duplicating
  the wikilink grammar *and* `resolveLink`'s basename/suffix semantics in
  Rust (drift risk), plus a staleness protocol, to derive data the client can
  derive in one pass over rows it already holds. Zero backend changes, zero
  new Tauri commands in this phase.
- **Corollary, documented deviation:** the master plan's aside that issue
  files are "*already in a corpus* once Phase 5 lands" is not true —
  `walk_markdown` skips dot-dirs, so `.agency/issues/*.md` is in no corpus
  (by design; the tracker shouldn't appear as notes). Issue-side links are
  scanned from `Issue.body` strings instead, same outcome.
- **Typed target grammar.** A wikilink target classifies as:
  - `run:<id>` (exact lowercase prefix `run:`) → **run link**; the id is a
    run uuid. Alias display (`[[run:…|the refactor run]]`) is the expected
    idiom since uuids are unreadable.
  - `KEY-n` (`/^[A-Za-z][A-Za-z0-9]{1,7}-\d+$/`) **where the prefix matches
    some project's `issue_key` case-insensitively** → **issue link**,
    resolved across all projects. If no issue with that seq exists, it's an
    *unresolved issue link*: styled unresolved, ⌘-click shows an info toast,
    and it never falls through to the create-note prompt (creating
    `AGE-99.md` as a note would shadow the tracker's namespace).
  - anything else → note link, exactly as today (including the create
    prompt when unresolved).
  A key-pattern target whose prefix matches **no** project key (e.g.
  `[[FOO-3]]` with no FOO project) is a plain note target — hyphenated note
  names keep working.
- **`resolveLink` stays note-only; the typed layer wraps it.** The master
  plan said "extend `resolveLink`"; mechanically that lands as a new
  `resolveTarget(index, cross, target)` in `ui/src/lib/links.ts` that
  consults the classifier first and delegates note targets to the existing
  `resolveLink`. Callers (live preview styling, DocsView navigation) switch
  to `resolveTarget`; `buildIndex`'s note-backlink pass is untouched.
- **Cross-domain reference data comes from a fan-out poll,** the palette's
  pattern promoted to a hook: `useCrossRefs(active)` fetches
  `listProjects` + per-project `listIssues` + `listRuns` every 5s while the
  consuming tab is active (per-project failures drop that project). Issues
  and runs are small SQLite reads; ~2N calls per tick is the same class of
  cost as the runs poll. Product of the hook: a `CrossRefs` lookup
  (label→issue, id→run, known key prefixes) plus the raw per-project rows
  for mention building.
- **The links table** lives in `ui/src/lib/links.ts` as
  `buildLinkIndex(corpora, sources) -> Map<"kind:id", LinkEdge[]>` keyed by
  the *target*; an edge carries
  `{fromKind, fromProjectId, fromId, fromTitle, line, snippet, toKind, toId}`.
  Sources scanned: every loaded corpus doc's already-parsed `links`
  (note→issue, note→run edges; note→note stays in `DocsIndex.backlinks`)
  and every issue body (issue→issue, issue→run, issue→note edges) via a new
  fence-aware `extractWikilinks(text)` shared with the corpus parser.
  Issue-body note targets resolve against the issue's own project corpus
  first, then the other loaded corpora (shortest-path rule inside each).
  Run→issue edges need no parsing — `run.issueId` already exists and the
  detail pane consumes it directly.
- **Issue identity in the table is the uuid** (stable across reconcile, per
  `issuefs.rs` — files are keyed `AGE-14` but rows keep their id), so
  mention lists survive renames of titles and external file edits. Note
  identity is `${projectId}:${path}`.
- **Which corpora feed which view:**
  - DocsView: its own corpus only. The side panel's new **Mentions** section
    is "issues linking to this note" (master plan wording) — that needs
    cross-project issues (from `useCrossRefs`) resolved against the open
    corpus, not other corpora.
  - IssuesView: its project's corpus **plus the workspace corpus** (two
    `useDocs` instances; deduped when the project *is* the workspace). This
    is exactly what the ship gate needs — journal notes live in the
    workspace, the issue lives in a repo project. Notes in a *third*
    project's docs mentioning the issue are out of scope this phase
    (documented limitation; loading N corpora from the Issues tab isn't
    worth it before someone hits it).
- **Cross-domain navigation is one Shell event.** A new `agency:navigate`
  CustomEvent (`{kind: "note"|"issue"|"run", projectId, ...}`) handled in
  `App.tsx`'s Shell, reusing `selectProject`/`openRun`/`setTab` and the
  existing handoffs (`PENDING_ISSUE_KEY` sessionStorage, `docs:last` +
  `agency:open-note` for notes). This is the established pattern
  (`agency:add-project`, `agency:workspace-ready`, tray events); panels and
  the editor can then navigate from anywhere without prop-drilling Shell
  handlers through five layers. The palette keeps its prop-based routing.
- **Completion** (`[[` in the docs editor) gains issue entries: label
  `AGE-14`, detail = issue title, sorted after note matches on ties
  (CM `boost`). Runs are *not* offered (uuids are unguessable; the idiom is
  paste-from-focus or alias). Closed issues are offered too — linking a
  done issue from a retro note is normal.
- **AgentFocus issue chip:** when `focused.issueId` is set, the agent
  focus header shows `▧ AGE-14` (label via a one-shot `listIssues` +
  `listProjects` lookup on focus change, tooltip = issue title); click
  navigates via `agency:navigate`. The reverse direction ("runs on this
  issue" in IssueDetail with jump-to-focus) already shipped in P5/P6 —
  verify, don't rebuild.
- **Staleness policy:** mention panels and link styling lag live edits by at
  most autosave (800ms) + one poll tick (≤2s corpus, ≤5s cross-refs), same
  contract as today's backlinks panel. No push machinery.

## File structure

**Backend:** no changes.

**Frontend:**
- `ui/src/lib/docsIndex.ts` — export `extractWikilinks(text)` (fence- and
  inline-code-aware scan; `parseDoc` delegates to it).
- `ui/src/lib/links.ts` (new, pure, unit-tested) — target classifier,
  `CrossRefs` + `buildCrossRefs`, `resolveTarget`, `LinkEdge` +
  `buildLinkIndex` + `mentionsOf`, `issueCompletionOptions`.
- `ui/src/lib/links.test.ts` (new).
- `ui/src/hooks/useCrossRefs.ts` (new) — the fan-out poll.
- `ui/src/lib/livePreview.ts` — `crossRefsFacet`; typed styling/tooltips for
  wikilinks; issue completion source.
- `ui/src/components/DocsEditor.tsx` — `cross` prop wired through a
  compartment (same shape as the index facet).
- `ui/src/components/DocsView.tsx` — `useCrossRefs`, typed `navigate`,
  mentions for the side panel.
- `ui/src/components/DocsSidePanel.tsx` — Mentions section.
- `ui/src/components/IssuesView.tsx` — workspace + project corpora,
  link index, mentions to the detail pane.
- `ui/src/components/IssueDetail.tsx` — Mentions section (notes + issues).
- `ui/src/components/AgentFocus.tsx` — issue chip in the focus header.
- `ui/src/App.tsx` — `agency:navigate` listener.
- `ui/src/styles.css` — chip, mention rows, unresolved-issue styling reuses
  `.lp-wikilink.unresolved`.

---

### Task 1: link model — classifier, CrossRefs, resolveTarget

- [x] Failing tests (`ui/src/lib/links.test.ts`): `classifyTarget` — `run:x`
      → run, `AGE-14`/`age-14` → issue-pattern, `FOO-3`, `Note`,
      `guides/Setup`, `2026-07-31` (date-named daily notes are
      key-pattern-shaped but their prefix won't match a project key — the
      classifier alone can't decide, so classification of `KEY-n` requires
      the known-prefix check in `resolveTarget`; assert the raw pattern
      match only); `buildCrossRefs` — label map is case-insensitive, keyed
      off each project's `issue_key`, run map spans projects;
      `resolveTarget` — known prefix + existing issue → issue hit; known
      prefix + missing seq → unresolved-issue (never note fallback);
      unknown prefix falls through to `resolveLink`; `run:` resolved and
      unresolved; null `cross` degrades to note behavior.
- [x] Implement `ui/src/lib/links.ts`: `ISSUE_TARGET_RE`, `classifyTarget`,
      `IssueRef`/`RunRef`, `CrossRefs`, `buildCrossRefs(sources)`,
      `resolveTarget(index, cross, target) -> Resolution` (discriminated
      union: `note` | `issue` | `run` | `unresolved-issue` |
      `unresolved-run` | `unresolved-note`).

### Task 2: extractWikilinks + the links table

- [x] Failing tests: `extractWikilinks` — finds targets with heading/alias
      parts, skips fenced blocks and inline code, reports 0-based lines
      (mirror the parseDoc cases in `docsIndex.test.ts`); `buildLinkIndex` —
      fixture of two corpora (workspace + project) and issue rows: a
      workspace note `[[AGE-14]]` yields a note→issue edge with
      title/snippet/line; an issue body `[[2026-07-31]]` resolves against
      the workspace corpus → issue→note edge; `[[AGE-2]]` in an issue body
      → issue→issue edge; `[[run:<uuid>]]` in a note → note→run edge;
      self-links are dropped; `mentionsOf` filters by target key.
- [x] Implement: export `extractWikilinks` from `docsIndex.ts` (parseDoc
      delegates to one shared line scan); `buildLinkIndex` + `mentionsOf` +
      `issueCompletionOptions(cross)` in `links.ts`.

### Task 3: useCrossRefs

- [x] Implement `ui/src/hooks/useCrossRefs.ts`: while `active`, fetch
      projects then fan out `listIssues` + `listRuns` per project
      (per-project `.catch` drops that project), 5s interval + leading
      fetch, stale-reply guard, returns
      `{ cross, sources, refresh }` (`null`/`[]` until the first pass).
      Hook logic mirrors `useIssues`; no test (IPC-bound, thin).

### Task 4: editor — typed styling, completion, navigation

- [x] `livePreview.ts`: `crossRefsFacet`; the Wikilink decoration case calls
      `resolveTarget` — resolved issue/run/note all read as links, the
      unresolved variants get `.unresolved`; tooltips: "⌘-click to open
      AGE-14" / "No matching issue" / "⌘-click to open run" / "⌘-click to
      create".
- [x] `livePreview.ts` completion: `wikilinkCompletions` appends
      `issueCompletionOptions` (label `AGE-14`, detail title, apply
      `AGE-14]]`, negative boost so notes win ties).
- [x] `DocsEditor.tsx`: `cross: CrossRefs | null` prop, second compartment
      reconfigured like the index one.
- [x] `DocsView.tsx`: `useCrossRefs(true)`; `navigate` classifies via
      `resolveTarget` — issue → `agency:navigate` issue, run →
      `agency:navigate` run, unresolved-issue → `toastInfo("No issue …")`,
      note behavior unchanged (create prompt included).
- [x] `App.tsx` Shell: `agency:navigate` listener — note: stamp
      `docs:last`, `selectProject`, `setTab("docs")`, re-dispatch
      `agency:open-note`; issue: `PENDING_ISSUE_KEY`, `selectProject`,
      `setTab("issues")`; run: `openRun`. Project rows resolved via
      `listProjects` like the tray handlers.

### Task 5: mentions panels

- [x] `DocsView.tsx`: build the link index from `[{project, index}]` +
      cross sources (memoized on index/sources identity); side panel gets
      `mentions = mentionsOf(table, "note", `${project.id}:${selected}`)`.
- [x] `DocsSidePanel.tsx`: collapsible **Mentions** section under
      Backlinks — rows `▧ AGE-14 <issue title>` + snippet, count badge,
      click → `agency:navigate` issue; "No mentions" empty state.
- [x] `IssuesView.tsx`: `getWorkspace` once; second `useDocs` on the
      workspace (skipped when this project is the workspace);
      `useCrossRefs(tab === "issues")`; memoized link index over both
      corpora; `mentions = mentionsOf(table, "issue", selected.id)` into
      the detail pane.
- [x] `IssueDetail.tsx`: **Mentions** section under the runs list — note
      rows (▥ title + snippet) and issue rows (▧ label + title), click →
      `agency:navigate` (note/issue); section hidden when empty.
- [x] `styles.css`: mention rows (reuse `.docs-backlink-row` skin), chip.

### Task 6: AgentFocus issue chip

- [x] `AgentFocus.tsx`: when the focused agent has `issueId`, resolve
      `▧ KEY-seq` + title via one-shot `listProjects`/`listIssues` on
      `focused.issueId` change; render a chip after the branch code in the
      focus head; click → `agency:navigate` issue; chip absent for
      terminals and issue-less runs, stale-reply guarded.
- [x] Verify (no code): IssueDetail's existing "Agents on this issue" list
      still jumps to focus from both IssuesView and via the board.

### Task 7: verification + ship gate

- [x] `cargo test`, `tsc --noEmit`, `vite build`, `vitest run` all green.
- [ ] Ship gate (dev app): in a workspace journal note type `[[AG` →
      completion offers issues; accept `[[AGE-14]]` → renders as a resolved
      link; ⌘-click → lands on the issue in its project; the issue's detail
      pane lists the journal entry under Mentions, click returns to the
      note; dispatch the issue to an agent → focus header shows `▧ AGE-14`,
      click returns to the issue; type `[[run:<id>]]` (id from a live run)
      in a note → resolves, ⌘-click lands in that run's focus; `[[AGE-999]]`
      styles unresolved and ⌘-click toasts instead of offering to create a
      note; a made-up `[[FOO-3]]` still offers note creation.
- [x] Update `docs/one-stop-plan.md` status line for Phase 7.

**Non-goals (resist):** a backend links table or any new Tauri command,
rendering wikilinks inside the issue-body textarea, run completion after
`[[`, mentions from non-workspace third-project corpora, note frontmatter
and task aggregation (P8), promote-checkbox-to-issue (P8), notifications.
