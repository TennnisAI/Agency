use agency_app_lib::{ActivityState, AppState, NoticeKind};
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
    // AGE-186: no local model until the user names one. The settings field
    // shows LM Studio's port as a placeholder, not as text they have to delete.
    assert_eq!(s.lm_studio_base_url, "");
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

    // Sentinels in the environment the daemon inherits, so the two halves of
    // this test can tell "Agency injected it" from "it was already there".
    // Agency is routinely run from a shell that already has these two set,
    // because Agency itself sets them in every shell it opens.
    std::env::set_var("OPENAI_BASE_URL", "http://inherited.invalid/v1");
    std::env::set_var("OPENAI_API_KEY", "inherited-key");

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
                "echo BASE=$OPENAI_BASE_URL; echo KEY=$OPENAI_API_KEY; sleep 2".into(),
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
    assert!(out.contains("KEY=lm-studio"), "got: {out}");

    state.discard_run(&info.id).unwrap();

    // AGE-186: blanking the field has to set neither variable. It used to set
    // OPENAI_BASE_URL="" and a placeholder OPENAI_API_KEY regardless, which
    // shadowed the user's real key in every session Agency opens.
    state
        .save_settings(&agency_app_lib::ProviderSettings {
            lm_studio_base_url: "".into(),
            default_agent: None,
            default_worktree: true,
        })
        .unwrap();
    let off = state.create_run(&project.id, "p", "envcheck", None, "HEAD", None).unwrap();
    let mut out = String::new();
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        if let Ok(s) = state.run_preview(&off.id, 20) {
            out = s;
            if out.contains("KEY=") {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // The inherited values come through untouched: Agency set neither.
    assert!(out.contains("BASE=http://inherited.invalid/v1"), "got: {out}");
    assert!(out.contains("KEY=inherited-key"), "got: {out}");

    state.discard_run(&off.id).unwrap();
}

/// AGE-186: the local-model URL used to arrive prefilled with LM Studio's
/// default port as real, selectable text, under copy that said "leave blank to
/// disable". Nobody chose it: the getter invented it whenever the row was
/// unset, and any unrelated save (the worktree toggle, the default-agent
/// select) persisted the whole struct and wrote it back. The clear runs once,
/// so the same URL typed in deliberately survives the next launch.
#[test]
fn the_prefilled_local_model_url_is_cleared_once() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agency.db");
    {
        let reg = agency_core::registry::Registry::open(&db).unwrap();
        reg.set_setting("lm_studio_base_url", "http://localhost:1234/v1").unwrap();
    }
    {
        let state = common::state(&dir);
        assert_eq!(state.get_settings().unwrap().lm_studio_base_url, "");
        state
            .save_settings(&agency_app_lib::ProviderSettings {
                lm_studio_base_url: "http://localhost:1234/v1".into(),
                default_agent: None,
                default_worktree: true,
            })
            .unwrap();
    }
    let state = common::state(&dir);
    assert_eq!(state.get_settings().unwrap().lm_studio_base_url, "http://localhost:1234/v1");
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

/// Settings and the launch picker write the same per-agent setting, so a model
/// chosen in Settings is the one the next run starts on and the picker reopens
/// on. Two stores would need a rule for which wins, with the loser ignored.
#[test]
fn set_agent_model_writes_what_the_picker_reads() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "claude".into(),
            command: "claude".into(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();

    let entry = |state: &AppState| {
        state
            .list_agent_models()
            .unwrap()
            .into_iter()
            .find(|m| m.agent == "claude")
            .expect("claude in the model list")
    };

    // Nothing chosen yet: the agent's own default, and nothing in the recents.
    assert_eq!(entry(&state).selected, None);
    assert!(entry(&state).recent.is_empty());

    state.set_agent_model("claude", Some("opus")).unwrap();
    assert_eq!(entry(&state).selected.as_deref(), Some("opus"));
    assert_eq!(entry(&state).recent, ["opus"]);

    // Choosing again puts the newest first without repeating an entry, which is
    // what makes a typed id a one-click choice the second time.
    state.set_agent_model("claude", Some("sonnet")).unwrap();
    state.set_agent_model("claude", Some("opus")).unwrap();
    assert_eq!(entry(&state).selected.as_deref(), Some("opus"));
    assert_eq!(entry(&state).recent, ["opus", "sonnet"]);

    // The agent's own default is a choice in its own right: it clears the
    // selection and leaves the recents alone.
    state.set_agent_model("claude", None).unwrap();
    assert_eq!(entry(&state).selected, None);
    assert_eq!(entry(&state).recent, ["opus", "sonnet"]);

    // An empty id says the same thing as null. Stored as a model it would sit
    // at the head of the recents as a blank row.
    state.set_agent_model("claude", Some("  ")).unwrap();
    assert_eq!(entry(&state).selected, None);
    assert_eq!(entry(&state).recent, ["opus", "sonnet"]);

    // An agent with no profile is a stale UI, not a setting to write.
    assert!(state.set_agent_model("nope", Some("opus")).is_err());
}

/// A stand-in for an agent CLI's listing command: it records the arguments it
/// was handed (so the test can prove they came from the catalog) and prints the
/// models substituted into it.
const FAKE_CLI: &str =
    concat!("#!/bin/sh\n", "echo \"$@\" >> \"$0.calls\"\n", "printf '{{models}}\\n'\n",);

/// The same, but its answer is the directory it was run in: the whole point of
/// a project-scoped listing is that the same command says different things in
/// different places, and this is the smallest CLI that behaves that way.
const FAKE_CWD_CLI: &str = concat!(
    "#!/bin/sh\n",
    "echo \"$@\" >> \"$0.calls\"\n",
    "printf 'local/%s\\n' \"$(basename \"$(pwd)\")\"\n",
);

/// Write `body` as an executable stand-in CLI at `path`.
fn write_fake_cli(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// How many times a stand-in CLI has been run.
fn fake_cli_calls(path: &Path) -> usize {
    std::fs::read_to_string(path.with_extension("calls")).map_or(0, |s| s.lines().count())
}

/// The picker's on-demand probe: run the agent's own listing command, keep
/// what it says for the session, and never run it for an agent that has no
/// such command (AGE-117).
#[test]
fn probing_an_agents_models_runs_its_own_cli_once_per_session() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);

    // Stand in for `opencode models`: the catalog supplies the arguments, so
    // what this proves is that the *profile's* command is what gets run.
    let fake = dir.path().join("fake-opencode");
    write_fake_cli(
        &fake,
        &FAKE_CLI.replace("{{models}}", "openai/gpt-5.6\nanthropic/claude-opus-5"),
    );
    state
        .register_profile(AgentProfile {
            name: "opencode".into(),
            command: fake.to_string_lossy().into_owned(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();

    let models = state.probe_agent_models("opencode", None).unwrap();
    assert_eq!(models, ["openai/gpt-5.6", "anthropic/claude-opus-5"]);
    // The arguments came from the catalog, not from the profile.
    let calls = std::fs::read_to_string(dir.path().join("fake-opencode.calls")).unwrap();
    assert_eq!(calls.trim(), "models");

    // Cached for the session: a second picker open costs nothing, so the CLI
    // is not run again even though its answer has changed.
    write_fake_cli(&fake, &FAKE_CLI.replace("{{models}}", "openai/gpt-9"));
    assert_eq!(state.probe_agent_models("opencode", None).unwrap(), models);
    assert_eq!(fake_cli_calls(&fake), 1);

    // And the hint the picker shows names the command the probe runs.
    let listed = state.list_agent_models().unwrap();
    let opencode = listed.iter().find(|m| m.agent == "opencode").expect("opencode in the list");
    assert_eq!(opencode.list_command.as_deref(), Some("opencode models"));
}

/// opencode merges an `opencode.json` from the directory it runs in over the
/// user's own, so asking it from home lists the home directory's providers and
/// a project's own are simply missing (AGE-135). The probe therefore runs in
/// the project, and caches per project rather than per agent.
#[test]
fn a_project_scoped_cli_is_asked_in_the_project_it_is_picking_for() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);

    let fake = dir.path().join("fake-opencode");
    write_fake_cli(&fake, FAKE_CWD_CLI);
    state
        .register_profile(AgentProfile {
            name: "opencode".into(),
            command: fake.to_string_lossy().into_owned(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();

    let one = dir.path().join("one");
    let two = dir.path().join("two");
    std::fs::create_dir_all(&one).unwrap();
    std::fs::create_dir_all(&two).unwrap();
    let one = state.add_project("one", &one).unwrap();
    let two = state.add_project("two", &two).unwrap();

    // Each project gets the answer its own directory gives, not the other's.
    assert_eq!(state.probe_agent_models("opencode", Some(&one.id)).unwrap(), ["local/one"]);
    assert_eq!(state.probe_agent_models("opencode", Some(&two.id)).unwrap(), ["local/two"]);
    assert_eq!(fake_cli_calls(&fake), 2);

    // And the cache is per project: reopening the first project's picker is
    // still free, and does not hand it the second project's list.
    assert_eq!(state.probe_agent_models("opencode", Some(&one.id)).unwrap(), ["local/one"]);
    assert_eq!(fake_cli_calls(&fake), 2);

    // A picker with no project falls back to home, which is a third answer and
    // so a third entry rather than either project's.
    assert!(state.probe_agent_models("opencode", None).is_ok());
    assert_eq!(fake_cli_calls(&fake), 3);

    // A project id that names nothing is a bug in the caller, and says so
    // rather than quietly answering from somewhere else.
    let err =
        state.probe_agent_models("opencode", Some("no-such-project")).unwrap_err().to_string();
    assert!(err.contains("unknown project"), "{err}");
}

/// pi's providers are configured once for the user, so its answer is the same
/// in every project. It keeps the single cache entry it had: a probe per
/// project would be a second ~1s wait for a list that cannot have changed.
#[test]
fn a_user_scoped_cli_is_asked_once_for_every_project() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);

    let fake = dir.path().join("fake-pi");
    write_fake_cli(
        &fake,
        &FAKE_CLI.replace(
            "{{models}}",
            "provider  model            context\nanthropic  claude-opus-5   200K",
        ),
    );
    state
        .register_profile(AgentProfile {
            name: "pi".into(),
            command: fake.to_string_lossy().into_owned(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();

    let one = dir.path().join("one");
    let two = dir.path().join("two");
    std::fs::create_dir_all(&one).unwrap();
    std::fs::create_dir_all(&two).unwrap();
    let one = state.add_project("one", &one).unwrap();
    let two = state.add_project("two", &two).unwrap();

    let models = state.probe_agent_models("pi", Some(&one.id)).unwrap();
    assert_eq!(models, ["anthropic/claude-opus-5"]);
    assert_eq!(state.probe_agent_models("pi", Some(&two.id)).unwrap(), models);
    assert_eq!(state.probe_agent_models("pi", None).unwrap(), models);
    assert_eq!(fake_cli_calls(&fake), 1);
}

/// Most of these CLIs cannot be asked at all, and a probe that guessed a flag
/// would launch the agent instead of questioning it.
#[test]
fn an_agent_with_no_listing_command_is_never_probed() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "claude".into(),
            command: "false".into(),
            args: vec![],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();

    for agent in ["claude", "my-own-agent"] {
        let err = state.probe_agent_models(agent, None).unwrap_err().to_string();
        assert!(err.contains("no command for listing its models"), "{agent}: {err}");
    }
    // The picker shows no hint for them either, so nothing offers the probe.
    let listed = state.list_agent_models().unwrap();
    assert!(listed.iter().all(|m| m.list_command.is_none()));
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

    let err = state.send_merge_conflict(&info.id, None).unwrap_err().to_string();
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

/// AGE-201: merging an agent's branch here is the one ending that could not
/// reach the remote. The teardown that follows takes the worktree and the
/// local branch; the published copy stayed on the remote for good, and a
/// month of agents leaves a month of branches.
#[test]
fn merging_can_take_the_agent_branch_off_the_remote() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let git = |at: &Path, args: &[&str]| {
        let out = Command::new("git").args(args).current_dir(at).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    };
    let origin = dir.path().join("origin.git");
    git(dir.path(), &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
    git(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&repo, &["push", "-q", "origin", "main"]);

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
    std::fs::write(wt.join("feature.txt"), "x\n").unwrap();
    git(&wt, &["add", "-A"]);
    git(&wt, &["commit", "-qm", "feat"]);
    git(&wt, &["push", "-q", "origin", &info.branch]);

    let tracking = format!("origin/{}", info.branch);
    // The window can only offer this because the preview says the name is out
    // there; an unpublished branch has nothing to offer.
    let preview = state.merge_preview(&info.id).unwrap();
    assert_eq!(preview.remote_branch.as_deref(), Some(tracking.as_str()));

    // Before the merge the remote copy is the only copy off this machine, and
    // the deletion is refused rather than trusted to the caller's ordering.
    let err = state.delete_run_remote_branch(&info.id).unwrap_err().to_string();
    assert!(err.contains("aren't on main"), "explains the refusal: {err}");

    assert!(matches!(state.merge_task(&info.id).unwrap(), MergeOutcome::Clean { .. }));
    assert_eq!(state.delete_run_remote_branch(&info.id).unwrap(), Some(tracking.clone()));
    let listed = Command::new("git").args(["branch"]).current_dir(&origin).output().unwrap();
    let listed = String::from_utf8_lossy(&listed.stdout).to_string();
    assert!(!listed.contains(&info.branch), "origin still has the branch: {listed}");
    // A second press is a no-op, not an error: the window offers this straight
    // after a merge, and by then GitHub may have deleted the branch itself.
    assert_eq!(state.delete_run_remote_branch(&info.id).unwrap(), None);
    // The local branch is the teardown's to take, with the worktree, once the
    // user picks archive or delete.
    assert!(state.run_cleanup(&info.id).unwrap().facts.merged);

    let _ = state.discard_run(&info.id);
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
    // Fake agent: one script that fails fast when handed the resume flag and
    // otherwise prints FRESH and stays. A profile whose two recipes are both
    // `sh -c <script>` would not do since AGE-100: a resume launches the
    // profile's own args ahead of the resume recipe, so the second `-c` would
    // never be read and the resume would "succeed" as the fresh command.
    let fake = dir.path().join("flaky-agent");
    std::fs::write(
        &fake,
        "#!/bin/sh\nfor a in \"$@\"; do\n  [ \"$a\" = --continue ] && { printf NO-CONV; exit 1; }\ndone\nprintf FRESH\nsleep 5\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    state
        .register_profile(AgentProfile {
            name: "flaky".into(),
            command: fake.to_string_lossy().into_owned(),
            args: vec![],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
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

/// AGE-100: the flags a user put in the profile's arguments are part of how the
/// agent runs, not part of the prompt, so a resume has to carry them too. This
/// drives the real launch path, where the resume argv used to be the resume
/// recipe on its own.
#[test]
fn ensure_run_active_resumes_with_the_profiles_own_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    // Fake agent: prints the argv it was launched with, then stays up.
    let fake = dir.path().join("echo-agent");
    std::fs::write(&fake, "#!/bin/sh\nprintf 'ARGV[%s]' \"$@\"\nsleep 5\n").unwrap();
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    state
        .register_profile(AgentProfile {
            name: "echoer".into(),
            command: fake.to_string_lossy().into_owned(),
            args: vec!["--permission-mode".into(), "acceptEdits".into()],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "echoer", None, "HEAD", None).unwrap();

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

    state.ensure_run_active(&info.id).unwrap();
    let mut argv = String::new();
    for _ in 0..150 {
        argv = state.run_preview(&info.id, 10).unwrap_or_default().replace('\n', "");
        if argv.contains("ARGV[--continue]") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(
        argv.contains("ARGV[--permission-mode]ARGV[acceptEdits]ARGV[--continue]"),
        "resume dropped the profile's arguments: {argv:?}"
    );
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

    // AGE-184: the run's own tab closes like any other while a tab is left to
    // carry the workspace. The run stays — it is the worktree and the branch,
    // not the session — and its status now comes from the tab that is left.
    state.close_run_session(&run.id).unwrap();
    let mut primary_gone = false;
    for _ in 0..75 {
        if matches!(state.run_status(&run.id).unwrap(), SessionStatus::Gone) {
            primary_gone = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(primary_gone, "the run's own session survived its tab being closed");
    let info = state.list_runs(&project.id).unwrap().into_iter().find(|r| r.id == run.id).unwrap();
    assert!(info.primary_closed, "the close must survive as more than a dead session");
    assert!(
        !matches!(info.status, SessionStatus::Gone),
        "the remaining tab carries the run's status"
    );
    // Nothing revives a tab the user closed: this is the call every attach
    // makes, and before AGE-184 it would have started the agent straight back.
    state.ensure_run_active(&run.id).unwrap();
    assert!(matches!(state.run_status(&run.id).unwrap(), SessionStatus::Gone));

    // And the last one standing cannot be closed — that is what archive and
    // delete are for.
    let err = state.close_run_session(&s3.id).unwrap_err().to_string();
    assert!(err.contains("last agent"), "expected a last-agent refusal, got: {err}");

    // The way back: the + menu's reopen clears the stamp and starts the agent.
    state.reopen_primary_session(&run.id).unwrap();
    let info = state.list_runs(&project.id).unwrap().into_iter().find(|r| r.id == run.id).unwrap();
    assert!(!info.primary_closed);
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

/// The regression this pins: quitting Agency kills every daemon session and
/// shuts the daemon down, so on the next launch every extra tab reads as gone.
/// The strip dropped a gone tab, so a worktree with three agents in it came
/// back from a restart with one. A tab is the registry row, not the process.
#[test]
fn extra_tabs_outlive_their_sessions_and_come_back() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "sleeper".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 30".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let run = state.create_run(&project.id, "p", "sleeper", None, "HEAD", None).unwrap();
    let s2 = state.start_run_session(&run.id, None, "").unwrap();
    let s3 = state.start_run_session(&run.id, None, "").unwrap();

    // The quit, as far as an extra tab can tell: its session is killed and
    // nothing revives it until the tab is looked at again.
    for id in [&s2.id, &s3.id] {
        state.stop_run(id).unwrap();
        let mut gone = false;
        for _ in 0..75 {
            if matches!(state.run_status(id).unwrap(), SessionStatus::Gone) {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        assert!(gone, "extra session {id} did not become Gone after stop_run");
    }

    // Both tabs are still the run's, dead sessions and all.
    assert_eq!(state.run_sessions(&run.id).unwrap().len(), 2);

    // And they still count as tabs. The run's own can go because two are left
    // to carry the workspace; the last one standing is still refused, even
    // though nothing of it is running.
    state.close_run_session(&run.id).unwrap();
    state.close_run_session(&s3.id).unwrap();
    let err = state.close_run_session(&s2.id).unwrap_err().to_string();
    assert!(err.contains("last agent"), "expected a last-agent refusal, got: {err}");

    // Selecting the tab is what brings its agent back — the same call the
    // terminal pane makes on mount.
    state.ensure_run_active(&s2.id).unwrap();
    let mut back = false;
    for _ in 0..150 {
        if !matches!(state.run_status(&s2.id).unwrap(), SessionStatus::Gone) {
            back = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(back, "the tab's agent did not come back up");
    state.discard_run(&run.id).unwrap();
}

/// AGE-175 made extras launch fresh every time, because a resume recipe means
/// "the most recent conversation in this directory" and a sibling tab shares
/// the directory. Session stores and minted conversation ids answered that, so
/// a tab coming back after a restart comes back into its own conversation
/// rather than staring at a blank agent.
#[test]
fn an_extra_tab_relaunches_on_the_conversation_it_owns() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state_with_agent_home(&dir, home.path());
    // The launch command's basename is what decides whether Agency can name
    // this agent's conversations, so the fake has to be called `claude`.
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("claude");
    std::fs::write(&fake, "#!/bin/sh\nprintf 'ARGV[%s]' \"$@\"\nsleep 30\n").unwrap();
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    state
        .register_profile(AgentProfile {
            name: "claude".into(),
            command: fake.to_string_lossy().into_owned(),
            args: vec![],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let run = state.create_run(&project.id, "p", "claude", None, "HEAD", None).unwrap();
    let tab = state.start_run_session(&run.id, None, "").unwrap();

    // The conversation Agency minted for this tab, read off the argv the tab
    // was opened with.
    let mut argv = String::new();
    for _ in 0..150 {
        argv = state.run_preview(&tab.id, 10).unwrap_or_default().replace('\n', "");
        if argv.contains("ARGV[--session-id]") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    let conversation = argv
        .split("ARGV[--session-id]ARGV[")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .unwrap_or_else(|| panic!("the tab opened no named conversation: {argv:?}"))
        .to_string();

    // The transcript is what the resume probe asks about: without one there is
    // nothing to come back to, and claude will not start on a `--resume` id
    // that is not there.
    let wt = state.worktree_path(&run.id).unwrap();
    let transcript =
        home.path().join(".claude").join("projects").join(agency_core::usage::claude_enc(&wt));
    std::fs::create_dir_all(&transcript).unwrap();
    std::fs::write(transcript.join(format!("{conversation}.jsonl")), "{}\n").unwrap();

    state.stop_run(&tab.id).unwrap();
    let mut gone = false;
    for _ in 0..75 {
        if matches!(state.run_status(&tab.id).unwrap(), SessionStatus::Gone) {
            gone = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(gone, "the tab's session did not become Gone after stop_run");

    state.ensure_run_active(&tab.id).unwrap();
    let want = format!("ARGV[--resume]ARGV[{conversation}]");
    let mut resumed = String::new();
    for _ in 0..150 {
        resumed = state.run_preview(&tab.id, 10).unwrap_or_default().replace('\n', "");
        if resumed.contains(&want) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(
        resumed.contains(&want),
        "the tab did not come back on its own conversation: {resumed:?}"
    );
    state.discard_run(&run.id).unwrap();
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

/// A web-served agent (dsh) binds one port for the whole workspace, so exactly
/// one session may serve it — and whichever session that is, the run has to
/// name it, or its GUI is a server nobody in the app can reach.
#[test]
fn one_web_gui_session_per_workspace_and_the_run_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    // Registered under the catalog's own name, which is what the web recipe is
    // keyed by; the command only has to exist so the tab can be launched.
    for name in ["dsh", "idle"] {
        state
            .register_profile(AgentProfile {
                name: name.into(),
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 30".into()],
                env: vec![],
                resume_args: None,
                loop_args: None,
            })
            .unwrap();
    }
    let project = state.add_project("demo", &repo).unwrap();

    // A run whose own agent serves the GUI: the primary session owns the port.
    let web = state.create_run(&project.id, "", "dsh", None, "HEAD", None).unwrap();
    assert_eq!(web.gui_session_id.as_deref(), Some(web.id.as_str()));
    assert!(web.gui_port.is_some(), "a dsh run with a port block has a GUI port");
    // A second one in the same workspace would lose the bind; refuse with why.
    let err = state.start_run_session(&web.id, Some("dsh"), "").unwrap_err().to_string();
    assert!(err.contains("already uses"), "unexpected error: {err}");

    // A run whose agent is a plain terminal one: no GUI until a web tab opens,
    // and then the tab — not the run — is what the pane keys off.
    let plain = state.create_run(&project.id, "", "idle", None, "HEAD", None).unwrap();
    assert_eq!(plain.gui_port, None);
    assert_eq!(plain.gui_session_id, None);
    let tab = state.start_run_session(&plain.id, Some("dsh"), "").unwrap();
    let refreshed = state
        .list_runs(&project.id)
        .unwrap()
        .into_iter()
        .find(|r| r.id == plain.id)
        .expect("run still listed");
    assert_eq!(refreshed.gui_session_id.as_deref(), Some(tab.id.as_str()));
    // Its own block's port, not the dsh run's: each workspace serves its own.
    assert_eq!(refreshed.gui_port, refreshed.port.map(|base| base + 8));
    assert_ne!(refreshed.gui_port, web.gui_port);

    state.discard_run(&web.id).unwrap();
    state.discard_run(&plain.id).unwrap();
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

/// The hole AGE-127 closes: a queue thrown away for a session that has since
/// exited used to go to the log and nowhere else, so the marker appeared for a
/// tick and then vanished, which from the user's side is what a message going
/// in looks like.
#[test]
fn a_queue_dropped_for_a_gone_session_is_reported_rather_than_just_logged() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();
    let id = live_terminal(&state, &project.id);

    state.add_review_comment(&id, "src/a.rs", 7, 7, "never-arrives-marker").unwrap();
    // Unobserved sessions count as working, so this is held rather than typed.
    assert!(!state.send_review_comments(&id).unwrap());
    assert_eq!(state.list_queued_messages(&id).len(), 1);

    // The session goes away with the message still held.
    state.run_input(&id, b"exit\r").unwrap();
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        if !matches!(state.run_status(&id).unwrap(), SessionStatus::Running) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(
        !matches!(state.run_status(&id).unwrap(), SessionStatus::Running),
        "shell never exited"
    );

    let notices = state.drain_send_queues(now_ms());
    assert_eq!(notices.len(), 1, "the drop must come back to the caller, not only to the log");
    assert_eq!(notices[0].kind, NoticeKind::Dropped);
    assert_eq!(notices[0].run_id, id);
    assert!(notices[0].text.contains("review comments"), "{}", notices[0].text);
    assert!(notices[0].text.contains("that session is gone"), "{}", notices[0].text);
    assert!(state.list_queued_messages(&id).is_empty(), "the queue is gone with the session");
    // Said once. The tick runs every few seconds and the queue is already
    // empty, so there is nothing left to report.
    assert!(state.drain_send_queues(now_ms()).is_empty());
    // And it does not come back on the next launch either.
    let reopened = AppState::new(&dir.path().join("agency.db"), dir.path()).unwrap();
    assert!(reopened.list_queued_messages(&id).is_empty());

    let _ = state.discard_run(&id);
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

/// Whether a teardown touches the agent's conversation at all, which is the one
/// thing archive and delete really differ over and so the one thing the dialogs
/// have to get right.
///
/// Both halves are load-bearing, and both were being claimed wrongly by copy
/// that said "the transcript included" for every agent: Agency only knows where
/// two agents keep their sessions, and even for those it leaves a checkout run's
/// directory alone, because that directory holds the user's own conversations in
/// that folder. Users noticed — deleting an agent and then resuming it from the
/// agent itself is the normal experience for everyone else.
#[test]
fn only_an_agent_agency_can_read_has_a_transcript_the_teardown_touches() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    // `manages_transcript` keys off the *basename* of the profile's command, so
    // a stand-in named `claude` proves the same thing the real CLI would. Using
    // the real one made this test pass only on a machine that had claude
    // installed: CI has no such binary, and create_run failed there with
    // "Unable to spawn claude because it doesn't exist on the filesystem".
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    write_fake_cli(&claude, "#!/bin/sh\nsleep 1\n");

    for (name, command) in [("claude", claude.to_str().unwrap()), ("other", "sh")] {
        state
            .register_profile(AgentProfile {
                name: name.into(),
                command: command.into(),
                args: vec!["-c".into(), "sleep 1".into()],
                env: vec![],
                resume_args: None,
                loop_args: None,
            })
            .unwrap();
    }
    let project = state.add_project("demo", &repo).unwrap();

    let known = state.create_run(&project.id, "a", "claude", None, "main", None).unwrap();
    assert!(
        state.run_cleanup(&known.id).unwrap().manages_transcript,
        "claude's session layout is one of the two Agency reads"
    );

    let unknown = state.create_run(&project.id, "b", "other", None, "main", None).unwrap();
    assert!(
        !state.run_cleanup(&unknown.id).unwrap().manages_transcript,
        "an agent whose transcript format is unknown keeps its own history"
    );

    // Same agent, no worktree: its session directory is the one the user's own
    // conversations in this folder live in, so neither verb may claim it.
    let checkout = state
        .create_run_with_progress(&project.id, "c", "claude", None, "HEAD", None, false, |_| {})
        .unwrap();
    assert!(!state.run_cleanup(&checkout.id).unwrap().manages_transcript);

    for id in [&known.id, &unknown.id, &checkout.id] {
        session_gone_or_cleanup(&state, id);
    }
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

/// AGE-149. After a merge the agent branch is a second name for commits that
/// are on the base, so archiving takes it, and the record is what is left to
/// read. Restoring still works: the base is where the work went, so the branch
/// is cut again from there.
#[test]
fn archiving_merged_work_takes_the_branch_and_leaves_a_record() {
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
    let info = state.create_run(&project.id, "add a feature", "noop", None, "main", None).unwrap();
    let wt = state.worktree_path(&info.id).unwrap();
    std::fs::write(wt.join("feature.txt"), "x\n").unwrap();
    Command::new("git").args(["add", "-A"]).current_dir(&wt).status().unwrap();
    Command::new("git")
        .args(["commit", "-qm", "add the feature"])
        .current_dir(&wt)
        .status()
        .unwrap();

    // Before the merge the branch is the only copy, so the plan keeps it.
    let before = state.run_cleanup(&info.id).unwrap();
    assert!(before.archive.keeps_branch, "unmerged work keeps its branch");
    assert_eq!(before.delete.commits_at_risk, 1, "deleting it now would lose the commit");
    // This run's profile is a plain `sh`, whose transcript layout Agency does
    // not know, so neither teardown can move or remove a conversation for it
    // and the dialog must not say either one does. The copy reads this flag;
    // `rescue_transcript` and `discard_run` make the same test before they
    // touch a file, which is the point of asking it here rather than in the UI.
    assert!(!before.manages_transcript);

    assert!(matches!(state.merge_task(&info.id).unwrap(), MergeOutcome::Clean { .. }));

    let after = state.run_cleanup(&info.id).unwrap();
    assert!(after.facts.merged);
    assert!(after.archive.deletes_branch, "a merged branch is a duplicate, not a backup");
    assert!(!after.delete.danger(), "nothing is at risk once the work is on main");

    state.archive_run(&info.id).unwrap();
    let branches = state.list_project_branches(&project.id).unwrap().branches;
    assert!(!branches.contains(&info.branch), "{} went with the archive", info.branch);
    assert!(!repo.join(".agency").join("worktrees").join(&info.id).exists());

    // The run is still in the archive, and what is left of it says so.
    let archived = state.list_archived_runs(&project.id).unwrap();
    assert_eq!(archived.len(), 1);
    let held = archived[0].archived.as_ref().unwrap();
    assert!(!held.branch_kept);
    assert_eq!(
        held.restore_base.as_deref(),
        Some("main"),
        "the branch is gone, but the work it carried is on main, so a restore has a start point"
    );
    assert!(held.has_record);
    // This run's agent is a plain `sh` profile, whose transcript format we do
    // not read; the viewer must get "cannot see", not an empty conversation.
    assert!(!held.has_conversation);
    let convo = state.read_run_conversation(&info.id).unwrap();
    assert!(!convo.supported);
    assert!(convo.sessions.is_empty());

    let record = state.read_run_record(&info.id).unwrap().unwrap();
    assert!(record.contains("outcome: merged"), "{record}");
    assert!(record.contains("the commits are on `main`"), "{record}");
    assert!(record.contains("add the feature"), "the commit list survives the branch: {record}");
    assert!(record.contains("1 file, +1"), "{record}");

    // And the ordinary ending restores. It used to refuse — "there is no branch
    // to restore this agent onto" — which disabled Restore on every run that
    // merged, i.e. nearly all of them. The branch is cut again from main, where
    // the work landed, so the worktree comes back with that work in it.
    let restored = state.restore_run(&info.id).unwrap();
    assert!(restored.worktree);
    assert!(restored.archived_at.is_none(), "restoring puts the run back on the board");
    assert_eq!(
        std::fs::read_to_string(wt.join("feature.txt")).unwrap(),
        "x\n",
        "the merged work is in the restored worktree, because main is where it went"
    );
    assert!(
        state.list_project_branches(&project.id).unwrap().branches.contains(&info.branch),
        "the run's branch is back, cut fresh from main"
    );
    // `<run>.md` is gone, because a record beside a running agent reads as the
    // account of a run that has finished. It is set aside rather than deleted:
    // it is the only account of the stint that just ended.
    assert!(!agency_core::record::path(&repo, &info.id).exists(), "a live run has no live record");
    let earlier = state.read_run_record(&info.id).unwrap().expect("the ended stint is kept");
    assert!(earlier.contains("## Superseded"), "{earlier}");
    assert!(earlier.contains("add the feature"), "with the commits it accounted for: {earlier}");
    session_gone_or_cleanup(&state, &info.id);
}

/// A claude session file, as the real store writes them.
fn claude_session(user: &str, assistant: &str) -> String {
    format!(
        "{}\n{}\n",
        format_args!(
            r#"{{"type":"user","timestamp":"2026-08-02T16:11:21.491Z","message":{{"role":"user","content":"{user}"}}}}"#
        ),
        format_args!(
            r#"{{"type":"assistant","timestamp":"2026-08-02T16:11:25.000Z","message":{{"id":"m1","role":"assistant","content":[{{"type":"text","text":"{assistant}"}}]}}}}"#
        ),
    )
}

fn set_mtime(path: &Path, at: std::time::SystemTime) {
    let times = std::fs::FileTimes::new().set_modified(at);
    std::fs::File::options().write(true).open(path).unwrap().set_times(times).unwrap();
}

/// The conversation's round trip: rescued into the archive when the worktree
/// goes, put back in the agent's own store when the run is restored, swept
/// when it is deleted.
///
/// AGE-152 built this and AGE-157 rests its whole answer on it — a merged
/// run's branch is cut again from the base, and the only reason the restored
/// run is worth having is that its agent picks the conversation back up — and
/// none of it was tested. Every step reads
/// `$HOME/.claude/projects/<encoded worktree>/`, so until that home became
/// injectable the only files a test could have moved were the developer's own
/// sessions.
///
/// The agent is a stub *named* `claude`: `usage::session_dir` and
/// `state::resume_command` both key off the command's basename, so a script by
/// that name gets the real claude layout without the real claude.
#[test]
fn a_merged_runs_conversation_is_rescued_reinstated_and_swept() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let home = dir.path().join("home");
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    let stub = bin.join("claude");
    std::fs::write(&stub, "#!/bin/sh\nexec sleep 30\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let state = common::state_with_agent_home(&dir, &home);
    state
        .register_profile(AgentProfile {
            name: "claude".into(),
            command: stub.to_string_lossy().into_owned(),
            args: vec![],
            env: vec![],
            resume_args: Some(vec!["--continue".into()]),
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info =
        state.create_run(&project.id, "add a feature", "claude", None, "main", None).unwrap();
    let wt = state.worktree_path(&info.id).unwrap();

    // Two conversations in the worktree's store, the way claude keeps them:
    // one file per session, named for the session id. The mtimes are set
    // apart on purpose — "the newest session" is an mtime decision, both in
    // claude's own resume and in the resume line the record writes.
    let store = home.join(".claude").join("projects").join(agency_core::usage::claude_enc(&wt));
    std::fs::create_dir_all(&store).unwrap();
    let older = "1a4970ef-de34-4b3c-a444-41e9a73722fb";
    let newest = "9674f5a1-334c-49a5-9952-89e592b0bc5b";
    let older_text = claude_session("what is here?", "A README.");
    let newest_text = claude_session("add a feature", "Done.");
    std::fs::write(store.join(format!("{older}.jsonl")), &older_text).unwrap();
    std::fs::write(store.join(format!("{newest}.jsonl")), &newest_text).unwrap();
    let then = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_756_000_000);
    let later = then + std::time::Duration::from_secs(600);
    set_mtime(&store.join(format!("{older}.jsonl")), then);
    set_mtime(&store.join(format!("{newest}.jsonl")), later);

    // Merge it, which is the ending that deletes the branch and so the one
    // AGE-157 is about.
    std::fs::write(wt.join("feature.txt"), "x\n").unwrap();
    Command::new("git").args(["add", "-A"]).current_dir(&wt).status().unwrap();
    Command::new("git")
        .args(["commit", "-qm", "add the feature"])
        .current_dir(&wt)
        .status()
        .unwrap();
    assert!(matches!(state.merge_task(&info.id).unwrap(), MergeOutcome::Clean { .. }));

    state.archive_run(&info.id).unwrap();

    // The store is keyed by the worktree path, which has just stopped
    // existing, so the archive takes the whole directory rather than leaving
    // it to leak.
    let rescued = repo.join(".agency").join("records").join(format!("{}.transcript", info.id));
    assert!(!store.exists(), "the agent's own directory for a path that is gone goes with it");
    assert_eq!(
        std::fs::read_to_string(rescued.join(format!("{newest}.jsonl"))).unwrap(),
        newest_text,
        "the conversation is in the archive, byte for byte"
    );
    assert!(rescued.join(format!("{older}.jsonl")).exists(), "both sessions, not just the last");

    let archived = state.list_archived_runs(&project.id).unwrap();
    assert!(archived[0].archived.as_ref().unwrap().has_conversation);
    let record = state.read_run_record(&info.id).unwrap().unwrap();
    assert!(record.contains("2 session files"), "{record}");
    assert!(
        record.contains(&format!("claude --resume {newest}")),
        "the resume line names the newest session by mtime, not the first one read: {record}"
    );
    // And it can be read without restoring anything, which is the whole point
    // of rescuing it rather than naming where it used to be.
    let convo = state.read_run_conversation(&info.id).unwrap();
    assert!(convo.supported);
    assert_eq!(convo.sessions.len(), 2);

    state.restore_run(&info.id).unwrap();

    // The worktree comes back at the path it had, which is why the store
    // directory's name still fits it — and why AGE-157 needed no per-agent
    // proof that a session file's own `cwd` may name somewhere else.
    assert_eq!(state.worktree_path(&info.id).unwrap(), wt);
    assert_eq!(
        std::fs::read_to_string(store.join(format!("{newest}.jsonl"))).unwrap(),
        newest_text,
        "the conversation is back in the agent's own store, so its resume finds it"
    );
    assert_eq!(
        std::fs::metadata(store.join(format!("{newest}.jsonl"))).unwrap().modified().unwrap(),
        later,
        "with its mtime, which is what \"resume the most recent session\" goes by"
    );
    assert!(!rescued.exists(), "moved back, not copied: one archive, one store, never both");

    // Delete sweeps what restore reinstated. Before AGE-152 this directory was
    // keyed to a path that no longer existed and nothing ever removed it.
    state.discard_run(&info.id).unwrap();
    assert!(!store.exists(), "the conversation goes with the run the user deleted");
}

/// A run archived twice keeps both accounts.
///
/// Restoring used to delete the record outright, so a run that was archived,
/// restored and archived again remembered only the second stint: the first
/// one's commits, outcome and cost went with the file. The conversation
/// survived the same cycle, which made the loss easy to miss and strange when
/// found. Now the record is set aside as `<run>.1.md` and the two point at
/// each other.
#[test]
fn a_run_archived_twice_keeps_the_account_of_both_stints() {
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
    let info = state.create_run(&project.id, "add a feature", "noop", None, "main", None).unwrap();

    let commit = |wt: &Path, file: &str, subject: &str| {
        std::fs::write(wt.join(file), "x\n").unwrap();
        Command::new("git").args(["add", "-A"]).current_dir(wt).status().unwrap();
        Command::new("git").args(["commit", "-qm", subject]).current_dir(wt).status().unwrap();
    };

    // First stint: one commit, merged, archived.
    let wt = state.worktree_path(&info.id).unwrap();
    commit(&wt, "first.txt", "the first thing");
    assert!(matches!(state.merge_task(&info.id).unwrap(), MergeOutcome::Clean { .. }));
    state.archive_run(&info.id).unwrap();
    let first = state.read_run_record(&info.id).unwrap().unwrap();
    assert!(first.contains("the first thing"), "{first}");
    assert!(!first.contains("## Earlier"), "nothing came before it: {first}");

    state.restore_run(&info.id).unwrap();

    // Second stint: another commit, merged, archived again.
    commit(&wt, "second.txt", "the second thing");
    assert!(matches!(state.merge_task(&info.id).unwrap(), MergeOutcome::Clean { .. }));
    state.archive_run(&info.id).unwrap();

    let records = repo.join(".agency").join("records");
    let retired = std::fs::read_to_string(records.join(format!("{}.1.md", info.id))).unwrap();
    let current = std::fs::read_to_string(records.join(format!("{}.md", info.id))).unwrap();
    assert!(retired.contains("the first thing"), "the first stint is still accounted for");
    assert!(!retired.contains("the second thing"), "and says only what it knew: {retired}");
    assert!(retired.contains("## Superseded"), "and says why it is not the current one");
    assert!(current.contains("the second thing"), "{current}");
    assert!(
        current.contains(&format!("`{}.1.md`", info.id)),
        "the current record names the one it was restored out of: {current}"
    );

    // The dialog has no way to open a sibling file, so the read hands back
    // both, newest first.
    let shown = state.read_run_record(&info.id).unwrap().unwrap();
    let second = shown.find("the second thing").expect("the current stint");
    let earlier = shown.find("the first thing").expect("and the one before it");
    assert!(second < earlier, "newest first: {shown}");

    // Deleting the run takes every stint's record, not just the last.
    state.discard_run(&info.id).unwrap();
    assert!(!records.join(format!("{}.md", info.id)).exists());
    assert!(!records.join(format!("{}.1.md", info.id)).exists(), "no orphan no run names");
}

/// The other half of AGE-149: work that landed nowhere keeps its branch, and
/// the record says what that branch is holding.
#[test]
fn archiving_unmerged_work_keeps_the_branch_and_stays_restorable() {
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
    let info = state.create_run(&project.id, "explore", "noop", None, "main", None).unwrap();
    let wt = state.worktree_path(&info.id).unwrap();
    std::fs::write(wt.join("draft.txt"), "x\n").unwrap();
    Command::new("git").args(["add", "-A"]).current_dir(&wt).status().unwrap();
    Command::new("git").args(["commit", "-qm", "a draft"]).current_dir(&wt).status().unwrap();

    state.archive_run(&info.id).unwrap();
    let branches = state.list_project_branches(&project.id).unwrap().branches;
    assert!(branches.contains(&info.branch), "the only copy of the work stays");

    let archived = state.list_archived_runs(&project.id).unwrap();
    let held = archived[0].archived.as_ref().unwrap();
    assert!(held.branch_kept && held.has_record);

    let record = state.read_run_record(&info.id).unwrap().unwrap();
    assert!(record.contains("outcome: kept"), "{record}");
    assert!(record.contains("carrying 1 commit that is nowhere else"), "{record}");

    // Restoring cuts the worktree again and sets the record aside: it
    // described a finished run, and this one is going again.
    let restored = state.restore_run(&info.id).unwrap();
    assert!(restored.worktree);
    assert!(!agency_core::record::path(&repo, &info.id).exists());
    assert!(state.read_run_record(&info.id).unwrap().unwrap().contains("carrying 1 commit"));
    session_gone_or_cleanup(&state, &info.id);
}

/// Deleting a run takes its record with it: a file left behind for a run the
/// user asked to be rid of is the opposite of what they pressed.
#[test]
fn deleting_an_archived_run_removes_its_record() {
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
    let info = state.create_run(&project.id, "p", "noop", None, "main", None).unwrap();
    state.archive_run(&info.id).unwrap();
    let path = repo.join(".agency").join("records").join(format!("{}.md", info.id));
    assert!(path.exists());

    state.discard_run(&info.id).unwrap();
    assert!(!path.exists(), "the record went with the run");
}

/// Anything Agency writes into a project must be git-excluded, or it turns up
/// in every diff the user takes afterwards.
#[test]
fn the_records_directory_is_excluded_from_git() {
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
    let info = state.create_run(&project.id, "p", "noop", None, "main", None).unwrap();
    state.archive_run(&info.id).unwrap();

    let out = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(&repo)
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&out.stdout);
    assert!(!status.contains(".agency/records"), "records must not show in git status: {status}");
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
            "Saving the conversation",
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
/// landed, so the feature looked broken. It builds on request now, and the
/// settings UI can watch that build finish.
///
/// Enabling on its own must NOT build. A build reads the whole project with an
/// LLM, against a plan or an API key, and flicking a toggle is not consent to
/// spend either: the panel shows what the chosen model costs, and the Build
/// button is where the user agrees to it.
#[test]
fn the_knowledge_graph_builds_on_request_and_not_before() {
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
    // configured build in the primary checkout, and only when asked.
    let build = "mkdir -p graphify-out && printf '{}' > graphify-out/graph.json";
    state.save_knowledge_config(&p.id, true, true, None, Some(build.to_string())).unwrap();

    let enabled = state.knowledge_config(&p.id).unwrap();
    assert!(!enabled.building, "enabling must not start a build");
    assert!(!enabled.graph_built, "enabling must not write a graph");

    state.build_knowledge_graph(&p.id).unwrap();
    let cfg = await_settled_build(&state, &p.id);
    assert_eq!(cfg.last_build_error, None, "the build should have succeeded");
    assert!(cfg.graph_built, "the build must produce {}", cfg.graph_path);
    assert!(repo.join("graphify-out/graph.json").is_file());

    // AGE-170: the graph is Agency's artifact, so git must not offer it. The
    // stand-in build above writes only graph.json; a real one also leaves the
    // cache/ that turned up in the user's Untracked Changes, so put that file
    // there too and let git rule on the directory.
    std::fs::create_dir_all(repo.join("graphify-out/cache")).unwrap();
    std::fs::write(repo.join("graphify-out/cache/stat-index.json"), "{}").unwrap();
    let out =
        Command::new("git").args(["status", "--porcelain"]).current_dir(&repo).output().unwrap();
    let status = String::from_utf8_lossy(&out.stdout);
    assert!(!status.contains("graphify-out"), "the build left git dirty: {status}");
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
    state.save_knowledge_config(&p.id, true, true, None, Some(build.to_string())).unwrap();
    state.build_knowledge_graph(&p.id).unwrap();

    let cfg = await_settled_build(&state, &p.id);
    assert!(!cfg.graph_built);
    assert_eq!(cfg.last_build_error.as_deref(), Some("graphify: no parser for this repo"));
    assert!(!cfg.last_build_stopped, "it failed on its own");
    assert_eq!(cfg.build_log, ["graphify: no parser for this repo"], "and keeps the whole tail");
    assert!(cfg.last_build_secs.is_some());

    // A build whose command isn't installed at all is refused up front, and
    // pointed at the install the settings panel offers to run.
    state
        .save_knowledge_config(
            &p.id,
            true,
            true,
            None,
            Some("definitely-not-a-real-binary-4k2x .".into()),
        )
        .unwrap();
    let err = state.build_knowledge_graph(&p.id).unwrap_err().to_string();
    assert!(err.contains("'definitely-not-a-real-binary-4k2x' is not installed"), "{err}");
    assert!(err.contains("Install the graphify tooling"), "{err}");
}

/// AGE-180: a build that runs for ten minutes showed "Building the graph" and
/// nothing else, so a working build and a wedged one looked identical and
/// neither could be got out of. The panel gets the build's own output and the
/// elapsed time while it runs, and a Stop that reaches the whole process group
/// (graphify spawns an agent CLI per document; signalling only the process
/// Agency started leaves those spending).
#[test]
fn a_running_build_shows_its_output_and_can_be_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let p = state.add_project("demo", &repo).unwrap();

    // Prints like a real build does, then goes quiet for longer than the test
    // will wait. Two commands, so `sh` does not exec into the sleep and the
    // stop has a process group to reach rather than one child.
    let build = "printf 'AST extraction: 1/2 uncached files\\n'; sleep 120";
    state.save_knowledge_config(&p.id, true, true, None, Some(build.to_string())).unwrap();
    state.build_knowledge_graph(&p.id).unwrap();

    let mut cfg = state.knowledge_config(&p.id).unwrap();
    for _ in 0..200 {
        if cfg.build_log.iter().any(|l| l.contains("AST extraction")) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        cfg = state.knowledge_config(&p.id).unwrap();
    }
    assert!(cfg.building, "the build is still running");
    assert!(cfg.build_elapsed_secs.is_some(), "a running build says how long it has been going");
    assert!(
        cfg.build_log.iter().any(|l| l == "AST extraction: 1/2 uncached files"),
        "the build's own output is what says what it is doing: {:?}",
        cfg.build_log
    );
    assert!(state.build_knowledge_graph(&p.id).is_err(), "one build at a time");

    state.stop_knowledge_build(&p.id).unwrap();
    let cfg = await_settled_build(&state, &p.id);
    assert!(cfg.last_build_stopped, "the panel has to say the build was stopped");
    assert_eq!(cfg.last_build_error, None, "a build the user stopped is not one that failed");
    assert!(cfg.last_build_secs.is_some(), "a finished build says how long it took");
    assert!(!cfg.build_log.is_empty(), "the output stays readable after the build ends");
    assert!(state.stop_knowledge_build(&p.id).is_err(), "nothing to stop is an error, not a no-op");
}

/// Picking a model writes the build command and starts nothing. The whole
/// choice lives in that one string, so the panel reads it back out rather than
/// keeping a second copy, and a command it can't have written is reported as
/// custom instead of being silently rewritten.
#[test]
fn choosing_a_model_writes_the_build_command_and_runs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let p = state.add_project("demo", &repo).unwrap();

    state.save_knowledge_config(&p.id, true, true, None, None).unwrap();
    state.set_knowledge_backend(&p.id, "code-only", "").unwrap();

    let cfg = state.knowledge_config(&p.id).unwrap();
    assert_eq!(cfg.build_command.as_deref(), Some("graphify . --code-only"));
    assert_eq!(cfg.build_backend, "code-only");
    assert!(cfg.graph, "picking a model must not disturb the toggle");
    assert!(!cfg.building && !cfg.graph_built, "picking a model must not build");
    // Whatever else this machine has, the build that needs no LLM is offered.
    assert!(cfg.backends.iter().any(|b| b.id == "code-only"), "{:?}", cfg.backends);

    state
        .save_knowledge_config(&p.id, true, true, None, Some("graphify . --mode deep".into()))
        .unwrap();
    assert_eq!(state.knowledge_config(&p.id).unwrap().build_backend, "custom");
}

/// A build command saved before the model was part of the choice runs `claude
/// -p` per file on whatever the CLI defaults to, which spent a 5-hour usage
/// window in about 30 minutes. Reading the config repairs it in place, so the
/// panel and the post-merge rebuild both see a command that names its model.
#[test]
fn a_saved_command_that_names_no_model_is_repaired_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let p = state.add_project("demo", &repo).unwrap();

    state
        .save_knowledge_config(
            &p.id,
            true,
            true,
            None,
            Some("graphify . --backend claude-cli".into()),
        )
        .unwrap();
    let cfg = state.knowledge_config(&p.id).unwrap();

    // Machine-dependent by nature: claude-cli is only offered where the CLI is
    // installed, and only an offered backend has a default model to name. Both
    // halves are asserted so neither machine passes by accident.
    if cfg.backends.iter().any(|b| b.id == "claude-cli") {
        assert_eq!(cfg.build_backend, "claude-cli");
        assert!(!cfg.build_model.is_empty(), "{:?}", cfg.build_command);
        let saved = cfg.build_command.as_deref().unwrap_or_default();
        assert!(saved.contains("GRAPHIFY_CLAUDE_CLI_MODEL="), "repaired in place: {saved}");
        // Idempotent: a second read leaves the repaired command alone.
        assert_eq!(state.knowledge_config(&p.id).unwrap().build_command.as_deref(), Some(saved));
    } else {
        assert_eq!(cfg.build_backend, "claude-cli");
        assert_eq!(cfg.build_command.as_deref(), Some("graphify . --backend claude-cli"));
    }

    // A hand-written command is never rewritten, whatever the machine has.
    state
        .save_knowledge_config(&p.id, true, true, None, Some("graphify . --mode deep".into()))
        .unwrap();
    let cfg = state.knowledge_config(&p.id).unwrap();
    assert_eq!(cfg.build_backend, "custom");
    assert_eq!(cfg.build_command.as_deref(), Some("graphify . --mode deep"));
}

/// The rebuild that runs after every clean merge is the only build nobody
/// presses, so it is a switch the user can see and turn off, and it is on for
/// every project configured before the switch existed.
#[test]
fn the_post_merge_rebuild_is_a_switch_that_defaults_to_on() {
    let dir = tempfile::tempdir().unwrap();
    let state = common::state(&dir);
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let p = state.add_project("demo", &repo).unwrap();

    assert!(state.knowledge_config(&p.id).unwrap().rebuild_on_merge);
    // A config written before the field existed reads as on, not off.
    std::fs::create_dir_all(repo.join(".agency")).unwrap();
    std::fs::write(repo.join(".agency/agency.local.toml"), "[knowledge]\ngraph = true\n").unwrap();
    assert!(state.knowledge_config(&p.id).unwrap().rebuild_on_merge);

    state.save_knowledge_config(&p.id, true, false, None, None).unwrap();
    let cfg = state.knowledge_config(&p.id).unwrap();
    assert!(!cfg.rebuild_on_merge);
    assert!(cfg.graph, "turning the rebuild off must not disturb the feature toggle");
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

/// AGE-148. The name a run's branch is born with comes from its prompt, and a
/// merge writes it into the base branch's history for good, so it has to be
/// editable while the branch is still local — in git *and* in the registry,
/// since the merge reads the registry.
#[test]
fn renaming_a_runs_branch_moves_git_and_the_registry_together() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    // Named "claude" so the workspace skill is emitted: it states the branch
    // the run owns, and a rename that left it stale would have the agent name
    // a branch that no longer exists.
    state
        .register_profile(AgentProfile {
            name: "claude".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "sleep 3".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "claude", None, "HEAD", None).unwrap();
    let old = info.branch.clone();
    let wt = state.worktree_path(&info.id).unwrap();

    // Typed without the prefix: it comes back with one either way.
    let applied = state.rename_run_branch(&info.id, "tidy-name").unwrap();
    assert_eq!(applied, "agent/tidy-name");

    assert!(!agency_core::merge::branch_exists(&repo, &old));
    assert!(agency_core::merge::branch_exists(&repo, "agent/tidy-name"));
    // The registry is what merge reads.
    let listed = state.list_runs(&project.id).unwrap();
    assert_eq!(listed[0].branch, "agent/tidy-name");
    // The worktree neither moved nor came off its branch.
    assert_eq!(state.worktree_path(&info.id).unwrap(), wt);
    assert_eq!(agency_core::merge::current_branch(&wt).as_deref(), Some("agent/tidy-name"));
    // The workspace skill states the branch the run owns; a stale copy would
    // have the agent name a branch that is gone.
    let skill = wt.join(".claude/skills/agency-workspace/SKILL.md");
    let text = std::fs::read_to_string(&skill).unwrap();
    assert!(text.contains("agent/tidy-name"), "the workspace skill still names the old branch");
    assert!(!text.contains(&old), "the workspace skill still names the old branch");

    let _ = state.discard_run(&info.id);
}

#[test]
fn renaming_a_branch_refuses_names_git_or_the_project_will_not_take() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "noop".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "sleep 3".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "noop", None, "HEAD", None).unwrap();
    let old = info.branch.clone();

    let bad = state.rename_run_branch(&info.id, "has a space").unwrap_err().to_string();
    assert!(bad.contains("space"), "unexpected error: {bad}");

    assert!(Command::new("git")
        .args(["branch", "agent/taken"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let clash = state.rename_run_branch(&info.id, "agent/taken").unwrap_err().to_string();
    assert!(clash.contains("already has a branch"), "unexpected error: {clash}");

    // Every refusal left both halves as they were.
    assert!(agency_core::merge::branch_exists(&repo, &old));
    assert_eq!(state.list_runs(&project.id).unwrap()[0].branch, old);

    let _ = state.discard_run(&info.id);
}

/// A pushed name is out of Agency's hands: the remote keeps the old branch and
/// a PR opened from it goes on pointing at that name, so a local rename would
/// only make the two disagree.
#[test]
fn renaming_a_branch_refuses_once_the_name_is_published() {
    let dir = tempfile::tempdir().unwrap();
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

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "noop".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "sleep 3".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "noop", None, "HEAD", None).unwrap();
    assert!(Command::new("git")
        .args(["push", "-q", "origin", &info.branch])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());

    let err = state.rename_run_branch(&info.id, "too-late").unwrap_err().to_string();
    assert!(err.contains("already published"), "unexpected error: {err}");
    assert_eq!(state.list_runs(&project.id).unwrap()[0].branch, info.branch);

    let _ = state.discard_run(&info.id);
}

/// A run working in the project's own checkout is sitting on the user's branch,
/// shared with everything else they do there. Not Agency's to rename.
#[test]
fn renaming_a_branch_refuses_a_run_in_the_project_checkout() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "noop".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "sleep 3".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state
        .create_run_with_progress(&project.id, "p", "noop", None, "HEAD", None, false, |_| {})
        .unwrap();

    let err = state.rename_run_branch(&info.id, "agent/nope").unwrap_err().to_string();
    assert!(err.contains("project checkout"), "unexpected error: {err}");
    assert_eq!(agency_core::merge::current_branch(&repo).as_deref(), Some("main"));

    let _ = state.discard_run(&info.id);
}

/// AGE-183. A promptless start cuts `agent/agent-<suffix>` before anyone types.
/// The first prompt already titled the run; the branch stayed on the fallback
/// until someone renamed it by hand. Applying the first prompt now renames
/// both, keeping the same short suffix so the worktree and session id stay put.
#[test]
fn first_prompt_renames_the_empty_prompt_fallback_branch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "claude".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "sleep 3".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "", "claude", None, "HEAD", None).unwrap();
    let old = info.branch.clone();
    assert!(
        old.starts_with("agent/agent-"),
        "promptless start should cut the empty-prompt fallback, got {old}"
    );
    assert!(info.title.as_deref().unwrap_or("").is_empty(), "promptless start has no title yet");
    let suffix = info.id.rsplit_once('-').unwrap().1;
    let wt = state.worktree_path(&info.id).unwrap();

    state
        .apply_first_prompt(&info.id, "interesting/memorable names? I'm thinking about like")
        .unwrap();

    let listed = state.list_runs(&project.id).unwrap();
    let run = &listed[0];
    let title = run.title.as_deref().unwrap_or("");
    assert!(!title.is_empty(), "first prompt should title the run");
    assert!(title.to_lowercase().contains("interesting"), "unexpected title: {title}");

    assert!(
        run.branch.starts_with("agent/interesting-memorable-names"),
        "branch should follow the first prompt, got {}",
        run.branch
    );
    assert!(
        run.branch.ends_with(&format!("-{suffix}")),
        "branch should keep the run id's suffix, got {}",
        run.branch
    );
    assert_ne!(run.branch, old);
    assert!(!agency_core::merge::branch_exists(&repo, &old));
    assert!(agency_core::merge::branch_exists(&repo, &run.branch));
    assert_eq!(state.worktree_path(&info.id).unwrap(), wt);
    assert_eq!(agency_core::merge::current_branch(&wt).as_deref(), Some(run.branch.as_str()));

    // A second capture must not rename again (title already set).
    let after_first = run.branch.clone();
    state.apply_first_prompt(&info.id, "a completely different second line").unwrap();
    assert_eq!(state.list_runs(&project.id).unwrap()[0].branch, after_first);
    assert_eq!(state.list_runs(&project.id).unwrap()[0].title.as_deref(), Some(title));

    let _ = state.discard_run(&info.id);
}

/// A branch the user already renamed is theirs: the first prompt still titles
/// the run, but must not move the branch out from under that choice.
#[test]
fn first_prompt_leaves_a_manually_renamed_branch_alone() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "noop".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "sleep 3".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "", "noop", None, "HEAD", None).unwrap();
    state.rename_run_branch(&info.id, "already-chosen").unwrap();

    state.apply_first_prompt(&info.id, "fix the login page").unwrap();

    let run = &state.list_runs(&project.id).unwrap()[0];
    assert_eq!(run.branch, "agent/already-chosen");
    assert!(run.title.as_deref().unwrap_or("").contains("fix"));

    let _ = state.discard_run(&info.id);
}

/// A run created with a real prompt already has a branch named after it. The
/// title normally guards that, but `rename_run` stores a trimmed title and an
/// empty one is the documented way to clear it, which puts a prompt-derived
/// branch in front of the first-prompt pass with the title guard down.
///
/// Observed on a probe of exactly this path before `is_auto_cut_branch` also
/// checked the id shape: clearing the title and typing one line renamed
/// `agent/narrate-the-weekly-review-note-docs-week-q3w7` to
/// `agent/continue-please-q3w7`. The title is the first prompt's to set; a
/// branch that was never the empty-prompt fallback is not.
#[test]
fn first_prompt_spares_a_prompt_derived_branch_after_the_title_is_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "noop".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "sleep 3".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state
        .create_run(
            &project.id,
            "Narrate the weekly review note docs/week.md",
            "noop",
            None,
            "HEAD",
            None,
        )
        .unwrap();
    let named = info.branch.clone();
    assert!(named.starts_with("agent/narrate-the-weekly-review"), "unexpected branch: {named}");

    // What `rename_run` does with an empty title.
    state.store_run_title(&info.id, "").unwrap();

    state.apply_first_prompt(&info.id, "continue please").unwrap();

    let run = &state.list_runs(&project.id).unwrap()[0];
    assert_eq!(run.branch, named, "a prompt-derived branch is not the first prompt's to rename");
    assert!(agency_core::merge::branch_exists(&repo, &named));
    // The title still refills, which is the whole point of clearing it.
    assert!(run.title.as_deref().unwrap_or("").to_lowercase().contains("continue"));

    let _ = state.discard_run(&info.id);
}

/// AGE-183: a blocked rename must not undo the title. Pre-cut the leaf the
/// first prompt would want so `rename_run_branch` refuses (name taken); the
/// title still lands and the fallback branch stays put.
#[test]
fn first_prompt_keeps_the_title_when_the_branch_rename_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = common::state(&dir);
    state
        .register_profile(AgentProfile {
            name: "noop".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "sleep 3".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "", "noop", None, "HEAD", None).unwrap();
    let old = info.branch.clone();
    let suffix = info.id.rsplit_once('-').unwrap().1;
    let taken = format!("agent/fix-the-login-page-{suffix}");
    assert!(Command::new("git")
        .args(["branch", &taken])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());

    state.apply_first_prompt(&info.id, "fix the login page").unwrap();

    let run = &state.list_runs(&project.id).unwrap()[0];
    assert!(
        run.title.as_deref().unwrap_or("").contains("fix"),
        "title must land even when rename is refused"
    );
    assert_eq!(run.branch, old, "refused rename must leave the fallback branch alone");
    assert!(agency_core::merge::branch_exists(&repo, &old));
    assert!(agency_core::merge::branch_exists(&repo, &taken));

    let _ = state.discard_run(&info.id);
}

// ── what the board reads off a run ──────────────────────────────────────────

/// The derived state has to survive the trip the board actually takes:
/// `update_activity` writes the pane observation, `list_runs` classifies it,
/// and the tile reads the answer off `RunInfo`. Unit tests cover
/// `activity::classify` itself; this covers the wiring around it, including
/// the `prompted` set that separates "waiting on you" from plain idle.
#[test]
fn a_quiet_user_driven_run_reads_as_waiting_then_decays_to_idle() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();
    let id = live_terminal(&state, &project.id);

    let activity = |s: &AppState| {
        s.list_runs(&project.id).unwrap().into_iter().find(|r| r.id == id).unwrap().activity
    };

    // Nothing has observed the pane yet, so there is no sample to classify.
    assert!(activity(&state).is_none(), "unobserved: no activity yet");

    // Quiet past the working TTL, but nobody drove a turn: idle, not waiting.
    mark_quiet_since(&state, &id, now_ms() - 60_000);
    assert_eq!(activity(&state).unwrap().state, ActivityState::Idle, "never prompted");

    // A turn the user drove. The same quiet pane now means the run finished or
    // is blocked on input, which is the state the board badges.
    state.run_input(&id, b"\r").unwrap();
    mark_quiet_since(&state, &id, now_ms() - 60_000);
    assert_eq!(activity(&state).unwrap().state, ActivityState::Waiting, "waiting on the user");

    // Past the decay window an urgent badge stops being signal, so it lapses
    // back to idle even though the turn was driven.
    mark_quiet_since(&state, &id, now_ms() - 90 * 60 * 1000);
    assert_eq!(activity(&state).unwrap().state, ActivityState::Idle, "decayed");

    state.discard_run(&id).unwrap();
}

/// Pinning is about placement. Order is the order they were pinned in;
/// unpinning and pinning again moves a run to the end.
#[test]
fn pins_number_from_one_and_reorder_by_unpin_pin() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();
    let first = live_terminal(&state, &project.id);
    let second = live_terminal(&state, &project.id);

    let rank = |state: &AppState, id: &str| {
        state.list_runs(&project.id).unwrap().into_iter().find(|r| r.id == id).unwrap().pin_rank
    };

    state.pin_run(&first, true).unwrap();
    state.pin_run(&second, true).unwrap();
    assert_eq!(rank(&state, &first), Some(1.0));
    assert_eq!(rank(&state, &second), Some(2.0), "pinned second, ordered second");

    // Unpinning and pinning again is how a run is moved to the end.
    state.pin_run(&first, false).unwrap();
    assert_eq!(rank(&state, &first), None);
    state.pin_run(&first, true).unwrap();
    assert_eq!(rank(&state, &first), Some(3.0));

    state.discard_run(&first).unwrap();
    state.discard_run(&second).unwrap();
}
