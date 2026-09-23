//! Per-turn workspace checkpoints (AGE-140): the pure half.
//!
//! The most common thing a person wants after an agent takes a wrong turn is
//! the previous state of the files back, and before this the only way to get it
//! was `git reset` typed into a terminal pane. A checkpoint is the whole
//! worktree written as a tree object through a throwaway index, wrapped in a
//! parentless commit, and pointed at by a ref under [`REF_ROOT`]:
//!
//! ```text
//! refs/agency/checkpoints/<run>/<seq>
//! ```
//!
//! The user's index, HEAD, branches and reflog are never touched, so `git
//! status` does not move; the refs sit outside `refs/heads` and `refs/tags`, so
//! nothing that lists branches sees them. The commit has no parent, so pruning a
//! ref really does let its objects go: a chain of checkpoints would keep every
//! earlier one reachable from the newest.
//!
//! Everything here is a function of strings in and strings out: ref naming, the
//! commit message, parsing `for-each-ref` and `diff-tree --raw`, which refs to
//! prune, and what a restore has to write and remove. The git calls are the
//! side effects and live in [`store`].

pub mod store;

use serde::Serialize;
use std::collections::HashSet;

/// The private ref namespace. Shared by every worktree of a repository (only
/// `refs/worktree`, `refs/bisect` and `refs/rewritten` are per-worktree), which
/// is why the run id is part of every name.
pub const REF_ROOT: &str = "refs/agency/checkpoints";

/// Checkpoints kept per run before the oldest start to go. A capture that finds
/// nothing changed stores nothing, so this is a hundred *distinct* states: a
/// long day of turns, not a long day of polling.
pub const KEEP_PER_RUN: usize = 100;

/// An untracked file larger than this is left out of a checkpoint.
///
/// `git add` writes every byte it is handed into the object store, and the ref
/// keeps it there. One dataset or build artefact an agent drops into the tree
/// without a `.gitignore` line would otherwise cost its full size again for as
/// long as the run lives. 50 MB is where GitHub starts warning about a file,
/// which is a fair line for "not source". Tracked files are never skipped: their
/// history is already in the repository.
pub const MAX_UNTRACKED_BYTES: u64 = 50 * 1024 * 1024;

/// Every checkpoint commit's subject starts with this, followed by the
/// [`Kind`]'s word.
const SUBJECT_PREFIX: &str = "agency checkpoint: ";

/// The mode git gives a submodule (or any nested repository `add` meets).
const GITLINK_MODE: &str = "160000";

/// What prompted a checkpoint. Stored as a word in the commit subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// The run's workspace as it was handed to the agent.
    RunStart,
    /// A turn began: the user pressed Enter in the agent's pane, or the send
    /// queue typed something in on their behalf.
    PromptSent,
    /// The agent's pane went quiet after a stretch of work.
    TurnEnded,
    /// The files as they stood just before a restore replaced them, which is
    /// what makes a restore itself undoable.
    BeforeRestore,
    /// A subject this build does not recognise: a newer build's kind, or a ref
    /// somebody wrote by hand. Listed rather than dropped, since its tree is
    /// still a real state of the files.
    Unknown,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::RunStart => "run start",
            Kind::PromptSent => "prompt sent",
            Kind::TurnEnded => "turn ended",
            Kind::BeforeRestore => "before restore",
            Kind::Unknown => "unknown",
        }
    }

    /// The kind a subject names. Exact match against the words above; anything
    /// else is [`Kind::Unknown`].
    fn from_subject(subject: &str) -> Kind {
        let Some(word) = subject.strip_prefix(SUBJECT_PREFIX) else { return Kind::Unknown };
        [Kind::RunStart, Kind::PromptSent, Kind::TurnEnded, Kind::BeforeRestore]
            .into_iter()
            .find(|k| k.word() == word.trim())
            .unwrap_or(Kind::Unknown)
    }
}

