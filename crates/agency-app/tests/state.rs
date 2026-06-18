use agency_app_lib::AppState;
use std::path::Path;

#[test]
fn new_seeds_default_shell_profile_and_version_holds() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    assert_eq!(AppState::version(), "0.1.0");
    assert!(state.profile_names().contains(&"shell".to_string()));
}

#[test]
fn project_crud_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(&dir.path().join("agency.db")).unwrap();

    let p = state.add_project("demo", Path::new("/tmp/demo-repo")).unwrap();
    assert_eq!(p.name, "demo");

    let all = state.list_projects().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, p.id);

    state.remove_project(&p.id).unwrap();
    assert_eq!(state.list_projects().unwrap().len(), 0);
}
