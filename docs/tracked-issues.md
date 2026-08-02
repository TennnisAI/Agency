# Tracked issues — should `.agency/issues/` go in git?

Status: **exploration**. No decision locked, nothing implemented. This doc
exists so the question can be picked up later without re-deriving the
constraints.

The question: Agency keeps issue files out of git today. For a team using this
professionally, should there be an option to put them in?

Recommendation up front: **yes, eventually — but not as a "track issues"
checkbox.** A raw toggle re-creates the exact failure that made us untrack them
in the first place. The version worth building splits the issue file by
volatility, and it is a per-project setting, not a global one.

---

## What is true today

Canonical issue storage is one markdown file per issue under
`.agency/issues/AGE-14.md` — YAML frontmatter, H1 title, markdown body
(`issuefs.rs:1`). SQLite is a rebuildable index over those files, not the
source of truth.

Those files were tracked once. There is a committed, wired one-shot migration
that untracks them:

- `ensure_agency_excludes` (`worktree.rs:373`) writes `.agency/worktrees/`,
  `.agency/agency.local.toml`, and `.agency/issues/` into **`.git/info/exclude`**,
  and strips any legacy blanket `.agency/` line. Note what it deliberately
  leaves visible to git: `.agency/agency.toml`, the project's shared config.
- `untrack_issue_files` (`worktree.rs:421`) runs `git rm -r --cached` over the
  issues dir and **commits** the removal (staged deletions being exactly the
  dirt it is trying to eliminate). It refuses over a merge in progress or a
  staged index, and logs a warning if a repo's own `.gitignore` un-ignores the
  directory and outranks the exclude file.
- Both are driven from `state.rs:1653` (`ensure_issues_untracked`), gated on a
  per-project setting `issues_untracked:{project_id}`, called at the top of
  every issue-touching path.

So "untracked issues" is not a property of this checkout. It is **product
behavior applied to every project Agency manages**, and it actively reverses
itself if it finds tracked issue files.

(Correction to an earlier read: `DEFAULT_GITIGNORE` at `setup.rs:44` does not
list `.agency/`, but that is not a gap — the exclude machinery above covers it
per-project, and does so more precisely than a blanket ignore would.)

## Why we untracked them — and why those reasons still hold

Recorded in `issuefs.rs:19` and `worktree.rs:410`. Three reasons, in increasing
order of how hard they are to design around:

**1. Churn.** The app rewrites frontmatter on every status change — every
dispatch, every merge, every board drag. Tracked, that leaves the main checkout
permanently dirty, and a merge refuses to start on a dirty checkout. Tracking
issues would therefore intermittently disable merging agent branches. This is a
workflow bug, fixable by design.

**2. Conflicts.** Agent branches write the same frontmatter lines the main
checkout writes, so every agent merge carries the same trivial conflict on both
sides. Also fixable by design.

**3. Branch-locality.** This is the one that is not a workflow problem.

Issue state is **workspace-global**: `.agency/issues/` lives in the project's
own checkout and every worktree reads and writes that single shared copy
(`worktree.rs:29`, `state.rs:431`). Git content is **branch-local**: tracked,
each worktree gets its own copy at its own commit. "What status is AGE-14"
stops having one answer. A backlog that forks per feature branch and merges by
three-way text diff is not a tracker.

Any design that tracks the churny fields has to answer point 3, and the honest
answers are all bad: pick a branch as authoritative (then worktrees write to a
tree they do not own), or accept divergence (then the board lies).

## The design that works: split by volatility

Do not track the issue file. Track **half** of it.

| Half | Fields | Storage | Rationale |
| --- | --- | --- | --- |
| Stable | `key`, title (H1), body, `created`, attachments | tracked | Changes when a human edits an issue. That is a commit-worthy event, reviewable in a PR, and conflicts on it are real conflicts worth resolving. |
| Volatile | `status`, `rank`, `due`, `scheduled`, `updated` | local, gitignored | Changes on drag, on dispatch, on merge. Means nothing to a teammate on another branch. |

Current frontmatter key set is at `serialize_issue_file` (`issuefs.rs:293`):
`key`, `status`, `priority`, `due`, `scheduled`, `rank`, `created`, `updated`,
plus preserved unknown keys. `priority` is the interesting edge — arguably
stable (a team fact) rather than volatile. Worth deciding explicitly.

This kills reasons 1 and 2 outright, and dissolves reason 3: the fields that
would fork per branch never enter git at all. Merging an agent branch brings
back edited issue *text*; board position stays yours.

It also matches a split the codebase already made and likes: `agency.toml`
tracked and shared, `agency.local.toml` gitignored and per-machine
(`config.rs:203`). Same shape, same reasoning, one layer down.

Sidecar shape is an open question — a single `.agency/issues.local.toml`
keyed by issue key is probably better than one sidecar file per issue (fewer
inodes, one write per board change, trivially ignorable with one exclude line).

