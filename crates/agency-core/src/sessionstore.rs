//! Where one Agency session's conversation is kept, and the environment that
//! pins the agent's CLI to it.
//!
//! Every resume recipe Agency ships means "continue the most recent
//! conversation in this working directory" (`pi -c`, `claude --continue`,
//! `codex resume --last`). That is only the run's own conversation while one
//! run has the directory to itself, which is not the general case: a
//! `worktree: false` run works in the project checkout, a gitless project has
//! nothing else to work in, and extra agent tabs share their run's worktree.
//! Several agents in one directory then all resume whichever session was
//! touched last (AGE-175: three pi runs in one project checkout, one
//! conversation between them, and each restart appending to whichever the
//! other had used).
//!
//! Reproduced from first principles against pi 0.84.4: two processes in one
//! cwd, `-p "run one"`, `-p "run two"`, then `-c` in the first — the first's
//! turn landed in the second's session file.
//!
//! The fix is to stop asking "which conversation was most recent here" at all.
//! There are two levers for that, and which one an agent gets is decided by
//! what its CLI actually offers:
//!
//! - **A store of its own** ([`Pin::Store`], pi). The CLI takes a session
//!   directory per process, so pointing each Agency session at one of its own
//!   makes "the most recent conversation here" that session's, with no state
//!   for Agency to keep.
//! - **A conversation of its own** ([`Pin::Id`], claude). The CLI names
//!   conversations, so Agency mints an id, opens the conversation under it,
//!   records it against the session, and later reopens that exact one.
//!
//! Everything else keeps its old resume recipe. A guessed flag would be worse
//! than the bug: `claude --session-id` on an id already in use and
//! `claude --resume` on one that is not there both refuse to start.
//!
//! Verified live rather than read off `--help` alone. Pi 0.84.4: two processes
//! in one cwd, `-p "run one"`, `-p "run two"`, then `-c` in the first — the
//! first's turn landed in the second's session file, and pinning the store
//! separated them. Claude 2.x, driven in a pty the way Agency launches it:
//! `--session-id <uuid>` opened `<uuid>.jsonl` in the cwd's project directory,
//! `--resume <uuid>` reopened that conversation and appended to the same file,
//! and two conversations in one directory each came back to their own.

use std::path::{Path, PathBuf};

/// How Agency keeps one session's conversation to itself, per agent. A
/// default-deny list: an agent is here only once its recipe has been read off
/// its own CLI and checked against a live launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pin {
    /// The CLI takes a session store per process; Agency gives it one.
    Store,
    /// The CLI names conversations; Agency mints the name.
    Id,
    /// No verified lever. The agent keeps whatever resume recipe it has, and
    /// two of these sharing a directory can still cross (AGE-177).
    None,
}

/// Which lever `command` offers, by launch command basename (a profile may
/// carry an absolute path).
///
/// Codex, opencode and copilot are deliberately absent: none is installed on
/// any machine this has been developed on, so neither their store layout nor a
/// pin has been checked against a running CLI. Cursor, hermes, gemini and kimi
/// key their sessions globally rather than by cwd and so ship with no resume
/// recipe at all; there is nothing here for them to cross.
pub fn pin(command: &str) -> Pin {
    match base(command) {
        "pi" => Pin::Store,
        "claude" => Pin::Id,
        _ => Pin::None,
    }
}

/// A conversation id `command` will accept for a conversation that does not
/// exist yet, or `None` for an agent that does not name them.
///
/// Claude requires a UUID ("--session-id <uuid>: Use a specific session ID for
/// the conversation (must be a valid UUID)") and refuses an id already in use,
/// so this is minted per fresh launch and never reused: a rerun is a new
/// conversation by definition.
pub fn mint(command: &str) -> Option<String> {
    match pin(command) {
        Pin::Id => Some(uuid::Uuid::new_v4().to_string()),
        Pin::Store | Pin::None => None,
    }
}

/// Argv that opens `conversation` as a new conversation, for an agent that
/// names them. Empty for everyone else, and empty when `so_far` already
/// carries the flag: a profile the user wrote one into keeps theirs, and
/// claude refuses a second `--session-id` outright.
pub fn open_args(command: &str, conversation: &str, so_far: &[String]) -> Vec<String> {
    match pin(command) {
        Pin::Id if !has_flag(so_far, &["--session-id"]) => {
            vec!["--session-id".into(), conversation.to_string()]
        }
        _ => Vec::new(),
    }
}

/// Argv that reopens exactly `conversation`, replacing the CLI's generic
/// "most recent conversation here" recipe. Empty for everyone else, which is
/// what keeps that recipe in use for them.
pub fn resume_args(command: &str, conversation: &str, so_far: &[String]) -> Vec<String> {
    match pin(command) {
        Pin::Id if !has_flag(so_far, &["--resume", "-r"]) => {
            vec!["--resume".into(), conversation.to_string()]
        }
        _ => Vec::new(),
    }
}

