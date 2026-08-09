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
fn migration_exports_sqlite_issues_to_local_files_once() {
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

    // Invisible to git: the app rewrites these files constantly, and a merge
    // refuses to start on a checkout they have dirtied.
    let out =
        Command::new("git").args(["status", "--porcelain"]).current_dir(&repo).output().unwrap();
    let status = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(status.trim(), "", "issue files left the checkout dirty");

    // Second pass is a no-op (no duplicate export, same rows).
    let again = state.list_issues(&project_id).unwrap();
    assert_eq!(again.len(), 2);
    assert_eq!(again[0].id, issues[0].id);

    // Removing the project leaves the files: they are the tracker, not an
    // artifact of the app's database.
    state.delete_project(&project_id).unwrap();
    assert!(d.join("DEM-1.md").exists());
}

/// A project carried over from when issue files were committed: the first
/// issue-touching call untracks them, in a commit of its own, leaving the
/// checkout clean and the files on disk.
#[test]
fn previously_tracked_issue_files_are_untracked_on_first_use() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let git = |args: &[&str]| {
        let out = Command::new("git").args(args).current_dir(&repo).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let d = issues_dir(&repo);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("DEM-1.md"),
        "---\nkey: DEM-1\nstatus: todo\npriority: 2\n---\n# Committed issue\n",
    )
    .unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "issues"]);
    assert!(git(&["ls-files", "--", ".agency/issues"]).contains("DEM-1.md"));

    let state = common::state(&dir);
    let p = state.add_project("demo", &repo).unwrap();
    let issues = state.list_issues(&p.id).unwrap();
    assert_eq!(issues.len(), 1, "the committed file is still the tracker");

    assert!(git(&["ls-files", "--", ".agency/issues"]).trim().is_empty());
    assert!(d.join("DEM-1.md").exists());
    assert_eq!(git(&["status", "--porcelain"]).trim(), "");
    // Exactly one migration commit, and it only touched the issues dir.
    assert_eq!(git(&["rev-list", "--count", "HEAD"]).trim(), "3");
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
        .update_issue(
            &issue.id,
            &agency_core::registry::IssuePatch {
                body: Some("new body".into()),
                ..Default::default()
            },
        )
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
fn create_never_clobbers_an_unreconciled_hand_filed_issue() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    let p = state.add_project("demo", &repo).unwrap();
    let d = issues_dir(&repo);

    // Hand-filed issues the counter has never heard of (no list_issues has
    // run, so no reconcile raised the high-water mark). The next create must
    // skip past them, not overwrite them.
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("DEM-1.md"), "---\nkey: DEM-1\nstatus: todo\n---\n# Hand one\n").unwrap();
    std::fs::write(d.join("DEM-2.md"), "---\nkey: DEM-2\nstatus: todo\n---\n# Hand two\n").unwrap();
    let created = state.create_issue(&p.id, "App issue", "", IssueStatus::Todo).unwrap();
    assert_eq!(created.seq, 3, "numbers taken on disk are skipped");
    assert!(std::fs::read_to_string(d.join("DEM-1.md")).unwrap().contains("# Hand one"));
    assert!(std::fs::read_to_string(d.join("DEM-2.md")).unwrap().contains("# Hand two"));
    assert!(std::fs::read_to_string(d.join("DEM-3.md")).unwrap().contains("# App issue"));
    assert_eq!(state.list_issues(&p.id).unwrap().len(), 3);
}

#[test]
fn titles_normalize_and_priority_validates() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    let p = state.add_project("demo", &repo).unwrap();
    let d = issues_dir(&repo);

    // A pasted multi-line title collapses to one line — the title is one H1
    // line in the file, so a raw newline would smear it into the body.
    let issue = state.create_issue(&p.id, " Multi\nline\ttitle ", "", IssueStatus::Todo).unwrap();
    assert_eq!(issue.title, "Multi line title");
    let text = std::fs::read_to_string(d.join("DEM-1.md")).unwrap();
    let parsed = agency_core::issuefs::parse_issue_file("DEM-1", &text).unwrap();
    assert_eq!(parsed.title, "Multi line title");
    assert_eq!(parsed.body, "");

    let patch = agency_core::registry::IssuePatch {
        title: Some("Renamed\nagain".into()),
        ..Default::default()
    };
    assert_eq!(state.update_issue(&issue.id, &patch).unwrap().title, "Renamed again");

    // A priority the file format rejects never reaches the file.
    let patch = agency_core::registry::IssuePatch { priority: Some(9), ..Default::default() };
    assert!(state.update_issue(&issue.id, &patch).is_err());
    let after = &state.list_issues(&p.id).unwrap()[0];
    assert_eq!(after.priority, 0, "failed patch mutated state");
    assert!(
        agency_core::issuefs::parse_issue_file(
            "DEM-1",
            &std::fs::read_to_string(d.join("DEM-1.md")).unwrap()
        )
        .is_ok(),
        "the app wrote a file its own parser rejects"
    );
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
        text.contains("due: 2026-08-01")
            && text.contains("scheduled: 2026-07-30")
            && text.contains("rank: 1.5"),
        "{text}"
    );

    // Absent fields stay put; explicit clear removes key from file and row.
    let updated = state
        .update_issue(
            &issue.id,
            &agency_core::registry::IssuePatch { due: Some(None), ..Default::default() },
        )
        .unwrap();
    assert_eq!(updated.due, None);
    assert_eq!(updated.scheduled.as_deref(), Some("2026-07-30"), "absent field was touched");
    let text = std::fs::read_to_string(d.join("DEM-1.md")).unwrap();
    assert!(!text.contains("due:"), "cleared key still in file: {text}");
    assert!(text.contains("scheduled: 2026-07-30"), "{text}");

    // Bad values are rejected before anything is written.
    for patch in [
        agency_core::registry::IssuePatch {
            due: Some(Some("whenever".into())),
            ..Default::default()
        },
        agency_core::registry::IssuePatch {
            due: Some(Some("2026-02-30".into())),
            ..Default::default()
        },
        agency_core::registry::IssuePatch { rank: Some(Some(f64::NAN)), ..Default::default() },
    ] {
        assert!(state.update_issue(&issue.id, &patch).is_err());
    }
    let after = state.list_issues(&p.id).unwrap();
    assert_eq!(after[0].scheduled.as_deref(), Some("2026-07-30"), "failed patch mutated state");
}

