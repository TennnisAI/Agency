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
//! - **A conversation of its own** ([`Pin::Id`], claude and copilot). The CLI
//!   names conversations, so Agency mints an id, opens the conversation under
//!   it, records it against the session, and later reopens that exact one.
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
//! and two conversations in one directory each came back to their own. Copilot
//! CLI 1.0.83, three sessions opened in one directory under ids of Agency's
//! choosing, each asked for a word of its own: `--continue` there came back
//! with the third one's word, which is the bug, and `--session-id <first id>`
//! came back with the first one's, which is the fix. Then the same flag in a
//! pty on the interactive launch Agency actually uses, where it wrote a
//! `workspace.yaml` naming that exact id.

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
    /// No lever its CLI offers. The agent keeps whatever resume recipe it has,
    /// and two of these sharing a directory can still cross (AGE-177,
    /// AGE-188).
    None,
}

/// Which lever `command` offers, by launch command basename (a profile may
/// carry an absolute path).
///
/// Codex and opencode stay absent, now for having been installed and read
/// rather than for nothing being known about them (AGE-188). Neither offers
/// either lever:
///
/// - **codex** mints its own UUID per thread and has no flag or variable that
///   sets it. `CODEX_SESSION_ID` and `CODEX_THREAD_ID` are handed *out* to the
///   tools codex runs, not read as input: 0.153.4 with either set to a UUID of
///   ours still printed a session id of its own. A thread *name* would do,
///   since `codex resume` takes "Session id (UUID) or session name", but
///   `thread_name` is a field of the app-server's new-thread call and the CLI
///   exposes only `--thread-source`. That leaves a store, and both variables
///   that move one move too much with it: `CODEX_HOME` carries `auth.json`,
///   and `CODEX_SQLITE_HOME` carries `memories_1.sqlite`, the cross-thread
///   memory codex distils from past rollouts, so a per-session store would
///   trade this bug for an agent that remembers nothing it ever did here.
/// - **opencode** names conversations but will not open one under a name of
///   ours. `-s, --session` is "session id to continue", and 1.17.10 answers an
///   id that is not already there with "Error: Session not found" and exits.
///   Its store is no better: sessions live in the sqlite database beside the
///   `account` and `credential` tables, so pinning it per session with
///   `OPENCODE_DB` (or `XDG_DATA_HOME` above it, which also carries
///   `auth.json`) would log the user out on every Agency launch.
///
/// Cursor, hermes, gemini and kimi key their sessions globally rather than by
/// cwd and so ship with no resume recipe at all; there is nothing here for
/// them to cross.
pub fn pin(command: &str) -> Pin {
    match base(command) {
        "pi" => Pin::Store,
        _ if id_flags(command).is_some() => Pin::Id,
        _ => Pin::None,
    }
}

/// The flags `command` takes a conversation id behind: the one that opens a
/// fresh conversation under a name of Agency's choosing, the one that reopens
/// exactly that conversation, and the flags a user's own arguments may already
/// carry that name a conversation themselves. `None` for an agent that does
/// not name them, which is what makes [`pin`] a default-deny list.
///
/// Claude splits the two flags and each refuses the other's case: read off
/// `claude --help` and verified in a pty. Copilot's one flag does both, and
/// its `--help` says so in as many words: "--session-id <id>  Resume an
/// existing session or task by ID, or set the UUID for a new session", with
/// `copilot --session-id=0cb916db-…` given as the example of starting a new
/// session under a chosen UUID. Verified against 1.0.83 rather than taken on
/// trust: a fresh id opened a session that reported that exact id back in its
/// own `--resume` hint, and the same id later reopened it.
///
/// One flag for both directions is the better shape of the two, because it
/// cannot refuse: copilot creates the session when the id is unknown instead
/// of failing to start, which is the risk that keeps a guessed recipe out of
/// here in the first place.
fn id_flags(command: &str) -> Option<IdFlags> {
    match base(command) {
        "claude" => Some(IdFlags { open: "--session-id", resume: "--resume" }),
        "copilot" => Some(IdFlags { open: "--session-id", resume: "--session-id" }),
        _ => None,
    }
}

