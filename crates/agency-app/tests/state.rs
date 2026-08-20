use agency_app_lib::AppState;
use std::path::Path;

mod common;

#[test]
fn new_opens_clean_and_version_holds() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    assert_eq!(AppState::version(), "0.1.0");
    // Nothing is auto-seeded: agents come from onboarding, terminals aren't
    // profiles. Fresh-DB profile behavior is covered in tests/profiles.rs.
    assert!(state.profile_names().unwrap().is_empty());
}

#[test]
fn project_crud_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);

    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let p = state.add_project("demo", &repo).unwrap();
    assert_eq!(p.name, "demo");

    let all = state.list_projects().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, p.id);

    state.delete_project(&p.id).unwrap();
    assert_eq!(state.list_projects().unwrap().len(), 0);
}

/// A scratch folder is a project like any other: git is what unlocks worktrees,
/// not what makes a folder addable.
#[test]
fn add_project_accepts_a_plain_folder() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);

    let plain = dir.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();

    let p = state.add_project("plain", &plain).unwrap();
    assert_eq!(state.list_projects().unwrap().len(), 1);
    assert_eq!(state.list_projects().unwrap()[0].id, p.id);
    // It really is repo-less; the UI reads this to hide the git-shaped surfaces.
    assert_eq!(state.inspect_repo(&plain), agency_core::setup::RepoReadiness::NotARepo);
}

#[test]
fn add_project_rejects_a_path_that_is_not_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);

    let missing = dir.path().join("nope");
    let err = state.add_project("nope", &missing).unwrap_err().to_string();
    assert!(err.contains("does not exist"), "unexpected error: {err}");

    let file = dir.path().join("notes.md");
    std::fs::write(&file, "hi").unwrap();
    let err = state.add_project("notes", &file).unwrap_err().to_string();
    assert!(err.contains("not a folder"), "unexpected error: {err}");

    assert_eq!(state.list_projects().unwrap().len(), 0);
}

use agency_core::profile::AgentProfile;
use agency_core::term::SessionStatus;
use std::process::Command;

fn init_repo(dir: &Path) {
    let run = |args: &[&str]| {
        assert!(
            Command::new("git").args(args).current_dir(dir).status().unwrap().success(),
            "git {:?}",
            args
        );
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "t@e.com"]);
    run(&["config", "user.name", "T"]);
    std::fs::write(dir.join("README.md"), "hi").unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "init"]);
}

fn session_gone_or_cleanup(state: &AppState, id: &str) {
    // ensure no lingering tmux session after a test
    let _ = state.discard_run(id);
}

#[test]
fn create_run_persists_starts_session_and_lists() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo); // repo on `main` with a commit

    let state = common::state(&dir);
    // a profile that stays alive so the session is Running
    state
        .register_profile(AgentProfile {
            name: "stay".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "echo HI; sleep 3".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    let info = state.create_run(&project.id, "do it", "stay", None, "HEAD", None).unwrap();
    assert_eq!(info.agent, "stay");
    assert_eq!(info.branch, format!("agent/{}", info.id));

    // worktree exists
    let wt = state.worktree_path(&info.id).unwrap();
    assert!(wt.exists());

    // listed for the project, status Running
    let runs = state.list_runs(&project.id).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, info.id);
    // poll status until Running observed (session just started)
    let mut ok = false;
    for _ in 0..50 {
        if matches!(
            state.run_status(&info.id).unwrap(),
            SessionStatus::Running | SessionStatus::Exited { .. }
        ) {
            ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(ok);

    // discard cleans up worktree + record + session
    state.discard_run(&info.id).unwrap();
    assert!(!wt.exists());
    assert_eq!(state.list_runs(&project.id).unwrap().len(), 0);
    session_gone_or_cleanup(&state, &info.id);
}

#[test]
fn worktree_path_resolves_for_active_run() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

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
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "noop", None, "HEAD", None).unwrap();

    let wt = state.worktree_path(&info.id).unwrap();
    assert!(wt.ends_with(format!(".agency/worktrees/{}", info.id)));
    assert!(wt.exists());

    assert!(state.worktree_path("does-not-exist").is_err());

    // cleanup
    state.discard_run(&info.id).unwrap();
}

#[test]
fn worktree_path_resolves_to_repo_root_for_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();

    // Agent run: still resolves under .agency/worktrees/<id>.
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
    let agent = state.create_run(&project.id, "p", "noop", None, "HEAD", None).unwrap();
    let agent_wt = state.worktree_path(&agent.id).unwrap();
    assert!(agent_wt.ends_with(format!(".agency/worktrees/{}", agent.id)));

    // Terminal run: resolves to the project repo root, NOT a worktrees subdir.
    let term = state.create_terminal(&project.id).unwrap();
    let term_dir = state.worktree_path(&term.id).unwrap();
    assert_eq!(term_dir, state.project_repo_path(&project.id).unwrap());
    assert!(!term_dir.to_string_lossy().contains("worktrees"));

    // Cleanup sessions/records.
    state.discard_run(&agent.id).unwrap();
    state.discard_run(&term.id).unwrap();
}

#[test]
fn settings_default_and_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    let s = state.get_settings().unwrap();
    assert_eq!(s.lm_studio_base_url, "http://localhost:1234/v1");
    // Unset by default (empty string surfaces as None, i.e. "auto").
    assert_eq!(s.default_agent, None);
    // Unset means worktrees on, so an upgrade doesn't silently change where
    // the next agent runs.
    assert!(s.default_worktree);

    state
        .save_settings(&agency_app_lib::ProviderSettings {
            lm_studio_base_url: "http://localhost:9999/v1".into(),
            default_agent: Some("codex".into()),
            default_worktree: true,
        })
        .unwrap();
    let s2 = state.get_settings().unwrap();
    assert_eq!(s2.lm_studio_base_url, "http://localhost:9999/v1");
    assert_eq!(s2.default_agent.as_deref(), Some("codex"));

    // Clearing it round-trips back to None, not Some("").
    state
        .save_settings(&agency_app_lib::ProviderSettings {
            lm_studio_base_url: "http://localhost:9999/v1".into(),
            default_agent: None,
            default_worktree: false,
        })
        .unwrap();
    assert_eq!(state.get_settings().unwrap().default_agent, None);
    // Turning worktrees off persists, and turning them back on clears it.
    assert!(!state.get_settings().unwrap().default_worktree);
    state
        .save_settings(&agency_app_lib::ProviderSettings {
            lm_studio_base_url: "http://localhost:9999/v1".into(),
            default_agent: None,
            default_worktree: true,
        })
        .unwrap();
    assert!(state.get_settings().unwrap().default_worktree);
}

