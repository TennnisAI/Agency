use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

/// Files larger than this are reported as `too_large` rather than read.
const MAX_FILE_BYTES: u64 = 2_000_000;

/// Cap for raw (binary) reads used by previews — images/PDFs run bigger than
/// source files, but a preview still shouldn't drag hundreds of MB over IPC.
pub(crate) const MAX_BINARY_BYTES: u64 = 25_000_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    /// For directories: whether they contain at least one entry, so the tree can
    /// show an expand arrow only on folders that have something to reveal. Always
    /// false for files. Best-effort: an unreadable directory reports false.
    pub has_children: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContents {
    pub text: String,
    pub binary: bool,
    pub too_large: bool,
}

/// Join `rel` onto `root` and return a path guaranteed to stay inside `root`.
///
/// Three layers of defense:
///  1. Lexical: reject parent-dir (`..`), absolute, and drive-prefix components.
///  2. If the target already resolves on disk, canonicalize it (following every
///     symlink) and require it to stay within the canonicalized root — so an
///     existing symlink pointing outside `root` cannot be read or overwritten.
///  3. If the target does not resolve (a new file to create, or a dangling
///     symlink), canonicalize its parent directory and require *that* to stay
///     within the root, then reject when the final component is itself a symlink.
///     Without this, `fs::write` would follow a dangling symlink and create the
///     file at its out-of-root target — a write-anywhere primitive.
pub(crate) fn resolve_within(root: &Path, rel: &str) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(c) => normalized.push(c),
            Component::CurDir => {}
            Component::ParentDir => bail!("path escapes root: {rel}"),
            Component::RootDir | Component::Prefix(_) => bail!("absolute path not allowed: {rel}"),
        }
    }
    let candidate = root.join(normalized);

    let real_root =
        root.canonicalize().map_err(|e| anyhow!("cannot resolve root {}: {e}", root.display()))?;

    // Case 2: the candidate exists (file, dir, or non-dangling symlink). Resolve
    // it fully and require containment.
    if let Ok(resolved) = candidate.canonicalize() {
        if !resolved.starts_with(&real_root) {
            bail!("path escapes root via symlink: {rel}");
        }
        return Ok(candidate);
    }

    // Case 3: the candidate does not resolve. Its parent must exist and stay
    // within the root once symlinks are resolved...
    let parent = candidate.parent().ok_or_else(|| anyhow!("path has no parent: {rel}"))?;
    let real_parent =
        parent.canonicalize().map_err(|e| anyhow!("cannot resolve parent of {rel}: {e}"))?;
    if !real_parent.starts_with(&real_root) {
        bail!("path escapes root via symlink: {rel}");
    }
    // ...and the final component must not itself be a symlink, which `fs::write`
    // would follow out of the root (e.g. a dangling symlink pointing outside).
    // This is deliberately conservative: a symlink whose target merely doesn't
    // exist yet is rejected even if it points within root — safe over permissive,
    // and the editor never needs to write through an unresolved symlink.
    if candidate.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        bail!("path is a symlink that does not resolve within root: {rel}");
    }
    Ok(candidate)
}

/// List a single directory level. Directories sort before files; ties broken by name.
pub fn list_dir(root: &Path, rel: &str) -> Result<Vec<DirEntry>> {
    let dir = resolve_within(root, rel)?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let is_dir = entry.file_type()?.is_dir();
        // Cheap emptiness probe: open the subdir and pull a single entry. Only
        // runs for directories, and only one level below what the user opened,
        // so browsing cost stays proportional to what's actually expanded.
        let has_children = is_dir && dir_has_entry(&entry.path());
        out.push(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir,
            has_children,
        });
    }
    out.sort_by(|a, b| (!a.is_dir, &a.name).cmp(&(!b.is_dir, &b.name)));
    Ok(out)
}

/// True if `dir` contains at least one entry. Unreadable directories (permission
/// denied, races) report false rather than erroring — the arrow is a hint, not a
/// guarantee, and expansion surfaces any real error.
fn dir_has_entry(dir: &Path) -> bool {
    std::fs::read_dir(dir).map(|mut it| it.next().is_some()).unwrap_or(false)
}

/// Create an empty file at `rel`. Fails if it already exists so an accidental
/// "New File" over an existing name can't truncate it.
pub fn create_file(root: &Path, rel: &str) -> Result<()> {
    let path = resolve_within(root, rel)?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| anyhow!("cannot create {}: {e}", path.display()))?;
    Ok(())
}

