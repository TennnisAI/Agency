//! What an agent's lifecycle hook says about it (AGE-206), parsed from the
//! body the hook posts to [`crate::state_hooks::STATE_PATH`] (Claude Code) or
//! [`crate::state_hooks::CODEX_STATE_PATH`] (Codex, AGE-251).
//!
//! Pure, and default-deny: an event or a notification type not named below is
//! ignored, never guessed at, so the first new event an agent ships cannot
//! quietly mis-state a run. Only the fields named in [`Body`] are read. A hook
//! payload also carries the tool's input (the agent's actual commands and file
//! contents) and the transcript path, and none of that is needed or kept.

use serde::Deserialize;

/// The state one report puts a session in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reported {
    /// A turn is in flight: a prompt was submitted or a tool ran.
    Working,
    /// Stopped on a permission dialog or a question, and cannot go on until
    /// the user answers it.
    Blocked,
    /// The turn is over and the result is waiting for the user.
    Done,
    /// Sitting at the input box with nothing in flight (`idle_prompt`). Not a
    /// state on its own: it is a turn ending only after a turn, which is the
    /// app's to know (see `activity::apply` there). Read as `Done` outright, a
    /// fresh agent nobody had prompted would announce a finished turn.
    AtPrompt,
    /// No session in flight: it ended, or was cleared.
    Idle,
}

/// Which agent's hooks a post came from, by the route it was posted to. The
/// two share event names and a payload shape, and mostly agree on what they
/// mean, but not entirely: each one's allowlist is its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Claude,
    Codex,
}

/// One hook post: the state it reports, and which of the agent's own sessions
/// sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub state: Reported,
    /// Claude Code's `session_id`. The Agency session header cannot tell a
    /// tab's agent from a `claude` that agent started through its Bash tool,
    /// since the child inherits the variable the header is built from; this
    /// can, and the app pins each tab to the first one it hears from.
    pub conversation: Option<String>,
    /// Sent by a subagent (the Task tool) rather than the main agent.
    pub subagent: bool,
    /// Opens a conversation in the tab's own agent, which the tab is pinned to
    /// from now on (see [`admit`]). Codex's `SessionStart` only: after `/clear`
    /// its next prompt comes under a new `session_id`, with no `SessionEnd` for
    /// the old one to free the pin first.
    pub opens: bool,
}

#[derive(Deserialize)]
struct Body {
    hook_event_name: Option<String>,
    notification_type: Option<String>,
    session_id: Option<String>,
    /// Set on events from a subagent.
    agent_id: Option<serde_json::Value>,
    /// Codex's: the tool a `PreToolUse` is for, and why a session started.
    tool_name: Option<String>,
    source: Option<String>,
}

/// What a hook body from `dialect`'s agent reports, or `None` for a body that
/// says nothing we act on. See `state_hooks::CLAUDE_EVENTS` and
/// `state_hooks::CODEX_EVENTS` for what each event was verified to mean.
pub fn parse(dialect: Dialect, body: &[u8]) -> Option<Event> {
    let b: Body = serde_json::from_slice(body).ok()?;
    let subagent = b.agent_id.is_some_and(|v| !v.is_null());
    let mut opens = false;
    let state = match (dialect, b.hook_event_name.as_deref()?) {
        (_, "UserPromptSubmit" | "PostToolUse") => Reported::Working,
        (Dialect::Claude, "PreToolUse" | "PostToolUseFailure") => Reported::Working,
        // Codex's question tool fires no `PermissionRequest`: this is the only
        // sign of it, and it is on screen until its `PostToolUse`.
        (Dialect::Codex, "PreToolUse") if b.tool_name.as_deref() == Some("request_user_input") => {
            Reported::Blocked
        }
        (Dialect::Codex, "PreToolUse") => Reported::Working,
        (_, "PermissionRequest") => Reported::Blocked,
        (_, "Stop") => Reported::Done,
        (Dialect::Claude, "Notification")
            if b.notification_type.as_deref() == Some("idle_prompt") =>
        {
            Reported::AtPrompt
        }
        // The user stopped the turn, on a dialog or in the middle of it. Codex
        // says so where Claude Code says nothing, so this needs none of
        // `AtPrompt`'s guessing about whether a turn ran.
        (Dialect::Codex, "Interrupt") => Reported::Done,
        // A new conversation, nothing in flight in it yet. A `source` not
        // named here (`compact` is in the binary) is not a new conversation.
        (Dialect::Codex, "SessionStart")
            if matches!(b.source.as_deref(), Some("startup" | "resume" | "clear")) =>
        {
            opens = true;
            Reported::Idle
        }
        (_, "SessionEnd") => Reported::Idle,
        _ => return None,
    };
    // A subagent's turn ending is not the run's: four of them finishing would
    // read as four turns ending. Its tool calls and its dialogs are the run's,
    // though. Every subagent post was dropped here once, and a permission
    // dialog a subagent raised never read blocked: the report lapsed, the
    // quiet pane read done, and a queued message's Enter approved the command.
    if subagent && !matches!(state, Reported::Working | Reported::Blocked) {
        return None;
    }
    Some(Event { state, conversation: b.session_id.filter(|s| !s.is_empty()), subagent, opens })
}

