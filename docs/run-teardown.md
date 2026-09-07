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

## What adjacent products do differently

Four dated readings of adjacent tools, made 2026-08, split cleanly on this
question, and the split is the useful part rather than any one product.

Only one of the four has a session lifecycle worth taking: settle/unsettle,
snooze, pin, archive, delete, none of it coupled to a checkout, because the
durable record of a session there is a thread in an event-sourced log rather
than a directory. Archiving costs nothing when the thing being kept was never a
worktree. The same tool keeps per-turn workspace snapshots in hidden refs, which
is the identical insight from the other end: **git objects are the cheap place
to keep history, and a checkout is the expensive place.**

Of the rest, one is one-worktree-per-session like us with cleanup behaviour that
was not readable from the available sources, and two have no worktree model at
all — a "workspace" is a directory record with an ordered list of sessions — so
the question never arises for them.

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
transcript is readable for two of eleven agents (`usage.rs` is a default-deny list
of `claude` and `pi`), so the record names the directory when it exists and
claims nothing otherwise. Rescuing and rendering those transcripts is a separate
piece of work; see "Not done" below.

## What changed in the flow

- **Merge → tidy up is one flow.** The merge window's last step lists what
  archiving removes and what it keeps, in the same shape the teardown dialogs
  use, so "Archive agent" reads as the end of the merge rather than as a
  separate decision about housekeeping. "Delete agent" sits beside it and is no
  longer red, because after a merge it cannot lose anything. (The lists are
  gone since AGE-164; see the second addendum.)
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
worktree to put the sessions back into — was the remaining piece, tracked as
its own issue. The addendum below closes it, by removing the premise: there is
almost always a worktree to put the sessions back into.

## Addendum: the last step says all three buttons (AGE-164, 2026-08-27)

The step above shipped with the *archive* plan's Removes/Keeps lists under a
heading, "Tidy up", and three buttons under that: Archive agent, Delete agent,
Keep agent. No button said "tidy up", and the lists described exactly one of
the three. Read cold, the panel looked like an explanation of a fourth thing,
and the two options it did not mention were left to be guessed at.

The two lists were the right answer to a different question. In a confirm
dialog the user has already chosen a verb and is being asked to check it, so
Removes and Keeps are a checklist of one action. In the merge window the verb
is the question, and a checklist of one candidate is not an answer to it.

So the last step is two sentences now (`runRemoval.ts::mergeTidyCopy`), and the
first names all three buttons in their own words: Archive puts the agent away
and keeps a record under Archived, Delete removes it and its record, Keep
leaves it as it is. The second is the mechanics both verbs share — the agent
stops, the worktree goes, and the merged branch goes with it, "which is already
on main" in the same clause `safeClause` writes for the dialogs. It is still
built from the backend's `CleanupPlan` rather than written by hand, so the
divergences stay honest: a dirty worktree keeps the branch through an archive
and loses it to a delete, and the sentence then says "only deleting takes the
agent/foo branch with it" instead of claiming both do.