/// Create a directory at `rel` (its parent must already exist). Fails if the
/// path already exists.
pub fn create_dir(root: &Path, rel: &str) -> Result<()> {
    let path = resolve_within(root, rel)?;
    std::fs::create_dir(&path).map_err(|e| anyhow!("cannot create {}: {e}", path.display()))?;
    Ok(())
}

/// Append `rel` to the root's `.gitignore` as an anchored pattern, creating the
/// file if it doesn't exist. Anchoring with a leading slash ("/src/foo") means
/// the entry ignores this exact path rather than every same-named file in the
/// tree; directories get a trailing slash. A no-op when the identical pattern is
/// already present, so repeated use never piles up duplicates. Returns whether a
/// new line was written (false = already ignored).
pub fn add_to_gitignore(root: &Path, rel: &str) -> Result<bool> {
    let target = resolve_within(root, rel)?;
    let mut pattern = format!("/{}", rel.trim_start_matches('/'));
    if target.is_dir() {
        pattern.push('/');
    }
    let gi_path = root.join(".gitignore");
    let existing = std::fs::read_to_string(&gi_path).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == pattern) {
        return Ok(false);
    }
    let mut out = existing;
    // Ensure the previous entry is newline-terminated before appending ours.
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&pattern);
    out.push('\n');
    std::fs::write(&gi_path, out).map_err(|e| anyhow!("cannot write .gitignore: {e}"))?;
    Ok(true)
}

/// Rename/move `from` to `to`, both resolved within `root`. Refuses to clobber an
/// existing destination.
pub fn rename_path(root: &Path, from: &str, to: &str) -> Result<()> {
    let src = resolve_within(root, from)?;
    let dst = resolve_within(root, to)?;
    if dst.symlink_metadata().is_ok() {
        bail!("destination already exists: {to}");
    }
    std::fs::rename(&src, &dst).map_err(|e| anyhow!("cannot rename {from}{to}: {e}"))?;
    Ok(())
}

/// Move the file or directory at `rel` to the OS trash (recoverable), rather than
/// deleting it permanently.
pub fn trash_path(root: &Path, rel: &str) -> Result<()> {
    let path = resolve_within(root, rel)?;
    trash::delete(&path).map_err(|e| anyhow!("cannot delete {}: {e}", path.display()))?;
    Ok(())
}

/// Resolve `rel` to an absolute path within `root` (for "reveal in Finder" /
/// "copy path"). The path is validated by `resolve_within` and must exist.
pub fn abs_path(root: &Path, rel: &str) -> Result<PathBuf> {
    let path = resolve_within(root, rel)?;
    if path.symlink_metadata().is_err() {
        bail!("path does not exist: {rel}");
    }
    Ok(path)
}

/// A path printed by a program running in a root, resolved on disk.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedPath {
    /// Absolute, symlink-resolved path.
    pub abs_path: String,
    /// The same path relative to the root, when it lives inside it. `None` for
    /// anything outside — that opens with the OS rather than in the Files tab.
    pub rel_path: Option<String>,
    pub is_dir: bool,
}

/// Resolve a path as an agent printed it — the backing check behind clickable
/// paths in terminal output (`lib/termLinks.ts` finds the candidates).
///
/// Unlike `resolve_within`, this deliberately resolves *outside* the root too:
/// a terminal prints paths from anywhere, and `~/notes.md` or `/etc/hosts` is
/// as clickable in a bare terminal as a file in the checkout. Containment is
/// reported (`rel_path`) rather than enforced, so the caller can open what is
/// inside the root in the app and hand the rest to the OS.
///
/// Returns `None` for anything that doesn't exist, which is what keeps prose
/// that merely looks path-shaped ("e.g", "v1.2") from being underlined.
pub fn resolve_printed_path(root: &Path, text: &str, home: Option<&Path>) -> Option<LinkedPath> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let expanded = if text == "~" || text.starts_with("~/") {
        home?.join(text.trim_start_matches('~').trim_start_matches('/'))
    } else if text.starts_with('/') {
        PathBuf::from(text)
    } else {
        root.join(text)
    };
    // Canonicalizing is both the existence check and the symlink resolution, so
    // containment is judged on where the path really lands.
    let target = expanded.canonicalize().ok()?;
    let is_dir = target.is_dir();
    let rel = root
        .canonicalize()
        .ok()
        .and_then(|r| target.strip_prefix(r).ok().map(|p| p.to_string_lossy().into_owned()))
        // The root itself is inside the root, but "" is not a file the Files
        // tab can open; treat it as an outside hit and let the OS have it.
        .filter(|p| !p.is_empty());
    Some(LinkedPath { abs_path: target.to_string_lossy().into_owned(), rel_path: rel, is_dir })
}

