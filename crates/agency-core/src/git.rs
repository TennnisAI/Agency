use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    /// Staged (index) status code: one of M A D R C ? ! or space.
    pub index: String,
    /// Unstaged (worktree) status code.
    pub worktree: String,
}

/// Run a git command in `worktree`, returning stdout on success.
fn git(worktree: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree)
        // No TTY in the app: a network command must fail fast rather than block
        // forever on a credential prompt no one can answer. When Agency is
        // launched from a terminal it *does* inherit that terminal, so without
        // this a fetch for an unauthenticated remote would sit on a hidden
        // password prompt and never return.
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if !output.status.success() {
        bail!("git {:?} failed: {}", args, String::from_utf8_lossy(&output.stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// `git`, but handing back stdout unchanged. Blob contents are not text: the
/// lossy UTF-8 conversion above would replace every byte an image is made of.
fn git_bytes(worktree: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if !output.status.success() {
        bail!("git {:?} failed: {}", args, String::from_utf8_lossy(&output.stderr));
    }
    Ok(output.stdout)
}

/// The name git would sign a commit here with, if it has one. Used to sign
/// issue comments: whoever the repo says you are is who you are on its issues.
/// A directory git doesn't manage (the git-less workspace) has none.
pub fn user_name(repo: &Path) -> Option<String> {
    let name = git(repo, &["config", "user.name"]).ok()?.trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// How many untracked files a folder may hold and still be listed file by file.
/// Above it the folder stays one row: `git status -uall` on a 40,000-file
/// unignored `node_modules` took 0.27s and produced 1.4 MB of output against
/// 0.01s and 17 bytes collapsed, and the panel re-reads status every 2s.
const UNTRACKED_DIR_CAP: usize = 500;

pub fn status(worktree: &Path) -> Result<Vec<FileChange>> {
    // `-z` gives NUL-separated, unquoted paths: without it git C-quotes any
    // path with spaces/unicode (core.quotePath default) and every downstream
    // per-file operation fails with "pathspec did not match".
    let out = git(worktree, &["status", "--porcelain", "-z"])?;
    let changes = parse_porcelain_z(&out);
    let dirs: Vec<String> = changes
        .iter()
        .filter(|c| is_untracked_dir(c) && folder_within_cap(&worktree.join(&c.path)))
        .map(|c| c.path.clone())
        .collect();
    let listing = list_untracked_files(worktree, &dirs);
    Ok(expand_untracked_dirs(changes, |dir| listing.get(dir).cloned()))
}

/// A folder git has never seen, reported as one entry with a trailing slash.
fn is_untracked_dir(change: &FileChange) -> bool {
    change.index == "?" && change.worktree == "?" && change.path.ends_with('/')
}

/// Replace each collapsed untracked-folder row with the files inside it.
///
/// `git status --porcelain` reports a folder git has never seen as one entry:
/// `sub/a.txt` and `sub/b.txt` arrive as a single `sub/` row. That row can't be
/// diffed (there is no one file to read), so its files could only be staged
/// sight unseen, which is exactly the "stage without reading" the untracked
/// diff work set out to remove.
///
/// `list` returns nothing for a folder over the cap, and for one git can't see
/// into: a nested repository lists as itself, and `-uall` reports it as a
/// single entry too. Those keep the collapsed row.
fn expand_untracked_dirs(
    changes: Vec<FileChange>,
    mut list: impl FnMut(&str) -> Option<Vec<String>>,
) -> Vec<FileChange> {
    let mut out = Vec::with_capacity(changes.len());
    for change in changes {
        let files =
            is_untracked_dir(&change).then(|| list(&change.path)).flatten().unwrap_or_default();
        if files.is_empty() {
            out.push(change);
            continue;
        }
        out.extend(files.into_iter().map(|path| FileChange {
            path,
            index: "?".to_string(),
            worktree: "?".to_string(),
        }));
    }
    out
}

/// The untracked files under each of `dirs`, keyed by the folder they came from.
///
/// `--exclude-standard` is what makes this safe to show: it applies the same
/// ignore rules `status` does, so an unignored build directory nested inside a
/// brand-new folder doesn't come back as a hundred rows. One invocation covers
/// every folder because this runs on every two-second poll of the panel, and a
/// git process costs more than the listing it does.
fn list_untracked_files(worktree: &Path, dirs: &[String]) -> HashMap<String, Vec<String>> {
    let mut listing: HashMap<String, Vec<String>> = HashMap::new();
    if dirs.is_empty() {
        return listing;
    }
    let mut args = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
    args.extend(dirs.iter().map(String::as_str));
    let Ok(out) = git(worktree, &args) else {
        return listing;
    };
    for path in out.split('\0').filter(|p| !p.is_empty()) {
        // A path equal to the folder itself is git saying it cannot see inside:
        // that's a nested repository, and it keeps its single row.
        let Some(dir) = dirs.iter().find(|d| path.starts_with(d.as_str()) && path != *d) else {
            continue;
        };
        listing.entry(dir.clone()).or_default().push(path.to_string());
    }
    listing
}

/// Whether `dir` holds at most `UNTRACKED_DIR_CAP` entries, counted straight
/// off the filesystem and abandoned at the first one past it.
///
/// The obvious cap does not work: `git ls-files` builds its whole result in
/// memory before writing a byte, so streaming its output and killing the walk
/// once it goes over the cap saves nothing. Measured against a 40,000-file
/// unignored `node_modules`, status that way took 390ms, worse than the 270ms
/// of the plain `-uall` this cap exists to avoid. A read_dir walk that stops at
/// entry 501 costs the same for any size of folder, so git is only ever asked
/// about folders it can answer for cheaply.
///
/// It counts ignored files too, which git would not, so a folder can stay
/// collapsed on a count git would call smaller. That is the safe direction: the
/// reason to collapse is how much disk the listing has to touch.
fn folder_within_cap(dir: &Path) -> bool {
    let mut seen = 0usize;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            return false;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > UNTRACKED_DIR_CAP {
                return false;
            }
            // file_type() off the dir entry does not follow symlinks; a link
            // pointing back at an ancestor would otherwise walk forever.
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                pending.push(entry.path());
            }
        }
    }
    true
}

fn parse_porcelain_z(out: &str) -> Vec<FileChange> {
    let mut changes = Vec::new();
    let mut fields = out.split('\0');
    while let Some(entry) = fields.next() {
        // "XY path" — XY status, one space, then the path (no quoting in -z).
        if entry.len() < 4 {
            continue;
        }
        let index = entry[0..1].to_string();
        let work = entry[1..2].to_string();
        let path = entry[3..].to_string();
        // Renames/copies carry the original path as an extra NUL-separated
        // field after the new path; consume and drop it.
        if index == "R" || index == "C" || work == "R" || work == "C" {
            let _ = fields.next();
        }
        changes.push(FileChange { path, index, worktree: work });
    }
    changes
}

pub fn diff(worktree: &Path, path: &str, staged: bool) -> Result<String> {
    if staged {
        return git(worktree, &["diff", "--cached", "--", path]);
    }
    let raw = git(worktree, &["diff", "--", path])?;
    if raw.is_empty() {
        return untracked_diff(worktree, path);
    }
    Ok(raw)
}

/// The whole of an untracked file, as additions. `git diff` says nothing at all
/// about a path git does not track yet, so every file under "Untracked Changes"
/// read as "no textual changes" in the diff pane and its contents were never
/// shown. Comparing against /dev/null outside the index produces exactly the
/// diff git gives once the file is staged, `new file mode` header and all, so
/// the hunk and line staging buttons still build patches that apply.
///
/// Empty unless `path` is an untracked regular file. A tracked-but-unchanged
/// path is not our business, and a symlink would diff as whatever it points at,
/// so applying that patch would replace the link with a copy of its target. An
/// oversized file, and the folder rows `expand_untracked_dirs` could not open,
/// come back as a header alone, which the diff pane reads as its reason for
/// showing no lines.
fn untracked_diff(worktree: &Path, path: &str) -> Result<String> {
    // Every other diff here is scoped to the repo by git itself; `--no-index`
    // is not, and would read "../../etc/passwd" as happily as a file in the
    // worktree. Normalize first and refuse anything that escapes.
    crate::files::resolve_within(worktree, path)?;
    let Ok(meta) = std::fs::symlink_metadata(worktree.join(path)) else {
        return Ok(String::new());
    };
    if !git(worktree, &["ls-files", "-z", "--", path])?.is_empty() {
        return Ok(String::new());
    }
    // A folder still shown as one row is one `expand_untracked_dirs` left
    // collapsed. `--no-index` errors on a directory, and before this the pane
    // answered "no textual changes", which reads as "this folder is empty".
    //
    // Only those two folders have something to explain, and the reason has to be
    // the true one: a blanket "more than 500 files" was returned for any folder
    // asked about, including a folder holding a single file, which the panel
    // lists file by file and gives no row of its own at all.
    if meta.is_dir() {
        let dir = worktree.join(path);
        let why = if dir.join(".git").exists() {
            "Untracked folder holding its own git repository".to_string()
        } else if !folder_within_cap(&dir) {
            format!("Untracked folder with more than {UNTRACKED_DIR_CAP} files")
        } else {
            return Ok(String::new());
        };
        return Ok(format!("diff --git a/{path} b/{path}\n{why}\n"));
    }
    if !meta.is_file() {
        return Ok(String::new());
    }
    // A tracked file's diff stays small however big the file is; an untracked
    // one is the whole file, so the same 2 MB the editor refuses to open would
    // arrive here as a single IPC string and tens of thousands of DOM rows.
    if meta.len() > crate::files::MAX_FILE_BYTES {
        let size = meta.len();
        return Ok(format!("diff --git a/{path} b/{path}\nFile too large to diff: {size} bytes\n"));
    }
    // `--no-index` exits 1 when the two sides differ, which here is every time,
    // so this cannot go through `git()` — that treats any non-zero exit as a
    // failure and would bail on the diff we asked for.
    let output = Command::new("git")
        .args(["diff", "--no-index", "--", "/dev/null", path])
        .current_dir(worktree)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    match output.status.code() {
        Some(0 | 1) => Ok(String::from_utf8_lossy(&output.stdout).to_string()),
        _ => bail!(
            "git diff --no-index -- /dev/null {path} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    }
}

/// Which pair of revisions a binary comparison reads. Mirrors the diff viewer's
/// modes: the unstaged view compares the index against the file on disk, the
/// staged view HEAD against the index, and a commit against its first parent
/// (the side `git show` picks for a merge).
#[derive(Debug, Clone, PartialEq)]
pub enum BlobMode {
    Unstaged,
    Staged,
    Commit(String),
}

/// One side of a binary comparison: a file's bytes at one revision. `bytes` is
/// empty when the blob is over the preview cap, which `too_large` reports so the
/// UI can say so instead of showing a broken image.
#[derive(Debug, Clone)]
pub struct BlobSide {
    pub bytes: Vec<u8>,
    pub size: u64,
    pub too_large: bool,
}

/// The before/after bytes of `path` — what a text diff can't show for a binary
/// file. Either side is None when the path doesn't exist there: an added file has
/// no old side, a deleted one no new side.
pub fn blob_sides(
    worktree: &Path,
    path: &str,
    mode: &BlobMode,
) -> Result<(Option<BlobSide>, Option<BlobSide>)> {
    match mode {
        // ":path" is the index entry; the new side is the working tree itself.
        BlobMode::Unstaged => Ok((blob_at(worktree, ":0", path)?, disk_blob(worktree, path)?)),
        BlobMode::Staged => Ok((blob_at(worktree, "HEAD", path)?, blob_at(worktree, ":0", path)?)),
        BlobMode::Commit(hash) => {
            Ok((blob_at(worktree, &format!("{hash}^"), path)?, blob_at(worktree, hash, path)?))
        }
    }
}

/// `path` as stored at `rev`, or None when it isn't there (or `rev` itself
/// doesn't resolve — a root commit's `^`, or HEAD in a repo with no commits).
fn blob_at(worktree: &Path, rev: &str, path: &str) -> Result<Option<BlobSide>> {
    // No "./" prefix: `rev:path` is repo-root relative, which is what git status
    // and git show hand us, whereas `rev:./path` would resolve from the cwd.
    let spec = format!("{rev}:{path}");
    // `cat-file -s` doubles as the existence check: it fails for a path that
    // isn't in that revision, and gives the size without reading the blob, so an
    // oversized one is never loaded into memory at all.
    let Some(size) =
        git(worktree, &["cat-file", "-s", &spec]).ok().and_then(|s| s.trim().parse::<u64>().ok())
    else {
        return Ok(None);
    };
    if size > crate::files::MAX_BINARY_BYTES {
        return Ok(Some(BlobSide { bytes: Vec::new(), size, too_large: true }));
    }
    Ok(Some(BlobSide {
        bytes: git_bytes(worktree, &["cat-file", "blob", &spec])?,
        size,
        too_large: false,
    }))
}

/// `path` as it sits in the working tree, or None when it isn't on disk (a
/// deletion git hasn't been told about yet).
fn disk_blob(worktree: &Path, path: &str) -> Result<Option<BlobSide>> {
    let abs = crate::files::resolve_within(worktree, path)?;
    let Ok(meta) = std::fs::metadata(&abs) else {
        return Ok(None);
    };
    let size = meta.len();
    if size > crate::files::MAX_BINARY_BYTES {
        return Ok(Some(BlobSide { bytes: Vec::new(), size, too_large: true }));
    }
    Ok(Some(BlobSide { bytes: std::fs::read(&abs)?, size, too_large: false }))
}

pub fn stage(worktree: &Path, path: &str) -> Result<()> {
    git(worktree, &["add", "--", path])?;
    Ok(())
}

pub fn unstage(worktree: &Path, path: &str) -> Result<()> {
    // `restore --staged` requires git >= 2.23; available on all supported setups.
    git(worktree, &["restore", "--staged", "--", path])?;
    Ok(())
}

pub fn commit(worktree: &Path, message: &str) -> Result<()> {
    git(worktree, &["commit", "-m", message])?;
    Ok(())
}

/// The revision to read a branch's history from: the local branch when there is
/// one, otherwise origin's copy. A PR can be opened between branches that only
/// exist on the remote, and `main..feature` is not a revision range when neither
/// name is a local ref — the log and diffstat would fail and take the whole PR
/// down with them.
fn summary_rev(repo: &Path, branch: &str) -> String {
    if !local_branch_exists(repo, branch) && remote_branch_exists(repo, branch) {
        return format!("origin/{branch}");
    }
    branch.to_string()
}

/// A markdown summary of what `branch` adds over `base` — commit subjects plus
/// a diffstat — used as the generated PR description.
pub fn branch_summary(repo: &Path, branch: &str, base: &str) -> Result<String> {
    let branch = &summary_rev(repo, branch);
    let base = &summary_rev(repo, base);
    let subjects = git(repo, &["log", "--reverse", "--format=%s", &format!("{base}..{branch}")])?;
    let stat = git(repo, &["diff", "--stat", &format!("{base}...{branch}")])?;
    let mut body = String::from("## Summary\n\n");
    for s in subjects.lines().filter(|l| !l.trim().is_empty()) {
        body.push_str(&format!("- {s}\n"));
    }
    if !stat.trim().is_empty() {
        body.push_str("\n## Changes\n\n```\n");
        body.push_str(stat.trim_end());
        body.push_str("\n```\n");
    }
    Ok(body)
}

/// Whether `branch` is currently checked out in any worktree of `repo`
/// (including the primary working tree). git forbids updating a branch ref that
/// is checked out, and forbids checking the same branch out twice — callers use
/// this to avoid both failures.
pub fn branch_checked_out(repo: &Path, branch: &str) -> bool {
    let Ok(out) = git(repo, &["worktree", "list", "--porcelain"]) else {
        return false;
    };
    let needle = format!("branch refs/heads/{branch}");
    out.lines().any(|l| l.trim() == needle)
}

/// Update (or create) the local `branch` from origin's copy without checking
/// it out. Deliberately not forced: a local branch that diverged from origin
/// fails loudly instead of being clobbered.
pub fn fetch_branch(repo: &Path, branch: &str) -> Result<()> {
    // A branch that's already checked out in a worktree can't be updated via a
    // fetch refspec ("refusing to fetch into branch ... checked out at ...") —
    // and we already have it locally, so there's nothing to pull it into. Update
    // the remote-tracking ref only so ahead/behind stays accurate, and stop.
    if branch_checked_out(repo, branch) {
        let _ = git(repo, &["fetch", "origin", branch]);
        return Ok(());
    }
    git(repo, &["fetch", "origin", &format!("{branch}:{branch}")])?;
    Ok(())
}

pub fn push(worktree: &Path) -> Result<()> {
    push_with_progress(worktree, &crate::setup::CancelToken::new(), |_| {})
}

/// Publish a specific local branch to `origin` (setting upstream) without
/// checking it out — used to open a PR from an existing branch in the project
/// repo. git's stderr is carried on failure so the UI can show the real reason.
pub fn push_branch(repo: &Path, branch: &str) -> Result<()> {
    git(repo, &["push", "-u", "origin", branch])?;
    Ok(())
}

/// Push the current branch to `origin` (setting upstream), streaming git's
/// `--progress` output to `on_progress` so a large push shows movement instead
/// of a frozen UI. On failure, the error carries git's raw stderr so the UI can
/// surface the real reason (rejected push, protected branch, no permission, …).
///
/// `cancel` kills the push and fails with [`crate::setup::CANCELLED`]. Uploading
/// a branch with large objects over a slow uplink is minutes the user must be
/// able to escape. Nothing is cleaned up afterwards: a killed push leaves the
/// remote exactly as it was, and the local branch is the user's own.
pub fn push_with_progress(
    worktree: &Path,
    cancel: &crate::setup::CancelToken,
    mut on_progress: impl FnMut(crate::setup::CloneProgress),
) -> Result<()> {
    push_streaming(worktree, false, cancel, &mut on_progress)
}

/// [`push_with_progress`] with `--force-with-lease`: same streaming, same
/// cancellation. Force-pushing after a rebase re-uploads the whole branch, so it
/// is the same minutes-long upload as an ordinary push and needs the same
/// escape; a killed one leaves the remote's ref where it was, since git moves it
/// only once every object has arrived.
pub fn push_force_with_progress(
    worktree: &Path,
    cancel: &crate::setup::CancelToken,
    mut on_progress: impl FnMut(crate::setup::CloneProgress),
) -> Result<()> {
    push_streaming(worktree, true, cancel, &mut on_progress)
}

/// The body both push flavours share: `git push --progress -u origin <branch>`
/// through the cancellable streaming runner, plus `--force-with-lease` when
/// `force`. One function so the two can't drift apart in how they report a
/// cancel or a failure, which is the part that is easy to get subtly wrong.
fn push_streaming(
    worktree: &Path,
    force: bool,
    cancel: &crate::setup::CancelToken,
    on_progress: &mut dyn FnMut(crate::setup::CloneProgress),
) -> Result<()> {
    let branch = git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    let mut cmd = Command::new("git");
    cmd.arg("push").arg("--progress");
    if force {
        // With lease, never bare `--force`: a remote that moved since the last
        // fetch is someone else's work, and this refuses rather than erasing it.
        cmd.arg("--force-with-lease");
    }
    cmd.arg("-u")
        .arg("origin")
        .arg(&branch)
        .current_dir(worktree)
        // No TTY in the app: fail fast rather than blocking on a credential prompt.
        .env("GIT_TERMINAL_PROMPT", "0");
    let (ok, stderr) = crate::setup::run_clone_streaming(cmd, cancel, on_progress)?;
    // Checked before `ok`: a killed git exits non-zero carrying its own error
    // text, and showing that would put a push failure in front of the user who
    // just pressed Cancel.
    if cancel.is_cancelled() {
        bail!("{}", crate::setup::CANCELLED);
    }
    if ok {
        return Ok(());
    }
    bail!("git push failed:\n{}", stderr.trim());
}

/// Whether an `origin` remote is configured.
pub fn has_origin(repo: &Path) -> bool {
    git(repo, &["remote"]).map(|out| out.lines().any(|r| r.trim() == "origin")).unwrap_or(false)
}

/// How long a fetch may run before it is killed. Fetches happen on their own
/// now (see the auto-fetch scheduler in the app), so a wedged one must not sit
/// there forever: the scheduler treats a project as "fetch in flight" until the
/// call returns, and would never fetch it again.
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);

/// Fetch from `origin`, updating remote-tracking refs (and pruning branches
/// deleted upstream) so ahead/behind reflects reality instead of a stale local
/// snapshot. Nothing is checked out or merged.
pub fn fetch(repo: &Path) -> Result<()> {
    git_capped(repo, &["fetch", "--prune", "origin"], FETCH_TIMEOUT)?;
    Ok(())
}

/// [`git`] with a wall-clock cap: the child is killed if it outlives `limit`.
/// `Command::output()` waits forever, which is fine for the local commands but
/// not for one that talks to a remote — an unreachable host, a stalled TLS
/// handshake or a credential helper waiting on the keychain can all hang far
/// longer than any caller wants to wait.
fn git_capped(worktree: &Path, args: &[&str], limit: Duration) -> Result<String> {
    use std::io::Read;

    let mut child = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Drain both pipes on their own threads: a child that fills a pipe buffer
    // blocks on the write and would never reach the exit we are polling for.
    fn reader<R: Read + Send + 'static>(mut r: Option<R>) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(r) = r.as_mut() {
                let _ = r.read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    }
    let out_thread = reader(child.stdout.take());
    let err_thread = reader(child.stderr.take());

    let deadline = std::time::Instant::now() + limit;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                // Return without joining the readers. Killing git does not kill
                // whatever it spawned (`git-remote-https`, `ssh`), and those
                // hold the write ends of these pipes — joining here would wait
                // out the very hang the cap exists to escape. The threads end
                // on their own once the last writer goes.
                bail!("git {:?} timed out after {}s", args, limit.as_secs());
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let stdout = out_thread.join().unwrap_or_default();
    let stderr = err_thread.join().unwrap_or_default();
    if !status.success() {
        bail!("git {:?} failed: {}", args, stderr);
    }
    Ok(stdout)
}

/// Fast-forward the current branch to its upstream. `--ff-only` on purpose: a
/// branch that has diverged fails loudly rather than silently creating a merge
/// commit the user never asked for.
pub fn pull(worktree: &Path) -> Result<()> {
    git(worktree, &["pull", "--ff-only"])?;
    Ok(())
}

/// Ahead/behind counts against the configured upstream (`@{u}`), read from the
/// current remote-tracking refs — call [`fetch`] first if they may be stale.
/// Returns `(ahead, behind)`, or `(0, 0)` when there is no upstream.
pub fn ahead_behind(worktree: &Path) -> Result<(u32, u32)> {
    if git(worktree, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_err() {
        return Ok((0, 0));
    }
    let counts = git(worktree, &["rev-list", "--left-right", "--count", "@{u}...HEAD"])?;
    let mut p = counts.split_whitespace();
    let behind = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ahead = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    Ok((ahead, behind))
}

/// Outcome of [`sync`]. `Diverged` means the caller must decide how to
/// reconcile (rebase, or force-push) — sync never picks for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncOutcome {
    /// Branch is level with upstream: any incoming commits were fast-forwarded
    /// in and any local commits pushed.
    Synced,
    /// Local and upstream each have commits the other lacks. Nothing was
    /// changed — a fast-forward is impossible and sync won't silently create a
    /// merge commit.
    Diverged,
}

/// VS Code-style "Sync Changes": reconcile the branch with its upstream in both
/// directions — fetch, fast-forward in any incoming commits, then push any
/// local ones (streaming push progress). Unlike a bare push, this pulls first,
/// so a branch that is merely behind syncs cleanly instead of failing with a
/// non-fast-forward rejection. A branch that has genuinely diverged can't
/// fast-forward, so it returns [`SyncOutcome::Diverged`] without touching
/// anything and lets the caller offer a rebase or force-push.
///
/// `cancel` reaches the push half, which is the slow one; the fetch and the
/// fast-forward ahead of it are local-speed or already bounded by
/// [`FETCH_TIMEOUT`]. A cancel that lands during those stops the sync before it
/// starts uploading rather than killing anything mid-write.
pub fn sync(
    worktree: &Path,
    cancel: &crate::setup::CancelToken,
    mut on_progress: impl FnMut(crate::setup::CloneProgress),
) -> Result<SyncOutcome> {
    fetch(worktree)?;
    if cancel.is_cancelled() {
        bail!("{}", crate::setup::CANCELLED);
    }
    let (ahead, behind) = ahead_behind(worktree)?;
    if behind > 0 {
        if ahead > 0 {
            return Ok(SyncOutcome::Diverged);
        }
        // Pure catch-up: fast-forward onto the freshly fetched upstream.
        // `--ff-only` so a non-ff situation errors rather than merging.
        git(worktree, &["merge", "--ff-only", "@{u}"])?;
    }
    if ahead > 0 {
        push_with_progress(worktree, cancel, &mut on_progress)?;
    }
    Ok(SyncOutcome::Synced)
}

/// Point `origin` at `url`, adding the remote or updating it if it already
/// exists. Lets the UI publish a branch from a repo that has no remote yet.
pub fn set_origin(worktree: &Path, url: &str) -> Result<()> {
    let exists = git(worktree, &["remote"])
        .map(|out| out.lines().any(|r| r.trim() == "origin"))
        .unwrap_or(false);
    if exists {
        git(worktree, &["remote", "set-url", "origin", url])?;
    } else {
        git(worktree, &["remote", "add", "origin", url])?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffStat {
    pub added: u32,
    pub deleted: u32,
    pub files: u32,
}

pub fn diff_stat(worktree: &Path, base: &str) -> Result<DiffStat> {
    let range = format!("{base}...HEAD");
    Ok(numstat(&git(worktree, &["diff", "--numstat", &range])?))
}

/// What is currently uncommitted (staged and unstaged, tracked files) against
/// HEAD. The counterpart of [`diff_stat`] for a run that works in the project's
/// main checkout: it has no branch of its own to diff against a base, so its
/// visible progress is the pending changes sitting in the checkout.
pub fn uncommitted_stat(dir: &Path) -> Result<DiffStat> {
    Ok(numstat(&git(dir, &["diff", "--numstat", "HEAD"])?))
}

fn numstat(out: &str) -> DiffStat {
    let mut stat = DiffStat { added: 0, deleted: 0, files: 0 };
    for line in out.lines() {
        let mut parts = line.split('\t');
        let a = parts.next().unwrap_or("0");
        let d = parts.next().unwrap_or("0");
        // Binary files show "-" for counts; treat as 0 but still count the file.
        stat.added += a.parse::<u32>().unwrap_or(0);
        stat.deleted += d.parse::<u32>().unwrap_or(0);
        stat.files += 1;
    }
    stat
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub header: String,
    pub hunks: Vec<Hunk>,
}

pub fn parse_diff(diff: &str) -> FileDiff {
    let mut header_lines: Vec<&str> = Vec::new();
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut seen_hunk = false;

    for line in diff.lines() {
        if line.starts_with("@@") {
            seen_hunk = true;
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(Hunk { header: line.to_string(), lines: Vec::new() });
        } else if let Some(h) = current.as_mut() {
            h.lines.push(line.to_string());
        } else if !seen_hunk {
            header_lines.push(line);
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }

    let header = if header_lines.is_empty() {
        String::new()
    } else {
        let mut s = header_lines.join("\n");
        s.push('\n');
        s
    };
    FileDiff { header, hunks }
}

pub fn git_stdin(worktree: &Path, args: &[&str], input: &str) -> Result<()> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("failed to open git stdin"))?
        .write_all(input.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

fn build_hunk_patch(file_diff: &FileDiff, hunk_index: usize) -> Result<String> {
    let hunk = file_diff
        .hunks
        .get(hunk_index)
        .ok_or_else(|| anyhow::anyhow!("hunk index {hunk_index} out of range"))?;
    let mut patch = file_diff.header.clone();
    patch.push_str(&hunk.header);
    patch.push('\n');
    for line in &hunk.lines {
        patch.push_str(line);
        patch.push('\n');
    }
    Ok(patch)
}

pub fn stage_hunk(worktree: &Path, path: &str, hunk_index: usize) -> Result<()> {
    let raw = diff(worktree, path, false)?;
    let fd = parse_diff(&raw);
    let patch = build_hunk_patch(&fd, hunk_index)?;
    git_stdin(worktree, &["apply", "--cached", "--unidiff-zero", "-"], &patch)
}

pub fn unstage_hunk(worktree: &Path, path: &str, hunk_index: usize) -> Result<()> {
    let raw = diff(worktree, path, true)?;
    let fd = parse_diff(&raw);
    let patch = build_hunk_patch(&fd, hunk_index)?;
    git_stdin(worktree, &["apply", "--cached", "--reverse", "--unidiff-zero", "-"], &patch)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub date: i64,
    pub subject: String,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub base: Option<String>,
    /// Whether an `origin` remote is configured — i.e. whether publishing is
    /// even possible. `push` targets `origin`, so without it Publish can't work.
    pub has_remote: bool,
}

/// HEAD ancestry, newest first. Fields are unit-separated (\x1f); parents and
/// refs are space/comma lists. %D yields "HEAD -> main, origin/main, tag: v1".
pub fn log_graph(worktree: &Path, limit: usize) -> Result<Vec<HistoryItem>> {
    let limit_arg = format!("-n{limit}");
    let format = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D";
    let out = match git(worktree, &["log", &limit_arg, format]) {
        Ok(out) => out,
        // A repo with no commits yet: `log` fails on the unborn branch. An
        // empty history is the honest answer there, not an error banner.
        Err(_) if git(worktree, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_err() => {
            return Ok(Vec::new())
        }
        Err(e) => return Err(e),
    };
    let mut items = Vec::new();
    for line in out.lines() {
        let f: Vec<&str> = line.split('\u{1f}').collect();
        if f.len() < 7 {
            continue;
        }
        let parents = f[1].split_whitespace().map(str::to_string).collect();
        // Keep the "HEAD -> " marker verbatim: the UI uses it to highlight the
        // checked-out ref and to mark the HEAD commit in the graph. Drop
        // `origin/HEAD`, git's symbolic pointer to the remote's default branch:
        // it always sits on the same commit as `origin/<default>`, so showing
        // both is redundant noise (VS Code hides it too).
        let refs = f[6]
            .split(',')
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty() && r != "origin/HEAD")
            .collect();
        items.push(HistoryItem {
            hash: f[0].to_string(),
            parents,
            author: f[2].to_string(),
            email: f[3].to_string(),
            date: f[4].parse().unwrap_or(0),
            subject: f[5].to_string(),
            refs,
        });
    }
    Ok(items)
}

/// The repo's default branch as it exists *locally* (`main` or `master`), or
/// `None` in a repo that has neither.
fn local_default_branch(worktree: &Path) -> Option<&'static str> {
    ["main", "master"].into_iter().find(|b| {
        git(worktree, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")]).is_ok()
    })
}

pub fn branch_info(worktree: &Path) -> Result<BranchInfo> {
    // `rev-parse HEAD` fails in a repo with no commits yet (HEAD points at an
    // unborn branch), so fall back to the ref name git is holding for it: a
    // brand-new repo should show its branch, not an error banner.
    let branch = git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])
        .or_else(|_| git(worktree, &["branch", "--show-current"]))?
        .trim()
        .to_string();
    let upstream = git(worktree, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    // Base = where this branch's own work starts, so "ahead" counts only its
    // commits. On a feature branch that's the fork point from the *local*
    // default branch: worktrees share one object store and one `main`, so
    // measuring against `origin/main` instead would fold main's unpushed
    // commits into every branch and show the same count on all of them.
    // On the default branch itself there is no fork point, so fall back to
    // origin's copy — there, "ahead of base" does mean "not pushed yet".
    let base_candidates: Vec<String> = match local_default_branch(worktree) {
        Some(d) if d != branch => vec![d.to_string()],
        _ => {
            ["origin/HEAD", "origin/main", "origin/master"].iter().map(|c| c.to_string()).collect()
        }
    };
    let base = base_candidates.iter().find_map(|cand| {
        git(worktree, &["merge-base", "HEAD", cand])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    });
    let (mut ahead, mut behind) = (0, 0);
    if upstream.is_some() {
        if let Ok(counts) = git(worktree, &["rev-list", "--left-right", "--count", "@{u}...HEAD"]) {
            let mut p = counts.split_whitespace();
            behind = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            ahead = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        }
    } else if let Some(base) = &base {
        // No upstream yet: "ahead" means the commits this branch adds on top of
        // its base — the work Publish would put on a new remote branch. Lets the
        // UI hide Publish when there is nothing to publish.
        if let Ok(count) = git(worktree, &["rev-list", "--count", &format!("{base}..HEAD")]) {
            ahead = count.trim().parse().unwrap_or(0);
        }
    } else {
        // No upstream and nothing to measure against: a repo with no `origin`
        // at all, or one whose remote-tracking refs were never fetched. None of
        // this history has been published, so every commit counts as ahead —
        // otherwise the one case where "add a remote" is exactly what the user
        // wants is the case where the UI would never offer it. Still 0 in a
        // repo with no commits (`rev-list` fails on an unborn HEAD).
        if let Ok(count) = git(worktree, &["rev-list", "--count", "HEAD"]) {
            ahead = count.trim().parse().unwrap_or(0);
        }
    }
    let has_remote = has_origin(worktree);
    Ok(BranchInfo { branch, upstream, ahead, behind, base, has_remote })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBranches {
    /// The branch currently checked out in the primary worktree (or "HEAD" if detached).
    pub current: String,
    /// All local branch names, with `current` first.
    pub branches: Vec<String>,
    /// Branches that exist on `origin` but have no local branch of the same
    /// name, as short names without the `origin/` prefix, sorted. Read from
    /// remote-tracking refs, so they're as fresh as the last fetch — the app
    /// fetches every project in the background, and callers that care can ask
    /// for a fetch first. Kept separate from `branches` because the two aren't
    /// interchangeable: a remote-only branch isn't checked out anywhere and
    /// can't be pushed to, so only callers that handle that (the New PR picker)
    /// should offer them.
    pub remote: Vec<String>,
}

pub fn list_branches(repo: &Path) -> Result<ProjectBranches> {
    let current = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    let raw = git(repo, &["for-each-ref", "--format=%(refname:short)", "refs/heads"])?;
    let mut branches: Vec<String> =
        raw.lines().filter(|l| !l.is_empty()).map(str::to_string).collect();
    // Put the current branch first so the UI can preselect it.
    if let Some(pos) = branches.iter().position(|b| b == &current) {
        branches.remove(pos);
        branches.insert(0, current.clone());
    }
    // A repo with no `origin` (or no fetched refs) just has none of these.
    let raw_remote =
        git(repo, &["for-each-ref", "--format=%(refname:short)", "refs/remotes/origin"])
            .unwrap_or_default();
    let mut remote: Vec<String> = raw_remote
        .lines()
        .filter_map(|l| l.trim().strip_prefix("origin/"))
        // `origin/HEAD` is a symbolic alias for the default branch, not a
        // branch of its own — offering it would open a PR against a duplicate.
        .filter(|b| !b.is_empty() && *b != "HEAD" && !branches.iter().any(|l| l == b))
        .map(str::to_string)
        .collect();
    remote.sort();
    Ok(ProjectBranches { current, branches, remote })
}

fn ref_exists(repo: &Path, name: &str) -> bool {
    git(repo, &["rev-parse", "--verify", "--quiet", name]).is_ok()
}

/// Whether `branch` exists as a local branch in `repo`.
pub fn local_branch_exists(repo: &Path, branch: &str) -> bool {
    ref_exists(repo, &format!("refs/heads/{branch}"))
}

/// Whether `branch` exists on `origin`, as of the last fetch.
pub fn remote_branch_exists(repo: &Path, branch: &str) -> bool {
    ref_exists(repo, &format!("refs/remotes/origin/{branch}"))
}

pub fn stage_all(worktree: &Path) -> Result<()> {
    git(worktree, &["add", "-A"])?;
    Ok(())
}

pub fn unstage_all(worktree: &Path) -> Result<()> {
    git(worktree, &["reset", "-q"])?;
    Ok(())
}

/// Discard one file's changes. Untracked files are deleted; tracked files are
/// restored from the index (mirrors VSCode "Discard Changes" on the Changes group).
pub fn discard(worktree: &Path, path: &str, untracked: bool) -> Result<()> {
    if untracked {
        git(worktree, &["clean", "-f", "--", path])?;
    } else {
        git(worktree, &["restore", "--", path])?;
    }
    Ok(())
}

pub fn discard_all(worktree: &Path) -> Result<()> {
    git(worktree, &["restore", "--", "."])?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitFile {
    pub path: String,
    pub status: String,
}

pub fn commit_files(worktree: &Path, hash: &str) -> Result<Vec<CommitFile>> {
    // --format= strips commit metadata; root commits are handled by `show`.
    let out = git(worktree, &["show", "--name-status", "--format=", hash])?;
    let mut files = Vec::new();
    for line in out.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let status = parts.next().unwrap_or("").chars().next().unwrap_or(' ').to_string();
        // Renames have two paths "R100 old new"; keep the last.
        let path = parts.last().unwrap_or("").to_string();
        if !path.is_empty() {
            files.push(CommitFile { path, status });
        }
    }
    Ok(files)
}

pub fn commit_diff(worktree: &Path, hash: &str, path: &str) -> Result<String> {
    git(worktree, &["show", "--format=", hash, "--", path])
}

pub fn commit_amend(worktree: &Path, message: &str) -> Result<()> {
    git(worktree, &["commit", "--amend", "-m", message])?;
    Ok(())
}

/// Build a patch applying only `selected` body lines of one hunk.
/// Unselected '+' lines are dropped; unselected '-' lines become context so
/// they are preserved. The hunk header counts are recomputed. When `reverse`
/// is true (unstaging the cached diff) the roles of +/- are swapped for counting.
pub fn build_partial_patch(
    fd: &FileDiff,
    hunk_index: usize,
    selected: &[usize],
    reverse: bool,
) -> Result<String> {
    let hunk = fd
        .hunks
        .get(hunk_index)
        .ok_or_else(|| anyhow::anyhow!("hunk index {hunk_index} out of range"))?;
    // Parse "@@ -old_start,old_len +new_start,new_len @@".
    let (old_start, new_start) = parse_hunk_starts(&hunk.header)?;
    let mut body: Vec<String> = Vec::new();
    let (mut old_len, mut new_len) = (0u32, 0u32);
    // The patch is applied forward (stage, file == old side) or reversed
    // (unstage/revert, file == new side). An unselected change must be turned
    // into context on whichever side matches the file being patched, and
    // dropped from the other; otherwise that side won't match and the apply
    // is rejected.
    for (i, line) in hunk.lines.iter().enumerate() {
        let kind = line.chars().next().unwrap_or(' ');
        let sel = selected.contains(&i);
        match kind {
            '+' => {
                if sel {
                    body.push(line.clone());
                    new_len += 1;
                } else if reverse {
                    // Already present on the new side (the file we reverse onto):
                    // keep as context so it survives.
                    body.push(format!(" {}", &line[1..]));
                    old_len += 1;
                    new_len += 1;
                }
                // forward + unselected add: drop entirely
            }
            '-' => {
                if sel {
                    body.push(line.clone());
                    old_len += 1;
                } else if !reverse {
                    // Present on the old side (the file we apply onto): keep as
                    // context so it survives.
                    body.push(format!(" {}", &line[1..]));
                    old_len += 1;
                    new_len += 1;
                }
                // reverse + unselected delete: drop entirely
            }
            _ => {
                body.push(line.clone());
                old_len += 1;
                new_len += 1;
            }
        }
    }
    let mut patch =
        if reverse && old_len > 0 { demote_new_file(&fd.header) } else { fd.header.clone() };
    patch.push_str(&format!("@@ -{old_start},{old_len} +{new_start},{new_len} @@\n"));
    for l in body {
        patch.push_str(&l);
        patch.push('\n');
    }
    Ok(patch)
}

/// Rewrite a `new file mode` header as a plain modification of the same path.
/// Reversing only *some* of a new file's lines does not un-create it: the
/// unselected lines stay, so an old side now exists. Left as a creation against
/// /dev/null, git rejects the patch with "new file f.txt depends on old
/// contents" (observed reverting one line of an untracked file, and unstaging
/// one line of a newly added one). A header with no `new file mode` line is
/// returned unchanged.
fn demote_new_file(header: &str) -> String {
    if !header.lines().any(|l| l.starts_with("new file mode ")) {
        return header.to_string();
    }
    // The old side names the same file as the new one: "+++ b/f" -> "--- a/f",
    // keeping git's C-quoting of an unusual path intact.
    let old_side = header
        .lines()
        .find(|l| l.starts_with("+++ "))
        .map(|l| l.replacen("+++ b/", "--- a/", 1).replacen("+++ \"b/", "--- \"a/", 1));
    let mut out = String::new();
    for line in header.lines() {
        // The blob hashes describe a creation (0000000..new) and no longer hold.
        if line.starts_with("new file mode ") || line.starts_with("index ") {
            continue;
        }
        match (line.starts_with("--- "), old_side.as_deref()) {
            (true, Some(old)) => out.push_str(old),
            _ => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

fn parse_hunk_starts(header: &str) -> Result<(u32, u32)> {
    // header like "@@ -1,1 +1,3 @@ optional"
    let core = header.trim_start_matches("@@").trim();
    let mut parts = core.split_whitespace();
    let old = parts.next().unwrap_or("");
    let new = parts.next().unwrap_or("");
    let old_start =
        old.trim_start_matches('-').split(',').next().unwrap_or("0").parse().unwrap_or(0);
    let new_start =
        new.trim_start_matches('+').split(',').next().unwrap_or("0").parse().unwrap_or(0);
    Ok((old_start, new_start))
}

pub fn stage_lines(
    worktree: &Path,
    path: &str,
    hunk_index: usize,
    selected: &[usize],
) -> Result<()> {
    let fd = parse_diff(&diff(worktree, path, false)?);
    let patch = build_partial_patch(&fd, hunk_index, selected, false)?;
    git_stdin(worktree, &["apply", "--cached", "-"], &patch)
}

pub fn unstage_lines(
    worktree: &Path,
    path: &str,
    hunk_index: usize,
    selected: &[usize],
) -> Result<()> {
    let fd = parse_diff(&diff(worktree, path, true)?);
    let patch = build_partial_patch(&fd, hunk_index, selected, true)?;
    git_stdin(worktree, &["apply", "--cached", "--reverse", "-"], &patch)
}

pub fn revert_lines(
    worktree: &Path,
    path: &str,
    hunk_index: usize,
    selected: &[usize],
) -> Result<()> {
    let fd = parse_diff(&diff(worktree, path, false)?);
    // Reverse-applied onto the working tree (the new side), so use reverse framing.
    let patch = build_partial_patch(&fd, hunk_index, selected, true)?;
    git_stdin(worktree, &["apply", "--reverse", "-"], &patch)
}

/// Switch the worktree to an existing branch.
pub fn checkout_branch(worktree: &Path, name: &str) -> Result<()> {
    git(worktree, &["switch", name])?;
    Ok(())
}

/// Create `name` (optionally at `from` instead of HEAD) and optionally switch to it.
pub fn create_branch(
    worktree: &Path,
    name: &str,
    from: Option<&str>,
    checkout: bool,
) -> Result<()> {
    let mut args: Vec<&str> =
        if checkout { vec!["switch", "-c", name] } else { vec!["branch", name] };
    if let Some(f) = from {
        args.push(f);
    }
    git(worktree, &args)?;
    Ok(())
}

/// Delete a local branch. Non-forced by default so unmerged work fails loudly.
pub fn delete_branch(worktree: &Path, name: &str, force: bool) -> Result<()> {
    git(worktree, &["branch", if force { "-D" } else { "-d" }, name])?;
    Ok(())
}

/// Pull with rebase — the diverged-branch alternative to the ff-only `pull`.
pub fn pull_rebase(worktree: &Path) -> Result<()> {
    git(worktree, &["pull", "--rebase"])?;
    Ok(())
}

/// Force-push the current branch. `--force-with-lease` so a remote updated by
/// someone else since the last fetch is never clobbered silently. See
/// [`push_force_with_progress`]; this is the no-progress, no-cancel wrapper.
pub fn push_force(worktree: &Path) -> Result<()> {
    push_force_with_progress(worktree, &crate::setup::CancelToken::new(), |_| {})
}

/// Undo the last commit, keeping its changes staged (mirrors VSCode's
/// "Undo Last Commit"). Returns the undone commit's message so the UI can put
/// it back into the commit input.
pub fn undo_last_commit(worktree: &Path) -> Result<String> {
    let message = git(worktree, &["log", "-1", "--format=%B"])?.trim_end().to_string();
    git(worktree, &["reset", "--soft", "HEAD~1"])?;
    Ok(message)
}

/// Reset the current branch to `hash`. `mode` is validated against a fixed set
/// so arbitrary flags can never be smuggled into the git invocation.
pub fn reset_to(worktree: &Path, hash: &str, mode: &str) -> Result<()> {
    let flag = match mode {
        "soft" => "--soft",
        "mixed" => "--mixed",
        "hard" => "--hard",
        other => bail!("unsupported reset mode: {other}"),
    };
    git(worktree, &["reset", flag, hash])?;
    Ok(())
}

pub fn revert_commit(worktree: &Path, hash: &str) -> Result<()> {
    git(worktree, &["revert", "--no-edit", hash])?;
    Ok(())
}

pub fn cherry_pick(worktree: &Path, hash: &str) -> Result<()> {
    git(worktree, &["cherry-pick", hash])?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
}

/// All stashes, newest first (index 0 = most recent).
pub fn stash_list(worktree: &Path) -> Result<Vec<StashEntry>> {
    let out = git(worktree, &["stash", "list", "--format=%gd\u{1f}%gs"])?;
    let mut entries = Vec::new();
    for line in out.lines() {
        let mut f = line.split('\u{1f}');
        let (Some(refname), Some(message)) = (f.next(), f.next()) else { continue };
        // refname is "stash@{N}"
        let index =
            refname.trim_start_matches("stash@{").trim_end_matches('}').parse().unwrap_or(0);
        entries.push(StashEntry { index, message: message.to_string() });
    }
    Ok(entries)
}

pub fn stash_push(worktree: &Path, message: Option<&str>, include_untracked: bool) -> Result<()> {
    let mut args = vec!["stash", "push"];
    if include_untracked {
        args.push("--include-untracked");
    }
    if let Some(m) = message {
        if !m.trim().is_empty() {
            args.push("-m");
            args.push(m);
        }
    }
    git(worktree, &args)?;
    Ok(())
}

pub fn stash_apply(worktree: &Path, index: usize) -> Result<()> {
    git(worktree, &["stash", "apply", &format!("stash@{{{index}}}")])?;
    Ok(())
}

pub fn stash_pop(worktree: &Path, index: usize) -> Result<()> {
    git(worktree, &["stash", "pop", &format!("stash@{{{index}}}")])?;
    Ok(())
}

pub fn stash_drop(worktree: &Path, index: usize) -> Result<()> {
    git(worktree, &["stash", "drop", &format!("stash@{{{index}}}")])?;
    Ok(())
}

#[cfg(test)]
mod status_tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn parses_paths_with_spaces_verbatim() {
        let out = " M with space.txt\0?? new file.txt\0";
        let changes = parse_porcelain_z(out);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].path, "with space.txt");
        assert_eq!((changes[0].index.as_str(), changes[0].worktree.as_str()), (" ", "M"));
        assert_eq!(changes[1].path, "new file.txt");
        assert_eq!(changes[1].index, "?");
    }

    #[test]
    fn parses_unicode_paths_verbatim() {
        let out = "?? héllo wörld/naïve — фаил.txt\0";
        let changes = parse_porcelain_z(out);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "héllo wörld/naïve — фаил.txt");
    }

    #[test]
    fn rename_keeps_new_path_and_skips_original() {
        // -z rename entry: "R  new\0old\0" (new path first, origin second).
        let out = "R  new name.txt\0old name.txt\0 M other.txt\0";
        let changes = parse_porcelain_z(out);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].path, "new name.txt");
        assert_eq!(changes[0].index, "R");
        assert_eq!(changes[1].path, "other.txt");
    }

    #[test]
    fn untracked_folder_expands_to_its_files() {
        let changes = parse_porcelain_z(" M kept.txt\0?? sub/\0");
        let expanded = expand_untracked_dirs(changes, |dir| {
            assert_eq!(dir, "sub/");
            Some(vec!["sub/a.txt".into(), "sub/deep/b.txt".into()])
        });
        let paths: Vec<&str> = expanded.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["kept.txt", "sub/a.txt", "sub/deep/b.txt"]);
        assert!(expanded[1..].iter().all(|c| c.index == "?" && c.worktree == "?"));
    }

    #[test]
    fn folder_over_the_cap_stays_one_row() {
        let changes = parse_porcelain_z("?? node_modules/\0");
        let expanded = expand_untracked_dirs(changes, |_| None);
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].path, "node_modules/");
    }

    #[test]
    fn only_untracked_folders_are_expanded() {
        // A tracked path is never a folder, and a file row has no slash to
        // mistake for one: neither may reach the listing walk.
        let changes = parse_porcelain_z(" M src/main.rs\0?? loose.txt\0A  added.txt\0");
        let expanded = expand_untracked_dirs(changes, |dir| panic!("listed {dir}"));
        let paths: Vec<&str> = expanded.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["src/main.rs", "loose.txt", "added.txt"]);
    }

    #[test]
    fn status_lists_files_in_a_new_folder_and_diffs_them() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        let run = |args: &[&str]| {
            let ok = Command::new("git").args(args).current_dir(repo).status().unwrap().success();
            assert!(ok, "git {args:?} failed");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(repo.join("tracked.txt"), "x").unwrap();
        run(&["add", "tracked.txt"]);
        run(&["commit", "-qm", "init"]);

        std::fs::create_dir_all(repo.join("sub/deep")).unwrap();
        std::fs::write(repo.join("sub/a.txt"), "one\n").unwrap();
        std::fs::write(repo.join("sub/deep/b.txt"), "two\n").unwrap();
        // An ignore rule inside the new folder still applies: this is why the
        // expansion goes through ls-files rather than walking the tree itself.
        std::fs::write(repo.join("sub/.gitignore"), "*.log\n").unwrap();
        std::fs::write(repo.join("sub/noisy.log"), "skip me\n").unwrap();

        let paths: Vec<String> = status(repo).unwrap().into_iter().map(|c| c.path).collect();
        assert_eq!(paths, ["sub/.gitignore", "sub/a.txt", "sub/deep/b.txt"]);

        // The whole point: each file now reads, stages and reverts on its own.
        let d = diff(repo, "sub/deep/b.txt", false).unwrap();
        assert!(d.contains("+two"), "expected the file contents as additions, got: {d}");
        stage(repo, "sub/a.txt").unwrap();
        let staged: Vec<String> =
            status(repo).unwrap().into_iter().filter(|c| c.index == "A").map(|c| c.path).collect();
        assert_eq!(staged, ["sub/a.txt"]);
    }

    #[test]
    fn a_huge_folder_stays_one_row_and_says_why() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        let run = |args: &[&str]| {
            let ok = Command::new("git").args(args).current_dir(repo).status().unwrap().success();
            assert!(ok, "git {args:?} failed");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(repo.join("tracked.txt"), "x").unwrap();
        run(&["add", "tracked.txt"]);
        run(&["commit", "-qm", "init"]);

        std::fs::create_dir(repo.join("node_modules")).unwrap();
        for i in 0..=UNTRACKED_DIR_CAP {
            std::fs::write(repo.join("node_modules").join(format!("f{i}.js")), "x").unwrap();
        }
        let paths: Vec<String> = status(repo).unwrap().into_iter().map(|c| c.path).collect();
        assert_eq!(paths, ["node_modules/"]);
        let d = diff(repo, "node_modules/", false).unwrap();
        assert!(d.contains("Untracked folder with more than 500 files"), "got: {d}");

        // One under the cap and the same folder lists file by file.
        std::fs::remove_file(repo.join("node_modules/f0.js")).unwrap();
        assert_eq!(status(repo).unwrap().len(), UNTRACKED_DIR_CAP);
    }

    #[test]
    fn a_nested_repository_stays_one_row_and_says_why() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        let run = |at: &std::path::Path, args: &[&str]| {
            let ok = Command::new("git").args(args).current_dir(at).status().unwrap().success();
            assert!(ok, "git {args:?} failed");
        };
        run(repo, &["init", "-q", "-b", "main"]);
        run(repo, &["config", "user.email", "t@t"]);
        run(repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("tracked.txt"), "x").unwrap();
        run(repo, &["add", "tracked.txt"]);
        run(repo, &["commit", "-qm", "init"]);

        // git cannot see inside another repository's worktree; `-uall` reports
        // it as one entry too, so the row is honest rather than a bug.
        std::fs::create_dir(repo.join("vendored")).unwrap();
        run(&repo.join("vendored"), &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("vendored/inner.txt"), "y").unwrap();

        let paths: Vec<String> = status(repo).unwrap().into_iter().map(|c| c.path).collect();
        assert_eq!(paths, ["vendored/"]);
        let d = diff(repo, "vendored/", false).unwrap();
        assert!(d.contains("Untracked folder holding its own git repository"), "got: {d}");
    }

    #[test]
    fn status_round_trips_special_names_through_git() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        let run = |args: &[&str]| {
            let ok = Command::new("git").args(args).current_dir(repo).status().unwrap().success();
            assert!(ok, "git {args:?} failed");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(repo.join("spaced ünicode – file.txt"), "x").unwrap();
        std::fs::write(repo.join("orig.txt"), "y").unwrap();
        run(&["add", "orig.txt"]);
        run(&["commit", "-qm", "init"]);
        run(&["mv", "orig.txt", "moved name.txt"]);

        let changes = status(repo).unwrap();
        // Untracked path comes back unquoted so per-file ops can use it.
        let untracked = changes.iter().find(|c| c.index == "?").unwrap();
        assert_eq!(untracked.path, "spaced ünicode – file.txt");
        stage(repo, &untracked.path).expect("stage by returned path");
        // Rename reports the new path.
        let renamed = changes.iter().find(|c| c.index == "R").unwrap();
        assert_eq!(renamed.path, "moved name.txt");
    }
}

