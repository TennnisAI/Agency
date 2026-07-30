//! Issues as files: canonical issue storage is one markdown file per issue
//! under `.agency/issues/` (`AGE-14.md`) — frontmatter for the tracked fields,
//! H1 for the title, markdown body below. SQLite keeps only the seq high-water
//! mark and a rebuildable index; `reconcile` makes the index follow the files.
//!
//! Frontmatter is parsed strictly (a file that lies about its `key` or invents
//! a status is skipped, never guessed at) but round-trips forgivingly: unknown
//! keys are preserved verbatim so files written by a newer schema — Phase 6
//! adds `due`/`scheduled`/`rank` — survive an older build's write untouched.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Result};

use crate::registry::{Issue, IssueStatus, Registry};

/// Issue files live here, relative to the project root. Deliberately inside
/// `.agency/` but *tracked*: only `worktrees/` and `agency.local.toml` are
/// excluded, so issue files are versioned and land at merge like code.
pub const ISSUES_DIR: &str = ".agency/issues";

/// One parsed issue file: the issue-shaped fields plus any frontmatter lines
/// we don't understand, preserved verbatim and in order.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueFile {
    /// `AGE-14` — also the filename stem; the file's identity.
    pub key: String,
    /// The numeric part of the key.
    pub seq: i64,
    pub title: String,
    pub body: String,
    pub status: IssueStatus,
    pub priority: u8,
    /// Civil dates, `YYYY-MM-DD` — an issue is due on a day, not an instant.
    pub due: Option<String>,
    pub scheduled: Option<String>,
    /// Manual board order within a status group, ascending. Finite by
    /// construction (NaN/inf rejects the file).
    pub rank: Option<f64>,
    /// Epoch seconds; 0 = the file didn't say (reconcile substitutes mtime).
    pub created_at: i64,
    /// Epoch seconds; 0 = the file didn't say (reconcile substitutes mtime).
    pub updated_at: i64,
    /// Unknown frontmatter lines, verbatim. Written back between the known
    /// keys and the closing fence on serialize.
    pub extra: Vec<String>,
}

// ---------------------------------------------------------------------------
// Timestamps — epoch seconds ↔ RFC3339 UTC, no datetime dependency.
// Civil-date math per Howard Hinnant's algorithms.

/// Format epoch seconds as `2026-07-27T09:30:00Z`. Negative inputs clamp to
/// epoch 0 — issues cannot predate their tracker.
pub fn epoch_to_rfc3339(secs: i64) -> String {
    let secs = secs.max(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Parse `2026-07-27T09:30:00Z` (an optional fractional-seconds part is
/// tolerated and ignored) to epoch seconds. Anything else — offsets, missing
/// `Z`, out-of-range fields — is an error.
pub fn rfc3339_to_epoch(s: &str) -> Result<i64> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?Z$").unwrap()
    });
    let caps = re.captures(s).ok_or_else(|| anyhow!("invalid timestamp: {s}"))?;
    let num = |i: usize| caps[i].parse::<i64>().unwrap();
    let (y, m, d) = (num(1), num(2), num(3));
    let (hh, mm, ss) = (num(4), num(5), num(6));
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) || hh > 23 || mm > 59 || ss > 59 {
        bail!("invalid timestamp: {s}");
    }
    let yy = y - i64::from(m <= 2);
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Ok(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

/// Validate a civil date `2026-08-01`: exact shape and a real calendar day.
/// Returned as-is — date strings are the storage format, and they compare
/// lexicographically, which is all sorting and "overdue" need.
pub fn parse_date(s: &str) -> Result<String> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"^(\d{4})-(\d{2})-(\d{2})$").unwrap());
    let caps = re.captures(s).ok_or_else(|| anyhow!("invalid date: {s}"))?;
    let num = |i: usize| caps[i].parse::<i64>().unwrap();
    let (y, m, d) = (num(1), num(2), num(3));
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        bail!("invalid date: {s}");
    }
    Ok(s.to_string())
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        _ => 28,
    }
}

// ---------------------------------------------------------------------------
// Keys and filenames

/// `AGE-14` → `("AGE", 14)`. The shape every issue key and filename stem must
/// match: uppercase alphanumeric prefix, dash, positive number with no leading
/// zero.
pub fn parse_key(key: &str) -> Option<(&str, i64)> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"^([A-Z][A-Z0-9]*)-([1-9][0-9]*)$").unwrap());
    let caps = re.captures(key)?;
    let seq: i64 = caps.get(2)?.as_str().parse().ok()?;
    Some((caps.get(1).unwrap().as_str(), seq))
}