/// How one agent's CLI is told which conversation to use (see [`id_flags`]).
struct IdFlags {
    /// Opens a fresh conversation under the id that follows.
    open: &'static str,
    /// Reopens exactly the conversation named by the id that follows.
    resume: &'static str,
}

/// Flags that mean the user has named the conversation themselves, in either
/// direction, so Agency's own argument stands down. Broader than the flags
/// Agency adds: `claude --resume other --session-id <ours>` is two answers to
/// one question even on a fresh launch, and the CLI rejects it.
const USER_NAMED: &[&str] = &["--session-id", "--resume", "-r"];

/// A conversation id `command` will accept for a conversation that does not
/// exist yet, or `None` for an agent that does not name them.
///
/// A UUID because both agents that name conversations want one: claude's
/// "--session-id <uuid>: Use a specific session ID for the conversation (must
/// be a valid UUID)", and copilot's "or set the UUID for a new session". Never
/// reused, minted per fresh launch: a rerun is a new conversation by
/// definition, and claude refuses an id already in use.
pub fn mint(command: &str) -> Option<String> {
    match pin(command) {
        Pin::Id => Some(uuid::Uuid::new_v4().to_string()),
        Pin::Store | Pin::None => None,
    }
}

/// Argv that opens `conversation` as a new conversation, for an agent that
/// names them. Empty for everyone else, and empty when `so_far` already
/// carries a conversation flag: a profile the user wrote one into keeps
/// theirs, and claude refuses a second `--session-id` outright.
pub fn open_args(command: &str, conversation: &str, so_far: &[String]) -> Vec<String> {
    match id_flags(command) {
        Some(f) if !has_flag(so_far, USER_NAMED) => vec![f.open.into(), conversation.to_string()],
        _ => Vec::new(),
    }
}

/// Argv that reopens exactly `conversation`, replacing the CLI's generic
/// "most recent conversation here" recipe. Empty for everyone else, which is
/// what keeps that recipe in use for them.
pub fn resume_args(command: &str, conversation: &str, so_far: &[String]) -> Vec<String> {
    match id_flags(command) {
        Some(f) if !has_flag(so_far, USER_NAMED) => vec![f.resume.into(), conversation.to_string()],
        _ => Vec::new(),
    }
}

