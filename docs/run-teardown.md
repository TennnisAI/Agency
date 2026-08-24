# How a run ends

Written 2026-08-20 for AGE-149, which asked three things: why deleting an agent
feels dramatic at the end of the approve flow, why archiving one felt like it
cost a whole copy of the codebase, and whether tidying up could be the next step
of a merge rather than a separate act of housekeeping.

The first two turned out to have the same root, and it was not the teardown
code. It was what the teardown *said*.

## What was actually true before this

`archive_run` has removed the worktree since it was written. `remove_keep_branch`
unlinks the checkout and deletes the directory, and only the `agent/<id>` branch
survives — a ref plus objects git already has, effectively free. So the cost an
archived run carried was never a copy of the codebase.

The belief that it was came from `ArchivedSection.tsx`, whose bulk-cleanup
button was titled "Discard every archived agent and its worktree" and whose
confirm read "along with their worktrees and branches". That is the only place
in the app that described what an archive holds, and it described something that
had not existed since the feature shipped.

Two real problems were underneath it:

1. **Archiving kept every branch, forever.** Including branches whose commits
   were already on `main`. A merged `agent/3f9c` is a second name for commits
   that are on the base; keeping one per archived run is how a repo ends up with
   a hundred `agent/*` refs nobody can tell apart.
2. **An archived run's record was one line.** The row carried a prompt and a
   branch name. Kill the sessions, delete the session rows, and later delete the
   branch, and "archived" would mean a name and nothing else. Archiving was
   sold as "keep it in case you want it back", which is a guess about the
   future, when what a person actually wants months later is to know *what that
   agent did*.

And one more that only shows up in the copy: **"delete" was always red**, with
"this cannot be undone" under it, including immediately after a successful merge
when the work was on `main` and deleting the run could not lose anything at all.
A warning that fires when nothing is at risk is one nobody reads on the day
something is.

## How competitors handle it

From `competitive-landscape.md`, which has four dated readings. This question
splits them cleanly.

**T3 Code** (read 2026-08-20) is the one with something to teach. Its threads
have a real lifecycle — settle/unsettle, snooze, pin, archive, delete — and none
of it is coupled to a checkout, because the durable record of a session is a
thread in an event-sourced log, not a directory. Archiving there costs nothing
because the thing being kept was never a worktree. It also keeps per-turn
workspace snapshots in hidden refs (`refs/t3/checkpoints`), which is the same
insight from the other end: git objects are the cheap place to keep history, and
a checkout is the expensive place.

**Spotify Xirp** (read 2026-08-12) is one worktree per session like us, and its
cleanup behaviour was not readable from the sources available; nothing to take.

**DeepSeek Harness** (read 2026-08-19) has no worktree model at all — a
"workspace" is a directory record with an ordered list of sessions — so the
question does not arise for them.

**Munder Difflin** (read 2026-08-15) does not isolate per worktree either.

So the answer is not a feature any of them has. It is a decomposition three of
the four get for free by not having worktrees: **the record of a run and the
workspace of a run are different things with different lifetimes.** We are the
ones who conflated them, because our unit of work is a worktree.

## The model

Three things a finished run can hold, and they are now decided separately:

| | worktree | branch | record |
|---|---|---|---|
| Live | ✓ | ✓ | — |
| Archived, branch kept | removed | ✓ | ✓ |
| Archived, branch gone | removed | removed | ✓ |
| Deleted | removed | removed | removed |

The middle two are both "archived". Which one a run lands in is not a choice the
user makes; it is a fact about the branch, and `cleanup.rs` reads it:

- **Merged** — every commit is on the base. Deleting the branch loses nothing.
- **Pushed** — every commit is on some remote branch (a PR opened, or a PR
  merged upstream and fetched). Deleting the local branch loses nothing.
- **Empty** — the branch has no commits of its own.
- Anything else — the branch is the only copy, so archive keeps it and the run
  stays restorable.

Two facts default-deny rather than assert. If `base..branch` will not resolve
(a base branch renamed out from under the run), the count comes back zero, which
would read as "carries nothing" and get the branch deleted; `commits_known`
stops that. And if the worktree is dirty, nothing is treated as safe: archive is
about to commit those changes onto the branch, which makes the branch the only
copy of something, and delete is about to destroy them, which is dangerous
whatever state the branch is in.

`commits_at_risk` and `loses_uncommitted` fall out of the same function, and
`danger()` is exactly "one of those is non-zero". That is what decides the red
button, rather than which verb the user reached for. A delete after a merge is
not red. A delete of three unmerged commits is red whether the user pressed
archive or delete — except that archive never deletes such a branch, so it never
is.

Every probe is local: `merge-base --is-ancestor`, `rev-list --count`,
`branch --remotes --contains`, `status --porcelain`. This is read while a menu is
opening. A teardown dialog that waits on `gh` is a teardown dialog that hangs,
and being wrong in the stale-fetch direction costs one branch kept that could
have gone, which is the harmless direction.

## The record

`.agency/records/<run>.md`, written before anything is removed, git-excluded by
the same list as `.agency/issues/`. Markdown, in the project, for the reason the
tracker is: a file the user can grep and read without Agency outlives Agency.