/// Absolute path of an issue's file under a project root.
pub fn issue_path(root: &Path, key: &str) -> PathBuf {
    root.join(ISSUES_DIR).join(format!("{key}.md"))
}

// ---------------------------------------------------------------------------
// Parse / serialize

const KNOWN_KEYS: [&str; 8] =
    ["key", "status", "priority", "due", "scheduled", "rank", "created", "updated"];

/// Parse an issue file. `file_key` is the filename stem (`AGE-14`) — the
/// frontmatter `key` must agree with it or the file is rejected; a file's name
/// is its identity and a mismatch is an error, not a preference.
pub fn parse_issue_file(file_key: &str, text: &str) -> Result<IssueFile> {
    let (_, seq) = parse_key(file_key).ok_or_else(|| anyhow!("invalid key: {file_key}"))?;
    let mut lines = text.lines();
    if lines.next().map(|l| l.trim_end()) != Some("---") {
        bail!("missing frontmatter fence");
    }
    let mut key = None;
    let mut status = None;
    let mut priority: Option<u8> = None;
    let mut due = None;
    let mut scheduled = None;
    let mut rank: Option<f64> = None;
    let mut created = None;
    let mut updated = None;
    let mut extra = Vec::new();
    let mut closed = false;
    for raw in lines.by_ref() {
        let line = raw.trim_end();
        if line == "---" {
            closed = true;
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            bail!("malformed frontmatter line: {line}");
        };
        let k = k.trim();
        if !KNOWN_KEYS.contains(&k) {
            extra.push(raw.to_string());
            continue;
        }
        // Values are single tokens for every known key, so anything from a
        // whitespace-preceded `#` on is a trailing comment.
        let v = v.find(" #").map_or(v, |i| &v[..i]).trim();
        let slot = match k {
            "key" => &mut key,
            "status" => {
                if status.replace(IssueStatus::parse(v)?).is_some() {
                    bail!("duplicate frontmatter key: status");
                }
                continue;
            }
            "priority" => {
                let p: u8 = v.parse().map_err(|_| anyhow!("invalid priority: {v}"))?;
                if p > 4 {
                    bail!("invalid priority: {p}");
                }
                if priority.replace(p).is_some() {
                    bail!("duplicate frontmatter key: priority");
                }
                continue;
            }
            "due" => {
                if due.replace(parse_date(v)?).is_some() {
                    bail!("duplicate frontmatter key: due");
                }
                continue;
            }
            "scheduled" => {
                if scheduled.replace(parse_date(v)?).is_some() {
                    bail!("duplicate frontmatter key: scheduled");
                }
                continue;
            }
            "rank" => {
                let r: f64 = v.parse().map_err(|_| anyhow!("invalid rank: {v}"))?;
                if !r.is_finite() {
                    bail!("invalid rank: {v}");
                }
                if rank.replace(r).is_some() {
                    bail!("duplicate frontmatter key: rank");
                }
                continue;
            }
            "created" => &mut created,
            "updated" => &mut updated,
            _ => unreachable!(),
        };
        if slot.replace(v.to_string()).is_some() {
            bail!("duplicate frontmatter key: {k}");
        }
    }
    if !closed {
        bail!("unterminated frontmatter");
    }
    let key = key.ok_or_else(|| anyhow!("missing frontmatter key: key"))?;
    if key != file_key {
        bail!("frontmatter key {key} does not match filename {file_key}");
    }
    let status = status.ok_or_else(|| anyhow!("missing frontmatter key: status"))?;
    let created_at = created.map(|v| rfc3339_to_epoch(&v)).transpose()?.unwrap_or(0);
    let updated_at = updated.map(|v| rfc3339_to_epoch(&v)).transpose()?.unwrap_or(0);

    // Body: first non-blank line must be the H1 title; the rest is the body.
    let rest: Vec<&str> = lines.collect();
    let mut idx = 0;
    while idx < rest.len() && rest[idx].trim().is_empty() {
        idx += 1;
    }
    let title = rest
        .get(idx)
        .and_then(|l| l.strip_prefix("# "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| anyhow!("missing H1 title"))?
        .to_string();
    let body = rest.get(idx + 1..).unwrap_or(&[]).join("\n").trim().to_string();

    Ok(IssueFile {
        key,
        seq,
        title,
        body,
        status,
        priority: priority.unwrap_or(0),
        due,
        scheduled,
        rank,
        created_at,
        updated_at,
        extra,
    })
}

/// Canonical serialization: known keys in fixed order, preserved unknown lines
/// before the closing fence, H1 title, blank line, body, trailing newline.
/// Trailing comments seen on parse are not written back — the app's writes are
/// canonical, last-writer-wins.
pub fn serialize_issue_file(f: &IssueFile) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("key: {}\n", f.key));
    out.push_str(&format!("status: {}\n", f.status.as_str()));
    out.push_str(&format!("priority: {}\n", f.priority));
    if let Some(d) = &f.due {
        out.push_str(&format!("due: {d}\n"));
    }
    if let Some(d) = &f.scheduled {
        out.push_str(&format!("scheduled: {d}\n"));
    }
    if let Some(r) = f.rank {
        out.push_str(&format!("rank: {r}\n"));
    }
    out.push_str(&format!("created: {}\n", epoch_to_rfc3339(f.created_at)));
    out.push_str(&format!("updated: {}\n", epoch_to_rfc3339(f.updated_at)));
    for line in &f.extra {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(&format!("# {}\n", f.title));
    if !f.body.is_empty() {
        out.push('\n');
        out.push_str(&f.body);
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// File IO

/// Write `contents` to `path` atomically: temp file in the same directory,
/// then rename over the destination (rename clobbers on Unix). A crash leaves
/// either the old file or the new one, never a torn write. Creates the issues
/// directory on first use.
pub fn atomic_write(path: &Path, contents: &str) -> Result<()> {
    let dir = path.parent().ok_or_else(|| anyhow!("no parent dir for {}", path.display()))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, contents).map_err(|e| anyhow!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow!("cannot rename into {}: {e}", path.display())
    })?;
    Ok(())
}

