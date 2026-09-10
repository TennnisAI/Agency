//! PATH repair for GUI launch.
//!
//! When the app is launched from Finder/Dock as a macOS `.app` bundle it inherits
//! launchd's minimal default PATH (`/usr/bin:/bin:/usr/sbin:/sbin`). That excludes
//! Homebrew (`/opt/homebrew/bin`, `/usr/local/bin`) and other common locations, so
//! child processes we spawn — the `agency-termd` daemon and agent CLIs — fail to resolve
//! with a bare `ENOENT` ("No such file or directory (os error 2)"). `git` only
//! survives because macOS ships it in `/usr/bin`.
//!
//! [`repair`] appends the common tool directories that exist on disk to the current
//! process PATH, which child `Command`s inherit.
//!
//! The login-shell probe is the load-bearing half: the hardcoded list can only
//! name directories we already know about, and a machine's real tool directory
//! is whatever its version manager chose. Two observed ways the probe used to
//! come back empty, both fixed here (see [`login_shell_path`]): a profile whose
//! last command fails, and a PATH exported from `.zshrc` rather than
//! `.zprofile`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Marker the probe wraps `$PATH` in. A login shell runs the user's profile,
/// and anything that profile prints (a fetch banner, a motd echo, a version
/// manager's greeting) lands on the same stdout as the answer. Parsing the
/// whole of stdout as a PATH glues that chatter onto the first entry, which
/// then fails the `exists` check and is dropped; the marker makes the answer
/// findable in the noise instead.
const PATH_MARKER: &str = "__agency_path__";
/// Same probe, a second line: Finder-launched bundles inherit no user secrets
/// either, and dsh's first-run modal asks for `DEEPSEEK_API_KEY` on every
/// launch until it sees one in the process environment (or `$DSH_HOME`).
const ENV_MARKER: &str = "__agency_env__";
/// Env vars the login-shell probe copies onto this process when launchd did
/// not. Named explicitly: we will not dump `$HOME` or the whole environment
/// into the process, only the keys a child CLI has no other way to see.
const HARVEST_VARS: &[&str] = &["DEEPSEEK_API_KEY"];

/// How long the probe gets before it is killed. Interactive rc files do real
/// work (completion init, version-manager shims), so this is not the 3s a
/// `printf` needs — it is the budget for the user's whole startup file.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What [`repair`] did, for the log line the app writes once its logging
/// plugin is up — `repair` runs before that, so it cannot log for itself.
/// Observed 2026-08-26: on a machine where the probe returned nothing, working
/// out why took a `strings(1)` of the installed bundle and a `ps eww` of the
/// running app, because nothing the app wrote down said where its PATH came
/// from or whether the probe had run at all.
static REPORT: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

/// Harvested login-shell env, filled by [`repair`]. Secrets are never written
/// to the PATH cache file; waiters in [`harvested`] block on this instead.
struct Harvest {
    env: Mutex<HashMap<String, String>>,
    done: Condvar,
    finished: Mutex<bool>,
}

static HARVEST: OnceLock<Harvest> = OnceLock::new();

fn harvest_slot() -> &'static Harvest {
    HARVEST.get_or_init(|| Harvest {
        env: Mutex::new(HashMap::new()),
        done: Condvar::new(),
        finished: Mutex::new(false),
    })
}

/// A login-shell value for `name` if the probe found one and launchd did not
/// already have it. Waits briefly for a background probe (the PATH cache hit
/// path) so a dsh launch right after startup still sees `DEEPSEEK_API_KEY`.
pub fn harvested(name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(name) {
        if !v.is_empty() {
            return Some(v);
        }
    }
    let h = HARVEST.get()?;
    let mut finished = h.finished.lock().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !*finished {
        let wait = deadline.saturating_duration_since(Instant::now());
        if wait.is_zero() {
            break;
        }
        let (guard, result) = h.done.wait_timeout(finished, wait).ok()?;
        finished = guard;
        if result.timed_out() {
            break;
        }
    }
    drop(finished);
    h.env.lock().unwrap().get(name).cloned().filter(|s| !s.is_empty())
}

fn finish_harvest(env: HashMap<String, String>) {
    for (k, v) in &env {
        if std::env::var(k).ok().filter(|s| !s.is_empty()).is_none() {
            std::env::set_var(k, v);
        }
    }
    let h = harvest_slot();
    *h.env.lock().unwrap() = env;
    *h.finished.lock().unwrap() = true;
    h.done.notify_all();
}

