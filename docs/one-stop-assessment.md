# Agency as a one-stop app — assessment

**Question.** Agency wants to replace VS Code, Obsidian, and Linear for a solo
developer running several projects, who also writes, plans, and journals with
agents. How close is it, and what is the shortest path to closing the gap?

**Answer in one line.** The code-and-agents half is genuinely there; the
writing-and-planning half is scoped so tightly to "a git repo you are coding in"
that half of the stated use case has nowhere to live. Three structural changes —
a non-project space, one search primitive, and making issues legible to agents —
unlock most of the remaining value, and each is smaller than the features
already shipped.

---

## 1. What exists today

Per project, five tabs (`ui/src/components/AgentsView.tsx:95-121`):

| Tab | Backed by | Substance |
| --- | --- | --- |
| **Agents** | `state.rs`, `term/`, `looper.rs` | Grid/focus of agent runs, each in its own git worktree; racing, loops, extra terminal tabs, live preview ports, notifications |
| **Issues** | `registry.rs` `issues` table | Status-grouped board, `AGE-14` keys, priority, quick capture, keyboard nav, dispatch-to-agent |
| **Docs** | `docs/` in the repo, `lib/docsIndex.ts` | Obsidian-lite: wikilinks, backlinks, tags, outline, quick switcher, live-preview editor, image paste |
| **Source Control** | `git.rs`, `gh.rs` | Changes, diff viewer, commit, history/graph, branches, stash, fetch/pull/push, in-app PR review + create + merge |
| **Files** | `files.rs` | Tree + CodeMirror editor, language modes, media/PDF/markdown/SVG preview |

Plus an all-projects home (`HomeView.tsx`) in two modes — agents and issues — and
a global palette (`CommandPalette.tsx`).

**The crown jewel, which none of the three apps has:** the issue → agent →
worktree → PR → merge → issue-closed loop is already automatic. Dispatching an
issue moves it to `in_progress` (`state.rs:1209`), opening a PR moves it to
`in_review` (`state.rs:2867`), merging closes it (`state.rs:2809`). Linear can
approximate this with GitHub integration; nothing does it locally with the agent
in the loop. This is the thing to build *around*, not the thing to dilute.

---

## 2. Scorecard against each app

Rated on *essential* function only, not feature parity.

### VS Code — ~70%

Have: file tree, editor with syntax + save/revert/word-wrap, previews, a git
panel that is arguably better than VS Code's, in-app PR review, real terminals,
merge-conflict resolution, per-worktree run scripts and preview ports.

Missing, in order of how much it hurts:

1. **Find in files.** There is no cross-file search anywhere in the codebase —
   no ripgrep invocation, no backend search command. This is the single largest
   VS Code gap and the most-used feature in any editor.
2. **Quick-open by filename.** ⌘P exists but is bound inside `DocsView` only
   (`DocsView.tsx:36-45`) and only searches notes.
3. **Editor tabs.** `FilesView.tsx:10` holds one `selected` path — opening a
   second file closes the first. No back/forward, no recent files.
4. LSP (go-to-definition, hover, diagnostics), split editors, debugger,
   extensions.

Items 1–3 are days of work. Item 4 is a different product. See §6.

### Obsidian — ~45%

Have: a real markdown corpus with wikilink resolution, backlinks, tags, an
outline pane, a quick switcher, substring full-text search, image paste to
`assets/`, autosave, and — nicely — the editor follows external edits so an
agent writing a note updates the pane live (`DocsEditor.tsx:278-292`).

Missing, and the first item swamps the rest:

1. **There is no vault that isn't a code repo.** `FileRoot` is
   `{run} | {project}` (`api.ts:828`), a project must be a git repo
   (`repoSetupView.ts:15`), and docs are always the `docs/` folder at that
   repo's root (`useDocs.ts`). Journaling, cross-project planning, reading
   notes, personal writing — the explicitly stated use case — has no home. You
   would have to create a fake repo called "notes" and pretend it is a project.
2. **No daily note / journal.** No date concept exists anywhere in the app.
3. **No cross-project note search.** The index is rebuilt per project, on tab
   entry.
4. No frontmatter/properties, no templates, no task-checkbox aggregation, no
   saved searches, no graph view.
5. Scaling: `useDocs.ts` re-reads the whole corpus every 2s and rebuilds the
   index on any change; `searchDocs` is a naive substring scan over every note's
   text. Fine at 50 notes, visibly bad at 2,000 — which is where a real journal
   lands within a year.

### Linear — ~40%

Have: per-project boards, stable `AGE-14` keys that survive renames, six
statuses, priority with `0-4` hotkeys, `c` to capture, arrow navigation, a
detail pane, and the agent-dispatch automation described above.

Missing:

1. **No cross-project working view.** `HomeView` in issues mode groups open
   issues by project but is read-only — no filter, no sort, no search, no status
   change, no priority change, no "everything urgent across all five projects."
   For a solo dev with a pipeline of projects, *this is the primary view* and it
   does not exist yet.
