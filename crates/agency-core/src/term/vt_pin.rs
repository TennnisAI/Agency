//! One recorded version for each implementation that parses VT bytes here, and
//! tests that fail when the build stops matching it.
//!
//! Two things parse the same escape sequences in Agency: `alacritty_terminal` in
//! the daemon, which owns the grid a reattach snapshot is drawn from, and
//! xterm.js in the pane, which owns what the user actually sees. They agree only
//! as far as they read the specification the same way, and where they differ the
//! difference is invisible while a pane is live, because the pane is the only
//! thing drawing. It surfaces on reattach and nowhere else. That is the shape of
//! every emulator bug this code has had: the AGE-67 staircase (bare LF), the
//! double-spaced rows (trailing padding against a narrower client), the cursor
//! stranded at the bottom (a trailing CRLF on a full screen). Each was these two
//! disagreeing, and each cost an investigation to find because the symptom
//! appeared a switch-away-and-back after the cause.
//!
//! Two implementations of one specification will drift; the only question is
//! whether anything notices. So each side is pinned to an exact version rather
//! than a range, and the tests below check the pin against what was actually
//! built: the version string linked into this test binary, and the versions the
//! two lockfiles resolved. A parser that moves underneath us then fails a test
//! instead of producing a subtly wrong pane, and moving one deliberately is an
//! edit here plus a run of the reattach tests in [`super::emulator`], which are
//! the only place the two sides' agreement is written down.
//!
//! The discipline is borrowed from another project with the same split across a
//! wider gap — a native emulator on one target and a WebAssembly build of it on
//! another, both built from a single pinned revision, with a test that reads the
//! build metadata back out of the compiled artifact. We are not adopting their
//! emulator, only this. If the emulator question is ever genuinely reopened, "one
//! implementation, two targets" is the shape to reopen it toward: it is the only
//! arrangement in which the two sides cannot disagree at all.
//!
//! Reopened, on the roadmap as AGE-174: libghostty now ships both halves, a
//! zero-dependency C API for the daemon and a WebAssembly build with an
//! xterm.js-compatible API for the pane, from one source. It is blocked on
//! upstream declaring the API stable — it currently warns that breaking changes
//! are expected — so the pins below are still the mitigation, not a stale one.
//! Everything the swap has to preserve is the reattach tests in
//! [`super::emulator`].

/// The VT parser in the daemon. Pinned in `crates/agency-core/Cargo.toml`.
pub const ALACRITTY_TERMINAL: &str = "0.26.0";

/// The VT parser in the pane. Pinned in `ui/package.json`.
pub const XTERM_JS: &str = "5.5.0";

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn repo_root() -> PathBuf {
        // crates/agency-core -> crates -> the repo.
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf()
    }

    fn read(rel: &str) -> String {
        let path = repo_root().join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// Every version of crate `name` named by a source path inside `haystack`,
    /// deduplicated.
    ///
    /// The trailing `/` is what makes this a path component rather than any
    /// mention of the name: without it the scan also finds the needle this
    /// function passes in, which lives in the same binary as a string literal.
    fn linked_versions(haystack: &[u8], name: &str) -> Vec<String> {
        let needle = format!("{name}-").into_bytes();
        let mut out: Vec<String> = Vec::new();
        for (i, w) in haystack.windows(needle.len()).enumerate() {
            if w != needle {
                continue;
            }
            let rest = &haystack[i + needle.len()..];
            let end =
                rest.iter().position(|b| !(b.is_ascii_digit() || *b == b'.')).unwrap_or(rest.len());
            if rest.get(end) != Some(&b'/') {
                continue;
            }
            let v = String::from_utf8_lossy(&rest[..end]).to_string();
            if v.matches('.').count() == 2 && !out.contains(&v) {
                out.push(v);
            }
        }
        out
    }

    /// The `version` of `name` in a Cargo.lock.
    fn locked_crate(lock: &str, name: &str) -> Option<String> {
        lock.split("[[package]]")
            .find(|block| block.contains(&format!("name = \"{name}\"")))?
            .lines()
            .find_map(|l| l.trim().strip_prefix("version = \"")?.strip_suffix('"'))
            .map(str::to_string)
    }

    /// The `specifier`/`version` pair a pnpm lockfile's importer recorded for
    /// `name`, e.g. `("5.5.0", "5.5.0")`.
    fn locked_package(lock: &str, name: &str) -> Option<(String, String)> {
        let lines: Vec<&str> = lock.lines().collect();
        let at = lines.iter().position(|l| l.trim() == format!("'{name}':"))?;
        let field = |i: usize, key: &str| -> Option<String> {
            Some(lines.get(i)?.trim().strip_prefix(key)?.trim().to_string())
        };
        Some((field(at + 1, "specifier:")?, field(at + 2, "version:")?))
    }

    #[test]
    fn the_emulator_linked_into_this_binary_is_the_pinned_one() {
        // Read the version back out of the artifact, not off a manifest: a
        // lockfile records what cargo resolved, this records what is actually
        // linked into the binary running the reattach tests. rustc embeds the
        // package directory name of every crate it takes a panic location from,
        // so `alacritty_terminal-<version>` appears in any build that touches the
        // grid at all, and `-Ztrim-paths` rewrites the prefix but keeps this
        // component.
        let exe = std::env::current_exe().expect("current_exe");
        let bytes = std::fs::read(&exe).unwrap_or_else(|e| panic!("read {}: {e}", exe.display()));
        let found = linked_versions(&bytes, "alacritty_terminal");
        assert!(
            !found.is_empty(),
            "no alacritty_terminal version string in {}: the build stopped embedding crate paths, \
             so this test no longer checks anything. Fix the check or delete it on purpose.",
            exe.display(),
        );
        assert_eq!(
            found,
            vec![ALACRITTY_TERMINAL.to_string()],
            "the linked emulator is not the pinned one. Rebuild, or move the pin here and re-run \
             the reattach tests in term::emulator.",
        );
    }

    #[test]
    fn the_daemon_parser_is_pinned_to_the_recorded_version() {
        let manifest = read("crates/agency-core/Cargo.toml");
        assert!(
            manifest.contains(&format!("alacritty_terminal = \"={ALACRITTY_TERMINAL}\"")),
            "Cargo.toml must pin alacritty_terminal exactly (=x.y.z), not to a range",
        );
        assert_eq!(
            locked_crate(&read("Cargo.lock"), "alacritty_terminal").as_deref(),
            Some(ALACRITTY_TERMINAL),
        );
    }

    #[test]
    fn the_client_parser_is_pinned_to_the_recorded_version() {
        let pkg = read("ui/package.json");
        assert!(
            pkg.contains(&format!("\"@xterm/xterm\": \"{XTERM_JS}\"")),
            "ui/package.json must pin @xterm/xterm exactly, not with a caret",
        );
        assert_eq!(
            locked_package(&read("ui/pnpm-lock.yaml"), "@xterm/xterm"),
            Some((XTERM_JS.to_string(), XTERM_JS.to_string())),
        );
    }
}
