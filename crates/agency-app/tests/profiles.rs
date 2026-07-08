mod common;

#[test]
fn seeds_builtin_resume_recipes() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    let profiles = state.list_profiles().unwrap();
    let get = |name: &str| profiles.iter().find(|p| p.name == name).cloned().unwrap();
    assert_eq!(get("claude").resume_args, Some(vec!["--continue".into()]));
    assert_eq!(get("codex").resume_args, Some(vec!["resume".into(), "--last".into()]));
    assert_eq!(get("pi").resume_args, Some(vec!["--continue".into()]));
    assert_eq!(get("opencode").resume_args, Some(vec!["--continue".into()]));
    assert_eq!(get("copilot").resume_args, Some(vec!["--continue".into()]));
    assert_eq!(get("cursor").command, "cursor-agent");
    assert_eq!(get("cursor").resume_args, None);
    assert_eq!(get("hermes").resume_args, None);
}

#[test]
fn seeds_three_agent_profiles_with_bare_commands() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
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
