# Built-in Issue Tracker (Linear-inspired, agent-native) — Design

**Date:** 2026-07-14
**Status:** Accepted (open questions resolved 2026-07-14)

## Goal

Give every Agency project a lightweight, local-first issue/backlog tracker so
ideas and tasks can be captured while agents work, then dispatched to a new
agent with one action. Linear is the design inspiration (statuses, priorities,
keyboard-quick capture, clean list UI) but this is deliberately **not** a
Linear clone: no cycles, labels, teams, comments, or multi-user anything. The
differentiator is the piece Linear can't offer: **issue status is driven by
what agents actually did** — assigning an issue spawns a run, merging the
run's branch closes the issue.

Explicitly local-first: issues live in Agency's own SQLite registry. No GitHub
Issues dependency (a sync could be layered on later, but is out of scope).

## Background (what already exists)

- **Registry**: one SQLite DB opened in `Registry::open`
  (`crates/agency-core/src/registry.rs:85`), with an established migration
  pattern — `column_exists` + `ALTER TABLE` for new columns
  (`registry.rs:141-173`), `CREATE TABLE IF NOT EXISTS` for new tables, and
  backfill helpers for derived per-project values
  (`backfill_project_colors`, `registry.rs:182`).
- **Runs** (`registry.rs:27-50`): `id`, `project_id`, `agent`, `prompt`,
  `base`, `branch`, `created_at`, `archived_at`, `title`, `kind`,
  `merge_target`, `race_id`, `loop_config`, `loop_state`. An issue link slots
  in as one more nullable column.
- **Prompted run creation**: `create_run_spec` (`state.rs:781`) already
  delivers a non-empty prompt as the agent's initial positional prompt (or via
  a `{{prompt}}` token). `create_run_from_issue` (`state.rs:986`) is the
  exact pattern to mirror: it composes a prompt from a GitHub issue's
  title/body and sets the run `title` up front, so the first-line
  title-capture (`set_run_prompt_if_empty`, `registry.rs:447`) never fires.
- **Racing & looping**: `create_race` (`state.rs:902`) spawns N runs on one
  prompt with a shared `race_id`; `create_loop` re-invokes an agent headless
  until a check command passes. Both take a prompt, so both compose naturally
  with "hand this issue to agents".
