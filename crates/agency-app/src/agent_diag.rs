//! Judgment-free diagnostics for the agent CLIs on this machine (AGE-146).
//!
//! Three local facts per enabled agent, with no verdict attached to any of
//! them: where the profile's command resolves, what its `--version` prints,
//! and how it was installed. A stale agent CLI produces behaviour that looks
//! like an Agency bug, and the first support question is "what version are you
//! on"; these rows make the answer a screenshot. Deliberately not an
//! "out of date" check: that verdict needs a registry lookup Agency does not
//! make, and the vendors' own update banners already do it with real data.
//!
//! The install method is read off the canonicalized binary path, because the
//! path is the one place the truth is already written down: `npm install -g`
//! is wrong advice for a Homebrew formula and actively harmful for a
//! vendor-managed tree. The package or formula name in the offered command is
//! extracted from that same path rather than kept in a table that could
//! disagree with it.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How the binary on PATH got there, read off its canonicalized path.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "method", rename_all = "lowercase")]
pub enum InstallMethod {
    /// An npm global: the path runs through `node_modules/<package>/`.
    Npm { package: String },
    /// A Homebrew formula: the path runs through `Cellar/<formula>/`.
    Homebrew { formula: String },
    /// A vendor's own installer, recognised by the exact directories the
    /// observed installers keep their versioned trees in.
    Vendor,
    /// Anything else. No update command is offered for it: a guessed one is
    /// how a working install gets a second, conflicting copy.
    Unknown,
}

/// One Settings diagnostics row (serialized to the UI).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCliInfo {
    /// Profile name ("claude", "cursor", or a custom profile's own name).
    pub agent: String,
    /// Where the profile's command resolved, before following symlinks.
    /// `None` when it is not on PATH at all.
    pub path: Option<String>,
    /// First line of the CLI's own `--version` output, as printed.
    pub version: Option<String>,
    pub install: InstallMethod,
}

/// Classify how the binary at `canonical` (symlinks already followed) was
/// installed. Pure; all the judgment in this module lives here.
pub fn classify(canonical: &Path, home: &Path) -> InstallMethod {
    let comps: Vec<&str> = canonical.iter().filter_map(|c| c.to_str()).collect();

    // Homebrew before npm: a formula can wrap an npm package, and then both
    // markers are in the path. Observed: /opt/homebrew/bin/opencode ->
    // /opt/homebrew/Cellar/opencode/1.17.10/libexec/lib/node_modules/opencode-ai/…
    // is brew-managed, and `npm install -g opencode-ai` would fight the
    // formula for the bin link. The converse (an npm global under brew's
    // /opt/homebrew/lib prefix, e.g. pi here) has no Cellar component.
    if let Some(i) = comps.iter().position(|c| *c == "Cellar") {
        if let Some(formula) = comps.get(i + 1) {
            return InstallMethod::Homebrew { formula: (*formula).to_string() };
        }
    }

    // pnpm's global store also runs through node_modules/, but npm cannot
    // update it: `npm install -g` there installs a second copy beside pnpm's.
    if comps.iter().any(|c| *c == "pnpm" || *c == ".pnpm") {
        return InstallMethod::Unknown;
    }

    if let Some(i) = comps.iter().position(|c| *c == "node_modules") {
        match (comps.get(i + 1), comps.get(i + 2)) {
            (Some(scope), Some(name)) if scope.starts_with('@') => {
                return InstallMethod::Npm { package: format!("{scope}/{name}") };
            }
            (Some(name), _) => return InstallMethod::Npm { package: (*name).to_string() },
            _ => {}
        }
    }

    // Vendor installers, matched on the exact directories the observed
    // installs keep their versioned trees in, not on "any dot-directory" —
    // which would also match version-manager shims and hand-built binaries:
    // - claude 2.1.241:  ~/.local/bin/claude -> ~/.local/share/claude/versions/2.1.241
    //   (~/.claude/local is where its installer kept versions before .local/share)
    // - cursor-agent 2026.08.11: ~/.local/bin/cursor-agent
    //     -> ~/.local/share/cursor-agent/versions/2026.08.11-e8db854/cursor-agent
    for dir in [".local/share/claude", ".claude", ".local/share/cursor-agent"] {
        if canonical.starts_with(home.join(dir)) {
            return InstallMethod::Vendor;
        }
    }

    InstallMethod::Unknown
}