/// One line describing where this process's PATH came from. Empty before
/// [`repair`] has run.
pub fn report() -> String {
    REPORT.read().unwrap().clone()
}

/// Ask the user's login shell for its `PATH`. A Finder-launched bundle inherits
/// launchd's minimal `PATH`, so the only reliable way to learn where the user's
/// tools actually live (nvm/volta/fnm/asdf shims, npm global prefixes, custom
/// dirs added in `.zprofile`/`.bash_profile`) is to run their login shell and
/// read the `PATH` it composes. Returns `None` if `SHELL` is unset, the spawn
/// fails, or the shell never printed the marker — callers fall back to
/// [`common_bin_dirs`].
///
/// Two observed failures decide the shape of this:
///
/// - The shell runs `-il`, interactive *and* login, not `-lc`. A
///   non-interactive zsh reads `.zshenv`, `.zprofile` and `.zlogin` but never
///   `.zshrc`, which is where nvm's installer (and every tool that copies it)
///   writes its PATH export. bash reads its profile either way.
/// - The exit status is ignored. A profile whose last command fails makes the
///   shell exit non-zero with a perfectly good PATH already on stdout, and
///   discarding it strands every directory that profile contributed. Same for
///   a shell that prints the marker and then hangs: we kill it at the deadline
///   and keep what it said.
struct Probe {
    path: Option<String>,
    env: HashMap<String, String>,
}

fn harvest_script() -> String {
    let mut script = format!("printf '\\n{PATH_MARKER}%s\\n' \"$PATH\"");
    for var in HARVEST_VARS {
        script.push_str(&format!("; printf '\\n{ENV_MARKER}{var}=%s\\n' \"${{{var}-}}\""));
    }
    script
}

fn login_shell_probe() -> Probe {
    use std::process::{Command, Stdio};

    let empty = Probe { path: None, env: HashMap::new() };
    let Some(shell) = std::env::var("SHELL").ok().filter(|s| !s.is_empty()) else {
        return empty;
    };
    let mut child = match Command::new(&shell)
        .arg("-ilc")
        .arg(&harvest_script())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return empty,
    };

    // Drain on a thread rather than after the wait: a chatty profile can fill
    // the pipe buffer, and a child blocked on a write we never read would sit
    // there until the deadline killed it, answer and all.
    let out = crate::model_probe::drain(child.stdout.take());

    // Poll rather than block: a misconfigured profile could hang the shell, and
    // this runs on the startup path. Give it a budget, then kill.
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break,
        }
    }

    parse_probe(&out.join().unwrap_or_default())
}

/// The PATH out of a probe's stdout: the last marked line, whatever else the
/// user's profile printed around it. Pure so the noise cases are testable.
fn parse_marked_path(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix(PATH_MARKER))
        .map(str::to_string)
        .filter(|p| !p.is_empty())
}

/// Allowlisted `NAME=value` lines the probe printed. Unknown names are dropped
/// so a banner that happens to contain the marker cannot inject env.
fn parse_marked_env(stdout: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in stdout.lines() {
        let Some(rest) = line.trim().strip_prefix(ENV_MARKER) else {
            continue;
        };
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        if HARVEST_VARS.contains(&name) && !value.is_empty() {
            out.insert(name.to_string(), value.to_string());
        }
    }
    out
}

fn parse_probe(stdout: &str) -> Probe {
    Probe { path: parse_marked_path(stdout), env: parse_marked_env(stdout) }
}

/// Split a `PATH`-style string into directory entries.
fn path_dirs(path: &str) -> Vec<PathBuf> {
    path.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect()
}

/// File where the last successful [`login_shell_path`] result is cached, so we
/// don't pay the probe on the startup critical path every launch. `None` if
/// `HOME` is unset.
fn shell_path_cache_file() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().filter(|s| !s.is_empty())?;
    Some(PathBuf::from(home).join("Library/Caches/build.agency.app/login-shell-path"))
}

fn read_cached_shell_path() -> Option<String> {
    let text = std::fs::read_to_string(shell_path_cache_file()?).ok()?;
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn write_cached_shell_path(path: &str) {
    let Some(file) = shell_path_cache_file() else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, path);
}

