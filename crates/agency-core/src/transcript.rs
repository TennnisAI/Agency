//! A run's conversation, read from the agent's own transcript, and the move
//! that keeps that transcript when the run's worktree goes.
//!
//! The transcript directories `usage.rs::session_dir` resolves are keyed by
//! worktree path. Archiving or deleting a run removes that path, which used to
//! orphan the directory forever: nothing ever swept them, and nothing could
//! read them back (AGE-152). This module is both halves of the fix: a
//! verified move that carries the directory into the archive (or back out of
//! it on restore), and a parser that renders the two transcript dialects we
//! actually know as a read-only conversation.
//!
//! The dialect list is [`crate::usage::format_for`]'s, deliberately: `claude`
//! and `pi`, the two whose files have been read for real. Every parse rule
//! here was checked against live transcripts, not documentation — the sample
//! sizes are in the comments. For any other agent the caller must say "we
//! cannot see this agent's transcript", never "this agent said nothing"; that
//! is the same distinction `usage.rs` keeps for tokens.

use crate::usage::Format;
use anyhow::{bail, Context, Result};
use std::path::Path;

/// Who a rendered line belongs to. `Tool` lines are the agent acting rather
/// than speaking: one per tool call, so the conversation keeps its shape
/// without carrying megabytes of tool output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Turn {
    pub role: Role,
    pub text: String,
}

/// One session file, parsed. A worktree's directory usually holds one, but
/// extra agent tabs and `/clear` each start another (66 of 400 real claude
/// directories held more than one file).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Conversation {
    /// The agent's own name for the session, when it recorded one
    /// (claude's `ai-title` records).
    pub title: Option<String>,
    /// First timestamp in the file, as the agent wrote it (RFC 3339).
    pub started: Option<String>,
    pub turns: Vec<Turn>,
}

/// A pasted log or a huge prompt must not make one turn unrenderable; the
/// full text stays in the session file the archive keeps.
const TURN_CHARS: usize = 8_000;

/// Tool lines are one-liners by contract.
const TOOL_CHARS: usize = 160;

pub fn parse_conversation(text: &str, format: Format) -> Conversation {
    match format {
        Format::Claude => parse_claude(text),
        Format::Pi => parse_pi(text),
    }
}

/// The spoken text of a `message.content` value: the string itself, or the
/// concatenated `text` blocks of an array. Tool blocks contribute nothing —
/// tool results ride in user-role records in both dialects, and rendering
/// them as things the user said would misattribute most of the file.
fn text_of(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.trim().to_string(),
        serde_json::Value::Array(blocks) => {
            let texts: Vec<&str> = blocks
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect();
            texts.join("\n\n").trim().to_string()
        }
        _ => String::new(),
    }
}

/// Truncate on a char boundary, saying so. The count is chars rather than
/// bytes so a multi-byte boundary cannot panic the slice.
fn clip(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let cut: String = s.chars().take(max_chars).collect();
    format!("{} … (truncated)", cut.trim_end())
}

/// Append assistant prose, merging into a preceding assistant turn. Claude
/// streams one content block per record (6,057 messages over 120 real files:
/// every extra record for a message id carried only new blocks), so one spoken
/// turn arrives as several records and reads as one only if merged.
fn push_assistant_text(turns: &mut Vec<Turn>, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if let Some(last) = turns.last_mut() {
        if last.role == Role::Assistant {
            let joined = format!("{}\n\n{}", last.text, text);
            last.text = clip(&joined, TURN_CHARS);
            return;
        }
    }
    turns.push(Turn { role: Role::Assistant, text: clip(text, TURN_CHARS) });
}

fn push_user_text(turns: &mut Vec<Turn>, content: &serde_json::Value) {
    let text = text_of(content);
    if !text.is_empty() {
        turns.push(Turn { role: Role::User, text: clip(&text, TURN_CHARS) });
    }
}

