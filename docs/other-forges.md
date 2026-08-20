# The other forges

*Written 2026-08-21 as the back half of AGE-144. Nothing here is built.*

Agency speaks to exactly one forge: GitHub, through the user's own `gh`. That is
deliberate. Agency has one user's workflow to serve and that workflow is GitHub,
and breadth bought before anyone asks for it is breadth that rots. This document
exists so that the first time someone with a GitLab repo asks, the answer is a
plan rather than a fresh afternoon of reading `gh.rs`.

The prompt for it is a competitor reading (`competitive-landscape.md`,
2026-08-20): four forges — GitHub, GitLab, Bitbucket, Azure DevOps — behind one
provider interface, authenticated by shelling out to the user's own `gh`, `glab`
or `az`, or by a token in the environment for Bitbucket. Their layout is one
provider file plus one JSON-shape file per forge, which is a sound decomposition
and a usable estimate anchor.

## What already generalises, and what does not

`gh.rs` opens with the claim that `GhCli` "is the seam where another provider
could present the same surface later." That is mostly true, and it is worth
being precise about the parts where it is not.

**Generalises cleanly.** Everything with a shell-out shape and a JSON answer:
readiness, clone, list PRs, view one PR, list and view issues, create a PR, edit
a PR, merge, check rollup, current login, edit your own comment. Each is a
different subcommand and a different set of field names, which is what a
per-forge JSON-shape file is for.

**Generalises with a conversion.** The diff. GitHub, GitLab and Bitbucket will
all hand back a unified diff, so `parse_pr_diff` — already a pure function over
a `&str` — is reused verbatim and only the command that produces the text
changes. Azure DevOps has no "give me the PR diff" call; it exposes iterations
and per-file changes, so its provider either walks that API or diffs the two
commits locally. That is the single largest per-forge cost in the set.

**Does not generalise.** Two things:

1. **The atomic review.** `submit_pr_review` posts a verdict plus every pending
   inline comment as one object, because GitHub has such an object. GitLab does
   not: approval (`glab mr approve`) and each inline discussion are separate
   calls, and "request changes" has no native equivalent at all. Azure has
   votes on the PR plus separate threads. Bitbucket has `approve` /
   `request-changes` endpoints plus separate comments. So the interface method
   stays `submit_review(verdict, summary, comments)` and each non-GitHub
   provider implements it as a sequence, which means it can half-fail. It must
   report *what* landed, not just that something did — a partial review that
   claims success is worse than one that reports "approved, 3 of 5 comments
   posted".

2. **Comment identity.** Ours is `(threadId: GraphQL node id, databaseId: REST
   integer)`. GitLab's is `(discussion id: string, note id: integer)`, Azure's is
   `(threadId: int, commentId: int)`, Bitbucket's is a single comment id with
   parent links. The shared type has to carry an opaque string pair rather than
   today's `String` + `u64`, and no provider may parse another's ids.

## The shape

The existing structs are already the right vocabulary — `PrInfo`, `PrDetail`,
`PrFileDiff`, `ReviewThread`, `PrReviewComment`, `DraftComment`, `CheckItem`,
`MergeMethods` — because the frontend consumes them and the frontend should not
learn which forge it is talking to. Three of their fields are GitHub strings
that must become normalised enums at the provider boundary before a second
provider exists, or every provider will start guessing at the other's spellings:

| Field | GitHub | GitLab | Azure DevOps | Bitbucket |
| --- | --- | --- | --- | --- |
| `state` | `OPEN` / `CLOSED` / `MERGED` | `opened` / `closed` / `merged` / `locked` | `active` / `completed` / `abandoned` | `OPEN` / `MERGED` / `DECLINED` / `SUPERSEDED` |
| `mergeable` | `MERGEABLE` / `CONFLICTING` / `UNKNOWN` | `merge_status` | `mergeStatus` | `/merge` dry-run |
| `review_decision` | `APPROVED` / `CHANGES_REQUESTED` / `REVIEW_REQUIRED` / null | approval rules | vote ints `-10..10` | participant flags |

