//! The transport behind issue sync: `.agency/issues/` carried on a git ref
//! that is never checked out.
//!
//! [`issuesync`](crate::issuesync) decides *what* the merge should be; this
//! decides how the two sides reach each other. The whole design rests on one
//! property: the issue files are snapshotted into `refs/agency/issues` through
//! a **temporary index**, so nothing here ever touches the repository's real
//! index or working tree. That is what keeps the three reasons the files left
//! the working tree from coming back (see `docs/tracked-issues.md`) — the
//! checkout never goes dirty, agent branches never carry issue frontmatter, and
//! the tracker does not fork per branch, because no branch contains it.
//!
//! The ref's history is also what makes a three-way merge possible at all: the
//! merge base of the local ref and the fetched one *is* the last state both
//! sides saw, so deletions are distinguishable from never-having-existed
//! without inventing a tombstone format.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::issuefs::{self, IssueFile};
use crate::issuesync::{self, Action, Mode, Plan};

/// Where a project's issues live when they are shared. Not under `refs/heads/`
/// or `refs/remotes/`, so it is invisible to branch listings, to `git log
/// --all`, and to a plain `git push`; nothing carries it by accident.
pub const LOCAL_REF: &str = "refs/agency/issues";

/// Where a fetched copy of the other side lands. Kept as a real ref rather than
/// read from `FETCH_HEAD` so `git merge-base` can be asked about it.
pub const REMOTE_REF: &str = "refs/agency/issues-remote";

/// What a sync pass did, for the caller to show.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// Issue files written locally (created or updated) by the merge.
    pub written: usize,
    /// Issue files deleted locally by the merge.
    pub deleted: usize,
    /// The merge's judgement calls, verbatim from [`issuesync::Plan`].
    pub conflicts: Vec<issuesync::Conflict>,
    /// Local files left out because they carry no `uid` yet.
    pub skipped: Vec<(String, String)>,
    /// Did the local ref move? False means both sides already agreed.
    pub committed: bool,
    /// Did the push run and succeed? False with `committed` true means the
    /// merge landed locally but the remote did not take it.
    pub pushed: bool,
}

/// Why a sync could not run as asked.
#[derive(Debug, PartialEq)]
pub enum Blocked {
    /// Both sides have issues but no shared history, so there is no base to
    /// merge against and every key looks contested. The user has to say which
    /// side seeds the other: [`Mode::Publish`] or [`Mode::Adopt`].
    NeedsSeeding { local: usize, remote: usize },
}

impl std::fmt::Display for Blocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Blocked::NeedsSeeding { local, remote } => write!(
                f,
                "this machine has {local} issues and the shared tracker has {remote}, with no \
                 history in common. Choose which one seeds the other."
            ),
        }
    }
}
impl std::error::Error for Blocked {}

// ---------------------------------------------------------------------------
// git plumbing

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    git_env(repo, args, &[])
}

