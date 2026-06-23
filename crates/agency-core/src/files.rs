use anyhow::{bail, Result};
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

/// Join `rel` onto `root`, rejecting any component that could escape `root`
/// (parent dirs, absolute paths, drive prefixes). Does not require the target
/// to exist, so it is safe for writing new files.
/// Note: this is a lexical check; it does not resolve symlinks, so a symlink
/// already inside `root` could still point outside it.
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
    Ok(root.join(normalized))
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
/// containing a NUL byte are flagged `binary`. In both cases `text` is empty.
pub fn read_file(root: &Path, rel: &str) -> Result<FileContents> {
    let path = resolve_within(root, rel)?;
    let meta = std::fs::metadata(&path)?;
    if meta.len() > MAX_FILE_BYTES {
        return Ok(FileContents { text: String::new(), binary: false, too_large: true });
    }
    let bytes = std::fs::read(&path)?;
    if bytes.contains(&0u8) {
        return Ok(FileContents { text: String::new(), binary: true, too_large: false });
    }
    Ok(FileContents {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        binary: false,
        too_large: false,
    })
}

/// Write `contents` to the file at `rel` within `root`.
pub fn write_file(root: &Path, rel: &str, contents: &str) -> Result<()> {
    let path = resolve_within(root, rel)?;
    std::fs::write(&path, contents)?;
    Ok(())
}