## Alternatives considered

**Track everything, auto-commit on change.** Keeps the tree clean by committing
the churn. Rejected: it pollutes history with hundreds of status-flip commits,
which is precisely the thing this whole approach was chosen to avoid. It also
does nothing for branch-locality.

**Issues on a separate git ref (the `git-bug` model).** Store the tracker in an
orphan ref, synced by push/pull, never checked out. Cleanest result: no working
tree impact, no history pollution, no branch divergence, real multi-machine
sync. Costs: significantly more machinery, and it gives up the property that
issues are plain markdown readable on GitHub and in any editor — currently one
of the nicer things about the format, and the whole reason attachments use
relative links (`attachments.ts:6`). Worth revisiting only if the split-file
design proves insufficient.

**Do nothing.** Legitimate. Teams that need a shared tracker mostly already have
Linear/Jira/GitHub Issues, and Agency's issue view could grow an import instead
of becoming the system of record. The counter-argument is repos that want the
backlog to live with the code and travel with a clone.

## Where the setting belongs

**Per-project, not global.** Whether a backlog should be shared is a fact about
the repo and its team, not about the machine. A global toggle would push
personal-project issues into repos that should not have them.

Framing matters too. Present it as "share this project's backlog with the repo"
(and say what that means: issue text is committed and pushed; status and board
order stay local). Not as "git-track `.agency/issues/`", which describes the
mechanism and hides the consequence.

Given `.agency/agency.toml` is already the tracked, shared, team-visible config
file, that is the natural home for the flag — which conveniently means the
decision travels with the repo, so a teammate cloning it gets the same behavior
without configuring anything.

## Implementation hazards for whoever picks this up

- `ensure_issues_untracked` (`state.rs:1653`) runs at the top of **every**
  issue-touching path and is gated only by the `issues_untracked:{project_id}`
  setting. A tracking toggle that does not also suppress this will watch the app
  re-untrack the files and commit the removal underneath it.
- `ensure_agency_excludes` is also called on worktree creation
  (`worktree.rs:172`), so it re-adds the exclude line on a schedule the toggle
  does not control. Both call sites need the flag.
- `untrack_issue_files` commits with `--no-verify` and refuses over a staged
  index or a merge in progress. Whatever re-tracks needs the same guards, in
  reverse, and should be equally careful never to sweep up unrelated work.
- A repo `.gitignore` that un-ignores the issues dir outranks
  `.git/info/exclude`; `untrack_issue_files` already logs this case rather than
  editing the user's `.gitignore`. Keep that restraint.
- `reconcile` makes the index follow the files. Once half the fields come from a
  sidecar, reconcile has to merge two sources, and decide what happens to an
  issue whose text arrived via merge but which has no local sidecar row (answer
  is probably: defaults — `todo`, no rank, sorted last).
- The unknown-key passthrough (`extra`, `issuefs.rs:52`) exists so a newer
  schema's fields survive an older build's write. Moving fields *out* of
  frontmatter interacts with that: an old build writing a file it read from a
  new build must not resurrect a `status:` line the new build deliberately
  dropped.

## Cleanups (done 2026-08-02)

Both were independent of the decision above and have been fixed.

- **`ui/src/lib/attachments.ts`** described `.agency/issues/assets/` as "inside
  the one part of `.agency/` git tracks" and claimed attachments "merge onto
  main with the issue text when an agent branch lands." That was the pre-untrack
  design; neither clause had been true since. The header now states the actual
  consequence — assets are local to a checkout, so they do not travel with an
  agent branch and the relative links do not resolve on GitHub — and points
  here. (Both properties come back under the split-file design, which is a
  decent sign the split is the right shape.)
- **This repo's `.gitignore`** ignored `.agency/` wholesale, which also hid
  `.agency/agency.toml`, the one file the product's own exclude logic
  deliberately keeps trackable. Now lists the same three paths
  `ensure_agency_excludes` writes (`worktrees/`, `issues/`,
  `agency.local.toml`). No behavior change today — this repo has no
  `agency.toml` yet, and `.agency/.DS_Store` stays covered by the existing
  unanchored `.DS_Store` rule — but the repo now models the design instead of
  contradicting it.

A third, larger inconsistency is left alone deliberately: the doc comment on
`issuefs::ISSUES_DIR` and `worktree::untrack_issue_files` explains the untracked
decision at length and is still accurate. If the split-file design lands, those
are the comments to revisit.

## Open questions

1. Is `priority` stable or volatile?
2. Sidecar format and location — one file keyed by issue, or per-issue files?
3. What does a teammate see on first clone: every issue at `todo`, or does the
   tracked half carry an initial status that local state then overrides?
4. Does deleting an issue on a branch mean deleting it, given the file is the
   identity and merges can resurrect files?
5. Is there a story for issues that should stay private in an otherwise shared
   backlog, or is that out of scope?