/// Read a file's contents. Oversized files are flagged `too_large`; files
/// containing a NUL byte or any invalid UTF-8 are flagged `binary` (we only
/// return `text` for content that is genuinely valid UTF-8, so the editor never
/// silently rewrites replacement characters). In the binary/too-large cases
/// `text` is empty.
pub fn read_file(root: &Path, rel: &str) -> Result<FileContents> {
    let path = resolve_within(root, rel)?;
    let meta = std::fs::metadata(&path)?;
    if meta.len() > MAX_FILE_BYTES {
        return Ok(FileContents { text: String::new(), binary: false, too_large: true });
    }
    let bytes = std::fs::read(&path)?;
    // NUL is valid UTF-8 (U+0000) but a reliable binary signal, so check it
    // first; then require the rest to decode losslessly.
    if bytes.contains(&0u8) {
        return Ok(FileContents { text: String::new(), binary: true, too_large: false });
    }
    match String::from_utf8(bytes) {
        Ok(text) => Ok(FileContents { text, binary: false, too_large: false }),
        Err(_) => Ok(FileContents { text: String::new(), binary: true, too_large: false }),
    }
}

/// Write `contents` to the file at `rel` within `root`.
pub fn write_file(root: &Path, rel: &str, contents: &str) -> Result<()> {
    let path = resolve_within(root, rel)?;
    std::fs::write(&path, contents)?;
    Ok(())
}

/// Find a top-level directory whose name matches `name` case-insensitively and
/// return its actual on-disk name. An exact match wins over case variants;
/// among variants the alphabetically first is chosen so the result is stable.
pub fn find_dir_case_insensitive(root: &Path, name: &str) -> Result<Option<String>> {
    let want = name.to_ascii_lowercase();
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let actual = entry.file_name().to_string_lossy().into_owned();
        if actual.to_ascii_lowercase() == want {
            if actual == name {
                return Ok(Some(actual));
            }
            candidates.push(actual);
        }
    }
    candidates.sort();
    Ok(candidates.into_iter().next())
}

/// Write raw bytes to a new file at `rel`. Refuses to clobber an existing file
/// (the caller retries with a different name) and rejects oversized payloads.
pub fn write_file_bytes(root: &Path, rel: &str, bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 > MAX_BINARY_BYTES {
        bail!("file too large: {} bytes", bytes.len());
    }
    let path = resolve_within(root, rel)?;
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| anyhow!("cannot create {}: {e}", path.display()))?;
    f.write_all(bytes)?;
    Ok(())
}

