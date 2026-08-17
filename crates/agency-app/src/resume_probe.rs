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
pub fn resume_probe(home: &Path, command: &str, worktree: &Path) -> ResumeProbe {
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
        assert_eq!(resume_probe(home.path(), "claude", wt), ResumeProbe::None); // empty dir
        fs::write(dir.join("s.jsonl"), "x").unwrap();
        assert_eq!(resume_probe(home.path(), "claude", wt), ResumeProbe::Has);
    }

    #[test]
    fn claude_probe_none_when_dir_absent() {
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/Users/x/agency/.agency/worktrees/agent-zzzz");
        assert_eq!(resume_probe(home.path(), "claude", wt), ResumeProbe::None);
    }

    #[test]
    fn pi_probe_encoding_and_detection() {
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/Users/x/agency");
        // pi strips the leading '/', replaces '/' with '-' (dots kept), wraps in '--'..'--'.
        let enc = "--Users-x-agency--";
        let dir = home.path().join(".pi").join("agent").join("sessions").join(enc);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("session.jsonl"), "x").unwrap();
        assert_eq!(resume_probe(home.path(), "pi", wt), ResumeProbe::Has);
    }

    #[test]
    fn unknown_agents_are_unknown() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(resume_probe(home.path(), "opencode", Path::new("/x")), ResumeProbe::Unknown);
        assert_eq!(
            resume_probe(home.path(), "cursor-agent", Path::new("/x")),
            ResumeProbe::Unknown
        );
        assert_eq!(resume_probe(home.path(), "hermes", Path::new("/x")), ResumeProbe::Unknown);
    }
}
