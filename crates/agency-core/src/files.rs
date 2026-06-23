use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

/// Files larger than this are reported as `too_large` rather than read.
const MAX_FILE_BYTES: u64 = 2_000_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
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
/// Two layers of defense:
///  1. Lexical: reject parent-dir (`..`), absolute, and drive-prefix components.
///  2. Symlink-aware: canonicalize `root` and the deepest existing ancestor of
///     the target, and require the latter to remain within the former — so a
///     symlink that lives inside `root` but points outside it cannot be used to
///     escape. The target itself need not exist (so new files can be written);
///     in that case the nearest existing ancestor is checked.
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

    // Walk up to the closest path that exists on disk (the candidate may be a
    // file we are about to create), canonicalize it to resolve every symlink,
    // and confirm it is still under the real root.
    let mut probe = candidate.as_path();
    let resolved = loop {
        match probe.canonicalize() {
            Ok(p) => break p,
            Err(_) => match probe.parent() {
                Some(parent) => probe = parent,
                // Exhausted all ancestors (e.g. a dangling symlink): fall back to
                // the root itself. This is allowed — the subsequent I/O call will
                // fail naturally — and cannot escape, since nothing resolved out.
                None => break real_root.clone(),
            },
        }
    };
    if !resolved.starts_with(&real_root) {
        bail!("path escapes root via symlink: {rel}");
    }
    Ok(candidate)
}

/// List a single directory level. Directories sort before files; ties broken by name.
pub fn list_dir(root: &Path, rel: &str) -> Result<Vec<DirEntry>> {
    let dir = resolve_within(root, rel)?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        out.push(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: entry.file_type()?.is_dir(),
        });
    }
    out.sort_by(|a, b| (!a.is_dir, &a.name).cmp(&(!b.is_dir, &b.name)));
    Ok(out)
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
