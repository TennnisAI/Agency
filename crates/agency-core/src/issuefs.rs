//! Issues as files: canonical issue storage is one markdown file per issue
//! under `.agency/issues/` (`AGE-14.md`) — frontmatter for the tracked fields,
//! H1 for the title, markdown body below. SQLite keeps only the seq high-water
//! mark and a rebuildable index; `reconcile` makes the index follow the files.
//!
//! Frontmatter is parsed strictly (a file that lies about its `key` or invents
//! a status is skipped, never guessed at) but round-trips forgivingly: unknown
//! keys are preserved verbatim so files written by a newer schema — Phase 6
//! adds `due`/`scheduled`/`rank` — survive an older build's write untouched.
//!
//! `key` names an issue within a checkout; `uid` names it across checkouts, and
//! is what any future sync of this directory between machines has to match on.
//! See `docs/tracked-issues.md`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Result};

use crate::registry::{Issue, IssueComment, IssueStatus, Registry};

/// Issue files live here, relative to the project root, and are *not* tracked
/// by git (see `worktree::untrack_issue_files`). The app rewrites them on every
/// status change; tracking them kept the project's checkout dirty, which is
/// exactly what a merge refuses to start on, and put the same frontmatter lines
/// on both sides of every agent merge. One shared copy, in the project's own
/// checkout, is what every worktree reads and writes.
pub const ISSUES_DIR: &str = ".agency/issues";

/// Attachments live one level down, so an issue body can reference them with a
/// plain relative link (`assets/AGE-14-shot.png`) that resolves from the issue
/// file's own directory — in the app, on GitHub, and in any markdown editor.
pub const ASSETS_DIR: &str = "assets";

/// One parsed issue file: the issue-shaped fields plus any frontmatter lines
/// we don't understand, preserved verbatim and in order.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueFile {
    /// `AGE-14` — also the filename stem; the file's identity *within one
    /// checkout*.
    pub key: String,
    /// The issue's identity *across* checkouts: the uuid its index row is
    /// keyed by, persisted so it survives a copy to another machine.
    ///
    /// Without it the key is the only identity an issue has, and the key is
    /// minted from a per-machine counter (`registry::alloc_issue_seq`), so two
    /// machines filing offline both reach for the same number and produce two
    /// unrelated issues that claim to be `AGE-175` — indistinguishable, and
    /// impossible to renumber safely because any `links:` naming `AGE-175`
    /// could mean either. `None` for a file written before this field existed
    /// or hand-authored without one; `reconcile` backfills those in place.
    pub uid: Option<String>,
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
    /// The discussion, in file order (oldest first). Each comment is a
    /// `## <author> · <RFC3339>` section below the body — see `split_body`.
    pub comments: Vec<IssueComment>,
    /// Keys of the issues this one is linked to (`AGE-12`), in file order,
    /// deduped, never including this issue's own key. Any project's key may
    /// appear — links cross trackers. A key with no file behind it is kept:
    /// the target may be an issue that hasn't been written yet, or one in a
    /// project this checkout can't see.
    pub links: Vec<String>,
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

/// The grammar of a key, in one place. `parse_key` reads it, `rekey_text`
/// matches it in running text and `registry::validate_issue_key` accepts a
/// hand-typed prefix by it; the frontend's `ISSUE_TARGET_RE` in
/// `ui/src/lib/links.ts` mirrors it and has to be changed with it.
pub const KEY_PREFIX_PATTERN: &str = "[A-Z0-9]+";
/// A positive number with no leading zero.
pub const KEY_SEQ_PATTERN: &str = "[1-9][0-9]*";
/// Longest prefix a wikilink can name: `ISSUE_TARGET_RE` allows eight.
pub const KEY_PREFIX_MAX: usize = 8;

/// `AGE-14` → `("AGE", 14)`. The shape every issue key and filename stem must
/// match: uppercase alphanumeric prefix, dash, positive number with no leading
/// zero.
///
/// The prefix may start with a digit: `derive_issue_key` builds keys from the
/// project name's initials, so a project called `1bit-launcher` gets `1LB`.
/// Requiring a leading letter here made every one of its issue files invisible
/// to the filename filter — and reconcile then deleted the rows behind them.
pub fn parse_key(key: &str) -> Option<(&str, i64)> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(&format!("^({KEY_PREFIX_PATTERN})-({KEY_SEQ_PATTERN})$")).unwrap()
    });
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

const KNOWN_KEYS: [&str; 10] =
    ["key", "uid", "status", "priority", "due", "scheduled", "rank", "links", "created", "updated"];

/// Is this a hyphenated uuid (`8-4-4-4-12` hex, either case)? Used to reject a
/// `uid:` that could not have come from us. Shape only, and deliberately not a
/// version check: a uuid we did not mint is still a usable identity, but a
/// hand-typed word is not — it would collide with every other file someone
/// typed the same word into, which is the exact failure `uid` exists to stop.
fn is_uuid(s: &str) -> bool {
    let groups = [8, 4, 4, 4, 12];
    let mut parts = s.split('-');
    for len in groups {
        match parts.next() {
            Some(p) if p.len() == len && p.bytes().all(|b| b.is_ascii_hexdigit()) => {}
            _ => return false,
        }
    }
    parts.next().is_none()
}

/// Parse a `links:` value — a comma-separated list of issue keys. Keys are
/// upper-cased (a hand-written `age-12` means AGE-12), deduped, and kept in
/// the order written; `own_key` is dropped, since an issue linking to itself
/// says nothing. Anything that isn't `KEY-n` shaped is an error, like every
/// other malformed frontmatter value here — the app's own writes are always
/// well-formed, so a bad value is a hand edit worth reporting.
pub fn parse_links(value: &str, own_key: &str) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for raw in value.split(',') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        let key = token.to_ascii_uppercase();
        if parse_key(&key).is_none() {
            bail!("invalid link: {token}");
        }
        if key == own_key || out.contains(&key) {
            continue;
        }
        out.push(key);
    }
    Ok(out)
}

/// A comment's opening line: `## <author> · <RFC3339>`. Returns the author
/// and the timestamp when the line is one, and `None` for every other heading
/// — the body owns its own `##` sections, and only this exact shape is a
/// comment. The separator is the last ` · ` on the line, so an author may
/// contain one.
fn comment_head(line: &str) -> Option<(String, i64)> {
    let (author, at) = line.strip_prefix("## ")?.trim_end().rsplit_once(" · ")?;
    let author = author.trim();
    if author.is_empty() {
        return None;
    }
    Some((author.to_string(), rfc3339_to_epoch(at.trim()).ok()?))
}