/// Copy an outside file into `rel` under `root` — the drop/file-picker half of
/// attaching, where the UI gets a path rather than bytes and so cannot call
/// `write_file_bytes` itself.
///
/// The source is deliberately unconstrained: it is whatever the user dragged
/// out of Finder or chose in the picker, which is consent enough to read it
/// (the same consent the picker plugin already grants). Only the destination
/// is jailed — it goes through `write_file_bytes`, inheriting containment, the
/// no-clobber rule, and the size cap. `metadata` follows symlinks, so dropping
/// a Finder alias imports what it points at, which is what the user sees.
pub fn import_file(root: &Path, src: &Path, rel: &str) -> Result<()> {
    let meta = std::fs::metadata(src).map_err(|e| anyhow!("cannot read {}: {e}", src.display()))?;
    if meta.is_dir() {
        // Said plainly: this is what a user gets back for dropping a folder in.
        bail!("folders can't be added, only files");
    }
    if !meta.is_file() {
        bail!("not a file: {}", src.display());
    }
    // Checked here so an oversized file is rejected before it is read into
    // memory; write_file_bytes checks again on what actually arrived.
    if meta.len() > MAX_BINARY_BYTES {
        bail!("file too large: {} bytes", meta.len());
    }
    let bytes = std::fs::read(src).map_err(|e| anyhow!("cannot read {}: {e}", src.display()))?;
    write_file_bytes(root, rel, &bytes)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocFile {
    /// Path relative to the scanned directory, `/`-separated.
    pub path: String,
    pub text: String,
    pub too_large: bool,
}

/// Corpus caps: a docs folder is expected to be small; these guard against
/// pointing the scan at something pathological. Hitting a cap stops the walk
/// but still returns what was collected — the index is best-effort.
const MAX_CORPUS_FILES: usize = 2000;
const MAX_CORPUS_BYTES: u64 = 20_000_000;

/// Cap on reported empty folders, so a pathological tree can't flood the pane.
const MAX_CORPUS_DIRS: usize = 2000;

/// One pass over the corpus: every markdown file, plus every folder that holds
/// nothing at all.
struct Walk {
    /// `(rel_path, abs_path, metadata)` per markdown file.
    files: Vec<(String, std::path::PathBuf, std::fs::Metadata)>,
    /// Rel paths of folders with no visible entries, sorted. A folder with
    /// notes under it is implied by their paths; one holding only non-markdown
    /// files stays hidden, same as before. This is the "I just made it and it
    /// is still empty" case, which is otherwise invisible to a markdown walk.
    empty_dirs: Vec<String>,
}

/// Walk every markdown file under `rel_dir` (same skip rules everywhere the
/// corpus is touched: hidden dirs, symlinks, node_modules; file-count cap).
/// The shared base for the full read, the stat-only pass, and anything else
/// that must agree with them on what "the corpus" is.
///
/// Emptiness uses the same skip rules: a folder holding only `.DS_Store` reads
/// as empty, because Finder minting one behind the user's back must not make a
/// folder they just created vanish from the tree.
fn walk_markdown(root: &Path, rel_dir: &str) -> Result<Walk> {
    let base = resolve_within(root, rel_dir)?;
    let mut out = Walk { files: Vec::new(), empty_dirs: Vec::new() };
    let mut stack = vec![(base, String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(it) => it,
            Err(_) => continue, // unreadable subdir: skip, don't fail the corpus
        };
        let mut seen = false;
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            let skipped = name.starts_with('.') || name == "node_modules";
            seen |= !skipped;
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                if skipped {
                    continue;
                }
                stack.push((entry.path(), rel));
                continue;
            }
            if !ft.is_file() {
                continue; // symlinks (incl. symlinked dirs) are skipped entirely
            }
            let lower = name.to_ascii_lowercase();
            if !lower.ends_with(".md") && !lower.ends_with(".markdown") {
                continue;
            }
            if out.files.len() >= MAX_CORPUS_FILES {
                out.empty_dirs.sort();
                return Ok(out);
            }
            let Ok(meta) = entry.metadata() else { continue };
            out.files.push((rel, entry.path(), meta));
        }
        // The scanned dir itself is the tree root, never a row in it.
        if !seen && !prefix.is_empty() && out.empty_dirs.len() < MAX_CORPUS_DIRS {
            out.empty_dirs.push(prefix);
        }
    }
    out.empty_dirs.sort();
    Ok(out)
}

/// Recursively read every markdown file under `rel_dir` in one pass, for the
/// docs index (links, tags, search). Hidden directories, symlinked directories,
/// and node_modules are skipped; per-file and total caps apply. Files that
/// aren't valid UTF-8 are skipped; oversized ones are listed with `too_large`
/// set and empty text so the tree can still show them.
pub fn read_markdown_corpus(root: &Path, rel_dir: &str) -> Result<Vec<DocFile>> {
    let mut out = Vec::new();
    let mut total: u64 = 0;
    for (rel, abs, meta) in walk_markdown(root, rel_dir)?.files {
        if total >= MAX_CORPUS_BYTES {
            return Ok(out);
        }
        if meta.len() > MAX_FILE_BYTES {
            out.push(DocFile { path: rel, text: String::new(), too_large: true });
            continue;
        }
        let Ok(bytes) = std::fs::read(&abs) else { continue };
        let Ok(text) = String::from_utf8(bytes) else { continue };
        total += text.len() as u64;
        out.push(DocFile { path: rel, text, too_large: false });
    }
    Ok(out)
}

/// One corpus file's change signature: enough for a poll to decide whether the
/// body needs re-reading, at stat cost instead of read cost.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocStat {
    /// Path relative to the scanned directory, `/`-separated.
    pub path: String,
    /// Modification time, epoch milliseconds (0 when the platform won't say).
    pub mtime_ms: i64,
    pub size: u64,
}

/// What a docs poll gets back: a change signature per markdown file, and the
/// folders that hold nothing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocsScan {
    pub files: Vec<DocStat>,
    /// Rel paths of empty folders, sorted. The tree derives its folders from
    /// note paths, so without these a folder the user just created has nowhere
    /// to come from and vanishes the moment the view is rebuilt.
    pub empty_dirs: Vec<String>,
}

/// Stat-only pass over the markdown corpus — same walk, same skips, no body
/// reads. Steady-state polls diff this against their cache and fetch bodies
/// only for files that actually changed.
pub fn scan_markdown_stats(root: &Path, rel_dir: &str) -> Result<DocsScan> {
    let walk = walk_markdown(root, rel_dir)?;
    Ok(DocsScan {
        files: walk
            .files
            .into_iter()
            .map(|(rel, _, meta)| DocStat {
                path: rel,
                mtime_ms: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
                size: meta.len(),
            })
            .collect(),
        empty_dirs: walk.empty_dirs,
    })
}

