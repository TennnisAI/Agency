use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

/// Files larger than this are reported as `too_large` rather than read.
const MAX_FILE_BYTES: u64 = 2_000_000;

/// Cap for raw (binary) reads used by previews — images/PDFs run bigger than
/// source files, but a preview still shouldn't drag hundreds of MB over IPC.
const MAX_BINARY_BYTES: u64 = 25_000_000;

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
fn resolve_within(root: &Path, rel: &str) -> Result<PathBuf> {
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

    let real_root = root
        .canonicalize()
        .map_err(|e| anyhow!("cannot resolve root {}: {e}", root.display()))?;

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
    let parent = candidate
        .parent()
        .ok_or_else(|| anyhow!("path has no parent: {rel}"))?;
    let real_parent = parent
        .canonicalize()
        .map_err(|e| anyhow!("cannot resolve parent of {rel}: {e}"))?;
    if !real_parent.starts_with(&real_root) {
        bail!("path escapes root via symlink: {rel}");
    }
    // ...and the final component must not itself be a symlink, which `fs::write`
    // would follow out of the root (e.g. a dangling symlink pointing outside).
    // This is deliberately conservative: a symlink whose target merely doesn't
    // exist yet is rejected even if it points within root — safe over permissive,
    // and the editor never needs to write through an unresolved symlink.
    if candidate
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
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
    std::fs::read_dir(dir)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
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
