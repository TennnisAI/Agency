//! PATH repair for GUI launch.
//!
//! When the app is launched from Finder/Dock as a macOS `.app` bundle it inherits
//! launchd's minimal default PATH (`/usr/bin:/bin:/usr/sbin:/sbin`). That excludes
//! Homebrew (`/opt/homebrew/bin`, `/usr/local/bin`) and other common locations, so
//! child processes we spawn — `tmux`, the agent CLIs it launches — fail to resolve
//! with a bare `ENOENT` ("No such file or directory (os error 2)"). `git` only
//! survives because macOS ships it in `/usr/bin`.
//!
//! [`repair`] appends the common tool directories that exist on disk to the current
//! process PATH, which child `Command`s inherit.

use std::path::{Path, PathBuf};

/// Directories where CLI tools (tmux, node, agent CLIs, language version managers)
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
/// a UTF-8 locale the `tmux attach` client negotiates the ASCII charset, so tmux
/// strips box-drawing/block glyphs (Claude's quadrant-block logo) to blanks before
/// they reach the webview. Pure for testability; caller checks the environment.
fn locale_defaults(has_locale: bool) -> Vec<(&'static str, &'static str)> {
    if has_locale {
        vec![]
    } else {
        vec![("LANG", "en_US.UTF-8"), ("LC_CTYPE", "en_US.UTF-8")]
    }
}

/// Repair the current process environment so spawned children (tmux, the `tmux
/// attach` client, agent CLIs) behave as they do under a normal shell launch.
/// A Finder-launched bundle inherits launchd's minimal env — no Homebrew PATH and
/// no locale. Idempotent; children inherit the repaired env.
pub fn repair() {
    let current = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    let merged = merge_path(&current, &common_bin_dirs(&home), |p| p.exists());
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
        // tmux lives in /opt/homebrew/bin.
        let out = merge_path(
            "/usr/bin:/bin:/usr/sbin:/sbin",
            &common_bin_dirs(Path::new("/Users/x")),
            exists_set(&["/opt/homebrew/bin"]),
        );
        assert!(
            out.split(':').any(|e| e == "/opt/homebrew/bin"),
            "homebrew bin must be on PATH so tmux resolves, got: {out}"
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
            "a bundle with no locale must get a UTF-8 default so tmux keeps block glyphs"
        );
        assert!(locale_defaults(true).is_empty(), "an existing locale is left untouched");
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
