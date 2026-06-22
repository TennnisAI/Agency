use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RepoReadiness {
    NotARepo,
    NoCommits { stageable: bool },
    Ready { dirty: bool },
}

/// Run a git command in `dir`, returning (success, stdout).
fn git(dir: &Path, args: &[&str]) -> std::io::Result<(bool, String)> {
    let out = Command::new("git").args(args).current_dir(dir).output()?;
    Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).to_string()))
}

/// Inspect a folder and classify how ready it is to host agent worktrees.
/// Worktrees branch from `HEAD`, so a repo needs at least one commit to be `Ready`.
pub fn repo_readiness(path: &Path) -> RepoReadiness {
    let inside = git(path, &["rev-parse", "--is-inside-work-tree"]);
    if !matches!(inside, Ok((true, _))) {
        return RepoReadiness::NotARepo;
    }
    // HEAD resolves only when at least one commit exists.
    let has_head = matches!(git(path, &["rev-parse", "--verify", "HEAD"]), Ok((true, _)));
    if !has_head {
        // Anything that `git add -A` would stage means a non-empty initial commit is possible.
        let stageable = match git(path, &["status", "--porcelain"]) {
            Ok((true, out)) => !out.trim().is_empty(),
            _ => false,
        };
        return RepoReadiness::NoCommits { stageable };
    }
    let dirty = match git(path, &["status", "--porcelain"]) {
        Ok((true, out)) => !out.trim().is_empty(),
        _ => false,
    };
    RepoReadiness::Ready { dirty }
}

const DEFAULT_GITIGNORE: &str = "node_modules/\n.env\ndist/\ntarget/\n.DS_Store\n";

/// Run a git command in `dir`, returning Err with stderr on failure.
fn git_checked(dir: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git").args(args).current_dir(dir).output()?;
    if !out.status.success() {
        bail!("git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

/// `git init` in `path`. Uses the user's configured default branch name.
pub fn init_repo(path: &Path) -> Result<()> {
    git_checked(path, &["init"])
}

/// Write a sensible default `.gitignore`, but never overwrite an existing one.
pub fn write_default_gitignore(path: &Path) -> Result<()> {
    let gi = path.join(".gitignore");
    if !gi.exists() {
        std::fs::write(&gi, DEFAULT_GITIGNORE)?;
    }
    Ok(())
}

/// Stage everything and make the initial commit. Optionally writes a default
/// `.gitignore` first. Falls back to `--allow-empty` when nothing is staged
/// (empty or fully-ignored folder), so the repo still gains a usable `HEAD`.
pub fn initial_commit(path: &Path, add_gitignore: bool) -> Result<()> {
    if add_gitignore {
        write_default_gitignore(path)?;
    }
    git_checked(path, &["add", "-A"])?;
    // Nothing staged → empty commit so HEAD exists and worktrees can branch.
    let staged = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(path)
        .status()?;
    let mut commit_args = vec!["commit", "-m", "Initial commit"];
    if staged.success() {
        // exit 0 from --quiet means no staged changes.
        commit_args.push("--allow-empty");
    }
    git_checked(path, &commit_args)
}