/// The login-shell PATH to fold into the process PATH, using a cache to keep the
/// slow shell probe off the startup critical path. On a cache hit we return the
/// cached value immediately and refresh the cache in the background (the fresh
/// value applies next launch); only the first-ever launch pays the probe
/// synchronously. The bool is whether the value came from the cache — the
/// report says which, because "no cache file" is the evidence that the probe
/// has never once succeeded. `None` when neither cache nor probe yields
/// anything.
fn cached_login_shell_path() -> (Option<String>, bool) {
    if let Some(cached) = read_cached_shell_path() {
        std::thread::spawn(|| {
            let probe = login_shell_probe();
            if let Some(fresh) = &probe.path {
                write_cached_shell_path(fresh);
            }
            finish_harvest(probe.env);
        });
        return (Some(cached), true);
    }
    let probe = login_shell_probe();
    if let Some(p) = &probe.path {
        write_cached_shell_path(p);
    }
    finish_harvest(probe.env);
    (probe.path, false)
}

/// Directories where CLI tools (node, agent CLIs, language version managers)
/// are commonly installed but which a Finder-launched `.app` does not inherit.
/// Only the probe can find a directory this list has never heard of; these are
/// the floor under it, so a machine whose probe fails still gets the common
/// cases.
fn common_bin_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/local/bin"),
        home.join(".cargo/bin"),
        home.join(".local/bin"),
        home.join(".bun/bin"),
        // Observed 2026-08-26: `pi` installed with `npm install -g` on a
        // machine whose only node is the one the Hermes installer brings.
        // Both `pi` and the `node` its `#!/usr/bin/env node` shim needs live
        // here and nowhere else, so finding the binary without this directory
        // on PATH still leaves it unlaunchable.
        home.join(".hermes/node/bin"),
        // pnpm's default macOS global bin (PNPM_HOME), on the same machine.
        home.join("Library/pnpm"),
        home.join("Library/pnpm/bin"),
    ]
}

/// Append each `extra` dir that `exists` to `current`, skipping duplicates.
/// Pure so it can be unit-tested without touching the filesystem; the caller
/// supplies the existence check. Existing entries keep priority (appended, not
/// prepended) so we never shadow the system tools already on PATH.
fn merge_path(current: &str, extra: &[PathBuf], exists: impl Fn(&Path) -> bool) -> String {
    let mut entries: Vec<String> =
        current.split(':').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
    for dir in extra {
        if !exists(dir) {
            continue;
        }
        let s = dir.to_string_lossy().to_string();
        if !entries.iter().any(|e| e == &s) {
            entries.push(s);
        }
    }
    entries.join(":")
}

/// Directories adopted after startup, once something installed into them.
///
/// Observed 2026-09-10: an npm agent installed from onboarding on a Linux
/// machine whose node came from apt has to go under `~/.local` (the default
/// prefix is root-owned, see `tools::install_script`), and `~/.local/bin` did
/// not exist when the app started, so [`repair`] had nothing to append. The
/// tile then said "Not installed" about a binary that was right there, and a
/// spawn would have failed the same way. Rather than mutate the process
/// environment from a worker thread (a `setenv` racing another thread's
/// `getenv` is undefined behaviour), the late arrivals are kept here and
/// merged in by [`effective_path`], which every PATH lookup and every agent
/// spawn reads.
static ADOPTED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// The process PATH plus every directory adopted since startup.
pub fn effective_path() -> String {
    let current = std::env::var("PATH").unwrap_or_default();
    let adopted = ADOPTED.lock().unwrap();
    merge_path(&current, &adopted, |p| p.exists())
}

/// Re-check the common tool directories and adopt the ones that now exist.
/// Called after an install job finishes. Returns what was newly adopted, for
/// the log.
pub fn adopt_new_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    let on_path: Vec<PathBuf> = path_dirs(&effective_path());
    let mut adopted = ADOPTED.lock().unwrap();
    let mut new = Vec::new();
    for dir in common_bin_dirs(&home) {
        if dir.exists() && !on_path.contains(&dir) && !adopted.contains(&dir) {
            adopted.push(dir.clone());
            new.push(dir);
        }
    }
    new
}

/// Locale env vars to default when a Finder-launched bundle inherits none. Returns
/// the (key, value) pairs to set, or empty if a locale is already present. Without
/// a UTF-8 locale, terminal programs negotiate the ASCII charset and strip
/// box-drawing/block glyphs (Claude's quadrant-block logo) to blanks before
/// they reach the webview. Pure for testability; caller checks the environment.
fn locale_defaults(has_locale: bool) -> Vec<(&'static str, &'static str)> {
    if has_locale {
        vec![]
    } else {
        vec![("LANG", "en_US.UTF-8"), ("LC_CTYPE", "en_US.UTF-8")]
    }
}