- **The add-menu**: `AgentAddMenu.tsx` is the single shared spawn menu (both
  call sites use it so options can't drift): the list of agent profiles,
  `∥ Race agents…`, `⟳ Loop agent…`, GitHub issue/PR import, `New terminal`,
  and from/into branch pickers. `RaceDialog` and `LoopDialog` collect the
  prompt + options.
- **Merge lifecycle**: `merge_task` (`state.rs:2376`) returns
  `MergeOutcome::Clean` on success — the hook for auto-closing an issue.
  `create_pr` (`state.rs:2415`) is the hook for "in review".
  `discard_run` (`state.rs:1518`) / `archive_run` (`state.rs:1550`) are the
  hooks for rolling an abandoned issue back to the backlog.
- **UI shell**: `AgentsView` has a tab strip — Agents / Source Control / Files
  (`AgentsView.tsx:78-80`), tab state in the run store
  (`store/runs.tsx:6`). `HomeView.tsx` is the landing view when no project is
  selected: a live overview of every agent grouped by project, fetching
  per-project over the same APIs the project views use. `CommandPalette.tsx`
  exists for quick actions. UI polls `list_runs` every 1.5 s
  (`store/runs.tsx:110-114`).
- **Project identity**: projects have a `color` accent (`registry.rs:14-24`)
  the issue UI reuses for identity chips.

## Behavior

### The board

A new **Issues** tab per project, next to Agents / Source Control / Files.
Issues are scoped to the project they live in, exactly like agents.

**Decided:** v1 is a **list grouped by status** (Linear's default view).
Status changes happen via a click-through pill / context menu / keyboard. A
drag-and-drop kanban board is a documented future feature (see
[Future features](#future-features)) — the data model is identical either
way, so it's purely a presentation addition later.

Statuses (fixed set, no custom workflows):

| Status | Meaning |
|---|---|
| `backlog` | Captured, not yet committed to |
| `todo` | Ready to be picked up |
| `in_progress` | An agent run is working on it |
| `in_review` | Work exists and awaits human review/merge |
| `done` | Merged / completed |
| `cancelled` | Won't do |

Groups render in that order; `done` and `cancelled` are collapsed by default.
Within a group, issues sort by priority (urgent first), then `created_at`.

Priorities, Linear-style but numeric-simple: `0 none · 1 low · 2 medium ·
3 high · 4 urgent`, rendered as the familiar signal-bar glyph.

### Issue keys

**Decided:** every project gets a stable **3-letter key** derived from its
name (e.g. "Agency" → `AGE`, "Todo App" → `TAO`), and issues are numbered
per-project: `AGE-14`. This keeps local issues visually distinct from GitHub
issue numbers (`#14`), which already appear in run titles via the GitHub
import flow (`create_run_from_issue` formats `#<n> <title>`).

- Derivation: uppercase alphanumerics of the name; multi-word names use word
  initials (padded from the first word's remaining letters if fewer than 3),
  single words take the first three letters. On collision with an existing
  project's key, later characters of the name are substituted in the last
  position until unique (falling back to A–Z).
- The key is **stored** on the project (not derived on the fly) so renaming a
  project never re-keys its issues. Existing projects are backfilled on
  migration, same pattern as `backfill_project_colors`.
- The key + per-project `seq` display everywhere the issue appears, including
  the linked run's title (`AGE-14 <title>`).

### Quick capture

- A single-line quick-add input at the top of the Issues view: type a title,
  Enter → issue created in `todo` (Shift+Enter → `backlog`). Stays focused
  for rapid multi-entry.
- Command palette gains **"New Issue"**, available from anywhere in a
  project, so capture doesn't require leaving a running agent's focus view.

### Issue detail

Clicking a row opens a detail pane: editable title, markdown body (plain
textarea in v1), status + priority selects, created/updated timestamps, and a
**linked runs** section listing every run spawned from the issue with its live
session status — click to jump to that run's focus view (`openRun`,
`App.tsx:98`).

### The killer interaction: Start agent

**Decided:** the issue's **Start agent** action offers the same options as
the project's `+ Agent` button — it opens the shared `AgentAddMenu`,
parameterized for issue context:

- **Agent profiles list** — pick any defined agent; spawns a run on the
  issue.
- **∥ Race agents…** — `RaceDialog` with the prompt pre-filled from the
  issue (read-only); N runs share a `race_id` and all link the issue.
- **⟳ Loop agent…** — `LoopDialog` with the prompt pre-filled from the
  issue; the user supplies the check command as usual.
- **from/into branch pickers** — same base/merge-target selection as the
  normal menu.
- Omitted in issue context: `New terminal` and the GitHub issue/PR imports
  (neither makes sense when dispatching a local issue).

The default-click path (no menu) uses the same agent pick order as
`newTaskDefaultAgent` (`App.tsx:36`): Settings default → project's last-used
→ `claude`. That logic gets extracted into a shared helper so the menu flow
and Start-agent can't drift.

In every variant the composed prompt is:

```
Work on issue AGE-14: <title>

<body>
```

…the run is created via the existing spawn paths with `title:
"AGE-14 <title>"` and the new `issue_id` set, the issue moves to
`in_progress`, and the UI focuses the new run — same post-create behavior as
`createAgent` (`store/runs.tsx:59`).

### Status automation (the point of building this in)

Automation is **forward-only** — it never demotes an issue the user manually
advanced, with one deliberate exception (abandonment):

| Event | Effect on linked issue |
|---|---|
| Run created with `issue_id` | → `in_progress` (if currently before it) |
| `create_pr` succeeds for the run | → `in_review` (if currently before it) |
| `merge_task` returns `Clean` | → `done` |
| Run discarded **or** archived unmerged, and it was the issue's last non-archived linked run | → back to `todo` (only from `in_progress`/`in_review`) |

Manual moves are always allowed and win: e.g. moving an issue to `done` by
hand is fine even with no run attached.

**Decided:** issue and run lifecycles stay independent in v1 — closing an
issue never archives its runs, and archiving/restoring runs never touches a
`done` issue.

### All-issues overview (home)

Like the agents overview, there is a cross-project **Issues view on the home
screen**: `HomeView` gains a segmented toggle (**Agents | Issues**). Issues
mode lists every project (same headers, color accents, and fold state as the
agents mode) with that project's open issues beneath, sorted by status then
priority — `done`/`cancelled` are omitted here. Clicking an issue opens that
project's Issues tab with the issue's detail pane; clicking a project header
selects the project as today.

No new backend is needed: it fetches `list_issues(project.id)` per project,
the same per-project loop `HomeView` already runs for `listRuns`.

## Data model

New table, created in the `execute_batch` block of `Registry::open`
(`registry.rs:91`):

```sql
CREATE TABLE IF NOT EXISTS issues (
    id         TEXT PRIMARY KEY,          -- uuid, same as projects/runs
    project_id TEXT NOT NULL,
    seq        INTEGER NOT NULL,          -- per-project number (AGE-14)
    title      TEXT NOT NULL,
    body       TEXT NOT NULL DEFAULT '',
    status     TEXT NOT NULL DEFAULT 'todo',
    priority   INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (project_id, seq)
);
```

`seq` is allocated as `1 + MAX(seq)` for the project inside the insert
transaction. Numbers are never reused; deleting an issue leaves a gap (fine —
Linear does too).

Two column migrations, following the existing pattern (`registry.rs:141`):

```sql
ALTER TABLE runs ADD COLUMN issue_id TEXT;   -- nullable; many runs may share one issue (races, retries)
ALTER TABLE projects ADD COLUMN issue_key TEXT;  -- 3-letter key; backfilled for existing projects
```

The run→issue direction (a nullable FK on `runs`) is chosen over an
`issue.run_id` because races and retries mean **many runs per issue**; the
issue's "linked runs" list is `SELECT ... FROM runs WHERE issue_id = ?`.

Rust types in `agency-core`:

```rust
pub struct Issue {
    pub id: String,
    pub project_id: String,
    pub seq: i64,
    pub title: String,
    pub body: String,
    pub status: IssueStatus,   // Backlog | Todo | InProgress | InReview | Done | Cancelled
    pub priority: u8,          // 0..=4
    pub created_at: i64,
    pub updated_at: i64,
}
```

Registry methods: `create_issue`, `get_issue`, `list_issues(project_id)`
(all statuses; the UI groups/collapses), `update_issue` (title/body/
status/priority — bumps `updated_at`), `delete_issue`, and
`runs_for_issue(issue_id)`. Status changes route through one
`set_issue_status` that encodes the forward-only rule so the automation hooks
can't fight manual moves. Key derivation + backfill live beside
`pick_project_color` / `backfill_project_colors` (`registry.rs:182-237`),
and `Project` gains `issue_key: Option<String>`.

## Backend surface (Tauri commands)

New commands in `commands.rs`, mirrored in `ui/src/api.ts`:

```
list_issues(project_id)                          -> Issue[]
create_issue(project_id, title, body, status)    -> Issue
update_issue(id, { title?, body?, status?, priority? }) -> Issue
delete_issue(id)                                 -> ()
start_issue_run(issue_id, agent, base?, merge_target?)          -> RunInfo
start_issue_race(issue_id, agents, base?, merge_target?)        -> RunInfo[]
start_issue_loop(issue_id, agent, check_command, max_attempts, base?, merge_target?) -> RunInfo
```

The three `start_issue_*` commands are thin wrappers living in `state.rs`
beside `create_run_from_issue`: each composes the issue prompt + run title in
one place, sets `runs.issue_id`, advances the issue to `in_progress`, and
delegates to the existing `create_run_spec` / `create_race` / `create_loop`
paths (base defaults to `merge::detect_base`, like the GitHub flavor).

Hook edits (each a few lines, guarded on `run.issue_id`):

- `merge_task` (`state.rs:2376`): on `MergeOutcome::Clean`, set the linked
  issue `done`.
- `create_pr` (`state.rs:2415`): on success, advance to `in_review`.
- `discard_run` (`state.rs:1518`) and `archive_run` (`state.rs:1550`): if no
  other non-archived run links the issue and its status is
  `in_progress`/`in_review`, roll back to `todo`.

`delete_project` should also delete the project's issues (and
`discard_run`/`delete` paths leave `runs.issue_id` dangling-safe: lookups are
by id and tolerate a missing issue).

## UI

- **Store** (`store/runs.tsx`): extend `Tab` to
  `"agents" | "source" | "files" | "issues"`. Issues state lives in a small
  `useIssues(projectId)` hook (fetch on mount + refresh after every mutation +
  piggyback the existing 1.5 s poll cadence while the Issues tab is active) —
  the run store stays focused on runs.
- **`AgentsView.tsx`**: add the tab button (`▧ Issues`) and render
  `IssuesView` for `tab === "issues"`.
- **`AgentAddMenu.tsx`**: gains an optional issue context (e.g.
  `issue?: Issue`) that hides the terminal + GitHub entries, routes spawn/
  race/loop through the `start_issue_*` commands, and pre-fills the
  Race/Loop dialogs' prompt from the issue. Same component, both contexts —
  the existing "call sites cannot drift" rule extends to issues.
- **New components**:
  - `IssuesView.tsx` — quick-add input, status-grouped list, empty state
    ("Capture your first issue — you can hand it to an agent later").
  - `IssueRow.tsx` — `AGE-14`, priority glyph, title, linked-run activity
    dot (reusing run status colors from the tiles), hover actions: **Start
    agent** (default agent on click; caret opens the add-menu), status pill,
    overflow menu (priority, cancel, delete — delete behind
    `ConfirmDialog`).
  - `IssueDetail.tsx` — the detail pane described above.
- **`HomeView.tsx`**: the Agents | Issues segmented toggle; Issues mode
  reuses the project grouping/fold logic with `IssueRow`s (read-mostly:
  click-through to the project's Issues tab; no quick-add here in v1).
- **Command palette** (`CommandPalette.tsx`): "New Issue" action; opens the
  Issues tab with quick-add focused.
- **Keyboard** (v1, Linear-flavored but minimal): `c` in the Issues view
  focuses quick-add; `↑/↓` move selection; `Enter` opens detail; `1–4`/`0`
  set priority on the selected issue.

## Non-goals (v1)

- GitHub Issues sync or import (explicitly deferred; the existing
  `create_run_from_issue` GitHub flow remains as-is, untouched).
- Labels, cycles/sprints, projects-within-projects, sub-issues, comments,
  attachments, due dates.
- Human assignees — the only "assignee" is an agent run.
- Custom statuses/workflows.
- Manual ordering within a status group (no `sort_order` column yet; the
  migration pattern makes adding one later trivial).
- Search/filtering beyond the status grouping.
- Cross-project issues — an issue belongs to exactly one project, like an
  agent. The home overview aggregates per-project lists; it is not a shared
  pool.

## Future features

Documented so v1 decisions don't preclude them:

- **Kanban board presentation** — status columns over the same table;
  drag-and-drop becomes the status-change gesture. Would introduce a
  `sort_order` column for within-column ordering.
- **Quick-add / editing from the home overview** — v1's home Issues mode is
  navigate-only.
- **GitHub Issues sync** — local-first with optional two-way sync; the
  `issues` table would gain a `remote_url`/`remote_number`.
- **Palette-driven issue search** — fuzzy-jump to an issue from
  `CommandPalette`.

## Suggested build order

1. Core: `issues` table + `runs.issue_id` + `projects.issue_key` migrations
   and key backfill, `Issue` type, registry CRUD + `set_issue_status`, unit
   tests beside the existing registry tests (`registry.rs:680+`).
2. Commands: the seven Tauri commands + `api.ts` bindings.
3. Dispatch: `start_issue_run` / `start_issue_race` / `start_issue_loop` +
   the three lifecycle hooks (merge / PR / discard-archive).
4. UI: Issues tab, `IssuesView` + `IssueRow` + quick-add, `IssueDetail`,
   and the `AgentAddMenu` issue context.
5. Home overview: the Agents | Issues toggle in `HomeView`.
6. Polish: command-palette action, keyboard, empty states.

Steps 1–3 are independently testable before any UI exists; step 4 is where
the Linear-inspired look lands.
