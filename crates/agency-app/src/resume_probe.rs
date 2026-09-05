//! Proactive per-agent check: does a resumable session exist for this worktree?
//! Used so we never launch a resume command (e.g. `claude --continue`) when there
//! is nothing to resume — claude does not exit on resume-failure in a PTY, so the
//! daemon's early-exit fallback cannot recover it.
use agency_core::usage::{claude_enc, pi_enc};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeProbe {
    /// A per-worktree session store exists and is non-empty.
    Has,
    /// We can check this agent and there is no session.
    None,
    /// No probe for this agent — caller should attempt resume (the daemon
    /// early-exit fallback covers agents that exit fast on resume-failure).
    Unknown,
}

/// `home` is the user's home dir (injected for testability). `command` is the
/// agent launch command (e.g. "claude", "pi"); only the basename is matched.
/// `session` is the Agency session about to be launched, and `conversation`
/// the one it has on record, if any.
///
/// The probe has to ask about the same conversation the launch will ask for,
/// or it answers a different question than the one that decides
/// resume-vs-fresh:
///
/// - an agent with a store of its own (pi) resumes only what is in that store,
///   so a sibling session's conversation in the shared directory is not
///   something it can continue, and reading it as "Has" would resume an empty
///   store with the run's prompt nowhere;
/// - an agent that names conversations (claude, copilot) is about to ask for
///   that one conversation, so the question is whether that one transcript
///   exists and nothing else. A sibling's conversation next to it is no
///   answer, and for claude a wrong "Has" does not even start:
///   `--resume <id>` refuses outright when the id is not there;
/// - a run with no conversation on record — every run that predates Agency
///   minting them — falls back to the directory question it always asked.
pub fn resume_probe(
    home: &Path,
    command: &str,
    worktree: &Path,
    session: &str,
    conversation: Option<&str>,
) -> ResumeProbe {
    if let Some(dir) = agency_core::sessionstore::dir(home, command, worktree, session) {
        return dir_probe(&dir);
    }
    if let Some(file) = conversation
        .and_then(|c| agency_core::sessionstore::conversation_path(home, command, worktree, c))
    {
        return if file.exists() { ResumeProbe::Has } else { ResumeProbe::None };
    }
    let base = Path::new(command).file_name().and_then(|s| s.to_str()).unwrap_or(command);
    match base {
        "claude" => dir_probe(&home.join(".claude").join("projects").join(claude_enc(worktree))),
        "pi" => dir_probe(&home.join(".pi").join("agent").join("sessions").join(pi_enc(worktree))),
        _ => ResumeProbe::Unknown,
    }
}

fn dir_probe(dir: &Path) -> ResumeProbe {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                ResumeProbe::Has
            } else {
                ResumeProbe::None
            }
        }
        Err(_) => ResumeProbe::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn claude_probe_has_vs_none() {
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/Users/x/agency/.agency/worktrees/agent-abcd");
        // claude encodes '/' and '.' as '-' (leading '/' -> leading '-').
        let enc = "-Users-x-agency--agency-worktrees-agent-abcd";
        let dir = home.path().join(".claude").join("projects").join(enc);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(resume_probe(home.path(), "claude", wt, "agent-abcd", None), ResumeProbe::None);
        fs::write(dir.join("s.jsonl"), "x").unwrap();
        assert_eq!(resume_probe(home.path(), "claude", wt, "agent-abcd", None), ResumeProbe::Has);
    }

    #[test]
    fn claude_probe_none_when_dir_absent() {
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/Users/x/agency/.agency/worktrees/agent-zzzz");
        assert_eq!(resume_probe(home.path(), "claude", wt, "agent-zzzz", None), ResumeProbe::None);
    }

    #[test]
    fn pi_probe_reads_the_session_store_not_the_shared_directory() {
        // AGE-175's shape: one project checkout, two runs. A conversation in
        // the shared directory is a sibling's, and neither run can resume it.
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/Users/x/agency");
        // pi strips the leading '/', replaces '/' with '-' (dots kept), wraps in '--'..'--'.
        let shared =
            home.path().join(".pi").join("agent").join("sessions").join("--Users-x-agency--");
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join("session.jsonl"), "x").unwrap();
        assert_eq!(resume_probe(home.path(), "pi", wt, "agent-abcd", None), ResumeProbe::None);
        assert_eq!(resume_probe(home.path(), "pi", wt, "agent-zzzz", None), ResumeProbe::None);

        fs::create_dir_all(shared.join("agent-abcd")).unwrap();
        fs::write(shared.join("agent-abcd").join("session.jsonl"), "x").unwrap();
        assert_eq!(resume_probe(home.path(), "pi", wt, "agent-abcd", None), ResumeProbe::Has);
        assert_eq!(resume_probe(home.path(), "pi", wt, "agent-zzzz", None), ResumeProbe::None);
    }

    /// With a conversation on record the question is that one file, not the
    /// directory: `claude --resume <id>` refuses when the id is not there, and
    /// a sibling run's conversation next to it is no answer.
    #[test]
    fn claude_probe_asks_about_the_conversation_on_record() {
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/Users/x/proj");
        let dir = home.path().join(".claude").join("projects").join("-Users-x-proj");
        fs::create_dir_all(&dir).unwrap();
        let ours = "9674f5a1-334c-49a5-9952-89e592b0bc5b";
        let theirs = "1a4970ef-de34-4b3c-a444-41e9a73722fb";
        fs::write(dir.join(format!("{theirs}.jsonl")), "x").unwrap();
        assert_eq!(
            resume_probe(home.path(), "claude", wt, "agent-abcd", Some(ours)),
            ResumeProbe::None,
            "a sibling's conversation is not ours to resume"
        );
        fs::write(dir.join(format!("{ours}.jsonl")), "x").unwrap();
        assert_eq!(
            resume_probe(home.path(), "claude", wt, "agent-abcd", Some(ours)),
            ResumeProbe::Has
        );
    }

    /// Copilot keys sessions by id alone, under `~/.copilot/session-state/`,
    /// so the probe reads the transcript of the recorded conversation and
    /// ignores the worktree the run works in.
    #[test]
    fn copilot_probe_asks_about_the_conversation_on_record() {
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/Users/x/proj");
        let state = home.path().join(".copilot").join("session-state");
        let ours = "3f1c9a20-0001-4aaa-9aaa-000000000001";
        let theirs = "3f1c9a20-0001-4aaa-9aaa-000000000002";
        fs::create_dir_all(state.join(theirs)).unwrap();
        fs::write(state.join(theirs).join("events.jsonl"), "x").unwrap();
        assert_eq!(
            resume_probe(home.path(), "copilot", wt, "agent-abcd", Some(ours)),
            ResumeProbe::None,
            "a sibling's session in the same directory is not ours to resume"
        );
        fs::create_dir_all(state.join(ours)).unwrap();
        fs::write(state.join(ours).join("events.jsonl"), "x").unwrap();
        assert_eq!(
            resume_probe(home.path(), "copilot", wt, "agent-abcd", Some(ours)),
            ResumeProbe::Has
        );
    }

    #[test]
    fn unknown_agents_are_unknown() {
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/x");
        assert_eq!(
            resume_probe(home.path(), "opencode", wt, "agent-abcd", None),
            ResumeProbe::Unknown
        );
        assert_eq!(
            resume_probe(home.path(), "cursor-agent", wt, "agent-abcd", None),
            ResumeProbe::Unknown
        );
        assert_eq!(
            resume_probe(home.path(), "hermes", wt, "agent-abcd", None),
            ResumeProbe::Unknown
        );
    }
}