/// One line for a tool call: the tool's name, plus the most telling scalar of
/// its input. The keys are the common argument names across claude's built-in
/// tools; a tool carrying none of them renders as its name alone, which is
/// still honest.
fn tool_line(name: &str, input: Option<&serde_json::Value>) -> String {
    const KEYS: [&str; 9] = [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "skill",
        "prompt",
        "description",
    ];
    let summary = input.and_then(|input| {
        KEYS.iter().find_map(|k| input.get(k).and_then(|v| v.as_str())).map(|s| {
            let first = s.lines().next().unwrap_or("").trim();
            clip(first, TOOL_CHARS)
        })
    });
    match summary {
        Some(s) if !s.is_empty() => clip(&format!("{name}: {s}"), TOOL_CHARS + 40),
        _ => name.to_string(),
    }
}

/// Claude's dialect. Record types, block types and the flags below were read
/// from real session files (the counts are from a 120-file sample):
/// `user`/`assistant` records carry the conversation; `isMeta` user records
/// are injected context, not the user speaking; `isSidechain` records are a
/// subagent's conversation, not this one; `ai-title` records name the session.
fn parse_claude(text: &str) -> Conversation {
    let mut convo = Conversation::default();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if convo.started.is_none() {
            if let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()) {
                convo.started = Some(ts.to_string());
            }
        }
        if let Some(t) = v.get("aiTitle").and_then(|t| t.as_str()) {
            convo.title = Some(t.to_string());
        }
        if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true)
            || v.get("isMeta").and_then(|b| b.as_bool()) == Some(true)
        {
            continue;
        }
        let Some(message) = v.get("message") else { continue };
        match v.get("type").and_then(|t| t.as_str()) {
            // A user record whose content is only tool_result blocks is a
            // tool answering, not the user speaking; text_of leaves it empty
            // and push_user_text drops it. (6,983 of 7,266 user records in
            // the sample were that kind.)
            Some("user") => {
                if let Some(content) = message.get("content") {
                    push_user_text(&mut convo.turns, content);
                }
            }
            Some("assistant") => {
                let Some(blocks) = message.get("content").and_then(|c| c.as_array()) else {
                    continue;
                };
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                                push_assistant_text(&mut convo.turns, t);
                            }
                        }
                        // server_tool_use is claude's provider-side tool call
                        // (web search); same shape, same rendering.
                        Some("tool_use") | Some("server_tool_use") => {
                            let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                            convo.turns.push(Turn {
                                role: Role::Tool,
                                text: tool_line(name, b.get("input")),
                            });
                        }
                        // Thinking blocks are the model's scratch space, and
                        // they dwarf the prose. The session file the archive
                        // keeps still has them.
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    convo
}

/// Pi's dialect, from real session files: a `session` header, then `message`
/// records whose `message.role` is `user` or `assistant` with `text` content
/// blocks. Pi's tool-call block shape has not been read against a real file,
/// so an unknown block renders by its own declared `name` (or type) as a tool
/// line rather than being guessed at — or silently dropped.
fn parse_pi(text: &str) -> Conversation {
    let mut convo = Conversation::default();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if convo.started.is_none() {
            if let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()) {
                convo.started = Some(ts.to_string());
            }
        }
        if v.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        let Some(message) = v.get("message") else { continue };
        match message.get("role").and_then(|r| r.as_str()) {
            Some("user") => {
                if let Some(content) = message.get("content") {
                    push_user_text(&mut convo.turns, content);
                }
            }
            Some("assistant") => {
                let Some(blocks) = message.get("content").and_then(|c| c.as_array()) else {
                    continue;
                };
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                                push_assistant_text(&mut convo.turns, t);
                            }
                        }
                        Some("thinking") | None => {}
                        Some(other) => {
                            let name = b.get("name").and_then(|n| n.as_str()).unwrap_or(other);
                            convo.turns.push(Turn {
                                role: Role::Tool,
                                text: tool_line(
                                    name,
                                    b.get("arguments").or_else(|| b.get("input")),
                                ),
                            });
                        }
                    }
                }
            }
            // Other roles (tool results and whatever pi adds next) are not
            // the conversation.
            _ => {}
        }
    }
    convo
}

