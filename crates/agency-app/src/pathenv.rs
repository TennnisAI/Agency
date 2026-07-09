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

use std::path::{Path, PathBuf};

/// Ask the user's login shell for its `PATH`. A Finder-launched bundle inherits
/// launchd's minimal `PATH`, so the only reliable way to learn where the user's
/// tools actually live (nvm/volta/fnm/asdf shims, npm global prefixes, custom
/// dirs added in `.zprofile`/`.bash_profile`) is to run their login shell and
/// read the `PATH` it composes. Returns `None` if `SHELL` is unset, the probe
/// fails, times out, or yields nothing — callers fall back to [`common_bin_dirs`].
fn login_shell_path() -> Option<String> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty())?;
    let mut child = Command::new(&shell)
        .args(["-lc", "printf %s \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Poll rather than block: a misconfigured profile could hang the shell, and
    // this runs on the startup path. Give it a short budget, then kill.
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }

    use std::io::Read;
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let out = out.trim().to_string();
    (!out.is_empty()).then_some(out)
}

/// Split a `PATH`-style string into directory entries.
fn path_dirs(path: &str) -> Vec<PathBuf> {
    path.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect()
}

/// File where the last successful [`login_shell_path`] result is cached, so we
/// don't pay the (up to 3s) shell probe on the startup critical path every
/// launch. `None` if `HOME` is unset.
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
/// synchronously. `None` when neither cache nor probe yields anything.
fn cached_login_shell_path() -> Option<String> {
    if let Some(cached) = read_cached_shell_path() {
        std::thread::spawn(|| {
            if let Some(fresh) = login_shell_path() {
                write_cached_shell_path(&fresh);
            }
        });
        return Some(cached);
    }
    let probed = login_shell_path()?;
    write_cached_shell_path(&probed);
    Some(probed)
}

/// Directories where CLI tools (node, agent CLIs, language version managers)
/// are commonly installed but which a Finder-launched `.app` does not inherit.
fn common_bin_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/local/bin"),
        home.join(".cargo/bin"),
        home.join(".local/bin"),
        home.join(".bun/bin"),
    ]
}

/// Append each `extra` dir that `exists` to `current`, skipping duplicates.
/// Pure so it can be unit-tested without touching the filesystem; the caller
/// supplies the existence check. Existing entries keep priority (appended, not
/// prepended) so we never shadow the system tools already on PATH.
fn merge_path(current: &str, extra: &[PathBuf], exists: impl Fn(&Path) -> bool) -> String {
    let mut entries: Vec<String> = current
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
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

/// Repair the current process environment so spawned children (the `agency-termd`
/// daemon and agent CLIs) behave as they do under a normal shell launch.
/// A Finder-launched bundle inherits launchd's minimal env — no Homebrew PATH and
/// no locale. Idempotent; children inherit the repaired env.
pub fn repair() {
    let current = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    // Prefer the user's real login-shell PATH (picks up nvm/volta/fnm/asdf and
    // npm global prefixes), then fall back to the hardcoded common dirs for
    // anything the probe couldn't supply. `merge_path` dedupes and keeps the
    // existing (system) dirs at the front.
    let mut extra = cached_login_shell_path().map(|p| path_dirs(&p)).unwrap_or_default();
    extra.extend(common_bin_dirs(&home));
    let merged = merge_path(&current, &extra, |p| p.exists());
    std::env::set_var("PATH", merged);

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
}
