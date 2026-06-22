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

    /// Ensure agency's local artifacts are git-excluded without ignoring the
    /// tracked `.agency/agency.toml`. Writes `.agency/worktrees/` and
    /// `.agency/agency.local.toml`, and migrates away the legacy broad
    /// `.agency/` entry if present.
    pub(crate) fn ensure_excluded(&self) -> Result<()> {
        let exclude = self.repo_path.join(".git").join("info").join("exclude");
        let current = std::fs::read_to_string(&exclude).unwrap_or_default();
        let wanted = [".agency/worktrees/", ".agency/agency.local.toml"];

        let had_legacy = current.lines().any(|l| l.trim() == ".agency/");
        let mut lines: Vec<String> = current
            .lines()
            .filter(|l| l.trim() != ".agency/")
            .map(|l| l.to_string())
            .collect();

        let mut changed = had_legacy;
        for w in wanted {
            if !lines.iter().any(|l| l.trim() == w) {
                lines.push(w.to_string());
                changed = true;
            }
        }

        if changed {
            if let Some(parent) = exclude.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut out = lines.join("\n");
            out.push('\n');
            std::fs::write(&exclude, out)?;
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

    /// Remove the worktree and delete its branch. Tolerant: each git step is
    /// best-effort so it works whether or not the worktree still exists (e.g.
    /// discarding an already-archived run), and still deletes the branch.
    pub fn remove(&self, task_id: &str) -> Result<()> {
        let path = self.worktrees_root().join(task_id);
        let path_str = path.to_string_lossy().to_string();
        let _ = self.git(&["worktree", "remove", &path_str, "--force"]);
        let _ = self.git(&["worktree", "prune"]);
        let branch = Self::branch_for(task_id);
        let _ = self.git(&["branch", "-D", &branch]);
        Ok(())
    }

    /// Remove the worktree but KEEP the branch, so the work can be restored.
    pub fn remove_keep_branch(&self, task_id: &str) -> Result<()> {
        let path = self.worktrees_root().join(task_id);
        let path_str = path.to_string_lossy().to_string();
        self.git(&["worktree", "remove", &path_str, "--force"])?;
        self.git(&["worktree", "prune"])?;
        Ok(())
    }

    /// Re-create a worktree for `task_id` on its existing branch `agent/<id>`.
    pub fn restore(&self, task_id: &str) -> Result<Worktree> {
        self.ensure_excluded()?;
        let path = self.worktrees_root().join(task_id);
        let branch = Self::branch_for(task_id);
        let path_str = path.to_string_lossy().to_string();
        self.git(&["worktree", "add", &path_str, &branch])?;
        Ok(Worktree { task_id: task_id.to_string(), path, branch })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn exclude_path(repo: &std::path::Path) -> std::path::PathBuf {
        repo.join(".git").join("info").join("exclude")
    }

    #[test]
    fn ensure_excluded_writes_narrow_entries() {
        let dir = tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        fs::create_dir_all(repo.join(".git").join("info")).unwrap();
        let mgr = WorktreeManager::new(repo.clone());
        mgr.ensure_excluded().unwrap();
        let body = fs::read_to_string(exclude_path(&repo)).unwrap();
        let lines: Vec<&str> = body.lines().map(|l| l.trim()).collect();
        assert!(lines.contains(&".agency/worktrees/"));
        assert!(lines.contains(&".agency/agency.local.toml"));
        assert!(!lines.contains(&".agency/"));
    }

    #[test]
    fn ensure_excluded_migrates_legacy_entry() {
        let dir = tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let info = repo.join(".git").join("info");
        fs::create_dir_all(&info).unwrap();
        fs::write(info.join("exclude"), "# existing\n.agency/\n").unwrap();
        let mgr = WorktreeManager::new(repo.clone());
        mgr.ensure_excluded().unwrap();
        let body = fs::read_to_string(exclude_path(&repo)).unwrap();
        let lines: Vec<&str> = body.lines().map(|l| l.trim()).collect();
        assert!(lines.contains(&"# existing"), "preserves unrelated lines");
        assert!(!lines.contains(&".agency/"), "drops legacy broad ignore");
        assert!(lines.contains(&".agency/worktrees/"));
    }

    #[test]
    fn ensure_excluded_is_idempotent() {
        let dir = tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        fs::create_dir_all(repo.join(".git").join("info")).unwrap();
        let mgr = WorktreeManager::new(repo.clone());
        mgr.ensure_excluded().unwrap();
        let first = fs::read_to_string(exclude_path(&repo)).unwrap();
        mgr.ensure_excluded().unwrap();
        let second = fs::read_to_string(exclude_path(&repo)).unwrap();
        assert_eq!(first, second);
    }
}
