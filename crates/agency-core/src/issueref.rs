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

use anyhow::{anyhow, bail, Context, Result};

use crate::issuefs::{self, IssueFile};
use crate::issuesync::{self, Action, Mode, Plan};
use crate::setup::{CancelToken, CloneProgress};

/// Where a project's issues live when they are shared. Not under `refs/heads/`
/// or `refs/remotes/`, so it is invisible to branch listings, to `git log
/// --all`, and to a plain `git push`; nothing carries it by accident.
pub const LOCAL_REF: &str = "refs/agency/issues";

/// Where a fetched copy of the other side lands. Kept as a real ref rather than
/// read from `FETCH_HEAD` so `git merge-base` can be asked about it.
pub const REMOTE_REF: &str = "refs/agency/issues-remote";

/// What a sync pass did, for the caller to show.
#[derive(Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// Issue files written locally (created or updated) by the merge.
    pub written: usize,
    /// Issue files deleted locally by the merge.
    pub deleted: usize,
    /// Attachments copied here from the other side, so the relative links in an
    /// arriving issue's body resolve on this machine too.
    pub assets_fetched: usize,
    /// Attachments removed here because the other side removed them.
    pub assets_deleted: usize,
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

impl Outcome {
    /// Fold a retry's pass into the one it followed, so a race that took two
    /// passes to settle is reported as the one sync the user asked for.
    ///
    /// The first pass's writes already landed on disk, so its counts are part
    /// of what happened and are kept; only the second pass can speak for the
    /// push. Without this a retry reported the second pass alone, which is
    /// "Already up to date" for the 186 issues the first pass had just
    /// written.
    ///
    /// Conflicts are deduplicated on `(key, field)`. Most are settled by the
    /// first pass's writes and do not recur, but a contested key (two uids
    /// claiming one number) is reported from disk state that neither pass
    /// changes, so the retry reported it again: "2 conflicts decided for you"
    /// for one, and the row marker carrying the same line twice.
    fn followed_by(self, next: Outcome) -> Outcome {
        let mut conflicts = self.conflicts;
        for c in next.conflicts {
            if !conflicts.iter().any(|p| p.key == c.key && p.field == c.field) {
                conflicts.push(c);
            }
        }
        Outcome {
            written: self.written + next.written,
            deleted: self.deleted + next.deleted,
            assets_fetched: self.assets_fetched + next.assets_fetched,
            assets_deleted: self.assets_deleted + next.assets_deleted,
            conflicts,
            // Not summed: `skipped` is the current state of the directory (a
            // file still carrying no `uid`), not a count of events, and the
            // second read is the later one.
            skipped: next.skipped,
            committed: self.committed || next.committed,
            pushed: next.pushed,
        }
    }
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
// progress

/// One step of a pass, for whatever the caller is drawing a bar with. The
/// phases are the ones a person can act on ("Publishing to origin"), not the
/// plumbing; the fetch and the push additionally stream git's own `--progress`
/// lines straight through, exactly as a clone does.
///
/// `percent` is `None` for the steps with no fraction to report, which sweeps
/// the bar rather than pinning it at zero and calling that information.
fn report(on: &mut dyn FnMut(CloneProgress), phase: &str, percent: Option<u8>, detail: &str) {
    on(CloneProgress { phase: phase.to_string(), percent, detail: detail.to_string() });
}

fn pct(done: usize, total: usize) -> Option<u8> {
    (total > 0).then(|| (done * 100 / total) as u8)
}

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

/// Is `a` reachable from `b`? What tells a commit that would only re-say what
/// the ref already contains from one that would add a parent it lacks.
fn is_ancestor(repo: &Path, a: &str, b: &str) -> bool {
    // `git()` fails on a non-zero exit, which for `--is-ancestor` is the
    // answer "no", so the answer is its success alone.
    git(repo, &["merge-base", "--is-ancestor", a, b]).is_ok()
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
/// Returns `None` only when the commit would say nothing new — the tree
/// already matches the ref's tree *and* every parent is already reachable from
/// it. An unchanged backlog must not produce a commit per sync, or the ref's
/// history becomes noise and every merge base is the previous minute; but the
/// tree is only half of what a commit carries.
///
/// The other half is load-bearing. A pass that merged a remote commit whose
/// content this side already had produces exactly the state the tree test
/// alone gets wrong: same tree, and the remote commit still not an ancestor of
/// the local ref. Skipping the commit there leaves the ref unable to
/// fast-forward the remote, and no later pass repairs it, because every later
/// pass reaches the same tree and skips again. Observed as `! [rejected]
/// refs/agency/issues -> refs/agency/issues (non-fast-forward)` on every push
/// from that moment on, with "Try again" as the only advice the UI could give.
fn commit_tree(
    repo: &Path,
    tree: &str,
    parents: &[String],
    message: &str,
) -> Result<Option<String>> {
    if let Some(head) = rev(repo, LOCAL_REF) {
        let same_tree =
            git_opt(repo, &["rev-parse", &format!("{head}^{{tree}}")]).as_deref() == Some(tree);
        if same_tree && parents.iter().all(|p| is_ancestor(repo, p, &head)) {
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

/// Everything a commit says is in the issues directory: paths relative to
/// `.agency/issues/`, in tree order.
fn list_tree(repo: &Path, at: &str) -> Vec<String> {
    let prefix = format!("{}/", issuefs::ISSUES_DIR);
    git(repo, &["ls-tree", "-r", "--name-only", at, "--", issuefs::ISSUES_DIR])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().strip_prefix(prefix.as_str()).map(str::to_string))
        .filter(|p| !p.is_empty())
        .collect()
}

/// Read many objects out of a repository in one `git cat-file --batch`,
/// answering in the order asked. `None` is git saying it could not resolve that
/// spec, which is an answer and not a failure.
///
/// One process, not one per object, and the difference is not marginal: 600
/// blobs out of one commit measured 15.4s as `git cat-file blob` per file
/// against 0.21s as one batch, on the machine this was written on. A pass reads
/// two trees, so that was half a minute of a frozen window on a backlog of that
/// size, and it was the slowest step of a sync that never touched the network.
///
/// The protocol is `<oid> SP <type> SP <size> LF`, then exactly `size` bytes,
/// then a bare LF. An unresolvable spec answers `<spec> SP missing LF` with no
/// body, so the trailing newline must not be consumed for one.
///
/// stdin is written from its own thread, and that is load-bearing rather than
/// tidy: a git blocked on a full stdout pipe stops draining stdin, so a caller
/// that writes every spec before reading a byte wedges the pair once both
/// buffers fill. Where that lands depends on the kernel's pipe sizing and on
/// how large the issue files are, so there is no honest size to pin it at, and
/// a deadlock does not fail a test suite, it hangs one. The thread costs
/// nothing and takes the question away.
fn cat_file_batch(repo: &Path, specs: Vec<String>) -> Result<Vec<Option<Vec<u8>>>> {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::Stdio;

    if specs.is_empty() {
        return Ok(Vec::new());
    }
    let wanted = specs.len();
    let mut child = Command::new("git")
        .args(["cat-file", "--batch"])
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let writer = std::thread::spawn(move || {
        for spec in &specs {
            // A git that died early closes the pipe; stop rather than spin.
            if writeln!(stdin, "{spec}").is_err() {
                break;
            }
        }
        // Dropping stdin is the EOF that ends the batch.
    });
    let mut errs = child.stderr.take().expect("stderr was piped");
    let drain = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = errs.read_to_string(&mut s);
        s
    });

    let mut out = BufReader::new(child.stdout.take().expect("stdout was piped"));
    let mut blobs: Vec<Option<Vec<u8>>> = Vec::with_capacity(wanted);
    let mut header = String::new();
    let mut short = false;
    for _ in 0..wanted {
        header.clear();
        if out.read_line(&mut header)? == 0 {
            short = true;
            break;
        }
        let line = header.trim_end_matches('\n');
        // A path with a space in it is legal, so the missing case is read off
        // the end of the line rather than by splitting it into fields.
        if line.ends_with(" missing") {
            blobs.push(None);
            continue;
        }
        let size = line
            .rsplit(' ')
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .ok_or_else(|| anyhow!("git cat-file --batch said {line:?}"))?;
        let mut body = vec![0u8; size];
        out.read_exact(&mut body)?;
        let mut nl = [0u8; 1];
        out.read_exact(&mut nl)?;
        blobs.push(Some(body));
    }

    let _ = writer.join();
    let status = child.wait()?;
    let stderr = drain.join().unwrap_or_default();
    if !status.success() || short {
        bail!("git cat-file --batch failed: {}", stderr.trim());
    }
    Ok(blobs)
}

/// Every issue file in a commit, parsed. A file that does not parse is skipped
/// with a warning rather than failing the sync: one corrupt file on the other
/// side must not stop the other two hundred from arriving.
///
/// `phase` names which tree is being read, for the progress bar. There is no
/// fraction to report: the whole tree comes back from a single
/// [`cat_file_batch`], so the step is one wait rather than a countdown.
fn read_tree(
    repo: &Path,
    at: &str,
    phase: &str,
    on_progress: &mut dyn FnMut(CloneProgress),
) -> Result<Vec<IssueFile>> {
    // README.md, and anything else that isn't an issue, drops out here rather
    // than after the read, so nothing is fetched that cannot be parsed.
    let stems: Vec<(String, String)> = list_tree(repo, at)
        .into_iter()
        .filter_map(|rel| {
            let stem = rel.strip_suffix(".md")?.to_string();
            issuefs::parse_key(&stem).is_some().then_some((stem, rel))
        })
        .collect();
    report(on_progress, phase, None, &format!("{} issues", stems.len()));

    let specs = stems.iter().map(|(_, rel)| blob_spec(at, rel)).collect();
    let mut out = Vec::new();
    for ((stem, _), blob) in stems.iter().zip(cat_file_batch(repo, specs)?) {
        let Some(bytes) = blob else {
            log::warn!("issue {stem} in {at} skipped: git could not read it");
            continue;
        };
        match issuefs::parse_issue_file(stem, &String::from_utf8_lossy(&bytes)) {
            Ok(f) => out.push(f),
            Err(e) => log::warn!("issue {stem} in {at} skipped: {e}"),
        }
    }
    Ok(out)
}

/// Attachment paths in a commit, relative to the issues directory
/// (`assets/AGE-14-shot.png`) — the same spelling an issue body links them by,
/// so a plan can be compared against what a body references without rewriting
/// paths.
fn read_tree_assets(repo: &Path, at: &str) -> Vec<String> {
    let prefix = format!("{}/", issuefs::ASSETS_DIR);
    list_tree(repo, at).into_iter().filter(|p| p.starts_with(&prefix)).collect()
}

/// Attachment paths on disk, in the same spelling as [`read_tree_assets`]. One
/// level deep: the app writes attachments flat into `assets/`, and
/// `issuefs::body_attachments` refuses to link anything deeper.
fn disk_assets(repo: &Path) -> Vec<String> {
    let dir = repo.join(issuefs::ISSUES_DIR).join(issuefs::ASSETS_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| format!("{}/{}", issuefs::ASSETS_DIR, e.file_name().to_string_lossy()))
        .collect();
    out.sort();
    out
}

fn blob_spec(at: &str, rel: &str) -> String {
    format!("{at}:{}/{rel}", issuefs::ISSUES_DIR)
}

/// An attachment's bytes. Not the text helper: the lossy UTF-8 conversion there
/// would replace every byte a PNG is made of.
fn read_blob(repo: &Path, at: &str, rel: &str) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .args(["cat-file", "blob", &blob_spec(at, rel)])
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if !out.status.success() {
        bail!("reading {rel} at {at}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

/// Fetch the other side into [`REMOTE_REF`]. `Ok(false)` means the remote has
/// no tracker yet, which is the normal state before anyone has published.
///
/// Streamed rather than `--quiet`: this and the push are the two steps that
/// round-trip the network, so on a slow remote they are nearly the whole wait,
/// and git's own progress lines are the only truthful thing to show during one.
fn fetch(repo: &Path, remote: &str, on_progress: &mut dyn FnMut(CloneProgress)) -> Result<bool> {
    let spec = format!("+{LOCAL_REF}:{REMOTE_REF}");
    let mut cmd = Command::new("git");
    cmd.args(["fetch", "--progress", remote, &spec])
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0");
    // A remote that has never been published has no such ref, and git says so
    // by exiting non-zero. Nothing is wrong and the first push will create it.
    // Note which half is which: a non-zero *exit* is that answer, but a failure
    // to run git at all is not, and propagates rather than reading as an empty
    // remote the way it silently did before this streamed.
    let (ok, _) = crate::setup::run_clone_streaming(cmd, &CancelToken::new(), on_progress)?;
    Ok(ok && rev(repo, REMOTE_REF).is_some())
}

/// How a push attempt came back. `Stale` is the one worth telling apart: the
/// remote moved between this pass's fetch and its push, which one more pass
/// fixes by itself, where every other failure needs a person.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Push {
    Done,
    Stale,
    Failed,
}

/// Does git's refusal mean "you are behind"? Matched across the phrasings git
/// uses for it, since the reject line and the hint word it differently and
/// which of them appears depends on the git version.
///
/// Deliberately narrow. Everything that is not recognised here is treated as a
/// failure that a retry cannot help, which is the safe way round: a needless
/// retry costs a second round-trip, but treating an auth failure as a race
/// would retry it forever.
fn is_stale_push(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("non-fast-forward") || s.contains("fetch first") || s.contains("behind its remote")
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

/// Bring attachments into line with the plan. Runs after the issue files, so an
/// issue is never briefly on disk pointing at bytes that have not arrived.
///
/// An attachment that cannot be written is logged and skipped rather than
/// failing the pass: a broken image link is a much smaller loss than a sync that
/// refuses to finish, and the issue text it belongs to is already here.
fn apply_assets(
    repo: &Path,
    at: &str,
    plan: &issuesync::AssetPlan,
    on_progress: &mut dyn FnMut(CloneProgress),
) -> (usize, usize) {
    let dir = repo.join(issuefs::ISSUES_DIR);
    let (mut fetched, mut removed) = (0, 0);
    let total = plan.fetch.len();
    for (i, rel) in plan.fetch.iter().enumerate() {
        report(on_progress, "Copying attachments", pct(i, total), &format!("{} of {total}", i + 1));
        let dest = dir.join(rel);
        let wrote = read_blob(repo, at, rel).and_then(|bytes| {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, bytes)?;
            Ok(())
        });
        match wrote {
            Ok(()) => fetched += 1,
            Err(e) => log::warn!("attachment {rel} did not arrive: {e}"),
        }
    }
    for rel in &plan.delete {
        let path = dir.join(rel);
        if path.exists() {
            match std::fs::remove_file(&path) {
                Ok(()) => removed += 1,
                Err(e) => log::warn!("removing attachment {rel}: {e}"),
            }
        }
    }
    (fetched, removed)
}

/// One sync pass: fetch, merge, apply, commit, push.
///
/// `repo` is the project's own checkout — the one place `.agency/issues/`
/// exists. The caller should run `issuefs::reconcile` afterwards to bring the
/// index into line with the files this changed; that is left outside so this
/// stays a function over a directory and a remote, testable without a registry.
pub fn sync(repo: &Path, remote: &str, mode: Mode) -> Result<Outcome> {
    sync_with_progress(repo, remote, mode, |_| {})
}

/// [`sync`], reporting each step as it starts.
///
/// A pass runs two network round-trips and one `git cat-file` per issue per
/// tree, so on a real backlog it is seconds, not milliseconds; before this the
/// only feedback was the window going unresponsive, because the command was
/// also running on the main thread. Split the way `git::push_with_progress` is,
/// so the plain [`sync`] stays the shape the tests call.
pub fn sync_with_progress(
    repo: &Path,
    remote: &str,
    mode: Mode,
    mut on_progress: impl FnMut(CloneProgress),
) -> Result<Outcome> {
    sync_streaming(repo, remote, mode, &mut on_progress)
}

/// A pass, plus one retry when the only thing that went wrong was losing a
/// race for the ref.
///
/// Two projects sharing one backlog remote (two clones of a repo on one
/// machine, or two machines with the schedule on) push the same ref, so a pass
/// that fetched before the other side pushed and pushed after it is rejected
/// through no fault of the user. The state that fixes it is one more fetch.
///
/// Merge only: re-running a `Publish` or an `Adopt` would re-apply a seed the
/// user chose once, and a seed is the operation that discards one side. One
/// retry, not a loop — a remote still ahead of a fresh fetch is not racing, it
/// is being written by something faster than we can answer, and the toast is
/// then the right outcome.
fn sync_streaming(
    repo: &Path,
    remote: &str,
    mode: Mode,
    on_progress: &mut dyn FnMut(CloneProgress),
) -> Result<Outcome> {
    let (out, push) = one_pass(repo, remote, mode, on_progress)?;
    if push != Push::Stale || mode != Mode::Merge {
        return Ok(out);
    }
    log::info!("issue sync: remote moved under this pass, merging again");
    let (again, _) = one_pass(repo, remote, Mode::Merge, on_progress)?;
    Ok(out.followed_by(again))
}

fn one_pass(
    repo: &Path,
    remote: &str,
    mode: Mode,
    on_progress: &mut dyn FnMut(CloneProgress),
) -> Result<(Outcome, Push)> {
    report(on_progress, &format!("Fetching from {remote}"), None, "");
    let have_remote = fetch(repo, remote, on_progress)?;
    let local_files = issuefs::read_issue_dir(repo)?.issues;

    let remote_files = if have_remote {
        read_tree(repo, REMOTE_REF, "Reading the shared backlog", on_progress)?
    } else {
        Vec::new()
    };
    // Nothing published from here yet means whatever the remote has is entirely
    // new to us, and there is no shared past to merge against.
    let base_ref = match (rev(repo, LOCAL_REF), have_remote) {
        (Some(l), true) => merge_base(repo, &l, REMOTE_REF),
        _ => None,
    };
    let base_files = match &base_ref {
        Some(b) => read_tree(repo, b, "Reading the last synced copy", on_progress)?,
        None => Vec::new(),
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

    // Adopting replaces this machine's tracker wholesale, which is the one
    // operation here that destroys the user's issues on their say-so. Commit
    // what is about to be replaced onto the local ref first: the ref is then
    // the backup, recoverable with `git show`, and it costs one commit that
    // would have happened on the next sync anyway.
    if mode == Mode::Adopt && !local_files.is_empty() {
        let before = write_tree(repo)?;
        let parents: Vec<String> = rev(repo, LOCAL_REF).into_iter().collect();
        commit_tree(repo, &before, &parents, "issues before adopting the shared tracker")?;
    }

    report(on_progress, "Merging", None, "");
    let plan = issuesync::plan(mode, &base_files, &local_files, &remote_files);
    let (written, deleted) = apply(repo, &plan)?;

    let asset_plan = issuesync::plan_assets(
        mode,
        &base_ref.as_deref().map(|b| read_tree_assets(repo, b)).unwrap_or_default(),
        &disk_assets(repo),
        &if have_remote { read_tree_assets(repo, REMOTE_REF) } else { Vec::new() },
    );
    let (assets_fetched, assets_deleted) = apply_assets(repo, REMOTE_REF, &asset_plan, on_progress);

    report(on_progress, "Recording the merged backlog", None, "");
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

    let push = if rev(repo, LOCAL_REF).is_some() {
        report(on_progress, &format!("Publishing to {remote}"), None, "");
        let spec = format!("{LOCAL_REF}:{LOCAL_REF}");
        let mut cmd = Command::new("git");
        cmd.args(["push", "--progress", remote, &spec])
            .current_dir(repo)
            .env("GIT_TERMINAL_PROMPT", "0");
        match crate::setup::run_clone_streaming(cmd, &CancelToken::new(), on_progress) {
            Ok((true, _)) => Push::Done,
            // A push that git refused says why on stderr, and that line is the
            // whole diagnosis when the shared copy silently stops updating.
            Ok((false, stderr)) => {
                log::warn!("issue sync push failed: {}", stderr.trim());
                if is_stale_push(&stderr) {
                    Push::Stale
                } else {
                    Push::Failed
                }
            }
            Err(e) => {
                log::warn!("issue sync push failed: {e}");
                Push::Failed
            }
        }
    } else {
        Push::Failed
    };

    Ok((
        Outcome {
            written,
            deleted,
            assets_fetched,
            assets_deleted,
            conflicts: plan.conflicts,
            skipped: plan.skipped,
            committed,
            pushed: push == Push::Done,
        },
        push,
    ))
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
        let back = read_tree(&repo, &commit, "reading", &mut |_| {}).unwrap();

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

    /// The tree test alone is not enough to call a commit redundant: a commit
    /// also carries parents, and a merge whose result happens to match what we
    /// already had is exactly where those two disagree.
    #[test]
    fn an_unchanged_tree_still_commits_to_take_on_a_new_parent() {
        let dir = tempdir().unwrap();
        let repo = repo(dir.path());
        put(&repo, "AGE-1", U1, "todo", "One");
        let tree = write_tree(&repo).unwrap();
        let head = commit_tree(&repo, &tree, &[], "first").unwrap().unwrap();

        // A commit with the same tree that the ref does not descend from — the
        // shape of the other side having reached our content by its own route.
        let other = sh(&repo, &["commit-tree", &tree, "-m", "elsewhere"]).trim().to_string();

        assert!(
            commit_tree(&repo, &tree, std::slice::from_ref(&head), "again").unwrap().is_none(),
            "a commit that adds nothing was still written"
        );
        let merged = commit_tree(&repo, &tree, &[head, other.clone()], "merge").unwrap();
        assert!(merged.is_some(), "the ref never took on the parent it has to push past");
        assert!(
            is_ancestor(&repo, &other, LOCAL_REF),
            "the new parent is not reachable from the ref"
        );
    }

    /// A contested key is re-derived from disk on every pass, and a retry is
    /// two passes over the same disk.
    #[test]
    fn a_retry_does_not_report_the_same_conflict_twice() {
        let conflict = |key: &str, field: &str| crate::issuesync::Conflict {
            key: key.into(),
            field: field.into(),
            detail: "two different issues both claim it".into(),
        };
        let first = Outcome {
            written: 2,
            conflicts: vec![conflict("AGE-3", "key"), conflict("AGE-4", "title")],
            ..Default::default()
        };
        let second = Outcome {
            conflicts: vec![conflict("AGE-3", "key"), conflict("AGE-3", "body")],
            pushed: true,
            ..Default::default()
        };
        let out = first.followed_by(second);
        assert_eq!(out.written, 2);
        assert!(out.pushed);
        let seen: Vec<(&str, &str)> =
            out.conflicts.iter().map(|c| (c.key.as_str(), c.field.as_str())).collect();
        assert_eq!(seen, vec![("AGE-3", "key"), ("AGE-4", "title"), ("AGE-3", "body")]);
    }

    #[test]
    fn gits_ways_of_saying_you_are_behind_are_all_read_as_a_race() {
        for s in [
            " ! [rejected]        refs/agency/issues -> refs/agency/issues (non-fast-forward)",
            "! [rejected] main -> main (fetch first)",
            "hint: Updates were rejected because a pushed branch tip is behind its remote",
        ] {
            assert!(is_stale_push(s), "not read as a race: {s}");
        }
        for s in [
            "fatal: unable to access 'https://example.com/': Could not resolve host: example.com",
            "remote: Permission to x/y.git denied to z.",
        ] {
            assert!(!is_stale_push(s), "read as a race, and would retry forever: {s}");
        }
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

    /// Install a `pre-receive` on the bare remote. `once` rejects only the
    /// first push; otherwise every push is rejected, in git's own words for
    /// being behind.
    fn reject_pushes(url: &str, once: bool) -> PathBuf {
        let hooks = Path::new(url).join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let marker = hooks.join("rejected");
        let guard = if once {
            format!("if [ -e '{}' ]; then exit 0; fi\n", marker.display())
        } else {
            String::new()
        };
        let path = hooks.join("pre-receive");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n{guard}touch '{}'\necho 'rejected (non-fast-forward)' >&2\nexit 1\n",
                marker.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        marker
    }

    fn allow_pushes(url: &str, marker: PathBuf) {
        std::fs::remove_file(Path::new(url).join("hooks/pre-receive")).unwrap();
        let _ = std::fs::remove_file(marker);
    }

    /// The bug that made one lost race permanent. Both machines reach the same
    /// content by their own route, so the merge has nothing to write — but the
    /// ref still has to take on the other side's commit, or it can never fast
    /// forward the remote again. Nothing about that state decays, so before the
    /// parent check in `commit_tree` every later pass reached the same tree,
    /// skipped the same commit, and was rejected the same way.
    #[test]
    fn a_merge_that_changes_nothing_can_still_push() {
        let (_d, a, b, url) = two_machines();
        put(&a, "AGE-1", U1, "todo", "One");
        assert!(sync(&a, &url, Mode::Merge).unwrap().pushed);
        // b joins, so the two now share history and later passes are merges.
        assert!(sync(&b, &url, Mode::Merge).unwrap().pushed);

        // a commits its own AGE-2 but cannot publish it: offline, or a remote
        // that was briefly refusing. Its ref moves; the shared copy does not.
        let marker = reject_pushes(&url, false);
        put(&a, "AGE-2", U2, "todo", "Two");
        assert!(!sync(&a, &url, Mode::Merge).unwrap().pushed);
        allow_pushes(&url, marker);

        // b files the same issue, byte for byte — same uid, same fields — and
        // gets there first. Two people acting on one instruction, or the same
        // issue arriving through a third checkout.
        put(&b, "AGE-2", U2, "todo", "Two");
        assert!(sync(&b, &url, Mode::Merge).unwrap().pushed);

        let out = sync(&a, &url, Mode::Merge).unwrap();
        assert_eq!(
            out.written, 0,
            "the two sides already agreed; nothing should have been written"
        );
        assert!(out.pushed, "the shared copy was left behind by a merge that agreed with it");
        let mine = sh(&a, &["rev-parse", LOCAL_REF]).trim().to_string();
        let theirs = sh(&a, &["ls-remote", &url, LOCAL_REF]);
        assert_eq!(mine, theirs.split_whitespace().next().unwrap(), "the remote did not take it");
    }

    /// A push that loses the ref to someone faster is not a failure to report,
    /// it is a fetch that has not happened yet. Two projects sharing one backlog
    /// remote do exactly this to each other on the schedule.
    #[test]
    fn a_push_that_lost_a_race_is_retried_rather_than_reported() {
        let (_d, a, _b, url) = two_machines();
        let marker = reject_pushes(&url, true);

        put(&a, "AGE-1", U1, "todo", "One");
        let out = sync(&a, &url, Mode::Merge).unwrap();
        assert!(marker.exists(), "the hook never ran, so nothing was rejected to retry");
        assert!(out.pushed, "a lost race was reported to the user as a failed sync");
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

    /// A backlog large enough that the responses run to hundreds of KB, so the
    /// batch is exercised well past git's stdout pipe buffer rather than as one
    /// tidy read, together with a spec git cannot resolve.
    ///
    /// The missing case is the framing trap: it answers `<spec> missing` with
    /// *no body and no trailing newline*, so consuming one there reads every
    /// response after it out of frame. The assert that catches that is the one
    /// on the last blob, not the one on the missing blob.
    ///
    /// It does not pin the deadlock the writer thread exists for. That needs
    /// both pipes full at once and this size does not reach it; see
    /// `cat_file_batch` for why there is no size that reliably would.
    #[test]
    fn a_large_batch_read_survives_a_missing_object() {
        let d = tempdir().unwrap();
        let repo = repo(&d.path().join("a"));
        const N: usize = 600;
        let filler = "lorem ipsum dolor sit amet consectetur. ".repeat(10);
        for n in 1..=N {
            let key = format!("AGE-{n}");
            let uid = uuid::Uuid::new_v4();
            std::fs::write(
                issuefs::issue_path(&repo, &key),
                format!(
                    "---\nkey: {key}\nuid: {uid}\nstatus: todo\nupdated: \
                     2026-09-01T00:00:00Z\n---\n# Issue {n}\n\n{filler}\n"
                ),
            )
            .unwrap();
        }
        let tree = write_tree(&repo).unwrap();
        let commit = commit_tree(&repo, &tree, &[], "many").unwrap().unwrap();

        let files = read_tree(&repo, &commit, "reading", &mut |_| {}).unwrap();
        assert_eq!(files.len(), N, "the batch lost issues");

        let specs = vec![
            blob_spec(&commit, "AGE-1.md"),
            blob_spec(&commit, "AGE-9999.md"), // never written
            blob_spec(&commit, &format!("AGE-{N}.md")),
        ];
        let got = cat_file_batch(&repo, specs).unwrap();
        assert!(got[1].is_none(), "an unresolvable spec should answer None");
        let last = String::from_utf8_lossy(got[2].as_ref().unwrap()).to_string();
        assert!(
            last.contains(&format!("# Issue {N}")),
            "the response after a missing one is out of frame"
        );
    }

    /// A sync used to run on the main thread and report nothing, so the whole
    /// window stopped answering for as long as the remote took. The fix is only
    /// half done if the pass is silent: pin that every step a person waits on
    /// names itself, and that the two network steps name the remote they are
    /// talking to rather than saying "the shared copy" at someone trying to
    /// work out where their issues went.
    #[test]
    fn a_pass_names_every_step_it_waits_on() {
        let (_d, a, b, url) = two_machines();
        put(&a, "AGE-1", U1, "todo", "One");
        sync(&a, &url, Mode::Publish).unwrap();
        sync(&b, &url, Mode::Adopt).unwrap();
        put(&b, "AGE-2", U2, "todo", "Two");

        let mut phases: Vec<String> = Vec::new();
        sync_with_progress(&b, &url, Mode::Merge, |p| phases.push(p.phase)).unwrap();

        let has = |needle: &str| phases.iter().any(|p| p.contains(needle));
        assert!(has(&format!("Fetching from {url}")), "no fetch phase in {phases:?}");
        assert!(has("Reading the shared backlog"), "no remote-read phase in {phases:?}");
        assert!(has("Reading the last synced copy"), "no base-read phase in {phases:?}");
        assert!(has("Merging"), "no merge phase in {phases:?}");
        assert!(has(&format!("Publishing to {url}")), "no push phase in {phases:?}");
    }

    /// The state a second machine is in the moment it clones a shared project:
    /// nothing local, the whole backlog on the ref. `Merge` has to bring it
    /// down rather than ask which side seeds, and it does, because the
    /// NeedsSeeding guard turns on `!local_files.is_empty()` for exactly this
    /// case. What was missing was any way to call it: the board's Sync button
    /// lived inside `issues.length > 0`, so the one screen that needed it was
    /// the one screen that did not show it (AGE-177).
    #[test]
    fn an_empty_machine_merges_the_shared_backlog_down() {
        let (_d, a, b, url) = two_machines();
        put(&a, "AGE-1", U1, "todo", "One");
        put(&a, "AGE-2", U2, "in_progress", "Two");
        sync(&a, &url, Mode::Publish).unwrap();

        let out = sync(&b, &url, Mode::Merge).unwrap();
        assert_eq!(out.written, 2, "a fresh machine did not receive the backlog: {out:?}");
        assert!(out.conflicts.is_empty(), "a one-sided merge reported a conflict: {out:?}");
        assert_eq!(keys(&b), vec!["AGE-1", "AGE-2"]);

        // And the two sides now agree: B's pass left a descendant of A's commit
        // rather than a divergent copy, so A's next pass has nothing to do.
        let out = sync(&a, &url, Mode::Merge).unwrap();
        assert_eq!((out.written, out.deleted), (0, 0), "the round trip was not settled: {out:?}");
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

    /// Bytes, not text: the point of reading attachments as blobs is that a PNG
    /// survives, and a lossy UTF-8 round trip would replace half of one.
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0xff, 0x00, 0xfe];

    fn attach(repo: &Path, name: &str, bytes: &[u8]) {
        let dir = repo.join(issuefs::ISSUES_DIR).join(issuefs::ASSETS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
    }

    fn asset(repo: &Path, name: &str) -> Option<Vec<u8>> {
        std::fs::read(repo.join(issuefs::ISSUES_DIR).join(issuefs::ASSETS_DIR).join(name)).ok()
    }

    #[test]
    fn an_attachment_travels_with_the_issue_that_links_it() {
        let (_d, a, b, url) = two_machines();
        put(&a, "AGE-1", U1, "todo", "One");
        attach(&a, "AGE-1-shot.png", PNG);

        sync(&a, &url, Mode::Publish).unwrap();
        let out = sync(&b, &url, Mode::Adopt).unwrap();

        assert_eq!(out.assets_fetched, 1, "the issue arrived without its attachment");
        assert_eq!(
            asset(&b, "AGE-1-shot.png").as_deref(),
            Some(PNG),
            "attachment bytes did not survive the transport"
        );

        // A second attachment added later reaches the other side by merge, not
        // only by seeding.
        attach(&b, "AGE-1-log.txt", b"hello");
        sync(&b, &url, Mode::Merge).unwrap();
        let out = sync(&a, &url, Mode::Merge).unwrap();
        assert_eq!(out.assets_fetched, 1);
        assert_eq!(asset(&a, "AGE-1-log.txt").as_deref(), Some(&b"hello"[..]));

        // Nothing to do on a third pass: an attachment already here is not
        // re-fetched, because the bytes at a path never change.
        let out = sync(&a, &url, Mode::Merge).unwrap();
        assert_eq!((out.assets_fetched, out.assets_deleted), (0, 0));
    }

    #[test]
    fn adopting_leaves_the_replaced_tracker_recoverable() {
        let (_d, a, b, url) = two_machines();
        put(&a, "AGE-1", U1, "todo", "From A");
        sync(&a, &url, Mode::Publish).unwrap();

        // B has its own work that adopting is about to throw away.
        put(&b, "AGE-9", U3, "todo", "Only on B");
        sync(&b, &url, Mode::Adopt).unwrap();
        assert_eq!(keys(&b), vec!["AGE-1"], "adopt did not replace the tracker");

        // The ref carries what was replaced, so it is not actually gone.
        let history = sh(&b, &["log", "--format=%s", LOCAL_REF]);
        assert!(history.contains("before adopting"), "no recovery point was committed: {history}");
        let backup = sh(&b, &["rev-parse", &format!("{LOCAL_REF}^")]).trim().to_string();
        let restored = read_tree(&b, &backup, "reading", &mut |_| {}).unwrap();
        assert!(
            restored.iter().any(|f| f.key == "AGE-9"),
            "the replaced issues are not in the backup commit"
        );
    }

    #[test]
    fn a_first_sync_with_nothing_anywhere_is_a_no_op() {
        let (_d, a, _b, url) = two_machines();
        let out = sync(&a, &url, Mode::Merge).unwrap();
        assert_eq!((out.written, out.deleted), (0, 0));
        assert!(out.conflicts.is_empty());
    }
}