/// One line saying where the PATH came from, for the log. Pure; `repair` feeds
/// it what actually happened.
fn report_line(shell: Option<&str>, probed: Option<&str>, cached: bool, merged: &str) -> String {
    let source = match (probed, cached) {
        (Some(p), true) => format!("cached login-shell PATH ({} dirs)", path_dirs(p).len()),
        (Some(p), false) => format!("login-shell probe ({} dirs)", path_dirs(p).len()),
        (None, _) => format!(
            "login-shell probe returned nothing (SHELL={}); falling back to the built-in dirs",
            shell.unwrap_or("unset")
        ),
    };
    format!("PATH repair: {source}; PATH={merged}")
}

/// Repair the current process environment so spawned children (the `agency-termd`
/// daemon and agent CLIs) behave as they do under a normal shell launch.
/// A Finder-launched bundle inherits launchd's minimal env — no Homebrew PATH and
/// no locale. Idempotent; children inherit the repaired env.
///
/// Called before the app builds anything, so the daemon it later spawns
/// inherits the repaired PATH — and so this is the only thread running when the
/// process environment is mutated.
pub fn repair() {
    // So [`harvested`] can wait for the background probe on a PATH cache hit
    // instead of treating an uninitialised slot as "no key".
    harvest_slot();
    let current = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    // Prefer the user's real login-shell PATH (picks up nvm/volta/fnm/asdf and
    // npm global prefixes), then fall back to the hardcoded common dirs for
    // anything the probe couldn't supply. `merge_path` dedupes and keeps the
    // existing (system) dirs at the front.
    let (probed, cached) = cached_login_shell_path();
    let mut extra = probed.as_deref().map(path_dirs).unwrap_or_default();
    extra.extend(common_bin_dirs(&home));
    let merged = merge_path(&current, &extra, |p| p.exists());
    std::env::set_var("PATH", &merged);

    let shell = std::env::var("SHELL").ok();
    *REPORT.write().unwrap() = report_line(shell.as_deref(), probed.as_deref(), cached, &merged);

    let has_locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()));
    for (k, v) in locale_defaults(has_locale) {
        std::env::set_var(k, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn exists_set(dirs: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<String> = dirs.iter().map(|s| s.to_string()).collect();
        move |p: &Path| set.contains(&p.to_string_lossy().to_string())
    }

    #[test]
    fn appends_existing_homebrew_dir_under_minimal_launchd_path() {
        // The exact scenario: Finder-launched bundle gets launchd's stripped PATH,
        // spawned binaries live in /opt/homebrew/bin.
        let out = merge_path(
            "/usr/bin:/bin:/usr/sbin:/sbin",
            &common_bin_dirs(Path::new("/Users/x")),
            exists_set(&["/opt/homebrew/bin"]),
        );
        assert!(
            out.split(':').any(|e| e == "/opt/homebrew/bin"),
            "homebrew bin must be on PATH so spawned binaries resolve, got: {out}"
        );
        assert!(out.starts_with("/usr/bin:/bin:/usr/sbin:/sbin"), "system dirs keep priority");
    }

    #[test]
    fn skips_dirs_that_do_not_exist() {
        let out = merge_path(
            "/usr/bin",
            &common_bin_dirs(Path::new("/Users/x")),
            exists_set(&[]), // nothing exists
        );
        assert_eq!(out, "/usr/bin");
    }

    #[test]
    fn defaults_utf8_locale_only_when_absent() {
        assert_eq!(
            locale_defaults(false),
            vec![("LANG", "en_US.UTF-8"), ("LC_CTYPE", "en_US.UTF-8")],
            "a bundle with no locale must get a UTF-8 default so terminals keep block glyphs"
        );
        assert!(locale_defaults(true).is_empty(), "an existing locale is left untouched");
    }

    #[test]
    fn appends_login_shell_dirs_a_version_manager_installs_to() {
        // The Pi-not-recognized scenario: `pi` was installed globally under nvm,
        // whose bin dir the login shell exports but a Finder launch never sees.
        let nvm = PathBuf::from("/Users/x/.nvm/versions/node/v20.11.0/bin");
        let mut extra = path_dirs("/opt/homebrew/bin:/Users/x/.nvm/versions/node/v20.11.0/bin");
        extra.extend(common_bin_dirs(Path::new("/Users/x")));
        let out = merge_path(
            "/usr/bin:/bin",
            &extra,
            exists_set(&["/opt/homebrew/bin", &nvm.to_string_lossy()]),
        );
        assert!(
            out.split(':').any(|e| Path::new(e) == nvm),
            "the login-shell nvm bin must land on PATH so `pi` resolves, got: {out}"
        );
        assert!(out.starts_with("/usr/bin:/bin"), "system dirs keep priority");
    }

    #[test]
    fn a_vendored_node_prefix_survives_a_failed_probe() {
        // Observed 2026-08-26: `pi` and the only `node` on the machine both
        // live in ~/.hermes/node/bin. When the probe comes back empty this
        // list is all that is left, so it has to carry that directory.
        let hermes = PathBuf::from("/Users/x/.hermes/node/bin");
        let out = merge_path(
            "/usr/bin:/bin:/usr/sbin:/sbin",
            &common_bin_dirs(Path::new("/Users/x")),
            exists_set(&[&hermes.to_string_lossy()]),
        );
        assert!(
            out.split(':').any(|e| Path::new(e) == hermes),
            "a vendored node prefix must be on PATH, got: {out}"
        );
    }

    #[test]
    fn path_dirs_ignores_empty_segments() {
        assert_eq!(path_dirs("/a::/b:"), vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn does_not_duplicate_already_present_dirs() {
        let out = merge_path(
            "/usr/bin:/opt/homebrew/bin",
            &common_bin_dirs(Path::new("/Users/x")),
            exists_set(&["/opt/homebrew/bin", "/usr/local/bin"]),
        );
        assert_eq!(out.matches("/opt/homebrew/bin").count(), 1);
        assert!(out.split(':').any(|e| e == "/usr/local/bin"));
    }

    #[test]
    fn reads_the_path_out_of_a_chatty_profile() {
        // A profile that prints a banner writes to the same stdout as the
        // answer. Without the marker the banner is glued to the first entry,
        // which then fails the exists check and is silently dropped.
        let out = parse_marked_path(&format!(
            "        _\n     ,-' `-.\n  hello, nick\n\n{PATH_MARKER}/usr/bin:/Users/x/.hermes/node/bin\n"
        ));
        assert_eq!(out.as_deref(), Some("/usr/bin:/Users/x/.hermes/node/bin"));
    }

    #[test]
    fn takes_the_last_marked_line_and_rejects_an_unmarked_probe() {
        assert_eq!(
            parse_marked_path(&format!("{PATH_MARKER}/first\ntalk\n{PATH_MARKER}/second\n"))
                .as_deref(),
            Some("/second")
        );
        assert_eq!(parse_marked_path("/usr/bin:/bin\n"), None, "unmarked output is not a PATH");
        assert_eq!(parse_marked_path(""), None);
        assert_eq!(
            parse_marked_path(&format!("{PATH_MARKER}\n")),
            None,
            "an empty PATH is nothing"
        );
    }

    #[test]
    fn reads_allowlisted_env_out_of_the_same_probe() {
        let stdout = format!(
            "hello\n{PATH_MARKER}/usr/bin\n{ENV_MARKER}DEEPSEEK_API_KEY=sk-test\n{ENV_MARKER}HOME=/tmp\n{ENV_MARKER}DEEPSEEK_API_KEY=\n"
        );
        let env = parse_marked_env(&stdout);
        assert_eq!(env.get("DEEPSEEK_API_KEY").map(String::as_str), Some("sk-test"));
        assert!(!env.contains_key("HOME"), "unknown names must not be harvested");
        assert_eq!(parse_probe(&stdout).path.as_deref(), Some("/usr/bin"));
    }

    #[test]
    fn the_report_names_a_probe_that_returned_nothing() {
        // The line that would have answered "why doesn't Agency see pi" without
        // a strings(1) of the bundle.
        let line = report_line(Some("/bin/zsh"), None, false, "/usr/bin:/bin");
        assert!(line.contains("returned nothing"), "{line}");
        assert!(line.contains("/bin/zsh"), "the shell we asked must be in the line: {line}");
        assert!(
            report_line(Some("/bin/zsh"), Some("/a:/b"), true, "/usr/bin:/a:/b").contains("cached"),
            "a cache hit must be distinguishable from a fresh probe"
        );
    }
}
