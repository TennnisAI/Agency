use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MergeOutcome {
    Clean { commit: String },
    Conflicts { files: Vec<String> },
}

/// Where a conflicted merge stands right now, asked of git rather than
/// remembered from the attempt that started it. What a resolver (agent or
/// human) leaves behind is one of three states, and the caller needs to tell
/// them apart: still conflicted, resolved but uncommitted, or already
/// committed. Re-running the merge cannot answer that question, because a
/// merge in progress is by definition a dirty checkout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeState {
    /// `MERGE_HEAD` exists *and* names this branch: a merge of it is started
    /// but not yet committed. See [`blocked_by`](Self::blocked_by) for why the
    /// second half of that matters.
    pub merging: bool,
    /// Paths git still considers unmerged, either because conflict markers
    /// remain or because the resolution was never `git add`ed.
    pub unresolved: Vec<String>,
    /// The branch tip is already an ancestor of the base branch, i.e. the
    /// merge landed (whoever committed it).
    pub merged: bool,
    /// Set when the project's checkout is mid-merge of a *different* branch.
    /// Every run merges in the one shared checkout, so an unfinished merge is
    /// visible from all of them; without this they'd each read it as their own.
    pub blocked_by: Option<String>,
}

fn git(repo: &Path, args: &[&str]) -> Result<std::process::Output> {
    Ok(Command::new("git").args(args).current_dir(repo).output()?)
}

/// [`git`] for a command that talks to a remote. There is no TTY behind the
/// app, so anything that stops to ask a question hangs the call forever rather
/// than asking anyone anything; this fails fast instead.
///
/// `GIT_TERMINAL_PROMPT=0` alone does not buy that: it silences git's own
/// prompts and nothing else. An SSH remote whose key has a passphrase and no
/// agent loaded still stops on ssh's own `/dev/tty` prompt, and so does an
/// unknown host key; a credential helper still blocks on its own UI. Started
/// from a terminal by `./dev.sh` that is a real hang, with the merge window
/// sitting on "Deleting origin/agent/x…" for good and a runtime thread pinned
/// behind it. `BatchMode=yes` turns ssh's prompts into a refusal and the
/// askpass pair does the same for git's credential path. The `http.lowSpeed*`
/// settings cover the other half, a transfer that connects and then stalls,
/// which no prompt setting can reach.
///
/// The credential *helpers* are untouched, and they are consulted first: a
/// working osxkeychain still answers. `GIT_ASKPASS` is only reached once every
/// helper has failed, which is exactly the prompt this must not draw.
fn git_net(repo: &Path, args: &[&str]) -> Result<std::process::Output> {
    // Appended to whatever the user already set rather than replacing it: a
    // custom `GIT_SSH_COMMAND` is usually an `-i <key>`, and dropping that
    // breaks the push we are trying to make.
    let ssh = std::env::var("GIT_SSH_COMMAND").unwrap_or_else(|_| "ssh".to_string());
    Ok(Command::new("git")
        .args(["-c", "http.lowSpeedLimit=1000", "-c", "http.lowSpeedTime=20"])
        .args(args)
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        // `echo` exits 0 with an empty answer, which git reads as a failed
        // credential rather than as a password to retry with.
        .env("GIT_ASKPASS", "echo")
        .env("SSH_ASKPASS", "echo")
        .env("SSH_ASKPASS_REQUIRE", "never")
        .env("GIT_SSH_COMMAND", format!("{ssh} -o BatchMode=yes -o ConnectTimeout=10"))
        .output()?)
}