2. **No time.** No due dates, no cycles, no created/updated display, no "what
   did I ship this week."
3. **No labels, no manual ordering within a status, no sub-issues, no
   milestones.** Manual ordering is the one Linear users miss most — `compareIssues`
   sorts by priority then sequence, so you cannot say "this one first."
4. **No comments or activity log** — an issue is a title, a body, a status, and
   a priority. Agent runs link to it, but nothing narrates what happened.
5. **No search over issues at all.**

---

## 3. The cross-cutting gaps (the actual thesis)

The per-app lists above collapse into six things. These are what to fix.

**G1 — Everything is scoped to a git project.** This is the load-bearing
limitation. Writing, journaling, and cross-project planning are first-class in
the stated use case and structurally homeless in the app.

**G2 — No search primitive.** Not for code, not across notes, not for issues,
not in the palette. Every one of the three apps treats search as the main way
you move around. Agency has four different partial answers (docs substring
search, palette prefix filter, xterm search addon, git file filter) and no
general one.

**G3 — The palette is a navigation stub.** It lists projects, runs, and exactly
one action, "New Issue" (`CommandPalette.tsx:17-43`). In a one-stop app the
palette is the app. It should reach notes, issues, files, commands, and
settings.

**G4 — The three domains do not link to each other.** `[[wikilinks]]` resolve
only to notes. An issue cannot reference a note; a note cannot reference
`AGE-14`; nothing references an agent run. The whole value of merging the three
apps is the links between them, and there are none.

**G5 — Issues are invisible to agents.** Docs are files: git-versioned, readable
and writable by any agent in the worktree, diffable, portable. Issues are rows
in the app's private SQLite (`registry.rs:223`). So the agent you dispatch
cannot read the backlog, cannot see the acceptance criteria of a neighbouring
issue, cannot file a follow-up, and cannot tick anything off. For an
*agent-native* tracker this is the deepest inconsistency in the product.

**G6 — There is no time dimension.** No dates on issues, no daily notes, no
activity history, no weekly review. A solo operator's core question is "what
should I do today, and what happened last week," and nothing in the app answers
it.

---

## 4. What this combination is actually for

Worth stating, because it should drive the cuts.

VS Code + Obsidian + Linear, glued, is not interesting. What is interesting is
that Agency can run the whole loop:

> a thought in a journal entry → a plan note → issues derived from it → agents
> dispatched onto worktrees → PRs reviewed in-app → merges that close the issues
> → a weekly note that writes itself from what merged

Two thirds of that already work. The missing third is entirely upstream (where
does the thought live) and downstream (what does the week look like). Positioning
Agency as *the place a solo operator's work loop closes* beats positioning it as
three lightweight clones.

That framing also settles the parity question: skip anything that does not touch
the loop. Debuggers, graph views, and cycles do not.

---

## 5. Recommended roadmap

Sizing follows `beta-readiness.md`: S ≈ a day, M ≈ a few days, L ≈ a week+.

### Tier 0 — foundations (do these first; everything else gets cheaper)

**T0.1 — A non-project space. (M)**
Add a third `FileRoot` variant, `{kind: "workspace"}`, pointing at one
user-chosen folder (default `~/Agency`). It bypasses `inspectRepo`, so no git
requirement, though `git init` stays offered. Docs, Files, and Issues all accept
it. Surface it in the sidebar above the project list as a permanent entry.

Touches: `api.ts:828`, `files.rs` root resolution, `state.rs`, `ProjectTree.tsx`,
`DocsView.tsx`, `FilesView.tsx`.

This single change gives journaling, personal planning, and cross-project notes
a home, and it is the prerequisite for T1.2 and T1.4.

**T0.2 — One search command. (M)**
`search_files(root, query, {glob, case, regex, maxHits})` in Rust, ripgrep-shaped
(shell out to `rg` when present, walk + `memchr` fallback so it works without it).
Return `{path, line, col, text}`. One backend, four consumers: Files find-in-files,
Docs search (replacing the O(corpus) substring scan), the palette, and issue
search once issues are files.

Touches: new `crates/agency-core/src/search.rs`, `commands.rs`, then the callers.

**T0.3 — Decide the issue storage question. (S to decide, M to implement)**
Two viable answers to G5:

- *(a) Issues as files.* Canonical storage becomes
  `.agency/issues/AGE-14.md` with YAML frontmatter (status, priority, dates,
  labels); SQLite becomes a derived index. Agents read and write them natively,
  they version with the repo, they diff in PRs, and they get search and
  wikilinks for free. Cost: file/DB reconciliation, and the workspace root needs
  a home for non-project issues.
- *(b) Issues over MCP.* Keep SQLite; ship an `agency` stdio MCP server exposing
  `list_issues`/`create_issue`/`update_issue`/`search_docs`. Agency already
  writes MCP config into every worktree in each agent's native format
  (`mcp.rs`), so wiring this in is nearly free at the config layer. Cost: not
  versioned, not diffable, not portable, and only agents that speak MCP can see
  it.

