//! The run tools (AGE-209): the one group on this server that points the
//! other way. Everything else here tells a dispatched agent about its own run;
//! these let it ask Agency to start another, wait for that one to finish or
//! stop on something, and read what it did.
//!
//! This module is the protocol half: what a call says, parsed and bounded
//! before anything acts on it. Acting is the app's, through
//! [`super::Hooks::runs`], because only the app can cut a worktree, start a
//! session and read a run's state. What the app may do with a call, and what
//! it must refuse, is in `docs/dispatch-tools.md`.

use serde_json::Value;
use std::time::Duration;

/// The longest a `run_wait` holds before answering "still working". Long
/// enough that an orchestrator waiting on a real task calls a handful of
/// times, not hundreds; short enough that a client with its own tool-call
/// timeout can be given one under it.
pub const WAIT_MAX: Duration = Duration::from_secs(30 * 60);
/// What `run_wait` holds for when the call does not say.
pub const WAIT_DEFAULT: Duration = Duration::from_secs(5 * 60);
/// What a `run_read` answer is cut to. A transcript or a diff is unbounded,
/// and the whole of it lands in the caller's context; the full text is in the
/// child's worktree, which the answer names.
pub const READ_CHARS: usize = 24_000;

/// How many `run_wait`s one run's server holds open at once. Each holds a
/// connection, and the server has `MAX_CONNS` for everything: the
/// agent's `set_status`, the preview bridge and the state hooks' posts too. A
/// parent has at most four children to wait on, so twice that is room for a
/// retry overlapping the call it replaces, and nowhere near the whole server.
pub const WAITS_AT_ONCE: usize = 8;

/// Who made a run tool call: the session [`super::Hooks::caller`] named when
/// it admitted the request, and whether the client has since hung up.
pub struct Asker<'a> {
    /// The caller's session, by the name the app gave it.
    pub session: &'a str,
    /// True once the client has closed its end of the connection. A blocking
    /// call asks between looks and gives up, since its answer has nowhere to
    /// go: a client whose own tool-call timeout fires drops the connection and
    /// calls again, and each abandoned `run_wait` used to hold its connection
    /// for the whole of the timeout it asked for, up to half an hour.
    pub hung_up: &'a dyn Fn() -> bool,
}

/// What carries a call out: the app's half, behind [`super::Hooks::runs`].
pub type Exec = dyn Fn(&Asker, Call) -> Result<String, String> + Send + Sync;

/// One call to a run tool, parsed.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    /// Start a run in this run's project, the way the user's Dispatch does.
    Dispatch(Work),
    /// Block until `run_id` is done, blocked, or gone, or `timeout` passes.
    Wait { run_id: String, timeout: Duration },
    /// What `run_id` has done so far.
    Read { run_id: String, what: Read },
}

/// What a dispatched run is to work on.
#[derive(Debug, Clone, PartialEq)]
pub enum Work {
    /// An issue in this project's tracker, by key: `AGE-14`.
    Issue(String),
    /// A prompt, as the user would type one into New run.
    Prompt(String),
}

/// Which view of a run `run_read` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Read {
    /// The conversation, from the agent's own transcript.
    Transcript,
    /// What the run's branch and worktree change against its base.
    Diff,
    /// The bottom of the run's terminal, as the user would see it.
    Screen,
}

pub const DISPATCH: &str = "run_dispatch";
pub const WAIT: &str = "run_wait";
pub const READ: &str = "run_read";

