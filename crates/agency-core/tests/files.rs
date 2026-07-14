use agency_core::files;
use std::fs;

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(dir.path().join("README.md"), "# hi\n").unwrap();
    fs::write(dir.path().join("bin.dat"), [0u8, 1, 2, 0, 3]).unwrap();
    dir
}

#[test]
fn list_dir_sorts_dirs_first_then_name() {
    let dir = fixture();
    let entries = files::list_dir(dir.path(), "").unwrap();
    let names: Vec<_> = entries.iter().map(|e| (e.name.as_str(), e.is_dir)).collect();
    assert_eq!(names, vec![("src", true), ("README.md", false), ("bin.dat", false)]);
}

#[test]
fn list_dir_reports_has_children_for_nonempty_dirs() {
    let dir = fixture();
    fs::create_dir(dir.path().join("empty")).unwrap();
    let entries = files::list_dir(dir.path(), "").unwrap();
    let src = entries.iter().find(|e| e.name == "src").unwrap();
    let empty = entries.iter().find(|e| e.name == "empty").unwrap();
    let readme = entries.iter().find(|e| e.name == "README.md").unwrap();
    assert!(src.has_children, "src holds main.rs");
    assert!(!empty.has_children, "empty dir has no children");
    assert!(!readme.has_children, "files never report children");
}

#[test]
fn create_file_and_dir_then_rename_and_trash() {
    let dir = fixture();
    files::create_dir(dir.path(), "sub").unwrap();
    assert!(dir.path().join("sub").is_dir());
    files::create_file(dir.path(), "sub/new.txt").unwrap();
    assert!(dir.path().join("sub/new.txt").is_file());
    // create_new refuses to clobber.
    assert!(files::create_file(dir.path(), "sub/new.txt").is_err());

    files::rename_path(dir.path(), "sub/new.txt", "sub/renamed.txt").unwrap();
    assert!(!dir.path().join("sub/new.txt").exists());
    assert!(dir.path().join("sub/renamed.txt").is_file());
    // rename refuses to clobber an existing destination.
    assert!(files::rename_path(dir.path(), "sub/renamed.txt", "README.md").is_err());

    files::trash_path(dir.path(), "sub/renamed.txt").unwrap();
    assert!(!dir.path().join("sub/renamed.txt").exists());
}

#[test]
fn mutations_reject_traversal() {
    let dir = fixture();
    assert!(files::create_file(dir.path(), "../evil.txt").is_err());
    assert!(files::create_dir(dir.path(), "../evil").is_err());
    assert!(files::rename_path(dir.path(), "README.md", "../escaped.md").is_err());
    assert!(files::trash_path(dir.path(), "../../etc/hosts").is_err());
}

#[test]
fn list_dir_reads_subdirectory() {
    let dir = fixture();
    let entries = files::list_dir(dir.path(), "src").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "main.rs");
    assert!(!entries[0].is_dir);
}

#[test]
fn read_file_returns_text() {
    let dir = fixture();
    let fc = files::read_file(dir.path(), "src/main.rs").unwrap();
    assert_eq!(fc.text, "fn main() {}\n");
    assert!(!fc.binary && !fc.too_large);
}

#[test]
fn read_file_flags_binary() {
    let dir = fixture();
    let fc = files::read_file(dir.path(), "bin.dat").unwrap();
    assert!(fc.binary);
    assert_eq!(fc.text, "");
}

#[test]
fn write_file_roundtrips() {
    let dir = fixture();
    files::write_file(dir.path(), "README.md", "# changed\n").unwrap();
    assert_eq!(fs::read_to_string(dir.path().join("README.md")).unwrap(), "# changed\n");
}

#[test]
fn rejects_parent_traversal() {
    let dir = fixture();
    assert!(files::read_file(dir.path(), "../secret").is_err());
    assert!(files::list_dir(dir.path(), "../").is_err());
    assert!(files::write_file(dir.path(), "../../x", "no").is_err());
}

#[test]
fn rejects_absolute_path() {
    let dir = fixture();
    assert!(files::read_file(dir.path(), "/etc/passwd").is_err());
}

#[test]
fn read_file_flags_invalid_utf8_as_binary() {
    let dir = fixture();
    // 0xFF/0xFE are never valid UTF-8 lead bytes, and there is no NUL byte —
    // so this must be caught by the UTF-8 check, not the NUL check.
    fs::write(dir.path().join("bad.txt"), [0xFFu8, 0xFE, 0x41, 0x42]).unwrap();
    let fc = files::read_file(dir.path(), "bad.txt").unwrap();
    assert!(fc.binary, "invalid non-NUL UTF-8 should be flagged binary");
    assert_eq!(fc.text, "");
}

#[cfg(unix)]
#[test]
fn rejects_symlink_escape() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.txt"), "secret\n").unwrap();
    // A symlink that lives inside root but points at a directory outside it.
    symlink(outside.path(), root.path().join("link")).unwrap();

    // Reading or listing through the escaping symlink must be rejected even
    // though the path is lexically clean (no `..`, not absolute).
    assert!(files::read_file(root.path(), "link/secret.txt").is_err());
    assert!(files::list_dir(root.path(), "link").is_err());
    // Writing through it must be rejected too.
    assert!(files::write_file(root.path(), "link/planted.txt", "no").is_err());
}

#[cfg(unix)]
#[test]
fn rejects_write_through_dangling_symlink() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    // A dangling symlink inside root whose target does not exist yet. fs::write
    // would follow it and CREATE the target outside root — a write-anywhere
    // primitive that canonicalize() can't catch (a dangling link won't resolve).
    let target = outside.path().join("evil.txt");
    symlink(&target, root.path().join("link")).unwrap();
    assert!(!target.exists());

    assert!(
        files::write_file(root.path(), "link", "pwned").is_err(),
        "writing through a dangling symlink must be rejected"
    );
    assert!(!target.exists(), "must not have written outside root");
}

#[cfg(unix)]
#[test]
fn allows_symlink_within_root() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("real.txt"), "hi\n").unwrap();
    // A symlink that stays inside root resolves and is allowed.
    symlink(root.path().join("real.txt"), root.path().join("alias.txt")).unwrap();
    let fc = files::read_file(root.path(), "alias.txt").unwrap();
    assert_eq!(fc.text, "hi\n");
}
