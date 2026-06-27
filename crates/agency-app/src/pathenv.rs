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

/// Repair the current process PATH so spawned children (tmux, agent CLIs) resolve
/// even under a Finder-launched bundle's minimal PATH. Idempotent.
pub fn repair() {
    let current = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    let merged = merge_path(&current, &common_bin_dirs(&home), |p| p.exists());
    std::env::set_var("PATH", merged);
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