It carries the prompt, the issue it came from, the commit list, the diffstat, the
cost, and — the part a database row could not say — one sentence naming where
the work is now: "Merged into `main`. The worktree and the `agent/3f9c` branch
were removed; the commits are on `main`."

The commit list is why `Run::base_commit` now exists. `merge-base(base, branch)`
is the fork point only while the branch is unmerged; once it lands, the merge
base *is* the branch tip and `merge-base..branch` is empty. Without a base
commit recorded at creation, the archive record would list no commits for
exactly the runs that succeeded. Runs created before this fall back to the merge
base and lose the list only if they also merged, and the record says
"not recoverable" rather than "no commits" — those are different claims.

What the record does **not** do is keep the conversation. The agent's own
transcript is readable for two of ten agents (`usage.rs` is a default-deny list
of `claude` and `pi`), so the record names the directory when it exists and
claims nothing otherwise. Rescuing and rendering those transcripts is a separate
piece of work; see "Not done" below.

## What changed in the flow

- **Merge → tidy up is one flow.** The merge window's last step lists what
  archiving removes and what it keeps, in the same shape the teardown dialogs
  use, so "Archive agent" reads as the end of the merge rather than as a
  separate decision about housekeeping. "Delete agent" sits beside it and is no
  longer red, because after a merge it cannot lose anything.
- **PR merged → tidy up is offered where you are.** When a run's PR shows as
  merged, the PR section offers "Archive agent" inline. That is the "after a
  branch is created" half of AGE-149: once the work is on the remote, the local
  branch and the worktree have nothing left to hold.
- **Both dialogs show two lists, Removes and Keeps.** The old dialogs asked for
  a choice between two verbs and then explained it in a sentence of prose. Side
  by side, with the branch's real state in them, the decision makes itself. That
  is the "more deliberate" the issue asked for: not more friction, more
  information.
- **"Discard" is gone.** The tile menu said "Discard agent" and the merge window
  said "Delete agent" for the same call. One verb now: delete.
- **Archived rows say what they hold** — `branch` or `record` — and Restore is
  disabled, with a reason, when there is no branch to restore onto. `restore_run`
  also refuses in words rather than surfacing git's "invalid reference".

## Not done

- **The conversation.** The record says what an agent did, not what it said.
  ~~Rescuing `~/.claude/projects/<encoded-worktree>/` (and pi's equivalent) into
  the archive and rendering it read-only would cover the two agents whose
  transcripts we can parse.~~ Done since 2026-08-23; see the addendum below.
- **Records for the other eight agents' transcripts.** Blocked on the same
  default-deny list `usage.rs` keeps, and it should stay default-deny.
- **Per-turn checkpoints** (AGE-140) are the neighbouring idea from the same T3
  Code reading and are tracked separately. If they ship, a record could link the
  turn a change arrived in.

## Addendum: the conversation (AGE-152, 2026-08-23)

The transcript directories the record used to only point at are keyed by
worktree path (`~/.claude/projects/<encoded>/`, `~/.pi/agent/sessions/<encoded>/`).
The moment a teardown removes the worktree, that key names a path that no
longer exists: the agent can never resume the sessions, Agency never read them
again, and nothing ever swept them — one directory leaked per finished run,
forever. The fix moves the directory with the run instead of orphaning it:

- **Archive** moves it into the record's own folder,
  `.agency/records/<run>.transcript/`, after the sessions are stopped and
  before the worktree goes. The move is copy, verify, then delete
  (`transcript.rs::move_tree_verified`), so a failure leaves the original
  untouched and the record falls back to naming it in place, exactly as
  before. Modification times survive the move because "resume the most recent
  session" is an mtime decision in the agent's own CLI.
- **Restore** moves it back. The worktree returns at the same path, so the
  store directory gets its old name and the agent's `--continue` finds the
  conversation as if the run had never been archived. This is not optional
  polish: once archive removes the agent's copy, a restore that did not
  reinstate it would have broken resume, which used to work by accident of
  the leak.
- **Delete** removes it — the agent's own directory for the worktree and any
  rescued copy — along with the record. This is the disk-leak half of AGE-152
  and applies even where nothing can be rendered.
- **The viewer** is a second tab in `RunRecordDialog`, rendered read-only from
  the rescued files (or the still-in-place directory, for runs archived before
  this shipped). `transcript.rs` parses the two dialects `usage.rs` already
  vouches for, against the same rule: for the eight agents whose format has
  not been read against real files, the tab says Agency cannot see the
  conversation, which is a different claim from "the agent said nothing".
  Their transcripts are also never rescued or removed: `session_dir` cannot
  name a directory for them at all, so their leak — wherever it is — remains,
  and stays honestly unclaimed.

Two boundaries hold everywhere: only worktree runs are touched (a run in the
project's own checkout shares its session directory with the user's own
sessions in that folder, which is not Agency's to move), and the record for a
rescued transcript names the newest session's own resume command
(`claude --resume <id>`) only for claude, whose session-id-is-the-filename
layout has actually been verified.

Resuming an *unrestorable* archived run's conversation — branch gone, so no
worktree to put the sessions back into — is the remaining piece, tracked as
its own issue.
