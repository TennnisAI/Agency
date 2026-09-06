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
//! - **A conversation of its own** ([`Pin::Id`], claude, copilot, cursor and
//!   kimi). The CLI names conversations, so Agency mints an id, opens the
//!   conversation under it, records it against the session, and later
//!   reopens that exact one.
//!
//! Everything else keeps its old resume recipe. A guessed flag would be worse
//! than the bug: `claude --session-id` on an id already in use and
//! `claude --resume` on one that is not there both refuse to start.
//!
//! Cursor and kimi shipped with no resume recipe at all until AGE-190, on the
//! belief that they keyed sessions globally rather than by cwd, so that
//! "continue the last conversation" would have picked up whatever the user
//! last did anywhere on the machine. Driven, both turned out to key by cwd
//! after all (an md5 of it, see [`store_dir`]), so their generic `--continue`
//! stands on the same footing as claude's; and both name conversations, so
//! a session's own is reopened exactly the way claude's and copilot's are.
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
//!
//! Cursor 2026.09.02 and kimi 1.50.0 (AGE-190), each driven headless in two
//! directories and then interactively in a pty: two conversations opened in
//! one directory under ids of Agency's choosing, each asked for a word of its
//! own, and the first reopened by id came back with the first one's word.
//! Cursor answered through the user's own account; kimi (and hermes, below)
//! had no credential here and were driven against a local stand-in for an
//! OpenAI-compatible endpoint that replies with the first user message in
//! the request, which shows exactly which conversation the CLI sent and is
//! the whole question. What each writes at launch and what only a turn adds
//! is recorded on [`conversation_path`] and [`turn_marker`], because the two
//! differ.

use md5::{Digest, Md5};
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
/// Hermes and gemini stay absent too, after AGE-190 installed and read all
/// four agents that had no resume recipe (cursor and kimi are in
/// [`id_flags`] now):
///
/// - **hermes** mints its own session id, `<YYYYmmdd_HHMMSS>_<6 hex>`, in its
///   CLI and takes none from outside: no flag, and the one variable that
///   carries an id (`HERMES_TUI_RESUME`) is the TUI's own resume handoff.
///   `--resume <id>` reopens exactly that session and answers an unknown id
///   with "Session not found" and exit 1. `--continue` is the newest session
///   for the source (cli or tui) in the one global `state.db`, with no cwd
///   filter: driven in 0.19.0, from one directory with a newer session in
///   another, it came back with the other's conversation *and moved into
///   that directory* ("restored workspace dir"), which is the AGE-175 bug
///   with a `cd` on top. So the generic recipe stays empty. Its store is
///   `HERMES_HOME`, which also carries `config.yaml`, the `.env` of API keys,
///   skills and memory, so a per-session store would launch an agent with no
///   provider. Sessions do record their cwd in `state.db`, so an id could be
///   read back after the first turn (the newest row whose cwd is the
///   worktree); that is a third lever, discovering rather than naming, and
///   is not built.
/// - **gemini** has claude's shape, read off 0.58.0's `--help` and source:
///   `--session-id <id>` "Start a new session with a manually provided UUID"
///   and refuses one already present ("already exists. Use --resume to
///   resume it"), `--resume <uuid>` reopens exactly and exits on an unknown
///   one. It is not here because no turn could be driven (it takes a
///   `GEMINI_API_KEY` or a login, and this machine had neither), and the
///   probe would need more than a path: the transcript is
///   `~/.gemini/tmp/<slug>/chats/session-<YYYY-MM-DDTHH-MM>-<first 8 of
///   id>.jsonl`, where the slug is assigned per project root in
///   `~/.gemini/projects.json` rather than derived from it, so the file is
///   found by registry lookup and glob. Launched in a pty with a fresh id,
///   it wrote that file at once with the metadata line alone; on the next
///   launch `--resume <that id>` said "No previous sessions found for this
///   project" and the file was gone. So a session with no turn is not a
///   session, and the probe's question would be whether the file has a
///   second line.
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
///
/// Cursor has claude's shape behind a flag its `--help` does not list:
/// `--new-session-id <uuid>`, "Create a new session with a caller-provided
/// ID", found in 2026.09.02's bundle as a hidden option and driven. It wants
/// a UUIDv4 ("expected a UUIDv4"), refuses an id already in use ("Session ID
/// ... is already in use.") and refuses to be combined with `--resume` or
/// `--continue`. `--resume <chatId>` reopened the exact chat, with the other
/// chat in the same directory left alone. What `--resume` does with an id it
/// does not have is the reason the probe has to be exact: it does not refuse.
/// In the chat's own directory it opened an empty chat under that id; from
/// another directory it came back with *that* directory's latest chat while
/// creating a folder named for the id there. So a wrong "Has" resumes
/// someone else's conversation silently, which is the AGE-188 bug.
///
/// Kimi has copilot's shape, and says so in its source rather than its
/// `--help`: `--session <id>` ("With ID: resume that session") is
/// `Session.find` and, when that finds nothing, "Session not found, creating
/// new session" under that same id. Driven in 1.50.0: a fresh id opened a
/// session, the same id later reopened it with its first prompt still in the
/// request, and a second id in the same directory stayed separate.
fn id_flags(command: &str) -> Option<IdFlags> {
    match base(command) {
        "claude" => Some(IdFlags {
            open: "--session-id",
            resume: "--resume",
            theirs: &["--session-id", "--resume", "-r"],
        }),
        "copilot" => Some(IdFlags {
            open: "--session-id",
            resume: "--session-id",
            theirs: &["--session-id", "--resume", "-r"],
        }),
        "cursor-agent" => Some(IdFlags {
            open: "--new-session-id",
            resume: "--resume",
            theirs: &["--new-session-id", "--resume", "--continue"],
        }),
        "kimi" => Some(IdFlags {
            open: "--session",
            resume: "--session",
            theirs: &["--session", "-S", "--resume", "-r", "--continue", "-C"],
        }),
        _ => None,
    }
}

