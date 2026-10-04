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
    /// No session in flight: it ended, or was cleared.
    Idle,
}

#[derive(Deserialize)]
struct Body {
    hook_event_name: Option<String>,
    notification_type: Option<String>,
    /// Set on events from a subagent. A run's state is its main agent's: four
    /// subagents finishing would otherwise read as four turns ending.
    agent_id: Option<serde_json::Value>,
}

/// The state a Claude Code hook body reports, or `None` for a body that says
/// nothing we act on. See `state_hooks::CLAUDE_EVENTS` for what each event was
/// verified to mean.
pub fn parse(body: &[u8]) -> Option<Reported> {
    let b: Body = serde_json::from_slice(body).ok()?;
    if b.agent_id.is_some_and(|v| !v.is_null()) {
        return None;
    }
    match b.hook_event_name.as_deref()? {
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure" => {
            Some(Reported::Working)
        }
        "PermissionRequest" => Some(Reported::Blocked),
        "Stop" => Some(Reported::Done),
        "Notification" if b.notification_type.as_deref() == Some("idle_prompt") => {
            Some(Reported::Done)
        }
        "SessionEnd" => Some(Reported::Idle),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn of(v: serde_json::Value) -> Option<Reported> {
        parse(v.to_string().as_bytes())
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
        assert_eq!(of(body), Some(Reported::Blocked));
    }

    #[test]
    fn only_the_idle_notification_counts() {
        let n = |t: &str| of(json!({ "hook_event_name": "Notification", "notification_type": t }));
        assert_eq!(n("idle_prompt"), Some(Reported::Done));
        // Arrives about six seconds after PermissionRequest, and would race an
        // answer given inside them.
        assert_eq!(n("permission_prompt"), None);
        assert_eq!(n("auth_success"), None);
        assert_eq!(of(json!({ "hook_event_name": "Notification" })), None);
    }

    #[test]
    fn a_subagent_never_speaks_for_the_run() {
        let body = json!({ "hook_event_name": "Stop", "agent_id": "ad32eb3b97e43b23c" });
        assert_eq!(of(body), None);
        let main = json!({ "hook_event_name": "Stop", "agent_id": null });
        assert_eq!(of(main), Some(Reported::Done), "a null agent_id is the main agent");
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
}