/// Split the text under the H1 into the body and the comment thread. The body
/// runs to the first comment heading (all of it, when there is none); each
/// heading opens a comment that runs to the next one. Both sides are trimmed,
/// which is what makes the split round-trip through `serialize_issue_file`.
fn split_body(lines: &[&str]) -> (String, Vec<IssueComment>) {
    let start = lines.iter().position(|l| comment_head(l).is_some());
    let Some(start) = start else {
        return (lines.join("\n").trim().to_string(), Vec::new());
    };
    let body = lines[..start].join("\n").trim().to_string();
    let mut comments: Vec<IssueComment> = Vec::new();
    let mut open: Option<(String, i64, Vec<&str>)> = None;
    let close = |c: (String, i64, Vec<&str>), out: &mut Vec<IssueComment>| {
        out.push(IssueComment {
            author: c.0,
            created_at: c.1,
            body: c.2.join("\n").trim().to_string(),
        });
    };
    for line in &lines[start..] {
        if let Some((author, at)) = comment_head(line) {
            if let Some(c) = open.take() {
                close(c, &mut comments);
            }
            open = Some((author, at, Vec::new()));
        } else if let Some(c) = open.as_mut() {
            c.2.push(line);
        }
    }
    if let Some(c) = open.take() {
        close(c, &mut comments);
    }
    (body, comments)
}

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
    let mut uid: Option<String> = None;
    let mut status = None;
    let mut priority: Option<u8> = None;
    let mut due = None;
    let mut scheduled = None;
    let mut rank: Option<f64> = None;
    let mut links: Option<Vec<String>> = None;
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
            "uid" => {
                if !is_uuid(v) {
                    bail!("invalid uid: {v}");
                }
                if uid.replace(v.to_string()).is_some() {
                    bail!("duplicate frontmatter key: uid");
                }
                continue;
            }
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
            "links" => {
                if links.replace(parse_links(v, file_key)?).is_some() {
                    bail!("duplicate frontmatter key: links");
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
    let (body, comments) = split_body(rest.get(idx + 1..).unwrap_or(&[]));

    Ok(IssueFile {
        key,
        uid,
        seq,
        title,
        body,
        comments,
        status,
        priority: priority.unwrap_or(0),
        due,
        scheduled,
        rank,
        links: links.unwrap_or_default(),
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
    if let Some(u) = &f.uid {
        out.push_str(&format!("uid: {u}\n"));
    }
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
    if !f.links.is_empty() {
        out.push_str(&format!("links: {}\n", f.links.join(", ")));
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
    // The thread, below the body it discusses, in the shape `split_body` reads.
    for c in &f.comments {
        out.push_str(&format!("\n## {} · {}\n", c.author, epoch_to_rfc3339(c.created_at)));
        if !c.body.is_empty() {
            out.push('\n');
            out.push_str(&c.body);
            out.push('\n');
        }
    }
    out
}

/// Insert `uid: <uuid>` into an issue file's frontmatter, returning the new
/// text. `None` when the text already carries a `uid:` or has no frontmatter to
/// put one in, so a caller can treat `None` as "nothing to do".
///
/// A byte-level splice rather than a `parse_issue_file` → `serialize_issue_file`
/// round-trip, which is lossy in ways that would be gratuitous here: the round
/// trip drops the trailing `# backlog|todo|…` comments the README's own example
/// teaches people to write, normalizes whitespace, and rewrites CRLF as LF.
/// Backfilling an identity the user never asked for must not reformat a file
/// they hand-authored; everything outside the inserted line is preserved
/// byte-for-byte.
///
/// The line goes after `key:` so the two identities read together, and falls
/// back to the top of the block for a file that somehow lacks one.
fn splice_uid(text: &str, uid: &str) -> Option<String> {
    let mut insert_at: Option<usize> = None;
    let mut eol = "\n";
    let mut offset = 0usize;
    let mut in_frontmatter = false;
    for raw in text.split_inclusive('\n') {
        let next = offset + raw.len();
        let line = raw.trim_end_matches('\n').trim_end_matches('\r');
        if !in_frontmatter {
            if line.trim_end() != "---" {
                return None;
            }
            in_frontmatter = true;
            if raw.ends_with("\r\n") {
                eol = "\r\n";
            }
            insert_at = Some(next);
        } else if line.trim_end() == "---" {
            break;
        } else {
            match line.split_once(':').map(|(k, _)| k.trim()) {
                Some("uid") => return None,
                Some("key") => insert_at = Some(next),
                _ => {}
            }
        }
        offset = next;
    }
    let at = insert_at?;
    let mut out = String::with_capacity(text.len() + uid.len() + 6);
    out.push_str(&text[..at]);
    out.push_str("uid: ");
    out.push_str(uid);
    out.push_str(eol);
    out.push_str(&text[at..]);
    Some(out)
}

/// Repo-relative paths of the files an issue body attaches, deduped and in
/// order of first mention. Attachments are ordinary markdown links into
/// `assets/` — `![](assets/a.png)` or `[log](assets/a.txt)` — so this is a
/// read over the body text, not a separate index that could fall out of step
/// with it. Links that point anywhere else (http, a sibling note, `..`) are
/// not attachments and are left alone.
pub fn body_attachments(body: &str) -> Vec<String> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"!?\[[^\]]*\]\(\s*(?:<([^>]+)>|([^)\s]+))\s*\)").unwrap()
    });
    let prefix = format!("{ASSETS_DIR}/");
    let mut out: Vec<String> = Vec::new();
    for caps in re.captures_iter(body) {
        let target = caps.get(1).or_else(|| caps.get(2)).map_or("", |m| m.as_str());
        // Percent-decoding is deliberately not attempted: the app never writes
        // encoded names, and a path handed to an agent must be the literal one
        // on disk or nothing.
        if !target.starts_with(&prefix) || target.contains("..") {
            continue;
        }
        let path = format!("{ISSUES_DIR}/{target}");
        if !out.contains(&path) {
            out.push(path);
        }
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
    /// The issues directory could not be listed at all — absent (a branch
    /// without the tracker checked out) or unreadable (permissions). Nothing
    /// is known about the files, which is not the same as "there are none";
    /// reconcile must not treat it as an empty tracker.
    pub missing: bool,
}

/// Read every issue file in `root`'s issues dir. Only filenames shaped like a
/// key (`AGE-14.md`) are considered — `README.md`, editor droppings, and
/// anything else are ignored outright. Files that match the shape but fail to
/// parse land in `skipped`. A directory that cannot be listed sets `missing`.
pub fn read_issue_dir(root: &Path) -> Result<IssueDirRead> {
    let dir = root.join(ISSUES_DIR);
    let mut out = IssueDirRead::default();
    let entries = match std::fs::read_dir(&dir) {
        Ok(it) => it,
        Err(_) => {
            out.missing = true;
            return Ok(out);
        }
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

// ---------------------------------------------------------------------------
// Keys

/// The shared tracker's key, when this project should take it on rather than
/// go on ignoring every file in it.
///
/// Only when this project has nothing of its own under its own key: adopting is
/// then pure gain, because there is nothing the rename could strand. A project
/// holding its own issues has two real backlogs in one directory and no
/// automatic answer — the user picks, which is what the key setting is for.
///
/// One foreign key only. Two means a directory that has been shared with two
/// different projects, and guessing which one this checkout is would be
/// choosing whose issues to keep showing.
pub fn adoptable_key(ours_count: usize, foreign: &[(String, usize)]) -> Option<&str> {
    match (ours_count, foreign) {
        (0, [(prefix, _)]) => Some(prefix.as_str()),
        _ => None,
    }
}

/// One file's move under a key rename: `DEM-7` to `AGE-7`, or to `AGE-9` when
/// `AGE-7` was already someone else's.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct KeyMove {
    pub from: String,
    pub to: String,
}

/// What a key rename would do to a directory, worked out before anything is
/// touched so the settings UI can say it before the user commits.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct RekeyPlan {
    /// Every file that moves, in seq order.
    pub moves: Vec<KeyMove>,
    /// The subset that also takes a new number, because the old one was taken
    /// under the new key. An issue's identity is its `uid`, not its number, so
    /// renumbering loses nothing; every `links:` and wikilink that named the
    /// old number is rewritten to the new one in the same pass.
    pub renumbered: Vec<KeyMove>,
}

/// Is the `FROM-n` match at `start..end` in `line` a reference to an issue, as
/// opposed to a run of the same characters inside something longer?
///
/// Word-bounded matching is not enough, because `-` and `/` are word
/// boundaries. Every key `validate_issue_key` accepts is a valid prefix here,
/// and a project keyed `2026` saw its `updated: 2026-11-01T00:00:00Z` become
/// `AGE-11-01T00:00:00Z` on rename: the frontmatter then failed to parse, so
/// the rename was refused with a message blaming the file, and every comment
/// heading with an October-or-later date lost its shape silently, which folded
/// the comment into the body. `assets/AGN-14-shot.png` was rewritten the same
/// way, and nothing renames the attachment, so the link broke.
///
/// So a reference is not preceded by `/`, `-` or `.` (a path segment, or the
/// tail of a hyphenated token like a date or a uuid), and is not followed by
/// `-` or `.` and an alphanumeric (the head of one, or a filename's
/// extension). `AGN-14.` at the end of a sentence is still a reference.
fn is_key_reference(line: &str, start: usize, end: usize) -> bool {
    let before = line[..start].chars().next_back();
    if matches!(before, Some('/' | '-' | '.')) {
        return false;
    }
    let mut after = line[end..].chars();
    !matches!((after.next(), after.next()), (Some('-' | '.'), Some(c)) if c.is_ascii_alphanumeric())
}

/// The rename, compiled once for every line of every file it touches.
struct Rekey<'a> {
    re: regex::Regex,
    to: &'a str,
    renumber: &'a std::collections::BTreeMap<i64, i64>,
}

impl<'a> Rekey<'a> {
    fn new(from: &str, to: &'a str, renumber: &'a std::collections::BTreeMap<i64, i64>) -> Self {
        let re = regex::Regex::new(&format!(r"\b{}-({KEY_SEQ_PATTERN})\b", regex::escape(from)))
            .unwrap();
        Rekey { re, to, renumber }
    }

    /// The key `FROM-seq` moves to.
    fn key(&self, seq: i64) -> String {
        format!("{}-{}", self.to, self.renumber.get(&seq).copied().unwrap_or(seq))
    }

    /// Rewrite every reference in one line of running text.
    fn refs(&self, line: &str) -> String {
        self.re
            .replace_all(line, |caps: &regex::Captures| {
                let m = caps.get(0).unwrap();
                if !is_key_reference(line, m.start(), m.end()) {
                    return m.as_str().to_string();
                }
                match caps[1].parse::<i64>() {
                    Ok(seq) => self.key(seq),
                    Err(_) => m.as_str().to_string(),
                }
            })
            .into_owned()
    }

