use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    /// Staged (index) status code: one of M A D R C ? ! or space.
    pub index: String,
    /// Unstaged (worktree) status code.
    pub worktree: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitInfo {
    pub hash: String,
    pub summary: String,
}

/// Run a git command in `worktree`, returning stdout on success.
fn git(worktree: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .output()?;
    if !output.status.success() {
        bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn status(worktree: &Path) -> Result<Vec<FileChange>> {
    let out = git(worktree, &["status", "--porcelain"])?;
    let mut changes = Vec::new();
    for line in out.lines() {
        if line.len() < 4 {
            continue;
        }
        let index = line[0..1].to_string();
        let work = line[1..2].to_string();
        // Path begins at column 3 (after "XY ").
        let mut path = line[3..].to_string();
        // Renames are "orig -> new"; keep the new path.
        if let Some(idx) = path.find(" -> ") {
            path = path[idx + 4..].to_string();
        }
        changes.push(FileChange {
            path,
            index,
            worktree: work,
        });
    }
    Ok(changes)
}

pub fn diff(worktree: &Path, path: &str, staged: bool) -> Result<String> {
    if staged {
        git(worktree, &["diff", "--cached", "--", path])
    } else {
        git(worktree, &["diff", "--", path])
    }
}

pub fn log(worktree: &Path, limit: usize) -> Result<Vec<CommitInfo>> {
    let limit_arg = format!("-n{limit}");
    // Tab-separated hash\tsummary, one commit per line.
    let out = git(
        worktree,
        &["log", &limit_arg, "--format=%H%x09%s"],
    )?;
    let mut commits = Vec::new();
    for line in out.lines() {
        if let Some((hash, summary)) = line.split_once('\t') {
            commits.push(CommitInfo {
                hash: hash.to_string(),
                summary: summary.to_string(),
            });
        }
    }
    Ok(commits)
}
