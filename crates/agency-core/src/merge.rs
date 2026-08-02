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
    /// `MERGE_HEAD` exists: a merge is started but not yet committed.
    pub merging: bool,
    /// Paths git still considers unmerged, either because conflict markers
    /// remain or because the resolution was never `git add`ed.
    pub unresolved: Vec<String>,
    /// The branch tip is already an ancestor of the base branch, i.e. the
    /// merge landed (whoever committed it).
    pub merged: bool,
}

fn git(repo: &Path, args: &[&str]) -> Result<std::process::Output> {
    Ok(Command::new("git").args(args).current_dir(repo).output()?)
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

fn unmerged_files(repo: &Path) -> Result<Vec<String>> {
    let out = git_ok(repo, &["diff", "--name-only", "--diff-filter=U"])?;
    Ok(out.lines().map(|l| l.to_string()).collect())
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
fn is_ancestor(repo: &Path, branch: &str, base: &str) -> bool {
    git(repo, &["merge-base", "--is-ancestor", branch, base])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn merge_state(repo: &Path, branch: &str, base: &str) -> Result<MergeState> {
    let merging = is_merging(repo)?;
    Ok(MergeState {
        merging,
        unresolved: if merging { unmerged_files(repo)? } else { Vec::new() },
        merged: is_ancestor(repo, branch, base),
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
            bail!(
                "{} file(s) still conflicted: {}",
                unresolved.len(),
                unresolved.join(", ")
            );
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

pub fn merge(repo: &Path, branch: &str, base: &str) -> Result<MergeOutcome> {
    // An unfinished merge is dirty by construction, so check for it first:
    // otherwise it reports as "uncommitted changes" and sends the user off to
    // stash work that is actually a half-done merge.
    if is_merging(repo)? {
        bail!("a merge is already in progress in the project's main checkout; finish or abort it first");
    }
    let dirty = git_ok(repo, &["status", "--porcelain"])?;
    if !dirty.trim().is_empty() {
        bail!("the project's main checkout has uncommitted changes; commit or stash them there before merging");
    }
    // Remember which branch the main checkout was on so a clean merge can put
    // it back — merging shouldn't hijack the user's checkout as a side effect.
    // Detached HEAD yields nothing and skips the restore.
    let original = current_branch(repo);
    git_ok(repo, &["checkout", base])?;
    let out = git(repo, &["merge", "--no-ff", branch])?;
    if out.status.success() {
        let commit = git_ok(repo, &["rev-parse", "HEAD"])?.trim().to_string();
        // Best-effort: a failed restore must not turn a successful merge into
        // an error. On conflicts we intentionally stay on `base` — resolution
        // (manual or agent-driven) happens there.
        if let Some(orig) = original.filter(|o| o != base) {
            let _ = git(repo, &["checkout", &orig]);
        }
        return Ok(MergeOutcome::Clean { commit });
    }
    // Distinguish conflicts from other failures.
    let conflicts = unmerged_files(repo)?;
    if !conflicts.is_empty() {
        Ok(MergeOutcome::Conflicts { files: conflicts })
    } else {
        bail!(
            "merge failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )
    }
}