fn git_ok(repo: &Path, args: &[&str]) -> Result<String> {
    let out = git(repo, args)?;
    if !out.status.success() {
        bail!("git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn ref_exists(repo: &Path, name: &str) -> bool {
    git(repo, &["rev-parse", "--verify", "--quiet", name])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn detect_base(repo: &Path) -> Result<String> {
    for candidate in ["main", "master"] {
        if ref_exists(repo, &format!("refs/heads/{candidate}")) {
            return Ok(candidate.to_string());
        }
    }
    bail!("no main/master branch found in repo")
}

/// The branch a run should merge into: the explicitly chosen target if set,
/// otherwise the auto-detected main/master. Centralizes the fallback so
/// `merge_preview` and `merge_task` stay in agreement.
pub fn resolve_target(explicit: Option<&str>, repo: &Path) -> Result<String> {
    match explicit {
        Some(t) if !t.trim().is_empty() => Ok(t.to_string()),
        _ => detect_base(repo),
    }
}

pub fn is_merging(repo: &Path) -> Result<bool> {
    Ok(ref_exists(repo, "MERGE_HEAD"))
}

/// Resolve a revision to a commit hash, or `None` if it doesn't exist.
pub fn rev(repo: &Path, r: &str) -> Option<String> {
    git(repo, &["rev-parse", "--verify", "--quiet", &format!("{r}^{{commit}}")])
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Whether `branch` still resolves in `repo`.
///
/// Worth checking before evaluating any range like `base..branch`. A branch
/// deleted or renamed outside Agency makes git fail with "ambiguous argument"
/// plus a hint about using `--` to separate paths from revisions, which reads
/// as a syntax problem rather than a missing branch and sends whoever hit it
/// looking in the wrong place.
pub fn branch_exists(repo: &Path, branch: &str) -> bool {
    !branch.is_empty() && rev(repo, branch).is_some()
}

/// The merge the project's checkout is currently in the middle of: the commit
/// being merged in (`MERGE_HEAD`) and, for the UI, a branch name for it.
#[derive(Debug, Clone, PartialEq)]
pub struct InProgressMerge {
    pub commit: String,
    /// A branch pointing at `commit`, or its short hash if none does.
    pub branch: String,
}

pub fn in_progress_merge(repo: &Path) -> Option<InProgressMerge> {
    let commit = rev(repo, "MERGE_HEAD")?;
    let branch = git(repo, &["branch", "--points-at", &commit, "--format=%(refname:short)"])
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .and_then(|out| out.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string))
        .unwrap_or_else(|| commit[..commit.len().min(10)].to_string());
    Some(InProgressMerge { commit, branch })
}

/// Whether the in-progress merge (if any) is `branch`'s. The project checkout
/// is shared by every run, so `MERGE_HEAD` on its own only says *somebody* is
/// mid-merge; matching it against the branch tip is what says who.
pub fn owns_merge(repo: &Path, branch: &str) -> bool {
    match (in_progress_merge(repo), rev(repo, branch)) {
        (Some(m), Some(tip)) => m.commit == tip,
        _ => false,
    }
}

/// Number of commits on `branch` that are not yet on `base` (i.e. what a merge
/// would actually integrate). Zero means the branch carries no new work and a
/// merge would be a no-op — surfaced in the UI as "nothing to merge" so a clean
/// no-op isn't mistaken for a successful integration.
pub fn commits_ahead(repo: &Path, branch: &str, base: &str) -> Result<usize> {
    let range = format!("{base}..{branch}");
    let out = git_ok(repo, &["rev-list", "--count", &range])?;
    Ok(out.trim().parse().unwrap_or(0))
}

/// Number of commits on `base` that `branch` doesn't have — how far the base
/// has moved on since the branch was created (or last updated). Non-zero means
/// the branch is stale and the merge lands on a base it has never seen, so the
/// UI warns before merging instead of letting staleness surface as conflicts.
pub fn commits_behind(repo: &Path, branch: &str, base: &str) -> Result<usize> {
    let range = format!("{branch}..{base}");
    let out = git_ok(repo, &["rev-list", "--count", &range])?;
    Ok(out.trim().parse().unwrap_or(0))
}

/// Undo an in-progress merge and, when given one, put the checkout back on the
/// branch it was on before the merge started. Restoring is best-effort: the
/// abort itself is what the caller asked for.
pub fn abort_merge(repo: &Path, restore_to: Option<&str>) -> Result<()> {
    git_ok(repo, &["merge", "--abort"])?;
    if let Some(orig) = restore_to.filter(|o| !o.is_empty()) {
        let _ = git(repo, &["checkout", orig]);
    }
    Ok(())
}

/// The paths a merge of `theirs` into `ours` would conflict on, computed
/// without checking anything out: `git merge-tree` does the three-way merge in
/// memory and writes nothing but objects. That matters here because the
/// question is asked about a *pull request* — two remote-tracking refs, from a
/// UI pane, while the user's checkout is theirs to keep.
///
/// An empty list means the merge is clean. `--write-tree` needs git 2.38, and
/// an older git exits with a usage error rather than a merge result, which
/// surfaces here as an `Err` so the caller degrades to "conflicts, files
/// unknown" instead of reporting a clean merge that isn't.
pub fn conflicting_paths(repo: &Path, ours: &str, theirs: &str) -> Result<Vec<String>> {
    let out = git(repo, &["merge-tree", "--write-tree", "--name-only", ours, theirs])?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    match out.status.code() {
        Some(0) => Ok(Vec::new()),
        // Exit 1 is "merged, with conflicts": stdout is the merged tree's oid,
        // then one conflicted path per line, then a blank line and git's own
        // "CONFLICT (content): ..." messages, which are for humans, not us.
        //
        // A ref it can't resolve *also* exits 1 ("merge-tree: origin/nope - not
        // something we can merge"), with an empty stdout — so the oid line is
        // what tells a merge result from a failure. Without that check a
        // mistyped or unfetched branch reads as "conflicts, no files", which is
        // the one answer that would leave the caller confidently wrong.
        Some(1) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut lines = stdout.lines();
            match lines.next() {
                Some(oid) if !oid.trim().is_empty() => Ok(lines
                    .take_while(|l| !l.trim().is_empty())
                    .map(|l| l.trim().to_string())
                    .collect()),
                _ => bail!("git merge-tree failed: {}", stderr.trim()),
            }
        }
        _ => bail!("git merge-tree failed: {}", stderr.trim()),
    }
}

fn unmerged_files(repo: &Path) -> Result<Vec<String>> {
    let out = git_ok(repo, &["diff", "--name-only", "--diff-filter=U"])?;
    Ok(out.lines().map(|l| l.to_string()).collect())
}

/// Rename `from` to `to`, and nothing else.
///
/// `git branch -m` moves the ref, its config (upstream and friends) and its
/// reflog together, and repoints the HEAD of whichever worktree has the branch
/// checked out, so a run's worktree keeps working under the new name without
/// being touched. Without `-M` it refuses to clobber an existing `to`, which is
/// the backstop behind [`branch_exists`]'s check at the call site.
pub fn rename_branch(repo: &Path, from: &str, to: &str) -> Result<()> {
    git_ok(repo, &["branch", "-m", from, to])?;
    Ok(())
}

/// Remote-tracking refs carrying `branch`'s exact name, as `origin/agent/foo`.
///
/// Distinct from [`is_pushed`], which asks whether the *commits* are safe
/// somewhere: this asks whether the *name* has been published, which is what a
/// local rename cannot take back. Answered from the local ref store, so it is
/// as fresh as the last fetch; a name pushed from another machine and never
/// fetched here reads as unpublished.
pub fn remote_copies(repo: &Path, branch: &str) -> Vec<String> {
    let pattern = format!("refs/remotes/*/{branch}");
    git(repo, &["for-each-ref", "--format=%(refname:short)", &pattern])
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .map(|out| {
            out.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
        })
        .unwrap_or_default()
}

/// The remote a published branch was pushed to, read off its remote-tracking
/// ref: `origin/agent/foo` for `agent/foo` is `origin`.
///
/// Not the first path segment of the ref. Every branch Agency creates has
/// slashes of its own, so splitting on the first one names a remote called
/// "origin" only by luck and calls the branch "agent/foo" something it isn't.
/// `None` when the ref does not end in the branch, which means it was never
/// this branch's copy.
pub fn remote_name<'a>(tracking_ref: &'a str, branch: &str) -> Option<&'a str> {
    let remote = tracking_ref.strip_suffix(branch)?.strip_suffix('/')?;
    (!remote.is_empty()).then_some(remote)
}