    /// Rewrite a whole issue file, by its structure: in the frontmatter only
    /// the `key:` and `links:` values hold references, and below it only the
    /// title, the body and the comment bodies do. A comment heading is an
    /// author and a timestamp, and is left alone whatever it looks like.
    fn text(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + 16);
        let mut in_frontmatter = false;
        let mut seen_fence = false;
        for raw in text.split_inclusive('\n') {
            let line = raw.trim_end_matches('\n').trim_end_matches('\r');
            let eol = &raw[line.len()..];
            let rewritten = if !seen_fence {
                seen_fence = true;
                if line.trim_end() == "---" {
                    in_frontmatter = true;
                    line.to_string()
                } else {
                    self.refs(line)
                }
            } else if in_frontmatter {
                if line.trim_end() == "---" {
                    in_frontmatter = false;
                    line.to_string()
                } else {
                    match line.split_once(':').map(|(k, _)| k.trim()) {
                        Some("key" | "links") => self.refs(line),
                        _ => line.to_string(),
                    }
                }
            } else if comment_head(line).is_some() {
                line.to_string()
            } else {
                self.refs(line)
            };
            out.push_str(&rewritten);
            out.push_str(eol);
        }
        out
    }

    /// What `parse_issue_file` has to return for the rewritten file, derived
    /// from the parse of the original rather than from the bytes we wrote.
    /// Every field a reference cannot live in (uid, status, timestamps, the
    /// comment authors and dates, the unknown lines) is carried over as it
    /// was, so a rewrite that strayed outside the references is caught.
    fn expect(&self, f: &IssueFile, new_key: &str) -> IssueFile {
        let (_, new_seq) = parse_key(new_key).expect("a planned key parses");
        let mut links: Vec<String> = Vec::new();
        for l in &f.links {
            let l = self.refs(l);
            if l != new_key && !links.contains(&l) {
                links.push(l);
            }
        }
        let lines = |s: &str| s.lines().map(|l| self.refs(l)).collect::<Vec<_>>().join("\n");
        IssueFile {
            key: new_key.to_string(),
            seq: new_seq,
            title: self.refs(&f.title),
            body: lines(&f.body),
            comments: f
                .comments
                .iter()
                .map(|c| IssueComment { body: lines(&c.body), ..c.clone() })
                .collect(),
            links,
            ..f.clone()
        }
    }
}

/// Rewrite every `FROM-n` this text uses as an issue key into `TO-n` (or
/// `TO-m`, for the seqs `renumber` maps) — the `key:` line, the `links:`
/// line, and any `[[FROM-n]]` or bare `FROM-n` the title, body or a comment
/// refers to. See `Rekey::text` for what is and is not a reference.
///
/// Byte-level, for the reason `splice_uid` is: a parse/serialize round trip
/// drops the trailing `# backlog|todo|…` comments the README teaches people to
/// write, normalizes whitespace and rewrites CRLF. Renaming a file is not a
/// licence to reformat it.
pub fn rekey_text(
    text: &str,
    from: &str,
    to: &str,
    renumber: &std::collections::BTreeMap<i64, i64>,
) -> String {
    Rekey::new(from, to, renumber).text(text)
}

/// Read the directory and decide what a rename from `from` to `to` moves where,
/// without touching anything.
///
/// A file whose number is already taken under `to` (this project's `DEM-7`
/// meeting a shared `AGE-7` that is a different issue) takes the next free
/// number instead of being refused. Both sides of a shared backlog number from
/// 1 by their own counters, so in the one case this rename exists for, a
/// collision is the normal shape, and a refusal there left the user with the
/// instruction to rename and no way to follow it. New numbers start above
/// every number in use under either key and above `seq_floor`, the project's
/// own high-water mark, so a number a deleted issue once carried is not handed
/// to a different issue.
pub fn plan_rekey(root: &Path, from: &str, to: &str, seq_floor: i64) -> Result<RekeyPlan> {
    if from == to {
        return Ok(RekeyPlan::default());
    }
    let read = read_issue_dir(root)?;
    plan_rekey_from(&read, from, to, seq_floor).map(|(p, _)| p)
}

fn plan_rekey_from<'a>(
    read: &'a IssueDirRead,
    from: &str,
    to: &str,
    seq_floor: i64,
) -> Result<(RekeyPlan, Vec<&'a IssueFile>)> {
    if read.missing {
        bail!("this project's issues directory could not be read");
    }
    // A file under the old key that we cannot parse is one we cannot verify,
    // and leaving it behind would split the backlog silently. One under any
    // other key is not ours to move and is none of this operation's business.
    let skipped_keys = read.skipped.iter().filter_map(|(n, r)| {
        let stem = n.strip_suffix(".md")?;
        parse_key(stem).map(|(p, s)| (p, s, n.as_str(), r.as_str()))
    });
    let mut taken: std::collections::BTreeSet<i64> = Default::default();
    let mut max_seq = 0;
    for (prefix, seq, name, reason) in skipped_keys {
        if prefix == from {
            bail!("{name} could not be read ({reason}); fix or remove it, then rename the key");
        }
        if prefix == to {
            taken.insert(seq);
            max_seq = max_seq.max(seq);
        }
    }
    let mut ours: Vec<&IssueFile> = Vec::new();
    for f in &read.issues {
        match parse_key(&f.key) {
            Some((p, seq)) if p == from => {
                ours.push(f);
                max_seq = max_seq.max(seq);
            }
            Some((p, seq)) if p == to => {
                taken.insert(seq);
                max_seq = max_seq.max(seq);
            }
            _ => {}
        }
    }
    let mut next = seq_floor.max(max_seq + 1);
    let mut plan = RekeyPlan::default();
    for f in &ours {
        let mv = if taken.contains(&f.seq) {
            let mv = KeyMove { from: f.key.clone(), to: format!("{to}-{next}") };
            next += 1;
            plan.renumbered.push(mv.clone());
            mv
        } else {
            KeyMove { from: f.key.clone(), to: format!("{to}-{}", f.seq) }
        };
        plan.moves.push(mv);
    }
    Ok((plan, ours))
}

/// A rename that happened, and enough to take it back.
#[derive(Debug)]
pub struct Rekeyed {
    pub plan: RekeyPlan,
    /// `(old path, new path, old bytes)` per file moved.
    originals: Vec<(PathBuf, PathBuf, String)>,
}

impl Rekeyed {
    /// Put every file back where it was, byte for byte. For the caller whose
    /// own next step failed after the files had moved: an index that still
    /// says `DEM` over files that now say `AGE` treats every one of them as
    /// foreign and drops its row, so the files have to follow the index back.
    pub fn restore(&self) -> Result<()> {
        let mut left: Vec<String> = Vec::new();
        for (src, dest, text) in &self.originals {
            if let Err(e) = atomic_write(src, text) {
                left.push(format!("{} ({e})", src.display()));
                continue;
            }
            if let Err(e) = std::fs::remove_file(dest) {
                left.push(format!("{} ({e})", dest.display()));
            }
        }
        if left.is_empty() {
            Ok(())
        } else {
            bail!("could not put back {}", left.join(", "))
        }
    }
}

/// Remove what a failed rename had written so far. Returns what it could not
/// remove, so the error the caller raises can say "nothing was changed" only
/// when that is true.
fn remove_all(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|p| match std::fs::remove_file(p) {
            Ok(()) => None,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => Some(format!("{} ({e})", p.display())),
        })
        .collect()
}

fn unchanged_or(left: Vec<String>) -> String {
    if left.is_empty() {
        "nothing was changed".to_string()
    } else {
        format!("and {} could not be removed", left.join(", "))
    }
}

