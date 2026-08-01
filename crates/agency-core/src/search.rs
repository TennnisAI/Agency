//! The app-wide content-search primitive (one-stop Phase 2). Files, Docs, the
//! palette, and (later) issues all consume this one entry point.
//!
//! Engine choice: shell out to `rg --json` when ripgrep is on PATH (the
//! process PATH is repaired at startup by `pathenv::repair`, the same
//! mechanism agent detection relies on), and fall back to a bounded Rust walk
//! + line scan so search works on machines without it. Both engines skip
//! hidden files (rg's default) and respect `.gitignore` in repo roots — rg
//! natively, the fallback by listing files through
//! `git ls-files --cached --others --exclude-standard`.
//!
//! Every code path is capped — total hits, hits per file, bytes per file,
//! total bytes scanned, and wall-clock — and hitting a cap returns what was
//! collected rather than erroring: search is best-effort and must never wedge
//! the (async) command that calls it.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Absolute ceiling on hits per search; `SearchQuery::max_hits` is clamped to it.
const MAX_HITS_CAP: usize = 500;
const DEFAULT_MAX_HITS: usize = 200;
/// One hit per matching line, at most this many lines per file (rg `--max-count`).
const PER_FILE_HITS: usize = 20;
/// Files larger than this are skipped entirely (matches rg `--max-filesize 2M`).
const MAX_FILE_BYTES: u64 = 2_000_000;
/// Fallback engine: stop after scanning this many bytes across all files.
const MAX_TOTAL_BYTES: u64 = 64_000_000;
/// Wall-clock budget for a whole search, either engine.
const TIMEOUT: Duration = Duration::from_secs(3);
/// Hit text is capped so a minified one-liner can't flood the IPC channel.
const MAX_TEXT_CHARS: usize = 500;
/// Fallback walk safety valve when a root has no git to bound the file list.
const MAX_WALK_FILES: usize = 50_000;

fn default_max_hits() -> usize {
    DEFAULT_MAX_HITS
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub query: String,
    /// Treat `query` as a regex; otherwise it is matched literally.
    #[serde(default)]
    pub regex: bool,
    /// Case-sensitive when true; the default is insensitive.
    #[serde(default)]
    pub case: bool,
    /// Gitignore-style globs ("*.md" matches at any depth); empty = all files.
    #[serde(default)]
    pub globs: Vec<String>,
    #[serde(default = "default_max_hits")]
    pub max_hits: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// Path relative to the searched root, `/`-separated.
    pub path: String,
    /// 1-based line number.
    pub line: u32,
    /// 1-based byte column of the first match on the line.
    pub col: u32,
    /// The matching line (trailing newline stripped, length-capped).
    pub text: String,
}

/// Search file contents under `root`. Returns at most the capped number of
/// hits; an empty query yields no hits. rg when available, Rust scan
/// otherwise; an rg *failure* (bad spawn, exit code 2) falls back too, so a
/// broken rg install can never take search down with it.
pub fn search_files(root: &Path, q: &SearchQuery) -> Result<Vec<SearchHit>> {
    if q.query.is_empty() {
        return Ok(Vec::new());
    }
    let deadline = Instant::now() + TIMEOUT;
    if let Some(rg) = rg_on_path() {
        if let Ok(hits) = search_rg(&rg, root, q, deadline) {
            return Ok(hits);
        }
    }
    search_scan(root, q, deadline)
}