The uncommitted-work caveat is the one line that cannot be hushed, because it
is the only one that changes which button you press. The hush ("Don't show this
again") now hides the mechanics sentence and leaves the three verbs, which is
the part that is one line long and worth keeping forever.

## Addendum: several agents in one worktree (AGE-184, 2026-09-05)

A run stopped being one agent the moment extra tabs could share its worktree,
and three things in this file were still written as if it were one. All three
were reported together.

**The first tab could not be closed.** Every extra tab carried a ✕; the run's
own did not, because the run *is* that session — closing it cannot delete a
row. So it stamps one instead: `runs.primary_closed_at`. The strip stops
drawing the tab, `ensure_run_active` stops reviving it (every other caller of
that function treats a missing session as an accident to repair, so without the
check the agent came straight back on the next poll), and
`state.rs::lead_session_name` hands the run's status dot, its notifier watch,
its idle gate and anything Agency types in to the lowest-numbered tab still
alive. Lowest-numbered because that is the strip's leftmost: what the rail's
dot describes is what the strip opens on.

Refused when it would leave the run with no agent at all — an empty tab strip
is a run you can only stare at, and "there is one left" is exactly when archive
or delete is the thing meant. Refused too for a looping run, whose driver owns
that session name and would only spawn into it again. The way back is the +
menu's "Reopen", which clears the stamp and goes through `ensure_run_active`,
so the agent comes back into the conversation it had rather than a blank one;
archiving clears it as well, because archiving takes every extra tab with it
and the run's own agent is then all there is to restore as.

**Archiving saved one agent's transcript, not the workspace's.** The rescue
above walks a single directory, `session_dir(run.agent, worktree)`. Tabs
running the *same* agent as the run always rode along for free — claude writes
one file per conversation in the workspace's directory and pi a store per
session under it, and the move takes the tree. A tab running a *different*
agent did not: its directory hangs off that agent's own root, nothing named it,
and archiving orphaned it in exactly the way AGE-152 fixed for the run itself.
So the rescue is now one directory per agent that worked here, read off the
session rows *before* the archive deletes them. The extra ones land beside the
run's own as `<run>.transcript.<agent>` — a sibling, not a child, because the
readers descend exactly one level looking for session files and pi already
spends that level on a store per session. Restore, delete and the record's
conversation tab all walk the same set; restore finds the agents by parsing the
directory names, since an archived run has no session rows left to ask. The
agent name is allowlisted rather than escaped (`record.rs::transcript_dir_for`):
profile names are user-editable, and a rescue that could be talked into `..` is
a delete outside the archive.

**"Fix with agent" always went to the first agent.** It typed the conflict into
`session_name(run.id)` unconditionally, which after a day's work is routinely
the agent with the least to do with the branch being merged. `send_merge_conflict`
now takes a session id, checked against the run rather than trusted, and the
merge window offers the run's tabs when there is more than one. It defaults to
the tab the run was last looked at — the focus view already remembers that per
run (AGE-31), and "the agent I was just working with" is what the phrase means.
Shell tabs are left out of the list: a login shell does not read a prompt. The
same `lead_session_name` covers the two senders that have no picker (CI
feedback, review comments), so a run whose first tab is closed still receives
them.

## Addendum: restoring a merged run (AGE-157, 2026-08-31)

The piece the AGE-152 addendum left open was "resume a rescued conversation
when the branch is gone". It was framed as a *new* flow — a Resume button that
cuts a fresh worktree as a new run, with a new id and a new branch, seeding the
rescued sessions into the encoded directory for the new path. It shipped as
something much smaller: the *existing* Restore stopped refusing.

Restore used to bail when `agent/<id>` was absent, with "there is no branch to
restore this agent onto". That is the ending of nearly every run, because
archiving a merged run deletes its branch on purpose: the branch is by then a
second name for commits that are already on the base. So Restore was disabled
on the common ending and worked only on the abandoned one. It now asks where
the work went instead (`state.rs::restore_start_point`: the merge target first,
`run.base` second, whichever is still in the repo) and cuts the branch again
from there with `WorktreeManager::recreate_on`. The worktree comes back holding
that work, `reinstate_transcript` puts the rescued sessions back, and the next
launch resumes into them the same way a branch-kept restore always did.

Three things are worth writing down. The first two are why the larger design
was not needed; the third is what the smaller one got wrong on its way past,
and had to be fixed here.

- **The cwd question evaporates.** A fresh-worktree run would have put the
  rescued session files under a *different* path's encoded directory, while
  their own `cwd` fields still named the old one, and no agent had been shown
  to tolerate that. Restoring the same run reuses `.agency/worktrees/<id>`, so
  the store directory's name and the session's own `cwd` agree, exactly as they
  did before the archive. Nothing per-agent had to be verified, and nothing had
  to be default-denied.
- **Only one archive is still unrestorable**, and honestly so: the branch gone
  *and* the base gone. `lib/restore.ts` disables Restore on exactly that case
  and the tooltip names both, rather than the old message that blamed the
  branch alone. `restore_start_point` looks for branches only; a run whose base
  branch was itself deleted could in principle be cut from the `base_commit`
  the record names, which is not done.
- **A restored run keeps the account of the stint it just finished.** Restore
  used to delete the record, on the grounds that a file left in place would be
  read as the account of a run that is live again. That is true of `<run>.md`,
  which is why this is a rename rather than nothing, but the deletion also
  threw away the only account of what the run had done: its commit list, how
  the work ended, what it cost. The conversation survived the cycle and those
  did not, so a run archived twice remembered everything it *said* and nothing
  it *did*. The record is now set aside as `<run>.1.md`
  (`record.rs::prior_path`), stamped with a Superseded section saying why it is
  not the current one, and the next archive's record names it back under
  Earlier. `read_run_record` hands the dialog the stack, newest first, because
  a sibling file is not something the dialog can open; on disk each stint stays
  its own greppable file. Deleting the run takes all of them.

One more thing was missing rather than wrong: none of AGE-152's round trip had
a test. Every step of it reads `$HOME/.claude/projects/<encoded worktree>/` or
`$HOME/.pi/agent/sessions/<encoded>/`, and while that home was read from the
environment at each use, the only files a test could have moved were the
developer's own conversations — so it was checked by hand, twice, and shipped
untested through two issues. `AppState` now holds the agents' session-store
home as a field (`AppState::with_agent_home`), read from `$HOME` in the
ordinary constructor and pointed at a tempdir by the test harness. The round
trip is covered end to end against a stub agent *named* `claude`, since
`usage::session_dir` and `resume_command` both key off the command's basename:
a merged run's two sessions are rescued into the archive, the record names the
newer of the two in its `claude --resume` line, the conversation reads back out
of the archive copy, restore puts both files back with their mtimes intact (the
thing "resume the most recent session" goes by), and deleting the run sweeps
them.

## Addendum: the remote copy (AGE-201, 2026-09-07)

The table above has three columns, and until now that was the whole ending:
worktree, branch, record. It never had a fourth for the copy of the branch on
the remote, and nothing in the app took it. Merging a PR from Source Control
did (`gh.rs::merge_pr` deletes the head ref through the API, and has since it
shipped), but merging the *agent's* branch from the Approve window did not, and
that is the path the approve flow leads to. So every locally merged agent
branch that had ever been pushed stayed on the remote for good, one per run,
which is what the issue reported: "it leaves all these remote branches hanging
around".

The merge window now offers it as a checkbox beside the merge, the way the PR
merge dialog offers its own, and `merge.rs::delete_published_branch` runs after
the merge has committed. Three things it does deliberately:

- **It asks the remote, not the ref store.** Everything in `cleanup.rs` is
  answered locally, on purpose, because it is read while a menu is opening. This
  is not: it is a button press, it already costs a `git push`, and the local
  remote-tracking ref is only as fresh as the last fetch. A colleague's push
  that landed after that fetch is precisely the work this must not delete
  unseen, so the tip comes from `ls-remote` and is then required to be an object
  this checkout actually has *and* an ancestor of the base. A commit we have
  never fetched cannot be weighed against anything, so it is refused rather than
  assumed harmless.
- **It runs after the merge, never before.** The same guard makes the ordering
  structural rather than a convention: before the merge commits, the remote copy
  is the only copy off this machine and the deletion refuses itself.
- **It leaves the local branch alone.** That is `cleanup.rs`'s to take, with the
  worktree, once the user picks archive or delete, and it decides that from the
  whole plan. Two places deleting branches by two rules is how they drift.

The one thing this cannot make safe is an **open PR**. GitHub closes a pull
request whose head branch is deleted; it only marks one merged once the commits
reach the base branch *on the remote*, which a local merge has not done yet. So
a branch with an open PR would be traded for a PR that says "closed" — the
record of how the work landed, lost to save a ref. The window unticks the box
when the PR probe it already runs reports an open one, says why beside it, and
points at merging the PR from Source Control instead, which ends with the PR
merged and the branch deleted. It unticks rather than refuses, and only while
the user has not touched the box themselves: this is the user's remote and
their PR.

## Addendum: a conflict handed to an agent that never took it (AGE-199, 2026-09-07)

The picker the AGE-184 section describes worked, and then the prompt sat there.
Handing the conflict to any tab but the one that speaks for the run put the
message in the queue under "waiting for it to finish its turn", on an agent
sitting at an empty prompt with no turn in flight, and left it there for five
minutes before it went in on top of whatever was on the line.

The queue was not the bug. `sendq::decide` reads one observation of a session,
and the field it holds out for is `working`, which comes from `crate::activity`
— bookkeeping the notifier tick keeps. That tick writes one entry per *run*,
keyed by the run id, from the pane of whichever session speaks for it
(`watch_snapshot`). An extra tab has no entry at all, and no entry reads as
working, deliberately: unknown holds, which is the right answer for a session
spawned seconds ago and the wrong one forever for a session nobody watches. So
every prompt aimed at a tab was held until `MAX_HOLD_MS` overrode it, and the
marker said the one thing that was not true about why.

The tick now observes those tabs too — `extra_session_panes`, the same pane
hash diffed the same way, into the same map, keyed by session id. Nothing else
follows from it: no notification, no tray entry, no watch. A tab is not a run
and a notification is about a run. Only the busy/idle bit, which is the one
thing the queue was reading and could not find.

Two things the issue asked for sit on top of that.

**A new agent is a target.** Every agent in the worktree may be busy with
something else, and waiting out someone else's turn is a poor answer when the
alternative costs a tab. "New agent" is the last option in the picker;
`spawn_merge_conflict_agent` runs the same two checks (a merge is in progress,
and it is this run's), then opens a tab with the conflict as its *opening
argv*. Nothing can be queued behind a session that did not exist a moment ago,
so that hand-off cannot wait. The tab is selected in the picker afterwards and
set as the run's pending session, so "Close and watch" lands on it.

**A conflict can be resolved by hand.** It could not be, and the way it failed
was expensive: a row under "Merge Changes" opened the ordinary diff viewer,
whose stage-by-hunk and stage-by-line buttons were the only thing on offer. But
`git diff` answers for an unmerged path with a *combined* diff — `diff --cc`,
`@@@` headers, two columns of prefixes — whose lines are not the file's lines.
Picking the side you wanted out of that and staging it fed marker lines back
through `git apply`, which is how `<<<<<<< HEAD` and a branch name ended up
written into the file. An unmerged row now opens `ConflictView` instead, which
reads the *file*: `lib/conflictFile.ts` parses it into blocks and rebuilds it
without the markers and without the side you did not pick, and the pane writes
that back and stages it. The transformation is pure and tested on its own,
including the cases where it must refuse — a marker it cannot pair up returns
no blocks at all, because a rewrite that leaves one `<<<<<<<` behind is the
failure the whole view exists to stop. Delete-against-edit conflicts, which
carry no markers, get their own two buttons; a `DD` gets the one that applies.