/// Everything `read_issue_dir` learned: parsed issues plus the files it had to
/// skip (`(filename, reason)`). Skips are surfaced, not swallowed — a
/// half-written agent edit shouldn't silently vanish from view.
#[derive(Debug, Default)]
pub struct IssueDirRead {
    pub issues: Vec<IssueFile>,
    pub skipped: Vec<(String, String)>,
}

/// Read every issue file in `root`'s issues dir. Only filenames shaped like a
/// key (`AGE-14.md`) are considered — `README.md`, editor droppings, and
/// anything else are ignored outright. Files that match the shape but fail to
/// parse land in `skipped`. A missing directory is an empty tracker.
pub fn read_issue_dir(root: &Path) -> Result<IssueDirRead> {
    let dir = root.join(ISSUES_DIR);
    let mut out = IssueDirRead::default();
    let entries = match std::fs::read_dir(&dir) {
        Ok(it) => it,
        Err(_) => return Ok(out),
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".md") else { continue };
        if parse_key(stem).is_none() {
            continue;
        }
        let text = match std::fs::read_to_string(entry.path()) {
            Ok(t) => t,
            Err(e) => {
                out.skipped.push((name, format!("unreadable: {e}")));
                continue;
            }
        };
        match parse_issue_file(stem, &text) {
            Ok(f) => out.issues.push(f),
            Err(e) => out.skipped.push((name, e.to_string())),
        }
    }
    out.issues.sort_by_key(|f| f.seq);
    Ok(out)
}