/// One stored checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    /// Position in the run's sequence, from 1. Also the last component of the
    /// ref name.
    pub seq: u32,
    pub commit: String,
    pub tree: String,
    /// Unix seconds, from the commit's committer date.
    pub at: i64,
    pub kind: Kind,
    /// The commit HEAD was on when it was taken, if there was one. A restore
    /// puts files back and nothing else, so this is how the UI can say that the
    /// branch has moved on since.
    pub head: Option<String>,
}

/// Whether `id` can be spliced into a ref name as one path component.
///
/// Run ids are slugs (`age-140-per-turn-…-qpur`), so this should never refuse
/// one; it exists because the id reaches `update-ref` and `for-each-ref`
/// arguments, and an allowlist of characters is the version of that check that
/// cannot leak. `-` may not lead, so the argument can never read as a flag.
pub fn valid_run_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && !id.starts_with(['.', '-'])
        && !id.ends_with(".lock")
        && !id.contains("..")
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// `refs/agency/checkpoints/<run>/`, the prefix every one of a run's refs has.
/// With the trailing slash, so `for-each-ref` on it cannot also match a run
/// whose id merely starts with this one.
pub fn run_prefix(run_id: &str) -> String {
    format!("{REF_ROOT}/{run_id}/")
}

pub fn ref_name(run_id: &str, seq: u32) -> String {
    format!("{}{seq}", run_prefix(run_id))
}

/// The commit message: the kind on the subject line, then the HEAD it was taken
/// on, if any. No timestamps or counters: the commit's own date carries the
/// time, and the sequence number is the ref's.
pub fn message(kind: Kind, head: Option<&str>) -> String {
    match head {
        Some(h) => format!("{SUBJECT_PREFIX}{}\n\nhead {h}\n", kind.word()),
        None => format!("{SUBJECT_PREFIX}{}\n", kind.word()),
    }
}

/// The `for-each-ref --format` [`parse_list`] reads. Fields are tab-separated
/// and each record ends in a NUL, because `%(contents)` is the whole message and
/// has newlines of its own.
pub const LIST_FORMAT: &str =
    "%(refname)%09%(objectname)%09%(tree)%09%(committerdate:unix)%09%(contents)%00";

/// A run's checkpoints from `for-each-ref --format=LIST_FORMAT <run_prefix>`,
/// oldest first. A ref whose last component is not a plain number, or that is
/// not a commit, is skipped: nothing here wrote it.
pub fn parse_list(run_id: &str, out: &str) -> Vec<Checkpoint> {
    let prefix = run_prefix(run_id);
    let mut list: Vec<Checkpoint> = out
        .split('\0')
        .filter_map(|record| {
            let record = record.trim_start_matches('\n');
            let mut fields = record.splitn(5, '\t');
            let name = fields.next()?;
            let commit = fields.next()?;
            let tree = fields.next()?;
            let at = fields.next()?.trim().parse::<i64>().ok()?;
            let contents = fields.next().unwrap_or("");
            let seq_str = name.strip_prefix(&prefix)?;
            if seq_str.is_empty() || !seq_str.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let seq = seq_str.parse::<u32>().ok()?;
            // `%(tree)` is empty for anything that is not a commit.
            if !is_oid(commit) || !is_oid(tree) {
                return None;
            }
            let subject = contents.lines().next().unwrap_or("");
            let head = contents
                .lines()
                .find_map(|l| l.strip_prefix("head "))
                .map(str::trim)
                .filter(|h| is_oid(h))
                .map(str::to_string);
            Some(Checkpoint {
                seq,
                commit: commit.to_string(),
                tree: tree.to_string(),
                at,
                kind: Kind::from_subject(subject),
                head,
            })
        })
        .collect();
    list.sort_by_key(|c| c.seq);
    list
}

/// The sequence number the next checkpoint takes.
pub fn next_seq(list: &[Checkpoint]) -> u32 {
    list.last().map_or(1, |c| c.seq.saturating_add(1))
}

