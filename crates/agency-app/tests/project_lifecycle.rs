use agency_app_lib::AppState;

#[test]
fn close_keeps_records_delete_removes_them() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    // init a git repo so project ops are valid
    std::process::Command::new("git").arg("init").current_dir(&repo).output().unwrap();

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    let project = state.add_project("repo", &repo).unwrap();

    // close_project must succeed and keep the project record
    state.close_project(&project.id).unwrap();
    assert!(state.list_projects().unwrap().iter().any(|p| p.id == project.id),
        "close_project must keep the project");

    // delete_project must remove the project record
    state.delete_project(&project.id).unwrap();
    assert!(!state.list_projects().unwrap().iter().any(|p| p.id == project.id),
        "delete_project must remove the project");
}
