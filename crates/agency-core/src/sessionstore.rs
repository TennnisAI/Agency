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
//! The fix is to stop sharing the directory: give each Agency session a store
//! of its own inside the agent's own project directory, so "the most recent
//! conversation here" is the one this session had. Only agents with a
//! documented, verified lever for that are pinned; everything else keeps the
//! old behaviour rather than getting a guessed flag that could kill the
//! launch.

use std::path::{Path, PathBuf};

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
    if !pinnable(command) {
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

/// The agents whose store Agency can pin, by launch command. A default-deny
/// list of one: pi.
///
/// Claude is open to the same collision by the same rule (`claude --help`:
/// "-c, --continue: Continue the most recent conversation in the current
/// directory"), though nothing has been observed hitting it — what decides the
/// exposure is whether two agents share a directory, and claude's runs here
/// have almost all had a worktree apiece. Its `--session-id` pins an id rather
/// than a store and takes a UUID, so it is a different shape of fix; filed as
/// AGE-177 rather than guessed at here.
fn pinnable(command: &str) -> bool {
    matches!(base(command), "pi")
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
    fn unpinnable_agents_get_no_store_and_no_environment() {
        let home = Path::new("/home/u");
        let wt = Path::new("/w");
        for command in ["claude", "codex", "opencode", "copilot", "cursor-agent", "hermes"] {
            assert_eq!(dir(home, command, wt, "agent-3f9c"), None, "{command}");
            assert!(env(home, command, wt, "agent-3f9c").is_empty(), "{command}");
        }
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