#[test]
fn create_run_injects_provider_env() {
    // Use a tmux session that echoes env vars so we can verify injection via capture.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    state
        .save_settings(&agency_app_lib::ProviderSettings {
            lm_studio_base_url: "http://localhost:1234/v1".into(),
            default_agent: None,
            default_worktree: true,
        })
        .unwrap();
    // Profile echoes env vars and then sleeps so we can capture output.
    state
        .register_profile(AgentProfile {
            name: "envcheck".into(),
            command: "sh".into(),
            args: vec![
                "-c".into(),
                "echo BASE=$OPENAI_BASE_URL; echo KEYSET=${OPENAI_API_KEY:+yes}; sleep 2".into(),
            ],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    let info = state.create_run(&project.id, "p", "envcheck", None, "HEAD", None).unwrap();

    // Poll tmux capture until we see the output (up to 5s)
    let mut out = String::new();
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        if let Ok(s) = state.run_preview(&info.id, 20) {
            out = s;
            if out.contains("BASE=http://localhost:1234/v1") {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    assert!(out.contains("BASE=http://localhost:1234/v1"), "got: {out}");
    assert!(out.contains("KEYSET=yes"), "got: {out}");

    state.discard_run(&info.id).unwrap();
}

/// The model a run is started on has to reach the agent's real command line —
/// a picker that stored a preference and launched the default anyway would be
/// worse than no picker, because the run would look like it was on the model
/// the user paid for.
#[test]
fn create_run_passes_the_chosen_model_to_the_agent_and_remembers_it() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    // Named after a catalog entry, so the real `--model` recipe applies; the
    // command underneath just prints the argv it was handed.
    state
        .register_profile(AgentProfile {
            name: "claude".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "echo ARGV=$*; sleep 2".into(), "sh".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    let info = state.create_run(&project.id, "p", "claude", Some("opus"), "HEAD", None).unwrap();
    assert_eq!(info.model.as_deref(), Some("opus"));

    let mut out = String::new();
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        if let Ok(s) = state.run_preview(&info.id, 20) {
            out = s;
            if out.contains("ARGV=") {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // Ahead of the prompt, which is this CLI's trailing positional.
    assert!(out.contains("ARGV=--model opus p"), "got: {out}");

    // And the choice is remembered, so the picker reopens on it.
    let models = state.list_agent_models().unwrap();
    let claude = models.iter().find(|m| m.agent == "claude").expect("claude in the model list");
    assert!(claude.supported);
    assert_eq!(claude.selected.as_deref(), Some("opus"));
    assert_eq!(claude.recent, vec!["opus".to_string()]);
    assert!(claude.suggested.contains(&"sonnet".to_string()));

    state.discard_run(&info.id).unwrap();
}

/// An agent whose CLI has no model flag must refuse the model outright. Taking
/// it and launching the default would bill the run to a model nobody chose.
#[test]
fn create_run_refuses_a_model_the_agent_cannot_be_told() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    for name in ["crush", "made-up"] {
        state
            .register_profile(AgentProfile {
                name: name.into(),
                command: "sh".into(),
                args: vec!["-c".into(), "sleep 1".into()],
                env: vec![],
                resume_args: None,
                loop_args: None,
            })
            .unwrap();
    }
    let project = state.add_project("demo", &repo).unwrap();

    for agent in ["crush", "made-up"] {
        let err = state
            .create_run(&project.id, "p", agent, Some("opus"), "HEAD", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no way to be told a model"), "{agent}: {err}");
    }
    // Nothing was created, so no worktree was left behind either.
    assert!(state.list_runs(&project.id).unwrap().is_empty());
    // And the picker knows not to offer one.
    let models = state.list_agent_models().unwrap();
    assert!(models.iter().all(|m| !m.supported));
    assert!(models.iter().all(|m| m.suggested.is_empty()));
}

use agency_core::merge::MergeOutcome;

/// "Fix with agent" only makes sense while git is mid-merge: with no
/// `MERGE_HEAD` there is nothing to describe, and sending a prompt anyway would
/// hand the agent an empty conflict.
#[test]
fn send_merge_conflict_requires_a_merge_in_progress() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

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
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "noop", None, "HEAD", None).unwrap();

    let err = state.send_merge_conflict(&info.id).unwrap_err().to_string();
    assert!(err.contains("no merge is in progress"), "got: {err}");

    state.discard_run(&info.id).unwrap();
}

#[test]
fn save_settings_rejects_bad_provider_url() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    // remote http (non-localhost) is rejected
    let bad = agency_app_lib::ProviderSettings {
        lm_studio_base_url: "http://evil.example.com/v1".into(),
        default_agent: None,
        default_worktree: true,
    };
    assert!(state.save_settings(&bad).is_err());
    // embedded credentials are rejected
    let creds = agency_app_lib::ProviderSettings {
        lm_studio_base_url: "http://user:pass@localhost:1234/v1".into(),
        default_agent: None,
        default_worktree: true,
    };
    assert!(state.save_settings(&creds).is_err());
    // localhost, IPv6 loopback http, and https are allowed
    for ok in ["http://localhost:1234/v1", "http://[::1]:1234/v1", "https://api.example.com/v1", ""]
    {
        let s = agency_app_lib::ProviderSettings {
            lm_studio_base_url: ok.into(),
            default_agent: None,
            default_worktree: true,
        };
        assert!(state.save_settings(&s).is_ok(), "should accept {ok}");
    }
}

#[test]
fn attach_streams_and_input_reaches_agent() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "echoer".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "echo READY; read x; echo GOT:$x; sleep 3".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "echoer", None, "HEAD", None).unwrap();

    let buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let b = buf.clone();
    state
        .attach_run(&info.id, 220, 50, move |bytes| {
            b.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes));
        })
        .unwrap();

    let wait = |needle: &str| {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            if buf.lock().unwrap().contains(needle) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        false
    };
    assert!(wait("READY"), "buf: {:?}", buf.lock().unwrap());
    state.run_input(&info.id, b"ping\n").unwrap();
    assert!(wait("GOT:ping"), "buf: {:?}", buf.lock().unwrap());

    // preview also reflects the pane
    let prev = state.run_preview(&info.id, 50).unwrap();
    assert!(prev.contains("READY"));

    state.detach_run(&info.id);
    state.discard_run(&info.id).unwrap();
}

#[test]
fn terminal_survives_attach_detach_reattach() {
    // Navigating away from a focused terminal and back drives attach → detach →
    // attach on the same tmux session. The detach drops the AgentHandle (killing
    // only the `tmux attach` *client*); the session's shell must stay alive across
    // the whole cycle so the pane never shows "Pane is dead". Regression guard for
    // the terminal-death report.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_terminal(&project.id).unwrap();
    assert_eq!(info.kind, "terminal");

    let wait_running = |state: &AppState, id: &str| {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            if matches!(state.run_status(id).unwrap(), SessionStatus::Running) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        false
    };
    assert!(wait_running(&state, &info.id), "shell should be running after create");

    // First attach (view the terminal).
    state.attach_run(&info.id, 220, 50, |_bytes| {}).unwrap();
    assert!(wait_running(&state, &info.id), "running after first attach");

    // Navigate away: detach (drops the handle, SIGKILLs the attach client).
    state.detach_run(&info.id);
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert!(
        matches!(state.run_status(&info.id).unwrap(), SessionStatus::Running),
        "shell must survive detach"
    );

    // Navigate back: re-attach to the same session.
    state.attach_run(&info.id, 220, 50, |_bytes| {}).unwrap();
    assert!(wait_running(&state, &info.id), "running after re-attach");

    state.detach_run(&info.id);
    state.discard_run(&info.id).unwrap();
    assert_eq!(state.list_runs(&project.id).unwrap().len(), 0);
}

