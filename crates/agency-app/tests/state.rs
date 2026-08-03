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

#[test]
fn add_project_rejects_non_git_folder() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);

    let plain = dir.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();

    let err = state.add_project("plain", &plain).unwrap_err().to_string();
    assert!(err.contains("git repository"), "unexpected error: {err}");
    // nothing persisted
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
    state.register_profile(AgentProfile {
        name: "stay".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "echo HI; sleep 3".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    let info = state.create_run(&project.id, "do it", "stay", "HEAD", None).unwrap();
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
        if matches!(state.run_status(&info.id).unwrap(), SessionStatus::Running | SessionStatus::Exited{..}) { ok = true; break; }
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
    state.register_profile(AgentProfile {
        name: "noop".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "sleep 1".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "noop", "HEAD", None).unwrap();

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
    state.register_profile(AgentProfile {
        name: "noop".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "sleep 1".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let agent = state.create_run(&project.id, "p", "noop", "HEAD", None).unwrap();
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
    state.save_settings(&agency_app_lib::ProviderSettings {
        lm_studio_base_url: "http://localhost:1234/v1".into(),
        default_agent: None,
        default_worktree: true,
    }).unwrap();
    // Profile echoes env vars and then sleeps so we can capture output.
    state.register_profile(AgentProfile {
        name: "envcheck".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "echo BASE=$OPENAI_BASE_URL; echo KEYSET=${OPENAI_API_KEY:+yes}; sleep 2".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    let info = state.create_run(&project.id, "p", "envcheck", "HEAD", None).unwrap();

    // Poll tmux capture until we see the output (up to 5s)
    let mut out = String::new();
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        if let Ok(s) = state.run_preview(&info.id, 20) {
            out = s;
            if out.contains("BASE=http://localhost:1234/v1") { break; }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    assert!(out.contains("BASE=http://localhost:1234/v1"), "got: {out}");
    assert!(out.contains("KEYSET=yes"), "got: {out}");

    state.discard_run(&info.id).unwrap();
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
    state.register_profile(AgentProfile {
        name: "noop".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "sleep 1".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "noop", "HEAD", None).unwrap();

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
    for ok in ["http://localhost:1234/v1", "http://[::1]:1234/v1", "https://api.example.com/v1", ""] {
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
    state.register_profile(AgentProfile {
        name: "echoer".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "echo READY; read x; echo GOT:$x; sleep 3".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "echoer", "HEAD", None).unwrap();

    let buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let b = buf.clone();
    state.attach_run(&info.id, 220, 50, move |bytes| {
        b.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes));
    }).unwrap();

    let wait = |needle: &str| {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            if buf.lock().unwrap().contains(needle) { return true; }
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
    state.register_profile(AgentProfile {
        name: "noop".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "sleep 1".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    // create_run → worktree on agent/<id>; make a non-conflicting commit in it.
    let info = state.create_run(&project.id, "p", "noop", "HEAD", None).unwrap();
    let wt = state.worktree_path(&info.id).unwrap();
    std::fs::write(wt.join("feature.txt"), "x\n").unwrap();
    std::process::Command::new("git").args(["add","-A"]).current_dir(&wt).status().unwrap();
    std::process::Command::new("git").args(["commit","-qm","feat"]).current_dir(&wt).status().unwrap();

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
    state.register_profile(AgentProfile {
        name: "sleeper".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "sleep 5".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "sleeper", "HEAD", None).unwrap();

    // Stop the run's session (record stays) -> status becomes Gone.
    state.stop_run(&info.id).unwrap();
    let mut gone = false;
    for _ in 0..75 {
        if matches!(state.run_status(&info.id).unwrap(), SessionStatus::Gone) { gone = true; break; }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(gone, "session did not become Gone after stop_run");

    // Reactivate -> a new session comes up (fresh fallback, since resume_args is None).
    state.ensure_run_active(&info.id).unwrap();
    let mut back = false;
    for _ in 0..75 {
        if !matches!(state.run_status(&info.id).unwrap(), SessionStatus::Gone) { back = true; break; }
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
    state.register_profile(AgentProfile {
        name: "flaky".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "printf FRESH; sleep 5".into()],
        env: vec![],
        resume_args: Some(vec!["-c".into(), "printf NO-CONV; exit 1".into()]),
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "flaky", "HEAD", None).unwrap();

    state.stop_run(&info.id).unwrap();
    let mut gone = false;
    for _ in 0..75 {
        if matches!(state.run_status(&info.id).unwrap(), SessionStatus::Gone) { gone = true; break; }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(gone, "session did not become Gone after stop_run");

    // Reactivate: resume (NO-CONV; exit 1) fails fast -> fallback fresh (FRESH; sleep 5).
    state.ensure_run_active(&info.id).unwrap();
    let mut fresh = false;
    for _ in 0..150 {
        let cap = state.run_preview(&info.id, 10).unwrap_or_default();
        if cap.contains("FRESH") && matches!(state.run_status(&info.id).unwrap(), SessionStatus::Running) {
            fresh = true; break;
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
    state.register_profile(AgentProfile {
        name: "pwds".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "pwd; sleep 5".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let run = state.create_run(&project.id, "p", "pwds", "HEAD", None).unwrap();
    let wt = state.worktree_path(&run.id).unwrap();

    // First extra tab: defaults to the run's agent, gets the --2 suffix.
    let s2 = state.start_run_session(&run.id, None, "").unwrap();
    assert_eq!(s2.id, format!("{}--2", run.id));
    assert_eq!(s2.agent, "pwds");

    // It runs in the SAME worktree as the primary agent.
    let mut cwd_ok = false;
    for _ in 0..150 {
        let cap = state.run_preview(&s2.id, 10).unwrap_or_default();
        if cap.contains(&wt.to_string_lossy().to_string()) { cwd_ok = true; break; }
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
        if matches!(state.run_status(&s2.id).unwrap(), SessionStatus::Gone) { gone = true; break; }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(gone, "closed tab session still present");
    assert_eq!(state.run_sessions(&run.id).unwrap().len(), 1);
    assert!(!matches!(state.run_status(&run.id).unwrap(), SessionStatus::Gone));

    // Discarding the run sweeps the remaining tab: session and row.
    state.discard_run(&run.id).unwrap();
    let mut swept = false;
    for _ in 0..75 {
        if matches!(state.run_status(&s3.id).unwrap(), SessionStatus::Gone) { swept = true; break; }
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
    state.register_profile(AgentProfile {
        name: "pwds".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "pwd; sleep 5".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let run = state.create_run(&project.id, "p", "pwds", "HEAD", None).unwrap();
    let wt = state.worktree_path(&run.id).unwrap();
    assert!(state.profile_names().unwrap().iter().all(|n| n != "shell"));

    let tab = state.start_run_session(&run.id, Some("shell"), "").unwrap();
    assert_eq!(tab.agent, "shell");

    // Prove it's a live shell rooted in the run's worktree.
    state.run_input(&tab.id, b"pwd\n").unwrap();
    let mut cwd_ok = false;
    for _ in 0..150 {
        let cap = state.run_preview(&tab.id, 20).unwrap_or_default();
        if cap.contains(&wt.to_string_lossy().to_string()) { cwd_ok = true; break; }
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
    state.register_profile(AgentProfile {
        name: "pwds".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "pwd; sleep 5".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let run = state.create_run(&project.id, "p", "pwds", "HEAD", None).unwrap();
    let wt = state.worktree_path(&run.id).unwrap();
    let tab = state.start_run_session(&run.id, None, "").unwrap();

    state.stop_run(&tab.id).unwrap();
    let mut gone = false;
    for _ in 0..75 {
        if matches!(state.run_status(&tab.id).unwrap(), SessionStatus::Gone) { gone = true; break; }
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
    state
        .add_review_comment(fake_run_id, "src/main.rs", 1, 3, "looks good")
        .unwrap();

    // send_review_comments must fail because the session is not running.
    let err = state.send_review_comments(fake_run_id).unwrap_err().to_string();
    assert!(
        err.contains("not running"),
        "expected 'not running' error, got: {err}"
    );

    // The comment must NOT have been marked sent.
    let unsent = state.list_review_comments(fake_run_id).unwrap();
    assert_eq!(unsent.len(), 1, "comment count should be unchanged");
    assert!(!unsent[0].sent, "comment must NOT be marked sent after failed send");
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
    state.register_profile(AgentProfile {
        name: "pwds".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "pwd; sleep 5".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    let info = state
        .create_run_with_progress(&project.id, "p", "pwds", "HEAD", None, false, |_| {})
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

/// Discarding such a run must not touch git: `main` is the user's branch, and
/// the worktree-removal path would try to delete it.
#[test]
fn discard_without_worktree_leaves_the_checkout_and_its_branch_alone() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    state.register_profile(AgentProfile {
        name: "noop".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "sleep 1".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state
        .create_run_with_progress(&project.id, "p", "noop", "HEAD", None, false, |_| {})
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
    state.register_profile(AgentProfile {
        name: "noop".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "sleep 1".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state
        .create_run_with_progress(&project.id, "p", "noop", "HEAD", None, false, |_| {})
        .unwrap();
    std::fs::write(repo.join("scratch.txt"), "work in progress").unwrap();

    let head_before = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .unwrap()
        .stdout;

    state.archive_run(&info.id).unwrap();
    assert_eq!(state.list_archived_runs(&project.id).unwrap().len(), 1);

    let head_after = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .unwrap()
        .stdout;
    assert_eq!(head_before, head_after, "archive must not commit anything");
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo)
        .output()
        .unwrap();
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
    state.register_profile(AgentProfile {
        name: "noop".into(),
        command: "sh".into(),
        args: vec!["-c".into(), "sleep 1".into()],
        env: vec![],
        resume_args: None,
        loop_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    let mut runs = Vec::new();
    for prompt in ["a", "b", "c"] {
        runs.push(
            state
                .create_run_with_progress(&project.id, prompt, "noop", "HEAD", None, true, |_| {})
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
