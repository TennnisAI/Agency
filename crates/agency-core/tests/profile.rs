use agency_core::profile::AgentProfile;

#[test]
fn render_args_substitutes_prompt_token() {
    let profile = AgentProfile {
        name: "claude".into(),
        command: "claude".into(),
        args: vec!["-p".into(), "{{prompt}}".into()],
        env: vec![],
        resume_args: None,
    };

    let rendered = profile.render_args("fix the bug");
    assert_eq!(rendered, vec!["-p".to_string(), "fix the bug".to_string()]);
}

#[test]
fn render_args_leaves_other_args_untouched() {
    let profile = AgentProfile {
        name: "x".into(),
        command: "x".into(),
        args: vec!["--flag".into(), "value".into()],
        env: vec![],
        resume_args: None,
    };

    assert_eq!(
        profile.render_args("anything"),
        vec!["--flag".to_string(), "value".to_string()]
    );
}
