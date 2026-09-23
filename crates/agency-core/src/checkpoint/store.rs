//! Per-turn workspace checkpoints: the git calls.
//!
//! Every decision is made by the pure functions in the parent module; this file
//! only runs git and moves files. Two rules hold throughout:
//!
//! - **The user's git state is read, never written.** Snapshots go through a
//!   throwaway index file (`GIT_INDEX_FILE`) in the worktree's own git dir, so
//!   the real index, HEAD, the branches and the reflog are exactly as they
//!   were. The only writes are objects and refs under [`REF_ROOT`].
//! - **A restore is verified, and undone if it does not verify.** It saves the
//!   files as they are first, writes the target, snapshots again and compares
//!   trees. A mismatch puts the saved state back and reports the failure, per
//!   the house rule for anything that rewrites a user's files.

use super::*;
use anyhow::{anyhow, bail, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// Who a checkpoint commit says made it. Fixed, so a snapshot never borrows the
/// user's identity for something they did not write, and so a machine with no
/// `user.name` set can still take one: `commit-tree` refuses to run without an
/// identity. `.invalid` is reserved and can never be anyone's address.
const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "Agency"),
    ("GIT_AUTHOR_EMAIL", "checkpoints@agency.invalid"),
    ("GIT_COMMITTER_NAME", "Agency"),
    ("GIT_COMMITTER_EMAIL", "checkpoints@agency.invalid"),
];

/// Run git in `dir` with extra environment and optional stdin; stdout on
/// success.
fn run(dir: &Path, args: &[&str], env: &[(&str, &str)], stdin: Option<&[u8]>) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(env.iter().copied())
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context("could not run git")?;
    if let Some(input) = stdin {
        // Dropped at the end of the block, which closes the pipe: git reads
        // until EOF.
        child.stdin.take().ok_or_else(|| anyhow!("git stdin unavailable"))?.write_all(input)?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A private index file in the worktree's git dir, removed when dropped (lock
/// file included, should git have died holding it).
struct TempIndex {
    path: PathBuf,
}

impl TempIndex {
    fn new(git_dir: &Path) -> TempIndex {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        TempIndex {
            path: git_dir.join(format!("agency-checkpoint-{}-{n}.index", std::process::id())),
        }
    }

    fn env(&self) -> [(&str, &str); 1] {
        [("GIT_INDEX_FILE", self.path.to_str().unwrap_or_default())]
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let mut lock = self.path.clone().into_os_string();
        lock.push(".lock");
        let _ = std::fs::remove_file(lock);
    }
}

/// The worktree's own git dir: `.git` for a main checkout, `.git/worktrees/<n>`
/// for a linked one. Where its index lives, and so where the copy goes.
fn git_dir(dir: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(run(dir, &["rev-parse", "--absolute-git-dir"], &[], None)?.trim()))
}

fn head(dir: &Path) -> Option<String> {
    run(dir, &["rev-parse", "-q", "--verify", "HEAD^{commit}"], &[], None)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| is_oid(s))
}

/// The worktree's files, written as a tree.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub tree: String,
    pub head: Option<String>,
    /// Untracked files left out for being over [`MAX_UNTRACKED_BYTES`].
    pub skipped: Vec<String>,
}

