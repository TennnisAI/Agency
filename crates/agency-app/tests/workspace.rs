mod common;

use agency_core::setup::RepoReadiness;

// The pinned workspace (one-stop Phase 1): a project row with kind="workspace"
// and relaxed git requirements. Created lazily at a user-chosen folder.

#[test]
fn create_workspace_without_git_is_a_plain_folder_project() {
    let dir = tempfile::tempdir().unwrap();
    let loc = dir.path().join("Agency");

    let state = common::state(&dir);
    assert!(state.get_workspace().unwrap().is_none());

    let ws = state.create_workspace(&loc, false).unwrap();
    assert_eq!(ws.kind.as_deref(), Some("workspace"));
    // Phase 5: the workspace is in the tracker, so it has an issue key.
    assert!(ws.issue_key.is_some());
    assert!(loc.is_dir(), "the folder is created");
    assert!(matches!(state.inspect_repo(&loc), RepoReadiness::NotARepo));

    // Listed like any project, findable as the workspace, flagged for gating.
    assert!(state.list_projects().unwrap().iter().any(|p| p.id == ws.id));
    assert_eq!(state.get_workspace().unwrap().unwrap().id, ws.id);
    assert!(state.project_is_workspace(&ws.id).unwrap());
}

#[test]
fn create_workspace_with_git_is_ready_for_agents_and_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let loc = dir.path().join("Agency");
    std::fs::create_dir_all(&loc).unwrap();
    std::fs::write(loc.join("hello.md"), "# hi\n").unwrap();

    let state = common::state(&dir);
    let ws = state.create_workspace(&loc, true).unwrap();
    // init + initial commit ran: agents can branch from HEAD immediately.
    assert!(matches!(state.inspect_repo(&loc), RepoReadiness::Ready { dirty: false }));

    // Creating again (any flag) reuses the row and never re-inits.
    let again = state.create_workspace(&loc, false).unwrap();
    assert_eq!(again.id, ws.id);
    assert_eq!(state.list_projects().unwrap().iter().filter(|p| p.kind.is_some()).count(), 1);
}

#[test]
fn move_workspace_renames_folder_and_repoints_row() {
    let dir = tempfile::tempdir().unwrap();
    let loc = dir.path().join("Agency");
    let dest = dir.path().join("Notes");

    let state = common::state(&dir);
    state.create_workspace(&loc, false).unwrap();
    std::fs::write(loc.join("keep.md"), "x").unwrap();

    let moved = state.move_workspace(&dest).unwrap();
    assert_eq!(moved.repo_path, dest);
    assert!(dest.join("keep.md").exists());
    assert!(!loc.exists());

    // Refuses to clobber an existing destination.
    let other = dir.path().join("Occupied");
    std::fs::create_dir_all(&other).unwrap();
    assert!(state.move_workspace(&other).is_err());

    // Refuses a destination inside the workspace itself (a rename into a
    // subfolder of the source can never succeed and the picker allows it).
    let inside = dest.join("Sub").join("Notes");
    let err = state.move_workspace(&inside).unwrap_err().to_string();
    assert!(err.contains("inside the current workspace"), "got: {err}");
    assert!(dest.join("keep.md").exists(), "nothing moved");
}