/// Read a named subset of the markdown corpus (the poll's "these changed"
/// list). Paths resolve through the jail relative to `rel_dir`; files deleted
/// between the stat pass and this read are silently skipped — the next poll
/// reports them as removed. Same UTF-8 / too-large handling as the full read.
pub fn read_markdown_files(root: &Path, rel_dir: &str, paths: &[String]) -> Result<Vec<DocFile>> {
    let base = resolve_within(root, rel_dir)?;
    let mut out = Vec::new();
    let mut total: u64 = 0;
    for rel in paths.iter().take(MAX_CORPUS_FILES) {
        if total >= MAX_CORPUS_BYTES {
            break;
        }
        let path = resolve_within(&base, rel)?;
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.len() > MAX_FILE_BYTES {
            out.push(DocFile { path: rel.clone(), text: String::new(), too_large: true });
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(text) = String::from_utf8(bytes) else { continue };
        total += text.len() as u64;
        out.push(DocFile { path: rel.clone(), text, too_large: false });
    }
    Ok(out)
}

/// One `- [ ]` / `- [x]` checkbox found in the markdown corpus.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskHit {
    /// Path relative to the scanned directory, `/`-separated.
    pub path: String,
    /// 0-based line number of the task line.
    pub line: u32,
    pub checked: bool,
    /// The task's text with bullet and marker stripped, trimmed.
    pub text: String,
}

/// Parse `- [ ] text` / `- [x] text` (also `*`/`+` bullets, any indentation).
/// The 3-char marker must be followed by a space or end the line. Returns
/// (checked, text) or None.
fn parse_task_line(line: &str) -> Option<(bool, &str)> {
    let s = line.trim_start();
    let rest = ["- ", "* ", "+ "].iter().find_map(|p| s.strip_prefix(p))?;
    let checked = match rest.get(..3)? {
        "[ ]" => false,
        "[x]" | "[X]" => true,
        _ => return None,
    };
    let tail = &rest[3..];
    if !tail.is_empty() && !tail.starts_with(' ') {
        return None;
    }
    Some((checked, tail.trim()))
}

/// Collect every checkbox task in the markdown corpus under `rel_dir` — the
/// same walk (and therefore the same file set) as the docs index, skipping
/// fenced code blocks the way the frontend parser does. Feeds the Home Tasks
/// aggregation, so only task lines cross the IPC boundary, not corpus bodies.
pub fn scan_tasks(root: &Path, rel_dir: &str) -> Result<Vec<TaskHit>> {
    let mut out = Vec::new();
    let mut total: u64 = 0;
    for (rel, abs, meta) in walk_markdown(root, rel_dir)?.files {
        if total >= MAX_CORPUS_BYTES {
            break;
        }
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&abs) else { continue };
        let Ok(text) = String::from_utf8(bytes) else { continue };
        total += text.len() as u64;
        let mut in_fence = false;
        for (i, line) in text.split('\n').enumerate() {
            if line.starts_with("```") || line.starts_with("~~~") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            if let Some((checked, task)) = parse_task_line(line) {
                out.push(TaskHit {
                    path: rel.clone(),
                    line: i as u32,
                    checked,
                    text: task.to_string(),
                });
            }
        }
    }
    Ok(out)
}

/// Flip the checkbox on one task line, in place. The caller's view of the file
/// may be stale (agents and editors write concurrently), so the addressed line
/// is re-verified: it must still parse as a task in the opposite state of
/// `checked`. On a mismatch nothing is written and `false` comes back — the
/// caller refreshes. The write is atomic (temp + rename) so a reader never
/// sees a torn file.
pub fn toggle_task(
    root: &Path,
    rel_dir: &str,
    rel: &str,
    line: u32,
    checked: bool,
) -> Result<bool> {
    let base = resolve_within(root, rel_dir)?;
    let path = resolve_within(&base, rel)?;
    let text = std::fs::read_to_string(&path)?;
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(target) = lines.get(line as usize) else {
        return Ok(false);
    };
    let Some((was, _)) = parse_task_line(target) else {
        return Ok(false);
    };
    if was == checked {
        return Ok(false);
    }
    let marker_at = (target.len() - target.trim_start().len()) + 2;
    let mut new_line = String::with_capacity(target.len());
    new_line.push_str(&target[..marker_at]);
    new_line.push_str(if checked { "[x]" } else { "[ ]" });
    new_line.push_str(&target[marker_at + 3..]);
    let mut out: Vec<&str> = lines;
    out[line as usize] = &new_line;
    crate::issuefs::atomic_write(&path, &out.join("\n"))?;
    Ok(true)
}

