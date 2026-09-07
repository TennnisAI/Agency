use crate::setup::CloneProgress;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

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
        Self::git_at(&self.repo_path, args)
    }

    /// Run a git command in an arbitrary directory (e.g. inside a worktree).
    fn git_at(dir: &std::path::Path, args: &[&str]) -> Result<String> {
        let output = Command::new("git").args(args).current_dir(dir).output()?;
        if !output.status.success() {
            bail!("git {:?} failed: {}", args, String::from_utf8_lossy(&output.stderr));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Commit every pending change (tracked and untracked) in the task's
    /// worktree onto its `agent/<id>` branch. Returns whether a commit was
    /// created; a clean or missing worktree is a no-op. Called before archive
    /// so `worktree remove --force` can't destroy uncommitted agent work —
    /// the branch is kept, so the commit comes back on restore.
    pub fn commit_all_if_dirty(&self, task_id: &str, message: &str) -> Result<bool> {
        let path = self.worktrees_root().join(task_id);
        if !path.exists() {
            return Ok(false);
        }
        let dirty = Self::git_at(&path, &["status", "--porcelain"])?;
        if dirty.trim().is_empty() {
            return Ok(false);
        }
        Self::git_at(&path, &["add", "-A"])?;
        Self::git_at(&path, &["commit", "-m", message])?;
        Ok(true)
    }

    /// Copy repo-root paths into the task's worktree. `git worktree add` only
    /// materializes tracked files, so untracked-but-needed ones (`.env` and
    /// friends, listed under `[files] copy` in `.agency/agency.toml`) must be
    /// copied over. Relative paths only; entries that are absolute, escape the
    /// repo via `..`, or don't exist are skipped. Directories copy recursively.
    /// Returns the entries actually copied.
    pub fn copy_into(&self, task_id: &str, rel_paths: &[String]) -> Result<Vec<String>> {
        let dest_root = self.worktrees_root().join(task_id);
        let mut copied = Vec::new();
        for rel in rel_paths {
            let rel_path = std::path::Path::new(rel);
            if rel_path.is_absolute()
                || rel_path.components().any(|c| matches!(c, std::path::Component::ParentDir))
            {
                continue;
            }
            let src = self.repo_path.join(rel_path);
            if !src.exists() {
                continue;
            }
            let dest = dest_root.join(rel_path);
            copy_recursive(&src, &dest)?;
            copied.push(rel.clone());
        }
        Ok(copied)
    }

    /// Copy the configured `[files] copy` list plus any auto-detected untracked
    /// root `.env` files into the worktree. This is the entry point run at every
    /// worktree create/restore — it unions the explicit list with the env
    /// defaults (config order kept, duplicates dropped) so a project's local
    /// `.env` reaches agent workspaces even when nothing is configured.
    pub fn copy_essentials(&self, task_id: &str, configured: &[String]) -> Result<Vec<String>> {
        let mut list: Vec<String> = configured.to_vec();
        for env in self.default_env_files() {
            if !list.iter().any(|p| p == &env) {
                list.push(env);
            }
        }
        self.copy_into(task_id, &list)
    }

    /// Root-level `.env` / `.env.*` files that git does not track (untracked or
    /// ignored). Tracked env files already materialize in a worktree, so only
    /// the untracked ones need copying. Returns repo-root-relative names, sorted;
    /// best-effort (empty on any io/git error).
    pub fn default_env_files(&self) -> Vec<String> {
        let mut candidates = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.repo_path) else {
            return candidates;
        };
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".env" || name.starts_with(".env.") {
                candidates.push(name);
            }
        }
        if candidates.is_empty() {
            return candidates;
        }
        let tracked = self.tracked_subset(&candidates);
        let mut names: Vec<String> =
            candidates.into_iter().filter(|n| !tracked.contains(n)).collect();
        names.sort();
        names
    }

    /// The subset of `rels` (repo-root-relative names) that git tracks. One git
    /// invocation for the whole set; best-effort (empty on any git error).
    fn tracked_subset(&self, rels: &[String]) -> std::collections::HashSet<String> {
        let output = Command::new("git")
            .args(["ls-files", "-z", "--"])
            .args(rels)
            .current_dir(&self.repo_path)
            .output();
        let Ok(output) = output else {
            return Default::default();
        };
        if !output.status.success() {
            return Default::default();
        }
        output
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect()
    }

    /// Ensure agency's local artifacts are git-excluded without ignoring the
    /// tracked `.agency/agency.toml`. Writes `.agency/worktrees/` and
    /// `.agency/agency.local.toml`, and migrates away the legacy broad
    /// `.agency/` entry if present.
    pub(crate) fn ensure_excluded(&self) -> Result<()> {
        ensure_agency_excludes(&self.repo_path)
    }

    pub fn create(&self, task_id: &str, base: &str) -> Result<Worktree> {
        self.create_with_progress(task_id, base, &mut |_| {})
    }

    /// Like [`create`], but streams progress as git checks out the tree. On a
    /// large repo the checkout is seconds-to-minutes of work; `git worktree add`
    /// reports no percentage to a pipe, so progress is approximated from how many
    /// files have materialized in the new worktree vs. the repo's tracked count —
    /// enough to show the app is working instead of frozen.
    pub fn create_with_progress(
        &self,
        task_id: &str,
        base: &str,
        on_progress: &mut dyn FnMut(CloneProgress),
    ) -> Result<Worktree> {
        self.ensure_excluded()?;
        let path = self.worktrees_root().join(task_id);
        let branch = Self::branch_for(task_id);
        let path_str = path.to_string_lossy().to_string();
        self.run_worktree_add(
            &["worktree", "add", &path_str, "-b", &branch, base],
            &path,
            on_progress,
        )?;
        Ok(Worktree { task_id: task_id.to_string(), path, branch })
    }

    /// Run a `git worktree add …` command, polling the destination's file count
    /// so `on_progress` shows the checkout advancing. git prints no progress for
    /// this to a pipe, so the fraction is files-materialized / files-tracked; the
    /// denominator is best-effort and the bar caps at 99% until git returns.
    fn run_worktree_add(
        &self,
        args: &[&str],
        dest: &std::path::Path,
        on_progress: &mut dyn FnMut(CloneProgress),
    ) -> Result<()> {
        let total = self.tracked_file_count();
        let emit = |on_progress: &mut dyn FnMut(CloneProgress), n: u64| {
            let (percent, detail) = match total {
                Some(t) if t > 0 => (Some(((n * 100 / t).min(99)) as u8), format!("{n}/{t} files")),
                _ => (None, format!("{n} files")),
            };
            on_progress(CloneProgress { phase: "Setting up workspace".into(), percent, detail });
        };
        emit(on_progress, 0);

        // stderr/stdout are piped but only read after the child exits. Safe here
        // because `git worktree add` prints only a couple of short lines; a chatty
        // command could fill the pipe and stall, so don't reuse this blindly.
        let mut child = Command::new("git")
            .args(args)
            .current_dir(&self.repo_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut last = 0u64;
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            let n = count_files(dest);
            if n != last {
                last = n;
                emit(on_progress, n);
            }
            std::thread::sleep(Duration::from_millis(150));
        }

        let output = child.wait_with_output()?;
        if !output.status.success() {
            bail!("git {:?} failed: {}", args, String::from_utf8_lossy(&output.stderr));
        }
        Ok(())
    }

    /// Number of files git tracks in this repo, the denominator for create
    /// progress. `None` when git can't be consulted; approximate (the index of
    /// the current checkout, not necessarily `base`), which is fine for a bar.
    fn tracked_file_count(&self) -> Option<u64> {
        let output = Command::new("git")
            .args(["ls-files", "-z"])
            .current_dir(&self.repo_path)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let n = output.stdout.iter().filter(|b| **b == 0).count() as u64;
        (n > 0).then_some(n)
    }

    pub fn list(&self) -> Result<Vec<Worktree>> {
        let out = self.git(&["worktree", "list", "--porcelain"])?;
        let root = self.worktrees_root();
        let mut worktrees = Vec::new();
        let mut cur_path: Option<PathBuf> = None;
        let mut cur_branch: Option<String> = None;

        let flush =
            |path: &mut Option<PathBuf>, branch: &mut Option<String>, acc: &mut Vec<Worktree>| {
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

    /// Every worktree this repo holds an admin entry for, at the path git has
    /// recorded (so a stale one after a move), the main worktree excluded.
    /// `list` answers the same question for Agency's own runs; this one is
    /// about the whole repo, the user's own worktrees included.
    fn recorded_worktree_paths(&self) -> Vec<PathBuf> {
        let Ok(out) = self.git(&["worktree", "list", "--porcelain"]) else {
            return Vec::new();
        };
        // The first block is always the main worktree, which git resolves from
        // the working directory and so always reports at its real path.
        out.lines().filter_map(|l| l.strip_prefix("worktree ")).skip(1).map(PathBuf::from).collect()
    }

    /// Where a worktree git still records at `stale` has ended up, if it moved
    /// as part of this project folder: the longest tail of `stale` that names a
    /// worktree inside the folder's new home.
    ///
    /// The old folder path is nowhere on disk to subtract, so the tail is found
    /// by trying each one, deepest first, so that a `wt/feature` is preferred
    /// to a `feature` at the root. A candidate counts only if its `.git` is a
    /// *file* naming a worktree admin directory: that is what a linked worktree
    /// has, and what a submodule (`gitdir: …/.git/modules/…`) does not.
    fn relocated_to(&self, stale: &std::path::Path) -> Option<PathBuf> {
        let comps: Vec<_> = stale.components().collect();
        // From 1, since component 0 is the root: joining an absolute path onto
        // the repo just hands back `stale`, which is the path we know is gone.
        for start in 1..comps.len() {
            let candidate =
                self.repo_path.join(comps[start..].iter().copied().collect::<PathBuf>());
            let dot_git = candidate.join(".git");
            if dot_git.is_file()
                && std::fs::read_to_string(&dot_git).is_ok_and(|s| s.contains("/.git/worktrees/"))
            {
                return Some(candidate);
            }
        }
        None
    }

    /// Reattach this repo's linked worktrees after the project folder itself
    /// moved on disk (the AGE-203 reconnect).
    ///
    /// Both halves of the link are absolute paths: a worktree's `.git` file
    /// records `gitdir: <repo>/.git/worktrees/<id>`, and the repo's
    /// `.git/worktrees/<id>/gitdir` records the worktree's own `.git` file.
    /// Agency keeps its worktrees under `<repo>/.agency/worktrees/`, so a
    /// folder dragged in Finder moves both and invalidates both: every agent's
    /// tab then answers "not a git repository" until the paths are rewritten.
    /// `git worktree repair` is git's own fix for exactly this, and it takes
    /// the moved trees' paths to mend the main repo's side of each link.
    ///
    /// Best-effort by design: a run whose worktree was already gone, or a
    /// folder the user pointed at that is a different repository entirely,
    /// must not fail the reconnect. Nothing here can destroy work; the worst
    /// case is a link left broken, which is where it started.
    pub fn repair(&self) -> Result<()> {
        let root = self.worktrees_root();
        let mut paths: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    paths.push(entry.path().to_string_lossy().to_string());
                }
            }
        }
        // The user's own worktrees, when they lived inside the folder too:
        // they moved with it and their links went stale in exactly the same
        // way, and nothing else in Agency will ever mend them. Their new paths
        // are not somewhere we can enumerate, so they are matched back from
        // what git still records. Left broken they are also the one thing a
        // later `worktree prune` would delete outright.
        for stale in self.recorded_worktree_paths() {
            // Still where git thinks it is: nothing to mend. A path that is
            // merely unreachable (an unmounted volume) finds no match below
            // and is left registered, which is the recoverable state.
            if stale.exists() {
                continue;
            }
            if let Some(moved) = self.relocated_to(&stale) {
                paths.push(moved.to_string_lossy().to_string());
            }
        }
        // Sorted so a failure is reproducible from the log; read_dir is not
        // ordered. Deduped because one of ours can arrive down both routes.
        paths.sort();
        paths.dedup();
        let mut args = vec!["worktree", "repair"];
        args.extend(paths.iter().map(|p| p.as_str()));
        let _ = self.git(&args);
        // No `worktree prune` here, however natural it looks next to a repair.
        // Prune deregisters every worktree whose recorded path is not on disk,
        // and after the folder moved that is every worktree the *user* made
        // inside it, which are not ours to repair and not in `paths`: on a repo
        // with a `<repo>/wt/feature` alongside our own, prune answered
        // "Removing worktrees/feature: gitdir file points to non-existent
        // location" and deleted `.git/worktrees/feature`. The checkout is then
        // dead and beyond hand-repair too (`git worktree repair <path>` answers
        // "unable to locate repository"), taking any uncommitted work in it
        // with it. A volume that is merely unmounted reads the same way. A
        // stale entry left registered costs nothing by comparison: `list`
        // ignores anything outside our own root, `remove` prunes its own entry
        // by name, and the user can still mend theirs by hand.
        Ok(())
    }

    /// Delete the admin entry for one of our own worktrees whose directory is
    /// gone, and nobody else's.
    ///
    /// `git worktree prune` is the tool git offers for this and it is
    /// repo-wide: it deregisters every worktree whose recorded path is not on
    /// disk, which after a project folder moves (or a volume unmounts) is also
    /// every worktree the user made themselves. A deregistered worktree cannot
    /// be repaired afterwards — `git worktree repair` then answers "unable to
    /// locate repository" — so whatever was uncommitted in it is stranded. The
    /// reconnect made that reachable from an ordinary archive, since it mends
    /// our own links and cannot always mend theirs. This removes the one entry
    /// prune would have removed for `task_id`, and only once its `gitdir`
    /// confirms it is ours and the tree really has gone.
    fn prune_entry(&self, task_id: &str) {
        let Ok(common) = self.git(&["rev-parse", "--git-common-dir"]) else {
            return;
        };
        let common = PathBuf::from(common.trim());
        let common = if common.is_absolute() { common } else { self.repo_path.join(common) };
        let admin = common.join("worktrees").join(task_id);
        // Gone already: `worktree remove` deregisters on its way out, and this
        // only has anything to do when that could not run.
        let Ok(recorded) = std::fs::read_to_string(admin.join("gitdir")) else {
            return;
        };
        // `gitdir` holds the worktree's `.git` file, so its parent is the tree.
        // It is relative to this admin directory whenever the repo has git
        // 2.48's `worktree.useRelativePaths` set (or the worktree was added
        // with `--relative-paths`): the file then reads
        // `../../../.agency/worktrees/<id>/.git`, which can never equal an
        // absolute root, so this returned early and left registered the one
        // entry it exists to remove.
        let recorded = PathBuf::from(recorded.trim());
        let recorded = if recorded.is_absolute() { recorded } else { admin.join(recorded) };
        let recorded = lexical_normalize(&recorded);
        let Some(tree) = recorded.parent() else {
            return;
        };
        if tree != lexical_normalize(&self.worktrees_root().join(task_id)) || tree.exists() {
            return;
        }
        let _ = std::fs::remove_dir_all(&admin);
    }

    /// Remove the worktree and delete its branch. Tolerant: each git step is
    /// best-effort so it works whether or not the worktree still exists (e.g.
    /// discarding an already-archived run), and still deletes the branch.
    pub fn remove(&self, task_id: &str) -> Result<()> {
        let path = self.worktrees_root().join(task_id);
        let path_str = path.to_string_lossy().to_string();
        let _ = self.git(&["worktree", "remove", &path_str, "--force"]);
        self.prune_entry(task_id);
        let branch = Self::branch_for(task_id);
        let _ = self.git(&["branch", "-D", &branch]);
        Ok(())
    }

    /// Remove the worktree but KEEP the branch, so the work can be restored.
    pub fn remove_keep_branch(&self, task_id: &str) -> Result<()> {
        let path = self.worktrees_root().join(task_id);
        let path_str = path.to_string_lossy().to_string();
        self.git(&["worktree", "remove", &path_str, "--force"])?;
        self.prune_entry(task_id);
        Ok(())
    }

    /// Create a worktree for `task_id` on an EXISTING branch (e.g. a PR head
    /// being reviewed) instead of cutting a fresh `agent/<id>` branch. Fails
    /// if the branch is already checked out elsewhere — git enforces that.
    pub fn create_on_branch(&self, task_id: &str, branch: &str) -> Result<Worktree> {
        self.create_on_branch_with_progress(task_id, branch, &mut |_| {})
    }

    /// [`create_on_branch`] with checkout progress; see [`create_with_progress`].
    pub fn create_on_branch_with_progress(
        &self,
        task_id: &str,
        branch: &str,
        on_progress: &mut dyn FnMut(CloneProgress),
    ) -> Result<Worktree> {
        self.ensure_excluded()?;
        let path = self.worktrees_root().join(task_id);
        let path_str = path.to_string_lossy().to_string();
        self.run_worktree_add(&["worktree", "add", &path_str, branch], &path, on_progress)?;
        Ok(Worktree { task_id: task_id.to_string(), path, branch: branch.to_string() })
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

    /// Re-create a worktree for `task_id` on a *fresh* `branch` cut from
    /// `start`, for a restore whose original branch is gone.
    ///
    /// Archiving a merged run deletes its branch, because the branch is by then
    /// a second name for commits that are already on the base — so the common
    /// ending leaves nothing to restore onto, and Restore failed for every run
    /// that ended normally. Cutting the same name again from the base gives
    /// back a worktree that contains that work, and the rescued conversation
    /// resumes in it.
    pub fn recreate_on(&self, task_id: &str, branch: &str, start: &str) -> Result<Worktree> {
        self.ensure_excluded()?;
        let path = self.worktrees_root().join(task_id);
        let path_str = path.to_string_lossy().to_string();
        self.git(&["worktree", "add", &path_str, "-b", branch, start])?;
        Ok(Worktree { task_id: task_id.to_string(), path, branch: branch.to_string() })
    }
}