/// The file `conversation` is kept in, for an agent that names them, so a
/// caller can ask whether there is anything to reopen before trying. Claude
/// names the file for the id: `~/.claude/projects/<cwd>/<uuid>.jsonl`, which
/// is the same convention the archive record's resume line already relies on.
pub fn conversation_path(
    home: &Path,
    command: &str,
    worktree: &Path,
    conversation: &str,
) -> Option<PathBuf> {
    if pin(command) != Pin::Id {
        return None;
    }
    let name = leaf(conversation)?;
    Some(crate::usage::session_dir(home, command, worktree)?.join(format!("{name}.jsonl")))
}

/// Whether `so_far` already sets one of `flags`, so Agency's own argument can
/// stand down. Same rule as the MCP flags in [`crate::mcp`]: what the user
/// wrote wins.
fn has_flag(so_far: &[String], flags: &[&str]) -> bool {
    so_far.iter().any(|a| flags.iter().any(|f| a == f || a.starts_with(&format!("{f}="))))
}

/// Pi reads this before falling back to its per-cwd default; `--session-dir`
/// on the command line overrides it, so a profile that sets its own store
/// still wins. Read off `pi --help`'s environment section and pi 0.84.4's
/// `main.js`, and verified live: with it set, the session file is written
/// there and no per-cwd directory is created at all.
const PI_SESSION_DIR: &str = "PI_CODING_AGENT_SESSION_DIR";

/// Where `command` should keep the conversation of the Agency session named
/// `session`, or `None` for an agent Agency has no way to pin.
///
/// A subdirectory of the agent's own per-cwd store rather than a directory of
/// Agency's own: the archive rescue moves that tree wholesale when a worktree
/// goes (AGE-152), a discard removes it, and both keep working on a nested
/// store for free. Pi ignores anything that is not a `.jsonl` when it lists a
/// directory, so the nested stores are invisible to the user's own `pi -c` in
/// the same folder rather than confusing it.
///
/// `session` is a run id (`agent-3f9c`) or an extra tab's (`agent-3f9c--2`).
/// It is allowlisted to alphanumerics, `-`, `_` and `.` rather than escaped:
/// these are app-generated ids, and a path component assembled from anything
/// else is a traversal waiting to be found rather than a name to fix up.
pub fn dir(home: &Path, command: &str, worktree: &Path, session: &str) -> Option<PathBuf> {
    if pin(command) != Pin::Store {
        return None;
    }
    let name = leaf(session)?;
    Some(crate::usage::session_dir(home, command, worktree)?.join(name))
}

/// The environment that makes `command` write and resume this session's
/// conversation in its own store. Empty for every agent Agency cannot pin, so
/// callers can extend unconditionally.
///
/// Deliberately not applied to a run's shell tab or a terminal run: an agent
/// the user starts by hand there is theirs, and it should see the workspace's
/// ordinary store, including whatever conversations predate this pinning.
pub fn env(home: &Path, command: &str, worktree: &Path, session: &str) -> Vec<(String, String)> {
    let Some(dir) = dir(home, command, worktree, session) else {
        return Vec::new();
    };
    match base(command) {
        "pi" => vec![(PI_SESSION_DIR.to_string(), dir.to_string_lossy().into_owned())],
        _ => Vec::new(),
    }
}

/// Only the basename is matched, since a profile may carry an absolute path.
fn base(command: &str) -> &str {
    Path::new(command).file_name().and_then(|s| s.to_str()).unwrap_or(command)
}