#[cfg(test)]
mod branch_tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn run(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git").args(args).current_dir(dir).status().unwrap().success();
        assert!(ok, "git {args:?} failed");
    }

    fn init_repo(repo: &std::path::Path) {
        run(repo, &["init", "-q", "-b", "main"]);
        run(repo, &["config", "user.email", "t@t"]);
        run(repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("f"), "x").unwrap();
        run(repo, &["add", "."]);
        run(repo, &["commit", "-qm", "init"]);
    }

    #[test]
    fn create_checkout_delete_branch_roundtrip() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        init_repo(repo);

        create_branch(repo, "feature", None, true).unwrap();
        assert_eq!(branch_info(repo).unwrap().branch, "feature");
        checkout_branch(repo, "main").unwrap();
        assert_eq!(branch_info(repo).unwrap().branch, "main");
        delete_branch(repo, "feature", false).unwrap();
        assert!(!list_branches(repo).unwrap().branches.contains(&"feature".to_string()));
    }

    #[test]
    fn create_branch_from_commit() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        init_repo(repo);
        let first = git(repo, &["rev-parse", "HEAD"]).unwrap().trim().to_string();
        std::fs::write(repo.join("f"), "y").unwrap();
        run(repo, &["commit", "-aqm", "second"]);

        create_branch(repo, "from-first", Some(&first), false).unwrap();
        let tip = git(repo, &["rev-parse", "from-first"]).unwrap().trim().to_string();
        assert_eq!(tip, first);
    }

    #[test]
    fn stash_push_list_pop() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        init_repo(repo);
        std::fs::write(repo.join("f"), "dirty").unwrap();

        stash_push(repo, Some("wip work"), true).unwrap();
        assert!(status(repo).unwrap().is_empty(), "worktree clean after stash");
        let list = stash_list(repo).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].index, 0);
        assert!(list[0].message.contains("wip work"), "message kept: {}", list[0].message);

        stash_pop(repo, 0).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("f")).unwrap(), "dirty");
        assert!(stash_list(repo).unwrap().is_empty());
    }

    #[test]
    fn undo_last_commit_keeps_changes_staged_and_returns_message() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        init_repo(repo);
        std::fs::write(repo.join("f"), "y").unwrap();
        run(repo, &["commit", "-aqm", "second commit"]);

        let msg = undo_last_commit(repo).unwrap();
        assert_eq!(msg, "second commit");
        let changes = status(repo).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].index, "M", "change is staged after soft reset");
    }

    #[test]
    fn reset_to_rejects_unknown_mode() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        init_repo(repo);
        assert!(reset_to(repo, "HEAD", "--hard; rm -rf /").is_err());
        reset_to(repo, "HEAD", "mixed").unwrap();
    }

    #[test]
    fn revert_and_cherry_pick() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        init_repo(repo);
        std::fs::write(repo.join("f"), "y").unwrap();
        run(repo, &["commit", "-aqm", "second"]);
        let second = git(repo, &["rev-parse", "HEAD"]).unwrap().trim().to_string();

        revert_commit(repo, &second).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("f")).unwrap(), "x");
        cherry_pick(repo, &second).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("f")).unwrap(), "y");
    }

    #[test]
    fn log_graph_keeps_head_marker_in_refs() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        init_repo(repo);
        let items = log_graph(repo, 10).unwrap();
        assert!(
            items[0].refs.iter().any(|r| r.starts_with("HEAD -> ")),
            "refs: {:?}",
            items[0].refs
        );
    }

    #[test]
    fn log_graph_hides_origin_head_but_keeps_origin_branch() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        init_repo(repo);
        // A bare remote gives us real remote-tracking refs, including the
        // symbolic origin/HEAD that `git remote set-head` creates.
        let remote_dir = tempdir().unwrap();
        run(remote_dir.path(), &["init", "-q", "--bare"]);
        run(repo, &["remote", "add", "origin", remote_dir.path().to_str().unwrap()]);
        run(repo, &["push", "-q", "-u", "origin", "main"]);
        run(repo, &["remote", "set-head", "origin", "main"]);

        let refs = &log_graph(repo, 10).unwrap()[0].refs;
        assert!(refs.iter().any(|r| r == "origin/main"), "origin/main kept: {refs:?}");
        assert!(!refs.iter().any(|r| r == "origin/HEAD"), "origin/HEAD hidden: {refs:?}");
    }

    #[test]
    fn list_branches_returns_current_first_and_all_locals() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        run(repo, &["init", "-q", "-b", "main"]);
        run(repo, &["config", "user.email", "t@t"]);
        run(repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("f"), "x").unwrap();
        run(repo, &["add", "."]);
        run(repo, &["commit", "-qm", "init"]);
        run(repo, &["branch", "develop"]);

        let pb = list_branches(repo).unwrap();
        assert_eq!(pb.current, "main");
        assert_eq!(pb.branches[0], "main", "current branch must be first");
        assert!(pb.branches.contains(&"main".to_string()));
        assert!(pb.branches.contains(&"develop".to_string()));
        assert!(pb.remote.is_empty(), "no origin, no remote branches: {:?}", pb.remote);
    }

    /// A repo whose only copy of `feature` is on origin, plus the symbolic
    /// `origin/HEAD` that `git remote set-head` writes.
    fn repo_with_remote_only_branch() -> (tempfile::TempDir, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        init_repo(repo);
        let remote_dir = tempdir().unwrap();
        run(remote_dir.path(), &["init", "-q", "--bare"]);
        run(repo, &["remote", "add", "origin", remote_dir.path().to_str().unwrap()]);
        run(repo, &["push", "-q", "-u", "origin", "main"]);
        run(repo, &["remote", "set-head", "origin", "main"]);
        run(repo, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(repo.join("f"), "y").unwrap();
        run(repo, &["commit", "-aqm", "work on the remote branch"]);
        run(repo, &["push", "-q", "origin", "feature"]);
        run(repo, &["checkout", "-q", "main"]);
        run(repo, &["branch", "-qD", "feature"]);
        (dir, remote_dir)
    }

    #[test]
    fn list_branches_reports_remote_only_branches_separately() {
        let (dir, _remote) = repo_with_remote_only_branch();
        let repo = dir.path();

        let pb = list_branches(repo).unwrap();
        assert_eq!(pb.branches, vec!["main".to_string()]);
        assert_eq!(pb.remote, vec!["feature".to_string()]);
        assert!(
            !pb.remote.contains(&"main".to_string()),
            "a branch we have locally is not remote-only"
        );
        assert!(!pb.remote.contains(&"HEAD".to_string()), "origin/HEAD is not a branch");
        assert!(local_branch_exists(repo, "main"));
        assert!(!local_branch_exists(repo, "feature"));
        assert!(remote_branch_exists(repo, "feature"));
        assert!(!remote_branch_exists(repo, "nope"));
    }

    #[test]
    fn branch_summary_reads_a_remote_only_branch_from_origin() {
        let (dir, _remote) = repo_with_remote_only_branch();

        let body = branch_summary(dir.path(), "feature", "main").unwrap();
        assert!(body.contains("- work on the remote branch"), "commit subject listed: {body}");
        assert!(body.contains("## Changes"), "diffstat included: {body}");
    }
}

#[cfg(test)]
mod capped_tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git init failed");
        dir
    }

    #[test]
    fn returns_stdout_when_the_command_finishes() {
        let dir = repo();
        let out = git_capped(
            dir.path(),
            &["rev-parse", "--is-inside-work-tree"],
            Duration::from_secs(30),
        )
        .unwrap();
        assert_eq!(out.trim(), "true");
    }

    #[test]
    fn carries_stderr_when_the_command_fails() {
        let dir = repo();
        let err =
            git_capped(dir.path(), &["rev-parse", "--verify", "nope"], Duration::from_secs(30))
                .unwrap_err()
                .to_string();
        assert!(err.contains("failed"), "unexpected error: {err}");
    }

    #[test]
    fn kills_a_command_that_outlives_its_budget() {
        let dir = repo();
        // A shell alias stands in for a fetch that never returns. Without the
        // cap this call would block for 30s (and a real one, forever).
        let started = std::time::Instant::now();
        let err = git_capped(
            dir.path(),
            &["-c", "alias.stall=!sleep 30", "stall"],
            Duration::from_millis(300),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("timed out"), "unexpected error: {err}");
        assert!(started.elapsed() < Duration::from_secs(10), "did not return promptly");
    }
}
