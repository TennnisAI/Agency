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

#[test]
fn find_dir_case_insensitive_matches_variants() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("Docs")).unwrap();
    assert_eq!(files::find_dir_case_insensitive(dir.path(), "docs").unwrap(), Some("Docs".into()));

    // An exact-case match wins over a variant. Only testable on a
    // case-sensitive filesystem (macOS default is case-insensitive, so
    // creating "docs" next to "Docs" fails there).
    if fs::create_dir(dir.path().join("docs")).is_ok() {
        assert_eq!(
            files::find_dir_case_insensitive(dir.path(), "docs").unwrap(),
            Some("docs".into())
        );
    }
}

#[test]
fn find_dir_case_insensitive_ignores_files_and_absence() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("docs"), "a file, not a dir").unwrap();
    assert_eq!(files::find_dir_case_insensitive(dir.path(), "docs").unwrap(), None);
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(files::find_dir_case_insensitive(empty.path(), "docs").unwrap(), None);
}

#[test]
fn read_markdown_corpus_walks_nested_md_only() {
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("docs");
    fs::create_dir_all(docs.join("guides/.hidden")).unwrap();
    fs::create_dir(docs.join("node_modules")).unwrap();
    fs::write(docs.join("index.md"), "# Index\n").unwrap();
    fs::write(docs.join("guides/setup.MD"), "# Setup\n").unwrap();
    fs::write(docs.join("guides/notes.markdown"), "notes\n").unwrap();
    fs::write(docs.join("guides/image.png"), [0u8, 1]).unwrap();
    fs::write(docs.join("guides/.hidden/skip.md"), "hidden\n").unwrap();
    fs::write(docs.join("node_modules/skip.md"), "dep\n").unwrap();

    let mut got = files::read_markdown_corpus(dir.path(), "docs").unwrap();
    got.sort_by(|a, b| a.path.cmp(&b.path));
    let paths: Vec<_> = got.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, vec!["guides/notes.markdown", "guides/setup.MD", "index.md"]);
    assert_eq!(got[2].text, "# Index\n");
    assert!(got.iter().all(|d| !d.too_large));
}

#[test]
fn scan_reports_every_folder_whatever_it_holds() {
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("docs");
    fs::create_dir_all(docs.join("ideas")).unwrap();
    fs::create_dir_all(docs.join("research/2026")).unwrap();
    fs::create_dir_all(docs.join("assets")).unwrap();
    fs::create_dir_all(docs.join("notes")).unwrap();
    fs::create_dir_all(docs.join(".hidden")).unwrap();
    fs::create_dir_all(docs.join("node_modules")).unwrap();
    fs::write(docs.join("index.md"), "# Index\n").unwrap();
    fs::write(docs.join("notes/a.md"), "# A\n").unwrap();
    // Finder mints these behind the user's back; one must not make a folder
    // they just created read as populated and vanish from the tree.
    fs::write(docs.join("ideas/.DS_Store"), [0u8]).unwrap();
    // The AGE-120 case: a folder filled with attachments used to drop out.
    fs::write(docs.join("assets/logo.png"), [0u8, 1]).unwrap();

    let scan = files::scan_markdown_stats(dir.path(), "docs").unwrap();
    let mut paths: Vec<_> = scan.files.iter().map(|f| f.path.as_str()).collect();
    paths.sort();
    assert_eq!(paths, vec!["index.md", "notes/a.md"]);
    // Empty, attachment-only, subfolder-only and note-bearing folders alike.
    // Hidden and node_modules folders are outside the corpus entirely.
    assert_eq!(
        scan.dirs,
        vec![
            "assets".to_string(),
            "ideas".to_string(),
            "notes".to_string(),
            "research".to_string(),
            "research/2026".to_string(),
        ]
    );
}

#[test]
fn scan_reports_no_folder_row_for_the_docs_dir_itself() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("docs")).unwrap();
    let scan = files::scan_markdown_stats(dir.path(), "docs").unwrap();
    assert!(scan.files.is_empty());
    assert!(scan.dirs.is_empty());
}

#[test]
fn read_markdown_corpus_rejects_traversal_and_flags_oversize() {
    let dir = tempfile::tempdir().unwrap();
    assert!(files::read_markdown_corpus(dir.path(), "../elsewhere").is_err());

    let docs = dir.path().join("docs");
    fs::create_dir(&docs).unwrap();
    fs::write(docs.join("big.md"), "x".repeat(2_000_001)).unwrap();
    let got = files::read_markdown_corpus(dir.path(), "docs").unwrap();
    assert_eq!(got.len(), 1);
    assert!(got[0].too_large);
    assert_eq!(got[0].text, "");
}

#[test]
fn write_file_bytes_creates_and_refuses_clobber() {
    let dir = tempfile::tempdir().unwrap();
    files::write_file_bytes(dir.path(), "img.png", &[1u8, 2, 3]).unwrap();
    assert_eq!(fs::read(dir.path().join("img.png")).unwrap(), vec![1u8, 2, 3]);
    assert!(files::write_file_bytes(dir.path(), "img.png", &[9u8]).is_err());
    assert!(files::write_file_bytes(dir.path(), "../evil.png", &[1u8]).is_err());
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

/// The check behind "Mark resolved" for a file the conflict pane cannot read
/// into the UI. It has to answer for a file of any size, and it has to say no
/// to the things that merely look like markers.
#[test]
fn conflict_marker_scan_reads_files_the_pane_cannot() {
    let dir = tempfile::tempdir().unwrap();
    let conflicted = "a\n<<<<<<< HEAD\nmine\n=======\ntheirs\n>>>>>>> agent/feature\n";
    fs::write(dir.path().join("small.txt"), conflicted).unwrap();
    assert!(files::has_conflict_markers(dir.path(), "small.txt").unwrap());

    // Over the 2 MB ceiling read_file refuses at, which is where a conflicted
    // lockfile lands and where this check is the only one there is.
    let mut big = "key: value\n".repeat(400_000);
    assert!(big.len() > 2_000_000);
    big.push_str(">>>>>>> agent/feature\n");
    fs::write(dir.path().join("big.yaml"), &big).unwrap();
    assert!(files::has_conflict_markers(dir.path(), "big.yaml").unwrap());
    assert!(files::read_file(dir.path(), "big.yaml").unwrap().too_large);

    // A resolved file, a heading underline, a marker with something joined to
    // it, and one that does not start the line: none of them are markers.
    fs::write(dir.path().join("resolved.md"), "Changes\n=======\ntext\n").unwrap();
    fs::write(dir.path().join("joined.txt"), ">>>>>>>>text\n  <<<<<<< HEAD\n").unwrap();
    assert!(!files::has_conflict_markers(dir.path(), "resolved.md").unwrap());
    assert!(!files::has_conflict_markers(dir.path(), "joined.txt").unwrap());

    // A CRLF checkout, where every marker git wrote ends in a carriage return.
    fs::write(dir.path().join("crlf.txt"), "a\r\n<<<<<<< HEAD\r\nmine\r\n").unwrap();
    assert!(files::has_conflict_markers(dir.path(), "crlf.txt").unwrap());

    // Binary, and containment: the same rules the rest of this module keeps.
    fs::write(dir.path().join("bin.dat"), [0u8, 1, 2, 0, 3]).unwrap();
    assert!(!files::has_conflict_markers(dir.path(), "bin.dat").unwrap());
    assert!(files::has_conflict_markers(dir.path(), "../../etc/hosts").is_err());
}
