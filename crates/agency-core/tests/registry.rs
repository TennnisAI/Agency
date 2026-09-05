use agency_core::profile::AgentProfile;
use agency_core::registry::{Registry, Run};
use std::path::Path;

#[test]
fn add_then_get_and_list_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(&dir.path().join("agency.db")).unwrap();

    let p = reg.add_project("demo", Path::new("/tmp/demo-repo")).unwrap();
    assert_eq!(p.name, "demo");
    assert_eq!(p.repo_path, Path::new("/tmp/demo-repo"));
    assert!(!p.id.is_empty());

    let fetched = reg.get_project(&p.id).unwrap().unwrap();
    assert_eq!(fetched, p);

    let all = reg.list_projects().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0], p);
}

#[test]
fn remove_project_deletes_it() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(&dir.path().join("agency.db")).unwrap();

    let p = reg.add_project("demo", Path::new("/tmp/demo-repo")).unwrap();
    reg.remove_project(&p.id).unwrap();

    assert!(reg.get_project(&p.id).unwrap().is_none());
    assert_eq!(reg.list_projects().unwrap().len(), 0);
}

#[test]
fn data_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agency.db");

    let id = {
        let reg = Registry::open(&db).unwrap();
        reg.add_project("demo", Path::new("/tmp/demo-repo")).unwrap().id
    };

    let reg2 = Registry::open(&db).unwrap();
    assert!(reg2.get_project(&id).unwrap().is_some());
}

#[test]
fn profiles_persist_and_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agency.db");
    let p = AgentProfile {
        name: "claude".into(),
        command: "claude".into(),
        args: vec!["{{prompt}}".into()],
        env: vec![("FOO".into(), "bar".into())],
        resume_args: None,
        loop_args: None,
    };
    {
        let reg = Registry::open(&db).unwrap();
        reg.upsert_profile(&p).unwrap();
    }
    let reg = Registry::open(&db).unwrap();
    assert_eq!(reg.get_profile("claude").unwrap().unwrap(), p);
    assert_eq!(reg.list_profiles().unwrap(), vec![p.clone()]);

    // upsert replaces by name
    let p2 = AgentProfile { command: "claude2".into(), ..p.clone() };
    reg.upsert_profile(&p2).unwrap();
    assert_eq!(reg.get_profile("claude").unwrap().unwrap().command, "claude2");
    assert_eq!(reg.list_profiles().unwrap().len(), 1);

    reg.delete_profile("claude").unwrap();
    assert!(reg.get_profile("claude").unwrap().is_none());
}

#[test]
fn settings_persist_and_upsert() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agency.db");
    let reg = Registry::open(&db).unwrap();
    assert!(reg.get_setting("some_key").unwrap().is_none());
    reg.set_setting("some_key", "value-1").unwrap();
    reg.set_setting("some_key", "value-2").unwrap();
    assert_eq!(reg.get_setting("some_key").unwrap().unwrap(), "value-2");
}

#[test]
fn runs_persist_list_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agency.db");
    let run = Run {
        id: "task-1".into(),
        project_id: "proj-1".into(),
        agent: "claude".into(),
        prompt: "do it".into(),
        base: "main".into(),
        branch: "agent/task-1".into(),
        created_at: 1000,
        port_base: None,
        archived_at: None,
        title: None,
        kind: "agent".into(),
        merge_target: None,
        race_id: Some("race-9".into()),
        loop_config: None,
        loop_state: None,
        issue_id: None,
        worktree: true,
        model: None,
        base_commit: None,
        pin_rank: None,
    };
    {
        let reg = Registry::open(&db).unwrap();
        reg.insert_run(&run).unwrap();
    }
    let reg = Registry::open(&db).unwrap();
    assert_eq!(reg.get_run("task-1").unwrap().unwrap(), run);
    assert_eq!(reg.list_runs("proj-1").unwrap(), vec![run.clone()]);
    assert_eq!(reg.list_runs("other").unwrap().len(), 0);

    reg.delete_run("task-1").unwrap();
    assert!(reg.get_run("task-1").unwrap().is_none());
}

#[test]
fn list_runs_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(&dir.path().join("agency.db")).unwrap();
    for (id, ts) in [("a", 1), ("b", 3), ("c", 2)] {
        reg.insert_run(&Run {
            id: id.into(),
            project_id: "p".into(),
            agent: "shell".into(),
            prompt: "".into(),
            base: "main".into(),
            branch: format!("agent/{id}"),
            created_at: ts,
            port_base: None,
            archived_at: None,
            title: None,
            kind: "agent".into(),
            merge_target: None,
            race_id: None,
            loop_config: None,
            loop_state: None,
            issue_id: None,
            worktree: true,
            model: None,
            base_commit: None,
            pin_rank: None,
        })
        .unwrap();
    }
    let ids: Vec<String> = reg.list_runs("p").unwrap().into_iter().map(|r| r.id).collect();
    assert_eq!(ids, vec!["b", "c", "a"]); // created_at desc
}

#[test]
fn new_projects_get_distinct_colors_until_palette_exhausts() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(&dir.path().join("colors.db")).unwrap();
    let mut colors = Vec::new();
    for i in 0..9 {
        let p = reg.add_project(&format!("p{i}"), std::path::Path::new("/tmp/x")).unwrap();
        colors.push(p.color.expect("assigned at add time"));
    }
    let unique: std::collections::HashSet<_> = colors.iter().collect();
    assert_eq!(unique.len(), 9, "first nine projects all differ: {colors:?}");
    // Tenth project cycles back to the least-used color rather than failing.
    let tenth = reg.add_project("p9", std::path::Path::new("/tmp/x")).unwrap();
    assert!(tenth.color.is_some());
}