/// How one agent's CLI is told which conversation to use (see [`id_flags`]).
struct IdFlags {
    /// Opens a fresh conversation under the id that follows.
    open: &'static str,
    /// Reopens exactly the conversation named by the id that follows.
    resume: &'static str,
    /// Flags that mean the user has named the conversation themselves, in
    /// either direction, so Agency's own argument stands down. Broader than
    /// the flags Agency adds: `claude --resume other --session-id <ours>` is
    /// two answers to one question even on a fresh launch, and the CLI
    /// rejects it; cursor refuses `--new-session-id` beside `--continue` in
    /// as many words. Per agent because the short forms collide: `-r` is
    /// resume for claude and kimi alike, but kimi's `-c` is `--command`, its
    /// prompt, and `-C` its continue.
    theirs: &'static [&'static str],
}

/// A conversation id `command` will accept for a conversation that does not
/// exist yet, or `None` for an agent that does not name them.
///
/// A UUID because every agent that names conversations wants one: claude's
/// "--session-id <uuid>: Use a specific session ID for the conversation (must
/// be a valid UUID)", copilot's "or set the UUID for a new session", cursor's
/// "expected a UUIDv4" (a version-4 one specifically, which is what `new_v4`
/// mints), and kimi's own ids are UUIDs. Never reused, minted per fresh
/// launch: a rerun is a new conversation by definition, and claude and cursor
/// refuse an id already in use.
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
        Some(f) if !has_flag(so_far, f.theirs) => vec![f.open.into(), conversation.to_string()],
        _ => Vec::new(),
    }
}

/// Argv that reopens exactly `conversation`, replacing the CLI's generic
/// "most recent conversation here" recipe. Empty for everyone else, which is
/// what keeps that recipe in use for them.
pub fn resume_args(command: &str, conversation: &str, so_far: &[String]) -> Vec<String> {
    match id_flags(command) {
        Some(f) if !has_flag(so_far, f.theirs) => vec![f.resume.into(), conversation.to_string()],
        _ => Vec::new(),
    }
}

