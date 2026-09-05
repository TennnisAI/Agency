mod common;

#[test]
fn fresh_db_seeds_no_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    assert!(state.list_profiles().unwrap().is_empty());
    assert!(state.agent_onboarding_needed().unwrap());
}

/// Terminals aren't agents. Older installs seeded a "shell" profile that showed
/// up as an editable card in Settings; opening the DB must drop it.
#[test]
fn legacy_shell_profile_is_removed_on_open() {
    let dir = tempfile::tempdir().unwrap();
    {
        let state = common::state(&dir);
        state
            .register_profile(agency_core::profile::AgentProfile {
                name: "shell".into(),
                command: "/bin/zsh".into(),
                args: vec!["-l".into()],
                env: vec![],
                resume_args: None,
                loop_args: None,
            })
            .unwrap();
        assert!(state.list_profiles().unwrap().iter().any(|p| p.name == "shell"));
    }
    let state = common::state(&dir);
    assert!(state.list_profiles().unwrap().iter().all(|p| p.name != "shell"));
    // A lone legacy shell row must not count as "has agents" and skip onboarding.
    assert!(state.agent_onboarding_needed().unwrap());
}

#[test]
fn enable_catalog_profiles_writes_recipes() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    state
        .enable_agent_profiles(&["claude".into(), "codex".into(), "cursor".into(), "gemini".into()])
        .unwrap();
    let profiles = state.list_profiles().unwrap();
    let get = |name: &str| profiles.iter().find(|p| p.name == name).cloned().unwrap();
    assert_eq!(get("claude").resume_args, Some(vec!["--continue".into()]));
    assert_eq!(get("codex").resume_args, Some(vec!["resume".into(), "--last".into()]));
    assert_eq!(get("cursor").command, "cursor-agent");
    // AGE-190: cursor's `--continue` turned out to be per cwd, so it resumes.
    assert_eq!(get("cursor").resume_args, Some(vec!["--continue".into()]));
    assert_eq!(get("gemini").command, "gemini");
    assert!(get("gemini").args.is_empty());
}

/// AGE-79: Copilot's headless mode was verified, so the catalog now carries a
/// loop recipe for it. Installs that already have a Copilot profile must be
/// retrofitted on open, since theirs was written when the recipe was None.
#[test]
fn copilot_gains_its_loop_recipe_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let state = common::state(&dir);
        state
            .register_profile(agency_core::profile::AgentProfile {
                name: "copilot".into(),
                command: "copilot".into(),
                args: vec![],
                env: vec![],
                resume_args: Some(vec!["--continue".into()]),
                loop_args: None,
            })
            .unwrap();
    }
    let state = common::state(&dir);
    let copilot = state.list_profiles().unwrap().into_iter().find(|p| p.name == "copilot").unwrap();
    assert_eq!(
        copilot.loop_args,
        Some(vec!["-p".into(), "{{prompt}}".into(), "--allow-all-tools".into()])
    );
}

#[test]
fn deleted_builtin_stays_gone_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let state = common::state(&dir);
        state.enable_agent_profiles(&["claude".into(), "pi".into()]).unwrap();
        state.delete_profile("claude").unwrap();
        assert!(state.list_profiles().unwrap().iter().all(|p| p.name != "claude"));
    }
    // Re-open on the same DB — claude must not be re-seeded.
    let state = common::state(&dir);
    assert!(state.list_profiles().unwrap().iter().all(|p| p.name != "claude"));
    assert!(state.list_profiles().unwrap().iter().any(|p| p.name == "pi"));
}

#[test]
fn complete_onboarding_enables_and_clears_flag() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    assert!(state.agent_onboarding_needed().unwrap());
    state.complete_agent_onboarding(&["hermes".into(), "kimi".into(), "crush".into()]).unwrap();
    assert!(!state.agent_onboarding_needed().unwrap());
    let names: Vec<_> = state.list_profiles().unwrap().into_iter().map(|p| p.name).collect();
    assert!(names.contains(&"hermes".into()));
    assert!(names.contains(&"kimi".into()));
    assert!(names.contains(&"crush".into()));
}

#[test]
fn existing_profiles_migrate_onboarding_done() {
    let dir = tempfile::tempdir().unwrap();
    {
        let state = common::state(&dir);
        state.enable_agent_profiles(&["claude".into()]).unwrap();
        // Simulate a pre-onboarding DB: clear the flag if somehow set, then
        // re-open. AppState::new should see the existing agent and mark done.
    }
    // Manually clear the flag to mimic an upgraded install that already had profiles.
    {
        use agency_core::registry::Registry;
        let reg = Registry::open(&dir.path().join("agency.db")).unwrap();
        reg.set_setting("agent_onboarding_completed", "").unwrap();
        assert!(reg.get_profile("claude").unwrap().is_some());
    }
    let state = common::state(&dir);
    assert!(!state.agent_onboarding_needed().unwrap());
}

#[test]
fn catalog_lists_all_builtins_with_enabled_flag() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    state.enable_agent_profiles(&["claude".into()]).unwrap();
    let catalog = state.list_agent_catalog().unwrap();
    assert_eq!(catalog.len(), 11);
    let claude = catalog.iter().find(|e| e.id == "claude").unwrap();
    assert!(claude.enabled);
    let gemini = catalog.iter().find(|e| e.id == "gemini").unwrap();
    assert!(!gemini.enabled);
    assert_eq!(gemini.command, "gemini");
    // The one web-served agent: opening prompt is session.prompt after boot.
    let dsh = catalog.iter().find(|e| e.id == "dsh").unwrap();
    assert!(dsh.serves_web_ui);
    assert!(dsh.accepts_prompt);
    assert!(!claude.serves_web_ui);
}

#[test]
fn enabled_profiles_have_bare_commands() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    state.enable_agent_profiles(&["claude".into(), "pi".into(), "hermes".into()]).unwrap();
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
