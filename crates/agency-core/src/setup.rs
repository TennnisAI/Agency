use crate::gh::{GhCli, GhReadiness};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RepoReadiness {
    /// The folder itself is not on disk: moved, renamed, deleted, or on a
    /// volume that is no longer mounted. Nothing in the app works against it,
    /// so it is its own answer rather than a flavour of `NotARepo`.
    Missing,
    NotARepo,
    NoCommits {
        stageable: bool,
    },
    Ready {
        dirty: bool,
    },
}

/// A cancel flag shared with a running git command. The setup and clone dialogs'
/// Cancel flips one so a `git add -A` hashing a huge tree, or a `git clone`
/// downloading a huge repository, is killed instead of grinding on invisibly
/// behind a dialog the user can no longer dismiss.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    /// Ask the command holding this token to stop. Its child is killed within
    /// one poll interval (see [`reap_or_kill`]).
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// The error text a cancelled setup command returns, so callers (and the UI)
/// can tell "the user stopped this" from a real git failure.
pub const CANCELLED: &str = "cancelled";

/// How often a reaper thread checks whether its command was cancelled.
const CANCEL_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// Wait for `child`, killing it as soon as `cancel` fires. This owns the child
/// on its own thread so the kill can't be stuck behind whoever is reading the
/// child's output: `git add -A` prints nothing for minutes while it hashes a
/// large file, so a cancel checked between output lines would not be honoured.
fn reap_or_kill(
    mut child: std::process::Child,
    cancel: &CancelToken,
) -> std::io::Result<std::process::ExitStatus> {
    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            return child.wait();
        }
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        std::thread::sleep(CANCEL_POLL);
    }
}

/// Run a git command in `dir`, returning (success, stdout).
fn git(dir: &Path, args: &[&str]) -> std::io::Result<(bool, String)> {
    let out = Command::new("git").args(args).current_dir(dir).output()?;
    Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).to_string()))
}

/// Whether `path` sits inside a git work tree, or `None` when git could not be
/// run there at all — the binary missing from `PATH`, or the folder gone.
///
/// "This is not a repository" and "we could not ask" are different answers, and
/// any caller that acts on the difference must use this rather than reading
/// [`RepoReadiness::NotARepo`], which still folds an unrunnable git into
/// "plain folder". (A folder that is not there at all is the one case
/// `repo_readiness` does separate out, as [`RepoReadiness::Missing`].)
pub fn inside_work_tree(path: &Path) -> Option<bool> {
    match git(path, &["rev-parse", "--is-inside-work-tree"]) {
        // git ran and answered: success means inside, a non-zero exit ("not a
        // git repository") means definitively outside.
        Ok((ok, _)) => Some(ok),
        Err(_) => None,
    }
}

/// Whether a project's folder is gone from disk: moved, renamed, deleted, or
/// sitting on a volume that is no longer mounted.
///
/// A `stat` that is not there and a `stat` we could not perform are different
/// answers, and only the first one is "missing". This was `!path.is_dir()`, and
/// `Path::is_dir()` folds every error into `false`: a network share whose
/// server was timing out, an `ESTALE` after a remount, an `EPERM` from a
/// sandboxed or TCC-protected location all read as "the folder is gone" with
/// exactly the confidence of a deleted one, and the whole project view was
/// replaced by the missing screen. The UI's rule for this ("no answer is no
/// claim", `useFolderMissing`) cannot defend against it, because the IPC call
/// *succeeds*, returning `true`. So "could not ask" lands on the same side as
/// [`inside_work_tree`]'s `None`: say nothing, and leave the tabs to fail in
/// their own words as they did before any of this existed.
///
/// `metadata` follows symlinks, like the `is_dir` it replaces, so a link left
/// dangling by the move it was pointing into is `NotFound` and reads as missing
/// too; and a path taken over by a file is as unusable as one that is not
/// there, so a successful stat of a non-directory is missing as well.
pub fn folder_missing(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(md) => !md.is_dir(),
        // NotADirectory: a component of the path is itself a file, so nothing
        // can be at the end of it either.
        Err(e) => {
            matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory)
        }
    }
}

