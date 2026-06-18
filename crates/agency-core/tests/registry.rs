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