/// Every session in a transcript directory, oldest first. Sessions where
/// nothing renderable was said are dropped — within a format we can read,
/// an empty parse really does mean nothing was said, unlike the unsupported
/// case the caller must keep distinct.
pub fn read_sessions(dir: &Path, format: Format) -> Vec<Conversation> {
    let mut sessions: Vec<(String, String, Conversation)> = Vec::new();
    // The directory's own files and its per-session stores below it; see
    // `usage::transcripts` for why one level and no further.
    for path in crate::usage::transcripts(dir) {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let convo = parse_conversation(&text, format);
        if convo.turns.is_empty() {
            continue;
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // RFC 3339 sorts lexicographically, so the recorded start orders the
        // sessions; the filename only tie-breaks (or stands in for a file with
        // no timestamp at all, which no real file in the sample was).
        sessions.push((convo.started.clone().unwrap_or_default(), name, convo));
    }
    sessions.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    sessions.into_iter().map(|(_, _, c)| c).collect()
}

/// Move a directory tree by copy, verify, remove — in that order, so a move
/// that fails leaves the source untouched and is a pure no-op apart from
/// stray copied files the next attempt overwrites. Used to carry a transcript
/// directory into the archive at teardown and back out of it on restore.
///
/// The destination is merged into, not replaced: re-archiving a restored run
/// moves into a directory that already holds the previous rescue, and
/// clobbering that on a failure halfway through would destroy the one copy.
///
/// Modification times are preserved. They are not cosmetic here: "resume the
/// most recent session" is decided by mtime in the agent's own CLI, so a
/// reinstated directory with scrambled mtimes could resume the wrong session.
///
/// Returns the number of files moved. Refuses (before copying anything) a
/// tree containing anything but regular files and directories.
pub fn move_tree_verified(src: &Path, dst: &Path) -> Result<usize> {
    let mut files: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
    plan_copy(src, dst, &mut files)?;
    for (from, to) in &files {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to).with_context(|| format!("copying {}", from.display()))?;
        let meta = std::fs::metadata(from)?;
        if let Ok(modified) = meta.modified() {
            let times = std::fs::FileTimes::new().set_modified(modified);
            std::fs::File::options().write(true).open(to)?.set_times(times)?;
        }
    }
    // Re-stat both sides before the delete. The agent is stopped by the time
    // this runs, but "the copy said Ok" and "the bytes are there" are
    // different claims and only the second one licenses removing the source.
    for (from, to) in &files {
        let (f, t) = (std::fs::metadata(from)?, std::fs::metadata(to)?);
        if f.len() != t.len() {
            bail!("verify failed: {} is {} bytes, copy is {}", from.display(), f.len(), t.len());
        }
    }
    std::fs::remove_dir_all(src)?;
    Ok(files.len())
}