/// The exclude-file rewrite behind [`WorktreeManager::ensure_excluded`], as a
/// free function so callers without a manager (the issues-as-files migration)
/// can fix a repo's excludes too. Idempotent; a repo without `.git` is left
/// alone.
///
/// `.agency/issues/` is excluded too since the tracker stopped being a tracked
/// part of the repo (see [`untrack_issue_files`]), as is `.agency/records/`,
/// where a finished run's archive record is written; only
/// `.agency/agency.toml`, the project's shared config, stays visible to git.
pub fn ensure_agency_excludes(repo_path: &std::path::Path) -> Result<()> {
    if !repo_path.join(".git").exists() {
        return Ok(());
    }
    let exclude = repo_path.join(".git").join("info").join("exclude");
    let current = std::fs::read_to_string(&exclude).unwrap_or_default();
    let wanted =
        [".agency/worktrees/", ".agency/agency.local.toml", ".agency/issues/", ".agency/records/"];

    let had_legacy = current.lines().any(|l| l.trim() == ".agency/");
    let mut lines: Vec<String> =
        current.lines().filter(|l| l.trim() != ".agency/").map(|l| l.to_string()).collect();

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

/// Add one pattern to a repo's `.git/info/exclude` if it isn't there already.
/// Returns whether the file was changed. Idempotent; a repo without `.git` is
/// left alone, as in [`ensure_agency_excludes`].
///
/// Separate from that function because the Agency paths above belong to every
/// project the app manages, while this is for a file Agency only sometimes
/// generates (the worktree's `AGENTS.md`) and so should only sometimes hide.
pub fn ensure_exclude_pattern(repo_path: &std::path::Path, pattern: &str) -> Result<bool> {
    if !repo_path.join(".git").is_dir() {
        return Ok(false);
    }
    let exclude = repo_path.join(".git").join("info").join("exclude");
    let current = std::fs::read_to_string(&exclude).unwrap_or_default();
    if current.lines().any(|l| l.trim() == pattern) {
        return Ok(false);
    }
    if let Some(parent) = exclude.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut out = current;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(pattern);
    out.push('\n');
    std::fs::write(&exclude, out)?;
    Ok(true)
}

/// Stop tracking `.agency/issues/` in git, once per project. Returns whether
/// anything was untracked.
///
/// Issue files were tracked so the tracker travelled with the repo and agent
/// edits landed at merge. In practice the app writes issue state into the main
/// checkout on every dispatch and every merge, which left that checkout
/// permanently dirty (merges refuse to start on a dirty checkout) and put the
/// same frontmatter lines on both sides of every agent merge. One shared,
/// untracked tracker is what makes both stop.
///
/// The removal is committed, not left staged: staged deletions are exactly the
/// dirt this is meant to remove. `--cached` keeps every file on disk, and
/// [`ensure_agency_excludes`] should run first so the files are ignored the
/// moment they stop being tracked.
pub fn untrack_issue_files(repo_path: &std::path::Path) -> Result<bool> {
    let issues = crate::issuefs::ISSUES_DIR;
    if !repo_path.join(".git").exists() {
        return Ok(false);
    }
    let git = |args: &[&str]| -> Result<std::process::Output> {
        Ok(std::process::Command::new("git").args(args).current_dir(repo_path).output()?)
    };
    let tracked = git(&["ls-files", "--", issues])?;
    if !tracked.status.success() || String::from_utf8_lossy(&tracked.stdout).trim().is_empty() {
        return Ok(false);
    }
    // Both of these would make the migration commit sweep up work that isn't
    // ours. Erroring (rather than skipping) leaves the project unmarked, so the
    // next issue-touching call tries again.
    if repo_path.join(".git").join("MERGE_HEAD").exists() {
        bail!("a merge is in progress; finish or abort it first");
    }
    if !git(&["diff", "--cached", "--quiet"])?.status.success() {
        bail!("the checkout has staged changes; commit or unstage them first");
    }

    let rm = git(&["rm", "-r", "--cached", "--quiet", "--", issues])?;
    if !rm.status.success() {
        bail!("git rm --cached failed: {}", String::from_utf8_lossy(&rm.stderr));
    }
    let msg = "Stop tracking issue files\n\n\
               Agency keeps .agency/issues/ local to each checkout: the app writes\n\
               issue state there continuously, which kept the checkout dirty and\n\
               conflicted with agent branches editing the same files.";
    // `--no-verify`: a repo's commit hooks have no say in a bookkeeping commit
    // that only removes paths from the index.
    let commit = git(&["commit", "--no-verify", "-q", "-m", msg])?;
    if !commit.status.success() {
        // Put the index back rather than leaving the deletions staged.
        let _ = git(&["reset", "-q", "--", issues]);
        bail!("committing the untrack failed: {}", String::from_utf8_lossy(&commit.stderr));
    }
    // A repo whose own .gitignore explicitly un-ignores the issues directory
    // outranks .git/info/exclude, so the files would come back as untracked
    // noise. Worth saying out loud; nothing here can safely edit a user's
    // .gitignore.
    let left = git(&["status", "--porcelain", "--", issues])?;
    if !String::from_utf8_lossy(&left.stdout).trim().is_empty() {
        log::warn!(
            "{issues} is untracked but still visible to git in {} — a .gitignore rule is un-ignoring it",
            repo_path.display()
        );
    }
    Ok(true)
}

/// Resolve `.` and `..` in a path textually, without touching the disk.
///
/// `Path::canonicalize` cannot stand in for this: the caller compares paths to
/// a worktree directory that has just been deleted, and canonicalize fails on
/// anything that is not there. Symlinks therefore go unresolved, which is the
/// same footing the rest of this module works on (`WorktreeManager::new`
/// canonicalizes the repo root once, and git records resolved paths from it).
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            // A `..` with nothing to pop (a relative path that climbs above
            // its own start) is kept, so the result still names what it named.
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Best-effort recursive count of regular files under `dir` (directories are
/// descended, symlinks counted as files, all io errors swallowed). Drives only
/// the create progress indicator, so an approximate count is fine.
fn count_files(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => count += count_files(&entry.path()),
            Ok(_) => count += 1,
            Err(_) => {}
        }
    }
    count
}

