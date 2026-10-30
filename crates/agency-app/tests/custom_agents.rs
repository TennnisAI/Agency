//! A custom agent through the real spawn path: the definition written by the
//! same call the Settings form makes, a run created against it, and what the
//! harness was actually handed, read back out of a stub standing in for its CLI.

mod common;

use agency_app_lib::AppState;
use agency_core::profile::AgentProfile;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let run = |args: &[&str]| {
        assert!(Command::new("git").args(args).current_dir(dir).status().unwrap().success());
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "t@e.com"]);
    run(&["config", "user.name", "T"]);
    std::fs::write(dir.join("README.md"), "hi").unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "init"]);
}

/// A profile named for `harness`, which is what delivery is keyed on, whose
/// command prints the arguments it was handed.
fn stub(state: &AppState, harness: &str) {
    state
        .register_profile(AgentProfile {
            name: harness.into(),
            command: "sh".into(),
            args: vec!["-c".into(), "printf 'ARGV:%s\\n' \"$*\"; sleep 5".into(), "stub".into()],
            env: vec![],
            resume_args: None,
            loop_args: None,
        })
        .unwrap();
}

fn argv_of(state: &AppState, id: &str) -> String {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(out) = state.run_preview(id, 40) {
            if out.contains("ARGV:") {
                return out.replace('\n', "");
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the stub never printed its arguments");
}

fn worktree_of(repo: &Path, id: &str) -> std::path::PathBuf {
    repo.join(".agency/worktrees").join(id)
}

#[test]
fn a_run_starts_as_the_agent_it_was_given_and_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let state = common::state(&dir);
    stub(&state, "claude");
    let project = state.add_project("demo", &repo).unwrap();

    let saved = state
        .save_custom_agent(
            "project",
            Some(&project.id),
            None,
            "Code Reviewer",
            "Reviews: a diff",
            "Lead with what is wrong.",
        )
        .unwrap();
    assert_eq!(saved.id, "code-reviewer");
    // In the project, where it can be committed and shared.
    assert!(repo.join(".agency/agents/code-reviewer.md").is_file());

    let info = state
        .create_run_as(
            &project.id,
            "review it",
            "claude",
            None,
            Some("code-reviewer"),
            "HEAD",
            None,
            true,
            |_| {},
        )
        .unwrap();
    assert_eq!(info.custom_agent.as_deref(), Some("Code Reviewer"));

    let argv = argv_of(&state, &info.id);
    assert!(argv.contains("--agent agency-code-reviewer"), "{argv}");

    let worktree = worktree_of(&repo, &info.id);
    let written =
        std::fs::read_to_string(worktree.join(".claude/agents/agency-code-reviewer.md")).unwrap();
    assert!(written.contains("Lead with what is wrong."), "{written}");

    // Not offered to git, or it lands in the project at merge.
    let status = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(&worktree)
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&status.stdout);
    assert!(!status.contains("agency-code-reviewer"), "{status}");

    // The file going away does not change what the run is: it kept a copy, which
    // is what a resume, a rerun and a loop's next attempt launch from.
    state.delete_custom_agent("project", Some(&project.id), "code-reviewer").unwrap();
    assert!(!repo.join(".agency/agents/code-reviewer.md").exists());
    let listed = state.list_runs(&project.id).unwrap();
    let again = listed.iter().find(|r| r.id == info.id).unwrap();
    assert_eq!(again.custom_agent.as_deref(), Some("Code Reviewer"));

    let _ = state.discard_run(&info.id);
}

#[test]
fn a_project_agent_shadows_a_user_one_of_the_same_id() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let state = common::state(&dir);
    let project = state.add_project("demo", &repo).unwrap();

    state.save_custom_agent("user", None, None, "Reviewer", "", "From the user").unwrap();
    // No project asked about, so only the user's.
    assert_eq!(state.list_custom_agents(None).unwrap().agents.len(), 1);
    state
        .save_custom_agent("project", Some(&project.id), None, "Reviewer", "", "From the project")
        .unwrap();

    let listed = state.list_custom_agents(Some(&project.id)).unwrap();
    assert_eq!(listed.agents.len(), 1);
    assert_eq!(listed.agents[0].scope, "project");
    assert_eq!(listed.agents[0].instructions, "From the project");
    assert!(listed.harnesses.iter().any(|h| h == "claude"));

    // Creating over an existing one is refused; editing it by id is not.
    let dup = state.save_custom_agent("user", None, None, "Reviewer", "", "Again");
    assert!(dup.unwrap_err().to_string().contains("already"));
    state.save_custom_agent("user", None, Some("reviewer"), "Renamed", "", "Again").unwrap();
    let user = state.list_custom_agents(None).unwrap();
    assert_eq!(user.agents[0].id, "reviewer");
    assert_eq!(user.agents[0].name, "Renamed");
}

/// Each of these would otherwise start a plain agent that looks exactly like
/// the one asked for, and none may leave a worktree behind.
#[test]
fn a_spawn_that_cannot_honour_the_agent_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let state = common::state(&dir);
    stub(&state, "claude");
    stub(&state, "crush");
    stub(&state, "cursor");
    let project = state.add_project("demo", &repo).unwrap();
    state.save_custom_agent("user", None, None, "Reviewer", "", "Instructions").unwrap();

    let start = |agent: &str, custom: &str, worktree: bool| {
        state.create_run_as(
            &project.id,
            "go",
            agent,
            None,
            Some(custom),
            "HEAD",
            None,
            worktree,
            |_| {},
        )
    };

    let e = start("claude", "gone", true).unwrap_err().to_string();
    assert!(e.contains("no custom agent"), "{e}");
    let e = start("claude", "../../etc/passwd", true).unwrap_err().to_string();
    assert!(e.contains("not a valid agent id"), "{e}");
    let e = start("crush", "reviewer", true).unwrap_err().to_string();
    assert!(e.contains("no way to be given a custom agent"), "{e}");
    // A rules file in the checkout would apply to everything else running there.
    let e = start("cursor", "reviewer", false).unwrap_err().to_string();
    assert!(e.contains("own worktree"), "{e}");

    assert!(state.list_runs(&project.id).unwrap().is_empty());
    let worktrees = repo.join(".agency/worktrees");
    let left = std::fs::read_dir(&worktrees).map(|d| d.count()).unwrap_or(0);
    assert_eq!(left, 0, "a refused spawn left a worktree behind");

    // In its own worktree cursor gets the always-on rule, and no flag.
    let info = start("cursor", "reviewer", true).unwrap();
    let rule = worktree_of(&repo, &info.id).join(".cursor/rules/agency-custom-agent.mdc");
    let rule = std::fs::read_to_string(rule).unwrap();
    assert!(rule.contains("alwaysApply: true") && rule.contains("Instructions"), "{rule}");
    assert!(!argv_of(&state, &info.id).contains("--agent"));
    let _ = state.discard_run(&info.id);
}