/// Inspect a folder and classify how ready it is to host agent worktrees.
/// Worktrees branch from `HEAD`, so a repo needs at least one commit to be `Ready`.
///
/// A folder that is not there at all answers `Missing`, ahead of every git
/// question: git cannot be spawned in a directory that does not exist, so
/// without that arm a moved project folder read as `NotARepo` and every caller
/// took the gitless path — "no branch here, so run the agent in the folder
/// itself" — against a folder there was nothing in. That is what AGE-203 saw
/// as a mix of errors and loading screens across the tabs.
///
/// An unrunnable git in a folder that *is* there still lands on `NotARepo`,
/// which is what setup UI wants. Callers deciding what to *do* with a folder
/// want [`inside_work_tree`] instead.
pub fn repo_readiness(path: &Path) -> RepoReadiness {
    if folder_missing(path) {
        return RepoReadiness::Missing;
    }
    if inside_work_tree(path) != Some(true) {
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

/// The error for a git that could not be started at all. `Command::output`
/// reports a missing binary as the bare `No such file or directory (os error
/// 2)`, which is also what it says about a missing working directory; the
/// setup dialog showed exactly that on a Linux machine with no git (observed
/// 2026-09-10), and the user had no way to tell which of the two it meant.
/// The UI matches on the wording to offer the install (lib/missingTool.ts).
pub fn git_spawn_error(e: std::io::Error, dir: &Path) -> anyhow::Error {
    match e.kind() {
        std::io::ErrorKind::NotFound if !dir.is_dir() => {
            anyhow::anyhow!("{} is not there, so git cannot run in it", dir.display())
        }
        std::io::ErrorKind::NotFound => anyhow::anyhow!("git is not installed"),
        _ => anyhow::anyhow!("could not run git: {e}"),
    }
}

/// Run a git command in `dir`, returning Err with stderr on failure.
fn git_checked(dir: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| git_spawn_error(e, dir))?;
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
    let last = trimmed.rsplit(['/', ':']).next().unwrap_or(trimmed);
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

/// A single progress update from a long-running git command, streamed to the UI
/// so it shows movement instead of a frozen dialog. Named for its first caller;
/// clone, push, and the initial commit all report through it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneProgress {
    /// The git phase, e.g. "Receiving objects" or "Resolving deltas".
    pub phase: String,
    /// Percent complete for this phase (0-100), when git reports one.
    pub percent: Option<u8>,
    /// The rest of the line — e.g. "47% (5000/11000), 12.5 MiB | 5.2 MiB/s".
    pub detail: String,
}

/// Parse one line of `git clone --progress` stderr into a progress update, or
/// `None` for lines that aren't phase progress (e.g. "Cloning into '…'"). Git
/// overwrites these in place with carriage returns, so the streamer splits on
/// `\r` as well as `\n` before handing each line here.
fn parse_clone_progress(line: &str) -> Option<CloneProgress> {
    // Phases that carry a percentage; "remote: " prefixes the server-side ones.
    // Clone reports the "Receiving/Resolving/Updating" phases; push reports the
    // local "Enumerating/Counting/Compressing/Writing objects" phases — this
    // parser feeds both, so the union is listed here.
    const PHASES: &[&str] = &[
        "remote: Counting objects",
        "remote: Compressing objects",
        "remote: Resolving deltas",
        "Enumerating objects",
        "Counting objects",
        "Compressing objects",
        "Writing objects",
        "Receiving objects",
        "Resolving deltas",
        "Updating files",
    ];
    let line = line.trim();
    let phase = PHASES.iter().find(|p| line.starts_with(**p))?;
    let rest = line[phase.len()..].trim_start_matches(':').trim();
    let percent = rest.split('%').next().and_then(|p| p.trim().parse::<u8>().ok());
    // Drop the "remote: " prefix from the label the UI shows.
    let label = phase.strip_prefix("remote: ").unwrap_or(phase);
    Some(CloneProgress { phase: label.to_string(), percent, detail: rest.to_string() })
}

/// Run a clone `Command`, streaming its progress to `on_progress` as git prints
/// it, and return `(success, full_stderr)`. Stderr is piped and split on both
/// `\r` and `\n` because git overwrites progress in place with carriage returns;
/// the full text is still accumulated so the caller can inspect failure output.
///
/// `cancel` kills the child, same as [`run_streaming_stdout`]: a clone of a huge
/// repository runs for many minutes, and until this existed the only way out of
/// the clone dialog was to quit the app. Callers with nothing to cancel from
/// (push) pass a token that is never flipped.
pub(crate) fn run_clone_streaming(
    mut cmd: Command,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(CloneProgress),
) -> std::io::Result<(bool, String)> {
    use std::io::Read;
    use std::process::Stdio;
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = crate::procutil::retry_etxtbsy(|| cmd.spawn())?;
    let mut stderr = child.stderr.take().expect("stderr was piped");
    // A second thread owns the child so the kill isn't stuck behind this one's
    // read: "Resolving deltas" on a large repo prints nothing for a long time,
    // so a cancel checked between progress lines would not be honoured.
    let reaper = {
        let cancel = cancel.clone();
        std::thread::spawn(move || reap_or_kill(child, &cancel))
    };
    let mut chunk = [0u8; 4096];
    let mut line: Vec<u8> = Vec::new();
    let mut full = String::new();
    // Killing the child can surface here as a read error rather than a clean EOF;
    // stopping on either leaves the reaper to report the exit status, which is
    // what the caller decides on.
    while let Ok(n) = stderr.read(&mut chunk) {
        if n == 0 {
            break;
        }
        for &b in &chunk[..n] {
            if b == b'\r' || b == b'\n' {
                flush_progress_line(&mut line, &mut full, on_progress);
            } else {
                line.push(b);
            }
        }
    }
    flush_progress_line(&mut line, &mut full, on_progress);
    let status =
        reaper.join().map_err(|_| std::io::Error::other("git reaper thread panicked"))??;
    Ok((status.success(), full))
}

/// Run `cmd`, handing each line of its stdout to `on_line` as git prints it, and
/// return `(success, full_stderr)`. Unlike [`run_clone_streaming`], progress here
/// comes from stdout, so stderr is drained on a side thread — a git that's chatty
/// there (one line-ending warning per file, say) would otherwise fill its pipe
/// and wedge the stdout reader. A third thread owns the child so `cancel` can
/// kill it while this one is blocked on a read.
fn run_streaming_stdout(
    mut cmd: Command,
    cancel: &CancelToken,
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<(bool, String)> {
    use std::io::{BufRead, BufReader, Read};
    use std::process::Stdio;
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let mut stderr = child.stderr.take().expect("stderr was piped");
    let drain = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let reaper = {
        let cancel = cancel.clone();
        std::thread::spawn(move || reap_or_kill(child, &cancel))
    };
    for line in BufReader::new(stdout).lines() {
        on_line(&line?);
    }
    let status =
        reaper.join().map_err(|_| std::io::Error::other("git reaper thread panicked"))??;
    Ok((status.success(), drain.join().unwrap_or_default()))
}

/// Emit `line` as a progress update (if it parses) and append it to `full`,
/// then clear it for the next line.
fn flush_progress_line(
    line: &mut Vec<u8>,
    full: &mut String,
    on_progress: &mut dyn FnMut(CloneProgress),
) {
    if line.is_empty() {
        return;
    }
    let s = String::from_utf8_lossy(line).to_string();
    line.clear();
    if let Some(p) = parse_clone_progress(&s) {
        on_progress(p);
    }
    full.push_str(&s);
    full.push('\n');
}

/// Clone `url` into a new folder under `parent_dir`. See
/// [`clone_repo_with_progress`]; this is the no-progress, no-cancel convenience
/// wrapper.
pub fn clone_repo(url: &str, parent_dir: &Path) -> Result<PathBuf> {
    clone_repo_with_progress(url, parent_dir, &CancelToken::new(), |_| {})
}

/// The folder [`clone_repo_with_progress`] will create for `url` under
/// `parent_dir`, or `None` when the URL yields no name. Public so a caller can
/// name an in-flight clone (to cancel it) from the same two arguments it started
/// the clone with, instead of reproducing the naming rule.
pub fn clone_destination(url: &str, parent_dir: &Path) -> Option<PathBuf> {
    let name = repo_name_from_url(url);
    (!name.is_empty()).then(|| parent_dir.join(name))
}

/// Delete a clone the user cancelled part-way. The setup commit deliberately
/// leaves a cancelled folder alone — there the files are the user's own — but
/// everything here was downloaded by the git we just killed:
/// [`clone_repo_with_progress`] refuses to start when the destination exists, so
/// nothing at this path predates the clone. Leaving the husk would also block
/// the retry, since that same existence check would then find it.
fn discard_partial_clone(dest: &Path) {
    let _ = std::fs::remove_dir_all(dest);
}

/// Clone `url` into a new folder under `parent_dir`, named after the repo, and
/// return the new path, calling `on_progress` as git reports download progress.
/// The destination must not already exist — git refuses to clone into a
/// non-empty dir, but checking up front yields a clearer message.
///
/// Credentials are the usual snag: the desktop app has no TTY, so a private repo
/// makes `git` try (and fail) to read a username from a terminal that isn't
/// there. We disable that prompt so the failure is fast and legible, then —
/// for github.com — retry through `gh`, which supplies the user's stored
/// credentials. That makes an authenticated private clone just work; when it
/// can't, `github_auth_message` says exactly what to configure.
///
/// `cancel` kills the clone and deletes the half-downloaded destination, failing
/// with [`CANCELLED`]. Cloning a large repository is minutes of download the user
/// must be able to escape without quitting the app.
pub fn clone_repo_with_progress(
    url: &str,
    parent_dir: &Path,
    cancel: &CancelToken,
    mut on_progress: impl FnMut(CloneProgress),
) -> Result<PathBuf> {
    let Some(dest) = clone_destination(url, parent_dir) else {
        bail!("couldn't determine a folder name from the URL");
    };
    if dest.exists() {
        bail!("{} already exists — choose another location", dest.display());
    }
    let mut cmd = Command::new("git");
    cmd.arg("clone")
        .arg("--progress")
        .arg(url)
        .arg(&dest)
        .current_dir(parent_dir)
        // No TTY in the app: fail fast instead of blocking on a prompt git can't read.
        .env("GIT_TERMINAL_PROMPT", "0");
    let (ok, stderr) = run_clone_streaming(cmd, cancel, &mut on_progress)?;
    // Checked before `ok`: a killed git exits non-zero carrying its own error
    // text, and reporting that as a clone failure would put it in front of the
    // user who just pressed Cancel — or send them down the sign-in path, since
    // a killed clone's stderr can contain "Permission denied".
    if cancel.is_cancelled() {
        discard_partial_clone(&dest);
        bail!("{CANCELLED}");
    }
    if ok {
        return Ok(dest);
    }

    if is_auth_failure(&stderr) {
        if is_github_url(url) {
            let gh = GhCli::default();
            // For github.com, gh can inject credentials where bare git couldn't.
            // If gh fails too (no access, bad URL), fall through to guidance.
            let retried = matches!(gh.auth_readiness(), GhReadiness::Ready)
                && gh.clone_with_progress(url, &dest, cancel, &mut on_progress).is_ok();
            if cancel.is_cancelled() {
                discard_partial_clone(&dest);
                bail!("{CANCELLED}");
            }
            if retried {
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

/// Emit a staged-file count every this many files. Per-file updates would flood
/// the IPC channel on a large folder without telling the user anything more.
const STAGE_PROGRESS_EVERY: u64 = 100;

/// What the setup dialog asked for when creating a repo's first commit.
#[derive(Debug, Clone, Default)]
pub struct CommitOptions {
    /// Write the default `.gitignore` first (brand-new repo only).
    pub add_gitignore: bool,
    /// Paths (relative to the repo, folders ending in `/`) to add to
    /// `.gitignore` instead of committing — the large files the user opted out
    /// of. See [`gitignore_line`].
    pub ignore_paths: Vec<String>,
    /// Flipped by the dialog's Cancel to kill the staging pass.
    pub cancel: CancelToken,
}

/// Stage everything and make the initial commit. See
/// [`initial_commit_with_progress`]; this is the no-progress convenience wrapper.
pub fn initial_commit(path: &Path, add_gitignore: bool) -> Result<()> {
    let opts = CommitOptions { add_gitignore, ..Default::default() };
    initial_commit_with_progress(path, &opts, |_| {})
}

/// Turn a repo-relative path into a `.gitignore` line matching exactly it:
/// anchored at the repo root with a leading `/`, with the characters git would
/// read as a pattern escaped. The leading `/` also spares `#` and `!`, which are
/// only special at the start of a line.
pub fn gitignore_line(rel: &str) -> String {
    let mut out = String::from("/");
    for c in rel.chars() {
        if matches!(c, '*' | '?' | '[' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    // Git drops unescaped trailing whitespace from a pattern.
    if out.ends_with(' ') {
        out.pop();
        out.push_str("\\ ");
    }
    out
}

/// Append ignore rules for `paths` to the repo's `.gitignore` under a header,
/// skipping any rule the file already has. Creates the file when missing.
fn append_gitignore(path: &Path, paths: &[String]) -> Result<()> {
    use std::io::Write;
    let gi = path.join(".gitignore");
    let existing = std::fs::read_to_string(&gi).unwrap_or_default();
    let fresh: Vec<String> = paths
        .iter()
        .map(|p| gitignore_line(p))
        .filter(|l| !existing.lines().any(|e| e.trim() == l))
        .collect();
    if fresh.is_empty() {
        return Ok(());
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&gi)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        writeln!(f)?;
    }
    writeln!(f, "\n# Large files, excluded when this repository was set up")?;
    for l in fresh {
        writeln!(f, "{l}")?;
    }
    Ok(())
}

/// Drop `paths` from the index. A cancelled staging pass leaves what it already
/// hashed staged, and `git add -A` never unstages what a new ignore rule hides —
/// so without this, retrying after choosing to ignore a huge file would commit it
/// anyway. Only safe before the first commit, where the index holds nothing but
/// what we staged ourselves, so the caller checks for that.
fn unstage_ignored(path: &Path, paths: &[String]) {
    // `:(literal)` so a path with glob characters in it means itself.
    let specs: Vec<String> = paths
        .iter()
        .map(|p| format!(":(literal){}", p.trim_end_matches('/')))
        .filter(|s| s != ":(literal)")
        .collect();
    if specs.is_empty() {
        return;
    }
    let mut args = vec!["rm", "-r", "--cached", "--quiet", "--ignore-unmatch", "--"];
    args.extend(specs.iter().map(|s| s.as_str()));
    // Best-effort: an empty index (nothing staged yet) is the common case.
    let _ = git(path, &args);
}

/// Delete a stale `index.lock`. Killing `git add` mid-run leaves one behind, and
/// every later git command in that repo fails until it's gone. Safe here because
/// the process that held it is the one we just killed.
fn clear_index_lock(path: &Path) {
    let Ok((true, out)) = git(path, &["rev-parse", "--git-dir"]) else { return };
    let dir = PathBuf::from(out.trim());
    let git_dir = if dir.is_absolute() { dir } else { path.join(dir) };
    let _ = std::fs::remove_file(git_dir.join("index.lock"));
}

/// Stage everything and make the initial commit, calling `on_progress` as it
/// goes. Optionally writes a default `.gitignore` first. Falls back to
/// `--allow-empty` when nothing is staged (empty or fully-ignored folder), so the
/// repo still gains a usable `HEAD`.
///
/// Staging is the slow part: on a folder with a large tree, `git add -A` hashes
/// every file, which is minutes of work the user was staring at a frozen window
/// through. Git reports no percentage for `add` or `commit` the way it does for
/// clone, so progress is a running count of staged files under hand-named phases
/// — enough to show the app is alive and working. `opts.cancel` cuts that pass
/// short: on a folder of 50GB model weights it is the difference between a
/// dismissable dialog and quitting the app.
pub fn initial_commit_with_progress(
    path: &Path,
    opts: &CommitOptions,
    mut on_progress: impl FnMut(CloneProgress),
) -> Result<()> {
    if opts.add_gitignore {
        write_default_gitignore(path)?;
    }
    if !opts.ignore_paths.is_empty() {
        append_gitignore(path, &opts.ignore_paths)?;
        // No HEAD yet → the index is ours alone, so clearing these entries can't
        // stage a deletion of something the user has committed.
        if !matches!(git(path, &["rev-parse", "--verify", "HEAD"]), Ok((true, _))) {
            unstage_ignored(path, &opts.ignore_paths);
        }
    }
    let staging = |detail: String| CloneProgress {
        phase: "Staging files".to_string(),
        percent: None,
        detail,
    };
    on_progress(staging(String::new()));

    // `--verbose` prints one line per staged path; counting those lines is the
    // only progress git offers here. Match on emptiness rather than the "add '…'"
    // prefix, which git localizes.
    let mut cmd = Command::new("git");
    cmd.args(["add", "-A", "--verbose"]).current_dir(path);
    let mut count: u64 = 0;
    let staged = run_streaming_stdout(cmd, &opts.cancel, &mut |line| {
        if line.trim().is_empty() {
            return;
        }
        count += 1;
        if count.is_multiple_of(STAGE_PROGRESS_EVERY) {
            on_progress(staging(format!("{count} files")));
        }
    });
    // Before unwrapping `staged`: killing the child can surface as a read error
    // rather than a clean EOF, and returning that error would skip the cleanup
    // below and leave behind exactly the lock it exists to remove.
    if opts.cancel.is_cancelled() {
        // The killed `git add` left its lock behind; without this, every later
        // git command in the folder fails and the user has to find the file.
        clear_index_lock(path);
        bail!("{CANCELLED}");
    }
    let (ok, stderr) = staged?;
    if !ok {
        bail!("git add -A failed: {}", stderr.trim());
    }

    on_progress(CloneProgress {
        phase: "Writing commit".to_string(),
        percent: None,
        detail: format!("{count} files"),
    });
    // Nothing staged → empty commit so HEAD exists and worktrees can branch.
    // Probe the index rather than trusting `count`: on an already-initialized
    // repo the user may have staged changes that `git add -A` had nothing to add.
    let staged =
        Command::new("git").args(["diff", "--cached", "--quiet"]).current_dir(path).status()?;
    let mut commit_args = vec!["commit", "-m", "Initial commit"];
    if staged.success() {
        // exit 0 from --quiet means no staged changes.
        commit_args.push("--allow-empty");
    }
    // Last chance to honour a Cancel: staging is the slow part, so a click that
    // lands as it finishes would otherwise still produce a commit, moments after
    // the dialog closed telling the user nothing had happened. The index keeps
    // whatever was staged, same as a cancelled `git add`.
    if opts.cancel.is_cancelled() {
        bail!("{CANCELLED}");
    }
    git_checked(path, &commit_args)
}

/// Files at least this big are worth a warning before `git add -A` hashes them
/// into the repo. 100 MB is also the file size GitHub refuses to accept, so it's
/// the size at which committing is likely a mistake either way.
pub const LARGE_FILE_BYTES: u64 = 100 * 1024 * 1024;

/// Largest files named individually in the scan result; the rest are counted.
const MAX_LARGE_FILES: usize = 8;
/// Ceilings on the walk. It runs while the setup dialog is already on screen and
/// never gates the button, so the cost is invisible — but a folder with a
/// million entries still has to stop somewhere, and a partial answer ("these
/// three are 48 GB") is worth as much as a complete one.
const SCAN_MAX_ENTRIES: usize = 400_000;
const SCAN_MAX_TIME: std::time::Duration = std::time::Duration::from_secs(3);
/// Large files tracked before the walk gives up on listing more of them.
const SCAN_MAX_HITS: usize = 2_000;

/// One file big enough to warn about, with its path relative to the folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub path: String,
    pub bytes: u64,
}

/// What a folder holds that would make its first commit expensive.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LargeFileScan {
    /// The biggest offenders, largest first, capped at [`MAX_LARGE_FILES`].
    pub files: Vec<LargeFile>,
    /// How many files were over the threshold in total.
    pub count: usize,
    /// Their combined size.
    pub bytes: u64,
    /// The walk hit a budget and stopped, so there may be more than this.
    pub truncated: bool,
    /// What to exclude to leave them all out: the folder (with a trailing `/`)
    /// where several sit together, else the file itself.
    pub ignore_paths: Vec<String>,
    /// The size a file had to reach to be counted here. Reported rather than
    /// assumed so the dialog can name the threshold it actually used, instead of
    /// spelling out a number that drifts the moment this constant changes.
    pub threshold_bytes: u64,
}

/// Collapse large-file paths into the shortest set of things to ignore: a folder
/// holding more than one of them is ignored whole, anything else by name.
fn ignore_paths_for(files: &[LargeFile]) -> Vec<String> {
    use std::collections::BTreeMap;
    let mut by_dir: BTreeMap<&str, Vec<&LargeFile>> = BTreeMap::new();
    for f in files {
        let dir = f.path.rfind('/').map(|i| &f.path[..i]).unwrap_or("");
        by_dir.entry(dir).or_default().push(f);
    }
    let mut out = Vec::new();
    for (dir, group) in by_dir {
        // Files sitting at the repo root are always named one by one — ignoring
        // "/" would exclude the whole project.
        if dir.is_empty() || group.len() < 2 {
            out.extend(group.iter().map(|f| f.path.clone()));
        } else {
            out.push(format!("{dir}/"));
        }
    }
    // Sorted so the dialog lists them the same way every time.
    out.sort();
    out
}

/// Find the files in `root` that are big enough to make committing it a slow
/// mistake — model weights, datasets, videos. Walks the folder itself rather
/// than asking git, because the folder isn't a repository yet when the setup
/// dialog first asks. `.git` is skipped; symlinks are never followed.
///
/// Only metadata is read, so this is a directory walk, not a hash of anything:
/// on an ordinary project it finishes in milliseconds, and on a pathological one
/// the budget stops it (see [`SCAN_MAX_ENTRIES`]).
pub fn scan_large_files(root: &Path) -> LargeFileScan {
    scan_files_over(root, LARGE_FILE_BYTES)
}

/// [`scan_large_files`] with the threshold spelled out, so tests don't have to
/// write hundred-megabyte files to exercise the walk.
fn scan_files_over(root: &Path, threshold: u64) -> LargeFileScan {
    let deadline = std::time::Instant::now() + SCAN_MAX_TIME;
    let mut stack = vec![root.to_path_buf()];
    let mut hits: Vec<LargeFile> = Vec::new();
    let mut scan = LargeFileScan { threshold_bytes: threshold, ..Default::default() };
    let mut visited = 0usize;
    'walk: while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            visited += 1;
            if visited > SCAN_MAX_ENTRIES
                || (visited.is_multiple_of(512) && std::time::Instant::now() > deadline)
            {
                scan.truncated = true;
                break 'walk;
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                if entry.file_name() != ".git" {
                    stack.push(path);
                }
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() < threshold {
                continue;
            }
            scan.count += 1;
            scan.bytes += meta.len();
            if hits.len() >= SCAN_MAX_HITS {
                scan.truncated = true;
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            hits.push(LargeFile { path: rel, bytes: meta.len() });
        }
    }
    scan.ignore_paths = ignore_paths_for(&hits);
    hits.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    hits.truncate(MAX_LARGE_FILES);
    scan.files = hits;
    scan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_git_and_a_missing_folder_get_different_sentences() {
        let not_found = || std::io::Error::from(std::io::ErrorKind::NotFound);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(git_spawn_error(not_found(), dir.path()).to_string(), "git is not installed");
        let gone = dir.path().join("gone");
        assert_eq!(
            git_spawn_error(not_found(), &gone).to_string(),
            format!("{} is not there, so git cannot run in it", gone.display())
        );
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(git_spawn_error(denied, dir.path()).to_string().starts_with("could not run git: "));
    }

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

    #[test]
    fn ignore_lines_are_anchored_and_escaped() {
        assert_eq!(gitignore_line("models/llama.gguf"), "/models/llama.gguf");
        // Characters git would read as a pattern must mean themselves; a space
        // is only a problem at the end, where git would drop it.
        assert_eq!(gitignore_line("data/[raw] set*.bin"), "/data/\\[raw] set\\*.bin");
        assert_eq!(gitignore_line("weights "), "/weights\\ ");
        // '#' needs no escape: the line already starts with '/'.
        assert_eq!(gitignore_line("#notes.bin"), "/#notes.bin");
    }

    fn large(path: &str) -> LargeFile {
        LargeFile { path: path.to_string(), bytes: LARGE_FILE_BYTES }
    }

    #[test]
    fn ignores_a_shared_folder_but_names_lone_files() {
        let paths = ignore_paths_for(&[
            large("models/a.gguf"),
            large("models/b.gguf"),
            large("data/one.bin"),
            large("root.bin"),
        ]);
        // Two files in models/ collapse to the folder; the singletons don't, and
        // a file at the root never collapses (that would be the whole project).
        assert_eq!(paths, vec!["data/one.bin", "models/", "root.bin"]);
    }

    #[test]
    fn scan_finds_big_files_and_skips_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("models")).unwrap();
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        std::fs::write(root.join("models/w.gguf"), vec![0u8; 64]).unwrap();
        // Git's own storage is never the user's problem to ignore.
        std::fs::write(root.join(".git/objects/pack"), vec![0u8; 64]).unwrap();
        std::fs::write(root.join("small.txt"), b"hi").unwrap();

        let scan = scan_files_over(root, 32);
        assert_eq!(scan.count, 1);
        assert_eq!(scan.files, vec![LargeFile { path: "models/w.gguf".into(), bytes: 64 }]);
        assert_eq!(scan.bytes, 64);
        assert!(!scan.truncated);
        assert_eq!(scan.ignore_paths, vec!["models/w.gguf"]);
    }

    #[test]
    fn scan_orders_by_size_and_counts_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (name, size) in [("a.bin", 40), ("b.bin", 90), ("c.bin", 60)] {
            std::fs::write(root.join(name), vec![0u8; size]).unwrap();
        }
        let scan = scan_files_over(root, 32);
        assert_eq!(scan.count, 3);
        assert_eq!(scan.bytes, 190);
        assert_eq!(
            scan.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["b.bin", "c.bin", "a.bin"]
        );
    }
}