fn copy_recursive(src: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dest.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dest)?;
    }
    Ok(())
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

    fn git(repo: &std::path::Path, args: &[&str]) {
        assert!(
            Command::new("git").args(args).current_dir(repo).status().unwrap().success(),
            "git {args:?}"
        );
    }

    /// A repo whose issue files a previous version committed: the migration
    /// removes them from the index in one commit, leaves every file on disk,
    /// and leaves the checkout clean (the whole reason it exists).
    #[test]
    fn untrack_issue_files_commits_the_removal_and_keeps_the_files() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        git(repo, &["init", "-q", "-b", "main"]);
        git(repo, &["config", "user.email", "t@e.com"]);
        git(repo, &["config", "user.name", "T"]);
        fs::create_dir_all(repo.join(crate::issuefs::ISSUES_DIR)).unwrap();
        fs::write(repo.join(crate::issuefs::ISSUES_DIR).join("AGE-1.md"), "# one\n").unwrap();
        fs::write(repo.join("code.rs"), "fn main() {}\n").unwrap();
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-q", "-m", "init"]);

        ensure_agency_excludes(repo).unwrap();
        assert!(untrack_issue_files(repo).unwrap());

        assert!(repo.join(crate::issuefs::ISSUES_DIR).join("AGE-1.md").exists());
        let tracked = Command::new("git")
            .args(["ls-files", "--", crate::issuefs::ISSUES_DIR])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&tracked.stdout).trim().is_empty());
        let status =
            Command::new("git").args(["status", "--porcelain"]).current_dir(repo).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&status.stdout).trim(), "");
        // Second pass has nothing left to do, and makes no second commit.
        assert!(!untrack_issue_files(repo).unwrap());
    }

    /// Staged work belongs to the user; the migration commit must not sweep it
    /// up, so it steps aside and is retried later.
    #[test]
    fn untrack_issue_files_refuses_over_staged_work() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        git(repo, &["init", "-q", "-b", "main"]);
        git(repo, &["config", "user.email", "t@e.com"]);
        git(repo, &["config", "user.name", "T"]);
        fs::create_dir_all(repo.join(crate::issuefs::ISSUES_DIR)).unwrap();
        fs::write(repo.join(crate::issuefs::ISSUES_DIR).join("AGE-1.md"), "# one\n").unwrap();
        fs::write(repo.join("code.rs"), "fn main() {}\n").unwrap();
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-q", "-m", "init"]);
        fs::write(repo.join("code.rs"), "fn main() { todo!() }\n").unwrap();
        git(repo, &["add", "code.rs"]);

        let err = untrack_issue_files(repo).unwrap_err();
        assert!(err.to_string().contains("staged changes"), "got: {err}");
        // The index is untouched: the issue file is still tracked.
        let tracked = Command::new("git")
            .args(["ls-files", "--", crate::issuefs::ISSUES_DIR])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&tracked.stdout).contains("AGE-1.md"));
    }

    /// A repo that never tracked its issues (or has none) is left alone.
    #[test]
    fn untrack_issue_files_is_a_no_op_when_nothing_is_tracked() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        git(repo, &["init", "-q", "-b", "main"]);
        git(repo, &["config", "user.email", "t@e.com"]);
        git(repo, &["config", "user.name", "T"]);
        fs::write(repo.join("code.rs"), "fn main() {}\n").unwrap();
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-q", "-m", "init"]);
        fs::create_dir_all(repo.join(crate::issuefs::ISSUES_DIR)).unwrap();
        fs::write(repo.join(crate::issuefs::ISSUES_DIR).join("AGE-1.md"), "# one\n").unwrap();

        assert!(!untrack_issue_files(repo).unwrap());
    }
}