#[derive(Debug, Clone)]
pub struct BinaryFile {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub too_large: bool,
}

/// MIME type by extension for the preview formats the UI knows how to render.
/// Everything else is `application/octet-stream`, which the UI treats as
/// "no preview".
pub fn mime_for(rel: &str) -> &'static str {
    let ext = Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "m4a" => "audio/mp4",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "ogv" => "video/ogg",
        _ => "application/octet-stream",
    }
}

/// Read a file's raw bytes for previewing (images, PDFs). Same containment
/// rules as `read_file`; oversized files come back empty with `too_large` set.
pub fn read_file_bytes(root: &Path, rel: &str) -> Result<BinaryFile> {
    let path = resolve_within(root, rel)?;
    let mime = mime_for(rel);
    let meta = std::fs::metadata(&path)?;
    if meta.len() > MAX_BINARY_BYTES {
        return Ok(BinaryFile { bytes: Vec::new(), mime, too_large: true });
    }
    Ok(BinaryFile { bytes: std::fs::read(&path)?, mime, too_large: false })
}

#[cfg(test)]
mod printed_path_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolves_paths_relative_to_the_root() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();

        let got = resolve_printed_path(dir.path(), "src/main.rs", None).unwrap();
        assert_eq!(got.rel_path.as_deref(), Some("src/main.rs"));
        assert!(!got.is_dir);
        assert!(got.abs_path.ends_with("src/main.rs"));

        // "./" prefixed and directory forms land the same way.
        assert_eq!(
            resolve_printed_path(dir.path(), "./src/main.rs", None).unwrap().rel_path.as_deref(),
            Some("src/main.rs"),
        );
        let d = resolve_printed_path(dir.path(), "src", None).unwrap();
        assert!(d.is_dir);
        assert_eq!(d.rel_path.as_deref(), Some("src"));
    }

    #[test]
    fn absolute_paths_inside_the_root_come_back_relative() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "x").unwrap();
        let abs = dir.path().canonicalize().unwrap().join("a.md");

        let got = resolve_printed_path(dir.path(), abs.to_str().unwrap(), None).unwrap();
        assert_eq!(got.rel_path.as_deref(), Some("a.md"));
    }

    #[test]
    fn paths_outside_the_root_resolve_without_a_relative_path() {
        let dir = tempdir().unwrap();
        let other = tempdir().unwrap();
        std::fs::write(other.path().join("elsewhere.txt"), "x").unwrap();
        let abs = other.path().join("elsewhere.txt");

        let got = resolve_printed_path(dir.path(), abs.to_str().unwrap(), None).unwrap();
        assert_eq!(got.rel_path, None);
        assert!(got.abs_path.ends_with("elsewhere.txt"));
    }

    #[test]
    fn expands_a_leading_tilde_against_the_given_home() {
        let home = tempdir().unwrap();
        std::fs::write(home.path().join("notes.md"), "x").unwrap();
        let root = tempdir().unwrap();

        let got = resolve_printed_path(root.path(), "~/notes.md", Some(home.path())).unwrap();
        assert_eq!(got.rel_path, None);
        assert!(got.abs_path.ends_with("notes.md"));
        // No home to expand against is a miss, not a path named "~".
        assert_eq!(resolve_printed_path(root.path(), "~/notes.md", None), None);
    }

    #[test]
    fn what_does_not_exist_is_not_a_link() {
        let dir = tempdir().unwrap();
        assert_eq!(resolve_printed_path(dir.path(), "e.g", None), None);
        assert_eq!(resolve_printed_path(dir.path(), "src/nope.rs", None), None);
        assert_eq!(resolve_printed_path(dir.path(), "", None), None);
        // The root itself exists but is nothing to open in the Files tab.
        assert_eq!(resolve_printed_path(dir.path(), ".", None).unwrap().rel_path, None);
    }
}

#[cfg(test)]
mod root_as_vault_tests {
    use super::*;
    use tempfile::tempdir;

    // The workspace project uses the *whole folder* as its docs vault, which
    // reaches these functions as the empty relative path. That must mean "the
    // root itself" — never an error, never an escape.