/// Write every file git would track here (tracked or untracked, never ignored)
/// into a tree, without touching the user's index.
///
/// The temporary index starts as a *copy* of the real one rather than `read-tree
/// HEAD`: the copy carries git's stat cache, so `add -A` rehashes only what
/// changed instead of every file in the tree. It gives the same tree either
/// way, since `add -A` makes every entry match the working tree.
pub fn snapshot(dir: &Path) -> Result<Snapshot> {
    let gd = git_dir(dir)?;
    let index = TempIndex::new(&gd);
    let env = index.env();
    match std::fs::copy(gd.join("index"), &index.path) {
        Ok(_) => {}
        // A repository nothing has been added to yet has no index file.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if head(dir).is_some() {
                run(dir, &["read-tree", "HEAD"], &env, None)?;
            }
        }
        Err(e) => return Err(e).context("could not copy the index"),
    }
    let untracked = run(dir, &["ls-files", "-z", "-o", "--exclude-standard"], &env, None)?;
    let sizes: Vec<(String, u64)> = untracked
        .split('\0')
        .filter(|p| !p.is_empty())
        .filter_map(|p| {
            let meta = std::fs::symlink_metadata(dir.join(p)).ok()?;
            Some((p.to_string(), meta.len()))
        })
        .collect();
    let skipped = oversized(&sizes, MAX_UNTRACKED_BYTES);
    let excludes: Vec<String> = skipped.iter().map(|p| format!(":(exclude,literal){p}")).collect();
    // An agent that clones something into the tree would otherwise print git's
    // "adding embedded git repository" advice into the log on every turn.
    let mut args = vec!["-c", "advice.addEmbeddedRepo=false", "add", "-A", "--", "."];
    args.extend(excludes.iter().map(String::as_str));
    run(dir, &args, &env, None)?;
    let tree = run(dir, &["write-tree"], &env, None)?.trim().to_string();
    Ok(Snapshot { tree, head: head(dir), skipped })
}

/// A run's checkpoints, oldest first.
pub fn list(dir: &Path, run_id: &str) -> Result<Vec<Checkpoint>> {
    if !valid_run_id(run_id) {
        bail!("not a run id: {run_id:?}");
    }
    let format = format!("--format={LIST_FORMAT}");
    let out = run(dir, &["for-each-ref", &format, &run_prefix(run_id)], &[], None)?;
    Ok(parse_list(run_id, &out))
}

/// Store `snap` as the run's next checkpoint.
///
/// `update-ref` is given the all-zero old value, which makes it create-only: if
/// another capture took the same number in between, this one fails instead of
/// silently replacing it.
fn record(
    dir: &Path,
    run_id: &str,
    kind: Kind,
    snap: &Snapshot,
    list: &[Checkpoint],
) -> Result<Checkpoint> {
    let msg = message(kind, snap.head.as_deref());
    // --no-gpg-sign: `commit.gpgSign` applies to commit-tree too, and a
    // pinentry prompt on every turn would be absurd for a private snapshot.
    let commit =
        run(dir, &["commit-tree", "--no-gpg-sign", &snap.tree, "-m", &msg], &IDENTITY, None)?
            .trim()
            .to_string();
    let seq = next_seq(list);
    let zero = "0".repeat(commit.len());
    run(dir, &["update-ref", &ref_name(run_id, seq), &commit, &zero], &[], None)?;
    Ok(Checkpoint {
        seq,
        commit,
        tree: snap.tree.clone(),
        at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64),
        kind,
        head: snap.head.clone(),
    })
}

/// Delete refs in one transaction. Missing refs are fine: `delete` without an
/// old value does not care whether the ref was there.
fn delete_refs(dir: &Path, refs: &[String]) -> Result<()> {
    if refs.is_empty() {
        return Ok(());
    }
    let script: String = refs.iter().map(|r| format!("delete {r}\n")).collect();
    run(dir, &["update-ref", "--stdin"], &[], Some(script.as_bytes()))?;
    Ok(())
}

/// Take a checkpoint of `dir` for `run_id`. `None` when the files are exactly
/// as the newest checkpoint has them, which stores nothing.
pub fn capture(dir: &Path, run_id: &str, kind: Kind) -> Result<Option<Checkpoint>> {
    let existing = list(dir, run_id)?;
    let snap = snapshot(dir)?;
    if is_unchanged(&existing, &snap.tree) {
        return Ok(None);
    }
    let cp = record(dir, run_id, kind, &snap, &existing)?;
    let mut all = existing;
    all.push(cp.clone());
    // Best-effort: a prune that fails leaves one checkpoint too many, which the
    // next capture's prune picks up.
    if let Err(e) = delete_refs(dir, &over_cap(run_id, &all, KEEP_PER_RUN)) {
        log::warn!("checkpoints: pruning {run_id} failed: {e:#}");
    }
    Ok(Some(cp))
}

