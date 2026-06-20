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

pub fn stage(worktree: &Path, path: &str) -> Result<()> {
    git(worktree, &["add", "--", path])?;
    Ok(())
}

pub fn unstage(worktree: &Path, path: &str) -> Result<()> {
    // `restore --staged` requires git >= 2.23; available on all supported setups.
    git(worktree, &["restore", "--staged", "--", path])?;
    Ok(())
}

pub fn commit(worktree: &Path, message: &str) -> Result<()> {
    git(worktree, &["commit", "-m", message])?;
    Ok(())
}

pub fn push(worktree: &Path) -> Result<()> {
    let branch = git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    git(worktree, &["push", "-u", "origin", &branch])?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffStat {
    pub added: u32,
    pub deleted: u32,
    pub files: u32,
}

pub fn diff_stat(worktree: &Path, base: &str) -> Result<DiffStat> {
    let range = format!("{base}...HEAD");
    let out = git(worktree, &["diff", "--numstat", &range])?;
    let mut stat = DiffStat { added: 0, deleted: 0, files: 0 };
    for line in out.lines() {
        let mut parts = line.split('\t');
        let a = parts.next().unwrap_or("0");
        let d = parts.next().unwrap_or("0");
        // Binary files show "-" for counts; treat as 0 but still count the file.
        stat.added += a.parse::<u32>().unwrap_or(0);
        stat.deleted += d.parse::<u32>().unwrap_or(0);
        stat.files += 1;
    }
    Ok(stat)
}
