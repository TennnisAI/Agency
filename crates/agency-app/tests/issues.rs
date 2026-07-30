//! Issues-as-files (one-stop Phase 5): the migration, the file-first write
//! path, and the files-are-truth reconcile, exercised through `AppState` the
//! way the Tauri commands drive it.

use std::path::Path;
use std::process::Command;

use agency_core::registry::{IssueStatus, Registry};

mod common;

fn init_repo(dir: &Path) {
    let run = |args: &[&str]| {
        assert!(
            Command::new("git").args(args).current_dir(dir).status().unwrap().success(),
            "git {:?}",
            args
        );
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "t@e.com"]);
    run(&["config", "user.name", "T"]);
    std::fs::write(dir.join("README.md"), "hi").unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "init"]);
}

fn issues_dir(repo: &Path) -> std::path::PathBuf {
    repo.join(".agency").join("issues")
}

#[test]
fn migration_exports_sqlite_issues_to_commitable_files_once() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    // A legacy project: issues exist as SQLite rows only (pre-Phase-5 state,
    // seeded through the registry the way old builds wrote it).
    let project_id = {
        let reg = Registry::open(&dir.path().join("agency.db")).unwrap();
        let p = reg.add_project("demo", &repo).unwrap();
        reg.create_issue(&p.id, "Legacy one", "with a body", IssueStatus::Todo, 100).unwrap();
        reg.create_issue(&p.id, "Legacy two", "", IssueStatus::Done, 200).unwrap();
        p.id
    };

    let state = common::state(&dir);
    let issues = state.list_issues(&project_id).unwrap();
    assert_eq!(issues.len(), 2);

    // The export happened: canonical files plus the README.
    let d = issues_dir(&repo);
    let one = std::fs::read_to_string(d.join("DEM-1.md")).unwrap();
    assert!(one.contains("key: DEM-1"), "{one}");
    assert!(one.contains("status: todo"), "{one}");
    assert!(one.contains("# Legacy one"), "{one}");
    assert!(one.contains("with a body"), "{one}");
    assert!(std::fs::read_to_string(d.join("DEM-2.md")).unwrap().contains("status: done"));
    assert!(d.join("README.md").exists());

    // Commit-able, not excluded: git sees the new files.
    let out = Command::new("git").args(["status", "--porcelain"]).current_dir(&repo).output().unwrap();
    let status = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(status.contains(".agency/"), "issue files invisible to git: {status}");

    // Second pass is a no-op (no duplicate export, same rows).
    let again = state.list_issues(&project_id).unwrap();
    assert_eq!(again.len(), 2);
    assert_eq!(again[0].id, issues[0].id);

    // Removing the project leaves the files: they are repo content.
    state.delete_project(&project_id).unwrap();
    assert!(d.join("DEM-1.md").exists());
}

