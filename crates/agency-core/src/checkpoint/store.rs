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
use std::collections::HashSet;
use std::path::{Path, PathBuf};
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

fn run(dir: &Path, args: &[&str], env: &[(&str, &str)], stdin: Option<&[u8]>) -> Result<String> {
    crate::git::git_with(dir, args, env, stdin)
}

/// Paths as `-z --stdin` takes them: each one NUL-terminated.
fn nul_list<'a>(paths: impl IntoIterator<Item = &'a String>) -> Vec<u8> {
    paths.into_iter().flat_map(|p| p.bytes().chain(std::iter::once(0))).collect()
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
/// HEAD`: the copy carries git's stat cache, so `add` rehashes only what changed
/// instead of every file in the tree. The copy also carries the user's
/// assume-unchanged and skip-worktree bits, which `add` would believe, so those
/// are cleared on the copy first (see [`parse_listing`]).
///
/// The untracked files are walked once. `ls-files` finds them (and the flags,
/// in the same pass), `add -u` brings the tracked entries up to date without
/// looking for new files, and the untracked list goes to `update-index` as it
/// is. `add -A` would have walked the whole tree for untracked files a second
/// time, on every Enter and every turn end, on a thread every run shares.
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
    let listing = parse_listing(&run(
        dir,
        &["ls-files", "-z", "-v", "-c", "-o", "--exclude-standard"],
        &env,
        None,
    )?);
    if !listing.assume_unchanged.is_empty() {
        let paths = nul_list(&listing.assume_unchanged);
        run(dir, &["update-index", "-z", "--no-assume-unchanged", "--stdin"], &env, Some(&paths))?;
    }
    // Only the ones with a file on disk. A sparse checkout's skip-worktree
    // entries have none, and clearing their bit would snapshot every file
    // outside the cone as deleted.
    let present: Vec<&String> =
        listing.skip_worktree.iter().filter(|p| dir.join(p).symlink_metadata().is_ok()).collect();
    if !present.is_empty() {
        let paths = nul_list(present);
        run(dir, &["update-index", "-z", "--no-skip-worktree", "--stdin"], &env, Some(&paths))?;
    }
    run(dir, &["add", "-u"], &env, None)?;
    let sizes: Vec<(String, u64)> = listing
        .untracked
        .iter()
        .filter_map(|p| Some((p.clone(), std::fs::symlink_metadata(dir.join(p)).ok()?.len())))
        .collect();
    let skipped = oversized(&sizes, MAX_UNTRACKED_BYTES);
    let skip: HashSet<&str> = skipped.iter().map(String::as_str).collect();
    let add = nul_list(sizes.iter().map(|(p, _)| p).filter(|p| !skip.contains(p.as_str())));
    if !add.is_empty() {
        // --remove: a file deleted since `ls-files` saw it is dropped, not an
        // error.
        run(dir, &["update-index", "--add", "--remove", "-z", "--stdin"], &env, Some(&add))?;
    }
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
    /// Files in the way of the restore that no checkpoint can hold (ignored,
    /// too large, or part of a nested repository). A restore refuses while
    /// this is non-empty.
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
        unsaved: unsaved(&plan, &in_the_way(dir, &plan)?),
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
/// [`restore_plan`]): a restore that would have to is refused (see
/// [`unsaved`]).
///
/// The caller must keep every other capture of `dir` from running at the same
/// time: the verify step reads the tree back, and a capture reading the tree
/// halfway through would store a state that never really existed.
pub fn restore(dir: &Path, run_id: &str, seq: u32) -> Result<Restored> {
    let all = list(dir, run_id)?;
    let target = find(&all, seq)?.clone();
    let before = snapshot(dir)?;
    let plan = restore_plan(&changes(dir, &before.tree, &target.tree)?);
    let unsaved = unsaved(&plan, &in_the_way(dir, &plan)?);
    if !unsaved.is_empty() {
        bail!(
            "restoring would overwrite or delete {}, which no checkpoint can hold (ignored, too \
             large, or part of a separate repository); move it out of the workspace and try again",
            name_some(&unsaved)
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
    let why = match put(dir, &before.tree, &target.tree) {
        Ok(()) => return Ok(Restored { saved, changed: plan.len() }),
        Err(e) => format!("{e:#}"),
    };
    // Put back what was there, and check that too: the same filter or
    // directory that stopped the restore matching can stop the rollback, and
    // telling someone their files were put back when they were not is worse
    // than either failure.
    let saved = saved.map_or_else(|| "?".to_string(), |s| s.to_string());
    match snapshot(dir).and_then(|now| put(dir, &now.tree, &before.tree)) {
        Ok(()) => bail!("the restore did not complete, so your files were put back: {why}"),
        Err(e) => bail!(
            "the restore did not complete ({why}), and putting the files back did not either \
             ({e:#}); what you had is saved as checkpoint {saved}"
        ),
    }
}

/// Take `dir`'s files from `now` (the tree they are known to match) to `tree`,
/// then snapshot and check that they got there.
fn put(dir: &Path, now: &str, tree: &str) -> Result<()> {
    let plan = restore_plan(&changes(dir, now, tree)?);
    apply(dir, tree, &plan)?;
    let after = snapshot(dir)?;
    if after.tree != tree {
        let differ: Vec<String> =
            changes(dir, &after.tree, tree)?.into_iter().map(|c| c.path).collect();
        bail!("these did not come back as they were: {}", name_some(&differ));
    }
    Ok(())
}

/// Up to five paths for a message, and how many more there were.
fn name_some(paths: &[String]) -> String {
    match paths.len() {
        0..=5 => paths.join(", "),
        n => format!("{} and {} more", paths[..5].join(", "), n - 5),
    }
}

/// Every file or symlink on disk that carrying out `plan` would unlink besides
/// the removals themselves: see [`unsaved`] for why each kind matters.
fn in_the_way(dir: &Path, plan: &RestorePlan) -> Result<Vec<String>> {
    let mut found = Vec::new();
    let mut parents_seen: HashSet<String> = HashSet::new();
    for p in &plan.write {
        if !safe_rel_path(p) {
            bail!("refusing to touch {p:?}: not a path inside the workspace");
        }
        for parent in parent_dirs(p) {
            if !parents_seen.insert(parent.clone()) {
                // Checked already, and so was everything above it.
                break;
            }
            match std::fs::symlink_metadata(dir.join(&parent)) {
                Ok(meta) if !meta.is_dir() => found.push(parent),
                _ => {}
            }
        }
        match std::fs::symlink_metadata(dir.join(p)) {
            Ok(meta) if meta.is_dir() => files_under(dir, p, &mut found)?,
            Ok(_) => found.push(p.clone()),
            // Nothing there, or a file where a parent should be: the loop above
            // has that one already.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) => {}
            Err(e) => return Err(e).with_context(|| format!("reading {p}")),
        }
    }
    Ok(found)
}

/// Every file and symlink below `rel`, as worktree-relative paths. Symlinks
/// are not followed: `checkout-index` unlinks the link, not what it points at.
fn files_under(dir: &Path, rel: &str, out: &mut Vec<String>) -> Result<()> {
    let entries = std::fs::read_dir(dir.join(rel)).with_context(|| format!("reading {rel}"))?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let child = format!("{rel}/{}", name.to_string_lossy());
        if entry.file_type()?.is_dir() {
            files_under(dir, &child, out)?;
        } else {
            out.push(child);
        }
    }
    Ok(())
}

/// Carry out a plan: removals first, deepest first, pruning directories they
/// empty; then every write from `tree` in one `checkout-index`.
///
/// Refuses before moving anything if a write would take a file with it that
/// the current snapshot does not hold (see [`unsaved`]). [`restore`] checks
/// that up front so it can say so plainly; this is the guard for the rollback,
/// whose plan is only known once the restore has failed.
fn apply(dir: &Path, tree: &str, plan: &RestorePlan) -> Result<()> {
    for p in plan.remove.iter().chain(&plan.write) {
        if !safe_rel_path(p) {
            bail!("refusing to touch {p:?}: not a path inside the workspace");
        }
    }
    let unsaved = unsaved(plan, &in_the_way(dir, plan)?);
    if !unsaved.is_empty() {
        bail!("{} would be overwritten, and no checkpoint holds it", name_some(&unsaved));
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
    run(dir, &["checkout-index", "-f", "-z", "--stdin"], &env, Some(&nul_list(&plan.write)))?;
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
    fn restore_refuses_to_take_an_ignored_file_with_a_directory() {
        let d = tempdir().unwrap();
        let dir = d.path();
        init(dir);
        fs::write(dir.join("out"), "a file\n").unwrap();
        let start = capture(dir, "r", Kind::RunStart).unwrap().unwrap();
        // The agent turns `out` into a directory, and something in it is ignored.
        fs::remove_file(dir.join("out")).unwrap();
        fs::create_dir(dir.join("out")).unwrap();
        fs::write(dir.join("out/a.txt"), "tracked-ish").unwrap();
        fs::write(dir.join("out/cache.log"), "ignored").unwrap();
        capture(dir, "r", Kind::TurnEnded).unwrap().unwrap();

        assert_eq!(preview(dir, "r", start.seq).unwrap().unsaved, vec!["out/cache.log"]);
        let err = restore(dir, "r", start.seq).unwrap_err().to_string();
        assert!(err.contains("out/cache.log"), "{err}");
        assert_eq!(read(dir, "out/cache.log").as_deref(), Some("ignored"));
        assert_eq!(read(dir, "out/a.txt").as_deref(), Some("tracked-ish"), "nothing moved");
    }

    #[test]
    fn restore_refuses_to_replace_ignored_files() {
        let d = tempdir().unwrap();
        let dir = d.path();
        init(dir);
        fs::create_dir_all(dir.join("gen")).unwrap();
        fs::write(dir.join("gen/x.txt"), "x").unwrap();
        fs::write(dir.join("notes.log"), "x").unwrap();
        git(dir, &["rm", "-q", "--cached", ".gitignore"]);
        fs::remove_file(dir.join(".gitignore")).unwrap();
        let start = capture(dir, "r", Kind::RunStart).unwrap().unwrap();
        // Now `gen` is an ignored *file* where the checkpoint has a directory,
        // and `notes.log` is ignored where the checkpoint has a file.
        fs::remove_dir_all(dir.join("gen")).unwrap();
        fs::write(dir.join("gen"), "ignored").unwrap();
        fs::write(dir.join(".gitignore"), "gen\n*.log\n").unwrap();
        capture(dir, "r", Kind::TurnEnded).unwrap().unwrap();
        let mut unsaved = preview(dir, "r", start.seq).unwrap().unsaved;
        unsaved.sort();
        assert_eq!(unsaved, vec!["gen", "notes.log"]);
        assert!(restore(dir, "r", start.seq).is_err());
        assert_eq!(read(dir, "gen").as_deref(), Some("ignored"));
    }

    #[test]
    fn a_nested_repository_without_a_commit_does_not_stop_checkpoints() {
        let d = tempdir().unwrap();
        let dir = d.path();
        init(dir);
        let start = capture(dir, "r", Kind::RunStart).unwrap().unwrap();
        // What a scaffold that runs `git init` leaves: `git add -A` exits 128
        // on it ("does not have a commit checked out").
        fs::create_dir(dir.join("sub")).unwrap();
        git(&dir.join("sub"), &["init", "-q"]);
        fs::write(dir.join("sub/f.txt"), "inner").unwrap();
        fs::write(dir.join("a.txt"), "changed\n").unwrap();
        let cp = capture(dir, "r", Kind::TurnEnded).unwrap().expect("a.txt changed");
        let files = git(dir, &["ls-tree", "-r", "--name-only", &cp.tree]);
        assert!(!files.contains("sub"), "left out: {files}");
        restore(dir, "r", start.seq).unwrap();
        assert_eq!(read(dir, "a.txt").as_deref(), Some("one\n"));
        assert_eq!(read(dir, "sub/f.txt").as_deref(), Some("inner"), "left alone");
    }

    #[test]
    fn edits_to_flagged_files_are_captured_and_restored() {
        let d = tempdir().unwrap();
        let dir = d.path();
        init(dir);
        fs::write(dir.join("local.cfg"), "mine\n").unwrap();
        fs::write(dir.join("skip.cfg"), "mine\n").unwrap();
        git(dir, &["add", "local.cfg", "skip.cfg"]);
        git(dir, &["commit", "-qm", "cfg"]);
        git(dir, &["update-index", "--assume-unchanged", "local.cfg"]);
        git(dir, &["update-index", "--skip-worktree", "skip.cfg"]);
        let start = capture(dir, "r", Kind::RunStart).unwrap().unwrap();
        fs::write(dir.join("local.cfg"), "agent\n").unwrap();
        fs::write(dir.join("skip.cfg"), "agent\n").unwrap();
        let end = capture(dir, "r", Kind::TurnEnded).unwrap().expect("both edits seen");
        let mut got: Vec<String> =
            changes(dir, &start.commit, &end.commit).unwrap().into_iter().map(|c| c.path).collect();
        got.sort();
        assert_eq!(got, vec!["local.cfg", "skip.cfg"]);
        restore(dir, "r", start.seq).unwrap();
        assert_eq!(read(dir, "local.cfg").as_deref(), Some("mine\n"));
        assert_eq!(read(dir, "skip.cfg").as_deref(), Some("mine\n"));
        // And the user's own index still has both bits.
        let flags = git(dir, &["ls-files", "-v", "local.cfg", "skip.cfg"]);
        assert!(flags.contains("h local.cfg") && flags.contains("S skip.cfg"), "{flags}");
    }

    #[test]
    fn a_rollback_that_does_not_verify_says_so() {
        // A clean filter that does not round-trip: it adds a line to whatever
        // it reads, so a snapshot never sees what was checked out and neither
        // the restore nor the rollback can match.
        let d = tempdir().unwrap();
        let dir = d.path();
        init(dir);
        let start = capture(dir, "r", Kind::RunStart).unwrap().unwrap();
        fs::write(dir.join("a.txt"), "edited\n").unwrap();
        capture(dir, "r", Kind::TurnEnded).unwrap().unwrap();
        git(dir, &["config", "filter.drift.clean", "cat; echo drift"]);
        fs::write(dir.join(".git/info/attributes"), "a.txt filter=drift\n").unwrap();
        let err = restore(dir, "r", start.seq).unwrap_err().to_string();
        assert!(err.contains("putting the files back did not either"), "{err}");
        assert!(!err.contains("your files were put back"), "{err}");
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