/// Every git call here goes through one place so the two environment rules hold
/// everywhere: no terminal prompt (the app has no TTY to answer a credential
/// question on, and a fetch that blocks on a hidden prompt never returns), and
/// an author identity that cannot be missing.
fn git_env(repo: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(repo).env("GIT_TERMINAL_PROMPT", "0");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        bail!("git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// A git call whose failure is an answer, not an error: "no such ref", "no
/// merge base". Distinguishing those from a real failure by parsing stderr
/// would be guesswork, and every caller here wants the same `Option`.
fn git_opt(repo: &Path, args: &[&str]) -> Option<String> {
    let out = git(repo, args).ok()?;
    let s = out.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn rev(repo: &Path, r: &str) -> Option<String> {
    git_opt(repo, &["rev-parse", "--verify", "--quiet", r])
}

/// The commit both refs descend from, or `None` when they share no history.
fn merge_base(repo: &Path, a: &str, b: &str) -> Option<String> {
    git_opt(repo, &["merge-base", a, b])
}

/// Snapshot `.agency/issues/` into a git tree without touching the real index.
///
/// `git add` is given a scratch `GIT_INDEX_FILE` and `--force`, the latter
/// because the directory is deliberately in `.git/info/exclude` and would
/// otherwise be skipped by the very mechanism that keeps it out of the working
/// tree. The scratch index is removed afterwards either way.
fn write_tree(repo: &Path) -> Result<String> {
    let dir = repo.join(issuefs::ISSUES_DIR);
    if !dir.is_dir() {
        // An empty tree, asked for rather than hardcoded: the well-known
        // 4b825dc… is the sha1 spelling, and a sha256 repo has a different one.
        return Ok(git(repo, &["mktree"])?.trim().to_string());
    }
    // Asked for rather than assumed to be `<repo>/.git`: in a linked worktree
    // that path is a *file* pointing elsewhere, and joining onto it would put
    // the scratch index somewhere that cannot be created.
    let git_dir = git(repo, &["rev-parse", "--absolute-git-dir"])?.trim().to_string();
    let index: PathBuf =
        Path::new(&git_dir).join(format!("agency-issues-index-{}", uuid::Uuid::new_v4()));
    let index_str = index.to_string_lossy().to_string();
    let env = [("GIT_INDEX_FILE", index_str.as_str())];
    let run = || -> Result<String> {
        git_env(repo, &["add", "--force", "--all", "--", issuefs::ISSUES_DIR], &env)?;
        Ok(git_env(repo, &["write-tree"], &env)?.trim().to_string())
    };
    let out = run();
    let _ = std::fs::remove_file(&index);
    out
}

/// Commit a tree onto the local ref. `parents` are the commits this snapshot
/// descends from — both sides after a merge, which is what gives the *next*
/// sync a merge base to work from.
///
/// Returns `None` when the tree already matches the ref's tree: an unchanged
/// backlog must not produce a commit per sync, or the ref's history becomes
/// noise and every merge base is the previous minute.
fn commit_tree(
    repo: &Path,
    tree: &str,
    parents: &[String],
    message: &str,
) -> Result<Option<String>> {
    if let Some(head) = rev(repo, LOCAL_REF) {
        if git_opt(repo, &["rev-parse", &format!("{head}^{{tree}}")]).as_deref() == Some(tree) {
            return Ok(None);
        }
    }
    let mut args: Vec<String> = vec!["commit-tree".into(), tree.into()];
    for p in parents {
        args.push("-p".into());
        args.push(p.clone());
    }
    args.push("-m".into());
    args.push(message.into());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();

    // A repo with no user.name cannot commit, and a tracker sync is no place to
    // fail on that. The repo's own identity is used when it has one.
    let name = crate::git::user_name(repo).unwrap_or_else(|| "Agency".into());
    let email =
        git_opt(repo, &["config", "user.email"]).unwrap_or_else(|| "agency@localhost".into());
    let env = [
        ("GIT_AUTHOR_NAME", name.as_str()),
        ("GIT_AUTHOR_EMAIL", email.as_str()),
        ("GIT_COMMITTER_NAME", name.as_str()),
        ("GIT_COMMITTER_EMAIL", email.as_str()),
    ];
    let commit = git_env(repo, &refs, &env)?.trim().to_string();
    git(repo, &["update-ref", LOCAL_REF, &commit])?;
    Ok(Some(commit))
}

/// Every issue file in a commit, parsed. A file that does not parse is skipped
/// with a warning rather than failing the sync: one corrupt file on the other
/// side must not stop the other two hundred from arriving.
fn read_tree(repo: &Path, at: &str) -> Result<Vec<IssueFile>> {
    let listing = git(repo, &["ls-tree", "-r", "--name-only", at, "--", issuefs::ISSUES_DIR])
        .unwrap_or_default();
    let mut out = Vec::new();
    for path in listing.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Some(name) = path.rsplit('/').next() else { continue };
        let Some(stem) = name.strip_suffix(".md") else { continue };
        if issuefs::parse_key(stem).is_none() {
            continue; // README.md, and anything else that isn't an issue.
        }
        let text = git(repo, &["cat-file", "blob", &format!("{at}:{path}")])?;
        match issuefs::parse_issue_file(stem, &text) {
            Ok(f) => out.push(f),
            Err(e) => log::warn!("issue {stem} in {at} skipped: {e}"),
        }
    }
    Ok(out)
}

/// Fetch the other side into [`REMOTE_REF`]. `Ok(false)` means the remote has
/// no tracker yet, which is the normal state before anyone has published.
fn fetch(repo: &Path, remote: &str) -> Result<bool> {
    let spec = format!("+{LOCAL_REF}:{REMOTE_REF}");
    match git(repo, &["fetch", "--quiet", remote, &spec]) {
        Ok(_) => Ok(rev(repo, REMOTE_REF).is_some()),
        // A remote that has never been published has no such ref, and git says
        // so by failing. Nothing is wrong and the first push will create it.
        Err(_) => Ok(false),
    }
}

// ---------------------------------------------------------------------------
// The pass

/// Apply a merge plan to the issues directory. Deletes run before writes so a
/// renamed key (delete the old name, write the new one) cannot leave both.
fn apply(repo: &Path, plan: &Plan) -> Result<(usize, usize)> {
    let (mut written, mut deleted) = (0, 0);
    for a in &plan.actions {
        if let Action::Delete(key) = a {
            let path = issuefs::issue_path(repo, key);
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
                deleted += 1;
            }
        }
    }
    for a in &plan.actions {
        if let Action::Write(f) = a {
            issuefs::atomic_write(
                &issuefs::issue_path(repo, &f.key),
                &issuefs::serialize_issue_file(f),
            )?;
            written += 1;
        }
    }
    Ok((written, deleted))
}

/// One sync pass: fetch, merge, apply, commit, push.
///
/// `repo` is the project's own checkout — the one place `.agency/issues/`
/// exists. The caller should run `issuefs::reconcile` afterwards to bring the
/// index into line with the files this changed; that is left outside so this
/// stays a function over a directory and a remote, testable without a registry.
pub fn sync(repo: &Path, remote: &str, mode: Mode) -> Result<Outcome> {
    let have_remote = fetch(repo, remote)?;
    let local_files = issuefs::read_issue_dir(repo)?.issues;

    let remote_files = if have_remote { read_tree(repo, REMOTE_REF)? } else { Vec::new() };
    let base_files = match (rev(repo, LOCAL_REF), have_remote) {
        (Some(l), true) => match merge_base(repo, &l, REMOTE_REF) {
            Some(b) => read_tree(repo, &b)?,
            None => Vec::new(),
        },
        // Nothing published from here yet, so whatever the remote has is
        // entirely new to us and there is no shared past to merge against.
        _ => Vec::new(),
    };

    // No common history and issues on both sides is the state a first sync of
    // two already-populated machines lands in, and merging it would report
    // every issue as a contested key. Refusing and asking which side seeds is
    // the honest move; `Publish`/`Adopt` are how the caller answers.
    if mode == Mode::Merge && base_files.is_empty() && !local_files.is_empty() && have_remote {
        return Err(
            Blocked::NeedsSeeding { local: local_files.len(), remote: remote_files.len() }.into()
        );
    }

    let plan = issuesync::plan(mode, &base_files, &local_files, &remote_files);
    let (written, deleted) = apply(repo, &plan)?;

    let tree = write_tree(repo)?;
    let mut parents: Vec<String> = Vec::new();
    if let Some(l) = rev(repo, LOCAL_REF) {
        parents.push(l);
    }
    if let Some(r) = rev(repo, REMOTE_REF) {
        if !parents.contains(&r) && merge_base(repo, &r, LOCAL_REF).as_deref() != Some(r.as_str()) {
            parents.push(r);
        }
    }
    let message = match mode {
        Mode::Merge => "sync issues",
        Mode::Publish => "publish issues",
        Mode::Adopt => "adopt issues",
    };
    let committed = commit_tree(repo, &tree, &parents, message)?.is_some();

    let pushed = if rev(repo, LOCAL_REF).is_some() {
        let spec = format!("{LOCAL_REF}:{LOCAL_REF}");
        match git(repo, &["push", "--quiet", remote, &spec]) {
            Ok(_) => true,
            Err(e) => {
                log::warn!("issue sync push failed: {e}");
                false
            }
        }
    } else {
        false
    };

    Ok(Outcome {
        written,
        deleted,
        conflicts: plan.conflicts,
        skipped: plan.skipped,
        committed,
        pushed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// A project checkout with an issues dir, excluded exactly as the app
    /// excludes it — the state `write_tree` has to punch through with `--force`.
    fn repo(dir: &Path) -> PathBuf {
        let repo = dir.to_path_buf();
        std::fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.email", "t@e.com"]);
        sh(&repo, &["config", "user.name", "T"]);
        std::fs::write(repo.join("code.rs"), "fn main() {}\n").unwrap();
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-q", "-m", "init"]);
        crate::worktree::ensure_agency_excludes(&repo).unwrap();
        std::fs::create_dir_all(repo.join(issuefs::ISSUES_DIR)).unwrap();
        repo
    }

    fn put(repo: &Path, key: &str, uid: &str, status: &str, title: &str) {
        std::fs::write(
            issuefs::issue_path(repo, key),
            format!("---\nkey: {key}\nuid: {uid}\nstatus: {status}\nupdated: 2026-09-01T00:00:00Z\n---\n# {title}\n"),
        )
        .unwrap();
    }

    fn keys(repo: &Path) -> Vec<String> {
        let mut k: Vec<String> =
            issuefs::read_issue_dir(repo).unwrap().issues.into_iter().map(|f| f.key).collect();
        k.sort();
        k
    }

    const U1: &str = "11111111-1111-4111-8111-111111111111";
    const U2: &str = "22222222-2222-4222-8222-222222222222";
    const U3: &str = "33333333-3333-4333-8333-333333333333";

    #[test]
    fn snapshotting_leaves_the_index_and_working_tree_untouched() {
        // The property the whole design rests on: taking a snapshot must not
        // dirty the checkout, because a dirty checkout is what merges refuse to
        // start on.
        let dir = tempdir().unwrap();
        let repo = repo(dir.path());
        put(&repo, "AGE-1", U1, "todo", "One");

        let before = sh(&repo, &["status", "--porcelain"]);
        let tree = write_tree(&repo).unwrap();
        let after = sh(&repo, &["status", "--porcelain"]);

        assert_eq!(before, after, "snapshot dirtied the checkout");
        assert!(before.trim().is_empty(), "fixture was already dirty: {before}");
        assert!(sh(&repo, &["ls-tree", "-r", "--name-only", &tree]).contains("AGE-1.md"));
        // And no scratch index was left behind in .git.
        let leftovers: Vec<_> = std::fs::read_dir(repo.join(".git"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("agency-issues-index"))
            .collect();
        assert!(leftovers.is_empty(), "scratch index left behind: {leftovers:?}");
    }

    #[test]
    fn a_tree_round_trips_through_read_tree() {
        let dir = tempdir().unwrap();
        let repo = repo(dir.path());
        put(&repo, "AGE-1", U1, "in_progress", "One");
        put(&repo, "AGE-2", U2, "todo", "Two");
        // The README lives beside the issues and must not be read back as one.
        issuefs::ensure_readme(&repo).unwrap();

        let tree = write_tree(&repo).unwrap();
        let commit = commit_tree(&repo, &tree, &[], "snap").unwrap().unwrap();
        let back = read_tree(&repo, &commit).unwrap();

        let mut got: Vec<_> = back.iter().map(|f| f.key.clone()).collect();
        got.sort();
        assert_eq!(got, vec!["AGE-1", "AGE-2"], "README or assets leaked in as an issue");
        assert_eq!(back.iter().find(|f| f.key == "AGE-1").unwrap().uid.as_deref(), Some(U1));
    }

    #[test]
    fn an_unchanged_backlog_does_not_commit_again() {
        let dir = tempdir().unwrap();
        let repo = repo(dir.path());
        put(&repo, "AGE-1", U1, "todo", "One");
        let tree = write_tree(&repo).unwrap();
        assert!(commit_tree(&repo, &tree, &[], "first").unwrap().is_some());
        assert!(
            commit_tree(&repo, &tree, &[], "again").unwrap().is_none(),
            "an unchanged tracker produced a second commit"
        );
    }

    /// Two checkouts of one project, syncing through a bare remote — the real
    /// shape of "the same person on two machines".
    fn two_machines() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
        let dir = tempdir().unwrap();
        let bare = dir.path().join("origin.git");
        std::fs::create_dir_all(&bare).unwrap();
        sh(&bare, &["init", "-q", "--bare"]);
        let a = repo(&dir.path().join("a"));
        let b = repo(&dir.path().join("b"));
        let url = bare.to_string_lossy().to_string();
        (dir, a, b, url)
    }

    #[test]
    fn publish_then_adopt_seeds_a_second_machine() {
        let (_d, a, b, url) = two_machines();
        put(&a, "AGE-1", U1, "in_progress", "One");
        put(&a, "AGE-2", U2, "todo", "Two");

        let out = sync(&a, &url, Mode::Publish).unwrap();
        assert!(out.committed && out.pushed, "publish did not reach the remote: {out:?}");

        let out = sync(&b, &url, Mode::Adopt).unwrap();
        assert_eq!(out.written, 2);
        assert_eq!(keys(&b), vec!["AGE-1", "AGE-2"]);
        let one = issuefs::read_issue_dir(&b).unwrap().issues;
        let one = one.iter().find(|f| f.key == "AGE-1").unwrap();
        assert_eq!(one.uid.as_deref(), Some(U1), "identity did not survive the transport");
        assert_eq!(one.status, crate::registry::IssueStatus::InProgress);
    }

    #[test]
    fn edits_on_both_machines_meet_in_the_middle() {
        let (_d, a, b, url) = two_machines();
        put(&a, "AGE-1", U1, "todo", "One");
        sync(&a, &url, Mode::Publish).unwrap();
        sync(&b, &url, Mode::Adopt).unwrap();

        // Each side files something the other has never seen.
        put(&a, "AGE-2", U2, "todo", "From A");
        put(&b, "AGE-3", U3, "todo", "From B");

        sync(&a, &url, Mode::Merge).unwrap();
        let out = sync(&b, &url, Mode::Merge).unwrap();
        assert!(out.conflicts.is_empty(), "clean merge reported a conflict: {out:?}");
        assert_eq!(keys(&b), vec!["AGE-1", "AGE-2", "AGE-3"]);

        // ...and A picks up B's on the next pass.
        sync(&a, &url, Mode::Merge).unwrap();
        assert_eq!(keys(&a), vec!["AGE-1", "AGE-2", "AGE-3"]);
    }

    #[test]
    fn a_deletion_on_one_machine_reaches_the_other() {
        let (_d, a, b, url) = two_machines();
        put(&a, "AGE-1", U1, "todo", "One");
        put(&a, "AGE-2", U2, "todo", "Two");
        sync(&a, &url, Mode::Publish).unwrap();
        sync(&b, &url, Mode::Adopt).unwrap();

        std::fs::remove_file(issuefs::issue_path(&a, "AGE-2")).unwrap();
        sync(&a, &url, Mode::Merge).unwrap();
        let out = sync(&b, &url, Mode::Merge).unwrap();

        assert_eq!(out.deleted, 1);
        assert_eq!(keys(&b), vec!["AGE-1"]);

        // And it stays deleted: the base says it existed and both sides now
        // agree it does not, so no later pass resurrects it.
        sync(&a, &url, Mode::Merge).unwrap();
        sync(&b, &url, Mode::Merge).unwrap();
        assert_eq!(keys(&b), vec!["AGE-1"]);
        assert_eq!(keys(&a), vec!["AGE-1"]);
    }

    #[test]
    fn two_populated_machines_refuse_to_merge_blind() {
        let (_d, a, b, url) = two_machines();
        put(&a, "AGE-1", U1, "todo", "Mine");
        put(&b, "AGE-1", U2, "todo", "Theirs");
        sync(&a, &url, Mode::Publish).unwrap();

        // B has its own AGE-1 with its own uid and no shared history. Merging
        // would be a guess, so it refuses and says what the choice is.
        let err = sync(&b, &url, Mode::Merge).unwrap_err();
        let blocked = err.downcast_ref::<Blocked>().expect("wrong error type");
        assert_eq!(*blocked, Blocked::NeedsSeeding { local: 1, remote: 1 });
        assert_eq!(keys(&b), vec!["AGE-1"], "a refused sync still changed the tracker");
    }

    #[test]
    fn syncing_never_dirties_either_checkout() {
        let (_d, a, b, url) = two_machines();
        put(&a, "AGE-1", U1, "todo", "One");
        sync(&a, &url, Mode::Publish).unwrap();
        sync(&b, &url, Mode::Adopt).unwrap();
        put(&b, "AGE-2", U2, "todo", "Two");
        sync(&b, &url, Mode::Merge).unwrap();
        sync(&a, &url, Mode::Merge).unwrap();

        for r in [&a, &b] {
            assert!(
                sh(r, &["status", "--porcelain"]).trim().is_empty(),
                "sync dirtied a checkout, which is what merges refuse to start on"
            );
            // And the tracker is on no branch: the ref is reachable, main is not
            // carrying it.
            let on_main = sh(r, &["ls-tree", "-r", "--name-only", "main"]);
            assert!(!on_main.contains(".agency/issues"), "issues leaked onto a branch");
        }
    }

    #[test]
    fn a_first_sync_with_nothing_anywhere_is_a_no_op() {
        let (_d, a, _b, url) = two_machines();
        let out = sync(&a, &url, Mode::Merge).unwrap();
        assert_eq!((out.written, out.deleted), (0, 0));
        assert!(out.conflicts.is_empty());
    }
}