/// Whether a report from `ev` speaks for the tab whose pinned conversation is
/// `pin`, updating the pin. Pure, so the rule is tested without a server.
///
/// The first conversation a tab hears from is its agent: a child `claude` can
/// only start through one of the agent's own tool calls, which posts first.
/// Another conversation is a child, and is ignored; without this, a `claude -p`
/// the agent ran through Bash posted `Stop` and `SessionEnd` under the tab's
/// header, and the tab read done and then idle in the middle of its own tool
/// call. The pinned conversation ending (`/clear` or exit) frees the pin for
/// whatever comes next, which after `/clear` is the same process under a new
/// id. A body with no id at all is admitted and pins nothing.
///
/// A conversation that [`Event::opens`] takes the pin whatever held it. That is
/// safe only because nothing but the tab's own agent can send one: Codex's
/// hooks are on its command line, which a `codex` it starts does not inherit,
/// and Claude Code's route never parses an event as opening.
pub fn admit(pin: &mut Option<String>, ev: &Event) -> bool {
    let Some(id) = ev.conversation.as_deref() else { return true };
    if ev.opens {
        *pin = Some(id.to_string());
        return true;
    }
    match pin.as_deref() {
        Some(p) if p != id => return false,
        _ => {}
    }
    *pin = (ev.state != Reported::Idle).then(|| id.to_string());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn of(v: serde_json::Value) -> Option<Reported> {
        parse(Dialect::Claude, v.to_string().as_bytes()).map(|e| e.state)
    }

    #[test]
    fn each_subscribed_event_maps_to_its_state() {
        for (event, want) in [
            ("UserPromptSubmit", Reported::Working),
            ("PreToolUse", Reported::Working),
            ("PostToolUse", Reported::Working),
            ("PostToolUseFailure", Reported::Working),
            ("PermissionRequest", Reported::Blocked),
            ("Stop", Reported::Done),
            ("SessionEnd", Reported::Idle),
        ] {
            assert_eq!(of(json!({ "hook_event_name": event })), Some(want), "{event}");
        }
    }

    /// A real `PermissionRequest` body from Claude Code 2.1.289, cut to its
    /// keys: the tool input rides along and is never read.
    #[test]
    fn a_real_permission_request_is_blocked() {
        let body = json!({
            "hook_event_name": "PermissionRequest",
            "session_id": "8049a1fc-49e5-4aee-a635-e918a53ca5de",
            "cwd": "/repo/.agency/worktrees/fix-a1",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_input": { "command": "curl -sI https://example.com | head -1" },
            "permission_suggestions": [],
            "transcript_path": "/x.jsonl",
        });
        let ev = parse(Dialect::Claude, body.to_string().as_bytes()).unwrap();
        assert_eq!(ev.state, Reported::Blocked);
        assert_eq!(ev.conversation.as_deref(), Some("8049a1fc-49e5-4aee-a635-e918a53ca5de"));
    }

    #[test]
    fn only_the_idle_notification_counts() {
        let n = |t: &str| of(json!({ "hook_event_name": "Notification", "notification_type": t }));
        assert_eq!(n("idle_prompt"), Some(Reported::AtPrompt));
        // Arrives about six seconds after PermissionRequest, and would race an
        // answer given inside them.
        assert_eq!(n("permission_prompt"), None);
        assert_eq!(n("auth_success"), None);
        assert_eq!(of(json!({ "hook_event_name": "Notification" })), None);
    }

    #[test]
    fn a_subagent_never_ends_the_runs_turn() {
        let sub =
            |event: &str| json!({ "hook_event_name": event, "agent_id": "ad32eb3b97e43b23c" });
        assert_eq!(of(sub("Stop")), None);
        assert_eq!(of(sub("SessionEnd")), None);
        let main = json!({ "hook_event_name": "Stop", "agent_id": null });
        assert_eq!(of(main), Some(Reported::Done), "a null agent_id is the main agent");
    }

    #[test]
    fn a_subagents_work_and_dialogs_are_the_runs() {
        let sub = |event: &str| {
            parse(
                Dialect::Claude,
                json!({ "hook_event_name": event, "agent_id": "a1" }).to_string().as_bytes(),
            )
        };
        let blocked = sub("PermissionRequest").unwrap();
        assert_eq!((blocked.state, blocked.subagent), (Reported::Blocked, true));
        assert_eq!(sub("PostToolUse").unwrap().state, Reported::Working);
        assert!(!parse(Dialect::Claude, br#"{"hook_event_name":"Stop"}"#).unwrap().subagent);
    }

    #[test]
    fn anything_unrecognised_is_ignored() {
        for event in ["SubagentStop", "SessionStart", "PreCompact", "stop", "Elicitation", ""] {
            assert_eq!(of(json!({ "hook_event_name": event })), None, "{event}");
        }
        assert_eq!(of(json!({})), None);
        assert_eq!(parse(Dialect::Claude, b"not json"), None);
        assert_eq!(of(json!({ "hook_event_name": 3 })), None);
    }

    fn ev(state: Reported, conversation: &str) -> Event {
        Event { state, conversation: Some(conversation.to_string()), subagent: false, opens: false }
    }

    #[test]
    fn a_child_claude_never_speaks_for_its_parents_tab() {
        let mut pin = None;
        assert!(admit(&mut pin, &ev(Reported::Working, "parent")));
        // The agent runs `claude -p` through Bash. The child inherits the
        // session variable, and reports its whole life under the same header.
        for s in [Reported::Working, Reported::Done, Reported::Idle] {
            assert!(!admit(&mut pin, &ev(s, "child")), "{s:?}");
        }
        assert_eq!(pin.as_deref(), Some("parent"), "the child's end frees nothing");
        assert!(admit(&mut pin, &ev(Reported::Done, "parent")));
    }

    #[test]
    fn the_pinned_conversation_ending_lets_the_next_one_in() {
        let mut pin = None;
        assert!(admit(&mut pin, &ev(Reported::Done, "before-clear")));
        assert!(admit(&mut pin, &ev(Reported::Idle, "before-clear")));
        assert_eq!(pin, None);
        assert!(admit(&mut pin, &ev(Reported::Working, "after-clear")));
        assert!(!admit(&mut pin, &ev(Reported::Working, "before-clear")));
    }

    #[test]
    fn a_body_without_a_conversation_is_admitted_and_pins_nothing() {
        let mut pin = None;
        let ev =
            Event { state: Reported::Working, conversation: None, subagent: false, opens: false };
        assert!(admit(&mut pin, &ev));
        assert_eq!(pin, None);
        let blank =
            parse(Dialect::Claude, br#"{"hook_event_name":"Stop","session_id":""}"#).unwrap();
        assert_eq!(blank.conversation, None);
    }

    fn codex(v: serde_json::Value) -> Option<Event> {
        parse(Dialect::Codex, v.to_string().as_bytes())
    }

    fn codex_state(event: &str) -> Option<Reported> {
        codex(json!({ "hook_event_name": event })).map(|e| e.state)
    }

    #[test]
    fn each_codex_event_maps_to_its_state() {
        for (event, want) in [
            ("UserPromptSubmit", Reported::Working),
            ("PreToolUse", Reported::Working),
            ("PostToolUse", Reported::Working),
            ("PermissionRequest", Reported::Blocked),
            ("Stop", Reported::Done),
            ("Interrupt", Reported::Done),
            ("SessionEnd", Reported::Idle),
        ] {
            assert_eq!(codex_state(event), Some(want), "{event}");
        }
    }

    /// Bodies from Codex 0.153.4, cut to their keys.
    #[test]
    fn a_codex_question_is_blocked_and_its_answer_is_not() {
        let question = json!({
            "session_id": "01a1546e-bb7c-7393-825c-806b10b2795f",
            "turn_id": "01a15470-3843-71f0-9f05-b994288e057c",
            "hook_event_name": "PreToolUse",
            "model": "gpt-5.5",
            "permission_mode": "default",
            "tool_name": "request_user_input",
            "tool_input": { "questions": [{ "id": "q1", "question": "Which one?" }] },
            "tool_use_id": "call_1",
        });
        assert_eq!(codex(question.clone()).unwrap().state, Reported::Blocked);
        let mut answered = question;
        answered["hook_event_name"] = json!("PostToolUse");
        assert_eq!(codex(answered).unwrap().state, Reported::Working);
        let bash = json!({ "hook_event_name": "PreToolUse", "tool_name": "Bash" });
        assert_eq!(codex(bash).unwrap().state, Reported::Working);
    }

    #[test]
    fn each_agent_keeps_to_its_own_allowlist() {
        // Claude Code's question is a `PermissionRequest`; a tool of Codex's
        // name means nothing on its route.
        let q = json!({ "hook_event_name": "PreToolUse", "tool_name": "request_user_input" });
        assert_eq!(of(q), Some(Reported::Working));
        for event in ["Interrupt", "SessionStart"] {
            let body = json!({ "hook_event_name": event, "source": "startup" });
            assert_eq!(of(body), None, "{event} on Claude Code's route");
        }
        let idle = json!({ "hook_event_name": "Notification", "notification_type": "idle_prompt" });
        assert_eq!(codex(idle), None);
        for event in ["PostToolUseFailure", "SubagentStart", "SubagentStop", "PreCompact"] {
            assert_eq!(codex_state(event), None, "{event}");
        }
    }

    #[test]
    fn a_codex_session_start_opens_a_conversation() {
        for source in ["startup", "resume", "clear"] {
            let ev = codex(json!({
                "hook_event_name": "SessionStart",
                "session_id": "01a15470-d053-74a1-b441-d2d97a281b28",
                "source": source,
            }))
            .unwrap();
            assert_eq!((ev.state, ev.opens), (Reported::Idle, true), "{source}");
        }
        let compact = json!({ "hook_event_name": "SessionStart", "source": "compact" });
        assert_eq!(codex(compact), None);
        assert_eq!(codex(json!({ "hook_event_name": "SessionStart" })), None);
        assert!(!codex(json!({ "hook_event_name": "Stop" })).unwrap().opens);
    }

    #[test]
    fn a_codex_subagent_reports_like_a_claude_one() {
        let sub = |event: &str| {
            codex(json!({
                "hook_event_name": event,
                "session_id": "parent",
                "agent_id": "01a15477-d8f6-7363-b4dd-41c9c57f99aa",
                "agent_type": "default",
            }))
        };
        assert_eq!(sub("PermissionRequest").map(|e| e.state), Some(Reported::Blocked));
        assert_eq!(sub("UserPromptSubmit").map(|e| e.state), Some(Reported::Working));
        assert_eq!(sub("Interrupt"), None);
        assert_eq!(sub("Stop"), None);
    }

    #[test]
    fn clear_moves_a_codex_tab_to_its_new_conversation() {
        let mut pin = None;
        let opening = |id: &str| Event {
            state: Reported::Idle,
            conversation: Some(id.to_string()),
            subagent: false,
            opens: true,
        };
        assert!(admit(&mut pin, &opening("first")));
        assert!(admit(&mut pin, &ev(Reported::Done, "first")));
        // `/clear`, then a prompt: no `SessionEnd` for "first" came between.
        assert!(admit(&mut pin, &opening("second")));
        assert_eq!(pin.as_deref(), Some("second"));
        assert!(admit(&mut pin, &ev(Reported::Working, "second")));
        // On exit both end, the cleared one included, and only one is the tab's.
        assert!(!admit(&mut pin, &ev(Reported::Idle, "first")));
        assert!(admit(&mut pin, &ev(Reported::Idle, "second")));
        assert_eq!(pin, None);
    }
}
