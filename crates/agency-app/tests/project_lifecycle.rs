mod common;

#[test]
fn add_project_allows_repo_without_commits() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    // A git repo with NO commits — previously rejected, now must be addable (gated).
    std::process::Command::new("git").args(["init", "-q"]).current_dir(&repo).output().unwrap();

    let state = common::state(&dir);
    let p = state.add_project("repo", &repo).unwrap();
    assert_eq!(p.name, "repo");

    // Inspection reports it as not-ready (no commits yet).
    assert!(matches!(
        state.inspect_repo(&repo),
        agency_core::setup::RepoReadiness::NoCommits { .. }
    ));
}

#[test]
fn close_hides_project_readd_revives_delete_removes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    // init a git repo with one commit so project ops are valid
    let git = |args: &[&str]| {
        std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap()
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@e.com"]);
    git(&["config", "user.name", "T"]);
    git(&["commit", "-q", "--allow-empty", "-m", "init"]);

    let state = common::state(&dir);
    let project = state.add_project("repo", &repo).unwrap();

    // close_project hides the project from the list but keeps its record
    state.close_project(&project.id).unwrap();
    assert!(!state.list_projects().unwrap().iter().any(|p| p.id == project.id),
        "close_project must hide the project");

    // re-adding the same path revives the closed project instead of duplicating it
    let revived = state.add_project("repo", &repo).unwrap();
    assert_eq!(revived.id, project.id, "re-add must revive the closed project");
    assert!(state.list_projects().unwrap().iter().any(|p| p.id == project.id),
        "revived project must be listed again");

    // delete_project must remove the project record
    state.delete_project(&project.id).unwrap();
    assert!(!state.list_projects().unwrap().iter().any(|p| p.id == project.id),
        "delete_project must remove the project");
}