/// First non-empty line of a `--version` run's stdout, ANSI-stripped. `None`
/// over 100 characters: every observed version line is short ("2.1.241
/// (Claude Code)", "crush version v0.51.2"), and a long first line is a CLI
/// talking about something else, which must not be shown as a version.
pub fn version_line(stdout: &str) -> Option<String> {
    let line = stdout
        .lines()
        .map(|l| crate::model_probe::strip_ansi(l).trim().to_string())
        .find(|l| !l.is_empty())?;
    (line.len() <= 100).then_some(line)
}

/// Where `command` runs from: an explicit path as itself, a bare name walked
/// along PATH (which pathenv::repair() has already fixed up for Finder
/// launches). Also backs `command_on_path` in state.rs.
pub fn resolve_on_path(command: &str) -> Option<PathBuf> {
    fn executable(p: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            p.is_file()
                && p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            p.is_file()
        }
    }
    if command.contains('/') {
        let p = Path::new(command);
        return executable(p).then(|| p.to_path_buf());
    }
    // `effective_path`, not `$PATH`: a directory an install created after
    // startup is on the former and not the latter (see pathenv::adopt_new_dirs).
    let paths = crate::pathenv::effective_path();
    std::env::split_paths(&paths).map(|dir| dir.join(command)).find(|p| executable(p))
}

/// How long a `--version` run gets before it is killed. The node-backed CLIs
/// take ~1s cold; the margin is for a slow disk, not for network (none of
/// these reaches one to print a version).
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// `<binary> --version`, killed at the deadline. The flag is not in the
/// catalog's verified-recipe class because its failure is contained: a CLI
/// that does not know it exits non-zero and the row shows no version, unlike
/// a wrong launch flag, which kills a user's run. stdin is closed so a CLI
/// that ignores the flag and waits for input exits instead of hanging; the
/// deadline catches one that draws a TUI anyway. Observed failure handled
/// here: hermes's broken wrapper exits 127 ("venv/bin/hermes: No such file or
/// directory"), which must read as "no version", not as a version.
pub fn probe_version(binary: &Path) -> Option<String> {
    let mut child = Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let out = crate::model_probe::drain(child.stdout.take());
    let deadline = Instant::now() + VERSION_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => return None,
        }
    };
    let stdout = out.join().unwrap_or_default();
    if !status.success() {
        return None;
    }
    version_line(&stdout)
}