/// Delete `branch` from every remote it was published to, once each remote's
/// copy is proved to hold nothing `base` does not already have.
///
/// Returns the remote-tracking refs that went, empty when the branch was
/// already gone from every remote (GitHub deleted it on merge, or another
/// machine did) or was never published at all. Deleting a branch nobody has is
/// what the user asked for, so that is a success with nothing to report, not a
/// failure.
///
/// *Every* copy, not the first one this checkout happens to list. A clone with
/// a fork remote beside `origin` has the branch on both, and taking one while
/// reporting "deleted" leaves behind exactly the branch that hangs around
/// forever, which is the whole thing this is here to stop. Every copy is
/// checked before any is deleted: a refusal on the second remote must not
/// leave the first already gone, with nothing on screen saying which.
///
/// Each remote is asked with `ls-remote` rather than read from its tracking
/// ref: the local one is only as fresh as the last fetch, and a colleague's
/// push landing after it is exactly the work this must not delete unseen. Two
/// refusals guard that. The tip must be an object this checkout actually has —
/// a commit we have never fetched cannot be weighed against anything — and it
/// must be contained in `base`.
///
/// `base` is resolved as the **local** branch, so what the containment check
/// proves is that the work survives in this repo, not that it survives
/// anywhere else. A merge made from the merge window is local and unpushed at
/// the moment this runs, so straight after it the commits are on no remote at
/// all. That is the intended flow, a merge you have not pushed yet is still a
/// merge, but it makes this a guard against losing work rather than a promise
/// about the remote's own history; the window's checkbox says as much.
pub fn delete_published_branch(repo: &Path, branch: &str, base: &str) -> Result<Vec<String>> {
    // Tracking ref, its remote, and whether that remote still has the branch.
    let mut checked: Vec<(String, String, bool)> = Vec::new();
    let head = format!("refs/heads/{branch}");
    for tracking in remote_copies(repo, branch) {
        let Some(remote) = remote_name(&tracking, branch) else {
            bail!("{tracking} does not name a remote copy of {branch}");
        };
        let remote = remote.to_string();
        let out = git_net(repo, &["ls-remote", "--heads", &remote, &head])?;
        if !out.status.success() {
            bail!("couldn't reach {remote}: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        let listing = String::from_utf8_lossy(&out.stdout).to_string();
        let Some(sha) = listing.split_whitespace().next() else {
            checked.push((tracking, remote, false));
            continue;
        };
        if rev(repo, sha).is_none() {
            bail!(
                "{tracking} is at {} on {remote}, a commit this checkout has never fetched; \
                 fetch it and merge that work before deleting the branch",
                &sha[..sha.len().min(10)]
            );
        }
        if !is_ancestor(repo, sha, base) {
            bail!(
                "{tracking} has commits that aren't on {base}; merge them before deleting the \
                 branch, or delete it on the remote yourself"
            );
        }
        checked.push((tracking, remote, true));
    }

    let mut deleted = Vec::new();
    for (tracking, remote, present) in checked {
        if present {
            let out = git_net(repo, &["push", &remote, "--delete", branch])?;
            if !out.status.success() {
                bail!(
                    "deleting {tracking} failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            deleted.push(tracking.clone());
        }
        // `push --delete` prunes the tracking ref itself; this is the backstop
        // for a git that didn't, and the only cleanup in the already-gone case,
        // where the stale ref would otherwise go on offering a branch that
        // isn't there. `-d` refuses nothing, since what it deletes is a
        // remote-tracking ref.
        let _ = git(repo, &["branch", "-r", "-d", &tracking]);
    }
    Ok(deleted)
}

/// The branch the checkout is on, or `None` on detached HEAD.
pub fn current_branch(repo: &Path) -> Option<String> {
    git(repo, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Whether `branch` is already contained in `base` (the merge landed).
pub fn is_merged(repo: &Path, branch: &str, base: &str) -> bool {
    is_ancestor(repo, branch, base)
}

fn is_ancestor(repo: &Path, branch: &str, base: &str) -> bool {
    git(repo, &["merge-base", "--is-ancestor", branch, base])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Whether some remote-tracking branch contains `branch`'s tip, i.e. deleting
/// the local branch would not be the last copy of its commits.
///
/// True for a branch that was pushed for review, and for one whose PR was
/// merged upstream and then fetched. Deliberately answered from the local ref
/// store rather than from `gh`: this is read while a teardown dialog is open,
/// and a dialog that waits on the network is a dialog that hangs. The cost of
/// answering from a stale fetch is one branch kept that could have gone, which
/// is the harmless direction to be wrong in.
pub fn is_pushed(repo: &Path, branch: &str) -> bool {
    git(repo, &["branch", "--remotes", "--contains", branch])
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
}

/// Commits on `branch` that `range_base` does not have, newest first, as
/// `(short hash, subject)`. Capped at `limit` so one record cannot be a
/// thousand lines of log.
pub fn log_commits(
    repo: &Path,
    range_base: &str,
    branch: &str,
    limit: usize,
) -> Result<Vec<(String, String)>> {
    let range = format!("{range_base}..{branch}");
    let out = git_ok(
        repo,
        &["log", "--no-merges", &format!("--max-count={limit}"), "--format=%h\x1f%s", &range],
    )?;
    Ok(out
        .lines()
        .filter_map(|l| l.split_once('\x1f'))
        .map(|(h, s)| (h.trim().to_string(), s.trim().to_string()))
        .collect())
}

/// The commit `branch` was cut from, as far as git can still tell: the merge
/// base with `base`.
///
/// Correct while the branch is unmerged, which is when it is asked. Once the
/// branch has landed the merge base *is* the branch tip, so the range it gives
/// is empty — see `Run::base_commit`, which is why the base commit is recorded
/// at creation and this is only the fallback for runs that predate it.
pub fn fork_point(repo: &Path, branch: &str, base: &str) -> Option<String> {
    git_ok(repo, &["merge-base", base, branch])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn merge_state(repo: &Path, branch: &str, base: &str) -> Result<MergeState> {
    let in_progress = in_progress_merge(repo);
    // Ownership, not just existence: an unfinished merge belonging to another
    // run would otherwise read as this run's own conflict in every window that
    // asks — and "Finish merge" there would commit that other merge and close
    // this run's issue for work it never landed.
    let mine =
        in_progress.as_ref().is_some_and(|m| rev(repo, branch).as_deref() == Some(&m.commit));
    Ok(MergeState {
        merging: mine,
        unresolved: if mine { unmerged_files(repo)? } else { Vec::new() },
        merged: is_ancestor(repo, branch, base),
        blocked_by: in_progress.filter(|_| !mine).map(|m| m.branch),
    })
}

/// Commit an in-progress merge whose conflicts have been resolved, then
/// restore the checkout's original branch. Already-committed merges (the
/// resolver ran `git commit` itself) fall through to the same restore-and-
/// report path, so both endings look the same to the caller.
pub fn finish_merge(repo: &Path, restore_to: Option<&str>) -> Result<String> {
    if is_merging(repo)? {
        let unresolved = unmerged_files(repo)?;
        if !unresolved.is_empty() {
            bail!("{} file(s) still conflicted: {}", unresolved.len(), unresolved.join(", "));
        }
        // `--no-edit` keeps git's own merge message; `--no-verify` keeps a
        // repo's commit hooks from blocking the completion of a merge the
        // user has already resolved.
        let out = git(repo, &["commit", "--no-edit", "--no-verify"])?;
        if !out.status.success() {
            bail!("completing the merge failed: {}", String::from_utf8_lossy(&out.stderr));
        }
    }
    let commit = git_ok(repo, &["rev-parse", "HEAD"])?.trim().to_string();
    if let Some(orig) = restore_to.filter(|o| !o.is_empty()) {
        let _ = git(repo, &["checkout", orig]);
    }
    Ok(commit)
}

fn step(on_progress: &mut dyn FnMut(crate::setup::CloneProgress), phase: &str, detail: &str) {
    on_progress(crate::setup::CloneProgress {
        phase: phase.to_string(),
        percent: None,
        detail: detail.to_string(),
    });
}

pub fn merge(repo: &Path, branch: &str, base: &str) -> Result<MergeOutcome> {
    merge_with_progress(repo, branch, base, &mut |_| {})
}

/// [`merge`] reporting the step it is on. The slow parts are git's, not ours:
/// checking out the base branch rewrites the working tree, and the merge then
/// rewrites it again. On a large repo that is seconds each, and the caller is a
/// modal with nothing else to say meanwhile. No percentages: git reports none
/// for either, so the readout sweeps rather than lying about a fraction.
pub fn merge_with_progress(
    repo: &Path,
    branch: &str,
    base: &str,
    on_progress: &mut dyn FnMut(crate::setup::CloneProgress),
) -> Result<MergeOutcome> {
    step(on_progress, "Checking the project's checkout", base);
    // An unfinished merge is dirty by construction, so check for it first:
    // otherwise it reports as "uncommitted changes" and sends the user off to
    // stash work that is actually a half-done merge.
    if let Some(m) = in_progress_merge(repo) {
        bail!(
            "the project's main checkout is mid-merge of {}; finish or abort that merge first",
            m.branch
        );
    }
    let dirty = git_ok(repo, &["status", "--porcelain"])?;
    if !dirty.trim().is_empty() {
        bail!("the project's main checkout has uncommitted changes; commit or stash them there before merging");
    }
    // Remember which branch the main checkout was on so a clean merge can put
    // it back — merging shouldn't hijack the user's checkout as a side effect.
    // Detached HEAD yields nothing and skips the restore.
    let original = current_branch(repo);
    step(on_progress, &format!("Switching to {base}"), "");
    git_ok(repo, &["checkout", base])?;
    step(on_progress, &format!("Merging {branch}"), &format!("into {base}"));
    let out = git(repo, &["merge", "--no-ff", branch])?;
    if out.status.success() {
        let commit = git_ok(repo, &["rev-parse", "HEAD"])?.trim().to_string();
        // Best-effort: a failed restore must not turn a successful merge into
        // an error. On conflicts we intentionally stay on `base` — resolution
        // (manual or agent-driven) happens there.
        if let Some(orig) = original.filter(|o| o != base) {
            step(on_progress, &format!("Switching back to {orig}"), "");
            let _ = git(repo, &["checkout", &orig]);
        }
        return Ok(MergeOutcome::Clean { commit });
    }
    // Distinguish conflicts from other failures.
    let conflicts = unmerged_files(repo)?;
    if !conflicts.is_empty() {
        Ok(MergeOutcome::Conflicts { files: conflicts })
    } else {
        bail!("merge failed: {}", String::from_utf8_lossy(&out.stderr))
    }
}