**Recommendation: (a), with (b) later as an ergonomic layer on top.** Files are
the reason the Docs tab already works well with agents; the tracker deserves the
same property. Do this before the issue model grows — migrating 6 fields is
cheap, migrating 15 is not.

### Tier 1 — the features that make it a daily driver

**T1.1 — Real command palette. (M)** Fuzzy, multi-source: projects, runs, notes,
issues, files (via T0.2), commands, settings. Scope-aware, with actions
(`>` for commands, `#` for tags, `@` for issues). This is the highest
value-per-hour item in the document.

**T1.2 — Daily note. (S)** `journal/2026-07-27.md` in the workspace root, an
optional template, ⌘⇧D to open today, prev/next navigation. Small, and it
converts "I could journal here" into "I journal here."

**T1.3 — Cross-project issue view that works. (M)** Make `HomeView`'s issues
mode a real board: filter by status/priority/project, sort, search, inline
status and priority edits, dispatch to an agent. Add a "Today" grouping once
T1.5 lands.

**T1.4 — Find in files, quick-open, editor tabs. (M total)** The three VS Code
gaps, all riding on T0.2 and small once it exists.

**T1.5 — Dates on issues. (S)** `due` and `scheduled` on the issue model, shown
in the board, sortable. The minimum viable time dimension; skip cycles entirely.

**T1.6 — Manual ordering within a status. (S)** A float `rank` column and drag
handles. The most-missed Linear behaviour that is genuinely cheap.

### Tier 2 — the links (where the combination pays off)

**T2.1 — Universal wikilinks. (M)** Extend `resolveLink` (`docsIndex.ts:120`)
so `[[AGE-14]]` resolves to an issue and `[[run:...]]` to an agent run. Extend
backlinks symmetrically: an issue's detail pane shows notes that reference it.
Trivially easier if issues are files (T0.3a) — they just join the corpus.

**T2.2 — Issue ⇄ doc ⇄ run panel. (S)** Each of the three shows the other two.
Most of the data already exists: `run.issueId` is on the run record
(`registry.rs:53`).

**T2.3 — Frontmatter/properties in notes. (S)** Parse YAML frontmatter into
`DocMeta`, filter and sort on it. Unlocks lightweight structured notes
(meeting notes, decisions, retros) without a database.

**T2.4 — Task aggregation. (S)** Collect `- [ ]` across the corpus into one
view, grouped by note. Bridges journaling and the tracker without forcing every
thought through issue creation.

**T2.5 — Weekly review, generated. (M)** A note assembled from what merged,
what closed, and what is still open — the natural payoff of having all three
domains in one process, and the sort of thing to hand to an agent to write.

### Tier 3 — later or never

Graph view, cycles/sprints, sub-issues, labels-with-colors, saved views,
canvas/whiteboard, publishing. All are real Obsidian/Linear features; none
serves the loop enough to earn its complexity for one user.

### Also worth doing regardless

- **Docs index scaling. (S)** Move the 2s full-corpus re-read to an mtime-based
  incremental refresh, or a filesystem watcher. Cheap now, painful to retrofit
  after the vault has 2,000 notes.
- **Issue search. (S)** Falls out of T0.2 + T0.3a for free.

---

## 6. Explicit non-goals

Naming these prevents scope drift, and each is defensible:

- **LSP / go-to-definition / debugger.** Navigation and debugging are what you
  delegate to the agent. Serving them well means becoming an IDE; serving them
  badly is worse than not offering them. If code navigation becomes painful,
  "open in $EDITOR" is the honest answer.
- **Extension ecosystem.** A single-user tool does not need a plugin API.
  MCP is already the extension point.
- **Multiplayer / sync / accounts.** Directly contradicts the zero-collection
  positioning. Notes and issues living in git *is* the sync story.
- **Rich-text or WYSIWYG editing.** Markdown files, agent-editable, full stop.

---

## 7. Decisions to make now

Three questions whose answers get expensive to change later:

1. **Do issues become files?** (§T0.3) Decide before the issue model grows.
   Recommendation: yes.
2. **Is the workspace one folder or many?** One global `~/Agency` vault is
   simpler and matches Obsidian; multiple non-git "projects" is more uniform
   with the existing model. Recommendation: one workspace root, treated as a
   project with `git` optional — least new concept.
3. **Does the workspace get git?** Recommendation: offer it, default on, never
   require it. Versioned notes are a real feature and the machinery already
   exists; forcing it at add time is the current barrier.

---

## 8. Suggested sequence

A defensible order, roughly two to three weeks of focused work:

1. T0.1 workspace root — unblocks the writing use case immediately
2. T1.2 daily note — small, immediate payoff, validates T0.1
3. T0.2 search command — the shared primitive
4. T1.1 palette + T1.4 find/quick-open/tabs — both ride on it
5. T0.3 issues as files — before the model grows
6. T1.3 cross-project board + T1.5 dates + T1.6 ordering — the tracker becomes real
7. Tier 2 links — the payoff for having all three in one app

After step 4 Agency is a plausible daily driver for all three roles. After step
7 it is doing something none of the three apps can.