/// The Settings diagnostics rows: one per profile, in the given order, probed
/// in parallel because the version runs dominate the wall clock.
pub fn diagnose(profiles: &[(String, String)], home: &Path) -> Vec<AgentCliInfo> {
    std::thread::scope(|s| {
        let handles: Vec<_> = profiles
            .iter()
            .map(|(agent, command)| {
                s.spawn(move || {
                    let resolved = resolve_on_path(command);
                    let install = match &resolved {
                        Some(p) => classify(&p.canonicalize().unwrap_or_else(|_| p.clone()), home),
                        None => InstallMethod::Unknown,
                    };
                    AgentCliInfo {
                        agent: agent.clone(),
                        version: resolved.as_deref().and_then(probe_version),
                        path: resolved.map(|p| p.display().to_string()),
                        install,
                    }
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("diagnose worker panicked")).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> &'static Path {
        Path::new("<home>")
    }

    fn npm(package: &str) -> InstallMethod {
        InstallMethod::Npm { package: package.into() }
    }

    /// Real paths observed on 2026-08-23; each test names its install.
    #[test]
    fn vendor_installers_are_recognised_by_their_versioned_trees() {
        // claude 2.1.241 and cursor-agent 2026.08.11, both via their own curl installers.
        for p in [
            "<home>/.local/share/claude/versions/2.1.241",
            "<home>/.local/share/cursor-agent/versions/2026.08.11-e8db854/cursor-agent",
            "<home>/.claude/local/claude",
        ] {
            assert_eq!(classify(Path::new(p), home()), InstallMethod::Vendor, "{p}");
        }
    }

    #[test]
    fn npm_global_under_the_homebrew_prefix_is_npm_not_homebrew() {
        // pi installed with npm whose node came from brew: /opt/homebrew is
        // brew's prefix, but there is no Cellar component, and brew knows
        // nothing about this tree.
        assert_eq!(
            classify(
                Path::new(
                    "/opt/homebrew/lib/node_modules/@earendil-works/pi-coding-agent/dist/cli.js"
                ),
                home()
            ),
            npm("@earendil-works/pi-coding-agent")
        );
    }

    #[test]
    fn brew_formula_wrapping_an_npm_package_is_homebrew() {
        // opencode 1.17.10 via brew: the formula vendors the npm package, so
        // the path carries both markers and the outer manager must win —
        // `npm install -g opencode-ai` here would fight the formula.
        assert_eq!(
            classify(
                Path::new(
                    "/opt/homebrew/Cellar/opencode/1.17.10/libexec/lib/node_modules/opencode-ai/bin/opencode.exe"
                ),
                home()
            ),
            InstallMethod::Homebrew { formula: "opencode".into() }
        );
        // crush 0.51.2, a plain formula.
        assert_eq!(
            classify(Path::new("/opt/homebrew/Cellar/crush/0.51.2/bin/crush"), home()),
            InstallMethod::Homebrew { formula: "crush".into() }
        );
    }

    #[test]
    fn unscoped_and_version_managed_npm_globals() {
        assert_eq!(
            classify(Path::new("/usr/local/lib/node_modules/opencode-ai/bin/opencode"), home()),
            npm("opencode-ai")
        );
        // nvm keeps globals under a dot-directory; node_modules still decides.
        assert_eq!(
            classify(
                Path::new(
                    "<home>/.nvm/versions/node/v22.1.0/lib/node_modules/@anthropic-ai/claude-code/cli.js"
                ),
                home()
            ),
            npm("@anthropic-ai/claude-code")
        );
    }

    #[test]
    fn pnpm_managed_globals_get_no_npm_advice() {
        // `npm install -g` against pnpm's store installs a second copy beside
        // it rather than updating it, so these must stay Unknown.
        for p in [
            "<home>/Library/pnpm/global/5/node_modules/opencode-ai/bin/opencode",
            "<home>/x/node_modules/.pnpm/foo@1.0.0/node_modules/foo/cli.js",
        ] {
            assert_eq!(classify(Path::new(p), home()), InstallMethod::Unknown, "{p}");
        }
    }

    #[test]
    fn hand_placed_binaries_are_unknown() {
        // hermes here: a wrapper script sitting directly in ~/.local/bin, no
        // installer tree behind it. Offering any update command for it would
        // be a guess.
        assert_eq!(classify(Path::new("<home>/.local/bin/hermes"), home()), InstallMethod::Unknown);
        assert_eq!(classify(Path::new("<home>/bin/mytool"), home()), InstallMethod::Unknown);
        assert_eq!(classify(Path::new("/usr/local/bin/something"), home()), InstallMethod::Unknown);
    }

    #[test]
    fn a_bare_node_modules_tail_names_no_package() {
        assert_eq!(classify(Path::new("/x/node_modules"), home()), InstallMethod::Unknown);
    }

    #[test]
    fn version_line_takes_the_first_nonempty_line_as_printed() {
        // Observed shapes, 2026-08-23.
        assert_eq!(version_line("2.1.241 (Claude Code)\n"), Some("2.1.241 (Claude Code)".into()));
        assert_eq!(version_line("crush version v0.51.2\n"), Some("crush version v0.51.2".into()));
        assert_eq!(version_line("\n\n  1.17.10  \n"), Some("1.17.10".into()));
        assert_eq!(version_line("\x1b[1m0.80.3\x1b[0m\n"), Some("0.80.3".into()));
        assert_eq!(version_line(""), None);
        assert_eq!(version_line("   \n \n"), None);
        // A CLI talking, not versioning.
        assert_eq!(version_line(&"y".repeat(200)), None);
    }

    #[test]
    fn resolve_on_path_finds_binaries_like_command_on_path_does() {
        assert_eq!(resolve_on_path("/bin/sh"), Some(PathBuf::from("/bin/sh")));
        assert!(resolve_on_path("sh").is_some());
        assert_eq!(resolve_on_path("definitely-not-a-real-binary-4k2x"), None);
        assert_eq!(resolve_on_path("/nonexistent/path/to/agent"), None);
    }
}
