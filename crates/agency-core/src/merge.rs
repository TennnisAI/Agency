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

pub fn is_merging(repo: &Path) -> Result<bool> {
    Ok(ref_exists(repo, "MERGE_HEAD"))
}

pub fn abort_merge(repo: &Path) -> Result<()> {
    git_ok(repo, &["merge", "--abort"])?;
    Ok(())
}

fn unmerged_files(repo: &Path) -> Result<Vec<String>> {
    let out = git_ok(repo, &["diff", "--name-only", "--diff-filter=U"])?;
    Ok(out.lines().map(|l| l.to_string()).collect())
}

pub fn merge(repo: &Path, branch: &str, base: &str) -> Result<MergeOutcome> {
    git_ok(repo, &["checkout", base])?;
    let out = git(repo, &["merge", "--no-ff", branch])?;
    if out.status.success() {
        let commit = git_ok(repo, &["rev-parse", "HEAD"])?.trim().to_string();
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
