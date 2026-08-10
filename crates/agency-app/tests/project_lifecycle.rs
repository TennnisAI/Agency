mod common;

use agency_core::profile::AgentProfile;

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
    assert!(
        !state.list_projects().unwrap().iter().any(|p| p.id == project.id),
        "close_project must hide the project"
    );

    // re-adding the same path revives the closed project instead of duplicating it
    let revived = state.add_project("repo", &repo).unwrap();
    assert_eq!(revived.id, project.id, "re-add must revive the closed project");
    assert!(
        state.list_projects().unwrap().iter().any(|p| p.id == project.id),
        "revived project must be listed again"
    );

    // delete_project must remove the project record
    state.delete_project(&project.id).unwrap();
    assert!(
        !state.list_projects().unwrap().iter().any(|p| p.id == project.id),
        "delete_project must remove the project"
    );
}

/// AGE-50: deleting a project is the agent teardown once per agent in it, so it
/// is the slowest thing in the app and used to freeze the window with nothing
/// to look at. Both project teardowns now report the step they are on; this
/// pins the steps, and the position-in-the-sweep detail that keeps a dozen
/// agents from reading as one teardown restarting.
#[test]
fn tearing_a_project_down_reports_each_step() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "t@e.com"]);
    git(&["config", "user.name", "T"]);
    git(&["commit", "-q", "--allow-empty", "-m", "init"]);

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "noop".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "sleep 1".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("repo", &repo).unwrap();
    for prompt in ["a", "b"] {
        state
            .create_run_with_progress(&project.id, prompt, "noop", "main", None, true, |_| {})
            .unwrap();
    }

    // Closing only stops sessions: one step per agent, then the project itself.
    let mut steps = Vec::new();
    state
        .close_project_with_progress(&project.id, &mut |p| steps.push((p.phase, p.detail)))
        .unwrap();
    assert_eq!(
        steps.iter().map(|(phase, _)| phase.as_str()).collect::<Vec<_>>(),
        vec!["Stopping the agents", "Stopping the agents", "Closing the project"],
    );
    assert!(steps[0].1.starts_with("1 of 2: "), "{steps:?}");
    assert!(steps[1].1.starts_with("2 of 2: "), "{steps:?}");

    // Deleting adds the slow one — a worktree to unlink per agent.
    state.add_project("repo", &repo).unwrap(); // revive the closed project
    let mut steps = Vec::new();
    state.delete_project_with_progress(&project.id, &mut |p| steps.push(p.phase)).unwrap();
    assert_eq!(
        steps,
        vec![
            "Stopping the agents",
            "Removing the worktrees",
            "Stopping the agents",
            "Removing the worktrees",
            "Cleaning up",
        ],
    );
    assert!(
        !repo.join(".agency/worktrees").read_dir().map(|mut d| d.next().is_some()).unwrap_or(false),
        "delete_project must leave no worktrees behind"
    );
}