/// `session` as a single path component, or `None` if it is not one.
fn leaf(session: &str) -> Option<&str> {
    let ok = !session.is_empty()
        && session != "."
        && session != ".."
        && session.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    ok.then_some(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_gets_a_store_under_its_own_project_directory() {
        let home = Path::new("/home/u");
        let wt = Path::new("/Users/x/proj");
        assert_eq!(
            dir(home, "pi", wt, "agent-3f9c").unwrap(),
            Path::new("/home/u/.pi/agent/sessions/--Users-x-proj--/agent-3f9c")
        );
        // An absolute path in the profile resolves the same way.
        assert_eq!(
            dir(home, "/opt/homebrew/bin/pi", wt, "agent-3f9c").unwrap(),
            Path::new("/home/u/.pi/agent/sessions/--Users-x-proj--/agent-3f9c")
        );
    }

    #[test]
    fn two_sessions_in_one_directory_get_two_stores() {
        // The bug in one assertion: same worktree, different session, and the
        // "most recent conversation here" each one resumes has to differ.
        let home = Path::new("/home/u");
        let wt = Path::new("/Users/x/proj");
        assert_ne!(dir(home, "pi", wt, "agent-3f9c"), dir(home, "pi", wt, "agent-77bd"));
        // An extra tab is its own session too: it shares the worktree with its
        // run, which is exactly how it used to steal the run's resume.
        assert_ne!(dir(home, "pi", wt, "agent-3f9c"), dir(home, "pi", wt, "agent-3f9c--2"));
    }

    #[test]
    fn agents_without_a_verified_lever_are_left_alone() {
        let home = Path::new("/home/u");
        let wt = Path::new("/w");
        for command in ["codex", "opencode", "copilot", "cursor-agent", "hermes"] {
            assert_eq!(pin(command), Pin::None, "{command}");
            assert_eq!(dir(home, command, wt, "agent-3f9c"), None, "{command}");
            assert!(env(home, command, wt, "agent-3f9c").is_empty(), "{command}");
            assert_eq!(mint(command), None, "{command}");
            assert!(open_args(command, "x", &[]).is_empty(), "{command}");
            assert!(resume_args(command, "x", &[]).is_empty(), "{command}");
        }
    }

    /// The two levers are exclusive: an agent given a store of its own is not
    /// also handed conversation ids, and the other way round.
    #[test]
    fn each_agent_gets_exactly_one_lever() {
        let home = Path::new("/home/u");
        let wt = Path::new("/Users/x/proj");
        assert_eq!(pin("pi"), Pin::Store);
        assert!(dir(home, "pi", wt, "agent-3f9c").is_some());
        assert_eq!(mint("pi"), None);
        assert!(open_args("pi", "x", &[]).is_empty());
        assert!(resume_args("pi", "x", &[]).is_empty(), "pi keeps -c, scoped by its own store");

        assert_eq!(pin("claude"), Pin::Id);
        assert_eq!(dir(home, "claude", wt, "agent-3f9c"), None);
        assert!(env(home, "claude", wt, "agent-3f9c").is_empty());
    }

    #[test]
    fn claude_opens_and_reopens_a_named_conversation() {
        let id = "9674f5a1-334c-49a5-9952-89e592b0bc5b";
        assert_eq!(open_args("claude", id, &[]), vec!["--session-id", id]);
        assert_eq!(resume_args("claude", id, &[]), vec!["--resume", id]);
        // An absolute path in the profile resolves to the same recipe.
        assert_eq!(open_args("/opt/homebrew/bin/claude", id, &[]), vec!["--session-id", id]);
        assert_eq!(
            conversation_path(Path::new("/home/u"), "claude", Path::new("/Users/x/proj"), id)
                .unwrap(),
            Path::new("/home/u/.claude/projects/-Users-x-proj").join(format!("{id}.jsonl"))
        );
    }

    #[test]
    fn a_minted_id_is_fresh_every_time_and_a_uuid() {
        // Claude refuses `--session-id` for an id already in use, so a rerun
        // reusing one would not start at all.
        let a = mint("claude").unwrap();
        let b = mint("claude").unwrap();
        assert_ne!(a, b);
        assert!(uuid::Uuid::parse_str(&a).is_ok(), "{a}");
    }

    #[test]
    fn a_profile_that_names_its_own_conversation_keeps_it() {
        let id = "9674f5a1-334c-49a5-9952-89e592b0bc5b";
        let set = ["--session-id".to_string(), "other".to_string()];
        assert!(open_args("claude", id, &set).is_empty());
        let resumed = ["-r".to_string(), "other".to_string()];
        assert!(resume_args("claude", id, &resumed).is_empty());
        // `--flag=value` counts as set too.
        let joined = ["--resume=other".to_string()];
        assert!(resume_args("claude", id, &joined).is_empty());
    }

    #[test]
    fn pi_environment_names_the_store() {
        let env = env(Path::new("/home/u"), "pi", Path::new("/Users/x/proj"), "agent-3f9c");
        assert_eq!(
            env,
            vec![(
                "PI_CODING_AGENT_SESSION_DIR".to_string(),
                "/home/u/.pi/agent/sessions/--Users-x-proj--/agent-3f9c".to_string()
            )]
        );
    }

    #[test]
    fn a_session_id_that_is_not_one_path_component_is_refused() {
        let home = Path::new("/home/u");
        let wt = Path::new("/w");
        for bad in ["", ".", "..", "../../etc", "a/b", "a b", "a\0b"] {
            assert_eq!(dir(home, "pi", wt, bad), None, "{bad:?}");
            assert!(env(home, "pi", wt, bad).is_empty(), "{bad:?}");
        }
    }
}
