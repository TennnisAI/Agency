use agency_core::profile::AgentProfile;
use agency_core::registry::Registry;
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
    assert!(reg.get_setting("anthropic_api_key").unwrap().is_none());
    reg.set_setting("anthropic_api_key", "sk-test").unwrap();
    reg.set_setting("anthropic_api_key", "sk-updated").unwrap();
    assert_eq!(reg.get_setting("anthropic_api_key").unwrap().unwrap(), "sk-updated");
}
