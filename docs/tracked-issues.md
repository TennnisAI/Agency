# Syncing `.agency/issues/` — across machines, and across a team

Status: **built and usable** (2026-09-02), manual sync only. The `uid`
groundwork, the merge, the ref transport, the setting and the UI are in and
tested. Nothing syncs on its own; every pass is something the user asks for.
This doc supersedes the earlier version, which asked a narrower question and got
a narrower answer.

The question this now answers: the tracker is local files, so a person with
three machines has three different backlogs, and a team has none. What makes
both work without being two features?

Recommendation up front: **one mechanism, not a solo mode and a team mode.**
Sync the whole issue file, over a dedicated git ref that is never checked out,
with a per-project setting that chooses *where* it syncs rather than *what* it
syncs.

---

## What is true today

Canonical issue storage is one markdown file per issue under
`.agency/issues/AGE-14.md` — YAML frontmatter, H1 title, markdown body
(`issuefs.rs:1`). SQLite is a rebuildable index over those files, not the source
of truth, and `reconcile` (`issuefs.rs:867`) makes the index follow the files.

Those files were tracked by git once. There is a committed, wired one-shot
migration that untracks them:

- `ensure_agency_excludes` (`worktree.rs:376`) writes `.agency/worktrees/`,
  `.agency/agency.local.toml`, `.agency/issues/` and `.agency/records/` into
  **`.git/info/exclude`**, and strips any legacy blanket `.agency/` line. Note
  what it deliberately leaves visible to git: `.agency/agency.toml`, the
  project's shared config.
- `untrack_issue_files` (`worktree.rs:451`) runs `git rm -r --cached` over the
  issues dir and **commits** the removal (staged deletions being exactly the
  dirt it is trying to eliminate). It refuses over a merge in progress or a
  staged index, and logs a warning if a repo's own `.gitignore` un-ignores the
  directory and outranks the exclude file.
- Both are driven from `state.rs:3369` (`ensure_issues_untracked`), gated on a
  per-project setting `issues_untracked:{project_id}`, called at the top of
  every issue-touching path.

So "untracked issues" is not a property of this checkout. It is **product
behavior applied to every project Agency manages**, and it actively reverses
itself if it finds tracked issue files.

## Why the files left the working tree — and why that still holds

Recorded in `issuefs.rs:28` and `worktree.rs:451`. Three reasons:

**1. Churn.** The app rewrites frontmatter on every status change — every
dispatch, every merge, every board drag. Tracked, that leaves the main checkout
permanently dirty, and a merge refuses to start on a dirty checkout. Tracking
issues therefore intermittently disabled merging agent branches.

**2. Conflicts.** Agent branches write the same frontmatter lines the main
checkout writes, so every agent merge carried the same trivial conflict on both
sides.

**3. Branch-locality.** Issue state is **workspace-global**: `.agency/issues/`
lives in the project's own checkout and every worktree reads and writes that
single shared copy. Git content is **branch-local**: tracked, each worktree gets
its own copy at its own commit. "What status is AGE-14" stops having one answer.
A backlog that forks per feature branch and merges by three-way text diff is not
a tracker.

All three are still true, and nothing below proposes putting the files back.

**But all three have the same root cause, and it is not git.** It is the files
being *in the working tree of a branch* that gets checked out, branched, and
merged. Bytes stored in git but never materialized into a working tree cause
none of them. That distinction is the whole of what follows.

## The correction: the axis is ownership, not volatility

The earlier version of this doc recommended splitting the issue file by
**volatility** — track `key`, title, body, `created`; keep `status`, `rank`,
`due`, `scheduled`, `updated` local and gitignored. That was a sound answer to
the question it was asked, which was *should a teammate see the backlog*.

It is the wrong answer for a person with several machines, and it is worth
saying plainly why, because the failure is not obvious: the fields it keeps
local are exactly the ones you most want to travel. "What is in progress" is the
first thing you look at on the second laptop. A split-file design would sync
your issue *text* and let your board diverge silently — worse than today,
because it looks synced.