#[test]
fn comments_are_written_to_the_file_and_survive_edits_from_both_sides() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    let p = state.add_project("demo", &repo).unwrap();
    let d = issues_dir(&repo);
    let issue =
        state.create_issue(&p.id, "Discussed", "The description.", IssueStatus::Todo).unwrap();
    assert!(issue.comments.is_empty());

    let after = state.add_issue_comment(&issue.id, "  First thought.  ").unwrap();
    assert_eq!(after.comments.len(), 1);
    assert_eq!(after.comments[0].body, "First thought.");
    assert!(!after.comments[0].author.is_empty(), "comment went unsigned");
    let path = d.join("DEM-1.md");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("First thought."), "{text}");
    // The description is untouched by a comment, and still reads as the body.
    assert_eq!(after.body, "The description.");

    // A comment appended to the file by hand (an agent working the issue) is
    // read back, and is not lost by an edit made in the app meanwhile.
    let ts = "2026-08-08T09:00:00Z";
    std::fs::write(&path, format!("{}\n## agent · {ts}\n\nFrom the worktree.\n", text.trim_end()))
        .unwrap();
    let listed = state.list_issues(&p.id).unwrap();
    assert_eq!(listed[0].comments.len(), 2, "hand-written comment not indexed");
    assert_eq!(listed[0].comments[1].author, "agent");

    let edited = state
        .update_issue(
            &issue.id,
            &agency_core::registry::IssuePatch {
                body: Some("Rewritten description.".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(edited.comments.len(), 2, "a body edit dropped the thread");
    assert_eq!(edited.body, "Rewritten description.");

    // Edit and delete address a comment by its timestamp.
    let at = edited.comments[0].created_at;
    let updated = state.update_issue_comment(&issue.id, at, "Second thought.").unwrap();
    assert_eq!(updated.comments[0].body, "Second thought.");
    assert!(std::fs::read_to_string(&path).unwrap().contains("Second thought."));

    let deleted = state.delete_issue_comment(&issue.id, at).unwrap();
    assert_eq!(deleted.comments.len(), 1);
    assert_eq!(deleted.comments[0].author, "agent");
    assert!(!std::fs::read_to_string(&path).unwrap().contains("Second thought."));

    // Nothing to address, nothing written.
    assert!(state.update_issue_comment(&issue.id, at, "again").is_err());
    assert!(state.delete_issue_comment(&issue.id, at).is_err());
    assert!(state.add_issue_comment(&issue.id, "   ").is_err());
}

#[test]
fn links_patch_writes_the_file_and_survives_reconcile() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    let p = state.add_project("demo", &repo).unwrap();
    let d = issues_dir(&repo);
    let issue = state.create_issue(&p.id, "Linked", "", IssueStatus::Todo).unwrap();
    let other = state.create_issue(&p.id, "Other", "", IssueStatus::Todo).unwrap();
    assert!(issue.links.is_empty());

    // The patch replaces the whole set, and lands normalized in the file.
    let updated = state
        .update_issue(
            &issue.id,
            &agency_core::registry::IssuePatch {
                // Own key and a duplicate are dropped; case is normalized.
                links: Some(vec!["dem-2".into(), "AGE-9".into(), "DEM-2".into(), "DEM-1".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(updated.links, vec!["DEM-2".to_string(), "AGE-9".to_string()]);
    let text = std::fs::read_to_string(d.join("DEM-1.md")).unwrap();
    assert!(text.contains("links: DEM-2, AGE-9"), "{text}");

    // A link to an issue in another project (AGE-9 here, in no project at all)
    // is kept as written — the target may live in a tracker this app can't see.
    assert_eq!(state.list_issues(&p.id).unwrap()[0].links, vec!["DEM-2", "AGE-9"]);
    // And the other side of a link is just its own file's `links:` — nothing
    // in the index infers it.
    assert!(state.list_issues(&p.id).unwrap()[1].links.is_empty());
    assert_eq!(other.seq, 2);

    // Emptying the set removes the key from the file.
    let cleared = state
        .update_issue(
            &issue.id,
            &agency_core::registry::IssuePatch { links: Some(vec![]), ..Default::default() },
        )
        .unwrap();
    assert!(cleared.links.is_empty());
    let text = std::fs::read_to_string(d.join("DEM-1.md")).unwrap();
    assert!(!text.contains("links:"), "cleared key still in file: {text}");

    // A malformed key is refused before anything is written.
    assert!(state
        .update_issue(
            &issue.id,
            &agency_core::registry::IssuePatch {
                links: Some(vec!["not a key".into()]),
                ..Default::default()
            }
        )
        .is_err());

    // Files are truth: a link written by hand reaches the board through
    // reconcile, on the next list.
    let path = d.join("DEM-1.md");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("status: todo", "status: todo\nlinks: dem-2")).unwrap();
    let listed = state.list_issues(&p.id).unwrap();
    assert_eq!(listed[0].links, vec!["DEM-2"], "hand-written link not indexed");
}
