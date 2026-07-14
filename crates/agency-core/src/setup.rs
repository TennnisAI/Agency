use crate::gh::{GhCli, GhReadiness};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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

/// Derive a destination folder name from a git remote URL. Handles the common
/// forms — `https://host/owner/repo.git`, `git@host:owner/repo.git`, and
/// trailing slashes — by taking the final path segment and stripping `.git`.
pub fn repo_name_from_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    // scp-style `git@host:owner/repo` separates the path with ':'; splitting on
    // both '/' and ':' lands on the final segment for either URL shape.
    let last = trimmed.rsplit(|c| c == '/' || c == ':').next().unwrap_or(trimmed);
    last.strip_suffix(".git").unwrap_or(last).to_string()
}

/// True when the URL points at github.com (https or scp-style), the only host
/// the `gh` fallback can authenticate for.
fn is_github_url(url: &str) -> bool {
    let u = url.trim();
    u.starts_with("https://github.com/")
        || u.starts_with("http://github.com/")
        || u.starts_with("git@github.com:")
        || u.starts_with("ssh://git@github.com/")
}

/// True when git's failure is about credentials, not a missing repo or network.
/// Covers the interactive-prompt case (`GIT_TERMINAL_PROMPT=0` turns the cryptic
/// "could not read Username … Device not configured" into "terminal prompts
/// disabled") and an outright rejected sign-in.
fn is_auth_failure(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("could not read username")
        || s.contains("could not read password")
        || s.contains("terminal prompts disabled")
        || s.contains("authentication failed")
        || s.contains("invalid username or password")
        || s.contains("permission denied")
}

/// Build an actionable message for a GitHub clone that failed on authentication,
/// tailored to how far the user is from being able to sign in. The frontend
/// keys off the `gh auth login` / `cli.github.com` hints to offer the fix.
fn github_auth_message(url: &str, gh: &GhCli) -> String {
    let ssh_hint = "Or use an SSH URL (git@github.com:owner/repo.git) if you have SSH keys set up.";
    match gh.auth_readiness() {
        GhReadiness::NotInstalled => format!(
            "This repository needs you to sign in to GitHub, and the GitHub CLI (gh) isn't installed. \
             Install it from https://cli.github.com and run `gh auth login`, then try again.\n\n{ssh_hint}"
        ),
        GhReadiness::NotAuthenticated => format!(
            "This repository needs you to sign in to GitHub. Open a terminal, run `gh auth login` \
             to sign in with the GitHub CLI, then try cloning again.\n\n{ssh_hint}"
        ),
        // Signed in but the clone still failed → most likely no access, or a typo
        // in the URL. Don't send them down the sign-in path they've already done.
        _ => format!(
            "Couldn't access {url}. You're signed in to GitHub, so this repository is probably \
             private and your account doesn't have access, or the URL is mistyped. \
             Double-check the URL and that your GitHub account can see this repository."
        ),
    }
}

/// Clone `url` into a new folder under `parent_dir`, named after the repo, and
/// return the new path. The destination must not already exist — git refuses to
/// clone into a non-empty dir, but checking up front yields a clearer message.
///
/// Credentials are the usual snag: the desktop app has no TTY, so a private repo
/// makes `git` try (and fail) to read a username from a terminal that isn't
/// there. We disable that prompt so the failure is fast and legible, then —
/// for github.com — retry through `gh`, which supplies the user's stored
/// credentials. That makes an authenticated private clone just work; when it
/// can't, `auth_error_message` says exactly what to configure.
pub fn clone_repo(url: &str, parent_dir: &Path) -> Result<PathBuf> {
    let name = repo_name_from_url(url);
    if name.is_empty() {
        bail!("couldn't determine a folder name from the URL");
    }
    let dest = parent_dir.join(&name);
    if dest.exists() {
        bail!("{} already exists — choose another location", dest.display());
    }
    let out = Command::new("git")
        .arg("clone")
        .arg(url)
        .arg(&dest)
        .current_dir(parent_dir)
        // No TTY in the app: fail fast instead of blocking on a prompt git can't read.
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if out.status.success() {
        return Ok(dest);
    }
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    if is_auth_failure(&stderr) {
        if is_github_url(url) {
            let gh = GhCli::default();
            // For github.com, gh can inject credentials where bare git couldn't.
            // If gh fails too (no access, bad URL), fall through to guidance.
            if matches!(gh.auth_readiness(), GhReadiness::Ready) && gh.clone(url, &dest).is_ok() {
                return Ok(dest);
            }
            bail!("{}", github_auth_message(url, &gh));
        }
        // Non-GitHub host: we can't broker credentials, so point at the general fix.
        bail!(
            "This repository needs credentials to clone. Set up a git credential helper \
             for its host, or use an SSH URL if you have SSH keys configured.\n\n{}",
            stderr.trim()
        );
    }
    bail!("git clone failed: {}", stderr.trim());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_github_urls() {
        assert!(is_github_url("https://github.com/owner/repo"));
        assert!(is_github_url("https://github.com/owner/repo.git"));
        assert!(is_github_url("git@github.com:owner/repo.git"));
        assert!(is_github_url("ssh://git@github.com/owner/repo.git"));
        // Non-github hosts and lookalikes must not match.
        assert!(!is_github_url("https://gitlab.com/owner/repo.git"));
        assert!(!is_github_url("https://github.com.evil.com/owner/repo"));
        assert!(!is_github_url("https://mygithub.com/owner/repo"));
    }

    #[test]
    fn detects_credential_failures_not_other_errors() {
        // The real stderr the desktop app hits (prompt disabled) and rejected auth.
        assert!(is_auth_failure(
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled"
        ));
        assert!(is_auth_failure("remote: Support for password authentication was removed.\nfatal: Authentication failed"));
        assert!(is_auth_failure("git@github.com: Permission denied (publickey)."));
        // A missing repo or DNS failure is NOT an auth problem.
        assert!(!is_auth_failure("fatal: repository 'https://github.com/x/y' not found"));
        assert!(!is_auth_failure("fatal: unable to access ... Could not resolve host"));
    }
}