/// Parse one call. `Err` is the text a malformed call is answered with,
/// addressed to the agent that made it.
pub fn parse(name: &str, args: &Value) -> Result<Call, String> {
    let text =
        |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    match name {
        DISPATCH => match (text("issue"), text("prompt")) {
            (Some(key), None) => Ok(Call::Dispatch(Work::Issue(key.to_ascii_uppercase()))),
            (None, Some(prompt)) => Ok(Call::Dispatch(Work::Prompt(prompt.to_string()))),
            (Some(_), Some(_)) => Err("run_dispatch takes an issue or a prompt, not both: \
                                       an issue's prompt is the issue itself."
                .to_string()),
            (None, None) => {
                Err("run_dispatch needs an issue key (e.g. \"AGE-14\") or a prompt.".to_string())
            }
        },
        WAIT => {
            let run_id = run_id(args, WAIT)?;
            // A missing, negative or non-numeric timeout is the default, not
            // an error: the agent asked to wait, and waiting is what it gets.
            let timeout = match args.get("timeout_secs").and_then(Value::as_f64) {
                Some(s) if s.is_finite() && s >= 0.0 => {
                    Duration::from_secs_f64(s.min(WAIT_MAX.as_secs_f64()))
                }
                _ => WAIT_DEFAULT,
            };
            Ok(Call::Wait { run_id, timeout })
        }
        READ => {
            let run_id = run_id(args, READ)?;
            let what = match text("what") {
                None | Some("transcript") => Read::Transcript,
                Some("diff") => Read::Diff,
                Some("screen") => Read::Screen,
                Some(other) => {
                    return Err(format!(
                        "run_read cannot show \"{other}\": ask for transcript, diff or screen."
                    ))
                }
            };
            Ok(Call::Read { run_id, what })
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

fn run_id(args: &Value, tool: &str) -> Result<String, String> {
    args.get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{tool} needs the run_id run_dispatch returned."))
}

/// `text` cut to its last `max` characters, on a line boundary, saying how
/// much went. For a transcript or a terminal, where the end is what matters.
pub fn keep_tail(text: &str, max: usize) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_string();
    }
    let skip = text.char_indices().nth(total - max).map_or(0, |(i, _)| i);
    let rest = &text[skip..];
    // Start on a whole line where there is one to start on.
    let rest = rest.split_once('\n').map_or(rest, |(_, after)| after);
    format!("({} earlier characters left out)\n{rest}", total - rest.chars().count())
}

/// `text` cut to its first `max` characters, on a line boundary, saying how
/// much went. For a diff, which reads top down.
pub fn keep_head(text: &str, max: usize) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_string();
    }
    let end = text.char_indices().nth(max).map_or(text.len(), |(i, _)| i);
    let kept = &text[..end];
    let kept = kept.rsplit_once('\n').map_or(kept, |(before, _)| before);
    format!("{kept}\n({} more characters left out)", total - kept.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dispatch_takes_exactly_one_of_issue_and_prompt() {
        assert_eq!(
            parse(DISPATCH, &json!({ "issue": " age-14 " })),
            Ok(Call::Dispatch(Work::Issue("AGE-14".into())))
        );
        assert_eq!(
            parse(DISPATCH, &json!({ "prompt": "fix the login test" })),
            Ok(Call::Dispatch(Work::Prompt("fix the login test".into())))
        );
        assert!(parse(DISPATCH, &json!({ "issue": "AGE-1", "prompt": "x" })).is_err());
        assert!(parse(DISPATCH, &json!({})).is_err());
        // Blank is absent: an empty prompt is a run with nothing to do.
        assert!(parse(DISPATCH, &json!({ "prompt": "  " })).is_err());
    }

    #[test]
    fn a_dispatch_cannot_name_a_profile() {
        // The child runs its parent's profile, so the argument the agent might
        // reach for to pick a wider one is not one this tool reads.
        assert_eq!(
            parse(DISPATCH, &json!({ "prompt": "x", "profile": "claude-yolo" })),
            Ok(Call::Dispatch(Work::Prompt("x".into())))
        );
    }

    #[test]
    fn wait_bounds_its_timeout() {
        let wait = |args: Value| match parse(WAIT, &args) {
            Ok(Call::Wait { timeout, .. }) => timeout,
            other => panic!("{other:?}"),
        };
        assert_eq!(wait(json!({ "run_id": "r" })), WAIT_DEFAULT);
        assert_eq!(wait(json!({ "run_id": "r", "timeout_secs": 30 })), Duration::from_secs(30));
        assert_eq!(wait(json!({ "run_id": "r", "timeout_secs": 1e9 })), WAIT_MAX);
        assert_eq!(wait(json!({ "run_id": "r", "timeout_secs": -5 })), WAIT_DEFAULT);
        assert_eq!(wait(json!({ "run_id": "r", "timeout_secs": "soon" })), WAIT_DEFAULT);
        assert_eq!(wait(json!({ "run_id": "r", "timeout_secs": 0 })), Duration::ZERO);
        assert!(parse(WAIT, &json!({})).is_err(), "no run_id");
    }

    #[test]
    fn read_defaults_to_the_transcript_and_refuses_what_it_cannot_show() {
        let what = |w: Value| match parse(READ, &json!({ "run_id": "r", "what": w })) {
            Ok(Call::Read { what, .. }) => Ok(what),
            Err(e) => Err(e),
            other => panic!("{other:?}"),
        };
        assert_eq!(what(Value::Null), Ok(Read::Transcript));
        assert_eq!(what(json!("diff")), Ok(Read::Diff));
        assert_eq!(what(json!("screen")), Ok(Read::Screen));
        assert!(what(json!("secrets")).unwrap_err().contains("transcript, diff or screen"));
    }

    #[test]
    fn clipping_says_what_it_left_out_and_keeps_whole_lines() {
        let text = "one\ntwo\nthree\nfour\n";
        assert_eq!(keep_tail(text, 100), text);
        let tail = keep_tail(text, 8);
        assert!(tail.ends_with("four\n"), "{tail}");
        assert!(tail.starts_with("("), "{tail}");
        assert!(!tail.contains("hree\n") || tail.contains("three\n"), "no half lines: {tail}");

        let head = keep_head(text, 9);
        assert!(head.starts_with("one\ntwo"), "{head}");
        assert!(head.contains("more characters left out"), "{head}");
        assert!(!head.contains("thr"), "no half lines: {head}");
    }

    #[test]
    fn clipping_is_safe_on_multibyte_text() {
        let text = "é".repeat(50);
        assert!(keep_tail(&text, 10).contains("left out"));
        assert!(keep_head(&text, 10).contains("left out"));
    }
}