/// Stat-only pass over the issues dir — `(filename, mtime, size)` per issue-
/// shaped file, no reads. Polls diff this signature and reconcile only on
/// change, so an idle board costs one readdir per tick. Flat directory: no
/// walk needed.
pub fn scan_issue_stats(root: &Path) -> Result<Vec<crate::files::DocStat>> {
    let dir = root.join(ISSUES_DIR);
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(it) => it,
        Err(_) => return Ok(out),
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".md") else { continue };
        if parse_key(stem).is_none() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        out.push(crate::files::DocStat {
            path: name,
            mtime_ms: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            size: meta.len(),
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

// ---------------------------------------------------------------------------
// Migration — one-shot export of a project's SQLite issues to files

/// Dropped into `.agency/issues/` at migration so agents (and people) opening
/// the folder learn the rules without asking. Ignored by the filename filter.
pub const ISSUES_README: &str = "\
# Issues

One file per issue; the filename is the issue key (`AGE-14.md`) and the H1 is
the title. These files are the tracker: the app's board is an index over them.

```markdown
---
key: AGE-14
status: in_progress        # backlog|todo|in_progress|in_review|done|cancelled
priority: 2                # 0-4
created: 2026-07-27T09:30:00Z
updated: 2026-07-27T14:02:00Z
---
# Issue title

Body markdown, wikilinks allowed.
```

- To change status, edit `status:`. To close an issue, set `status: done`.
- To file a new issue, add `<KEY>-<n>.md` using the next unused number for the
  key. Numbers are never reused and never renumbered, even after deletion.
- Timestamps are UTC RFC3339; `updated` should be bumped on edit (the app does
  this automatically; if you forget, file mtime is used).
- On an agent branch these files merge like code: edits land when the branch
  merges, and the app's board reads the main checkout. Merging a run also
  advances its linked issue to done automatically — but never backwards.
";

/// Export every index row that has no file yet, plus the README. Files that
/// already exist win (they are canonical); rows are left for `reconcile` to
/// square up. A project with no issues gets no directory at all — no surprise
/// folders in user repos; the README arrives with the first issue file.
/// Returns how many issue files were written.
pub fn export_project(reg: &Registry, project_id: &str, issue_key: &str, root: &Path) -> Result<usize> {
    let mut written = 0;
    for row in reg.list_issues(project_id)? {
        let key = format!("{issue_key}-{}", row.seq);
        let path = issue_path(root, &key);
        if path.exists() {
            continue;
        }
        let file = IssueFile {
            key,
            seq: row.seq,
            title: row.title,
            body: row.body,
            status: row.status,
            priority: row.priority,
            due: row.due,
            scheduled: row.scheduled,
            rank: row.rank,
            created_at: row.created_at,
            updated_at: row.updated_at,
            extra: vec![],
        };
        atomic_write(&path, &serialize_issue_file(&file))?;
        written += 1;
    }
    ensure_readme(root)?;
    Ok(written)
}

/// Write the README into an *existing* issues dir if it's missing. Called
/// wherever the dir may have just been created (export, first create) so the
/// rules travel with the files, but never creates the dir by itself.
pub fn ensure_readme(root: &Path) -> Result<()> {
    let dir = root.join(ISSUES_DIR);
    if !dir.is_dir() {
        return Ok(());
    }
    let readme = dir.join("README.md");
    if !readme.exists() {
        atomic_write(&readme, ISSUES_README)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reconcile — files are truth, index rows follow

/// What a reconcile pass did, for logging.
#[derive(Debug, Default, PartialEq)]
pub struct ReconcileSummary {
    /// Files with no index row: row inserted, fresh uuid minted.
    pub imported: usize,
    /// Files whose row differed: row overwritten from the file.
    pub updated: usize,
    /// Rows whose file is gone: row deleted.
    pub dropped: usize,
    /// Issue-shaped files that failed to parse: `(filename, reason)`. Their
    /// rows, if any, are kept — absence drops rows, corruption doesn't (a
    /// half-written agent edit must not vanish an issue from the board).
    pub skipped: Vec<(String, String)>,
}

/// Make the project's index rows follow its issue files. A row keyed by a seq
/// no file (healthy or corrupt) covers is deleted; a file with no row is
/// imported under a fresh uuid; on both, the row is overwritten from the file
/// (the existing uuid survives, so `runs.issue_id` links hold). Every file's
/// seq raises the high-water mark — numbers consumed by hand-authored files
/// are never handed out again. Files that omit `created`/`updated` get the
/// file's mtime.
pub fn reconcile(reg: &Registry, project_id: &str, root: &Path) -> Result<ReconcileSummary> {
    let read = read_issue_dir(root)?;
    let mut summary = ReconcileSummary { skipped: read.skipped, ..Default::default() };
    for (name, reason) in &summary.skipped {
        log::warn!("issue file skipped: {name}: {reason}");
    }
    let existing: std::collections::HashMap<i64, Issue> =
        reg.list_issues(project_id)?.into_iter().map(|i| (i.seq, i)).collect();

    let mut seen = std::collections::HashSet::new();
    for f in &read.issues {
        seen.insert(f.seq);
        reg.ensure_issue_seq_at_least(project_id, f.seq)?;
        let (created_at, updated_at) = if f.created_at == 0 || f.updated_at == 0 {
            let mtime = std::fs::metadata(issue_path(root, &f.key))
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            (
                if f.created_at == 0 { mtime } else { f.created_at },
                if f.updated_at == 0 { mtime } else { f.updated_at },
            )
        } else {
            (f.created_at, f.updated_at)
        };
        let prev = existing.get(&f.seq);
        let issue = Issue {
            id: prev.map_or_else(|| uuid::Uuid::new_v4().to_string(), |p| p.id.clone()),
            project_id: project_id.to_string(),
            seq: f.seq,
            title: f.title.clone(),
            body: f.body.clone(),
            status: f.status,
            priority: f.priority,
            due: f.due.clone(),
            scheduled: f.scheduled.clone(),
            rank: f.rank,
            created_at,
            updated_at,
        };
        match prev {
            None => {
                reg.upsert_issue_row(&issue)?;
                summary.imported += 1;
            }
            Some(p) if *p != issue => {
                reg.upsert_issue_row(&issue)?;
                summary.updated += 1;
            }
            Some(_) => {}
        }
    }

    // Seqs covered by a corrupt file keep their rows.
    for (name, _) in &summary.skipped {
        if let Some(seq) = name.strip_suffix(".md").and_then(|s| parse_key(s)).map(|(_, n)| n) {
            seen.insert(seq);
        }
    }
    for (seq, row) in &existing {
        if !seen.contains(seq) {
            reg.delete_issue(&row.id)?;
            summary.dropped += 1;
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = "---\n\
key: AGE-14\n\
status: in_progress        # backlog|todo|in_progress|in_review|done|cancelled\n\
priority: 2                # 0-4\n\
created: 2026-07-27T09:30:00Z\n\
updated: 2026-07-27T14:02:00Z\n\
---\n\
# Fix terminal resize on reattach\n\
\n\
Body markdown, wikilinks allowed.\n";

    #[test]
    fn epoch_rfc3339_fixtures_and_roundtrip() {
        assert_eq!(epoch_to_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(epoch_to_rfc3339(1_785_144_600), "2026-07-27T09:30:00Z");
        // Leap day.
        assert_eq!(rfc3339_to_epoch("2024-02-29T12:00:00Z").unwrap(), 1_709_208_000);
        for s in ["1970-01-01T00:00:00Z", "2024-02-29T23:59:59Z", "2026-07-30T08:15:01Z"] {
            assert_eq!(epoch_to_rfc3339(rfc3339_to_epoch(s).unwrap()), s);
        }
        for x in [0i64, 1, 86_399, 86_400, 1_785_144_600, 4_102_444_800] {
            assert_eq!(rfc3339_to_epoch(&epoch_to_rfc3339(x)).unwrap(), x);
        }
        // Fractional seconds tolerated, offsets and garbage rejected.
        assert_eq!(rfc3339_to_epoch("1970-01-01T00:00:00.123Z").unwrap(), 0);
        for bad in [
            "2026-07-27T09:30:00+02:00",
            "2026-07-27 09:30:00Z",
            "2026-02-30T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2023-02-29T00:00:00Z",
            "not-a-date",
        ] {
            assert!(rfc3339_to_epoch(bad).is_err(), "accepted {bad}");
        }
        // Negative clamps rather than panicking.
        assert_eq!(epoch_to_rfc3339(-5), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn parse_key_shapes() {
        assert_eq!(parse_key("AGE-14"), Some(("AGE", 14)));
        assert_eq!(parse_key("A2C-1"), Some(("A2C", 1)));
        for bad in ["age-14", "AGE-0", "AGE-01", "AGE", "AGE-", "-14", "AGE-14.md", "AGE_14"] {
            assert_eq!(parse_key(bad), None, "accepted {bad}");
        }
    }

    #[test]
    fn parses_locked_example_with_trailing_comments() {
        let f = parse_issue_file("AGE-14", EXAMPLE).unwrap();
        assert_eq!(f.key, "AGE-14");
        assert_eq!(f.seq, 14);
        assert_eq!(f.status, IssueStatus::InProgress);
        assert_eq!(f.priority, 2);
        assert_eq!(f.created_at, 1_785_144_600);
        assert_eq!(f.title, "Fix terminal resize on reattach");
        assert_eq!(f.body, "Body markdown, wikilinks allowed.");
        assert!(f.extra.is_empty());
    }

    #[test]
    fn serialize_is_canonical() {
        let f = IssueFile {
            key: "AGE-14".into(),
            seq: 14,
            title: "Fix terminal resize on reattach".into(),
            body: "Body markdown, wikilinks allowed.".into(),
            status: IssueStatus::InProgress,
            priority: 2,
            due: None,
            scheduled: None,
            rank: None,
            created_at: 1_785_144_600,
            updated_at: 1_785_160_920,
            extra: vec![],
        };
        assert_eq!(
            serialize_issue_file(&f),
            "---\nkey: AGE-14\nstatus: in_progress\npriority: 2\n\
created: 2026-07-27T09:30:00Z\nupdated: 2026-07-27T14:02:00Z\n---\n\
# Fix terminal resize on reattach\n\nBody markdown, wikilinks allowed.\n"
        );
        // Round-trip: canonical text parses back to the same value.
        let back = parse_issue_file("AGE-14", &serialize_issue_file(&f)).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn unknown_frontmatter_keys_survive_roundtrip_in_order() {
        let text = "---\nkey: AGE-2\nassignee: sam\nstatus: todo\nlabels: a, b\n---\n# T\n";
        let f = parse_issue_file("AGE-2", text).unwrap();
        assert_eq!(f.extra, vec!["assignee: sam".to_string(), "labels: a, b".to_string()]);
        let out = serialize_issue_file(&f);
        let assignee = out.find("assignee: sam").unwrap();
        let labels = out.find("labels: a, b").unwrap();
        let fence = out.find("\n---\n# ").unwrap();
        assert!(assignee < labels && labels < fence, "order lost: {out}");
        assert_eq!(parse_issue_file("AGE-2", &out).unwrap(), f);
    }

    #[test]
    fn parse_date_shapes() {
        assert_eq!(parse_date("2026-08-01").unwrap(), "2026-08-01");
        assert_eq!(parse_date("2024-02-29").unwrap(), "2024-02-29");
        for bad in
            ["2026-02-30", "2023-02-29", "2026-13-01", "26-8-1", "2026/08/01", "2026-08-01T00:00:00Z", ""]
        {
            assert!(parse_date(bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn dates_and_rank_roundtrip_in_canonical_order() {
        let text = "---\nkey: AGE-5\nstatus: todo\npriority: 1\ndue: 2026-08-01\n\
scheduled: 2026-07-30   # planning\nrank: 1.5\ncreated: 2026-07-27T09:30:00Z\n\
updated: 2026-07-27T09:30:00Z\n---\n# T\n";
        let f = parse_issue_file("AGE-5", text).unwrap();
        assert_eq!(f.due.as_deref(), Some("2026-08-01"));
        assert_eq!(f.scheduled.as_deref(), Some("2026-07-30"));
        assert_eq!(f.rank, Some(1.5));
        let out = serialize_issue_file(&f);
        assert_eq!(
            out,
            "---\nkey: AGE-5\nstatus: todo\npriority: 1\ndue: 2026-08-01\n\
scheduled: 2026-07-30\nrank: 1.5\ncreated: 2026-07-27T09:30:00Z\n\
updated: 2026-07-27T09:30:00Z\n---\n# T\n"
        );
        assert_eq!(parse_issue_file("AGE-5", &out).unwrap(), f);
        // Integer-valued ranks survive the float round-trip.
        let f2 = IssueFile { rank: Some(2.0), ..f.clone() };
        assert_eq!(parse_issue_file("AGE-5", &serialize_issue_file(&f2)).unwrap().rank, Some(2.0));
    }

    #[test]
    fn phase5_shaped_file_is_untouched_by_the_new_keys() {
        // A file with none of the new keys parses to Nones and serializes
        // byte-identically to what Phase 5 wrote.
        let f = parse_issue_file("AGE-14", EXAMPLE).unwrap();
        assert_eq!((f.due, f.scheduled, f.rank), (None, None, None));
        let f = parse_issue_file("AGE-14", &serialize_issue_file(&parse_issue_file("AGE-14", EXAMPLE).unwrap())).unwrap();
        assert_eq!(
            serialize_issue_file(&f),
            "---\nkey: AGE-14\nstatus: in_progress\npriority: 2\n\
created: 2026-07-27T09:30:00Z\nupdated: 2026-07-27T14:02:00Z\n---\n\
# Fix terminal resize on reattach\n\nBody markdown, wikilinks allowed.\n"
        );
    }

    #[test]
    fn date_and_rank_rejections() {
        let cases: &[&str] = &[
            "---\nkey: AGE-1\nstatus: todo\ndue: whenever\n---\n# T\n",
            "---\nkey: AGE-1\nstatus: todo\ndue: 2026-02-30\n---\n# T\n",
            "---\nkey: AGE-1\nstatus: todo\nscheduled: tomorrow\n---\n# T\n",
            "---\nkey: AGE-1\nstatus: todo\nrank: abc\n---\n# T\n",
            "---\nkey: AGE-1\nstatus: todo\nrank: NaN\n---\n# T\n",
            "---\nkey: AGE-1\nstatus: todo\nrank: inf\n---\n# T\n",
            "---\nkey: AGE-1\nstatus: todo\ndue: 2026-08-01\ndue: 2026-08-02\n---\n# T\n",
        ];
        for text in cases {
            assert!(parse_issue_file("AGE-1", text).is_err(), "accepted: {text}");
        }
    }

    #[test]
    fn parse_rejections() {
        let cases: &[(&str, &str)] = &[
            // Missing key.
            ("AGE-1", "---\nstatus: todo\n---\n# T\n"),
            // Key disagrees with filename.
            ("AGE-1", "---\nkey: AGE-2\nstatus: todo\n---\n# T\n"),
            // Unknown status.
            ("AGE-1", "---\nkey: AGE-1\nstatus: blocked\n---\n# T\n"),
            // Priority out of range.
            ("AGE-1", "---\nkey: AGE-1\nstatus: todo\npriority: 9\n---\n# T\n"),
            // Missing status.
            ("AGE-1", "---\nkey: AGE-1\n---\n# T\n"),
            // No H1 title.
            ("AGE-1", "---\nkey: AGE-1\nstatus: todo\n---\njust text\n"),
            // Empty title.
            ("AGE-1", "---\nkey: AGE-1\nstatus: todo\n---\n# \n"),
            // Unterminated frontmatter.
            ("AGE-1", "---\nkey: AGE-1\nstatus: todo\n# T\n"),
            // No frontmatter at all.
            ("AGE-1", "# T\n"),
            // Duplicate key.
            ("AGE-1", "---\nkey: AGE-1\nkey: AGE-1\nstatus: todo\n---\n# T\n"),
            // Bad timestamp.
            ("AGE-1", "---\nkey: AGE-1\nstatus: todo\ncreated: yesterday\n---\n# T\n"),
        ];
        for (key, text) in cases {
            assert!(parse_issue_file(key, text).is_err(), "accepted: {text}");
        }
    }

    #[test]
    fn parse_body_edges() {
        // H1 only: empty body. Missing priority defaults to 0; missing
        // timestamps to 0 (reconcile substitutes mtime).
        let f = parse_issue_file("AGE-1", "---\nkey: AGE-1\nstatus: todo\n---\n# Title only\n")
            .unwrap();
        assert_eq!(f.body, "");
        assert_eq!(f.priority, 0);
        assert_eq!((f.created_at, f.updated_at), (0, 0));
        // Blank lines before the H1 are fine; body keeps its own headings.
        let f = parse_issue_file(
            "AGE-1",
            "---\nkey: AGE-1\nstatus: todo\n---\n\n\n# Title\n\n## Notes\n\ntext\n",
        )
        .unwrap();
        assert_eq!(f.title, "Title");
        assert_eq!(f.body, "## Notes\n\ntext");
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_droppings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(ISSUES_DIR).join("AGE-1.md");
        atomic_write(&path, "first").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        atomic_write(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["AGE-1.md"], "temp file left behind: {names:?}");
    }

    #[test]
    fn read_issue_dir_filters_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let issues = root.join(ISSUES_DIR);
        std::fs::create_dir_all(&issues).unwrap();
        std::fs::write(issues.join("AGE-2.md"), "---\nkey: AGE-2\nstatus: todo\n---\n# Two\n")
            .unwrap();
        std::fs::write(issues.join("AGE-1.md"), "---\nkey: AGE-1\nstatus: done\n---\n# One\n")
            .unwrap();
        std::fs::write(issues.join("README.md"), "not an issue").unwrap();
        std::fs::write(issues.join("notes.txt"), "nope").unwrap();
        std::fs::write(issues.join("AGE-3.md"), "corrupt garbage").unwrap();
        let read = read_issue_dir(root).unwrap();
        let keys: Vec<_> = read.issues.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, vec!["AGE-1", "AGE-2"]);
        assert_eq!(read.skipped.len(), 1);
        assert_eq!(read.skipped[0].0, "AGE-3.md");
        // Missing dir: empty tracker, not an error.
        let empty = tempfile::tempdir().unwrap();
        let read = read_issue_dir(empty.path()).unwrap();
        assert!(read.issues.is_empty() && read.skipped.is_empty());
    }

    fn write_issue(root: &Path, key: &str, status: &str, title: &str) {
        let dir = root.join(ISSUES_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{key}.md")),
            format!("---\nkey: {key}\nstatus: {status}\n---\n# {title}\n"),
        )
        .unwrap();
    }

    #[test]
    fn reconcile_imports_updates_and_drops() {
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();

        // External file with no row: imported, seq high-water bumped past it.
        write_issue(root, "AGE-7", "todo", "Seven");
        let s = reconcile(&reg, "p1", root).unwrap();
        assert_eq!((s.imported, s.updated, s.dropped), (1, 0, 0));
        let rows = reg.list_issues("p1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].seq, 7);
        assert_eq!(rows[0].title, "Seven");
        // Missing timestamps got the file's mtime.
        assert!(rows[0].created_at > 0);
        let id = rows[0].id.clone();
        assert_eq!(reg.alloc_issue_seq("p1").unwrap(), 8);

        // Idempotent, and the uuid is stable across reconciles.
        let s = reconcile(&reg, "p1", root).unwrap();
        assert_eq!((s.imported, s.updated, s.dropped), (0, 0, 0));
        assert_eq!(reg.list_issues("p1").unwrap()[0].id, id);

        // Edit: the row follows, id survives.
        write_issue(root, "AGE-7", "done", "Seven edited");
        let s = reconcile(&reg, "p1", root).unwrap();
        assert_eq!((s.imported, s.updated, s.dropped), (0, 1, 0));
        let row = &reg.list_issues("p1").unwrap()[0];
        assert_eq!(row.id, id);
        assert_eq!(row.status, IssueStatus::Done);
        assert_eq!(row.title, "Seven edited");

        // Delete the file: the row is dropped.
        std::fs::remove_file(issue_path(root, "AGE-7")).unwrap();
        let s = reconcile(&reg, "p1", root).unwrap();
        assert_eq!((s.imported, s.updated, s.dropped), (0, 0, 1));
        assert!(reg.list_issues("p1").unwrap().is_empty());
    }

    #[test]
    fn reconcile_carries_dates_and_rank_into_rows() {
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();
        let dir = root.join(ISSUES_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("AGE-4.md"),
            "---\nkey: AGE-4\nstatus: todo\ndue: 2026-08-01\nrank: 2.5\n---\n# Four\n",
        )
        .unwrap();
        reconcile(&reg, "p1", root).unwrap();
        let row = &reg.list_issues("p1").unwrap()[0];
        assert_eq!(row.due.as_deref(), Some("2026-08-01"));
        assert_eq!(row.scheduled, None);
        assert_eq!(row.rank, Some(2.5));
    }

    #[test]
    fn reconcile_keeps_row_behind_a_corrupt_file() {
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();

        write_issue(root, "AGE-3", "todo", "Three");
        reconcile(&reg, "p1", root).unwrap();
        let id = reg.list_issues("p1").unwrap()[0].id.clone();

        // The file goes corrupt (say, a half-written agent edit): the row
        // stays, the skip is reported.
        std::fs::write(issue_path(root, "AGE-3"), "garbage").unwrap();
        let s = reconcile(&reg, "p1", root).unwrap();
        assert_eq!(s.dropped, 0);
        assert_eq!(s.skipped.len(), 1);
        assert_eq!(s.skipped[0].0, "AGE-3.md");
        let rows = reg.list_issues("p1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].title, "Three");
    }

    #[test]
    fn scan_issue_stats_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let issues = root.join(ISSUES_DIR);
        std::fs::create_dir_all(&issues).unwrap();
        std::fs::write(issues.join("AGE-1.md"), "12345").unwrap();
        std::fs::write(issues.join("README.md"), "ignored").unwrap();
        let stats = scan_issue_stats(root).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].path, "AGE-1.md");
        assert_eq!(stats[0].size, 5);
        assert!(stats[0].mtime_ms > 0);
        assert!(scan_issue_stats(tempfile::tempdir().unwrap().path()).unwrap().is_empty());
    }
}