/// Whether a capture of `tree` would repeat the newest checkpoint. Nothing
/// changed since, so storing it would add a row with an empty diff and push a
/// real state one step nearer the cap.
pub fn is_unchanged(list: &[Checkpoint], tree: &str) -> bool {
    list.last().is_some_and(|c| c.tree == tree)
}

/// The refs to delete so a run keeps at most `keep` checkpoints.
///
/// The first is always kept, whatever its age: it is the workspace as the agent
/// was handed it, the one state a person reaching for "undo all of it" wants,
/// and the one a rolling window would throw away first. The rest of the room
/// goes to the newest.
pub fn over_cap(run_id: &str, list: &[Checkpoint], keep: usize) -> Vec<String> {
    let keep = keep.max(2);
    if list.len() <= keep {
        return Vec::new();
    }
    let drop = list.len() - keep;
    list[1..=drop].iter().map(|c| ref_name(run_id, c.seq)).collect()
}

/// A full object id: 40 hex digits for SHA-1, 64 for SHA-256.
///
/// Every revision the UI sends back to be diffed goes through this before it
/// reaches git's argv. A free-form "revision" is also a free-form *argument*,
/// and `--output=<file>` is a revision to nobody but git's option parser.
pub fn is_oid(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// One path's change between two trees, from `diff-tree -r -z --no-renames
/// --raw`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawChange {
    pub old_mode: String,
    pub new_mode: String,
    /// `A`, `D`, `M` or `T` (type change). Renames and copies never appear:
    /// the diff is asked for without them.
    pub status: char,
    pub path: String,
}

impl RawChange {
    fn touches_gitlink(&self) -> bool {
        self.old_mode == GITLINK_MODE || self.new_mode == GITLINK_MODE
    }
}

/// Parse `diff-tree -r -z --no-renames --raw` output: records of
/// `:<old mode> <new mode> <old oid> <new oid> <status>` NUL `<path>` NUL.
///
/// Paths come through byte for byte (spaces, newlines, non-ASCII), which is the
/// point of `-z`: the quoted form git uses without it would have to be undone
/// here, and getting that wrong means restoring the wrong file.
pub fn parse_raw_z(out: &str) -> Vec<RawChange> {
    let mut changes = Vec::new();
    let mut parts = out.split('\0');
    while let Some(meta) = parts.next() {
        let Some(meta) = meta.trim_start_matches('\n').strip_prefix(':') else { continue };
        let Some(path) = parts.next() else { break };
        let fields: Vec<&str> = meta.split(' ').collect();
        if fields.len() < 5 || path.is_empty() {
            continue;
        }
        let Some(status) = fields[4].chars().next() else { continue };
        changes.push(RawChange {
            old_mode: fields[0].to_string(),
            new_mode: fields[1].to_string(),
            status,
            path: path.to_string(),
        });
    }
    changes
}

/// What a restore does to the working tree: files to delete, and files to write
/// from the target tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestorePlan {
    /// In the files now, not in the target: created since. Deleted.
    pub remove: Vec<String>,
    /// Different or missing now: written from the target.
    pub write: Vec<String>,
    /// The part of `write` the current snapshot has no file at. Anything on
    /// disk there is something the snapshot left out (ignored, or too large),
    /// so the "before restore" checkpoint does not hold it either.
    pub added: Vec<String>,
}

impl RestorePlan {
    pub fn is_empty(&self) -> bool {
        self.remove.is_empty() && self.write.is_empty()
    }

    pub fn len(&self) -> usize {
        self.remove.len() + self.write.len()
    }
}

/// The plan that takes the files from the current snapshot to the target, given
/// the changes `diff-tree <current> <target>` reports.
///
/// A submodule is left alone in both directions. Its entry is a commit in
/// another repository, not a file: "restoring" it would mean checking that
/// repository out, and "removing" it would mean deleting a directory with its
/// own history in it.
///
/// Removals are kept deepest-first so that when a file became a directory (or
/// the reverse) the old one is out of the way before [`store`] writes the new
/// one.
pub fn restore_plan(changes: &[RawChange]) -> RestorePlan {
    let mut plan = RestorePlan::default();
    for c in changes.iter().filter(|c| !c.touches_gitlink()) {
        match c.status {
            'D' => plan.remove.push(c.path.clone()),
            'A' => {
                plan.write.push(c.path.clone());
                plan.added.push(c.path.clone());
            }
            'M' | 'T' => plan.write.push(c.path.clone()),
            _ => {}
        }
    }
    plan.remove.sort_by(|a, b| b.matches('/').count().cmp(&a.matches('/').count()).then(a.cmp(b)));
    plan
}