/// Delete every checkpoint a run has. `dir` is anywhere in the repository: the
/// refs are shared by all its worktrees, so this works after the run's own
/// worktree is gone.
pub fn prune_run(dir: &Path, run_id: &str) -> Result<usize> {
    let refs: Vec<String> = list(dir, run_id)?.iter().map(|c| ref_name(run_id, c.seq)).collect();
    delete_refs(dir, &refs)?;
    Ok(refs.len())
}

/// What changed from `from` to `to` (commits or trees, as full object ids).
pub fn changes(dir: &Path, from: &str, to: &str) -> Result<Vec<RawChange>> {
    if !is_oid(from) || !is_oid(to) {
        bail!("checkpoints are compared by object id");
    }
    let out = run(dir, &["diff-tree", "-r", "-z", "--no-renames", "--raw", from, to], &[], None)?;
    Ok(parse_raw_z(&out))
}

/// The patch for one path between two checkpoints, in the form the diff viewer
/// already parses for a commit.
pub fn diff(dir: &Path, from: &str, to: &str, path: &str) -> Result<String> {
    if !is_oid(from) || !is_oid(to) {
        bail!("checkpoints are compared by object id");
    }
    run(dir, &["diff", "--no-renames", "--no-ext-diff", from, to, "--", path], &[], None)
}

fn find(list: &[Checkpoint], seq: u32) -> Result<&Checkpoint> {
    list.iter().find(|c| c.seq == seq).ok_or_else(|| anyhow!("that checkpoint no longer exists"))
}

/// What restoring a checkpoint would do, for the confirm dialog to say before
/// anything happens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    /// Files that go back to the checkpoint's version (edited, deleted or
    /// retyped since).
    pub write: usize,
    /// Files created since, which the restore deletes.
    pub remove: usize,
    /// HEAD is not where it was when the checkpoint was taken: the branch has
    /// commits made after it, which a restore leaves where they are.
    pub head_moved: bool,
    /// Files too large to save first that the restore would overwrite. A
    /// restore refuses while this is non-empty.
    pub unsaved: Vec<String>,
}

pub fn preview(dir: &Path, run_id: &str, seq: u32) -> Result<Preview> {
    let all = list(dir, run_id)?;
    let target = find(&all, seq)?;
    let now = snapshot(dir)?;
    let plan = restore_plan(&changes(dir, &now.tree, &target.tree)?);
    Ok(Preview {
        write: plan.write.len(),
        remove: plan.remove.len(),
        head_moved: target.head.is_some() && target.head != now.head,
        unsaved: unsaved_overwrites(&plan, &now.skipped),
    })
}

/// How a restore went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Restored {
    /// The checkpoint that holds the files as they were just before: a new
    /// "before restore" one, or the newest existing one when nothing had
    /// changed since it.
    pub saved: Option<u32>,
    /// Files written or deleted.
    pub changed: usize,
}