/// Locate `rg` on the process PATH. `AGENCY_NO_RG=1` disables it (tests,
/// timing comparisons, emergency escape hatch).
fn rg_on_path() -> Option<PathBuf> {
    if std::env::var_os("AGENCY_NO_RG").is_some_and(|v| v == "1") {
        return None;
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("rg")).find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

// ── rg engine ──────────────────────────────────────────────────────────────

fn search_rg(rg: &Path, root: &Path, q: &SearchQuery, deadline: Instant) -> Result<Vec<SearchHit>> {
    let max_hits = q.max_hits.clamp(1, MAX_HITS_CAP);
    let mut cmd = Command::new(rg);
    cmd.current_dir(root)
        .args(["--json", "--no-config", "--max-filesize", "2M"])
        .args(["--max-count", &PER_FILE_HITS.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if !q.case {
        cmd.arg("-i");
    }
    if !q.regex {
        cmd.arg("--fixed-strings");
    }
    for g in &q.globs {
        cmd.arg("-g").arg(g);
    }
    cmd.arg("-e").arg(&q.query);

    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().expect("stdout was piped");
    // The read loop only notices the deadline between output lines, so an rg
    // that is busy but silent (a huge tree on a slow volume) would block the
    // read past the budget. A watchdog thread kills the child at the
    // deadline; the kill closes stdout, which unblocks the read.
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let child = Arc::new(Mutex::new(child));
    let finished = Arc::new(AtomicBool::new(false));
    let watchdog = {
        let (child, finished) = (Arc::clone(&child), Arc::clone(&finished));
        std::thread::spawn(move || {
            while !finished.load(Ordering::Relaxed) {
                if Instant::now() >= deadline {
                    let _ = child.lock().unwrap().kill();
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        })
    };
    let mut hits = Vec::new();
    let mut stopped_early = false;
    for line in std::io::BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if Instant::now() >= deadline || hits.len() >= max_hits {
            stopped_early = true;
            break;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if v["type"] != "match" {
            continue;
        }
        let d = &v["data"];
        let Some(path) = d["path"]["text"].as_str() else { continue };
        hits.push(SearchHit {
            path: path.trim_start_matches("./").replace('\\', "/"),
            line: d["line_number"].as_u64().unwrap_or(0) as u32,
            col: d["submatches"][0]["start"].as_u64().unwrap_or(0) as u32 + 1,
            text: cap_text(d["lines"]["text"].as_str().unwrap_or("")),
        });
    }
    finished.store(true, Ordering::Relaxed);
    // Kill rg if a cap/deadline ended the read mid-stream; harmless if it
    // already exited on its own (or the watchdog got there first).
    let status = {
        let mut child = child.lock().unwrap();
        let _ = child.kill();
        child.wait()?
    };
    let _ = watchdog.join();
    // Exit 0 = matches, 1 = no matches — both fine (a deadline kill exits by
    // signal, so `code()` is None and lands here too). Anything else with no
    // output means rg itself failed; error out so the caller falls back.
    if hits.is_empty() && !stopped_early {
        if let Some(code) = status.code() {
            if code != 0 && code != 1 {
                anyhow::bail!("rg exited with {code}");
            }
        }
    }
    Ok(hits)
}

// ── fallback engine (bounded walk + line scan) ─────────────────────────────

fn search_scan(root: &Path, q: &SearchQuery, deadline: Instant) -> Result<Vec<SearchHit>> {
    let max_hits = q.max_hits.clamp(1, MAX_HITS_CAP);
    let pattern = if q.regex { q.query.clone() } else { regex::escape(&q.query) };
    let pattern = if q.case { pattern } else { format!("(?i){pattern}") };
    let re = regex::Regex::new(&pattern)?;
    let globs = build_globset(&q.globs)?;

    let mut hits = Vec::new();
    let mut scanned: u64 = 0;
    for rel in list_files(root) {
        if hits.len() >= max_hits || scanned >= MAX_TOTAL_BYTES || Instant::now() >= deadline {
            break;
        }
        if let Some(gs) = &globs {
            if !gs.is_match(&rel) {
                continue;
            }
        }
        let abs = root.join(&rel);
        let Ok(meta) = abs.metadata() else { continue }; // deleted since listing
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&abs) else { continue };
        scanned += bytes.len() as u64;
        if bytes.contains(&0) {
            continue; // binary
        }
        let Ok(text) = String::from_utf8(bytes) else { continue };
        let mut in_file = 0;
        for (i, line) in text.lines().enumerate() {
            if in_file >= PER_FILE_HITS || hits.len() >= max_hits {
                break;
            }
            if let Some(m) = re.find(line) {
                hits.push(SearchHit {
                    path: rel.clone(),
                    line: i as u32 + 1,
                    col: m.start() as u32 + 1,
                    text: cap_text(line),
                });
                in_file += 1;
            }
        }
    }
    Ok(hits)
}

/// The files to scan, relative `/`-separated paths. In a git repo this is
/// tracked + untracked-not-ignored (so `.gitignore` is respected exactly like
/// rg does natively); elsewhere a bounded walk. Hidden files are dropped in
/// both cases to match rg's default.
fn list_files(root: &Path) -> Vec<String> {
    git_files(root).unwrap_or_else(|| walk_files(root))
}

/// Ceiling on names returned by [`list_root_files`], whatever the caller asks.
const MAX_LIST_FILES: usize = 20_000;

/// Sorted relative file paths under `root`, for quick-open. Same file set as
/// the fallback search engine: `git ls-files` semantics in a repo (gitignore
/// respected, untracked included), bounded walk elsewhere; hidden files
/// dropped either way. Truncated to `max` (itself capped) — quick-open is
/// best-effort name matching, not an exhaustive listing.
pub fn list_root_files(root: &Path, max: usize) -> Vec<String> {
    let mut files = list_files(root);
    files.truncate(max.min(MAX_LIST_FILES));
    files
}

fn git_files(root: &Path) -> Option<Vec<String>> {
    let out = Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut files: Vec<String> = out
        .stdout
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .filter(|p| !p.split('/').any(|c| c.starts_with('.')))
        .collect();
    files.sort();
    files.dedup(); // a file can be both cached and listed via --others during renames
    Some(files)
}

fn walk_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }
            let rel = if prefix.is_empty() { name } else { format!("{prefix}/{}", name) };
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push((entry.path(), rel));
            } else if ft.is_file() {
                if out.len() >= MAX_WALK_FILES {
                    return out;
                }
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

/// Gitignore-style glob set: a slash-less pattern ("*.md") matches at any
/// depth, so it is registered both bare (top level) and under `**/`.
fn build_globset(globs: &[String]) -> Result<Option<globset::GlobSet>> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut b = globset::GlobSetBuilder::new();
    for g in globs {
        b.add(globset::Glob::new(g)?);
        if !g.contains('/') {
            b.add(globset::Glob::new(&format!("**/{g}"))?);
        }
    }
    Ok(Some(b.build()?))
}

fn cap_text(s: &str) -> String {
    let t = s.trim_end_matches(['\r', '\n']);
    if t.len() <= MAX_TEXT_CHARS {
        return t.to_string();
    }
    t.chars().take(MAX_TEXT_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn q(query: &str) -> SearchQuery {
        SearchQuery {
            query: query.into(),
            regex: false,
            case: false,
            globs: vec![],
            max_hits: DEFAULT_MAX_HITS,
        }
    }

    fn far_deadline() -> Instant {
        Instant::now() + Duration::from_secs(30)
    }

    /// Fixture tree exercising every skip rule the engines share.
    fn fixture() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.md"), "hello world\nsecond line\nHELLO again\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "fn hello() {}\n").unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join(".hidden/secret.md"), "hello hidden\n").unwrap();
        std::fs::write(root.join("bin.dat"), b"hello\x00world").unwrap();
        dir
    }

    #[test]
    fn scan_finds_lines_with_one_based_positions() {
        let dir = fixture();
        let hits = search_scan(dir.path(), &q("hello"), far_deadline()).unwrap();
        let a: Vec<&SearchHit> = hits.iter().filter(|h| h.path == "a.md").collect();
        assert_eq!(a.len(), 2, "case-insensitive by default: {hits:?}");
        assert_eq!((a[0].line, a[0].col), (1, 1));
        assert_eq!(a[0].text, "hello world");
        assert_eq!(a[1].line, 3, "HELLO on line 3 matches insensitively");
        assert!(hits.iter().any(|h| h.path == "src/lib.rs"));
    }

    #[test]
    fn scan_case_sensitive_when_asked() {
        let dir = fixture();
        let mut query = q("HELLO");
        query.case = true;
        let hits = search_scan(dir.path(), &query, far_deadline()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].path.as_str(), hits[0].line), ("a.md", 3));
    }

    #[test]
    fn scan_regex_mode() {
        let dir = fixture();
        let mut query = q(r"fn \w+\(\)");
        query.regex = true;
        let hits = search_scan(dir.path(), &query, far_deadline()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "src/lib.rs");
        // A literal search for the same string finds nothing (proves the flag).
        query.regex = false;
        assert!(search_scan(dir.path(), &query, far_deadline()).unwrap().is_empty());
    }

    #[test]
    fn scan_slashless_glob_matches_any_depth() {
        let dir = fixture();
        std::fs::create_dir_all(dir.path().join("notes/deep")).unwrap();
        std::fs::write(dir.path().join("notes/deep/n.md"), "hello nested\n").unwrap();
        let mut query = q("hello");
        query.globs = vec!["*.md".into()];
        let hits = search_scan(dir.path(), &query, far_deadline()).unwrap();
        assert!(hits.iter().all(|h| h.path.ends_with(".md")), "{hits:?}");
        assert!(hits.iter().any(|h| h.path == "a.md"), "top-level .md matches");
        assert!(hits.iter().any(|h| h.path == "notes/deep/n.md"), "nested .md matches");
    }

    #[test]
    fn scan_skips_hidden_and_binary() {
        let dir = fixture();
        let hits = search_scan(dir.path(), &q("hello"), far_deadline()).unwrap();
        assert!(!hits.iter().any(|h| h.path.contains(".hidden")), "hidden dirs skipped");
        assert!(!hits.iter().any(|h| h.path == "bin.dat"), "NUL byte marks binary");
    }

    #[test]
    fn scan_honors_hit_caps() {
        let dir = tempdir().unwrap();
        let many = "match\n".repeat(100);
        std::fs::write(dir.path().join("big.txt"), &many).unwrap();
        std::fs::write(dir.path().join("second.txt"), &many).unwrap();
        // Per-file cap: one file contributes at most PER_FILE_HITS lines.
        let hits = search_scan(dir.path(), &q("match"), far_deadline()).unwrap();
        assert_eq!(hits.iter().filter(|h| h.path == "big.txt").count(), PER_FILE_HITS);
        // Total cap: max_hits clamps the whole result.
        let mut query = q("match");
        query.max_hits = 7;
        assert_eq!(search_scan(dir.path(), &query, far_deadline()).unwrap().len(), 7);
    }

    #[test]
    fn scan_respects_gitignore_in_repos_and_finds_untracked() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            assert!(Command::new("git").args(args).current_dir(root).output().unwrap().status.success());
        };
        git(&["init", "-q"]);
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.join("tracked.txt"), "hello tracked\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "hello ignored\n").unwrap();
        std::fs::write(root.join("untracked.txt"), "hello untracked\n").unwrap();
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        git(&["add", "tracked.txt"]);

        let paths: Vec<String> = search_scan(root, &q("hello"), far_deadline())
            .unwrap()
            .into_iter()
            .map(|h| h.path)
            .collect();
        assert!(paths.contains(&"tracked.txt".to_string()));
        assert!(paths.contains(&"untracked.txt".to_string()), "untracked-not-ignored is searchable");
        assert!(!paths.contains(&"ignored.txt".to_string()), "gitignored files are not");
    }

    #[test]
    fn list_root_files_sorted_skips_hidden_and_truncates() {
        let dir = fixture();
        let files = list_root_files(dir.path(), 100);
        assert_eq!(files, vec!["a.md", "bin.dat", "src/lib.rs"], "sorted, hidden dropped");
        assert_eq!(list_root_files(dir.path(), 2), vec!["a.md", "bin.dat"], "truncated to max");
    }

    #[test]
    fn list_root_files_respects_gitignore() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            assert!(Command::new("git").args(args).current_dir(root).output().unwrap().status.success());
        };
        git(&["init", "-q"]);
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.join("tracked.txt"), "t\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "i\n").unwrap();
        std::fs::write(root.join("untracked.txt"), "u\n").unwrap();
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        git(&["add", "tracked.txt"]);

        let files = list_root_files(root, 100);
        assert!(files.contains(&"tracked.txt".to_string()));
        assert!(files.contains(&"untracked.txt".to_string()));
        assert!(!files.contains(&"ignored.txt".to_string()));
    }

    #[test]
    fn empty_query_returns_nothing() {
        let dir = fixture();
        assert!(search_files(dir.path(), &q("")).unwrap().is_empty());
    }

    #[test]
    fn rg_matches_scan_on_the_fixture() {
        let Some(rg) = rg_on_path() else {
            eprintln!("rg not on PATH — parity test skipped");
            return;
        };
        let dir = fixture();
        std::fs::create_dir_all(dir.path().join("notes")).unwrap();
        std::fs::write(dir.path().join("notes/n.md"), "hello nested\n").unwrap();

        for (label, query) in [
            ("plain", q("hello")),
            ("glob", {
                let mut x = q("hello");
                x.globs = vec!["*.md".into()];
                x
            }),
        ] {
            let mut a = search_rg(&rg, dir.path(), &query, far_deadline()).unwrap();
            let mut b = search_scan(dir.path(), &query, far_deadline()).unwrap();
            a.sort_by(|x, y| (&x.path, x.line).cmp(&(&y.path, y.line)));
            b.sort_by(|x, y| (&x.path, x.line).cmp(&(&y.path, y.line)));
            assert_eq!(a, b, "engines must agree ({label})");
        }
    }

    #[cfg(unix)]
    #[test]
    fn rg_deadline_kills_a_silent_engine() {
        use std::os::unix::fs::PermissionsExt;
        // A stand-in rg that produces no output and never exits on its own:
        // the watchdog must kill it at the deadline instead of blocking the
        // stdout read for the child's whole lifetime.
        let dir = tempdir().unwrap();
        let fake = dir.path().join("fake-rg");
        std::fs::write(&fake, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        let started = Instant::now();
        let hits =
            search_rg(&fake, dir.path(), &q("x"), Instant::now() + Duration::from_millis(200))
                .unwrap();
        assert!(hits.is_empty());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "silent rg outlived the deadline: {:?}",
            started.elapsed()
        );
    }

    /// Ship gate A: both engines finish a 10k-file tree within the search
    /// timeout. Heavy, so ignored by default:
    /// `cargo test -p agency-core --lib search -- --ignored`
    #[test]
    #[ignore]
    fn ship_gate_10k_files_within_timeout() {
        let dir = tempdir().unwrap();
        for i in 0..10_000 {
            let sub = dir.path().join(format!("d{}", i % 100));
            std::fs::create_dir_all(&sub).unwrap();
            std::fs::write(
                sub.join(format!("f{i}.txt")),
                format!("line one of file {i}\nneedle-{} here\n", i % 977),
            )
            .unwrap();
        }
        let started = Instant::now();
        let scan = search_scan(dir.path(), &q("needle-500"), Instant::now() + TIMEOUT).unwrap();
        let scan_ms = started.elapsed().as_millis();
        assert!(!scan.is_empty());
        assert!(started.elapsed() < TIMEOUT + Duration::from_millis(500), "scan took {scan_ms}ms");

        if let Some(rg) = rg_on_path() {
            let started = Instant::now();
            let hits = search_rg(&rg, dir.path(), &q("needle-500"), Instant::now() + TIMEOUT).unwrap();
            let rg_ms = started.elapsed().as_millis();
            assert!(!hits.is_empty());
            assert!(started.elapsed() < TIMEOUT + Duration::from_millis(500), "rg took {rg_ms}ms");
            eprintln!("10k files: scan {scan_ms}ms, rg {rg_ms}ms");
        } else {
            eprintln!("10k files: scan {scan_ms}ms (rg not installed)");
        }
    }
}
