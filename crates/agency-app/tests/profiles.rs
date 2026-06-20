use agency_app_lib::AppState;

#[test]
fn seeds_three_agent_profiles_with_bare_commands() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    let profiles = state.list_profiles().unwrap();
    let by_name = |n: &str| profiles.iter().find(|p| p.name == n).cloned();

    let claude = by_name("claude").expect("claude profile");
    assert_eq!(claude.command, "claude");
    assert!(claude.args.is_empty(), "claude must run interactively (no prompt arg)");

    let pi = by_name("pi").expect("pi profile");
    assert_eq!(pi.command, "pi");
    assert!(pi.args.is_empty());

    let hermes = by_name("hermes").expect("hermes profile");
    assert_eq!(hermes.command, "hermes");
    assert!(hermes.args.is_empty());
}