/// Put `dir`'s files back to checkpoint `seq`.
///
/// Files only. HEAD, the branches and the index stay where they are, and the
/// agent's conversation is not something a checkpoint holds at all. Ignored
/// files are never touched, and neither are submodules (see
/// [`restore_plan`]).
///
/// The caller must keep other captures of this run from running at the same
/// time: the verify step reads the tree back, and a capture reading the tree
/// halfway through would store a state that never really existed.
pub fn restore(dir: &Path, run_id: &str, seq: u32) -> Result<Restored> {
    let all = list(dir, run_id)?;
    let target = find(&all, seq)?.clone();
    let before = snapshot(dir)?;
    let plan = restore_plan(&changes(dir, &before.tree, &target.tree)?);
    let unsaved = unsaved_overwrites(&plan, &before.skipped);
    if !unsaved.is_empty() {
        bail!(
            "restoring would overwrite {}, which is too large to checkpoint first; move it \
             aside and try again",
            unsaved.join(", ")
        );
    }
    // The files already match: nothing moves, so there is nothing to save.
    if plan.is_empty() {
        return Ok(Restored { saved: None, changed: 0 });
    }
    // Saved before the first file moves. Nothing changed since the newest
    // checkpoint means that one already holds these files.
    let saved = if is_unchanged(&all, &before.tree) {
        all.last().map(|c| c.seq)
    } else {
        Some(record(dir, run_id, Kind::BeforeRestore, &before, &all)?.seq)
    };
    let applied = apply(dir, &target.tree, &plan);
    let after = applied.and_then(|_| snapshot(dir));
    match after {
        Ok(snap) if snap.tree == target.tree => Ok(Restored { saved, changed: plan.len() }),
        outcome => {
            // Put back what was there. The files the failed pass wrote are all
            // in `before`, so this is a restore to a tree we just made.
            let why = match outcome {
                Ok(snap) => {
                    let differ = changes(dir, &snap.tree, &target.tree)
                        .map(|c| c.into_iter().map(|c| c.path).take(5).collect::<Vec<_>>())
                        .unwrap_or_default();
                    format!("these did not come back as they were: {}", differ.join(", "))
                }
                Err(e) => format!("{e:#}"),
            };
            let rollback = snapshot(dir).and_then(|now| {
                let back = restore_plan(&changes(dir, &now.tree, &before.tree)?);
                apply(dir, &before.tree, &back)
            });
            match rollback {
                Ok(()) => bail!("the restore did not complete, so your files were put back: {why}"),
                Err(e) => bail!(
                    "the restore did not complete ({why}), and putting the files back failed too \
                     ({e:#}); they are saved as checkpoint {}",
                    saved.map_or_else(|| "?".to_string(), |s| s.to_string())
                ),
            }
        }
    }
}

