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
//! - **A conversation of its own** ([`Pin::Id`], claude, copilot, cursor, kimi
//!   and gemini). The CLI names conversations, so Agency mints an id, opens
//!   the conversation under it, records it against the session, and later
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
//!
//! Gemini 0.64.0 (AGE-192), the same way: no credential here either, so it
//! was pointed at a stand-in for the Gemini API through its own
//! `GOOGLE_GEMINI_BASE_URL`, replying with every "remember the word X" in the
//! request. In a pty under tmux, two conversations opened in one directory
//! under ids sharing their first 8 characters, kiwi and mango; `--resume
//! <first>` with a typed follow-up sent kiwi and the follow-up and nothing of
//! mango, and `--resume <second>` the reverse. Where it keeps them is on
//! [`gemini_chats_dir`], because no one path will do.

use md5::{Digest, Md5};
use std::collections::HashMap;
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
/// Hermes stays absent too, after AGE-190 installed and read all four agents
/// that had no resume recipe (cursor and kimi are in [`id_flags`] now, and
/// gemini followed once a turn could be driven, AGE-192):
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
///
/// Gemini has claude's shape, driven in 0.64.0. `--session-id <id>` "Start a
/// new session with a manually provided UUID", and an id already in this
/// project is refused with "Session ID ... already exists. Use --resume to
/// resume it" and exit 42. `--resume <id>` reopened the exact session; on an
/// id it has no turn for, it says "Invalid session identifier" and exits 42.
/// Timed in a pty, three runs each: 1.1-1.4 s with other sessions in the
/// project and 1.2-1.8 s with none, before the folder-trust dialog it shows a
/// fresh launch. So unlike claude a wrong "Has" does not hang the pane: it
/// exits inside the daemon's 3 s early-exit grace with no keystroke seen,
/// and the fresh fallback takes over.
/// `--session-file` loads a conversation from a file of the user's, which
/// names one too.
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
        "gemini" => Some(IdFlags {
            open: "--session-id",
            resume: "--resume",
            theirs: &["--session-id", "--resume", "-r", "--session-file"],
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
/// mints), kimi's own ids are UUIDs, and gemini's "manually provided UUID".
/// Never reused, minted per fresh launch: a rerun is a new conversation by
/// definition, and claude and cursor refuse an id already in use.
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
///
/// `None` for gemini as well, which names conversations but files them where
/// no path computed from the id alone can find them: [`gemini_chats_dir`].
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

/// Gemini's project registry, for `command` gemini, which the caller reads and
/// hands to [`gemini_chats_dir`]. `None` for everyone else.
///
/// The root can be moved by `GEMINI_CLI_HOME`, and gemini's own macOS sandbox
/// keeps everything under `~/.cache/.gemini` instead. Neither is followed, as
/// with [`store_dir`]: a store this cannot see is a probe answering "None" and
/// a fresh launch, not a wrong resume.
pub fn gemini_registry(home: &Path, command: &str) -> Option<PathBuf> {
    (base(command) == "gemini").then(|| home.join(".gemini").join("projects.json"))
}

/// The directory gemini keeps every conversation from `worktree` in, given
/// the contents of [`gemini_registry`].
///
/// Gemini files conversations under a short name per project, not under the
/// path or a hash of it: `~/.gemini/tmp/<slug>/chats/`, where `<slug>` is
/// what `projects.json` maps the launch directory to. 0.64.0 claims the
/// directory's basename and counts up on a clash (`wt`, then `wt-1` for
/// another directory of the same name; 0.58.0 handed out `a` in a fresh home),
/// so it has to be looked up, never computed. The key is the directory gemini
/// was started in, not its git root (a launch from a repo's subdirectory
/// registered the subdirectory), and with its symlinks resolved, since it is
/// `process.cwd()`: started from `/tmp/x`, 0.64.0 registered `/private/tmp/x`.
/// The caller passes `worktree` resolved the same way.
///
/// The slug is held to gemini's own registry schema (`^[a-z0-9-]+$`), which
/// also keeps it a single path component.
pub fn gemini_chats_dir(home: &Path, registry: &str, worktree: &Path) -> Option<PathBuf> {
    let registry: serde_json::Value = serde_json::from_str(registry).ok()?;
    let slug = registry.get("projects")?.get(worktree.to_str()?)?.as_str()?;
    let ok = !slug.is_empty()
        && slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    ok.then(|| home.join(".gemini").join("tmp").join(slug).join("chats"))
}

/// Whether `file_name`, in [`gemini_chats_dir`], may hold `conversation`, or
/// with `None` any conversation at all.
///
/// The file is `session-<YYYY-MM-DDTHH-MM>-<first 8 of id>.jsonl`, named for
/// the minute it was opened, so only the tail can be matched. And the tail is
/// not unique: two ids opened in one minute with the same first 8 characters
/// were filed as `session-2026-10-19T14-03-0b6f1a52.jsonl` and
/// `session-2026-10-19T14-03-1-0b6f1a52.jsonl`. So this narrows, and
/// [`gemini_chat_records_turn`] decides by the id written inside, which is
/// what gemini's own lookup does. A `.json` tail too: 0.64.0 still lists the
/// whole-file records older versions wrote.
pub fn gemini_chat_file(file_name: &str, conversation: Option<&str>) -> bool {
    if !file_name.starts_with("session-") {
        return false;
    }
    match conversation {
        Some(c) => {
            let short = c.get(..8).unwrap_or(c);
            file_name.ends_with(&format!("-{short}.jsonl"))
                || file_name.ends_with(&format!("-{short}.json"))
        }
        None => file_name.ends_with(".jsonl") || file_name.ends_with(".json"),
    }
}

/// Whether a gemini chat file, given as `contents`, is `conversation` (or with
/// `None`, any conversation) and has a turn in it: gemini's own rule for what
/// `--resume` will reopen, so that "Has" means it will.
///
/// The file is written at launch, before any turn, and a line count says
/// nothing. 0.64.0 opened a promptless session with three lines: the metadata
/// (`sessionId`, `projectHash`, `kind`), a `"type":"user"` message that is
/// gemini's own `<session_context>` preamble, and a `$set`. `--resume` on that
/// id answered "Invalid session identifier" and exit 42, and a clean exit
/// deleted the file. (0.58.0 wrote the metadata line alone, which is why a
/// second line looked like the answer before a turn was driven.) The first
/// turn appends the prompt as a `"type":"user"` message and the reply as a
/// `"type":"gemini"` one.
///
/// This is a port of the code 0.64.0 runs for both `--resume <id>` and
/// `--resume latest`, which list the project's sessions and drop any without
/// resumable content or of `kind` `subagent` (`getAllSessionFiles` in
/// `packages/cli/src/utils/sessionUtils.ts`). The listing reads each file with
/// `createJsonlRecordAccumulator` in metadata-only mode, falling back to
/// `parseLegacyContentFallback` for a whole-file JSON record, and counts a
/// message with `isResumableMessageRecord` (all three in
/// `packages/core/src/services/chatRecordingService.ts`). The first port
/// counted only the message lines and skipped every `$patch`. Read against
/// the 0.64.0 bundle, a patch can fill in a reply that was recorded empty,
/// rewrite a message into a slash command, remove messages or reorder them
/// ahead of a rewind, and a `$set` can replace the message list outright, so
/// each was an answer that could disagree with gemini's. Run beside those
/// three functions lifted out of the bundle, on 20,000 generated chat files,
/// the first port disagreed on 997 (414 a wrong "Has", 583 a wrong "None")
/// and this one on none. The comparison is only as good as the version it
/// was read from: re-read those functions when the CLI moves past 0.64.0.
pub fn gemini_chat_records_turn(contents: &str, conversation: Option<&str>) -> bool {
    let Some(chat) = gemini_jsonl_chat(contents).or_else(|| gemini_legacy_chat(contents)) else {
        return false;
    };
    let ours = match conversation {
        Some(c) => chat.session.as_str() == Some(c),
        None => true,
    };
    ours && !chat.subagent && chat.resumable
}

/// What gemini's listing keeps of one chat file.
struct GeminiChat {
    session: serde_json::Value,
    subagent: bool,
    resumable: bool,
}

/// `createJsonlRecordAccumulator` in metadata-only mode: each record kind is
/// tried in gemini's order, and the file is a session only once the metadata
/// carries both a `sessionId` and a `projectHash`. Unparseable lines are
/// skipped, as gemini skips them.
fn gemini_jsonl_chat(contents: &str) -> Option<GeminiChat> {
    use serde_json::{Map, Value};
    let mut meta = Map::new();
    // `messageIds` and `messageKinds`: the order a rewind cuts along, and
    // whether each message is the user's and counts as a turn.
    let mut order: Vec<String> = Vec::new();
    let mut kinds: HashMap<String, (bool, bool)> = HashMap::new();
    // A message list carried whole, by `$set` or the metadata line: every
    // entry appended, repeats included, as gemini appends them.
    let push = |order: &mut Vec<String>, kinds: &mut HashMap<String, (bool, bool)>, m: &Value| {
        if let Some(id) = gemini_message_id(m) {
            order.push(id.to_string());
            kinds.insert(id.to_string(), gemini_message_kind(m));
        }
    };
    for line in contents.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else { continue };
        if let Some(to) = record.get("$rewindTo").and_then(Value::as_str) {
            match order.iter().position(|id| id == to) {
                Some(i) => {
                    for id in order.split_off(i) {
                        kinds.remove(&id);
                    }
                }
                None => {
                    order.clear();
                    kinds.clear();
                }
            }
        } else if let Some(patch) = gemini_object(&record, "$patch") {
            if patch.get("id").and_then(Value::as_str).is_some() {
                gemini_patch(&mut kinds, patch);
            }
            for update in patch.get("updates").and_then(Value::as_array).into_iter().flatten() {
                if update.get("id").and_then(Value::as_str).is_some() {
                    gemini_patch(&mut kinds, update);
                }
            }
            let removed = patch.get("removeIds").and_then(Value::as_array);
            for id in removed.into_iter().flatten().filter_map(Value::as_str) {
                if let Some(i) = order.iter().position(|o| o == id) {
                    order.remove(i);
                }
                kinds.remove(id);
            }
            if let Some(ids) = patch.get("orderIds").and_then(Value::as_array) {
                let ordered: Vec<String> = ids
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|id| kinds.contains_key(*id))
                    .map(String::from)
                    .collect();
                order.retain(|id| !ordered.contains(id));
                order.extend(ordered);
            }
        } else if let Some(id) = gemini_message_id(&record) {
            if !kinds.contains_key(id) {
                order.push(id.to_string());
            }
            kinds.insert(id.to_string(), gemini_message_kind(&record));
        } else if let Some(set) = gemini_object(&record, "$set") {
            if let Some(messages) = set.get("messages").and_then(Value::as_array) {
                order.clear();
                kinds.clear();
                for m in messages {
                    push(&mut order, &mut kinds, m);
                }
            }
            if let Some(set) = set.as_object() {
                meta.extend(set.clone());
            }
        } else if record.get("sessionId").and_then(Value::as_str).is_some()
            && record.get("projectHash").and_then(Value::as_str).is_some()
        {
            if let Some(fields) = record.as_object() {
                meta.extend(fields.clone());
            }
            for m in record.get("messages").and_then(Value::as_array).into_iter().flatten() {
                push(&mut order, &mut kinds, m);
            }
        }
    }
    let session = meta.get("sessionId").cloned().unwrap_or(Value::Null);
    if !gemini_truthy(&session) || !meta.get("projectHash").is_some_and(gemini_truthy) {
        return None;
    }
    Some(GeminiChat {
        session,
        subagent: meta.get("kind").and_then(Value::as_str) == Some("subagent"),
        resumable: kinds.values().any(|&(_, turn)| turn),
    })
}