#[test]
fn merge_task_clean_merges_branch_into_base() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

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
    let project = state.add_project("demo", &repo).unwrap();

    // create_run → worktree on agent/<id>; make a non-conflicting commit in it.
    let info = state.create_run(&project.id, "p", "noop", None, "HEAD", None).unwrap();
    let wt = state.worktree_path(&info.id).unwrap();
    std::fs::write(wt.join("feature.txt"), "x\n").unwrap();
    std::process::Command::new("git").args(["add", "-A"]).current_dir(&wt).status().unwrap();
    std::process::Command::new("git")
        .args(["commit", "-qm", "feat"])
        .current_dir(&wt)
        .status()
        .unwrap();

    let outcome = state.merge_task(&info.id).unwrap();
    assert!(matches!(outcome, MergeOutcome::Clean { .. }));
    // feature.txt now on the repo's base branch working tree.
    assert!(repo.join("feature.txt").exists());

    // discard after merge (worktree may already be removed by merge; best-effort)
    let _ = state.discard_run(&info.id);
}

#[test]
fn ensure_run_active_respawns_a_stopped_agent_run() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo); // repo on `main` with a commit
    let state = common::state(&dir);
    // A fake agent whose command just sleeps, so a respawn is observable as Running.
    state
        .register_profile(AgentProfile {
            name: "sleeper".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 5".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "sleeper", None, "HEAD", None).unwrap();

    // Stop the run's session (record stays) -> status becomes Gone.
    state.stop_run(&info.id).unwrap();
    let mut gone = false;
    for _ in 0..75 {
        if matches!(state.run_status(&info.id).unwrap(), SessionStatus::Gone) {
            gone = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(gone, "session did not become Gone after stop_run");

    // Reactivate -> a new session comes up (fresh fallback, since resume_args is None).
    state.ensure_run_active(&info.id).unwrap();
    let mut back = false;
    for _ in 0..75 {
        if !matches!(state.run_status(&info.id).unwrap(), SessionStatus::Gone) {
            back = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(back, "ensure_run_active did not respawn the session");

    state.discard_run(&info.id).unwrap();
}

#[test]
fn ensure_run_active_is_noop_when_session_present() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_terminal(&project.id).unwrap();
    // Session is live; ensure_run_active must not error or take it down.
    state.ensure_run_active(&info.id).unwrap();
    assert!(!matches!(state.run_status(&info.id).unwrap(), SessionStatus::Gone));
    state.discard_run(&info.id).unwrap();
}

#[test]
fn ensure_run_active_falls_back_to_fresh_when_resume_fails() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    // Fake agent: resume fails fast; fresh (render_args of `args`) prints FRESH and stays.
    state
        .register_profile(AgentProfile {
            name: "flaky".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "printf FRESH; sleep 5".into()],
            env: vec![],
            resume_args: Some(vec!["-c".into(), "printf NO-CONV; exit 1".into()]),
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "flaky", None, "HEAD", None).unwrap();

    state.stop_run(&info.id).unwrap();
    let mut gone = false;
    for _ in 0..75 {
        if matches!(state.run_status(&info.id).unwrap(), SessionStatus::Gone) {
            gone = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(gone, "session did not become Gone after stop_run");

    // Reactivate: resume (NO-CONV; exit 1) fails fast -> fallback fresh (FRESH; sleep 5).
    state.ensure_run_active(&info.id).unwrap();
    let mut fresh = false;
    for _ in 0..150 {
        let cap = state.run_preview(&info.id, 10).unwrap_or_default();
        if cap.contains("FRESH")
            && matches!(state.run_status(&info.id).unwrap(), SessionStatus::Running)
        {
            fresh = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(fresh, "fallback fresh session did not come up");
    state.discard_run(&info.id).unwrap();
}

#[test]
fn extra_session_lifecycle_shares_worktree_and_cascades() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    // Prints its cwd so we can prove the extra tab runs in the run's worktree.
    state
        .register_profile(AgentProfile {
            name: "pwds".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "pwd; sleep 5".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let run = state.create_run(&project.id, "p", "pwds", None, "HEAD", None).unwrap();
    let wt = state.worktree_path(&run.id).unwrap();

    // First extra tab: defaults to the run's agent, gets the --2 suffix.
    let s2 = state.start_run_session(&run.id, None, "").unwrap();
    assert_eq!(s2.id, format!("{}--2", run.id));
    assert_eq!(s2.agent, "pwds");

    // It runs in the SAME worktree as the primary agent.
    let mut cwd_ok = false;
    for _ in 0..150 {
        let cap = state.run_preview(&s2.id, 10).unwrap_or_default();
        if cap.contains(&wt.to_string_lossy().to_string()) {
            cwd_ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(cwd_ok, "extra session did not report the run's worktree as cwd");

    // Second tab (explicit agent) takes the next number; both are listed.
    let s3 = state.start_run_session(&run.id, Some("pwds"), "").unwrap();
    assert_eq!(s3.id, format!("{}--3", run.id));
    assert_eq!(state.run_sessions(&run.id).unwrap().len(), 2);

    // The primary id is not closable through the tab path.
    assert!(state.close_run_session(&run.id).is_err());

    // Closing a tab kills only that session; siblings survive.
    state.close_run_session(&s2.id).unwrap();
    let mut gone = false;
    for _ in 0..75 {
        if matches!(state.run_status(&s2.id).unwrap(), SessionStatus::Gone) {
            gone = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(gone, "closed tab session still present");
    assert_eq!(state.run_sessions(&run.id).unwrap().len(), 1);
    assert!(!matches!(state.run_status(&run.id).unwrap(), SessionStatus::Gone));

    // Discarding the run sweeps the remaining tab: session and row.
    state.discard_run(&run.id).unwrap();
    let mut swept = false;
    for _ in 0..75 {
        if matches!(state.run_status(&s3.id).unwrap(), SessionStatus::Gone) {
            swept = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(swept, "discard did not kill the extra session");
    assert!(state.run_sessions(&run.id).unwrap().is_empty());
}

#[test]
fn shell_tab_needs_no_profile_and_runs_in_the_worktree() {
    // A terminal tab is not an agent: no "shell" row exists in the profiles
    // table, so this must launch the login shell directly rather than fail with
    // "unknown agent profile".
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "pwds".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "pwd; sleep 5".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let run = state.create_run(&project.id, "p", "pwds", None, "HEAD", None).unwrap();
    let wt = state.worktree_path(&run.id).unwrap();
    assert!(state.profile_names().unwrap().iter().all(|n| n != "shell"));

    let tab = state.start_run_session(&run.id, Some("shell"), "").unwrap();
    assert_eq!(tab.agent, "shell");

    // Prove it's a live shell rooted in the run's worktree.
    state.run_input(&tab.id, b"pwd\n").unwrap();
    let mut cwd_ok = false;
    for _ in 0..150 {
        let cap = state.run_preview(&tab.id, 20).unwrap_or_default();
        if cap.contains(&wt.to_string_lossy().to_string()) {
            cwd_ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(cwd_ok, "shell tab did not report the run's worktree as cwd");

    state.discard_run(&run.id).unwrap();
}

#[test]
fn ensure_run_active_revives_a_dead_extra_session() {
    // App-restart path: the tab's daemon session is gone, but its registry row
    // survives; mounting the tab must relaunch its agent in the run's worktree.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "pwds".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "pwd; sleep 5".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let run = state.create_run(&project.id, "p", "pwds", None, "HEAD", None).unwrap();
    let wt = state.worktree_path(&run.id).unwrap();
    let tab = state.start_run_session(&run.id, None, "").unwrap();

    state.stop_run(&tab.id).unwrap();
    let mut gone = false;
    for _ in 0..75 {
        if matches!(state.run_status(&tab.id).unwrap(), SessionStatus::Gone) {
            gone = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(gone, "tab session did not become Gone after stop");

    state.ensure_run_active(&tab.id).unwrap();
    let mut back = false;
    for _ in 0..150 {
        let cap = state.run_preview(&tab.id, 10).unwrap_or_default();
        if cap.contains(&wt.to_string_lossy().to_string())
            && matches!(state.run_status(&tab.id).unwrap(), SessionStatus::Running)
        {
            back = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(back, "revived tab did not come up in the run's worktree");

    state.discard_run(&run.id).unwrap();
}

#[test]
fn extra_session_rejects_terminal_runs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();
    let term = state.create_terminal(&project.id).unwrap();
    let err = state.start_run_session(&term.id, None, "").unwrap_err().to_string();
    assert!(err.contains("only agent runs"), "unexpected error: {err}");
    state.discard_run(&term.id).unwrap();
}

#[test]
fn send_review_comments_errors_when_session_not_running() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);

    // Use a fake run_id — no session has ever been started for it.
    let fake_run_id = "00000000-0000-0000-0000-000000000000";

    // Insert an unsent review comment for the fake run.
    state.add_review_comment(fake_run_id, "src/main.rs", 1, 3, "looks good").unwrap();

    // send_review_comments must fail because the session is not running.
    let err = state.send_review_comments(fake_run_id).unwrap_err().to_string();
    assert!(err.contains("not running"), "expected 'not running' error, got: {err}");

    // The comment must NOT have been marked sent.
    let unsent = state.list_review_comments(fake_run_id).unwrap();
    assert_eq!(unsent.len(), 1, "comment count should be unchanged");
    assert!(!unsent[0].sent, "comment must NOT be marked sent after failed send");
}

// ── the send queue (AGE-111) ────────────────────────────────────────────────

/// Epoch ms, the clock `AppState`'s activity bookkeeping runs on.
fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// A live shell session to type into. Terminals need no agent profile, and the
/// send queue does not care what is on the other end of the pty.
fn live_terminal(state: &AppState, project_id: &str) -> String {
    let term = state.create_terminal(project_id).unwrap();
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        if matches!(state.run_status(&term.id).unwrap(), SessionStatus::Running) {
            return term.id;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    panic!("shell never came up");
}

/// Poll the pane for `needle`, which arrives asynchronously: the write goes to
/// the daemon, the shell echoes it, the emulator renders it.
fn pane_gets(state: &AppState, id: &str, needle: &str) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        if state.run_preview(id, 50).unwrap().contains(needle) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

/// Teach the activity bookkeeping that this run's pane has been quiet since
/// `quiet_since_ms` — the same two observations the notifier tick would make.
fn mark_quiet_since(state: &AppState, id: &str, quiet_since_ms: i64) {
    state.update_activity(id, true, quiet_since_ms);
    state.update_activity(id, false, now_ms());
}

#[test]
fn a_message_waits_for_a_busy_agent_and_lands_once_it_is_quiet() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();
    let id = live_terminal(&state, &project.id);

    state.add_review_comment(&id, "src/a.rs", 7, 7, "rename-this-marker").unwrap();
    // Nothing has observed this session yet, so it counts as working: the text
    // must be queued, not typed. Reported to the caller as "not delivered".
    assert!(!state.send_review_comments(&id).unwrap(), "an unobserved session must hold");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(
        !state.run_preview(&id, 50).unwrap().contains("rename-this-marker"),
        "the message must not have been typed while the session looked busy"
    );
    // The comments are marked sent all the same: from the queue's acceptance on,
    // delivery is Agency's job and re-sending would only duplicate them.
    assert!(state.list_review_comments(&id).unwrap().iter().all(|c| c.sent));

    // The pane goes quiet past the working TTL, and the tick drains it.
    mark_quiet_since(&state, &id, now_ms() - 60_000);
    state.drain_send_queues(now_ms());
    assert!(pane_gets(&state, &id, "rename-this-marker"), "a quiet session must take it");
    state.discard_run(&id).unwrap();
}

/// The marker's whole point: while something is held, the run says so — and it
/// is still said after a quit, because the queue outlives the app that made it.
#[test]
fn a_held_message_shows_on_the_run_survives_a_quit_and_can_be_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();
    let id = live_terminal(&state, &project.id);

    let queued_on_tile = |s: &AppState| {
        s.list_runs(&project.id).unwrap().into_iter().find(|r| r.id == id).unwrap().queued_messages
    };
    assert_eq!(queued_on_tile(&state), 0, "nothing owed yet");

    state.add_review_comment(&id, "src/a.rs", 7, 7, "held-marker").unwrap();
    // Unobserved sessions count as working, so this one is held rather than typed.
    assert!(!state.send_review_comments(&id).unwrap());
    assert_eq!(queued_on_tile(&state), 1, "the run must say it is still owed a message");
    let waiting = state.list_queued_messages(&id);
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].session_id, id);
    assert_eq!(waiting[0].origin, "review comments");
    assert!(waiting[0].text.contains("held-marker"), "the popover shows what will be typed");

    // A quit and a relaunch: the same database, a new state. The daemon (and so
    // the session) outlives the app, which is why the message is still worth
    // keeping.
    let reopened = AppState::new(&dir.path().join("agency.db"), dir.path()).unwrap();
    let restored = reopened.list_queued_messages(&id);
    assert_eq!(restored.len(), 1, "a quit must not lose a held message");
    assert_eq!(restored[0].text, waiting[0].text);
    assert_eq!(queued_on_tile(&reopened), 1);

    // Dropping it is possible from either state, and leaves nothing for the
    // next launch to restore.
    assert!(reopened.cancel_queued_message(&id, &restored[0].text));
    assert!(!reopened.cancel_queued_message(&id, &restored[0].text), "already gone");
    assert!(reopened.list_queued_messages(&id).is_empty());
    let relaunched = AppState::new(&dir.path().join("agency.db"), dir.path()).unwrap();
    assert!(relaunched.list_queued_messages(&id).is_empty(), "a dropped message must stay dropped");

    state.discard_run(&id).unwrap();
}

#[test]
fn a_half_typed_prompt_survives_a_send() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();
    let id = live_terminal(&state, &project.id);

    // A quiet session — nothing but the human's own draft is in the way.
    mark_quiet_since(&state, &id, now_ms() - 60_000);
    state.run_input(&id, b"draft-marker-not-sent-yet").unwrap();
    assert!(pane_gets(&state, &id, "draft-marker-not-sent-yet"), "the draft should echo");

    state.add_review_comment(&id, "src/a.rs", 7, 7, "queued-marker").unwrap();
    assert!(!state.send_review_comments(&id).unwrap(), "a draft on the line must hold");
    // Past the echo grace the draft itself is still the block, and it holds for
    // as long as the human leaves it there.
    std::thread::sleep(std::time::Duration::from_millis(1_200));
    mark_quiet_since(&state, &id, now_ms() - 60_000);
    state.drain_send_queues(now_ms());
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(
        !state.run_preview(&id, 50).unwrap().contains("queued-marker"),
        "the queue typed over a half-written prompt"
    );

    // The human sends their line; the queue follows behind it.
    state.run_input(&id, b"\r").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1_200));
    mark_quiet_since(&state, &id, now_ms() - 60_000);
    state.drain_send_queues(now_ms());
    assert!(pane_gets(&state, &id, "queued-marker"), "must land once the line is free");
    state.discard_run(&id).unwrap();
}

// ── runs without a worktree (started directly in the project checkout) ───────

/// The headline of the feature: with `worktree = false` the agent runs in the
/// project's own checkout, on the branch already there, and nothing new is cut.
#[test]
fn create_run_without_worktree_uses_the_project_checkout() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo); // repo on `main` with a commit

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "pwds".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "pwd; sleep 5".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    let info = state
        .create_run_with_progress(&project.id, "p", "pwds", None, "HEAD", None, false, |_| {})
        .unwrap();
    assert!(!info.worktree);
    assert_eq!(info.branch, "main", "adopts the checkout's branch");
    // Canonicalized on both sides: on macOS the temp dir is reached through a
    // /var -> /private/var symlink, and the registry stores the path as given.
    assert_eq!(
        state.worktree_path(&info.id).unwrap().canonicalize().unwrap(),
        repo.canonicalize().unwrap(),
    );
    assert!(
        !repo.join(".agency").join("worktrees").join(&info.id).exists(),
        "no worktree directory was created"
    );

    // The agent's cwd really is the checkout.
    let mut cwd_ok = false;
    for _ in 0..150 {
        let cap = state.run_preview(&info.id, 10).unwrap_or_default();
        if cap.contains(&repo.canonicalize().unwrap().to_string_lossy().to_string()) {
            cwd_ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(cwd_ok, "agent did not report the project checkout as cwd");

    // No branch of its own means nothing to merge or open a PR for.
    let err = state.merge_preview(&info.id).unwrap_err().to_string();
    assert!(err.contains("project checkout"), "got: {err}");
    assert!(state.create_pr(&info.id).unwrap_err().to_string().contains("project checkout"));

    session_gone_or_cleanup(&state, &info.id);
}

/// A project with no repository at all: the agent works in the folder, records
/// no branch, and every branch-level operation stays refused.
#[test]
fn create_run_in_a_gitless_project_works_in_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("scratch");
    std::fs::create_dir_all(&plain).unwrap();

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "pwds".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "pwd; sleep 5".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("scratch", &plain).unwrap();

    let info = state
        .create_run_with_progress(&project.id, "p", "pwds", None, "HEAD", None, false, |_| {})
        .unwrap();
    assert!(!info.worktree);
    assert_eq!(info.branch, "", "no repository means no branch to adopt");
    assert_eq!(
        state.worktree_path(&info.id).unwrap().canonicalize().unwrap(),
        plain.canonicalize().unwrap(),
    );

    // The agent's cwd really is the folder.
    let mut cwd_ok = false;
    for _ in 0..150 {
        let cap = state.run_preview(&info.id, 10).unwrap_or_default();
        if cap.contains(&plain.canonicalize().unwrap().to_string_lossy().to_string()) {
            cwd_ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(cwd_ok, "agent did not report the project folder as cwd");

    let err = state.merge_preview(&info.id).unwrap_err().to_string();
    assert!(err.contains("not a git repository"), "got: {err}");

    session_gone_or_cleanup(&state, &info.id);
}

/// The backstop behind the hidden UI: races, loops and issue dispatch all ask
/// for a worktree, and there is nothing to cut one from here.
#[test]
fn create_run_with_a_worktree_in_a_gitless_project_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("scratch");
    std::fs::create_dir_all(&plain).unwrap();

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "stay".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 5".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("scratch", &plain).unwrap();

    let err = state
        .create_run_with_progress(&project.id, "p", "stay", None, "HEAD", None, true, |_| {})
        .unwrap_err()
        .to_string();
    assert!(err.contains("not a git repository"), "got: {err}");
    assert!(err.contains("worktree"), "the error should say what was refused: {err}");
    assert!(state.list_runs(&project.id).unwrap().is_empty(), "nothing was recorded");
}

/// Discarding such a run must not touch git: `main` is the user's branch, and
/// the worktree-removal path would try to delete it.
#[test]
fn discard_without_worktree_leaves_the_checkout_and_its_branch_alone() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

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
    let project = state.add_project("demo", &repo).unwrap();
    let info = state
        .create_run_with_progress(&project.id, "p", "noop", None, "HEAD", None, false, |_| {})
        .unwrap();

    // The run follows the checkout: switching branches under it renames the run
    // rather than leaving the branch it was started on showing.
    assert!(Command::new("git")
        .args(["checkout", "-q", "-b", "side"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    assert_eq!(state.list_runs(&project.id).unwrap()[0].branch, "side");

    // Uncommitted work in the checkout, as a user mid-task would have.
    std::fs::write(repo.join("scratch.txt"), "work in progress").unwrap();

    state.discard_run(&info.id).unwrap();

    assert!(repo.join("README.md").exists(), "checkout survived");
    assert_eq!(
        std::fs::read_to_string(repo.join("scratch.txt")).unwrap(),
        "work in progress",
        "uncommitted work untouched"
    );
    let branches = state.list_project_branches(&project.id).unwrap();
    assert!(branches.branches.contains(&"main".to_string()), "main still exists");
    assert_eq!(state.list_runs(&project.id).unwrap().len(), 0);
}

/// Archiving one is only the teardown and the stamp: no auto-commit sweeping
/// the user's uncommitted work onto their branch, and restore is a no-op.
#[test]
fn archive_without_worktree_does_not_commit_the_users_work() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

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
    let project = state.add_project("demo", &repo).unwrap();
    let info = state
        .create_run_with_progress(&project.id, "p", "noop", None, "HEAD", None, false, |_| {})
        .unwrap();
    std::fs::write(repo.join("scratch.txt"), "work in progress").unwrap();

    let head_before =
        Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout;

    state.archive_run(&info.id).unwrap();
    assert_eq!(state.list_archived_runs(&project.id).unwrap().len(), 1);

    let head_after =
        Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout;
    assert_eq!(head_before, head_after, "archive must not commit anything");
    let status =
        Command::new("git").args(["status", "--porcelain"]).current_dir(&repo).output().unwrap();
    assert!(
        String::from_utf8_lossy(&status.stdout).contains("scratch.txt"),
        "the uncommitted file is still uncommitted"
    );

    // Restoring brings the row back live without re-creating anything on disk.
    let restored = state.restore_run(&info.id).unwrap();
    assert!(!restored.worktree);
    assert_eq!(state.list_runs(&project.id).unwrap().len(), 1);
    session_gone_or_cleanup(&state, &info.id);
}

/// Cleaning up the archived list takes every archived run's branch and worktree
/// with it, and leaves the live runs alone.
#[test]
fn discard_archived_runs_clears_the_archive_and_spares_live_runs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

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
    let project = state.add_project("demo", &repo).unwrap();

    let mut runs = Vec::new();
    for prompt in ["a", "b", "c"] {
        runs.push(
            state
                .create_run_with_progress(
                    &project.id,
                    prompt,
                    "noop",
                    None,
                    "HEAD",
                    None,
                    true,
                    |_| {},
                )
                .unwrap(),
        );
    }
    let live = runs.pop().unwrap();
    for r in &runs {
        state.archive_run(&r.id).unwrap();
    }
    assert_eq!(state.list_archived_runs(&project.id).unwrap().len(), 2);

    let summary = state.discard_archived_runs(&project.id).unwrap();
    assert_eq!(summary.discarded, 2);
    assert!(summary.failed.is_empty(), "clean sweep: {:?}", summary.failed);

    assert!(state.list_archived_runs(&project.id).unwrap().is_empty());
    let branches = state.list_project_branches(&project.id).unwrap().branches;
    for r in &runs {
        assert!(!branches.contains(&r.branch), "{} was deleted", r.branch);
        assert!(
            !repo.join(".agency").join("worktrees").join(&r.id).exists(),
            "worktree dir is gone"
        );
    }

    // The un-archived run keeps its row, its branch, and its worktree.
    let remaining = state.list_runs(&project.id).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, live.id);
    assert!(branches.contains(&live.branch));
    assert!(repo.join(".agency").join("worktrees").join(&live.id).exists());
    session_gone_or_cleanup(&state, &live.id);
}

/// AGE-49: removing an agent stops its session and hands git a worktree to
/// unlink, which on a real repo runs for seconds. Both teardowns now report the
/// step they are on so the confirm dialog can show it instead of freezing —
/// this pins the steps actually being emitted, in order, with the worktree
/// removal (the slow one) named among them.
#[test]
fn removing_an_agent_reports_each_teardown_step() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

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
    let project = state.add_project("demo", &repo).unwrap();

    let archived = state
        .create_run_with_progress(&project.id, "a", "noop", None, "HEAD", None, true, |_| {})
        .unwrap();
    let mut steps = Vec::new();
    state.archive_run_with_progress(&archived.id, &mut |p| steps.push(p.phase)).unwrap();
    assert_eq!(
        steps,
        vec![
            "Saving uncommitted changes",
            "Stopping the agent",
            "Removing the worktree",
            "Cleaning up",
        ],
    );

    let doomed = state
        .create_run_with_progress(&project.id, "b", "noop", None, "HEAD", None, true, |_| {})
        .unwrap();
    let mut steps = Vec::new();
    state.discard_run_with_progress(&doomed.id, &mut |p| steps.push((p.phase, p.detail))).unwrap();
    assert_eq!(
        steps.iter().map(|(phase, _)| phase.as_str()).collect::<Vec<_>>(),
        vec!["Stopping the agent", "Removing the worktree", "Cleaning up"],
    );
    // The detail line names the branch, so a race tearing down five attempts
    // says which one it is on.
    assert!(steps.iter().all(|(_, detail)| *detail == doomed.branch), "{steps:?}");

    // A sweep relays each run's steps, with its own position as the detail so
    // the bar doesn't look like one teardown restarting over and over.
    let swept = state
        .create_run_with_progress(&project.id, "c", "noop", None, "HEAD", None, true, |_| {})
        .unwrap();
    state.archive_run(&swept.id).unwrap();
    let mut details = Vec::new();
    let summary = state
        .discard_archived_runs_with_progress(&project.id, &mut |p| details.push(p.detail))
        .unwrap();
    // Both archived runs: the one archived above, and `swept`.
    assert_eq!(summary.discarded, 2);
    assert!(
        details.iter().all(|d| d.starts_with("1 of 2: ") || d.starts_with("2 of 2: ")),
        "{details:?}",
    );
    session_gone_or_cleanup(&state, &swept.id);
}

/// Wait for `f` to hold, polling for up to ~4s. Sessions start and exit
/// asynchronously in the daemon, so nothing about a run script is immediate.
fn eventually(mut f: impl FnMut() -> bool) -> bool {
    for _ in 0..200 {
        if f() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

#[test]
fn run_scripts_are_per_script_and_run_at_project_level() {
    use agency_core::config::RunScript;

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();
    // The project's own checkout, no agent involved — AGE-34's "what if I just
    // want to run it from the project level?".
    let target = format!("project:{}", project.id);

    // Nothing configured yet: the tab has an empty list to set up from.
    let config = state.run_script_config(&target).unwrap();
    assert!(config.scripts.is_empty());
    assert_eq!(config.workspace, repo.display().to_string());

    let script = |name: &str, command: &str| RunScript {
        name: name.into(),
        command: command.into(),
        web: false,
        nonconcurrent: false,
    };
    state
        .save_run_scripts(
            &target,
            vec![
                script("serve", "sleep 30"),
                // Writes a file and exits 0 — a build, the case that must not
                // get a browser preview.
                script("build", "echo built > built.txt"),
            ],
        )
        .unwrap();

    let config = state.run_script_config(&target).unwrap();
    assert_eq!(config.scripts.len(), 2);
    assert!(!config.shared, "saved locally, so not the team's copy");

    let status_of = |name: &str| {
        state
            .run_scripts_status(&target)
            .unwrap()
            .into_iter()
            .find(|s| s.name == name)
            .map(|s| s.status)
    };

    // Nothing started, so the board's dot is out (AGE-39).
    assert!(!state.run_scripts_live(&target).unwrap());

    // Both run at once: the build finishing must not disturb the server.
    state.start_run_script(&target, "serve").unwrap();
    state.start_run_script(&target, "build").unwrap();
    assert!(
        eventually(|| repo.join("built.txt").exists()),
        "the build ran in the project checkout"
    );
    assert!(eventually(|| matches!(
        status_of("build"),
        Some(agency_core::term::SessionStatus::Exited { .. })
    )));
    assert!(
        matches!(status_of("serve"), Some(agency_core::term::SessionStatus::Running)),
        "the server keeps running while the build finishes"
    );

    // Logs are per script, not shared.
    state.stop_run_script(&target, "build").unwrap();
    assert!(matches!(status_of("build"), Some(agency_core::term::SessionStatus::Gone)));
    assert!(matches!(status_of("serve"), Some(agency_core::term::SessionStatus::Running)));

    // An unknown name is refused rather than silently starting nothing.
    assert!(state.start_run_script(&target, "nope").is_err());

    // The server is still up, so the dot stays lit even though the build has
    // finished and been stopped; it goes out only once nothing is running.
    assert!(state.run_scripts_live(&target).unwrap());
    state.stop_run_script(&target, "serve").unwrap();
    assert!(!state.run_scripts_live(&target).unwrap());
}

// AGE-39: the board polls `list_runs`, so the "a script is live here" dot has
// to come back on the run itself — one workspace's script must not light up
// another agent's tile.
#[test]
fn list_runs_reports_which_workspace_has_a_script_live() {
    use agency_core::config::RunScript;

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "stay".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "sleep 30".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let a = state.create_run(&project.id, "a", "stay", None, "HEAD", None).unwrap();
    let b = state.create_run(&project.id, "b", "stay", None, "HEAD", None).unwrap();
    state
        .save_run_scripts(
            &format!("project:{}", project.id),
            vec![RunScript {
                name: "serve".into(),
                command: "sleep 30".into(),
                web: false,
                nonconcurrent: false,
            }],
        )
        .unwrap();

    let live_of = |id: &str| {
        state
            .list_runs(&project.id)
            .unwrap()
            .into_iter()
            .find(|r| r.id == id)
            .unwrap()
            .run_scripts_live
    };
    assert!(!live_of(&a.id));
    assert!(!live_of(&b.id));

    state.start_run_script(&a.id, "serve").unwrap();
    assert!(eventually(|| live_of(&a.id)));
    assert!(!live_of(&b.id), "b's tile stays dark: the script runs in a's workspace");

    state.stop_run_script(&a.id, "serve").unwrap();
    assert!(!live_of(&a.id));

    state.discard_run(&a.id).unwrap();
    state.discard_run(&b.id).unwrap();
}

#[test]
fn one_app_at_a_time_stops_every_other_script_in_the_project() {
    use agency_core::config::RunScript;

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();
    let target = format!("project:{}", project.id);

    state
        .save_run_scripts(
            &target,
            vec![
                RunScript {
                    name: "a".into(),
                    command: "sleep 30".into(),
                    web: false,
                    nonconcurrent: false,
                },
                RunScript {
                    name: "fixed".into(),
                    command: "sleep 30".into(),
                    web: false,
                    nonconcurrent: true,
                },
            ],
        )
        .unwrap();

    let running = |name: &str| {
        matches!(
            state
                .run_scripts_status(&target)
                .unwrap()
                .into_iter()
                .find(|s| s.name == name)
                .map(|s| s.status),
            Some(agency_core::term::SessionStatus::Running)
        )
    };

    state.start_run_script(&target, "a").unwrap();
    assert!(eventually(|| running("a")));
    // "fixed" binds a fixed port, so starting it clears the field first.
    state.start_run_script(&target, "fixed").unwrap();
    assert!(eventually(|| running("fixed")));
    assert!(!running("a"), "the concurrent script was stopped for the exclusive one");

    state.stop_run_script(&target, "fixed").unwrap();
}

// --- automatic fetch -------------------------------------------------------
//
// The Source Control panel no longer waits for someone to press ⟲: it asks for
// a fetch when it opens and when the window is focused, and a background sweep
// covers projects nobody is looking at. Both go through
// `fetch_project_if_due`, so what matters is that it fetches, that it refuses
// to fetch again straight away, and that a dead remote backs off instead of
// being retried by every trigger.

/// A project repo wired to a bare origin that is one commit ahead of it, so a
/// successful fetch is visible as `origin/main` moving. Returns (repo, ahead).
fn repo_behind_its_origin(dir: &Path) -> (std::path::PathBuf, String) {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let remote = dir.join("remote.git");
    let git = |cwd: &Path, args: &[&str]| {
        let out = Command::new("git").args(args).current_dir(cwd).output().unwrap();
        assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    // `-b main` is load-bearing, and its absence only shows up off this
    // machine. Without it the bare repo takes its branch from the ambient
    // `init.defaultBranch`, which is `main` in this project's dev setup but
    // `master` on a stock runner. The push then creates `main` while HEAD
    // still points at a `master` that never exists, so the clone below checks
    // nothing out ("remote HEAD refers to nonexistent ref") and the `commit -a`
    // after it fails with "nothing to commit" and an empty stderr.
    git(dir, &["init", "--bare", "-q", "-b", "main", remote.to_str().unwrap()]);
    git(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&repo, &["push", "-q", "-u", "origin", "main"]);

    // A second clone pushes a commit, leaving `repo`'s remote-tracking ref stale
    // — exactly the state a merged PR leaves behind.
    let other = dir.join("other");
    git(dir, &["clone", "-q", remote.to_str().unwrap(), other.to_str().unwrap()]);
    git(&other, &["config", "user.email", "t@e.com"]);
    git(&other, &["config", "user.name", "T"]);
    std::fs::write(other.join("README.md"), "moved on").unwrap();
    git(&other, &["commit", "-qam", "upstream work"]);
    git(&other, &["push", "-q", "origin", "main"]);

    (repo, git(&other, &["rev-parse", "HEAD"]))
}

fn rev(repo: &Path, r: &str) -> String {
    let out = Command::new("git").args(["rev-parse", r]).current_dir(repo).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn auto_fetch_updates_tracking_refs_then_throttles() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, upstream) = repo_behind_its_origin(dir.path());
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();

    assert_ne!(rev(&repo, "origin/main"), upstream, "precondition: tracking ref is stale");

    let long = std::time::Duration::from_secs(300);
    assert!(state.fetch_project_if_due(&project.id, long).unwrap(), "first request fetches");
    assert_eq!(rev(&repo, "origin/main"), upstream, "origin/main caught up");

    // A second panel opening (or the window regaining focus) a moment later must
    // not hit the network again.
    assert!(!state.fetch_project_if_due(&project.id, long).unwrap(), "throttled");
    // …but a caller that accepts any staleness still gets one.
    assert!(state.fetch_project_if_due(&project.id, std::time::Duration::ZERO).unwrap());
}

#[test]
fn auto_fetch_backs_off_a_broken_remote() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let missing = dir.path().join("not-a-remote.git");
    assert!(Command::new("git")
        .args(["remote", "add", "origin", missing.to_str().unwrap()])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());

    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();

    assert!(state.fetch_project_if_due(&project.id, std::time::Duration::ZERO).is_err());
    // Backoff outranks the caller's staleness tolerance: without this, every
    // panel open would retry an unreachable origin and stall on it.
    assert!(
        !state.fetch_project_if_due(&project.id, std::time::Duration::ZERO).unwrap(),
        "an unreachable origin is left alone until its backoff expires"
    );
}

#[test]
fn auto_fetch_is_a_noop_without_an_origin() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();

    assert!(!state.fetch_project_if_due(&project.id, std::time::Duration::ZERO).unwrap());
}

#[test]
fn background_sweep_fetches_every_project() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, upstream) = repo_behind_its_origin(dir.path());
    let state = common::state(&dir);
    state.add_project("demo", &repo).unwrap();

    state.sweep_project_fetches(std::time::Duration::ZERO);
    assert_eq!(rev(&repo, "origin/main"), upstream);
}

#[test]
fn project_of_resolves_both_token_shapes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

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
    let project = state.add_project("demo", &repo).unwrap();

    assert_eq!(state.project_of(&format!("project:{}", project.id)).unwrap(), project.id);

    // An agent's worktree fetches through the project it was cut from: the two
    // share an object store, so fetching per run would be the same fetch twice.
    let run = state.create_run(&project.id, "p", "noop", None, "HEAD", None).unwrap();
    assert_eq!(state.project_of(&run.id).unwrap(), project.id);
    state.discard_run(&run.id).unwrap();

    assert!(state.project_of("nope").is_err());
}

/// AGE-61: a server saved before Agency could authenticate with more than one
/// agent carries the old global `userScope` flag. Reading it back must fold that
/// into the per-agent list — pinned to Claude, the only agent the old flow could
/// have registered with — so the server keeps flowing to every other agent.
#[test]
fn legacy_user_scope_flag_survives_a_save_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);

    let legacy = agency_core::mcp::McpServer {
        name: "atlassian".into(),
        url: Some("https://mcp.atlassian.com/v1/mcp".into()),
        user_scope: true,
        ..Default::default()
    };
    state.save_mcp_servers(&[legacy]).unwrap();

    let loaded = state.list_mcp_servers().unwrap();
    assert_eq!(loaded.len(), 1);
    assert!(!loaded[0].user_scope, "legacy flag is consumed on load");
    assert_eq!(loaded[0].user_scope_agents, vec!["claude".to_string()]);
    assert!(loaded[0].is_user_scope_for("claude"));
    assert!(!loaded[0].is_user_scope_for("copilot"), "Copilot still gets it emitted");

    // Un-authenticating Claude hands the server back to Agency for every agent.
    state.deauthenticate_mcp_server("claude", "atlassian").unwrap();
    let after = state.list_mcp_servers().unwrap();
    assert!(after[0].user_scope_agents.is_empty());
}

/// AGE-83: turning the knowledge graph on used to change nothing until a merge
/// landed, so the feature looked broken. Enabling it now builds the first graph,
/// and the settings UI can watch that build finish.
#[test]
fn enabling_the_knowledge_graph_builds_the_first_graph() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let p = state.add_project("demo", &repo).unwrap();

    let before = state.knowledge_config(&p.id).unwrap();
    assert!(!before.graph_built, "nothing built before the graph is enabled");
    assert!(before.graph_path.ends_with("graphify-out/graph.json"));

    // Stand in for `graphify .`: the point under test is that Agency runs the
    // configured build in the primary checkout when the graph is switched on.
    let build = "mkdir -p graphify-out && printf '{}' > graphify-out/graph.json";
    state.save_knowledge_config(&p.id, true, None, Some(build.to_string())).unwrap();

    let cfg = await_settled_build(&state, &p.id);
    assert_eq!(cfg.last_build_error, None, "the build should have succeeded");
    assert!(cfg.graph_built, "enabling the graph must produce {}", cfg.graph_path);
    assert!(repo.join("graphify-out/graph.json").is_file());
}

/// A build that fails has to say so in the settings panel — silently leaving the
/// graph unbuilt is the failure mode this whole area is about.
#[test]
fn a_failed_graph_build_reports_why() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let p = state.add_project("demo", &repo).unwrap();

    let build = "printf 'graphify: no parser for this repo\\n' >&2; exit 3";
    state.save_knowledge_config(&p.id, true, None, Some(build.to_string())).unwrap();

    let cfg = await_settled_build(&state, &p.id);
    assert!(!cfg.graph_built);
    assert_eq!(cfg.last_build_error.as_deref(), Some("graphify: no parser for this repo"));

    // A build whose command isn't installed at all is refused up front, and
    // pointed at the install the settings panel offers to run.
    state
        .save_knowledge_config(
            &p.id,
            true,
            None,
            Some("definitely-not-a-real-binary-4k2x .".into()),
        )
        .unwrap();
    let err = state.build_knowledge_graph(&p.id).unwrap_err().to_string();
    assert!(err.contains("'definitely-not-a-real-binary-4k2x' is not installed"), "{err}");
    assert!(err.contains("Install the graphify tooling"), "{err}");
}

/// Poll the knowledge config until the background build thread has finished.
fn await_settled_build(
    state: &common::TestState,
    project_id: &str,
) -> agency_app_lib::KnowledgeConfigDto {
    for _ in 0..200 {
        let cfg = state.knowledge_config(project_id).unwrap();
        if !cfg.building {
            return cfg;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("graph build never finished");
}

/// `refresh_usage` runs on every notifier tick, and it reads the registry more
/// than once per pass. An earlier version left a `self.registry.lock()` in a
/// `for` loop's iterator expression, where the guard lives for the whole loop
/// body: the next lock inside the body deadlocked the notifier thread the
/// first time any run existed. This test needs a run to reach that code at
/// all, which is why it creates one.
#[test]
fn refresh_usage_survives_a_board_with_runs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "noop".into(),
            command: "true".into(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let run = state.create_run(&project.id, "p", "noop", None, "HEAD", None).unwrap();

    // A deadlock cannot be caught by a timeout on a joined thread: the join
    // blocks forever too, and the suite hangs instead of failing. So a
    // detached watchdog aborts the process, turning a hang into a loud,
    // attributable failure.
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watchdog = done.clone();
    std::thread::spawn(move || {
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if watchdog.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
        }
        eprintln!("refresh_usage did not return within 10s: it is deadlocked");
        std::process::abort();
    });
    state.refresh_usage().unwrap();
    done.store(true, std::sync::atomic::Ordering::SeqCst);

    // "true" is not an agent whose transcript we can read, so the run reports
    // no usage at all. Absent, not zero.
    let listed = state.list_runs(&project.id).unwrap();
    let me = listed.iter().find(|r| r.id == run.id).unwrap();
    assert!(me.usage.is_none(), "an unaccountable agent must report no usage, not zero");
}

/// A user hit this: an agent branch deleted outside Agency (after its work had
/// already been merged) while the run was still on the board. "Approve & merge"
/// then evaluated `main..agent/<id>` and surfaced git's raw "ambiguous
/// argument" error with its `--` path-separator advice, which reads as an
/// Agency syntax bug and never says the branch is gone.
#[test]
fn merge_preview_explains_a_deleted_branch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "noop".into(),
            command: "true".into(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let run = state.create_run(&project.id, "p", "noop", None, "HEAD", None).unwrap();

    // Healthy first: the branch is there and the preview is a clean no-op.
    let preview = state.merge_preview(&run.id).unwrap();
    assert_eq!(preview.commits_ahead, 0, "a fresh worktree has nothing to merge");

    // Now delete the branch out from under the run, the way a history rewrite
    // or a manual cleanup would.
    let out = std::process::Command::new("git")
        .args(["worktree", "remove", "--force", &format!(".agency/worktrees/{}", run.id)])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let out = std::process::Command::new("git")
        .args(["branch", "-D", &run.branch])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let err = state.merge_preview(&run.id).unwrap_err().to_string();
    assert!(err.contains(&run.branch), "names the branch: {err}");
    assert!(err.contains("no longer exists"), "says what is wrong: {err}");
    assert!(!err.contains("ambiguous argument"), "no raw git error: {err}");
    assert!(!err.contains("rev-list"), "no internal command name: {err}");

    // The same guard covers the actions, not just the preview.
    let err = state.merge_task(&run.id).unwrap_err().to_string();
    assert!(err.contains("no longer exists"), "merge is guarded too: {err}");
}

/// The Cancel on source control's push bar has to reach the git the push is
/// waiting on. The token is registered by the worktree being pushed and looked
/// up again from the run token, so this covers the failure mode that would be
/// silent: a key computed two different ways, cancelling nothing.
#[test]
fn cancelling_a_push_stops_it() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);

    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let remote = dir.path().join("remote.git");
    assert!(Command::new("git")
        .args(["init", "--bare", "-q", remote.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["remote", "add", "origin", remote.to_str().unwrap()])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());

    let project = state.add_project("demo", &repo).unwrap();
    let token = format!("project:{}", project.id);

    // The click lands on the first progress line, which is as close to
    // mid-upload as a push to a local remote gets.
    let err = state.push_run(&token, |_| state.cancel_push(&token)).unwrap_err();
    assert_eq!(err.to_string(), agency_core::setup::CANCELLED);
}