/// Carry out a plan: removals first, deepest first, pruning directories they
/// empty; then every write from `tree` in one `checkout-index`.
fn apply(dir: &Path, tree: &str, plan: &RestorePlan) -> Result<()> {
    for p in plan.remove.iter().chain(&plan.write) {
        if !safe_rel_path(p) {
            bail!("refusing to touch {p:?}: not a path inside the workspace");
        }
    }
    for p in &plan.remove {
        let abs = dir.join(p);
        match std::fs::symlink_metadata(&abs) {
            // A real directory where the tree had a file: not ours to recurse
            // into. The verify step will name it.
            Ok(meta) if meta.is_dir() => continue,
            Ok(_) => std::fs::remove_file(&abs).with_context(|| format!("removing {p}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("reading {p}")),
        }
        for parent in parent_dirs(p) {
            if std::fs::remove_dir(dir.join(&parent)).is_err() {
                break;
            }
        }
    }
    if plan.write.is_empty() {
        return Ok(());
    }
    let index = TempIndex::new(&git_dir(dir)?);
    let env = index.env();
    run(dir, &["read-tree", tree], &env, None)?;
    let paths: Vec<u8> =
        plan.write.iter().flat_map(|p| p.bytes().chain(std::iter::once(0))).collect();
    run(dir, &["checkout-index", "-f", "-z", "--stdin"], &env, Some(&paths))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn git(dir: &Path, args: &[&str]) -> String {
        run(dir, args, &[], None).unwrap()
    }

    fn init(dir: &Path) {
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["config", "user.email", "t@t"]);
        git(dir, &["config", "user.name", "t"]);
        fs::write(dir.join(".gitignore"), "*.log\n").unwrap();
        fs::write(dir.join("a.txt"), "one\n").unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.rs"), "fn main() {}\n").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", "init"]);
    }

    fn read(dir: &Path, p: &str) -> Option<String> {
        fs::read_to_string(dir.join(p)).ok()
    }

    #[test]
    fn capture_leaves_the_users_git_state_alone() {
        let d = tempdir().unwrap();
        let dir = d.path();
        init(dir);
        fs::write(dir.join("a.txt"), "two\n").unwrap();
        fs::write(dir.join("staged.txt"), "s\n").unwrap();
        git(dir, &["add", "staged.txt"]);
        fs::write(dir.join("new.txt"), "n\n").unwrap();
        let status = git(dir, &["status", "--porcelain"]);
        let index = fs::read(dir.join(".git/index")).unwrap();
        let heads = git(dir, &["for-each-ref", "refs/heads"]);

        let cp = capture(dir, "run-1", Kind::TurnEnded).unwrap().expect("something changed");
        assert_eq!(cp.seq, 1);
        assert_eq!(git(dir, &["status", "--porcelain"]), status);
        assert_eq!(fs::read(dir.join(".git/index")).unwrap(), index);
        assert_eq!(git(dir, &["for-each-ref", "refs/heads"]), heads);
        // No temporary index left behind.
        let leftovers: Vec<_> = fs::read_dir(dir.join(".git"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("agency-checkpoint"))
            .collect();
        assert!(leftovers.is_empty());
        // The tree holds tracked edits, staged and untracked files; not ignored ones.
        let files = git(dir, &["ls-tree", "-r", "--name-only", &cp.tree]);
        assert!(files.contains("new.txt") && files.contains("staged.txt"));
        // And the commit is parentless, made by Agency.
        assert_eq!(git(dir, &["rev-list", "--count", &cp.commit]).trim(), "1");
        assert_eq!(git(dir, &["log", "-1", "--format=%an", &cp.commit]).trim(), "Agency");
    }

    #[test]
    fn an_unchanged_capture_stores_nothing() {
        let d = tempdir().unwrap();
        let dir = d.path();
        init(dir);
        assert!(capture(dir, "r", Kind::RunStart).unwrap().is_some());
        assert!(capture(dir, "r", Kind::PromptSent).unwrap().is_none());
        fs::write(dir.join("ignored.log"), "noise").unwrap();
        assert!(
            capture(dir, "r", Kind::TurnEnded).unwrap().is_none(),
            "ignored files do not count"
        );
        fs::write(dir.join("a.txt"), "changed\n").unwrap();
        let cp = capture(dir, "r", Kind::TurnEnded).unwrap().unwrap();
        assert_eq!(cp.seq, 2);
        let listed = list(dir, "r").unwrap();
        assert_eq!(
            listed.iter().map(|c| c.kind).collect::<Vec<_>>(),
            [Kind::RunStart, Kind::TurnEnded]
        );
        assert_eq!(listed[1].head, head(dir));
    }

    #[test]
    fn restore_puts_files_back_and_can_itself_be_undone() {
        let d = tempdir().unwrap();
        let dir = d.path();
        init(dir);
        fs::write(dir.join("keep.log"), "ignored, untouched").unwrap();
        let start = capture(dir, "r", Kind::RunStart).unwrap().unwrap();

        // A turn that edits, deletes, adds (deep) and turns a file into a dir.
        fs::write(dir.join("a.txt"), "agent was here\n").unwrap();
        fs::remove_file(dir.join("src/lib.rs")).unwrap();
        fs::create_dir_all(dir.join("gen/deep")).unwrap();
        fs::write(dir.join("gen/deep/out.rs"), "x").unwrap();
        capture(dir, "r", Kind::TurnEnded).unwrap().unwrap();
        fs::write(dir.join("a.txt"), "after the turn\n").unwrap();
        let status_before = git(dir, &["diff", "--cached", "--stat"]);

        let p = preview(dir, "r", start.seq).unwrap();
        assert_eq!((p.write, p.remove, p.head_moved), (2, 1, false));
        let done = restore(dir, "r", start.seq).unwrap();
        assert_eq!(done.changed, 3);
        assert_eq!(read(dir, "a.txt").as_deref(), Some("one\n"));
        assert_eq!(read(dir, "src/lib.rs").as_deref(), Some("fn main() {}\n"));
        assert!(!dir.join("gen").exists(), "emptied directories go too");
        assert_eq!(read(dir, "keep.log").as_deref(), Some("ignored, untouched"));
        assert_eq!(git(dir, &["diff", "--cached", "--stat"]), status_before, "index untouched");
        assert!(git(dir, &["status", "--porcelain"]).is_empty());

        // The state just before the restore was saved, so it can come back.
        let saved = done.saved.unwrap();
        let listed = list(dir, "r").unwrap();
        assert_eq!(listed.last().unwrap().kind, Kind::BeforeRestore);
        restore(dir, "r", saved).unwrap();
        assert_eq!(read(dir, "a.txt").as_deref(), Some("after the turn\n"));
        assert_eq!(read(dir, "gen/deep/out.rs").as_deref(), Some("x"));
        assert!(!dir.join("src/lib.rs").exists());
    }

    #[test]
    fn restore_reports_a_moved_head_and_leaves_it() {
        let d = tempdir().unwrap();
        let dir = d.path();
        init(dir);
        let start = capture(dir, "r", Kind::RunStart).unwrap().unwrap();
        fs::write(dir.join("a.txt"), "committed by the agent\n").unwrap();
        git(dir, &["commit", "-qam", "agent commit"]);
        let moved = head(dir);
        assert!(preview(dir, "r", start.seq).unwrap().head_moved);
        restore(dir, "r", start.seq).unwrap();
        assert_eq!(head(dir), moved, "restore never moves HEAD");
        assert_eq!(read(dir, "a.txt").as_deref(), Some("one\n"));
        assert_eq!(git(dir, &["status", "--porcelain"]).trim(), "M a.txt");
    }

    #[test]
    fn prune_takes_only_that_runs_refs() {
        let d = tempdir().unwrap();
        let dir = d.path();
        init(dir);
        capture(dir, "r", Kind::RunStart).unwrap();
        capture(dir, "r2", Kind::RunStart).unwrap();
        fs::write(dir.join("a.txt"), "x").unwrap();
        capture(dir, "r", Kind::TurnEnded).unwrap();
        assert_eq!(prune_run(dir, "r").unwrap(), 2);
        assert!(list(dir, "r").unwrap().is_empty());
        assert_eq!(list(dir, "r2").unwrap().len(), 1);
    }

    #[test]
    fn works_in_a_linked_worktree_and_shares_refs_with_the_repo() {
        let d = tempdir().unwrap();
        let repo = d.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        init(&repo);
        let wt = d.path().join("wt");
        git(&repo, &["worktree", "add", "-q", "-b", "agent/x", wt.to_str().unwrap()]);
        fs::write(wt.join("a.txt"), "in the worktree\n").unwrap();
        capture(&wt, "x", Kind::TurnEnded).unwrap().unwrap();
        assert_eq!(list(&repo, "x").unwrap().len(), 1, "visible from the main checkout");
        assert!(git(&repo, &["status", "--porcelain"]).is_empty());
        // Pruned from the repo, as archive does once the worktree is gone.
        git(&repo, &["worktree", "remove", "--force", wt.to_str().unwrap()]);
        assert_eq!(prune_run(&repo, "x").unwrap(), 1);
    }

    #[test]
    fn diff_and_changes_refuse_anything_but_object_ids() {
        let d = tempdir().unwrap();
        let dir = d.path();
        init(dir);
        assert!(changes(dir, "HEAD", "HEAD").is_err());
        assert!(diff(dir, "--output=/tmp/x", &"a".repeat(40), "a.txt").is_err());
    }

    #[test]
    fn paths_with_spaces_and_unicode_round_trip() {
        let d = tempdir().unwrap();
        let dir = d.path();
        init(dir);
        let start = capture(dir, "r", Kind::RunStart).unwrap().unwrap();
        fs::write(dir.join("héllo wörld.txt"), "new").unwrap();
        fs::write(dir.join("a.txt"), "edit").unwrap();
        let end = capture(dir, "r", Kind::TurnEnded).unwrap().unwrap();
        let mut got: Vec<String> =
            changes(dir, &start.commit, &end.commit).unwrap().into_iter().map(|c| c.path).collect();
        got.sort();
        assert_eq!(got, vec!["a.txt", "héllo wörld.txt"]);
        assert!(diff(dir, &start.commit, &end.commit, "a.txt").unwrap().contains("+edit"));
        restore(dir, "r", start.seq).unwrap();
        assert!(!dir.join("héllo wörld.txt").exists());
    }
}
