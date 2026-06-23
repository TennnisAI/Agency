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