fn plan_copy(
    src: &Path,
    dst: &Path,
    files: &mut Vec<(std::path::PathBuf, std::path::PathBuf)>,
) -> Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            plan_copy(&from, &to, files)?;
        } else if kind.is_file() {
            files.push((from, to));
        } else {
            // A symlink would either be copied through (duplicating whatever
            // it points at) or dropped silently by the remove; neither is a
            // faithful move, so the whole tree stays where it is.
            bail!("refusing to move {}: not a regular file", from.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A claude user record as the real files write it.
    fn c_user(text: &str) -> String {
        format!(
            r#"{{"type":"user","timestamp":"2026-08-02T16:11:21.491Z","message":{{"role":"user","content":{}}}}}"#,
            serde_json::to_string(text).unwrap()
        )
    }

    fn c_assistant_block(mid: &str, block: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"2026-08-02T16:11:25.000Z","message":{{"id":"{mid}","role":"assistant","content":[{block}]}}}}"#
        )
    }

    fn c_text(mid: &str, text: &str) -> String {
        c_assistant_block(
            mid,
            &format!(r#"{{"type":"text","text":{}}}"#, serde_json::to_string(text).unwrap()),
        )
    }

    #[test]
    fn a_claude_exchange_reads_as_user_then_assistant() {
        let text = [c_user("fix the bug"), c_text("m1", "Looking now.")].join("\n");
        let c = parse_conversation(&text, Format::Claude);
        assert_eq!(c.turns.len(), 2);
        assert_eq!(c.turns[0], Turn { role: Role::User, text: "fix the bug".into() });
        assert_eq!(c.turns[1], Turn { role: Role::Assistant, text: "Looking now.".into() });
        assert_eq!(c.started.as_deref(), Some("2026-08-02T16:11:21.491Z"));
    }

    #[test]
    fn streamed_assistant_records_merge_into_one_spoken_turn() {
        // Claude writes one content block per record; a turn of two paragraphs
        // is two records. Unmerged, every sentence would render as its own
        // bubble.
        let text = [c_text("m1", "First."), c_text("m1", "Second.")].join("\n");
        let c = parse_conversation(&text, Format::Claude);
        assert_eq!(c.turns.len(), 1);
        assert_eq!(c.turns[0].text, "First.\n\nSecond.");
    }

    #[test]
    fn tool_calls_render_as_one_line_each_and_break_the_merge() {
        let tool = c_assistant_block(
            "m1",
            r#"{"type":"tool_use","name":"Bash","input":{"command":"cargo test","description":"Run tests"}}"#,
        );
        let text = [c_text("m1", "Running tests."), tool, c_text("m2", "They pass.")].join("\n");
        let c = parse_conversation(&text, Format::Claude);
        let rendered: Vec<(Role, &str)> =
            c.turns.iter().map(|t| (t.role, t.text.as_str())).collect();
        assert_eq!(
            rendered,
            vec![
                (Role::Assistant, "Running tests."),
                // `command` outranks `description`: it is what the tool did.
                (Role::Tool, "Bash: cargo test"),
                (Role::Assistant, "They pass."),
            ]
        );
    }

    #[test]
    fn a_tool_with_no_summarisable_input_is_its_name_alone() {
        let tool = c_assistant_block(
            "m1",
            r#"{"type":"tool_use","name":"TodoWrite","input":{"todos":[]}}"#,
        );
        let c = parse_conversation(&tool, Format::Claude);
        assert_eq!(c.turns[0].text, "TodoWrite");
    }

    #[test]
    fn tool_result_records_are_not_the_user_speaking() {
        // 6,983 of 7,266 user records in the 120-file sample carry only tool
        // results. Rendering them as user turns would drown the conversation.
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"445 lines of output"}]}}"#;
        assert!(parse_conversation(line, Format::Claude).turns.is_empty());
    }

    #[test]
    fn a_pasted_user_message_with_text_blocks_still_speaks() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"try again"}]}}"#;
        let c = parse_conversation(line, Format::Claude);
        assert_eq!(c.turns[0], Turn { role: Role::User, text: "try again".into() });
    }

    #[test]
    fn meta_and_sidechain_records_are_not_this_conversation() {
        // isMeta user records are injected context; isSidechain records are a
        // subagent talking. Both are real records in real files, and both
        // would misattribute text if rendered here.
        let meta = r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"Caveat: injected"}}"#;
        let side = r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"subagent prose"}]}}"#;
        let text = [meta, side, &c_user("real")].join("\n");
        let c = parse_conversation(&text, Format::Claude);
        assert_eq!(c.turns.len(), 1);
        assert_eq!(c.turns[0].text, "real");
    }

    #[test]
    fn thinking_blocks_are_left_in_the_file() {
        let think = c_assistant_block("m1", r#"{"type":"thinking","thinking":"let me consider"}"#);
        assert!(parse_conversation(&think, Format::Claude).turns.is_empty());
    }

    #[test]
    fn the_session_title_is_the_agents_own() {
        let text = [
            c_user("hello"),
            r#"{"type":"ai-title","aiTitle":"Debug missing issues","sessionId":"s"}"#.to_string(),
        ]
        .join("\n");
        assert_eq!(
            parse_conversation(&text, Format::Claude).title.as_deref(),
            Some("Debug missing issues")
        );
    }

    #[test]
    fn malformed_and_bookkeeping_lines_are_skipped() {
        // Transcripts hold mode, permission-mode, file-history and truncated
        // lines; none of it is conversation and none of it may fail the pass.
        let text = [
            r#"{"type":"mode","mode":"normal"}"#,
            r#"{"type":"file-history-snapshot","messageId":"x"}"#,
            "not json",
            r#"{"type":"user","message":{"role":"user","content":"hi"#,
            &c_user("hi"),
        ]
        .join("\n");
        assert_eq!(parse_conversation(&text, Format::Claude).turns.len(), 1);
    }

    #[test]
    fn a_huge_turn_is_clipped_and_says_so() {
        let big = "x".repeat(TURN_CHARS + 100);
        let c = parse_conversation(&c_user(&big), Format::Claude);
        assert!(c.turns[0].text.chars().count() < TURN_CHARS + 30);
        assert!(c.turns[0].text.ends_with("… (truncated)"));
    }

    /// Pi records as the real files write them.
    fn p_msg(role: &str, text: &str) -> String {
        format!(
            r#"{{"type":"message","id":"a1","timestamp":"2026-06-30T21:22:20.084Z","message":{{"role":"{role}","content":[{{"type":"text","text":{}}}]}}}}"#,
            serde_json::to_string(text).unwrap()
        )
    }

    #[test]
    fn a_pi_exchange_reads_in_order() {
        let header = r#"{"type":"session","version":3,"id":"s","timestamp":"2026-06-30T21:22:20.067Z","cwd":"/w"}"#;
        let text =
            [header.to_string(), p_msg("user", "hey"), p_msg("assistant", "Hello.")].join("\n");
        let c = parse_conversation(&text, Format::Pi);
        assert_eq!(c.started.as_deref(), Some("2026-06-30T21:22:20.067Z"));
        assert_eq!(
            c.turns,
            vec![
                Turn { role: Role::User, text: "hey".into() },
                Turn { role: Role::Assistant, text: "Hello.".into() },
            ]
        );
    }

    #[test]
    fn pi_bookkeeping_and_unknown_roles_are_skipped() {
        let text = [
            r#"{"type":"model_change","id":"m","modelId":"claude-opus-4-8"}"#.to_string(),
            r#"{"type":"message","id":"t","message":{"role":"toolResult","content":[{"type":"text","text":"output"}]}}"#.to_string(),
            p_msg("user", "hey"),
        ]
        .join("\n");
        assert_eq!(parse_conversation(&text, Format::Pi).turns.len(), 1);
    }

    #[test]
    fn an_unread_pi_block_shape_renders_by_its_own_name() {
        // Pi's tool-call block has not been read against a real file, so the
        // rule is default-deny: show the block's declared name, invent nothing.
        let line = r#"{"type":"message","id":"a","message":{"role":"assistant","content":[{"type":"toolCall","name":"bash","arguments":{"command":"ls"}}]}}"#;
        let c = parse_conversation(line, Format::Pi);
        assert_eq!(c.turns, vec![Turn { role: Role::Tool, text: "bash: ls".into() }]);
        let bare = r#"{"type":"message","id":"a","message":{"role":"assistant","content":[{"type":"somethingNew"}]}}"#;
        let c = parse_conversation(bare, Format::Pi);
        assert_eq!(c.turns[0].text, "somethingNew");
    }

    #[test]
    fn sessions_read_oldest_first_and_empty_ones_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let later = [c_user("second session")].join("\n");
        let earlier = c_user("first session").replace("2026-08-02", "2026-08-01");
        std::fs::write(dir.path().join("b.jsonl"), later).unwrap();
        std::fs::write(dir.path().join("a.jsonl"), earlier).unwrap();
        // A session where nothing renderable was said: within a format we can
        // read, that really is "nothing was said", and it adds only noise.
        std::fs::write(dir.path().join("c.jsonl"), r#"{"type":"mode","mode":"normal"}"#).unwrap();
        std::fs::write(dir.path().join("notes.md"), "not a transcript").unwrap();
        let sessions = read_sessions(dir.path(), Format::Claude);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].turns[0].text, "first session");
        assert_eq!(sessions[1].turns[0].text, "second session");
    }

    #[test]
    fn read_sessions_of_a_missing_dir_is_empty() {
        assert!(read_sessions(Path::new("/nope/never"), Format::Claude).is_empty());
    }

    #[test]
    fn sessions_in_a_per_session_store_are_read_too() {
        // A pinned agent's conversation lives one level down (AGE-175); the
        // run's own directory still holds whatever it wrote before that.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.jsonl"),
            c_user("before").replace("2026-08-02", "2026-08-01"),
        )
        .unwrap();
        let store = dir.path().join("agent-3f9c");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("b.jsonl"), c_user("after")).unwrap();
        let sessions = read_sessions(dir.path(), Format::Claude);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].turns[0].text, "before");
        assert_eq!(sessions[1].turns[0].text, "after");
    }

    #[test]
    fn move_tree_carries_nested_files_and_removes_the_source() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("src");
        let dst = root.path().join("dst");
        std::fs::create_dir_all(src.join("s1").join("tool-results")).unwrap();
        std::fs::write(src.join("s1.jsonl"), "records").unwrap();
        std::fs::write(src.join("s1").join("tool-results").join("t.txt"), "output").unwrap();

        let moved = move_tree_verified(&src, &dst).unwrap();
        assert_eq!(moved, 2);
        assert!(!src.exists(), "a verified move leaves no source behind");
        assert_eq!(std::fs::read_to_string(dst.join("s1.jsonl")).unwrap(), "records");
        assert_eq!(
            std::fs::read_to_string(dst.join("s1").join("tool-results").join("t.txt")).unwrap(),
            "output"
        );
    }

    #[test]
    fn move_tree_merges_into_an_existing_destination() {
        // Re-archiving a restored run moves into the directory that already
        // holds the previous rescue; what is already there must survive.
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("src");
        let dst = root.path().join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("new.jsonl"), "new").unwrap();
        std::fs::write(dst.join("old.jsonl"), "old").unwrap();
        move_tree_verified(&src, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(dst.join("old.jsonl")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(dst.join("new.jsonl")).unwrap(), "new");
    }

    #[test]
    fn move_tree_preserves_mtimes() {
        // "Resume the most recent session" is decided by mtime in the agent's
        // own CLI; a reinstated directory with fresh mtimes could resume the
        // wrong session.
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("src");
        let dst = root.path().join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("s.jsonl"), "records").unwrap();
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        std::fs::File::options()
            .write(true)
            .open(src.join("s.jsonl"))
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(t))
            .unwrap();
        move_tree_verified(&src, &dst).unwrap();
        assert_eq!(std::fs::metadata(dst.join("s.jsonl")).unwrap().modified().unwrap(), t);
    }

    #[cfg(unix)]
    #[test]
    fn move_tree_refuses_a_symlink_and_touches_nothing() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("src");
        let dst = root.path().join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("s.jsonl"), "records").unwrap();
        std::os::unix::fs::symlink("/etc/hosts", src.join("link")).unwrap();
        assert!(move_tree_verified(&src, &dst).is_err());
        assert!(src.join("s.jsonl").exists(), "a refused move is a no-op");
        assert!(!dst.exists(), "the refusal happens before anything is copied");
    }
}