/// The files a restore would destroy that the checkpoint taken just before it
/// does not hold, given `in_the_way`: every file or symlink on disk that
/// writing `plan.write` has to unlink. That is whatever sits at a path being
/// written, *everything* under it when it is a directory, and any file sitting
/// where a written path needs a parent directory. `checkout-index -f` clears
/// all of those without asking, a whole directory included.
///
/// The "before restore" checkpoint holds exactly the files the restore
/// removes, plus the ones it overwrites that the snapshot already had. Anything
/// else in the way was never in a snapshot: an ignored file, an untracked file
/// over [`MAX_UNTRACKED_BYTES`], or a nested repository's working files. The
/// restore refuses rather than destroy those.
///
/// Observed: at a checkpoint `out` was a file; the agent then replaced it with
/// a directory holding `out/a.txt` and an ignored `out/cache.log`. Restoring
/// removed `a.txt`, could not remove the non-empty directory, and
/// `checkout-index -f out` then deleted `cache.log` with it. The verify step
/// passed, since ignored files are in no snapshot, and the file was gone for
/// good.
pub fn unsaved(plan: &RestorePlan, in_the_way: &[String]) -> Vec<String> {
    fn set(v: &[String]) -> HashSet<&str> {
        v.iter().map(String::as_str).collect()
    }
    let (remove, write, added) = (set(&plan.remove), set(&plan.write), set(&plan.added));
    let saved = |p: &str| remove.contains(p) || (write.contains(p) && !added.contains(p));
    let mut out: Vec<String> = in_the_way.iter().filter(|p| !saved(p)).cloned().collect();
    out.sort();
    out.dedup();
    out
}

/// The untracked files over `cap`, from `(path, size)` pairs.
pub fn oversized(untracked: &[(String, u64)], cap: u64) -> Vec<String> {
    untracked.iter().filter(|(_, size)| *size > cap).map(|(p, _)| p.clone()).collect()
}

/// What `ls-files -z -v -c -o --exclude-standard` says about a worktree, read
/// in one pass instead of one listing per question.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    /// Untracked, not ignored: what `add` has to be handed explicitly. Nested
    /// repositories are left out (see [`parse_listing`]).
    pub untracked: Vec<String>,
    /// Tracked entries marked assume-unchanged.
    pub assume_unchanged: Vec<String>,
    /// Tracked entries marked skip-worktree.
    pub skip_worktree: Vec<String>,
}

/// Parse `ls-files -z -v -c -o --exclude-standard`: records of `<tag> <path>`,
/// NUL-terminated. `?` is untracked, `S` skip-worktree, and a lower-case tag is
/// an assume-unchanged entry.
///
/// The index a snapshot starts from is a copy of the user's, flag bits and
/// all, and `add` believes both bits: a file marked assume-unchanged (the
/// usual trick for a local config) or skip-worktree was never re-read, so an
/// agent's edit to it reached no checkpoint and no restore could undo it. The
/// snapshot clears the bits on its own copy, and needs to know which to clear.
///
/// An untracked path ending in `/` is a nested repository: git lists the
/// directory, not its files. Those are left out of a snapshot altogether. One
/// with no commit yet makes `git add` fail outright ("does not have a commit
/// checked out"), which is what an agent running `git init` in a scaffold
/// leaves behind, and one failure there ended checkpoints for the rest of the
/// run. One with commits would only be stored as a gitlink, which a restore
/// never touches anyway.
pub fn parse_listing(out: &str) -> Listing {
    let mut listing = Listing::default();
    for record in out.split('\0') {
        let Some((tag, path)) = record.split_once(' ') else { continue };
        let mut chars = tag.chars();
        let (Some(tag), None) = (chars.next(), chars.next()) else { continue };
        if path.is_empty() {
            continue;
        }
        match tag {
            '?' if path.ends_with('/') => {}
            '?' => listing.untracked.push(path.to_string()),
            'S' => listing.skip_worktree.push(path.to_string()),
            's' => {
                listing.assume_unchanged.push(path.to_string());
                listing.skip_worktree.push(path.to_string());
            }
            t if t.is_ascii_lowercase() => listing.assume_unchanged.push(path.to_string()),
            _ => {}
        }
    }
    listing
}

