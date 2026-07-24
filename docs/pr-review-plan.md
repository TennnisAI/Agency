# In-app GitHub PR review

## Context

Today, reviewing a PR that an Agency agent produced means leaving the app for
github.com. The "review comments" that exist in-app (`ReviewComments.tsx`,
`review_comments` table) are a *local* mechanism — they get typed into the
agent's terminal, not posted to GitHub. There is no way to read a PR's diff,
read reviewer threads, comment, or approve/request-changes from inside Agency.

This plan adds a **real GitHub PR review** surface: view a PR's diff +
description + existing threads, add inline comments, reply into threads,
resolve/unresolve threads, and submit the review as Approve / Request changes /
Comment — all via the `gh` CLI (consistent with how all GitHub access already
works here; auth stays in gh's keychain, no tokens in Agency).

Per product decisions:
- **Where it lives:** inside the existing **Source Control** tab, as a new
  sub-tab switcher **Changes | Pull Requests** (not a new top-level tab). As
  part of this, the branch bar (which is Changes-specific) stops spanning the
  whole app and is confined to the Changes/left pane.
- **A "Review in Agency" button** on the agent's own PR in the Approve window
  (`PrSection.tsx`) deep-links into the Pull Requests sub-tab for that PR.
- **v1 scope is the full feature:** diff + description + threads, add inline
  comments, reply, resolve/unresolve, submit review with a verdict.

This builds on the two bug fixes already made in this branch (PR-section
layout-shift + the `openUrl` capability scope). Not yet committed.

---

## Backend (Rust + `gh`)

All new GitHub access lives in `crates/agency-core/src/gh.rs` (`GhCli`), each
method taking `repo: &Path`. **Canonical identity is `(projectId, prNumber)`**
for every command; run-based entry points resolve the number once via the
existing `view_pr(repo, branch)`.

**owner/repo helper** — `fn repo_slug(&self, repo) -> Result<(String,String)>`
via `gh repo view --json nameWithOwner -q .nameWithOwner`, split on `/`. Used for
explicit REST path interpolation (`repos/{owner}/{repo}/…`) and GraphQL vars
(deterministic + trivial to fake; don't rely on gh's `{owner}` placeholders).

**PR detail** — `view_pr_detail(repo, number) -> Result<Option<PrDetail>>` via
`gh pr view <n> --json number,url,title,state,isDraft,baseRefName,headRefName,body,headRefOid,mergeable,reviewDecision,author,createdAt,updatedAt`
(same no-PR→`None` handling as `view_pr_by_number`). New struct `PrDetail`
(camelCase) adds `body`, `head_ref_oid` (head SHA → review `commit_id`),
`mergeable`, `review_decision: Option<String>` (**JSON null when undecided — must
be Option; `#[serde(default)] String` won't absorb an explicit null**), and a
nested `Author { login }`. Keep existing `PrInfo` for lists/status.

**Full diff** — `pr_diff(repo, number) -> Result<Vec<PrFileDiff>>` via
`gh pr diff <n>` (captured with `.output()` → uncolored, unpaged; no extra
flags). Splitting is a **pure fn** `parse_pr_diff(&str) -> Vec<PrFileDiff>`:
each `diff --git a/X b/Y` starts a file; lines up to the first `@@` are its
`header`; each hunk reuses `crate::git::Hunk` and keeps its literal
`@@ -a,b +c,d @@` header (the FE's `diffModel.parseStarts` derives line numbers
from it — same shape the existing renderer consumes). Handles rename/delete/add
(`/dev/null`) and binary (`binary=true, hunks=[]`). `PrFileDiff` = `FileDiff` +
`{ path, old_path: Option, binary }`.

**Existing threads** — `pr_review_threads(repo, number) -> Result<Vec<ReviewThread>>`
via **GraphQL** (`gh api graphql -f owner= -f name= -F number= -f query='…
reviewThreads(first:100){ nodes{ id isResolved isOutdated comments(first:100){
nodes{ id databaseId path line originalLine diffHunk body createdAt author{login}
replyTo{id} }}}}'`). GraphQL is the single source (REST `/comments` lacks
resolution state + thread grouping). Private envelope structs (nested `nodes`)
map into public flat `ReviewThread { id, is_resolved, is_outdated, comments }` and
`PrReviewComment { id (node), database_id (REST, for replies), path, line: Option,
original_line: Option, diff_hunk, body, author, created_at, in_reply_to_id }`.
**`number` must be `-F`** (typed Int!). v1 caps at 100/100 with a `// TODO
pageInfo` note. (Name it `PrReviewComment` — `registry::ReviewComment` is the
unrelated local system.)

**Submit review (atomic)** — `submit_pr_review(repo, number, commit_id, event,
body: Option, comments: &[DraftComment])`. The reviews API body is **nested +
snake_case** (unlike everything else here) → build with `serde_json::json!` and
POST via a new stdin helper `run_stdin(repo, args, body)` (mirrors
`git::git_stdin`): `gh api --method POST repos/{o}/{r}/pulls/{n}/reviews --input -`.
`DraftComment` (camelCase, from FE): `{ path, body, line, side ("RIGHT"|"LEFT"),
start_line: Option, start_side: Option }`. **`commit_id` = live head SHA — fetch
server-side in `state::submit_pr_review` via `view_pr_detail`, don't trust the
FE.** Validate: REQUEST_CHANGES/COMMENT need a body or ≥1 comment; omit
`start_line`/`start_side` when single-line (GitHub rejects `start_line == line`).

**Reply** — `reply_review_comment(repo, number, in_reply_to: u64, body)` via
`gh api --method POST repos/{o}/{r}/pulls/{n}/comments -f body= -F in_reply_to=`
(`-F` int; `in_reply_to` is a comment's `database_id`). Returns the created
`PrReviewComment` for optimistic append.

**Resolve / unresolve** — `resolve_review_thread(repo, thread_id)` /
`unresolve_review_thread(...)` via GraphQL mutations
(`resolveReviewThread`/`unresolveReviewThread`, `-f threadId=` = the **node id**,
not database_id).

**Line/side anchoring** (shared with the FE, §"Comment anchoring" below): per
row, `kind=="del"` → `LEFT/oldNo`, else (`add`/`ctx`) → `RIGHT/newNo`. Single
row → `{line,side}`. Multi-row same-side → `start_*`=first, `line/side`=last
(ordering holds). Mixed-side selection → clamp to RIGHT using first/last rows
with non-null `newNo`. The FE computes the clamp; Rust forwards.

**state.rs** methods (resolve `repo = self.project_repo(project_id)?`):
`pr_detail`, `pr_diff`, `pr_review_threads`, `submit_pr_review` (fetches head SHA
internally), `reply_pr_comment`, `resolve_pr_thread`, `unresolve_pr_thread`, and
`pr_number_for_run(run_id)` (→ `view_pr(repo, branch).number`).

**commands.rs**: thin `async` `#[tauri::command]` wrappers for each (existing PR
commands are already `async`), registered in the `invoke_handler` list in
`lib.rs`. Rust snake_case params (`in_reply_to`, `thread_id`) ↔ JS camelCase
(`inReplyTo`, `threadId`), matching the `taskId`/`projectId` convention.

**Tests** — extend the `#[cfg(test)]` block using the existing `fake_gh` helper
(writes a `#!/bin/sh` fake), branching on `$1 $2`: `pr view` (assert
head_ref_oid + `review_decision.is_none()`), `pr diff` (+ a pure `parse_pr_diff`
test on a literal 2-file diff), `api graphql` (envelope→flat mapping), reviews
POST (fake `cat`s stdin → assert the piped JSON has right `event`/`side`/`line`/
`start_line`), reply POST, resolve mutation. FE anchoring clamp gets a vitest
beside `diffModel.test.ts` feeding synthetic `DiffRow[]`.

---

## Frontend

### 1. Restructure Source Control into Changes | Pull Requests

`ui/src/components/AgentsView.tsx` renders `<GitPanel layout="full" .../>` for
`tab === "source"`. Introduce a sub-tab switcher owned at the Source Control
level:

- Add a small segmented header **`Changes | Pull Requests`** at the top of the
  source-control area (full width, thin — like the existing `.seg` groups in
  `AgentsView`), tracked by local state (`srcTab: "changes" | "prs"`).
- **Changes** = the current `GitPanel` content, but with the **branch bar moved
  inside the left/changes pane** instead of spanning full width. In
  `GitPanel.tsx` the `full` layout currently renders `{branchBar}` above
  `git-full-body` (line ~217); move `branchBar` into the `git-full-left` column
  (above `sections`) so it only spans the changes pane, per the sizing note.
  The `compact` layout already scopes the branch bar to its narrow width — no
  change there.
- **Pull Requests** = a new `PrReviewPanel` (below).

The switcher can live in `GitPanel` (add a `layout="full"` internal tab) or in
`AgentsView` wrapping two children. Recommended: keep `GitPanel` focused on
changes and add the switcher in `AgentsView`'s `tab === "source"` block, so
`GitPanel` only handles Changes and `PrReviewPanel` is a sibling. The branch-bar
move happens inside `GitPanel` regardless.

### 2. `PrReviewPanel` — PR list + review view

New `ui/src/components/pr/PrReviewPanel.tsx` (+ children). Two-pane, matching the
`git-full-body` left/right pattern:

- **Left:** open PRs for the project via `listGhPrs(projectId)` (already exists),
  each row showing `#num title state check-glyph` — reuse the row styling from
  `GhImportDialog.tsx`. Selecting a PR (or arriving via deep-link) loads it.
- **Right:** the selected PR's review view (`PrReview` component):
  - **Header:** title, `#num`, state/draft badge, base←head, mergeable hint,
    check rollup (reuse `pr_status` checks), and an **Open ↗** to github.
  - **Description:** the PR body rendered as markdown. Extract a small reusable
    `<Markdown>` from the `marked`-based `markdownSrcDoc` in `FileEditor.tsx`
    (or render with `marked.parse` into a sanitized container). Used for the
    body and every comment body.
  - **Files + diff:** split `pr_diff` into per-file `FileDiff`s and render each
    with a **PR-flavored diff viewer**. Reuse the pure pieces of the existing
    renderer — `diffModel.buildRows`, `highlight.ts` — but author a
    `PrDiffFile` component (a trimmed sibling of `DiffViewer` without the local
    staging/revert/hunk actions). Line selection reuses `DiffViewer`'s
    `toggleLine` + `selectedRange` logic; on "Comment" it opens an inline draft
    keyed to the computed **(path, line, side)** anchor.
  - **Threads inline:** render each existing `ReviewThread` under its anchored
    line (path + line/side match), with author, markdown body, replies, a reply
    box, and a Resolve/Unresolve toggle. Unanchored/outdated threads (line no
    longer in the diff) render in a per-file "Outdated / resolved" list.
  - **Draft comments:** new inline comments accumulate in local component state
    as `{ path, line, side, startLine?, startSide?, body }`. They are NOT posted
    until the review is submitted.
  - **Submit bar (sticky footer):** a summary textarea + three actions
    **Approve · Request changes · Comment**. Submitting calls `submitReview`
    with the draft comments + event + body in one atomic request, then reloads
    threads and clears drafts.

### 3. Comment line/side anchoring (frontend)

The diff rows already carry `oldNo`/`newNo` per row (`diffModel.DiffRow`). For a
selected range within one file's hunks:
- An **added or context** line → `side: "RIGHT"`, `line: newNo`.
- A **deleted** line → `side: "LEFT"`, `line: oldNo`.
- Multi-line selection → `line`/`side` = the last (highest) selected line;
  `start_line`/`start_side` = the first. Both endpoints must be the same side
  for GitHub to accept a multi-line comment; if a selection mixes sides, clamp
  to RIGHT (the added side) or block with a hint.

This mirrors `DiffViewer.selectedRange()` but emits GitHub anchors instead of
local `(hunk, lineIndex)`.

### 4. Deep-link from the Approve window

In `PrSection.tsx`, when a PR exists, add **"Review in Agency"** next to
**Open ↗**. It switches the app to the Source Control tab, Pull Requests
sub-tab, preselecting this PR number. Wire via the runs store (add a light
`openPrReview(projectId, prNumber)` action / an event, consistent with how
`setTab`/`setView` already drive navigation) so the modal can close and the
panel opens focused on the PR.

---

## Files touched (representative)

**Rust**
- `crates/agency-core/src/gh.rs` — new methods + structs + fake-gh tests.
- `crates/agency-app/src/state.rs` — PR-review state methods keyed by
  `(projectId, prNumber)` + run→prNumber helper.
- `crates/agency-app/src/commands.rs` — `#[tauri::command]` wrappers.
- `crates/agency-app/src/lib.rs` — register the new commands in the handler.

**Frontend**
- `ui/src/api.ts` — bindings + types (`PrDetail`, `ReviewThread`, etc.).
- `ui/src/components/AgentsView.tsx` — Changes|Pull Requests switcher.
- `ui/src/components/git/GitPanel.tsx` — move branch bar into the changes pane.
- `ui/src/components/pr/PrReviewPanel.tsx`, `PrReview.tsx`, `PrDiffFile.tsx`,
  `PrThread.tsx` — new.
- `ui/src/components/Markdown.tsx` — new reusable renderer (extract from
  `FileEditor.tsx`).
- `ui/src/components/PrSection.tsx` — "Review in Agency" button.
- `ui/src/store/runs.tsx` — navigation action for the deep-link.
- `ui/src/styles.css` — PR review + sub-tab styles (monochrome glyphs only, no
  emoji, per project rule).

---

## Verification

- **Rust:** `cargo test -p agency-core` (fake-`gh` unit tests for the new
  methods — diff parse split, thread merge, submit-review payload shape,
  anchoring). `cargo build`.
- **Frontend:** `pnpm --dir ui build` (tsc + vite). Use the direct
  `node_modules/.bin` binaries if pnpm-exec misbehaves (known gotcha); this
  worktree currently has no `ui/node_modules`, so `pnpm --dir ui install` first.
- **End-to-end (real gh):** with `gh` authed on a repo that has an open PR:
  1. Source Control → **Pull Requests** → the PR list loads; branch bar now sits
     only over the Changes pane.
  2. Open a PR → diff + description + existing threads render.
  3. Select lines → add an inline comment; add a summary; **Request changes** →
     confirm on github.com the review + inline comments posted.
  4. Reply into an existing thread; **Resolve** it → confirm state on GitHub.
  5. From an agent's Approve window → **Review in Agency** opens that PR focused.
- Confirm the capability/`openUrl` fix (already in this branch) lets **Open ↗**
  reach github.com (needs the app rebuilt, since capabilities compile in).