#[test]
fn mutations_are_file_first_and_reconcile_follows_external_edits() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    let p = state.add_project("demo", &repo).unwrap();
    let d = issues_dir(&repo);

    // Create writes the file (and the README rides along with the dir).
    let issue = state.create_issue(&p.id, "  New thing  ", "some body", IssueStatus::Todo).unwrap();
    assert_eq!(issue.title, "New thing");
    assert_eq!(issue.seq, 1);
    let text = std::fs::read_to_string(d.join("DEM-1.md")).unwrap();
    assert!(text.contains("key: DEM-1") && text.contains("status: todo"), "{text}");
    assert!(text.contains("# New thing") && text.contains("some body"), "{text}");
    assert!(d.join("README.md").exists());

    // Update rewrites the file.
    let patch = agency_core::registry::IssuePatch {
        status: Some(IssueStatus::InProgress),
        priority: Some(3),
        ..Default::default()
    };
    let updated = state.update_issue(&issue.id, &patch).unwrap();
    assert_eq!(updated.id, issue.id);
    let text = std::fs::read_to_string(d.join("DEM-1.md")).unwrap();
    assert!(text.contains("status: in_progress") && text.contains("priority: 3"), "{text}");

    // An external (agent-style) edit is picked up by the next list — and
    // unknown frontmatter keys survive the app's next write.
    std::fs::write(
        d.join("DEM-1.md"),
        "---\nkey: DEM-1\nstatus: done\npriority: 3\ndue: 2026-08-01\nassignee: sam\n---\n# Renamed outside\n",
    )
    .unwrap();
    let listed = state.list_issues(&p.id).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, issue.id, "external edit must not change the row's id");
    assert_eq!(listed[0].status, IssueStatus::Done);
    assert_eq!(listed[0].title, "Renamed outside");
    assert_eq!(listed[0].due.as_deref(), Some("2026-08-01"), "known key must reach the row");
    let roundtrip = state
        .update_issue(&issue.id, &agency_core::registry::IssuePatch {
            body: Some("new body".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(roundtrip.body, "new body");
    let text = std::fs::read_to_string(d.join("DEM-1.md")).unwrap();
    assert!(text.contains("due: 2026-08-01"), "date lost on app write: {text}");
    assert!(text.contains("assignee: sam"), "unknown key lost on app write: {text}");

    // A hand-filed issue consumes its number: the next create skips past it.
    std::fs::write(d.join("DEM-9.md"), "---\nkey: DEM-9\nstatus: todo\n---\n# Filed by hand\n")
        .unwrap();
    assert_eq!(state.list_issues(&p.id).unwrap().len(), 2);
    let next = state.create_issue(&p.id, "After nine", "", IssueStatus::Todo).unwrap();
    assert_eq!(next.seq, 10);

    // Deleting the file deletes the issue; deleting an issue deletes its file.
    std::fs::remove_file(d.join("DEM-9.md")).unwrap();
    let keys: Vec<i64> = state.list_issues(&p.id).unwrap().iter().map(|i| i.seq).collect();
    assert_eq!(keys, vec![1, 10]);
    state.delete_issue(&next.id).unwrap();
    assert!(!d.join("DEM-10.md").exists());
    assert_eq!(state.list_issues(&p.id).unwrap().len(), 1);
}

#[test]
fn dates_and_rank_patch_set_clear_and_validate() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    let p = state.add_project("demo", &repo).unwrap();
    let d = issues_dir(&repo);
    let issue = state.create_issue(&p.id, "Dated", "", IssueStatus::Todo).unwrap();
    assert_eq!((issue.due.clone(), issue.scheduled.clone(), issue.rank), (None, None, None));

    // Set: file gains the keys, row matches.
    let patch = agency_core::registry::IssuePatch {
        due: Some(Some("2026-08-01".into())),
        scheduled: Some(Some("2026-07-30".into())),
        rank: Some(Some(1.5)),
        ..Default::default()
    };
    let updated = state.update_issue(&issue.id, &patch).unwrap();
    assert_eq!(updated.due.as_deref(), Some("2026-08-01"));
    assert_eq!(updated.scheduled.as_deref(), Some("2026-07-30"));
    assert_eq!(updated.rank, Some(1.5));
    let text = std::fs::read_to_string(d.join("DEM-1.md")).unwrap();
    assert!(
        text.contains("due: 2026-08-01") && text.contains("scheduled: 2026-07-30") && text.contains("rank: 1.5"),
        "{text}"
    );

    // Absent fields stay put; explicit clear removes key from file and row.
    let updated = state
        .update_issue(&issue.id, &agency_core::registry::IssuePatch {
            due: Some(None),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(updated.due, None);
    assert_eq!(updated.scheduled.as_deref(), Some("2026-07-30"), "absent field was touched");
    let text = std::fs::read_to_string(d.join("DEM-1.md")).unwrap();
    assert!(!text.contains("due:"), "cleared key still in file: {text}");
    assert!(text.contains("scheduled: 2026-07-30"), "{text}");

    // Bad values are rejected before anything is written.
    for patch in [
        agency_core::registry::IssuePatch { due: Some(Some("whenever".into())), ..Default::default() },
        agency_core::registry::IssuePatch { due: Some(Some("2026-02-30".into())), ..Default::default() },
        agency_core::registry::IssuePatch { rank: Some(Some(f64::NAN)), ..Default::default() },
    ] {
        assert!(state.update_issue(&issue.id, &patch).is_err());
    }
    let after = state.list_issues(&p.id).unwrap();
    assert_eq!(after[0].scheduled.as_deref(), Some("2026-07-30"), "failed patch mutated state");
}