/// The directories above `path`, deepest first: `a/b/c.txt` gives `a/b`, `a`.
/// A removal tries each in turn so a directory the restore emptied does not
/// linger; a directory with anything left in it simply refuses.
pub fn parent_dirs(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = path;
    while let Some((dir, _)) = rest.rsplit_once('/') {
        if dir.is_empty() {
            break;
        }
        out.push(dir.to_string());
        rest = dir;
    }
    out
}

/// Whether a path from a tree is safe to join onto the worktree: relative, and
/// no `.`, `..` or empty component. git itself refuses to write such an entry,
/// so this should never trip; it is here because the next line deletes files.
pub fn safe_rel_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    const C1: &str = "13ae59846e7c2fca8440bddda9b5714b9a24fb7d";
    const T1: &str = "29f4db92cfce562444a6da89d2493dbee1b28b31";
    const H1: &str = "bc272a21f717a8c420ea43e24743d9c33dae7ee5";

    fn cp(seq: u32, tree: &str) -> Checkpoint {
        Checkpoint {
            seq,
            commit: C1.to_string(),
            tree: tree.to_string(),
            at: 0,
            kind: Kind::TurnEnded,
            head: None,
        }
    }

    #[test]
    fn ref_names_nest_under_the_run() {
        assert_eq!(run_prefix("fix-login-ab12"), "refs/agency/checkpoints/fix-login-ab12/");
        assert_eq!(ref_name("fix-login-ab12", 7), "refs/agency/checkpoints/fix-login-ab12/7");
    }

    #[test]
    fn run_ids_are_allowlisted() {
        assert!(valid_run_id("age-140-per-turn-workspace-checkpoints-a-qpur"));
        assert!(valid_run_id("run_1.2"));
        for bad in ["", "-x", ".x", "a..b", "a/b", "a b", "x.lock", "a\nb", "a:b", "a~1", "é"] {
            assert!(!valid_run_id(bad), "{bad:?} must be refused");
        }
    }

    #[test]
    fn message_round_trips_through_the_list_parser() {
        let msg = message(Kind::PromptSent, Some(H1));
        let out = format!("{}\t{C1}\t{T1}\t1790190627\t{msg}\0\n", ref_name("r", 3));
        let list = parse_list("r", &out);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].seq, 3);
        assert_eq!(list[0].kind, Kind::PromptSent);
        assert_eq!(list[0].head.as_deref(), Some(H1));
        assert_eq!(list[0].at, 1790190627);
        assert_eq!(list[0].tree, T1);
    }

    #[test]
    fn every_kind_round_trips() {
        for k in [Kind::RunStart, Kind::PromptSent, Kind::TurnEnded, Kind::BeforeRestore] {
            let subject = message(k, None);
            assert_eq!(Kind::from_subject(subject.lines().next().unwrap()), k);
        }
        assert_eq!(Kind::from_subject("agency checkpoint: something new"), Kind::Unknown);
        assert_eq!(Kind::from_subject("WIP"), Kind::Unknown);
    }

    #[test]
    fn list_is_sorted_numerically_not_lexically() {
        // for-each-ref sorts by refname, so 10 comes before 9.
        let rec = |seq: u32| {
            format!("{}\t{C1}\t{T1}\t0\t{}\0\n", ref_name("r", seq), message(Kind::TurnEnded, None))
        };
        let out = [rec(10), rec(2), rec(9)].concat();
        let seqs: Vec<u32> = parse_list("r", &out).iter().map(|c| c.seq).collect();
        assert_eq!(seqs, vec![2, 9, 10]);
    }

    #[test]
    fn list_skips_refs_it_did_not_write() {
        let out = [
            // Not a number.
            format!("{}latest\t{C1}\t{T1}\t0\tx\0\n", run_prefix("r")),
            // A tag or blob: no tree.
            format!("{}4\t{C1}\t\t0\tx\0\n", run_prefix("r")),
            // Another run whose id starts with this one's.
            format!("{}5\t{C1}\t{T1}\t0\tx\0\n", run_prefix("r2")),
            // Nested deeper.
            format!("{}6/7\t{C1}\t{T1}\t0\tx\0\n", run_prefix("r")),
        ]
        .concat();
        assert!(parse_list("r", &out).is_empty());
    }

    #[test]
    fn a_forged_head_line_is_ignored() {
        let out = format!(
            "{}1\t{C1}\t{T1}\t0\tagency checkpoint: turn ended\n\nhead --output=x\n\0\n",
            run_prefix("r")
        );
        assert_eq!(parse_list("r", &out)[0].head, None);
    }

    #[test]
    fn next_seq_follows_the_newest() {
        assert_eq!(next_seq(&[]), 1);
        assert_eq!(next_seq(&[cp(1, T1), cp(4, T1)]), 5);
    }

    #[test]
    fn unchanged_compares_against_the_newest_only() {
        let other = "0".repeat(40);
        assert!(!is_unchanged(&[], T1));
        assert!(is_unchanged(&[cp(1, &other), cp(2, T1)], T1));
        // Going back to an older state is a change: that is a real turn.
        assert!(!is_unchanged(&[cp(1, T1), cp(2, &other)], T1));
    }

    #[test]
    fn over_cap_keeps_the_first_and_the_newest() {
        let list: Vec<Checkpoint> = (1..=6).map(|n| cp(n, T1)).collect();
        assert!(over_cap("r", &list, 6).is_empty());
        assert_eq!(over_cap("r", &list, 4), vec![ref_name("r", 2), ref_name("r", 3)]);
        // A cap under two would leave nothing but the start; it is clamped.
        assert_eq!(over_cap("r", &list, 0).len(), 4);
    }

    #[test]
    fn oids_are_full_hex_only() {
        assert!(is_oid(C1));
        assert!(is_oid(&"a".repeat(64)));
        for bad in ["HEAD", "--output=/tmp/x", &C1[..39], "g".repeat(40).as_str(), ""] {
            assert!(!is_oid(bad), "{bad:?}");
        }
    }

    #[test]
    fn parses_raw_z_including_awkward_paths() {
        let z = "0".repeat(40);
        let out = format!(
            ":100644 100644 {C1} {T1} M\0with space.txt\0\
             :000000 100644 {z} {T1} A\0new\nline.txt\0\
             :100644 000000 {C1} {z} D\0héllo/wörld.md\0\
             :100644 120000 {C1} {T1} T\0link\0"
        );
        let changes = parse_raw_z(&out);
        let got: Vec<(char, &str)> = changes.iter().map(|c| (c.status, c.path.as_str())).collect();
        assert_eq!(
            got,
            vec![
                ('M', "with space.txt"),
                ('A', "new\nline.txt"),
                ('D', "héllo/wörld.md"),
                ('T', "link")
            ]
        );
        assert_eq!(changes[3].new_mode, "120000");
        assert!(parse_raw_z("").is_empty());
    }

    #[test]
    fn plan_writes_additions_and_edits_and_removes_deletions() {
        let ch = |status: char, path: &str| RawChange {
            old_mode: "100644".into(),
            new_mode: "100644".into(),
            status,
            path: path.into(),
        };
        let plan = restore_plan(&[ch('M', "a"), ch('A', "b"), ch('D', "c"), ch('T', "d")]);
        assert_eq!(plan.write, vec!["a", "b", "d"]);
        assert_eq!(plan.remove, vec!["c"]);
        assert_eq!(plan.len(), 4);
    }

    #[test]
    fn plan_removes_deepest_first() {
        let d = |path: &str| RawChange {
            old_mode: "100644".into(),
            new_mode: "000000".into(),
            status: 'D',
            path: path.into(),
        };
        let plan = restore_plan(&[d("x"), d("a/b/c"), d("a/b"), d("a/z")]);
        assert_eq!(plan.remove, vec!["a/b/c", "a/b", "a/z", "x"]);
    }

    #[test]
    fn plan_leaves_submodules_alone() {
        let sub = |status: char, old: &str, new: &str| RawChange {
            old_mode: old.into(),
            new_mode: new.into(),
            status,
            path: "vendor/lib".into(),
        };
        let plan = restore_plan(&[
            sub('M', GITLINK_MODE, GITLINK_MODE),
            sub('D', GITLINK_MODE, "000000"),
            sub('A', "000000", GITLINK_MODE),
            sub('T', "100644", GITLINK_MODE),
        ]);
        assert!(plan.is_empty());
    }

    #[test]
    fn a_restore_may_not_destroy_what_it_could_not_save() {
        let plan = RestorePlan {
            remove: vec!["out/a.txt".into()],
            write: vec!["out".into(), "a".into(), "data.csv".into()],
            added: vec!["out".into(), "data.csv".into()],
        };
        let in_the_way: Vec<String> =
            ["out/a.txt", "out/cache.log", "a", "data.csv"].map(String::from).to_vec();
        // `out/a.txt` is removed (saved), `a` is overwritten but the snapshot
        // had it. `out/cache.log` is ignored and `data.csv` is where the
        // snapshot had nothing: neither was saved.
        assert_eq!(unsaved(&plan, &in_the_way), vec!["data.csv", "out/cache.log"]);
        assert!(unsaved(&plan, &["out/a.txt".to_string(), "a".to_string()]).is_empty());
    }

    #[test]
    fn plan_marks_what_the_current_files_do_not_have() {
        let ch = |status: char, path: &str| RawChange {
            old_mode: "100644".into(),
            new_mode: "100644".into(),
            status,
            path: path.into(),
        };
        let plan = restore_plan(&[ch('M', "a"), ch('A', "b"), ch('T', "c")]);
        assert_eq!(plan.added, vec!["b"]);
    }

    #[test]
    fn listing_sorts_out_untracked_files_and_index_flags() {
        let out =
            "? new.txt\0? sub/\0? deep/sub/\0H a\0h local.cfg\0S sparse/x\0s both\0? with space\0";
        let l = parse_listing(out);
        assert_eq!(l.untracked, vec!["new.txt", "with space"], "nested repositories left out");
        assert_eq!(l.assume_unchanged, vec!["local.cfg", "both"]);
        assert_eq!(l.skip_worktree, vec!["sparse/x", "both"]);
        assert_eq!(parse_listing(""), Listing::default());
    }

    #[test]
    fn oversized_is_strictly_over_the_cap() {
        let files = vec![("a".to_string(), 10), ("b".to_string(), 11), ("c".to_string(), 9)];
        assert_eq!(oversized(&files, 10), vec!["b"]);
    }

    #[test]
    fn parent_dirs_walk_up_to_the_root() {
        assert_eq!(parent_dirs("a/b/c.txt"), vec!["a/b", "a"]);
        assert!(parent_dirs("top.txt").is_empty());
    }

    #[test]
    fn safe_paths_stay_inside_the_worktree() {
        assert!(safe_rel_path("src/main.rs"));
        assert!(safe_rel_path(".github/workflows/ci.yml"));
        for bad in ["", "/etc/passwd", "../x", "a/../b", "a//b", "./a", "a\\b"] {
            assert!(!safe_rel_path(bad), "{bad:?}");
        }
    }
}