/// The file `conversation` is kept in, for an agent that names them, so a
/// caller can ask whether there is anything to reopen before trying. Whether
/// the file existing is enough of an answer is [`turn_marker`]'s to say.
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
        // Cursor keys chats by cwd (see `store_dir`) and keeps each in a
        // directory named for its id, with the transcript in a sqlite
        // `store.db` beside a `meta.json`. Both are written at launch: in a
        // pty, `--new-session-id <fresh id>` with no prompt produced the
        // directory, `store.db` and a `meta.json` saying
        // `"hasConversation":false` before any turn, and the first turn,
        // typed or passed on argv, flipped it to `true`. So the file to read
        // is `meta.json`, and reading it is the probe's job
        // (`turn_marker`); `prompt_history.json` would be the file that
        // appears with a turn, but only a *typed* one, and a run's prompt
        // arrives on argv.
        "cursor-agent" => Some(store_dir(home, command, worktree)?.join(name).join("meta.json")),
        // Kimi keys sessions by cwd too (see `store_dir`) and keeps each in a
        // directory named for its id. `context.jsonl` is not the marker:
        // `Session.create` touches it at launch and writes the system prompt
        // into it, 18 KB before a word is said. `wire.jsonl`, the turn log,
        // appeared only with the first turn (driven in a pty, 1.50.0), and
        // it is what kimi's own session list keys on: a session whose wire
        // file is empty is skipped as if it were not there.
        "kimi" => Some(store_dir(home, command, worktree)?.join(name).join("wire.jsonl")),
        _ => None,
    }
}

/// What has to be true of the file at [`conversation_path`] before it counts
/// as a conversation with a turn in it, per agent. Copilot's lesson (AGE-188):
/// ask about what the first turn wrote, never what the launch wrote, or a run
/// whose agent never answered resumes empty with its prompt undelivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    /// The file appears with the first turn, so existing is the answer.
    Exists,
    /// The file is written at launch and the first turn changes what it says:
    /// cursor's `meta.json`, which the caller reads and hands to
    /// [`cursor_meta_records_turn`].
    CursorMeta,
}

/// See [`Turn`].
pub fn turn_marker(command: &str) -> Turn {
    match base(command) {
        "cursor-agent" => Turn::CursorMeta,
        _ => Turn::Exists,
    }
}

/// Whether cursor's `meta.json`, given as `contents`, records a turn.
/// `{"schemaVersion":1,"createdAtMs":…,"hasConversation":true,"title":"Kiwi
/// Only","updatedAtMs":…,"cwd":"…"}` after the first turn; the same with
/// `"hasConversation":false` and no title straight after launch. Anything
/// unparseable is no turn: the probe then starts fresh with the prompt
/// delivered, which beats reopening a chat that may be empty.
pub fn cursor_meta_records_turn(contents: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(contents)
        .ok()
        .and_then(|v| v.get("hasConversation")?.as_bool())
        .unwrap_or(false)
}

/// The directory an agent keeps every conversation from `worktree` in, for
/// the agents that key their store by cwd with a hash rather than by an
/// encoding of the path ([`crate::usage::session_dir`] has those). `None`
/// for everyone else. Used to place a named conversation and, by the resume
/// probe, to ask whether the generic `--continue` has anything to continue.
///
/// Both hash the cwd string with md5, hex-encoded, and neither resolves
/// symlinks first: cursor's `chats` state module does
/// `createHash("md5").update(path.resolve(cwd))`, and kimi's `WorkDirMeta`
/// does `md5(path.encode())` of a path canonicalised "unlike
/// `pathlib.Path.resolve`" without following links. Checked against a live
/// store: the worktree's path sat under that exact string's md5 and nothing
/// else. `store_dir_is_the_md5_of_the_worktree_path` pins the pairing. The cwd
/// each hashes is the one the process was started in, so a worktree reached
/// through a symlink would hash to the resolved path; Agency's worktrees are
/// not.
///
/// The roots can be moved by the agents' own variables (`CURSOR_CONFIG_DIR`
/// or `XDG_CONFIG_HOME` for cursor, `KIMI_SHARE_DIR` for kimi). Not honoured
/// here, the same as `~/.claude` and `~/.copilot` above: a user who moves a
/// store is a user whose probe answers "None" and gets a fresh launch, not
/// a wrong resume.
pub fn store_dir(home: &Path, command: &str, worktree: &Path) -> Option<PathBuf> {
    let root = match base(command) {
        "cursor-agent" => home.join(".cursor").join("chats"),
        "kimi" => home.join(".kimi").join("sessions"),
        _ => return None,
    };
    Some(root.join(md5_hex(&worktree.to_string_lossy())))
}

