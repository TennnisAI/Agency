//! What an agent's lifecycle hook says about it (AGE-206), parsed from the
//! body the hook posts to [`crate::state_hooks::STATE_PATH`].
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
}

#[derive(Deserialize)]
struct Body {
    hook_event_name: Option<String>,
    notification_type: Option<String>,
    session_id: Option<String>,
    /// Set on events from a subagent.
    agent_id: Option<serde_json::Value>,
}

/// What a Claude Code hook body reports, or `None` for a body that says
/// nothing we act on. See `state_hooks::CLAUDE_EVENTS` for what each event was
/// verified to mean.
pub fn parse(body: &[u8]) -> Option<Event> {
    let b: Body = serde_json::from_slice(body).ok()?;
    let subagent = b.agent_id.is_some_and(|v| !v.is_null());
    let state = match b.hook_event_name.as_deref()? {
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure" => {
            Reported::Working
        }
        "PermissionRequest" => Reported::Blocked,
        "Stop" => Reported::Done,
        "Notification" if b.notification_type.as_deref() == Some("idle_prompt") => {
            Reported::AtPrompt
        }
        "SessionEnd" => Reported::Idle,
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
    Some(Event { state, conversation: b.session_id.filter(|s| !s.is_empty()), subagent })
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
pub fn admit(pin: &mut Option<String>, ev: &Event) -> bool {
    let Some(id) = ev.conversation.as_deref() else { return true };
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
        parse(v.to_string().as_bytes()).map(|e| e.state)
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
        let ev = parse(body.to_string().as_bytes()).unwrap();
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
            parse(json!({ "hook_event_name": event, "agent_id": "a1" }).to_string().as_bytes())
        };
        let blocked = sub("PermissionRequest").unwrap();
        assert_eq!((blocked.state, blocked.subagent), (Reported::Blocked, true));
        assert_eq!(sub("PostToolUse").unwrap().state, Reported::Working);
        assert!(!parse(br#"{"hook_event_name":"Stop"}"#).unwrap().subagent);
    }

    #[test]
    fn anything_unrecognised_is_ignored() {
        for event in ["SubagentStop", "SessionStart", "PreCompact", "stop", "Elicitation", ""] {
            assert_eq!(of(json!({ "hook_event_name": event })), None, "{event}");
        }
        assert_eq!(of(json!({})), None);
        assert_eq!(parse(b"not json"), None);
        assert_eq!(of(json!({ "hook_event_name": 3 })), None);
    }

    fn ev(state: Reported, conversation: &str) -> Event {
        Event { state, conversation: Some(conversation.to_string()), subagent: false }
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
        let ev = Event { state: Reported::Working, conversation: None, subagent: false };
        assert!(admit(&mut pin, &ev));
        assert_eq!(pin, None);
        let blank = parse(br#"{"hook_event_name":"Stop","session_id":""}"#).unwrap();
        assert_eq!(blank.conversation, None);
    }
}
