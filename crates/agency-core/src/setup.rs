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