fn md5_hex(s: &str) -> String {
    format!("{:x}", Md5::digest(s.as_bytes()))
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
        // All four are here having been asked, not assumed: codex will not be
        // told a thread id, opencode refuses one it has not seen, hermes
        // mints its own and keys `--continue` globally, and gemini has the
        // lever but no driven turn behind it (see `pin`).
        for command in ["codex", "opencode", "hermes", "gemini"] {
            assert_eq!(pin(command), Pin::None, "{command}");
            assert_eq!(dir(home, command, wt, "agent-3f9c"), None, "{command}");
            assert!(env(home, command, wt, "agent-3f9c").is_empty(), "{command}");
            assert_eq!(mint(command), None, "{command}");
            assert!(open_args(command, "x", &[]).is_empty(), "{command}");
            assert!(resume_args(command, "x", &[]).is_empty(), "{command}");
            assert_eq!(conversation_path(home, command, wt, "x"), None, "{command}");
            assert_eq!(store_dir(home, command, wt), None, "{command}");
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
        for command in ["claude", "copilot", "cursor-agent", "kimi"] {
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

        for named in ["claude", "copilot", "cursor-agent", "kimi"] {
            assert_eq!(pin(named), Pin::Id, "{named}");
            assert_eq!(dir(home, named, wt, "agent-3f9c"), None, "{named}");
            assert!(env(home, named, wt, "agent-3f9c").is_empty(), "{named}");
        }
    }

    /// Cursor's opening flag is the hidden `--new-session-id`, which refuses
    /// an id already in use, and its resume is `--resume`, which does not
    /// refuse anything: exactly claude's split, so the recipe is the same.
    #[test]
    fn cursor_opens_with_the_hidden_flag_and_reopens_with_resume() {
        let id = "712a88f3-e1c4-40f9-b5c3-842d55dc410a";
        assert_eq!(open_args("cursor-agent", id, &[]), vec!["--new-session-id", id]);
        assert_eq!(resume_args("cursor-agent", id, &[]), vec!["--resume", id]);
        assert_eq!(
            open_args("/Users/x/.local/bin/cursor-agent", id, &[]),
            vec!["--new-session-id", id]
        );
        // Keyed by cwd: the same id in two directories is two chats, which is
        // what `--resume` from the wrong directory quietly demonstrated.
        let home = Path::new("/home/u");
        let a = conversation_path(home, "cursor-agent", Path::new("/Users/x/proj"), id).unwrap();
        let b = conversation_path(home, "cursor-agent", Path::new("/Users/x/other"), id).unwrap();
        assert_ne!(a, b);
        assert!(a.ends_with(Path::new(id).join("meta.json")), "{}", a.display());
        assert!(a.starts_with("/home/u/.cursor/chats"), "{}", a.display());
        assert_eq!(turn_marker("cursor-agent"), Turn::CursorMeta);
    }

    /// Kimi's one flag opens and reopens, like copilot's, so a fresh launch
    /// and a resume are the same argument.
    #[test]
    fn kimi_opens_and_reopens_with_the_same_flag() {
        let id = "1bb18ec4-6ddb-474b-96c3-b55c73a027a8";
        assert_eq!(open_args("kimi", id, &[]), vec!["--session", id]);
        assert_eq!(resume_args("kimi", id, &[]), vec!["--session", id]);
        let home = Path::new("/home/u");
        let a = conversation_path(home, "kimi", Path::new("/Users/x/proj"), id).unwrap();
        let b = conversation_path(home, "kimi", Path::new("/Users/x/other"), id).unwrap();
        assert_ne!(a, b);
        assert!(a.ends_with(Path::new(id).join("wire.jsonl")), "{}", a.display());
        assert!(a.starts_with("/home/u/.kimi/sessions"), "{}", a.display());
        // The wire file appears with the first turn, so existing is enough.
        assert_eq!(turn_marker("kimi"), Turn::Exists);
    }

    /// The hash is the one the CLIs compute, checked against a chat cursor
    /// itself filed under this directory. The worktree path is synthetic on
    /// purpose: the digest is coupled to the exact string, so a real home
    /// directory here fails the moment anything rewrites that path, and it
    /// fails as a bare digest mismatch that says nothing about the cause.
    #[test]
    fn store_dir_is_the_md5_of_the_worktree_path() {
        let home = Path::new("/home/u");
        let wt = Path::new("/Users/x/agency/.agency/worktrees/agent-mm1d");
        assert_eq!(
            store_dir(home, "cursor-agent", wt).unwrap(),
            Path::new("/home/u/.cursor/chats/4bfe083e18e07a7bf883f1f391d9c337")
        );
        assert_eq!(
            store_dir(home, "kimi", wt).unwrap(),
            Path::new("/home/u/.kimi/sessions/4bfe083e18e07a7bf883f1f391d9c337")
        );
        // Not resolved, not normalised: the string as given is what is hashed.
        assert_ne!(
            store_dir(home, "kimi", wt),
            store_dir(home, "kimi", Path::new("/Users/x/agency/.agency/worktrees/agent-mm1d/"))
        );
    }

    /// The two `meta.json` shapes seen in a pty: straight after launch, and
    /// after the first turn. Only the second is a conversation to reopen.
    #[test]
    fn cursor_meta_records_a_turn_only_once_it_says_so() {
        let launched = r#"{"schemaVersion":1,"createdAtMs":1788619512182,"hasConversation":false,"updatedAtMs":1788619512736,"cwd":"/Users/x/proj"}"#;
        let answered = r#"{"schemaVersion":1,"createdAtMs":1788619512182,"hasConversation":true,"title":"Kiwi Only","updatedAtMs":1788619525193,"cwd":"/Users/x/proj"}"#;
        assert!(!cursor_meta_records_turn(launched));
        assert!(cursor_meta_records_turn(answered));
        // Half-written or missing: no turn, and the launch starts fresh.
        assert!(!cursor_meta_records_turn(""));
        assert!(!cursor_meta_records_turn("{\"schemaVersion\":1"));
        assert!(!cursor_meta_records_turn("{}"));
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
        // Cursor refuses `--new-session-id` beside `--continue` outright, so
        // a profile carrying either direction stands Agency's down.
        for theirs in [
            vec!["--continue".to_string()],
            vec!["--resume".to_string(), "other".to_string()],
            vec!["--new-session-id=other".to_string()],
        ] {
            assert!(open_args("cursor-agent", id, &theirs).is_empty(), "{theirs:?}");
            assert!(resume_args("cursor-agent", id, &theirs).is_empty(), "{theirs:?}");
        }
        for theirs in [
            vec!["-S".to_string(), "other".to_string()],
            vec!["--session=other".to_string()],
            vec!["-C".to_string()],
        ] {
            assert!(open_args("kimi", id, &theirs).is_empty(), "{theirs:?}");
            assert!(resume_args("kimi", id, &theirs).is_empty(), "{theirs:?}");
        }
        // The short flags are per agent: kimi's `-c` is its prompt, not a
        // conversation, so it does not stand anything down.
        assert!(!open_args("kimi", id, &["-c".to_string(), "hi".to_string()]).is_empty());
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