    #[test]
    fn resolve_within_empty_rel_is_the_root() {
        let dir = tempdir().unwrap();
        let got = resolve_within(dir.path(), "").unwrap();
        assert_eq!(got.canonicalize().unwrap(), dir.path().canonicalize().unwrap());
    }

    #[test]
    fn list_dir_empty_rel_lists_the_root() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "x").unwrap();
        std::fs::create_dir(dir.path().join("journal")).unwrap();
        let names: Vec<String> =
            list_dir(dir.path(), "").unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["journal", "a.md"]);
    }

    #[test]
    fn read_markdown_corpus_empty_rel_walks_the_root() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("top.md"), "# top").unwrap();
        std::fs::create_dir(dir.path().join("journal")).unwrap();
        std::fs::write(dir.path().join("journal/2026-07-28.md"), "# today").unwrap();
        // Hidden dirs are still skipped from the vault walk.
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/skip.md"), "no").unwrap();

        let mut paths: Vec<String> =
            read_markdown_corpus(dir.path(), "").unwrap().into_iter().map(|f| f.path).collect();
        paths.sort();
        assert_eq!(paths, vec!["journal/2026-07-28.md", "top.md"]);
    }
}

#[cfg(test)]
mod corpus_stats_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn stats_cover_the_same_files_as_the_corpus_read() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("docs/sub")).unwrap();
        std::fs::create_dir_all(root.join("docs/.hidden")).unwrap();
        std::fs::write(root.join("docs/a.md"), "# a").unwrap();
        std::fs::write(root.join("docs/sub/b.md"), "# b").unwrap();
        std::fs::write(root.join("docs/notes.txt"), "not markdown").unwrap();
        std::fs::write(root.join("docs/.hidden/c.md"), "skipped").unwrap();

        let mut stat_paths: Vec<String> =
            scan_markdown_stats(root, "docs").unwrap().files.into_iter().map(|s| s.path).collect();
        let mut corpus_paths: Vec<String> =
            read_markdown_corpus(root, "docs").unwrap().into_iter().map(|f| f.path).collect();
        stat_paths.sort();
        corpus_paths.sort();
        assert_eq!(stat_paths, corpus_paths);
        assert_eq!(stat_paths, vec!["a.md", "sub/b.md"]);

        // Signatures change when a file does.
        let before = scan_markdown_stats(root, "docs").unwrap();
        let a = before.files.iter().find(|s| s.path == "a.md").unwrap();
        assert_eq!(a.size, 3);
        assert!(a.mtime_ms > 0);
        std::fs::write(root.join("docs/a.md"), "# a grew").unwrap();
        let after = scan_markdown_stats(root, "docs").unwrap();
        assert_ne!(after.files.iter().find(|s| s.path == "a.md").unwrap().size, a.size);
    }

    #[test]
    fn read_markdown_files_reads_exactly_the_asked_subset() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/a.md"), "alpha").unwrap();
        std::fs::write(root.join("docs/b.md"), "beta").unwrap();

        let got = read_markdown_files(root, "docs", &["b.md".to_string(), "gone.md".to_string()])
            .unwrap();
        assert_eq!(got.len(), 1, "missing files are skipped, not errors");
        assert_eq!((got[0].path.as_str(), got[0].text.as_str()), ("b.md", "beta"));

        // Jail escape is an error, not a silent skip.
        assert!(read_markdown_files(root, "docs", &["../a.md".to_string()]).is_err());
    }
}

#[cfg(test)]
mod import_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn import_file_copies_in_and_refuses_to_clobber() {
        let outside = tempdir().unwrap();
        let src = outside.path().join("shot.png");
        std::fs::write(&src, b"\x89PNGbytes").unwrap();

        let root_dir = tempdir().unwrap();
        let root = root_dir.path();
        std::fs::create_dir_all(root.join(".agency/issues/assets")).unwrap();

        import_file(root, &src, ".agency/issues/assets/age-1-shot.png").unwrap();
        assert_eq!(
            std::fs::read(root.join(".agency/issues/assets/age-1-shot.png")).unwrap(),
            b"\x89PNGbytes"
        );

        // Second import to the same name fails rather than overwriting — the
        // caller retries with a counter suffix.
        assert!(import_file(root, &src, ".agency/issues/assets/age-1-shot.png").is_err());
    }

    #[test]
    fn import_file_rejects_escapes_directories_and_missing_sources() {
        let outside = tempdir().unwrap();
        let src = outside.path().join("a.png");
        std::fs::write(&src, b"x").unwrap();
        let root_dir = tempdir().unwrap();
        let root = root_dir.path();

        // The destination is jailed even though the source is not.
        assert!(import_file(root, &src, "../escaped.png").is_err());
        assert!(import_file(root, &src, "/tmp/escaped.png").is_err());
        // A dropped directory is not an attachment.
        assert!(import_file(root, outside.path(), "dir.png").is_err());
        // A source that isn't there reports rather than creating an empty file.
        assert!(import_file(root, &outside.path().join("nope.png"), "nope.png").is_err());
        assert!(!root.join("nope.png").exists());
    }
}