Volatility was the right axis only while the working tree was the constraint.
Move the tracker to a ref that is never checked out and volatility stops
mattering entirely: nothing is dirtied, nothing conflicts with an agent branch,
nothing forks per branch. Write to it a hundred times a day.

What survives is a different axis — **ownership**: is this a fact about the
issue, or about one person's plan? Sorted that way, `status`, `priority` and
`due` land on the shared side for *both* audiences. A team wants shared status
because a tracker whose status is private is not a tracker; you want shared
status because it is still you on the other machine. The only genuinely
person-scoped candidates are `rank` (board order) and `scheduled`, and a shared
ordering is the norm in the tools people already use.

So **solo-across-machines is the degenerate case of a team of one.** Same
fields, same code path. What differs is deployment: where it pushes, and how
often a genuine concurrent edit actually happens.

**Decision: v1 syncs the whole file. No field split.** It is what makes the solo
case work at all, it is defensible for teams, and it avoids the hazard the
split-file design carried (once half the fields come from a sidecar, `reconcile`
has to merge two sources and invent defaults for an issue whose text arrived
without a local row). Deferring is safe: adding a per-person overlay later
*removes* fields from the synced set, and the unknown-key passthrough already
absorbs that skew in both directions. Do it if team use actually produces board
thrash — as an additive layer, not as a second mode.

## The design: a ref as transport, not as storage

Keep `.agency/issues/*.md` on disk exactly as they are — canonical, untracked,
`issuefs.rs` unchanged. Add a sync step that snapshots the directory into
`refs/agency/issues` and pushes it.

The earlier version listed this under alternatives and deferred it on two costs.
Both were overstated:

- **"Significantly more machinery."** That assumed the model where the ref *is*
  the storage and issues are read out of git objects. Not needed. Snapshot into
  a temp index (`GIT_INDEX_FILE`), `write-tree`, `commit-tree`, `update-ref`,
  push. The real index is never touched, so the working tree never goes dirty
  and `untrack_issue_files` never has anything to remove — the two systems do
  not interact at all, which is what makes it cheap. It is the same shell-out
  shape as everything in `git.rs`.
- **"Gives up plain markdown readable on GitHub."** Measured against the wrong
  baseline. The files are not in the repo today, so they are *already* not
  readable there, and attachment links already do not resolve. That is a cost
  against the split-file design, not against the status quo.

Two properties fall out for free and are worth naming, because they are the
reason this shape beats syncing the directory through a file-sync service:

- **Deletion needs no tombstone format.** "Present in the merge base, absent
  here" is unambiguous from the ref's own history. A file-sync service cannot
  tell deletion from never-had-it, so every sync resurrects deleted issues.
- **Concurrent edits merge instead of forking.** A sync service writes
  `AGE-14 (conflicted).md`, which `read_issue_dir` skips (its frontmatter `key`
  will not match its filename) — the issue silently vanishes from the board.

### Merging

Three-way per issue against the last synced commit, not git's textual merge:

- Frontmatter is a flat key set, so **merge field by field**: status changed on
  the laptop, `due` changed on the desktop, both land.
- Comments are already structured `(author, created_at)` sections, so
  **union-merge** them. Losing a comment loses content, not a status flip.
- Body text changed on both sides is the only case that needs a person, and it
  should be surfaced rather than silently resolved.

A solo user simply never reaches the last branch. Same code either way, which is
the test of whether this really is one mechanism.

## The setting

One axis, phrased as the question a user is actually answering: **where does
this project's backlog live?**