/// `parseLegacyContentFallback`: the whole file as one JSON record with its
/// messages inline, for files the JSONL pass found no session in.
fn gemini_legacy_chat(contents: &str) -> Option<GeminiChat> {
    use serde_json::Value;
    let record: Value = serde_json::from_str(contents).ok()?;
    let session = record.get("sessionId")?.clone();
    if !gemini_truthy(&session) {
        return None;
    }
    let resumable = match record.get("messages") {
        None | Some(Value::Null) => false,
        Some(Value::Array(messages)) => messages.iter().any(|m| gemini_message_kind(m).1),
        // `messages?.some` on anything else throws, and the file is dropped.
        Some(_) => return None,
    };
    Some(GeminiChat {
        session,
        subagent: record.get("kind").and_then(Value::as_str) == Some("subagent"),
        resumable,
    })
}

/// `isMessageRecord`: a string `id`, and not a patch.
fn gemini_message_id(record: &serde_json::Value) -> Option<&str> {
    record.get("id").and_then(serde_json::Value::as_str).filter(|_| record.get("$patch").is_none())
}

/// `isObjectProperty`, which takes an array as readily as an object.
fn gemini_object<'a>(record: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    record.get(key).filter(|v| v.is_object() || v.is_array())
}

/// JavaScript truthiness, for the values gemini tests with `!`.
fn gemini_truthy(v: &serde_json::Value) -> bool {
    use serde_json::Value;
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `applySinglePatch` in metadata-only mode: new content re-decides a user
/// message either way, but can only ever make a reply count.
fn gemini_patch(kinds: &mut HashMap<String, (bool, bool)>, patch: &serde_json::Value) {
    let (Some(id), Some(content)) =
        (patch.get("id").and_then(serde_json::Value::as_str), patch.get("content"))
    else {
        return;
    };
    let Some((user, turn)) = kinds.get_mut(id) else { return };
    let text = gemini_part_text(content);
    let text = text.trim();
    if *user {
        *turn = !gemini_ignored_user_text(text);
    } else if !text.is_empty() {
        *turn = true;
    }
}

/// Whether a message is the user's, and `isResumableMessageRecord`: a user
/// message counts unless gemini ignores its text, and a reply counts if it
/// says anything, calls a tool or thinks. Every other type (`info`, `error`,
/// `warning`) never does.
fn gemini_message_kind(message: &serde_json::Value) -> (bool, bool) {
    use serde_json::Value;
    let text = message.get("content").map(gemini_part_text).unwrap_or_default();
    let text = text.trim();
    let any = |key: &str| message.get(key).and_then(Value::as_array).is_some_and(|a| !a.is_empty());
    match message.get("type").and_then(Value::as_str) {
        Some("user") => (true, !gemini_ignored_user_text(text)),
        Some("gemini") => (false, !text.is_empty() || any("toolCalls") || any("thoughts")),
        _ => (false, false),
    }
}

/// `isIgnoredUserContent`, on trimmed text: slash commands, help requests and
/// gemini's own injected context.
fn gemini_ignored_user_text(text: &str) -> bool {
    text.is_empty()
        || text.starts_with('/')
        || text.starts_with('?')
        || text.starts_with("<session_context>")
        || text.starts_with("<hook_context>")
}

/// `partListUnionToString`, verbose: a string, a part, or a list of either,
/// where a part that is not text stands in as a bracketed label. Only whether
/// the result is empty and how it starts are ever read, so the labels here
/// keep gemini's leading `[` and not its exact wording.
fn gemini_part_text(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().map(gemini_part_text).collect(),
        Value::Object(part) => {
            const LABELLED: &[&str] = &[
                "videoMetadata",
                "thought",
                "codeExecutionResult",
                "executableCode",
                "fileData",
                "functionCall",
                "functionResponse",
                "inlineData",
            ];
            if LABELLED.iter().any(|k| part.contains_key(*k)) {
                "[part]".into()
            } else {
                part.get("text").and_then(Value::as_str).unwrap_or_default().to_string()
            }
        }
        _ => String::new(),
    }
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
///
/// The basename as written, and nothing is resolved here: a CLI installed
/// under a second name (cursor ships `agent` beside `cursor-agent`) is mapped
/// back to the name these recipes are keyed by before it arrives, by
/// `agent_catalog::recipe_command`. AGE-197 is what that costs when it does
/// not happen.
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
        // All three are here having been asked, not assumed: codex will not be
        // told a thread id, opencode refuses one it has not seen, and hermes
        // mints its own and keys `--continue` globally (see `pin`).
        for command in ["codex", "opencode", "hermes"] {
            assert_eq!(pin(command), Pin::None, "{command}");
            assert_eq!(dir(home, command, wt, "agent-3f9c"), None, "{command}");
            assert!(env(home, command, wt, "agent-3f9c").is_empty(), "{command}");
            assert_eq!(mint(command), None, "{command}");
            assert!(open_args(command, "x", &[]).is_empty(), "{command}");
            assert!(resume_args(command, "x", &[]).is_empty(), "{command}");
            assert_eq!(conversation_path(home, command, wt, "x"), None, "{command}");
            assert_eq!(store_dir(home, command, wt), None, "{command}");
            assert_eq!(gemini_registry(home, command), None, "{command}");
        }
    }

    /// Every agent [`pin`] calls [`Pin::Id`] has to have both halves of the
    /// recipe, or it would launch with the generic "most recent here" one and
    /// this module would report a pin it is not applying. And somewhere the
    /// probe can look for the conversation, or a resume could never be chosen.
    #[test]
    fn every_named_conversation_agent_can_be_opened_and_reopened() {
        let home = Path::new("/home/u");
        let wt = Path::new("/Users/x/proj");
        let id = "9674f5a1-334c-49a5-9952-89e592b0bc5b";
        for command in ["claude", "copilot", "cursor-agent", "kimi", "gemini"] {
            assert_eq!(pin(command), Pin::Id, "{command}");
            assert!(mint(command).is_some(), "{command}");
            assert!(!open_args(command, id, &[]).is_empty(), "{command}");
            assert!(!resume_args(command, id, &[]).is_empty(), "{command}");
            assert!(
                conversation_path(home, command, wt, id).is_some()
                    || gemini_registry(home, command).is_some(),
                "{command}"
            );
        }
    }

    /// Claude's split, flag for flag: `--session-id` opens and refuses an id
    /// in use, `--resume` reopens and refuses one with no turn.
    #[test]
    fn gemini_opens_with_session_id_and_reopens_with_resume() {
        let id = "0b6f1a52-9c1e-4f6e-a1d2-3c4b5a697801";
        assert_eq!(open_args("gemini", id, &[]), vec!["--session-id", id]);
        assert_eq!(resume_args("gemini", id, &[]), vec!["--resume", id]);
        assert_eq!(open_args("/opt/homebrew/bin/gemini", id, &[]), vec!["--session-id", id]);
        // No single file to name; the probe goes through the registry.
        let home = Path::new("/home/u");
        assert_eq!(conversation_path(home, "gemini", Path::new("/w"), id), None);
        assert_eq!(store_dir(home, "gemini", Path::new("/w")), None);
        assert_eq!(
            gemini_registry(home, "gemini").unwrap(),
            Path::new("/home/u/.gemini/projects.json")
        );
        for theirs in [
            vec!["--resume".to_string(), "latest".to_string()],
            vec!["-r".to_string(), "3".to_string()],
            vec!["--session-file=/tmp/s.json".to_string()],
        ] {
            assert!(open_args("gemini", id, &theirs).is_empty(), "{theirs:?}");
            assert!(resume_args("gemini", id, &theirs).is_empty(), "{theirs:?}");
        }
    }

    /// The registry as 0.64.0 wrote it for two directories: the slug is
    /// looked up by the exact launch directory, and nothing else matches.
    #[test]
    fn gemini_chats_dir_is_looked_up_in_the_registry() {
        let home = Path::new("/home/u");
        let registry = r#"{
  "projects": {
    "/Users/x/agency/.agency/worktrees/wt": "wt",
    "/Users/y/wt": "wt-1"
  }
}"#;
        assert_eq!(
            gemini_chats_dir(home, registry, Path::new("/Users/x/agency/.agency/worktrees/wt"))
                .unwrap(),
            Path::new("/home/u/.gemini/tmp/wt/chats")
        );
        assert_eq!(
            gemini_chats_dir(home, registry, Path::new("/Users/y/wt")).unwrap(),
            Path::new("/home/u/.gemini/tmp/wt-1/chats")
        );
        // Never launched there: no entry, no directory to guess at.
        assert_eq!(gemini_chats_dir(home, registry, Path::new("/Users/z/wt")), None);
        assert_eq!(gemini_chats_dir(home, registry, Path::new("/Users/y/wt/")), None);
        assert_eq!(gemini_chats_dir(home, "", Path::new("/Users/y/wt")), None);
        assert_eq!(gemini_chats_dir(home, "{}", Path::new("/Users/y/wt")), None);
        // A slug outside gemini's own schema is not a path component to join.
        for bad in ["..", "a/b", "", "A"] {
            let registry = format!(r#"{{"projects":{{"/w":"{bad}"}}}}"#);
            assert_eq!(gemini_chats_dir(home, &registry, Path::new("/w")), None, "{bad:?}");
        }
    }

    /// File names as 0.64.0 wrote them, including the counter it inserts when
    /// two ids opened in one minute share their first 8 characters.
    #[test]
    fn gemini_chat_files_are_matched_on_the_short_id() {
        let id = "0b6f1a52-9c1e-4f6e-a1d2-3c4b5a697801";
        assert!(gemini_chat_file("session-2026-10-19T14-03-0b6f1a52.jsonl", Some(id)));
        assert!(gemini_chat_file("session-2026-10-19T14-03-1-0b6f1a52.jsonl", Some(id)));
        assert!(!gemini_chat_file("session-2026-10-19T14-03-7d1e2c3b.jsonl", Some(id)));
        assert!(!gemini_chat_file("0b6f1a52.jsonl", Some(id)));
        assert!(!gemini_chat_file("session-2026-10-19T14-03-0b6f1a52.jsonl.tmp", Some(id)));
        assert!(gemini_chat_file("session-2026-10-19T14-03-7d1e2c3b.jsonl", None));
        assert!(!gemini_chat_file(".project_root", None));
        // The whole-file records older versions wrote, which 0.64.0 still lists.
        assert!(gemini_chat_file("session-2026-10-19T14-03-0b6f1a52.json", Some(id)));
        assert!(gemini_chat_file("session-2026-10-19T14-03-7d1e2c3b.json", None));
        assert!(!gemini_chat_file("session-2026-10-19T14-03-7d1e2c3b.json", Some(id)));
    }

    /// Lines as 0.64.0 wrote them: at launch, and after the first turn.
    const GEMINI_LAUNCHED: &str = r#"{"sessionId":"0b6f1a52-9c1e-4f6e-a1d2-3c4b5a697801","projectHash":"8457e373","startTime":"2026-10-19T14:03:12.335Z","lastUpdated":"2026-10-19T14:03:12.335Z","kind":"main"}
{"id":"d04923d38bb0f6017037e74183378ef4","timestamp":"2026-10-19T14:03:12.336Z","type":"user","content":[{"text":"<session_context>\nThis is the Gemini CLI. We are setting up the context for our chat.\n</session_context>"}]}
{"$set":{"lastUpdated":"2026-10-19T14:03:12.336Z"}}
"#;
    const GEMINI_TURN: &str = r#"{"id":"da565697-69b0-4889-92c5-4cbd20aef517","timestamp":"2026-10-19T14:03:13.092Z","type":"user","content":[{"text":"Please remember the word kiwi"}]}
{"$set":{"lastUpdated":"2026-10-19T14:03:13.093Z"}}
{"id":"18bb8465-b505-4c30-beb2-b2b54319a092","timestamp":"2026-10-19T14:03:13.103Z","type":"gemini","content":"WORDS=kiwi","thoughts":[],"tokens":{"total":13},"model":"gemini-3.1-pro-preview"}
"#;

    #[test]
    fn gemini_chat_records_a_turn_only_once_one_is_appended() {
        let id = "0b6f1a52-9c1e-4f6e-a1d2-3c4b5a697801";
        // Two lines and a user message already, and still nothing to reopen:
        // the message is gemini's own preamble.
        assert!(!gemini_chat_records_turn(GEMINI_LAUNCHED, Some(id)));
        assert!(!gemini_chat_records_turn(GEMINI_LAUNCHED, None));
        let answered = format!("{GEMINI_LAUNCHED}{GEMINI_TURN}");
        assert!(gemini_chat_records_turn(&answered, Some(id)));
        assert!(gemini_chat_records_turn(&answered, None));
        // The prompt alone is enough: gemini reopens a turn it never
        // answered, with the prompt in it.
        let asked = format!("{GEMINI_LAUNCHED}{}\n", GEMINI_TURN.lines().next().unwrap());
        assert!(gemini_chat_records_turn(&asked, Some(id)));
        // Same short id, other conversation: the id inside decides.
        assert!(!gemini_chat_records_turn(&answered, Some("0b6f1a52-9c1e-4f6e-a1d2-3c4b5a697802")));
        // Half-written, empty or not gemini's at all: no turn.
        assert!(!gemini_chat_records_turn("", Some(id)));
        assert!(!gemini_chat_records_turn("{\"sessionId\":", Some(id)));
        assert!(!gemini_chat_records_turn(GEMINI_TURN, Some(id)), "no metadata, no session");
    }

    #[test]
    fn gemini_chat_follows_what_gemini_counts_as_a_turn() {
        let id = "0b6f1a52-9c1e-4f6e-a1d2-3c4b5a697801";
        let with = |line: &str| format!("{GEMINI_LAUNCHED}{line}\n");
        // A slash command or a help request is not a conversation.
        for text in ["/help", "?", "   ", "<hook_context>x</hook_context>"] {
            let line = serde_json::json!({"id": "m1", "type": "user", "content": [{"text": text}]});
            assert!(!gemini_chat_records_turn(&with(&line.to_string()), Some(id)), "{text:?}");
        }
        // A reply counts by itself, and so does a tool call with no text.
        assert!(gemini_chat_records_turn(
            &with(r#"{"id":"m1","type":"gemini","content":"hi"}"#),
            Some(id)
        ));
        assert!(gemini_chat_records_turn(
            &with(r#"{"id":"m1","type":"gemini","content":"","toolCalls":[{"name":"ls"}]}"#),
            Some(id)
        ));
        assert!(!gemini_chat_records_turn(
            &with(r#"{"id":"m1","type":"gemini","content":"","toolCalls":[]}"#),
            Some(id)
        ));
        // Info lines are not turns.
        assert!(!gemini_chat_records_turn(
            &with(r#"{"id":"m1","type":"info","content":"Resumed"}"#),
            Some(id)
        ));
        // Rewound to before the first turn: nothing left to reopen.
        let rewound = format!(
            "{GEMINI_LAUNCHED}{GEMINI_TURN}{}\n",
            r#"{"$rewindTo":"da565697-69b0-4889-92c5-4cbd20aef517"}"#
        );
        assert!(!gemini_chat_records_turn(&rewound, Some(id)));
        // Rewound to the reply only: the prompt stays.
        let partly = format!(
            "{GEMINI_LAUNCHED}{GEMINI_TURN}{}\n",
            r#"{"$rewindTo":"18bb8465-b505-4c30-beb2-b2b54319a092"}"#
        );
        assert!(gemini_chat_records_turn(&partly, Some(id)));
        // A subagent's transcript is never what `--resume` opens.
        let subagent =
            format!("{GEMINI_LAUNCHED}{GEMINI_TURN}").replace("\"main\"", "\"subagent\"");
        assert!(!gemini_chat_records_turn(&subagent, Some(id)));
    }

    /// The records the first port skipped, each of which 0.64.0's accumulator
    /// applies: a patch can make a reply count or a prompt stop counting, take
    /// messages out, or reorder them ahead of a rewind, and a `$set` can
    /// replace the list.
    #[test]
    fn gemini_chat_applies_patches_and_message_sets() {
        let id = "0b6f1a52-9c1e-4f6e-a1d2-3c4b5a697801";
        let turn = |lines: &[&str]| {
            gemini_chat_records_turn(&format!("{GEMINI_LAUNCHED}{}\n", lines.join("\n")), Some(id))
        };
        // A reply recorded empty and filled in later, directly or in a list
        // of updates; a later empty patch does not take it back.
        let reply = r#"{"id":"r","type":"gemini","content":""}"#;
        assert!(!turn(&[reply]));
        assert!(turn(&[reply, r#"{"$patch":{"id":"r","content":"hi"}}"#]));
        assert!(turn(&[
            reply,
            r#"{"$patch":{"updates":[{"id":"r","content":[{"text":"hi"}]}]}}"#,
            r#"{"$patch":{"id":"r","content":""}}"#,
        ]));
        // A patch for a message not recorded yet is dropped.
        assert!(!turn(&[r#"{"$patch":{"id":"r","content":"hi"}}"#, reply]));
        // A prompt is re-decided either way.
        let prompt = r#"{"id":"u","type":"user","content":[{"text":"kiwi"}]}"#;
        assert!(!turn(&[prompt, r#"{"$patch":{"id":"u","content":"/help"}}"#]));
        assert!(turn(&[
            r#"{"id":"u","type":"user","content":"/help"}"#,
            r#"{"$patch":{"id":"u","content":"kiwi"}}"#,
        ]));
        // So is a message recorded twice under one id.
        assert!(!turn(&[prompt, r#"{"id":"u","type":"user","content":"/help"}"#]));
        assert!(!turn(&[prompt, r#"{"$patch":{"removeIds":["u"]}}"#]));
        // A rewind cuts along gemini's order, which `orderIds` rearranges by
        // moving the listed messages to the end. Rewound to the preamble, the
        // prompt goes with it, unless the preamble was moved behind it.
        let preamble = "d04923d38bb0f6017037e74183378ef4";
        let rewind = format!(r#"{{"$rewindTo":"{preamble}"}}"#);
        assert!(!turn(&[prompt, &rewind]));
        let reorder = format!(r#"{{"$patch":{{"orderIds":["{preamble}"]}}}}"#);
        assert!(turn(&[prompt, &reorder, &rewind]));
        // A `$set` of the message list replaces it outright.
        assert!(!turn(&[prompt, r#"{"$set":{"messages":[]}}"#]));
        assert!(turn(&[r#"{"$set":{"messages":[{"id":"u","type":"user","content":"kiwi"}]}}"#]));
        // And a `$set` of the kind is merged like any other metadata.
        assert!(!turn(&[prompt, r#"{"$set":{"kind":"subagent"}}"#]));
    }

    /// What counts as content, and which records are sessions at all.
    #[test]
    fn gemini_chat_reads_content_and_records_as_gemini_does() {
        let id = "0b6f1a52-9c1e-4f6e-a1d2-3c4b5a697801";
        let with = |line: &str| format!("{GEMINI_LAUNCHED}{line}\n");
        // A part that is not text is content: gemini labels it, so an image
        // alone is a prompt. A single part needs no list around it.
        for content in [
            serde_json::json!([{"inlineData": {"mimeType": "image/png", "data": "iVBO"}}]),
            serde_json::json!([{"functionResponse": {"name": "ls"}}]),
            serde_json::json!({"text": "kiwi"}),
        ] {
            let line = serde_json::json!({"id": "u", "type": "user", "content": content});
            assert!(gemini_chat_records_turn(&with(&line.to_string()), Some(id)), "{content}");
        }
        // The metadata line is one only with a `projectHash` beside the id.
        let prompt = r#"{"id":"u","type":"user","content":"kiwi"}"#;
        let no_hash = format!("{{\"sessionId\":\"{id}\",\"kind\":\"main\"}}\n{prompt}\n");
        assert!(!gemini_chat_records_turn(&no_hash, Some(id)));
        // The whole-file record older versions wrote, read when no line is a
        // session: pretty-printed, and with no `projectHash` needed.
        let legacy = |record: serde_json::Value| serde_json::to_string_pretty(&record).unwrap();
        let messages = serde_json::json!([{"id": "u", "type": "user", "content": "kiwi"}]);
        let record = legacy(serde_json::json!({"sessionId": id, "messages": messages}));
        assert!(gemini_chat_records_turn(&record, Some(id)));
        assert!(gemini_chat_records_turn(&record, None));
        assert!(!gemini_chat_records_turn(&record, Some("0b6f1a52-9c1e-4f6e-a1d2-3c4b5a697802")));
        let subagent =
            legacy(serde_json::json!({"sessionId": id, "kind": "subagent", "messages": messages}));
        assert!(!gemini_chat_records_turn(&subagent, Some(id)));
        let unanswered = legacy(serde_json::json!({"sessionId": id, "messages": []}));
        assert!(!gemini_chat_records_turn(&unanswered, Some(id)));
        let malformed = legacy(serde_json::json!({"sessionId": id, "messages": {"u": "kiwi"}}));
        assert!(!gemini_chat_records_turn(&malformed, Some(id)));
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

        for named in ["claude", "copilot", "cursor-agent", "kimi", "gemini"] {
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
