use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Worktree {
    pub task_id: String,
    pub path: PathBuf,
    pub branch: String,
}

pub struct WorktreeManager {
    repo_path: PathBuf,
}

impl WorktreeManager {
    /// Creates a new `WorktreeManager` for the given repo path.
    /// Canonicalizes the path so it matches git's resolved paths (important on macOS
    /// where `/tmp` is a symlink to `/private/tmp`).
    pub fn new(repo_path: PathBuf) -> WorktreeManager {
        let repo_path = repo_path.canonicalize().unwrap_or(repo_path);
        WorktreeManager { repo_path }
    }

    fn worktrees_root(&self) -> PathBuf {
        self.repo_path.join(".agency").join("worktrees")
    }

    fn branch_for(task_id: &str) -> String {
        format!("agent/{task_id}")
    }

    /// Run a git command in the repo, returning stdout on success.
    fn git(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.repo_path)
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

    /// Ensure `.agency/` is excluded so worktrees never show as untracked.
    fn ensure_excluded(&self) -> Result<()> {
        let exclude = self.repo_path.join(".git").join("info").join("exclude");
        let current = std::fs::read_to_string(&exclude).unwrap_or_default();
        if !current.lines().any(|l| l.trim() == ".agency/") {
            if let Some(parent) = exclude.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut updated = current;
            if !updated.is_empty() && !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(".agency/\n");
            std::fs::write(&exclude, updated)?;
        }
        Ok(())
    }

    pub fn create(&self, task_id: &str, base: &str) -> Result<Worktree> {
        self.ensure_excluded()?;
        let path = self.worktrees_root().join(task_id);
        let branch = Self::branch_for(task_id);
        let path_str = path.to_string_lossy().to_string();
        self.git(&["worktree", "add", &path_str, "-b", &branch, base])?;
        Ok(Worktree {
            task_id: task_id.to_string(),
            path,
            branch,
        })
    }

    pub fn list(&self) -> Result<Vec<Worktree>> {
        let out = self.git(&["worktree", "list", "--porcelain"])?;
        let root = self.worktrees_root();
        let mut worktrees = Vec::new();
        let mut cur_path: Option<PathBuf> = None;
        let mut cur_branch: Option<String> = None;

        let flush = |path: &mut Option<PathBuf>, branch: &mut Option<String>, acc: &mut Vec<Worktree>| {
            if let (Some(p), Some(b)) = (path.take(), branch.take()) {
                if p.starts_with(&root) {
                    if let Some(task_id) = p.file_name().and_then(|s| s.to_str()) {
                        acc.push(Worktree {
                            task_id: task_id.to_string(),
                            path: p.clone(),
                            branch: b,
                        });
                    }
                }
            }
        };

        for line in out.lines() {
            if let Some(rest) = line.strip_prefix("worktree ") {
                flush(&mut cur_path, &mut cur_branch, &mut worktrees);
                cur_path = Some(PathBuf::from(rest));
            } else if let Some(rest) = line.strip_prefix("branch ") {
                cur_branch = Some(rest.trim_start_matches("refs/heads/").to_string());
            }
        }
        flush(&mut cur_path, &mut cur_branch, &mut worktrees);
        Ok(worktrees)
    }

    pub fn remove(&self, task_id: &str) -> Result<()> {
        let path = self.worktrees_root().join(task_id);
        let path_str = path.to_string_lossy().to_string();
        self.git(&["worktree", "remove", &path_str, "--force"])?;
        // Branch deletion is best-effort; ignore failure (e.g. already merged/gone).
        let branch = Self::branch_for(task_id);
        let _ = self.git(&["branch", "-D", &branch]);
        Ok(())
    }
}
