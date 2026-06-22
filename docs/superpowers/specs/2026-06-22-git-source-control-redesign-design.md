# Git Source-Control Redesign — Design

**Date:** 2026-06-22
**Status:** Approved

## Problem

The app has two git surfaces that duplicate each other and both fall short of VSCode:

- **`SourceControl.tsx`** (the full "Source Control" tab): commit box, staged/changes lists with file-level stage/unstage, a weak "History" section (project pills → flat `hash + summary` list, not clickable, no dates/authors/diffs), and a hunk-staging diff pane.
- **`GitReviewPanel.tsx`** (the "Review" sidebar next to the agent grid): a stripped-down copy of Source Control's left column — commit box and staged/changes lists, **no diff view and no history at all**.

The two share zero code despite doing nearly the same thing. The Review panel lacks git history entirely. Both are under-featured and visually rough compared to VSCode's Source Control.

## Goal

Rebuild both surfaces as **one shared, feature-rich git component with two layouts**, faithfully copying VSCode's built-in Git/SCM functionality and adapting its design to the app's existing Catppuccin Mocha theme. The Review sidebar gains full history; the duplication is eliminated.

## Decisions (from brainstorming)

- **Architecture:** one shared component, two layouts (compact sidebar / full tab).
- **Feature scope (all in):** side-by-side diff, syntax highlighting, line/selection-level staging, commit graph.
- **History scope:** the focused run's branch with commits ahead of base highlighted, drawn back into the base/main line so the branch point is visible (full graph).
- **Libraries:** Shiki for highlighting (VSCode's own engine + official Catppuccin theme); reimplement VSCode's swimlane graph algorithm directly (no `@gitgraph`/Mermaid); custom diff component fed by a client-side diff model using `jsdiff` for intra-line word diffs.

## Tauri integration note

The frontend (`ui/`) is a standard React + Vite SPA running in the Tauri webview. Rust (`crates/agency-app`, `crates/agency-core`) exposes `#[tauri::command]` functions called from JS via `invoke()` (see `api.ts`). New React components, Shiki, and jsdiff are ordinary frontend work bundled by Vite — Rust is unaware of them. Rust grows only to surface richer git data and missing write actions. The graph, diff parsing, and highlighting all run in the webview.

## 1. Component architecture

New folder `ui/src/components/git/`. One container, `GitPanel`, drives both surfaces via a `layout` prop.

```
GitPanel({ taskId, layout: "compact" | "full" })   ← owns shared state
├─ BranchBar          branch name · ahead/behind · sync/publish (full only)
├─ ChangesPanel
│   ├─ CommitBox       textarea + action-button dropdown
│   └─ ResourceGroup   ×N  (Merge / Staged / Changes / Untracked)
│       └─ FileRow     status letter+color · path · hover actions
├─ HistoryPanel
│   ├─ CommitRow       Graph svg · subject · refs · author · rel-date
│   └─ CommitDetail    message + changed-file list → diffs
└─ DiffViewer          toolbar + side-by-side/inline diff
```

Supporting non-component modules in the same folder:

- `graph.ts` — swimlane lane-assignment + SVG path geometry (port of VSCode `scmHistory.ts`).
- `diffModel.ts` — parse a unified diff into aligned rows; intra-line word diff via `jsdiff`.
- `highlight.ts` — Shiki singleton (Catppuccin Mocha theme), tokenize-by-language, ext→lang map.
- `status.ts` — status-code → letter/color/group mapping, plus group partitioning.

**Removed:** `SourceControl.tsx`, `GitReviewPanel.tsx`, `DiffView.tsx`. `AgentsView` renders `<GitPanel layout="compact">` in the sidebar and `<GitPanel layout="full">` in the Source Control tab.

## 2. Backend additions (`agency-core/src/git.rs` → `commands.rs` → `api.ts`)

The graph, diff parsing, and highlighting are pure frontend. Rust grows only to surface richer git data and the missing write actions:

| New Rust fn | git invocation | Purpose |
|---|---|---|
| `stage_all` / `unstage_all` | `add -A` / `reset` | group "stage/unstage all" |
| `discard(path, untracked)` | `restore -- path` or `clean -f -- path` | discard one file |
| `discard_all` | `restore -- .` (+ optional `clean`) | discard-all per group |
| `stage_lines` / `unstage_lines` / `revert_lines` | construct partial patch → `apply --cached [--reverse]` / `apply --reverse` | line/range staging + gutter revert |
| `log_graph(limit)` | `log --format=…%H%P%an%ae%at%s%D…` (unit-separated) | history: hash, parents, author, email, date, subject, refs |
| `commit_files(hash)` | `diff-tree --no-commit-id --name-status -r` | files changed in a commit |
| `commit_diff(hash, path)` | `show hash -- path` (root-commit safe) | diff for a file within a commit |
| `branch_info` | `rev-parse --abbrev-ref HEAD`, `rev-list --left-right --count @{u}...HEAD`, `merge-base HEAD <default>` | branch, upstream, ahead/behind, base |
| `commit_amend(msg)` | `commit --amend -m` | amend from action-button dropdown |

New TS types in `api.ts`: `HistoryItem { hash, parents[], author, email, date, subject, refs: Ref[] }`, `Ref { name, kind }`, `BranchInfo { branch, upstream, ahead, behind, base }`, `CommitFile { path, status }`. Each new command gets a one-line `invoke` wrapper. `stage_hunk`/`unstage_hunk` stay (the line-range patch builder generalizes them). `projectLog` + the project-pills concept are removed.

Base detection: `merge-base HEAD <default branch>` of the project. (Alternative considered: persist the run's base ref explicitly — deferred unless merge-base proves unreliable.)

## 3. Changes view — VSCode parity

**Resource groups**, VSCode order, hide-when-empty except "Changes":
`Merge Changes` · `Staged Changes` · `Changes` · `Untracked Changes`. Each header shows a count badge and hover toolbar actions (stage-all / unstage-all / discard-all as appropriate), and is collapsible.

**File rows** — status letter + color, path with directory dimmed and filename bright, hover-revealed inline actions: open file, stage (`+`) / unstage (`−`), discard (`↩`, with confirm).

Decoration mapping (VSCode token → app CSS var):

| Status | Letter | → app var |
|---|:--:|---|
| Modified | M | `--yellow` |
| Added | A | `--green` |
| Deleted | D | `--red` |
| Renamed/Copied | R/C | `--teal` |
| Untracked | U | `--green` |
| Conflict | ! | `--peach` |
| Ignored | I | `--o0` |

**Commit box** — textarea, branch indicator, and a split action button: primary **Commit**, dropdown **Commit & Push** / **Commit (Amend)**. With no upstream it becomes **Publish Branch**; when ahead/behind it offers **Sync**.

## 4. Diff viewer

Custom component fed by `diffModel.ts`, which parses the unified diff from `git_diff` into **aligned rows** (old line | new line, with gaps for inserts/deletes).

- **Side-by-side by default, inline toggle** — mirrors VSCode `renderSideBySide`, auto-collapsing to inline below a width breakpoint (compact sidebar uses inline automatically).
- **Intra-line word highlighting** — `jsdiff` computes char/word ranges between paired lines; changed spans get the saturated overlay (`--red`/`--green` higher alpha) over a line wash (lower alpha), VSCode's two-layer scheme.
- **Syntax highlighting** — Shiki tokenizes each line by language (from file extension), rendered under the diff overlays.
- **Gutter actions** — per-change stage/revert arrows (`stage_lines` / `revert_lines`), plus **select a line range and stage/revert the selection**. Whole-hunk staging stays as a hunk-header button. Historical-commit diffs render read-only (no staging gutter).

## 5. History + commit graph

`HistoryPanel` calls `log_graph` for the focused run's branch. Shows the branch with commits ahead of base highlighted, drawn back into the base/main line.

- **`graph.ts`** ports VSCode's single-pass swimlane algorithm: each lane is identified by the commit hash it points toward; row *N* input lanes = row *N−1* output; the first parent continues a commit's column (keeping color), extra parents append new lanes, converging lanes collapse. Colors cycle a 5-entry palette (`--peach`, `--pink`, `--yellow`, `--teal`, `--mauve`); ref-bearing commits use ref colors (`--blue` local/HEAD, `--mauve` remote, `--peach` base). Geometry: VSCode's 11×22px lanes, radius-5 curves, radius-4 nodes, one inline `<svg>` per row.
- **`CommitRow`** — graph cell, subject, ref badges, author initials, relative date, short hash. Commits ahead of base get a subtle accent; base/main commits render dimmer.
- **`CommitDetail`** — selecting a commit shows full message/author/date and the `commit_files` list; clicking a file loads `commit_diff` into the `DiffViewer` (read-only).

This is the heaviest piece; the swimlane port is the main risk, covered by unit tests (§8).

## 6. The two layouts + theming

**Compact (sidebar, Agents view):** single resizable column — `CommitBox`, resource groups, then a collapsible **History accordion** (graph + subject + short hash, condensed). Diffs use inline mode; clicking a working file opens its diff in an expandable region, and a "maximize" affordance jumps to the full tab on that file.

**Full (Source Control tab):** `BranchBar` across the top, then two resizable panes — left with **[Changes | History]** tabs (+ commit box under Changes), right showing either `DiffViewer` (working file selected) or `CommitDetail` (commit selected).

**Theming** reuses the existing CSS-variable system: diff line/text washes from `--green`/`--red` at layered alpha, graph palette and decorations as tabled above. New rules go in `styles.css` under a `git-*` namespace; stale `git-status-*`/`sc-*`/`hist-*` rules are replaced.

## 7. Data flow, refresh, errors

`GitPanel` loads status + branch info on mount, after every action, on a manual refresh button, and on a light interval (~2s) **only while the surface is visible**. Errors render in the existing `git-error` style. Actions follow the current `act()` wrapper pattern (run → clear/set error → refresh). No new global store — `GitPanel` owns its state; both layouts read it.

## 8. Testing

- **`graph.ts`** — unit tests on hand-built commit DAGs (linear, branch, merge, octopus) asserting input/output swimlanes per row, ported from VSCode's graph test cases. De-risks the hardest piece.
- **`diffModel.ts`** — fixture unified-diffs → expected aligned rows + word-diff spans.
- **`status.ts`** — status code → letter/color/group mapping table.
- **Rust** — extend `crates/agency-core/tests/git.rs`: `log_graph` parsing (parents/refs), `commit_files`, `branch_info` ahead/behind, `stage_lines` patch round-trip.
- Existing Vitest/component test conventions preserved.

## 9. Suggested phasing

1. Backend commands + TS types.
2. `status.ts` + Changes view (groups, file rows, commit box).
3. `DiffViewer` (+ Shiki/jsdiff, diffModel, line staging).
4. History + graph (`graph.ts`, CommitRow, CommitDetail).
5. Wire the two layouts into `AgentsView`; remove old files.
6. Polish + theming.

Each phase is independently testable.