| Option | Solo | Team |
| --- | --- | --- |
| **This machine only** (default, today's behavior) | fine | fine |
| **Sync with the repo** (`origin`) | multi-machine sync | shared backlog |
| **Sync to another remote** | public repo, private backlog | public code, private triage |

Every option is viable for both audiences, which is the bar. The third is not a
nicety: this repo is published, and a backlog of product planning should not be.
That is why the sync target is a remote, not a boolean — the cost of getting it
right is a string instead of a bool.

Deliberately **not** offered: "share the text, keep status local." It is the
option that fails the solo case outright, and for a team it makes the board lie.

**Per project, split across the two config files** the way `config.rs:227`
already merges them (`agency.toml` under `agency.local.toml`, local wins):

- *Whether* this project syncs its backlog → tracked `agency.toml`, so the team
  decision travels with the repo and a teammate's clone needs no setup.
- *Which remote* → `agency.local.toml`, so a private tracker URL does not ship
  to people who cannot push to it.

Frame it in the UI as sharing a backlog and say what that means, not as
"git-track `.agency/issues/`", which names the mechanism and hides the
consequence.

## What is built (2026-09-02)

Four pieces, in the order they landed. `issuesync.rs` is the merge and nothing
else — a pure function over three sets of parsed files — and `issueref.rs` is
the transport. Keeping them apart is what lets the judgement calls be tested
without a repository.

**`issuefs.rs` — `uid`.** Below.

**`issuesync.rs` — the merge.** `plan(mode, base, local, remote)` returns writes
and deletes. Field-level rather than whole-file, so a status flipped here and a
due date set there both survive. Comments and links merge as three-way sets, so
a deliberate removal is not undone and an addition from either side lives. The
unknown-key passthrough merges the same way; it exists so a newer schema's
fields survive an older build, and a merge is no place to drop them. A plan
carries only work: when the merge agrees with what is already on disk it is
empty, which is what makes the steady state cheap.

**`issueref.rs` — the transport.** Snapshots `.agency/issues/` into
`refs/agency/issues` through a scratch `GIT_INDEX_FILE`, so the real index and
working tree are never touched. `git add` needs `--force`, because the directory
is deliberately in `.git/info/exclude`. The scratch index path comes from
`rev-parse --absolute-git-dir` rather than `<repo>/.git`, which in a linked
worktree is a file. Commits carry both sides as parents, so the *next* pass has
a merge base; an unchanged tree does not commit at all. Tested end to end
against real repositories pushing through a bare remote, including the property
that neither checkout is ever dirtied and no branch ever carries the issues.

**`config.rs` — the setting.** `[issues] sync` and `[issues] remote`.
`issue_sync_remote()` is the one place to ask. Both fields are *read* from the
tracked file merged under the local one, but only ever *written* to the local
one: Agency has never written a tracked file, and starting here would dirty the
checkout every time the toggle was touched, which is the exact condition this
whole design avoids. A team that wants the decision to travel commits one line
into `agency.toml` by hand, and `load` already merges it.

**The UI.** A per-project Backlog section in Settings, and a Sync button on the
issues toolbar shown only when the project is configured to share and has a
remote. The seeding question comes back as an outcome rather than an error
(`SyncResult::NeedsSeeding`), so the dialog can state both counts without
parsing a message. Conflicts get a marker on the affected row as well as a
toast: the merge decides a field both sides changed by `updated`, a body edit
that lost that way is content quietly gone, and a dismissed toast would be the
last anyone heard of it.

### `uid`

Issue files carry `uid:` — a uuid in the frontmatter, right after `key:`, which
is the id the index row is keyed by. `key` names an issue within one checkout;
`uid` names it across checkouts.

Why it could not wait:

1. **Identity did not survive a copy.** The uuid existed but lived only in
   SQLite, so copying the issues directory to a second machine minted *fresh*
   ids there and `runs.issue_id` linkage was already per-machine.
2. **Keys collide, and without a uid the collision is unresolvable.**
   `alloc_issue_seq` (`registry.rs:1101`) is a per-machine counter, so two
   people filing offline both reach for `AGE-175` and produce two unrelated
   issues with one name — indistinguishable, and impossible to renumber safely
   because a `links: AGE-175` could mean either. Narrow for one person, wide for
   a team.

How it behaves:

- Parsed strictly, uuid shape only, like every other known frontmatter key. A
  hand-typed word is rejected: it would collide with every other file someone
  typed the same word into, which is the failure the field exists to prevent.
- `reconcile` backfills a file that has none, adopting the id of the row that
  already holds that seq so existing `runs.issue_id` links hold, and minting one
  only when there is no row. This makes reconcile a writer of the files it
  reads, which is a real departure — justified because it is one converging
  write per file, it changes nothing an index row derives from, and a one-shot
  migration would miss every file that arrives later by hand or by sync, which
  is most of the ones that need it.
- The backfill is a **byte-level splice** (`splice_uid`, `issuefs.rs:489`), not
  a parse/serialize round-trip. The round-trip drops the trailing `# 0-4`
  comments the README's own example teaches people to write, normalizes
  whitespace, and would rewrite CRLF as LF. Backfilling an identity the user
  never asked for must not reformat a file they hand-authored.
- A file whose `uid` disagrees with the row already holding its seq keeps the
  row's id and logs. Re-keying would dangle every run pointing at it, and
  telling apart "one issue whose ids diverged" from "two issues that raced for a
  number" needs a common ancestor — which only a sync pass has. **This is the
  hook the sync engine resolves against.**

## Implementation hazards for whoever picks up the sync engine

- `ensure_issues_untracked` (`state.rs:3369`) runs at the top of **every**
  issue-touching path. The ref design does not fight it — nothing enters the
  working-tree index — but anything that ever does must suppress it, or watch
  the app re-untrack the files and commit the removal underneath it.
- `ensure_agency_excludes` also runs on worktree creation, so it re-adds the
  exclude line on a schedule no setting controls.
- A repo `.gitignore` that un-ignores the issues dir outranks
  `.git/info/exclude`; `untrack_issue_files` logs this rather than editing the
  user's `.gitignore`. Keep that restraint.
- Pushing `refs/agency/issues` is outside the default refspec, so it will not
  travel on a plain `git push`, and a clone will not have it without an explicit
  fetch. Both are features (no surprise data in a normal push) and both need
  saying in the UI.
- On a public remote, a ref is not a secret. If the setting says `origin` and
  `origin` is public, the backlog is public. Say so at the point of choosing.
- `alloc_issue_seq` is a per-machine counter, but `reconcile` calls
  `ensure_issue_seq_at_least` for every file it sees and that is monotonic — so
  a fetch drags the counter past anything that arrived. The collision window is
  only "both sides filed before either synced."
- `atomic_write` writes `.{uuid}.tmp` beside the target before renaming. Harmless
  for a ref snapshot; it is the kind of thing a file-sync service would pick up,
  which is another small mark against that route.

## What is left

Each of these was decided against for now rather than missed.

- **A trigger.** Every sync is manual, from the button on the issues toolbar.
  Automatic on project open is the obvious next step and needs no engine change;
  manual first is how you find out whether the merge behaves before it runs
  unattended.
- **A push that fails is only `pushed: false`.** The common cause is the remote
  having moved, and "sync again" is the right advice, but telling that apart
  from an auth failure means parsing git's stderr or re-fetching to compare.
  Left undone rather than guessed at; the toast says the shared copy was not
  updated, which is true either way.
- **Renumbering.** Two uids claiming one key is reported and neither side is
  touched. Resolving it means renaming a file and rewriting every `links:` that
  names it, which is its own pass. Publish/Adopt avoids the only case a single
  user hits, so this is a team problem and not yet a real one.
- **Orphaned attachments.** An asset whose issue was deleted is never collected.
  A merge is the wrong place to decide a file is unreachable.

## Open questions

1. Sync trigger: on app focus, on project open, on dispatch, on a timer?
2. Is there a story for issues that stay private in an otherwise shared backlog,
   or is that out of scope?
3. Does `rank` eventually become the per-person overlay, and if so is that a
   separate ref, a separate file, or a local-only column?
4. Should the ref be pruned? Its history grows one commit per changed sync
   forever, and nothing ever reads further back than the merge base.