#[cfg(test)]
mod gitignore_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn add_to_gitignore_anchors_creates_and_dedupes() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("secret.env"), "x").unwrap();
        std::fs::create_dir(root.join("build")).unwrap();

        // First add creates the file with an anchored file pattern.
        assert!(add_to_gitignore(root, "secret.env").unwrap());
        let gi = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(gi, "/secret.env\n");

        // A directory gets a trailing slash.
        assert!(add_to_gitignore(root, "build").unwrap());
        let gi = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(gi, "/secret.env\n/build/\n");

        // Re-adding an existing pattern is a no-op and reports false.
        assert!(!add_to_gitignore(root, "secret.env").unwrap());
        assert_eq!(
            std::fs::read_to_string(root.join(".gitignore")).unwrap(),
            "/secret.env\n/build/\n"
        );
    }

    #[test]
    fn add_to_gitignore_appends_a_newline_when_missing() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "node_modules").unwrap(); // no trailing newline
        std::fs::write(root.join("a.log"), "x").unwrap();

        add_to_gitignore(root, "a.log").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join(".gitignore")).unwrap(),
            "node_modules\n/a.log\n"
        );
    }
}

#[cfg(test)]
mod task_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scan_tasks_finds_checkboxes_with_lines_and_text() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("docs")).unwrap();
        std::fs::write(
            root.join("docs/todo.md"),
            "# Todo\n\n- [ ] call the bank\n- [x] done thing\n  * [ ] indented star\n- not a task\n- [z] bad marker\n",
        )
        .unwrap();

        let hits = scan_tasks(root, "docs").unwrap();
        assert_eq!(
            hits,
            vec![
                TaskHit {
                    path: "todo.md".into(),
                    line: 2,
                    checked: false,
                    text: "call the bank".into()
                },
                TaskHit {
                    path: "todo.md".into(),
                    line: 3,
                    checked: true,
                    text: "done thing".into()
                },
                TaskHit {
                    path: "todo.md".into(),
                    line: 4,
                    checked: false,
                    text: "indented star".into()
                },
            ]
        );
    }

    #[test]
    fn scan_tasks_skips_fenced_blocks_and_non_corpus_files() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("note.md"), "```\n- [ ] in code\n```\n- [ ] real\n").unwrap();
        std::fs::write(root.join("script.sh"), "- [ ] not markdown\n").unwrap();
        std::fs::create_dir(root.join(".hidden")).unwrap();
        std::fs::write(root.join(".hidden/x.md"), "- [ ] hidden\n").unwrap();

        // Workspace-vault style: rel_dir "" walks the root.
        let hits = scan_tasks(root, "").unwrap();
        assert_eq!(
            hits,
            vec![TaskHit { path: "note.md".into(), line: 3, checked: false, text: "real".into() }]
        );
    }

    #[test]
    fn toggle_task_flips_in_place_and_preserves_the_rest() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let filler = "filler line\n".repeat(200);
        let text = format!("# T\n{filler}- [ ] the task  \ntail\n");
        std::fs::write(root.join("n.md"), &text).unwrap();

        assert!(toggle_task(root, "", "n.md", 201, true).unwrap());
        let after = std::fs::read_to_string(root.join("n.md")).unwrap();
        assert_eq!(after, text.replace("- [ ] the task  ", "- [x] the task  "));

        assert!(toggle_task(root, "", "n.md", 201, false).unwrap());
        assert_eq!(std::fs::read_to_string(root.join("n.md")).unwrap(), text);
    }

    #[test]
    fn toggle_task_refuses_stale_or_invalid_lines() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let text = "- [ ] a\nplain\n";
        std::fs::write(root.join("n.md"), text).unwrap();

        // Line is not a task.
        assert!(!toggle_task(root, "", "n.md", 1, true).unwrap());
        // Line out of range.
        assert!(!toggle_task(root, "", "n.md", 99, true).unwrap());
        // Already in the requested state (caller's view was stale).
        assert!(!toggle_task(root, "", "n.md", 0, false).unwrap());
        // Nothing was written by any refusal.
        assert_eq!(std::fs::read_to_string(root.join("n.md")).unwrap(), text);
    }
}
