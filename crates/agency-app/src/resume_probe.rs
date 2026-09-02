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
/// `session` is the Agency session about to be launched.
///
/// The probe has to ask about the same store the launch will use, or it
/// answers a different question than the one that decides resume-vs-fresh: a
/// pinned agent (AGE-175) resumes its own session's store, so a sibling
/// session's conversation in the shared directory is not something it can
/// continue, and reading it as "Has" would resume an empty store with the
/// run's prompt nowhere.
pub fn resume_probe(home: &Path, command: &str, worktree: &Path, session: &str) -> ResumeProbe {
    if let Some(dir) = agency_core::sessionstore::dir(home, command, worktree, session) {
        return dir_probe(&dir);
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
        assert_eq!(resume_probe(home.path(), "claude", wt, "agent-abcd"), ResumeProbe::None);
        fs::write(dir.join("s.jsonl"), "x").unwrap();
        assert_eq!(resume_probe(home.path(), "claude", wt, "agent-abcd"), ResumeProbe::Has);
    }

    #[test]
    fn claude_probe_none_when_dir_absent() {
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/Users/x/agency/.agency/worktrees/agent-zzzz");
        assert_eq!(resume_probe(home.path(), "claude", wt, "agent-zzzz"), ResumeProbe::None);
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
        assert_eq!(resume_probe(home.path(), "pi", wt, "agent-abcd"), ResumeProbe::None);
        assert_eq!(resume_probe(home.path(), "pi", wt, "agent-zzzz"), ResumeProbe::None);

        fs::create_dir_all(shared.join("agent-abcd")).unwrap();
        fs::write(shared.join("agent-abcd").join("session.jsonl"), "x").unwrap();
        assert_eq!(resume_probe(home.path(), "pi", wt, "agent-abcd"), ResumeProbe::Has);
        assert_eq!(resume_probe(home.path(), "pi", wt, "agent-zzzz"), ResumeProbe::None);
    }

    #[test]
    fn unknown_agents_are_unknown() {
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/x");
        assert_eq!(resume_probe(home.path(), "opencode", wt, "agent-abcd"), ResumeProbe::Unknown);
        assert_eq!(
            resume_probe(home.path(), "cursor-agent", wt, "agent-abcd"),
            ResumeProbe::Unknown
        );
        assert_eq!(resume_probe(home.path(), "hermes", wt, "agent-abcd"), ResumeProbe::Unknown);
    }
}