/// The file `conversation` is kept in, for an agent that names them, so a
/// caller can ask whether there is anything to reopen before trying.
pub fn conversation_path(
    home: &Path,
    command: &str,
    worktree: &Path,
    conversation: &str,
) -> Option<PathBuf> {
    let name = leaf(conversation)?;
    match base(command) {
        // Claude names the file for the id, under the directory it keys by
        // cwd: `~/.claude/projects/<cwd>/<uuid>.jsonl`, the same convention
        // the archive record's resume line already relies on.
        "claude" => {
            Some(crate::usage::session_dir(home, command, worktree)?.join(format!("{name}.jsonl")))
        }
        // Copilot keys sessions by id alone, not by cwd, so `worktree` says
        // nothing about where this one lives: one directory per session under
        // `~/.copilot/session-state/`, with the transcript in `events.jsonl`.
        //
        // The transcript and not the directory, because 1.0.83 writes them at
        // different moments. An interactive launch under a fresh id created
        // the directory and a `workspace.yaml` naming that id immediately,
        // then sat on its folder-trust prompt and recorded no turn at all;
        // `events.jsonl` appeared only once a turn did. Answering "Has" for
        // that session would resume an empty conversation with the run's
        // prompt undelivered, which is the one outcome this probe exists to
        // avoid.
        "copilot" => {
            Some(home.join(".copilot").join("session-state").join(name).join("events.jsonl"))
        }
        _ => None,
    }
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
    fn agents_whose_cli_offers_no_lever_are_left_alone() {
        let home = Path::new("/home/u");
        let wt = Path::new("/w");
        // codex and opencode are here having been asked, not assumed: codex
        // will not be told a thread id, and opencode refuses one it has not
        // seen (see `pin`). The rest key sessions globally and never resume.
        for command in ["codex", "opencode", "cursor-agent", "hermes", "gemini", "kimi"] {
            assert_eq!(pin(command), Pin::None, "{command}");
            assert_eq!(dir(home, command, wt, "agent-3f9c"), None, "{command}");
            assert!(env(home, command, wt, "agent-3f9c").is_empty(), "{command}");
            assert_eq!(mint(command), None, "{command}");
            assert!(open_args(command, "x", &[]).is_empty(), "{command}");
            assert!(resume_args(command, "x", &[]).is_empty(), "{command}");
            assert_eq!(conversation_path(home, command, wt, "x"), None, "{command}");
        }
    }

    /// Every agent [`pin`] calls [`Pin::Id`] has to have both halves of the
    /// recipe, or it would launch with the generic "most recent here" one and
    /// this module would report a pin it is not applying.
    #[test]
    fn every_named_conversation_agent_can_be_opened_and_reopened() {
        let home = Path::new("/home/u");
        let wt = Path::new("/Users/x/proj");
        let id = "9674f5a1-334c-49a5-9952-89e592b0bc5b";
        for command in ["claude", "copilot"] {
            assert_eq!(pin(command), Pin::Id, "{command}");
            assert!(mint(command).is_some(), "{command}");
            assert!(!open_args(command, id, &[]).is_empty(), "{command}");
            assert!(!resume_args(command, id, &[]).is_empty(), "{command}");
            assert!(conversation_path(home, command, wt, id).is_some(), "{command}");
        }
    }

    /// Copilot's one flag does both, and creates the session when the id is
    /// unknown rather than refusing to start, so a fresh launch and a resume
    /// are the same argument.
    #[test]
    fn copilot_opens_and_reopens_with_the_same_flag() {
        let id = "3f1c9a20-0001-4aaa-9aaa-000000000001";
        assert_eq!(open_args("copilot", id, &[]), vec!["--session-id", id]);
        assert_eq!(resume_args("copilot", id, &[]), vec!["--session-id", id]);
        // An absolute path in the profile resolves to the same recipe.
        assert_eq!(open_args("/opt/homebrew/bin/copilot", id, &[]), vec!["--session-id", id]);
        // Keyed by id alone, so the same conversation is at the same path
        // whichever worktree the run works in.
        let expected = Path::new("/home/u/.copilot/session-state").join(id).join("events.jsonl");
        for wt in ["/Users/x/proj", "/Users/x/other"] {
            assert_eq!(
                conversation_path(Path::new("/home/u"), "copilot", Path::new(wt), id).unwrap(),
                expected
            );
        }
        // Agency has no store to give it, only a name.
        assert_eq!(dir(Path::new("/home/u"), "copilot", Path::new("/w"), "agent-3f9c"), None);
        assert!(env(Path::new("/home/u"), "copilot", Path::new("/w"), "agent-3f9c").is_empty());
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

        for named in ["claude", "copilot"] {
            assert_eq!(pin(named), Pin::Id, "{named}");
            assert_eq!(dir(home, named, wt, "agent-3f9c"), None, "{named}");
            assert!(env(home, named, wt, "agent-3f9c").is_empty(), "{named}");
        }
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
        for command in ["claude", "copilot"] {
            for theirs in [
                vec!["--session-id".to_string(), "other".to_string()],
                vec!["-r".to_string(), "other".to_string()],
                // `--flag=value` counts as set too.
                vec!["--resume=other".to_string()],
            ] {
                assert!(open_args(command, id, &theirs).is_empty(), "{command} {theirs:?}");
                assert!(resume_args(command, id, &theirs).is_empty(), "{command} {theirs:?}");
            }
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