/// Move this project's issue files from one key prefix to another, as
/// `plan_rekey` lays it out.
///
/// Every new file is written and read back before a single old one is removed,
/// so a failure part-way through leaves the directory exactly as it was. That
/// is the rule for anything that rewrites a user's files, and it matters more
/// than usual here: the alternative is a backlog half under each key, which is
/// the state this whole function exists to get someone out of. The read-back
/// is checked against what the *old* file parsed to, with only the reference
/// fields allowed to differ, so a rewrite that strayed into a timestamp or a
/// comment heading is refused rather than shipped.
///
/// The removals are part of that contract. An old file that cannot be removed
/// used to be logged and counted as moved: the index then took the new key,
/// and the file left behind read as a foreign one the next sync nagged about,
/// with advice that led straight back to a refusal. Now a removal that fails
/// puts everything back and reports it.
pub fn rekey_issue_files(root: &Path, from: &str, to: &str, seq_floor: i64) -> Result<Rekeyed> {
    if from == to {
        return Ok(Rekeyed { plan: RekeyPlan::default(), originals: Vec::new() });
    }
    let read = read_issue_dir(root)?;
    let (plan, ours) = plan_rekey_from(&read, from, to, seq_floor)?;
    let renumber: std::collections::BTreeMap<i64, i64> = plan
        .renumbered
        .iter()
        .filter_map(|m| Some((parse_key(&m.from)?.1, parse_key(&m.to)?.1)))
        .collect();
    let rekey = Rekey::new(from, to, &renumber);

    // (source, destination, original bytes, rewritten bytes, expected parse)
    let mut moves: Vec<(PathBuf, PathBuf, String, String, IssueFile)> = Vec::new();
    for (f, mv) in ours.iter().zip(&plan.moves) {
        let dest = issue_path(root, &mv.to);
        if dest.exists() {
            bail!("{}.md already exists here, so {} cannot take that name", mv.to, mv.from);
        }
        let src = issue_path(root, &mv.from);
        let text = std::fs::read_to_string(&src)?;
        let rewritten = rekey.text(&text);
        moves.push((src, dest, text, rewritten, rekey.expect(f, &mv.to)));
    }

    let mut written: Vec<PathBuf> = Vec::new();
    for (_, dest, _, rewritten, want) in &moves {
        let stem = dest.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        if let Err(e) = atomic_write(dest, rewritten) {
            let left = remove_all(&written);
            bail!("could not write {stem}.md ({e}); {}", unchanged_or(left));
        }
        written.push(dest.clone());
        match std::fs::read_to_string(dest)
            .map_err(anyhow::Error::from)
            .and_then(|t| parse_issue_file(&stem, &t))
        {
            Ok(got) if got == *want => {}
            Ok(_) => {
                let left = remove_all(&written);
                bail!(
                    "{stem}.md did not read back as the issue it was renamed from; {}",
                    unchanged_or(left)
                );
            }
            Err(e) => {
                let left = remove_all(&written);
                bail!("{stem}.md could not be read back ({e}); {}", unchanged_or(left));
            }
        }
    }

    // Every new file is on disk and verified; only now is anything removed.
    for (i, (src, _, _, _, _)) in moves.iter().enumerate() {
        if let Err(e) = std::fs::remove_file(src) {
            let mut left: Vec<String> = Vec::new();
            for (s, _, text, _, _) in &moves[..i] {
                if let Err(e) = atomic_write(s, text) {
                    left.push(format!("{} ({e})", s.display()));
                }
            }
            left.extend(remove_all(&written));
            bail!("could not remove {} ({e}); {}", src.display(), unchanged_or(left));
        }
    }
    Ok(Rekeyed {
        plan,
        originals: moves.into_iter().map(|(src, dest, text, _, _)| (src, dest, text)).collect(),
    })
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

This project's issues are tracked in Agency, and these files are the tracker:
the app's board is an index over them. One file per issue; the filename is the
issue key (`AGE-14.md`) and the H1 is the title.

```markdown
---
key: AGE-14
uid: 8fbc9e2a-3d41-4c7e-9a10-5b6d2f8e04c3   # the app writes this; don't edit
status: in_progress        # backlog|todo|in_progress|in_review|done|cancelled
priority: 2                # 0-4
created: 2026-07-27T09:30:00Z
updated: 2026-07-27T14:02:00Z
---
# Issue title

Body markdown, wikilinks allowed.

## Sam · 2026-07-27T15:10:00Z

A comment. Everything below the first heading of this shape is the
issue's discussion, not its description.
```

- To change status, edit `status:`. To close an issue, set `status: done`.
- To file a new issue, add `<KEY>-<n>.md` using the next unused number for the
  key. Numbers are never reused and never renumbered, even after deletion.
  Leave `uid:` out; the app writes one in. It is the issue's identity when this
  directory is copied somewhere the numbering isn't shared, so never copy one
  from another issue and never edit one by hand.
- To comment, append a `## <author> · <UTC RFC3339>` section to the end of the
  file and write under it. The body is everything between the H1 and the first
  such heading, so a comment never eats the description. Sign it with your own
  name; the app signs yours with the repo's git user.
- To link issues, list their keys in `links:` (`links: AGE-12, LBH-3`, any
  project's key). Links are undirected: the app writes the other side too, and
  shows them in the issue's Links section. A link to an issue that doesn't
  exist is kept, not pruned.
- Timestamps are UTC RFC3339; `updated` should be bumped on edit (the app does
  this automatically; if you forget, file mtime is used).
- Attachments live in `assets/`, referenced from the body by a relative link:
  `![](assets/AGE-14-shot.png)` for images, `[label](assets/AGE-14-log.txt)`
  for anything else. Relative to this directory, so the same link resolves in
  the app and in any markdown editor. Drop, paste, or pick a file in the
  issue's detail pane to add one.
- These files are not tracked by git. There is one copy, here in the project's
  own checkout, shared by every agent worktree; an agent working an issue is
  given this absolute path and edits the file in place. Edits take effect as
  soon as they are written, with no commit or merge involved.
- Merging an agent run advances its linked issue to done automatically, but
  never backwards.
";

/// Export every index row that has no file yet, plus the README. Files that
/// already exist win (they are canonical); rows are left for `reconcile` to
/// square up. A project with no issues gets no directory at all — no surprise
/// folders in user repos; the README arrives with the first issue file.
/// Returns how many issue files were written.
pub fn export_project(
    reg: &Registry,
    project_id: &str,
    issue_key: &str,
    root: &Path,
) -> Result<usize> {
    let mut written = 0;
    for row in reg.list_issues(project_id)? {
        let key = format!("{issue_key}-{}", row.seq);
        let path = issue_path(root, &key);
        if path.exists() {
            continue;
        }
        let file = IssueFile {
            key,
            uid: Some(row.id.clone()),
            seq: row.seq,
            title: row.title,
            body: row.body,
            comments: row.comments,
            status: row.status,
            priority: row.priority,
            due: row.due,
            scheduled: row.scheduled,
            rank: row.rank,
            links: row.links,
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
///
/// An existing README that still opens with our own heading is refreshed: it
/// states the rules agents are told to follow, and a stale copy (issue files
/// used to be tracked and to merge with a branch) is worse than none. A README
/// someone has made their own, heading and all, is left alone.
pub fn ensure_readme(root: &Path) -> Result<()> {
    let dir = root.join(ISSUES_DIR);
    if !dir.is_dir() {
        return Ok(());
    }
    let readme = dir.join("README.md");
    let current = std::fs::read_to_string(&readme).unwrap_or_default();
    let ours = current.is_empty() || current.starts_with("# Issues\n");
    if ours && current != ISSUES_README {
        atomic_write(&readme, ISSUES_README)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reconcile — files are truth, index rows follow

/// The index row a parsed file describes. `id` is the row's own uuid (minted
/// at import and stable for the life of the file, so `runs.issue_id` holds);
/// the timestamps are passed in because a file that omits them is dated by its
/// mtime instead.
pub fn issue_from_file(
    f: &IssueFile,
    id: String,
    project_id: &str,
    created_at: i64,
    updated_at: i64,
) -> Issue {
    Issue {
        id,
        project_id: project_id.to_string(),
        seq: f.seq,
        title: f.title.clone(),
        body: f.body.clone(),
        status: f.status,
        priority: f.priority,
        due: f.due.clone(),
        scheduled: f.scheduled.clone(),
        rank: f.rank,
        links: f.links.clone(),
        comments: f.comments.clone(),
        created_at,
        updated_at,
    }
}

/// What a reconcile pass did, for logging.
#[derive(Debug, Default, PartialEq)]
pub struct ReconcileSummary {
    /// Files with no index row: row inserted, fresh uuid minted.
    pub imported: usize,
    /// Files whose row differed: row overwritten from the file.
    pub updated: usize,
    /// Files that carried no `uid:` and had one written into them.
    pub backfilled: usize,
    /// Files whose `uid:` is already some other row's id: imported under a
    /// fresh uuid instead, and the file left alone. Two checkouts of one repo
    /// carry the same uids, and `issues.id` is unique across projects.
    pub remapped: usize,
    /// Rows whose file is gone: row deleted.
    pub dropped: usize,
    /// Issue-shaped files that failed to parse: `(filename, reason)`. Their
    /// rows, if any, are kept — absence drops rows, corruption doesn't (a
    /// half-written agent edit must not vanish an issue from the board).
    pub skipped: Vec<(String, String)>,
    /// Files ignored because their key prefix is not this project's, as
    /// `(prefix, count)`, most files first. Reconcile only ever logged these,
    /// which made a sync that pulled 186 issues keyed `AGE` into a project
    /// keyed `AGN` report a clean success over an empty board. The caller is
    /// the only layer that can do anything about it, so it has to be told.
    pub foreign: Vec<(String, usize)>,
}

/// Make the project's index rows follow its issue files. A row keyed by a seq
/// no file (healthy or corrupt) covers is deleted; a file with no row is
/// imported under the uuid its `uid:` names, or a fresh one when it names none
/// (or when that uuid is already another row's id: `issues.id` is unique across
/// projects, a `uid` only within one backlog); on both, the row is overwritten
/// from the file (the existing uuid survives, so `runs.issue_id` links hold).
/// Every file's seq raises the high-water mark
/// — numbers consumed by hand-authored files are never handed out again. Files
/// that omit `created`/`updated` get the file's mtime.
///
/// This is also where a file that predates `uid` gets one written into it. That
/// makes reconcile a writer of the files it reads, which is a departure worth
/// naming: it is a single converging write per file, it changes nothing an
/// index row is derived from, and the alternative (a one-shot migration, as
/// `untrack_issue_files` does) would miss every file that arrives later by hand
/// or by sync — which is most of the ones that need it.
///
/// Only files under the project's own `issue_key` prefix belong to it: the
/// filename is the issue's identity, and the app writes/deletes exactly
/// `{issue_key}-{seq}.md`. A foreign-prefix file (a repo cloned from someone
/// whose project derived a different key, an agent using the wrong key) is
/// ignored with a warning — importing it would leave a row whose edits land
/// in a *different* file, duplicating and resurrecting issues.
///
/// An unlistable issues dir (`read.missing`) with rows in the index is a
/// no-op, not a wipe: a branch checkout without the tracker or a permissions
/// blip must not delete every row (and with them, re-mint the uuids that
/// `runs.issue_id` points at).
pub fn reconcile(
    reg: &Registry,
    project_id: &str,
    issue_key: &str,
    root: &Path,
) -> Result<ReconcileSummary> {
    let read = read_issue_dir(root)?;
    let mut existing: std::collections::HashMap<i64, Issue> =
        reg.list_issues(project_id)?.into_iter().map(|i| (i.seq, i)).collect();
    if read.missing && !existing.is_empty() {
        log::warn!(
            "issues dir unlistable for project {project_id}; keeping {} indexed rows",
            existing.len()
        );
        return Ok(ReconcileSummary::default());
    }
    let mut summary = ReconcileSummary { skipped: read.skipped, ..Default::default() };
    summary.skipped.retain(|(name, reason)| {
        let ours = name
            .strip_suffix(".md")
            .and_then(parse_key)
            .is_some_and(|(prefix, _)| prefix == issue_key);
        if ours {
            log::warn!("issue file skipped: {name}: {reason}");
        }
        ours
    });

    // Which seqs the files cover, and how many carry someone else's prefix,
    // both settled before a single row is written.
    let mut foreign: std::collections::BTreeMap<String, usize> = Default::default();
    let mut seen = std::collections::HashSet::new();
    for f in &read.issues {
        match parse_key(&f.key) {
            Some((prefix, _)) if prefix == issue_key => {
                seen.insert(f.seq);
            }
            Some((prefix, _)) => *foreign.entry(prefix.to_string()).or_default() += 1,
            None => {}
        }
    }
    // Seqs covered by a corrupt file keep their rows.
    for (name, _) in &summary.skipped {
        if let Some(seq) = name.strip_suffix(".md").and_then(|s| parse_key(s)).map(|(_, n)| n) {
            seen.insert(seq);
        }
    }

    // Rows whose seq no file covers are dropped first, before anything is
    // imported. A file renumbered outside the app (AGE-3.md arriving as
    // AGE-20.md on a branch checkout) then finds its own uid free at the new
    // number and keeps it, and with it every `runs.issue_id` pointing at it.
    // Dropping at the end of the pass instead met the old row still holding
    // that uid, and failed the whole sync with `UNIQUE constraint failed:
    // issues.id`.
    let stale: Vec<(i64, String)> = existing
        .iter()
        .filter(|(seq, _)| !seen.contains(*seq))
        .map(|(seq, row)| (*seq, row.id.clone()))
        .collect();
    for (seq, id) in stale {
        reg.delete_issue(&id)?;
        existing.remove(&seq);
        summary.dropped += 1;
    }

    let mut divergent = 0usize;
    for f in &read.issues {
        if !matches!(parse_key(&f.key), Some((prefix, _)) if prefix == issue_key) {
            continue;
        }
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
        // Identity comes from the file when the file carries one — that is what
        // `uid` is for, and it is what lets an issue copied to another machine
        // keep the id its runs point at. A file whose uid disagrees with the row
        // already holding its seq is left alone: `upsert_issue_row` keeps the
        // existing row's id regardless, and re-keying the row here would dangle
        // every `runs.issue_id` pointing at it. Telling the two apart (the same
        // issue whose ids diverged, versus two issues that raced for one number)
        // needs a common ancestor, which only a sync pass has.
        let id = match (f.uid.as_deref(), prev) {
            (Some(u), Some(p)) if u != p.id => {
                divergent += 1;
                log::debug!(
                    "issue {} carries uid {u} but is indexed as {}; keeping the indexed id",
                    f.key,
                    p.id
                );
                p.id.clone()
            }
            // A uid is unique within one backlog; `issues.id` is unique across
            // every project in the index, and two checkouts of one repo share
            // a backlog by design. Both of them reconciling the same files
            // claimed the same uid, and the second project failed its whole
            // sync on `UNIQUE constraint failed: issues.id`: 185 issues
            // indexed under one checkout, 0 under the clone, with the toast
            // the only sign of it. A uid another row already holds is not this
            // row's identity, so this row gets its own. The file is left
            // alone: the uid in it belongs to the checkout that got there
            // first, and rewriting it would break that one's links.
            (Some(u), _) => match reg.issue_id_owner(u)? {
                Some((owner, seq)) if (owner.as_str(), seq) != (project_id, f.seq) => {
                    summary.remapped += 1;
                    uuid::Uuid::new_v4().to_string()
                }
                _ => u.to_string(),
            },
            (None, Some(p)) => p.id.clone(),
            (None, None) => uuid::Uuid::new_v4().to_string(),
        };
        // Persist that identity into the file if it has none, so it survives the
        // next copy of this directory. A failure here is logged, not fatal: a
        // read-only issues dir must not take the board down with it.
        if f.uid.is_none() {
            let path = issue_path(root, &f.key);
            match std::fs::read_to_string(&path).ok().and_then(|t| splice_uid(&t, &id)) {
                Some(next) => match atomic_write(&path, &next) {
                    Ok(()) => summary.backfilled += 1,
                    Err(e) => log::warn!("could not write uid into {}: {e}", f.key),
                },
                None => log::warn!("could not place a uid in {}", f.key),
            }
        }
        let issue = issue_from_file(f, id, project_id, created_at, updated_at);
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

    // One line per pass for these two as well, for the reason `foreign` has
    // one: a shared backlog remaps every file it holds, every pass.
    if summary.remapped > 0 {
        log::warn!(
            "{} issue files carry a uid another project already indexes; \
             imported under fresh ids",
            summary.remapped
        );
    }
    if divergent > 0 {
        log::warn!(
            "{divergent} issue files carry a uid the index disagrees with; kept the indexed ids"
        );
    }
    // One line per pass, not one per file: this used to log 186 warnings every
    // time the board polled, which buried everything else in the log and was
    // the only place the problem was stated at all.
    summary.foreign = foreign.into_iter().collect();
    summary.foreign.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (prefix, n) in &summary.foreign {
        log::warn!("{n} issue files keyed {prefix} ignored: this project's key is {issue_key}");
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
        // Digit-leading prefixes are real: "1bit-launcher" derives 1LB.
        assert_eq!(parse_key("1LB-1"), Some(("1LB", 1)));
        assert_eq!(parse_key("123-7"), Some(("123", 7)));
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
            uid: None,
            seq: 14,
            title: "Fix terminal resize on reattach".into(),
            body: "Body markdown, wikilinks allowed.".into(),
            comments: vec![],
            status: IssueStatus::InProgress,
            priority: 2,
            due: None,
            scheduled: None,
            rank: None,
            links: vec![],
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
    fn splits_the_body_from_the_comment_thread_and_round_trips() {
        let text = "---\nkey: AGE-5\nstatus: todo\n---\n# T\n\n\
Body prose.\n\n\
## Notes on the body\n\n\
Still the body: no timestamp, so not a comment.\n\n\
## Sam · 2026-07-27T15:10:00Z\n\n\
First comment.\n\n\
### Even this heading is comment text\n\n\
## agent · 2026-07-27T16:00:00Z\n\n\
Second.\n";
        let f = parse_issue_file("AGE-5", text).unwrap();
        assert!(f.body.starts_with("Body prose."), "{}", f.body);
        assert!(f.body.ends_with("not a comment."), "{}", f.body);
        assert_eq!(f.comments.len(), 2);
        assert_eq!(f.comments[0].author, "Sam");
        assert_eq!(f.comments[0].created_at, rfc3339_to_epoch("2026-07-27T15:10:00Z").unwrap());
        assert!(f.comments[0].body.contains("### Even this heading"), "{:?}", f.comments[0]);
        assert_eq!(f.comments[1].author, "agent");
        assert_eq!(f.comments[1].body, "Second.");
        // What the app writes parses back to the same value.
        assert_eq!(parse_issue_file("AGE-5", &serialize_issue_file(&f)).unwrap(), f);
    }

    #[test]
    fn comment_heads_are_only_the_exact_shape() {
        // Not comments: no timestamp, an unparseable one, no author.
        for line in [
            "## Sam",
            "## Sam · yesterday",
            "##  · 2026-07-27T15:10:00Z",
            "# Sam · 2026-07-27T15:10:00Z",
        ] {
            assert_eq!(comment_head(line), None, "accepted {line}");
        }
        // An author may itself contain the separator: the last one wins.
        let (author, at) = comment_head("## a · b · 2026-07-27T15:10:00Z").unwrap();
        assert_eq!(author, "a · b");
        assert_eq!(at, rfc3339_to_epoch("2026-07-27T15:10:00Z").unwrap());
    }

    #[test]
    fn parses_and_writes_links() {
        let text = "---\nkey: AGE-5\nstatus: todo\n\
links: AGE-12, lbh-3, AGE-12, AGE-5\n---\n# T\n";
        let f = parse_issue_file("AGE-5", text).unwrap();
        // Upper-cased, deduped, self-link dropped, order kept.
        assert_eq!(f.links, vec!["AGE-12".to_string(), "LBH-3".to_string()]);
        let out = serialize_issue_file(&f);
        assert!(out.contains("links: AGE-12, LBH-3\n"), "{out}");
        assert_eq!(parse_issue_file("AGE-5", &out).unwrap(), f);
        // No links, no line — the frontmatter of an unlinked issue is unchanged.
        let plain = parse_issue_file("AGE-5", "---\nkey: AGE-5\nstatus: todo\n---\n# T\n").unwrap();
        assert!(plain.links.is_empty());
        assert!(!serialize_issue_file(&plain).contains("links:"));
    }

    #[test]
    fn rejects_malformed_links() {
        for bad in [
            "---\nkey: AGE-1\nstatus: todo\nlinks: not a key\n---\n# T\n",
            "---\nkey: AGE-1\nstatus: todo\nlinks: AGE-0\n---\n# T\n",
            "---\nkey: AGE-1\nstatus: todo\nlinks: AGE-2\nlinks: AGE-3\n---\n# T\n",
        ] {
            assert!(parse_issue_file("AGE-1", bad).is_err(), "accepted {bad}");
        }
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
    fn body_attachments_finds_asset_links_only() {
        let body = "\
Steps:

![crash](assets/AGE-14-crash.png)

The log is [here](assets/AGE-14-log.txt), and the shot again:
![](assets/AGE-14-crash.png)

Not attachments: [docs](https://example.com/a.png), ![](../elsewhere/a.png),
[up](assets/../../etc/passwd), [note](other/a.png), bare assets/a.png.
";
        assert_eq!(
            body_attachments(body),
            vec![
                ".agency/issues/assets/AGE-14-crash.png".to_string(),
                ".agency/issues/assets/AGE-14-log.txt".to_string(),
            ]
        );
        assert!(body_attachments("no links at all").is_empty());
        // Angle-bracket targets (what other editors write for spaced names).
        assert_eq!(
            body_attachments("![](<assets/a b.png>)"),
            vec![".agency/issues/assets/a b.png".to_string()]
        );
    }

    #[test]
    fn parse_date_shapes() {
        assert_eq!(parse_date("2026-08-01").unwrap(), "2026-08-01");
        assert_eq!(parse_date("2024-02-29").unwrap(), "2024-02-29");
        for bad in [
            "2026-02-30",
            "2023-02-29",
            "2026-13-01",
            "26-8-1",
            "2026/08/01",
            "2026-08-01T00:00:00Z",
            "",
        ] {
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
        let f = parse_issue_file(
            "AGE-14",
            &serialize_issue_file(&parse_issue_file("AGE-14", EXAMPLE).unwrap()),
        )
        .unwrap();
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
        // Missing dir: flagged as unlistable, not an error — and not the same
        // as an empty tracker.
        let empty = tempfile::tempdir().unwrap();
        let read = read_issue_dir(empty.path()).unwrap();
        assert!(read.issues.is_empty() && read.skipped.is_empty());
        assert!(read.missing);
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
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
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
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!((s.imported, s.updated, s.dropped), (0, 0, 0));
        assert_eq!(reg.list_issues("p1").unwrap()[0].id, id);

        // Edit: the row follows, id survives.
        write_issue(root, "AGE-7", "done", "Seven edited");
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!((s.imported, s.updated, s.dropped), (0, 1, 0));
        let row = &reg.list_issues("p1").unwrap()[0];
        assert_eq!(row.id, id);
        assert_eq!(row.status, IssueStatus::Done);
        assert_eq!(row.title, "Seven edited");

        // Delete the file: the row is dropped.
        std::fs::remove_file(issue_path(root, "AGE-7")).unwrap();
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!((s.imported, s.updated, s.dropped), (0, 0, 1));
        assert!(reg.list_issues("p1").unwrap().is_empty());
    }

    const UID: &str = "8fbc9e2a-3d41-4c7e-9a10-5b6d2f8e04c3";

    fn write_issue_uid(root: &Path, key: &str, uid: &str) {
        let dir = root.join(ISSUES_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{key}.md")),
            format!("---\nkey: {key}\nuid: {uid}\nstatus: todo\n---\n# T\n"),
        )
        .unwrap();
    }

    #[test]
    fn uid_round_trips_and_is_omitted_when_absent() {
        let text = format!("---\nkey: AGE-9\nuid: {UID}\nstatus: todo\n---\n# Nine\n");
        let f = parse_issue_file("AGE-9", &text).unwrap();
        assert_eq!(f.uid.as_deref(), Some(UID));
        // Not swept into `extra` — a known key that fell through would be
        // written twice on the next serialize.
        assert!(f.extra.is_empty());
        let back = parse_issue_file("AGE-9", &serialize_issue_file(&f)).unwrap();
        assert_eq!(back, f);
        assert!(serialize_issue_file(&f).contains(&format!("uid: {UID}\n")));

        let none =
            parse_issue_file("AGE-9", "---\nkey: AGE-9\nstatus: todo\n---\n# Nine\n").unwrap();
        assert_eq!(none.uid, None);
        assert!(!serialize_issue_file(&none).contains("uid:"));
    }

    #[test]
    fn rejects_a_uid_that_is_not_a_uuid() {
        for bad in ["mine", "8fbc9e2a3d414c7e9a105b6d2f8e04c3", "8fbc9e2a-3d41-4c7e-9a10-zzz"] {
            let text = format!("---\nkey: AGE-9\nuid: {bad}\nstatus: todo\n---\n# Nine\n");
            assert!(
                parse_issue_file("AGE-9", &text).is_err(),
                "accepted a uid that cannot be one: {bad}"
            );
        }
        // Duplicate uid lines are a conflict, not a last-one-wins.
        let dup = format!("---\nkey: AGE-9\nuid: {UID}\nuid: {UID}\nstatus: todo\n---\n# N\n");
        assert!(parse_issue_file("AGE-9", &dup).is_err());
    }

    #[test]
    fn splice_uid_preserves_everything_it_did_not_insert() {
        // Trailing comments and odd spacing survive, which a parse/serialize
        // round-trip would have eaten.
        let text = "---\nkey: AGE-14\nstatus: todo    # backlog|todo|done\n---\n# T\n\nBody\n";
        let out = splice_uid(text, UID).unwrap();
        assert_eq!(
            out,
            format!(
                "---\nkey: AGE-14\nuid: {UID}\nstatus: todo    # backlog|todo|done\n\
---\n# T\n\nBody\n"
            )
        );
        // And the result is still parseable, with the uid where we put it.
        assert_eq!(parse_issue_file("AGE-14", &out).unwrap().uid.as_deref(), Some(UID));

        // CRLF stays CRLF — a Windows checkout must not be rewritten wholesale.
        let crlf = "---\r\nkey: AGE-1\r\nstatus: todo\r\n---\r\n# T\r\n";
        let out = splice_uid(crlf, UID).unwrap();
        assert_eq!(
            out,
            format!("---\r\nkey: AGE-1\r\nuid: {UID}\r\nstatus: todo\r\n---\r\n# T\r\n")
        );
    }

    /// The invariant behind letting `reconcile` rewrite a user's file: the
    /// splice adds a uid and changes *nothing else* the parser can see. Run
    /// over the shapes a real tracker actually contains — trailing comments, a
    /// comment thread, links, dates, and frontmatter keys this build does not
    /// know — because those are what a lossy rewrite would quietly eat.
    #[test]
    fn splice_uid_changes_nothing_but_the_uid() {
        let cases = [
            EXAMPLE,
            "---\nkey: AGE-5\nstatus: done\npriority: 4\ndue: 2026-08-01\n\
scheduled: 2026-07-30\nrank: 2.5\nlinks: AGE-1, AGE-2\n\
created: 2026-07-27T09:30:00Z\nupdated: 2026-07-27T14:02:00Z\n\
owner: nic\nepic: platform\n---\n# Five\n\nBody with an ![](assets/a.png).\n\
\n## Sam · 2026-07-27T15:10:00Z\n\nFirst.\n\
\n## Ada · 2026-07-28T09:00:00Z\n\nSecond.\n",
            // No body, no comments — the minimum a file can be.
            "---\nkey: AGE-2\nstatus: todo\n---\n# Two\n",
        ];
        for text in cases {
            let key = &text[text.find("key: ").unwrap() + 5..];
            let key = &key[..key.find('\n').unwrap()];
            let before = parse_issue_file(key, text).unwrap();
            assert_eq!(before.uid, None, "fixture already has a uid: {key}");
            let after = parse_issue_file(key, &splice_uid(text, UID).unwrap()).unwrap();
            assert_eq!(after.uid.as_deref(), Some(UID));
            assert_eq!(IssueFile { uid: None, ..after }, before, "splice altered {key}");
        }
    }

    #[test]
    fn splice_uid_declines_when_there_is_nothing_to_do() {
        let has = format!("---\nkey: AGE-1\nuid: {UID}\nstatus: todo\n---\n# T\n");
        assert_eq!(splice_uid(&has, UID), None);
        assert_eq!(splice_uid("# no frontmatter here\n", UID), None);
    }

    #[test]
    fn reconcile_backfills_a_uid_and_converges() {
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();

        write_issue(root, "AGE-7", "todo", "Seven");
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!(s.backfilled, 1);
        let id = reg.list_issues("p1").unwrap()[0].id.clone();
        let on_disk = std::fs::read_to_string(issue_path(root, "AGE-7")).unwrap();
        assert!(on_disk.contains(&format!("uid: {id}\n")), "uid not written: {on_disk}");

        // Second pass has nothing to write: the backfill is a one-time,
        // converging write, not churn on every issue-touching call.
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!((s.backfilled, s.imported, s.updated), (0, 0, 0));

        // A file that loses its uid (an old build's write, a hand edit) adopts
        // the id the row already holds rather than minting a new one — this is
        // what keeps `runs.issue_id` pointing at the same issue.
        write_issue(root, "AGE-7", "todo", "Seven");
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!(s.backfilled, 1);
        assert_eq!(reg.list_issues("p1").unwrap()[0].id, id);
        assert!(std::fs::read_to_string(issue_path(root, "AGE-7"))
            .unwrap()
            .contains(&format!("uid: {id}\n")));
    }

    #[test]
    fn reconcile_imports_under_the_uid_the_file_carries() {
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();

        // The whole point: this file's identity was minted on another machine,
        // and importing it here must not invent a second one for it.
        write_issue_uid(root, "AGE-3", UID);
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!((s.imported, s.backfilled), (1, 0));
        assert_eq!(reg.list_issues("p1").unwrap()[0].id, UID);
    }

    #[test]
    fn reconcile_mints_a_fresh_id_when_another_project_holds_the_uid() {
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();

        // Two checkouts of one repo, sharing a backlog and so sharing uids.
        // The second used to fail the whole pass on `UNIQUE constraint failed:
        // issues.id` and index nothing.
        write_issue_uid(one.path(), "AGE-3", UID);
        write_issue_uid(two.path(), "AGE-3", UID);
        let s = reconcile(&reg, "p1", "AGE", one.path()).unwrap();
        assert_eq!((s.imported, s.remapped), (1, 0));
        let s = reconcile(&reg, "p2", "AGE", two.path()).unwrap();
        assert_eq!((s.imported, s.remapped), (1, 1));

        assert_eq!(reg.list_issues("p1").unwrap()[0].id, UID);
        let theirs = reg.list_issues("p2").unwrap()[0].id.clone();
        assert_ne!(theirs, UID);
        // The file keeps the uid it came with: it belongs to the checkout that
        // indexed it first, and rewriting it would break that one's links.
        assert!(std::fs::read_to_string(issue_path(two.path(), "AGE-3"))
            .unwrap()
            .contains(&format!("uid: {UID}\n")));
        // Stable across passes, and no second remap.
        let s = reconcile(&reg, "p2", "AGE", two.path()).unwrap();
        assert_eq!((s.imported, s.updated, s.remapped), (0, 0, 0));
        assert_eq!(reg.list_issues("p2").unwrap()[0].id, theirs);
    }

    #[test]
    fn reconcile_carries_a_renumbered_files_id_to_its_new_seq() {
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();

        write_issue_uid(root, "AGE-3", UID);
        reconcile(&reg, "p1", "AGE", root).unwrap();

        // Renumbered behind the app's back (a branch checkout bringing someone
        // else's rename in). The row at the old seq is dropped before the new
        // one is imported, so the uid is free and the issue keeps its identity
        // instead of failing the pass on `UNIQUE constraint failed: issues.id`.
        std::fs::remove_file(issue_path(root, "AGE-3")).unwrap();
        write_issue_uid(root, "AGE-20", UID);
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!((s.imported, s.dropped, s.remapped), (1, 1, 0));
        let rows = reg.list_issues("p1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].seq, rows[0].id.as_str()), (20, UID));
    }

    #[test]
    fn reconcile_survives_two_files_sharing_one_uid() {
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();

        // AGE-3.md copied to AGE-4.md, uid and all. One of them keeps the uid,
        // the other gets its own; neither takes the board down.
        write_issue_uid(root, "AGE-3", UID);
        write_issue_uid(root, "AGE-4", UID);
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!((s.imported, s.remapped), (2, 1));
        let rows = reg.list_issues("p1").unwrap();
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].id, rows[1].id);
    }

    #[test]
    fn reconcile_keeps_the_indexed_id_when_a_files_uid_disagrees() {
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();

        write_issue_uid(root, "AGE-3", UID);
        reconcile(&reg, "p1", "AGE", root).unwrap();

        // Same seq, different identity. Re-keying the row would dangle every
        // run pointing at it, so the row wins and the file is left as-is for a
        // sync pass (which has the common ancestor this does not) to resolve.
        let other = "11111111-2222-3333-4444-555555555555";
        write_issue_uid(root, "AGE-3", other);
        reconcile(&reg, "p1", "AGE", root).unwrap();
        let rows = reg.list_issues("p1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, UID);
        assert!(std::fs::read_to_string(issue_path(root, "AGE-3"))
            .unwrap()
            .contains(&format!("uid: {other}\n")));
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
        reconcile(&reg, "p1", "AGE", root).unwrap();
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
        reconcile(&reg, "p1", "AGE", root).unwrap();
        let id = reg.list_issues("p1").unwrap()[0].id.clone();

        // The file goes corrupt (say, a half-written agent edit): the row
        // stays, the skip is reported.
        std::fs::write(issue_path(root, "AGE-3"), "garbage").unwrap();
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!(s.dropped, 0);
        assert_eq!(s.skipped.len(), 1);
        assert_eq!(s.skipped[0].0, "AGE-3.md");
        let rows = reg.list_issues("p1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].title, "Three");
    }

    #[test]
    fn reconcile_ignores_foreign_prefix_files() {
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();

        // A foreign-prefix file shares seq 1 with the project's own file
        // (say, a repo cloned from someone whose project derived another
        // key). Only the project's own file becomes a row.
        write_issue(root, "AGE-1", "todo", "Ours");
        write_issue(root, "XYZ-1", "done", "Theirs");
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!((s.imported, s.updated, s.dropped), (1, 0, 0));
        let rows = reg.list_issues("p1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Ours");
        assert_eq!(rows[0].status, IssueStatus::Todo);

        // Deleting the project's file deletes the issue for good — the
        // foreign file must not resurrect it (or flip the row's content).
        std::fs::remove_file(issue_path(root, "AGE-1")).unwrap();
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!((s.imported, s.updated, s.dropped), (0, 0, 1));
        assert!(reg.list_issues("p1").unwrap().is_empty());

        // A corrupt foreign file is not ours to report either.
        std::fs::write(root.join(ISSUES_DIR).join("XYZ-2.md"), "garbage").unwrap();
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert!(s.skipped.is_empty());
    }

    /// Ignoring a file is a decision the caller has to be able to see. This
    /// went unreported for as long as it existed, which is how a sync that
    /// pulled a whole backlog into a project keyed differently reported
    /// success over an empty board.
    #[test]
    fn reconcile_reports_the_files_it_ignored() {
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();

        write_issue(root, "AGE-1", "todo", "Theirs one");
        write_issue(root, "AGE-2", "todo", "Theirs two");
        write_issue(root, "XYZ-9", "todo", "Someone else's");
        let s = reconcile(&reg, "p1", "AGN", root).unwrap();

        assert_eq!(s.imported, 0, "nothing here is this project's");
        // Most files first, so the caller can name the one that matters.
        assert_eq!(s.foreign, vec![("AGE".to_string(), 2), ("XYZ".to_string(), 1)]);
        assert_eq!(adoptable_key(0, &s.foreign), None, "two keys is not a decision to make alone");
        assert_eq!(adoptable_key(0, &s.foreign[..1]), Some("AGE"));
        assert_eq!(
            adoptable_key(3, &s.foreign[..1]),
            None,
            "a project with its own issues chooses"
        );
    }

    #[test]
    fn rekey_moves_only_the_keys_that_are_references() {
        let none = std::collections::BTreeMap::new();
        let text = "---\nkey: AGN-3\nlinks: AGN-4, AGE-1\n---\n# T\n\nSee [[AGN-4]] and AGN-40, \
                    not MAGN-3 or AGN-3x.\n";
        let got = rekey_text(text, "AGN", "AGE", &none);
        assert!(got.contains("key: AGE-3"));
        assert!(got.contains("links: AGE-4, AGE-1"), "a link to our own old key did not follow");
        assert!(got.contains("[[AGE-4]]"));
        assert!(got.contains("AGE-40"), "a longer seq is still a key");
        assert!(got.contains("MAGN-3"), "a word ending in the key was rewritten");
        assert!(got.contains("AGN-3x"), "a token that is not a key was rewritten");
    }

    /// The characters a key is made of also make up dates, uuids and
    /// filenames, and `-` is a word boundary. A project keyed `2026` had its
    /// timestamps rewritten on rename, which refused the rename with a message
    /// blaming the file and, for the comment headings, silently folded every
    /// comment into the body. An attachment path was rewritten the same way
    /// while the attachment itself stayed put.
    #[test]
    fn rekey_leaves_dates_uuids_headings_and_paths_alone() {
        let none = std::collections::BTreeMap::new();
        let text = "---\nkey: 2026-3\nuid: 20262026-1234-4123-8123-123456789012\nstatus: todo\n\
                    due: 2026-11-01\nupdated: 2026-11-01T00:00:00Z\n---\n# Fix 2026-4\n\n\
                    Broke on 2026-12-31. See [[2026-4]] and ![](assets/2026-3-shot.png) \
                    and 2026-3.png, also 2026-3.\n\n## nic · 2026-10-01T10:00:00Z\n\n\
                    Still 2026-4 here.\n";
        let got = rekey_text(text, "2026", "AGE", &none);
        assert!(got.contains("key: AGE-3"));
        assert!(got.contains("uid: 20262026-1234-4123-8123-123456789012"), "{got}");
        assert!(got.contains("due: 2026-11-01"), "{got}");
        assert!(got.contains("updated: 2026-11-01T00:00:00Z"), "{got}");
        assert!(got.contains("# Fix AGE-4"), "{got}");
        assert!(got.contains("Broke on 2026-12-31."), "a date in the body was rewritten: {got}");
        assert!(got.contains("[[AGE-4]]"), "{got}");
        assert!(got.contains("assets/2026-3-shot.png"), "an attachment path moved: {got}");
        assert!(got.contains("2026-3.png"), "a filename was rewritten: {got}");
        assert!(got.contains("also AGE-3."), "a reference ending a sentence was skipped: {got}");
        assert!(got.contains("## nic · 2026-10-01T10:00:00Z"), "a comment heading: {got}");
        assert!(got.contains("Still AGE-4 here."), "{got}");
        // The proof that matters: the rewritten file parses to the same issue
        // with its references moved and nothing else touched.
        let before = parse_issue_file("2026-3", text).unwrap();
        let after = parse_issue_file("AGE-3", &got).unwrap();
        assert_eq!(after.comments.len(), 1, "a comment folded into the body");
        assert_eq!(after.comments[0].created_at, before.comments[0].created_at);
        assert_eq!((after.due, after.updated_at), (before.due, before.updated_at));
        assert_eq!(after.uid, before.uid);
    }

    #[test]
    fn rekey_renumbers_the_seqs_it_is_told_to() {
        let renumber = [(7, 9)].into_iter().collect();
        let got = rekey_text(
            "---\nkey: AGN-8\nlinks: AGN-7\n---\n# T\n\nAGN-7 AGN-8\n",
            "AGN",
            "AGE",
            &renumber,
        );
        assert_eq!(got, "---\nkey: AGE-8\nlinks: AGE-9\n---\n# T\n\nAGE-9 AGE-8\n");
    }

    #[test]
    fn rekeying_files_is_all_or_nothing() {
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();
        write_issue(root, "AGN-1", "todo", "One");
        write_issue(root, "AGN-2", "todo", "Two");
        write_issue(root, "AGE-7", "todo", "Already theirs");

        let done = rekey_issue_files(root, "AGN", "AGE", 1).unwrap();
        assert_eq!(done.plan.moves.len(), 2);
        assert!(done.plan.renumbered.is_empty());
        assert!(issue_path(root, "AGE-1").exists() && issue_path(root, "AGE-2").exists());
        assert!(!issue_path(root, "AGN-1").exists(), "the old file was left behind");
        assert!(issue_path(root, "AGE-7").exists(), "a file we do not own was touched");
        let one =
            parse_issue_file("AGE-1", &std::fs::read_to_string(issue_path(root, "AGE-1")).unwrap())
                .unwrap();
        assert_eq!(one.key, "AGE-1");
        assert_eq!(one.title, "One");

        // A file under the old key that cannot be verified refuses the whole
        // rename: the half that could have moved must not move, or the backlog
        // ends up under two keys.
        write_issue(root, "AGE-3", "todo", "Three");
        std::fs::write(issue_path(root, "AGE-4"), "---\nkey: AGE-4\n---\n# no status\n").unwrap();
        let err = rekey_issue_files(root, "AGE", "AGN", 1).unwrap_err().to_string();
        assert!(err.contains("AGE-4.md"), "the refusal did not name the file: {err}");
        assert!(issue_path(root, "AGE-1").exists() && issue_path(root, "AGE-3").exists());
        assert!(!issue_path(root, "AGN-1").exists(), "a file moved despite the refusal");
    }

    /// Both sides of a shared backlog number from 1 by their own counters, so
    /// the rename that joins them collides on the low numbers as a rule. A
    /// refusal there left the user holding advice they could not follow.
    #[test]
    fn rekeying_renumbers_a_file_whose_number_is_taken() {
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();
        write_issue(root, "AGE-7", "todo", "Already theirs");
        write_issue(root, "AGN-7", "todo", "Collides");
        std::fs::write(
            issue_path(root, "AGN-8"),
            "---\nkey: AGN-8\nstatus: todo\nlinks: AGN-7\n---\n# Links to the collision\n\nSee [[AGN-7]].\n",
        )
        .unwrap();

        let plan = plan_rekey(root, "AGN", "AGE", 1).unwrap();
        assert_eq!(
            plan.renumbered,
            vec![KeyMove { from: "AGN-7".into(), to: "AGE-9".into() }],
            "the next free number is above every number in use under either key"
        );
        assert!(issue_path(root, "AGN-7").exists(), "planning moved something");

        let done = rekey_issue_files(root, "AGN", "AGE", 1).unwrap();
        assert_eq!(done.plan, plan);
        let seven = std::fs::read_to_string(issue_path(root, "AGE-7")).unwrap();
        assert!(seven.contains("Already theirs"), "the collision was overwritten");
        let nine =
            parse_issue_file("AGE-9", &std::fs::read_to_string(issue_path(root, "AGE-9")).unwrap())
                .unwrap();
        assert_eq!(nine.title, "Collides");
        let eight =
            parse_issue_file("AGE-8", &std::fs::read_to_string(issue_path(root, "AGE-8")).unwrap())
                .unwrap();
        assert_eq!(
            eight.links,
            vec!["AGE-9"],
            "a link to the renumbered issue still names the old number"
        );
        assert!(eight.body.contains("[[AGE-9]]"), "{}", eight.body);
        assert!(!issue_path(root, "AGN-7").exists() && !issue_path(root, "AGN-8").exists());

        // The project's own counter is a floor: a number a deleted issue once
        // carried is never handed to a different issue.
        write_issue(root, "AGN-1", "todo", "Late");
        write_issue(root, "AGE-1", "todo", "Taken");
        let plan = plan_rekey(root, "AGN", "AGE", 40).unwrap();
        assert_eq!(plan.renumbered[0].to, "AGE-40");

        // And the whole thing comes back on request, byte for byte.
        let before = std::fs::read_to_string(issue_path(root, "AGN-1")).unwrap();
        let done = rekey_issue_files(root, "AGN", "AGE", 40).unwrap();
        assert!(issue_path(root, "AGE-40").exists());
        done.restore().unwrap();
        assert!(!issue_path(root, "AGE-40").exists(), "restore left the new file");
        assert_eq!(std::fs::read_to_string(issue_path(root, "AGN-1")).unwrap(), before);
    }

    #[test]
    fn reconcile_keeps_rows_when_the_dir_is_unlistable() {
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();

        write_issue(root, "AGE-2", "todo", "Two");
        reconcile(&reg, "p1", "AGE", root).unwrap();
        let id = reg.list_issues("p1").unwrap()[0].id.clone();

        // The whole dir vanishes (a branch without the tracker checked out):
        // the rows — and with them the uuids runs.issue_id points at — stay.
        std::fs::remove_dir_all(root.join(ISSUES_DIR)).unwrap();
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!(s, ReconcileSummary::default());
        let rows = reg.list_issues("p1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);

        // The dir comes back (checkout returns): same row, same uuid.
        write_issue(root, "AGE-2", "done", "Two");
        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!((s.imported, s.updated, s.dropped), (0, 1, 0));
        assert_eq!(reg.list_issues("p1").unwrap()[0].id, id);

        // An empty index plus a missing dir really is an empty tracker.
        let fresh = tempfile::tempdir().unwrap();
        let s = reconcile(&reg, "p2", "AGE", fresh.path()).unwrap();
        assert_eq!(s, ReconcileSummary::default());
    }

    #[test]
    fn the_assets_dir_is_invisible_to_the_tracker() {
        // Attachments live in a subdirectory of the issues dir, so every pass
        // over that dir has to step around it: a stray `assets/AGE-9.md` must
        // not become an issue, and the directory itself must not be read,
        // reported as skipped, or counted in the poll signature.
        let db = tempfile::tempdir().unwrap();
        let reg = Registry::open(&db.path().join("r.db")).unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path();

        write_issue(root, "AGE-1", "todo", "One");
        let assets = root.join(ISSUES_DIR).join(ASSETS_DIR);
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("age-1-shot.png"), b"\x89PNG").unwrap();
        std::fs::write(assets.join("AGE-9.md"), "---\nkey: AGE-9\nstatus: todo\n---\n# Nested\n")
            .unwrap();

        let read = read_issue_dir(root).unwrap();
        assert_eq!(read.issues.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(), vec!["AGE-1"]);
        assert!(read.skipped.is_empty(), "assets reported as skipped: {:?}", read.skipped);

        let stats = scan_issue_stats(root).unwrap();
        assert_eq!(stats.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), vec!["AGE-1.md"]);

        let s = reconcile(&reg, "p1", "AGE", root).unwrap();
        assert_eq!((s.imported, s.updated, s.dropped), (1, 0, 0));
        assert_eq!(reg.list_issues("p1").unwrap().len(), 1);
        // The attachment is still on disk: reconcile never touches it.
        assert!(assets.join("age-1-shot.png").exists());
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