Azure's `sourceRefName` / `targetRefName` arrive as full ref paths
(`refs/heads/x`) and GitLab numbers merge requests by per-project `iid`, not the
global `id` — both are normalisation at the boundary, not new fields.

So:

- `crates/agency-core/src/forge/mod.rs` — the shared types (moved out of
  `gh.rs`) plus `trait Forge`, one method per current `GhCli` method, each
  taking `repo: &Path`.
- `forge/github.rs` — today's `gh.rs`, unchanged behaviour, `impl Forge for
  GhCli`. Its fake-`gh` tests move with it and stay the template: every provider
  gets a fake CLI on a temp dir and asserts on the argv and the piped stdin. No
  provider needs a network to be tested.
- `forge/gitlab.rs`, `forge/azure.rs`, `forge/bitbucket.rs` — one file each,
  plus a JSON-shape module per forge if the `#[derive(Deserialize)]` structs
  outgrow their provider file.
- `forge/detect.rs` — resolve a project's provider.

`gh.rs` keeps its path as a re-export for one release so nothing outside
`agency-core` breaks in the same commit.

## Detection is an allowlist, not a guess

Which forge a project uses is read from `git remote get-url origin`, never from
a user setting — a setting that disagrees with the remote is a support ticket.
Per the house rule on anything imported: **allowlist exact hosts**. `github.com`
→ GitHub, `gitlab.com` → GitLab, `bitbucket.org` → Bitbucket, `dev.azure.com`
and `*.visualstudio.com` → Azure. Anything else is `Forge::Unknown`, which
disables the PR surface with a plain message, rather than being assumed to be
GitHub and failing later with a parse error nobody can read.

Self-hosted instances (GitHub Enterprise, self-managed GitLab) are the exception
that has to be handled explicitly, because their hostnames are arbitrary. Today
GHE works only because `gh` resolves its own host config and we never look. The
honest rule: if the host is not on the allowlist, ask each installed CLI whether
it claims the host (`gh auth status`, `glab auth status`) and take the first
that says yes; if none does, `Unknown`.

## Readiness and auth

`GhReadiness` (`NotInstalled` → `NotAuthenticated` → `NoGithubRemote` →
`Ready`) is already the right ladder and is forge-independent once
`NoGithubRemote` is renamed. What is per-forge is the copy: `GhSetupHint.tsx`
names `brew install gh` and `gh auth login`, and each provider needs its own
install and sign-in line (and its own Windows equivalent in
`windows-compat.md`, which tracks every shell-out).

Three of the four keep Agency's auth story intact: `gh`, `glab` and `az` each
own their credentials, in their own keychain, and Agency never sees a token.
**Bitbucket breaks that**, and it should not be slipped in as an implementation
detail. It has no first-party CLI, so it means a token in the environment, which
means Agency reads a user secret for the first time. That is a positioning
decision — it belongs in `docs/webdesign/02-messaging.md`'s claim about what
Agency touches — and it should be taken deliberately, or Bitbucket should be
dropped from the set. GitLab and Azure DevOps carry none of that cost.

## Order and estimate

The interface extraction is the only part with any risk in it, because it
touches the 24 `GhCli::default()` call sites in `state.rs` and `setup.rs` and
must not change a single behaviour while doing so.

1. **Extract `trait Forge`**, GitHub as the only impl, resolve the provider once
   per project in `state.rs`. Pure refactor; the existing tests are the proof.
   The largest step, and the one worth doing on its own.
2. **GitLab.** The closest analogue and the one likely to be asked for first.
   `glab mr` for everything except discussions, which go through `glab api`.
   Reuses `parse_pr_diff`. The sequenced review submit is written here and the
   partial-failure reporting with it.
3. **Azure DevOps.** `az repos pr` plus `az rest` for threads, and the diff
   problem above.
4. **Bitbucket.** Only after the token question is answered.

Steps 2–4 are each a file, a JSON-shape module, and a fake-CLI test module, on
the order of a day apiece once step 1 exists. Step 1 is the one to budget for.

## What is not in scope

Issues (`list_issues` / `view_issue`) feed the "start a run from an issue"
picker, and Agency has its own tracker in `.agency/issues/`. A second forge does
not have to bring its issue tracker with it; PRs alone are a coherent slice.
