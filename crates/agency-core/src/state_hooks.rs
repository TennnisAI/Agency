//! Lifecycle hooks that have an agent report its own state to Agency (AGE-206).
//!
//! A pane diff can say an agent is producing output and nothing more: a quiet
//! pane is an agent sitting on a permission prompt or an agent that finished
//! its turn, and those want opposite things from the user. The agent knows
//! which. This module writes the hooks that have it say so, posting each event
//! to the run's own loopback server (`crate::preview`, route [`STATE_PATH`]).
//! The receiving end and the parse are in `preview::report`.
//!
//! Claude Code only, so far. Its hooks take an `http` type, so nothing here is
//! a script written into the user's worktree or a `jq` assumed on the PATH:
//! the agent posts straight at the server. Every other agent stays on the pane
//! heuristic until its hooks are verified against a live binary, one agent per
//! change, the way `skills.rs` built its directory table.
//!
//! The same three rules as [`crate::mcp::emit_for_agent`]: merge into the file
//! and never replace it, leave a file the repo tracks alone, and exclude what
//! we write. The file is `.claude/settings.local.json`, which Claude Code
//! itself writes to (a "don't ask again" answer lands there as a permission
//! rule), so a file that does not parse is left exactly as it is rather than
//! started over.

use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;

/// The route on a run's server that hook events are posted to.
pub const STATE_PATH: &str = "/__agency__/state";
/// The variable naming the Agency session an agent process belongs to, and the
/// header the hook carries it in. A worktree holds every tab of its run, and
/// they all read the same settings file, so the URL alone cannot say which tab
/// is reporting; and a `claude` the user starts in their own terminal inside
/// the worktree has the variable unset, sends an empty header, and is ignored.
pub const SESSION_ENV: &str = "AGENCY_SESSION_ID";
pub const SESSION_HEADER: &str = "x-agency-session";
/// The secret a post has to carry for the server to act on it, handed to the
/// agent in its environment and never written into the worktree. The route
/// shares an origin with the app the preview proxies, and the session header
/// alone is a run id, which is in the worktree's path: any script on the
/// previewed page could post a dialog that holds the send queue for good, or a
/// `Stop` that clears a real one.
pub const TOKEN_ENV: &str = "AGENCY_STATE_TOKEN";
pub const TOKEN_HEADER: &str = "x-agency-token";
/// Where [`token`] keeps the secret, under the app's data directory.
const TOKEN_FILE: &str = "state-token";

/// Where Claude Code reads workspace-local settings.
const CLAUDE_SETTINGS: &[&str] = &[".claude", "settings.local.json"];

/// The hook events Agency subscribes to, with their matchers. Verified against
/// Claude Code 2.1.289 in an interactive session, not read off the docs:
///
/// - `PermissionRequest` fires the moment a permission dialog or an
///   `AskUserQuestion` question appears. `Notification` with
///   `permission_prompt` fires about six seconds later, and an answer given
///   inside those six seconds would race it, so it is not subscribed.
/// - Answering "No" to a permission dialog, or pressing Esc on one, fires
///   nothing at all. No `Stop`, no `PostToolUseFailure`. The way out of
///   `blocked` without a report is the app's (see `activity::answered`).
/// - `PostToolUseFailure` replaces `PostToolUse` when a tool fails.
/// - `Notification` with `idle_prompt` fires about sixty seconds after `Stop`,
///   and is the report that lands after an Esc left no other.
/// - `SessionEnd` fires on `/clear` as well as on exit.
/// - `SessionStart` never fires an `http` hook, so it is not here.
/// - `SubagentStop` fires after `Stop` for an internal subagent, carrying an
///   `agent_id`. It is not subscribed. A subagent's other events carry the
///   same `agent_id`, and the parse keeps only its tool calls and dialogs.
const CLAUDE_EVENTS: &[(&str, Option<&str>)] = &[
    ("UserPromptSubmit", None),
    ("PreToolUse", Some("*")),
    ("PostToolUse", Some("*")),
    ("PostToolUseFailure", Some("*")),
    ("PermissionRequest", Some("*")),
    ("Notification", Some("idle_prompt")),
    ("Stop", None),
    ("SessionEnd", None),
];

/// Seconds the agent waits on one post. The server answers before doing
/// anything else, so this only bites a wedged app. It is not an idle cost:
/// `async: true` is accepted on an `http` hook but not honoured (2.1.289 held a
/// tool call for the full 1.5s a test listener slept), so every post is waited
/// on, and the bound is what keeps a wedged app from holding up a turn.
const TIMEOUT_SECS: u64 = 2;

/// Whether Agency writes state hooks for this agent, by the recipe command its
/// profile runs (`claude` for any profile that launches Claude Code).
pub fn agent_supported(recipe_command: &str) -> bool {
    recipe_command == "claude"
}

fn hook_entry(port: u16) -> Value {
    json!({
        "type": "http",
        "url": format!("http://127.0.0.1:{port}{STATE_PATH}"),
        "timeout": TIMEOUT_SECS,
        "headers": {
            "X-Agency-Session": format!("${SESSION_ENV}"),
            "X-Agency-Token": format!("${TOKEN_ENV}"),
        },
        "allowedEnvVars": [SESSION_ENV, TOKEN_ENV],
    })
}

/// The install's state-hook secret, made on first use and kept in `data_dir`.
///
/// Kept rather than minted per start: an agent that survives an app restart in
/// the daemon goes on posting the token it was launched with. A directory we
/// cannot keep it in gets a token for this start alone, which costs those
/// agents their reports after a restart and nothing else.
pub fn token(data_dir: &Path) -> String {
    let path = data_dir.join(TOKEN_FILE);
    if let Ok(t) = std::fs::read_to_string(&path) {
        let t = t.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    let fresh = uuid::Uuid::new_v4().simple().to_string();
    let kept = crate::issuefs::atomic_write(&path, &format!("{fresh}\n")).and_then(|()| {
        use std::os::unix::fs::PermissionsExt;
        Ok(std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?)
    });
    if let Err(e) = kept {
        log::warn!("keeping the state-hook token in {}: {e}", path.display());
    }
    fresh
}

/// Whether `given` is the token, in time that does not depend on where the two
/// first differ.
pub fn token_matches(given: &str, token: &str) -> bool {
    given.len() == token.len()
        && given.bytes().zip(token.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// The matcher group Agency writes for one event.
fn our_group(matcher: Option<&str>, port: u16) -> Value {
    let mut group = json!({ "hooks": [hook_entry(port)] });
    if let Some(m) = matcher {
        group["matcher"] = json!(m);
    }
    group
}

/// Whether the settings already hold exactly the hooks we would write for
/// `port`, wherever in each event's list they sit. Taking ours out and putting
/// them back appends them, so without this a user's own group after ours read
/// as a change, and every launch rewrote a file Claude Code itself writes to.
fn already_current(root: &Value, port: u16) -> bool {
    let Some(events) = root.get("hooks").and_then(Value::as_object) else { return false };
    let mut found = Vec::new();
    for (event, groups) in events {
        let Some(groups) = groups.as_array() else { return false };
        for g in groups {
            let holds_ours =
                g.get("hooks").and_then(Value::as_array).is_some_and(|hs| hs.iter().any(is_ours));
            if !holds_ours {
                continue;
            }
            let Some((_, matcher)) = CLAUDE_EVENTS.iter().find(|(e, _)| e == event) else {
                return false;
            };
            if *g != our_group(*matcher, port) || found.contains(event) {
                return false;
            }
            found.push(event.clone());
        }
    }
    found.len() == CLAUDE_EVENTS.len()
}

/// Whether one hook entry is one of ours, on any port: a run whose port block
/// moved leaves the old port's entries behind, and they have to be recognised
/// to be replaced.
fn is_ours(hook: &Value) -> bool {
    hook.get("type").and_then(Value::as_str) == Some("http")
        && hook
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|u| u.starts_with("http://127.0.0.1:") && u.ends_with(STATE_PATH))
}

/// The settings file with our hooks pointed at `port`, or with them taken out
/// when `port` is `None`. `None` back means leave the file as it is: it already
/// says that, or it is not a shape we can merge into without guessing.
///
/// Pure, so the merge is tested without a worktree. The user's own hooks,
/// matcher groups and every other key survive untouched; ours are recognised by
/// their URL, wherever a previous emission put them.
pub fn merged(existing: Option<&str>, port: Option<u16>) -> Option<Value> {
    let mut root = match existing {
        None => json!({}),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(v) if v.is_object() => v,
            _ => return None,
        },
    };
    if !root.get("hooks").is_none_or(Value::is_object) {
        return None;
    }
    if port.is_some_and(|p| already_current(&root, p)) {
        return None;
    }
    let before = root.clone();
    let obj = root.as_object_mut()?;
    if let Some(events) = obj.get_mut("hooks").and_then(Value::as_object_mut) {
        for groups in events.values_mut() {
            let Some(groups) = groups.as_array_mut() else { continue };
            // Only a group or an event we emptied goes: one the user left
            // empty is theirs to keep.
            groups.retain_mut(|g| {
                let Some(hooks) = g.get_mut("hooks").and_then(Value::as_array_mut) else {
                    return true;
                };
                let had = hooks.len();
                hooks.retain(|h| !is_ours(h));
                hooks.len() == had || !hooks.is_empty()
            });
        }
        let original = before.get("hooks").and_then(Value::as_object);
        events.retain(|name, groups| {
            let was_empty = original
                .and_then(|s| s.get(name))
                .and_then(Value::as_array)
                .is_some_and(|a| a.is_empty());
            was_empty || !groups.as_array().is_some_and(|a| a.is_empty())
        });
    }
    if let Some(port) = port {
        let events = obj.entry("hooks").or_insert_with(|| json!({}));
        let events = events.as_object_mut()?;
        for (event, matcher) in CLAUDE_EVENTS {
            let groups = events.entry(event.to_string()).or_insert_with(|| json!([]));
            let groups = groups.as_array_mut()?;
            groups.push(our_group(*matcher, port));
        }
    }
    let left_empty = |v: Option<&Value>| v.and_then(Value::as_object).is_some_and(|h| h.is_empty());
    if left_empty(obj.get("hooks")) && !left_empty(before.get("hooks")) {
        obj.remove("hooks");
    }
    (root != before).then_some(root)
}

/// The settings file the hooks are written into.
pub fn settings_path(worktree: &Path) -> std::path::PathBuf {
    CLAUDE_SETTINGS.iter().fold(worktree.to_path_buf(), |p, s| p.join(s))
}

/// Whether the worktree's settings already carry our hooks. This is what keeps
/// a run's server up across an app restart: an agent still running in the
/// daemon read these hooks when it started, and posts to its port whether
/// anything listens there or not.
pub fn emitted(worktree: &Path) -> bool {
    let Some(root) = std::fs::read_to_string(settings_path(worktree))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
        return false;
    };
    let Some(events) = root.get("hooks").and_then(Value::as_object) else { return false };
    events
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .any(|g| g.get("hooks").and_then(Value::as_array).is_some_and(|hs| hs.iter().any(is_ours)))
}

/// Write our hooks into the worktree for an agent launched by `recipe_command`,
/// pointed at `port`, or take them out with `None`.
///
/// `None` is for a run whose server will not be up. With nothing listening the
/// agent pays no time, since loopback refuses at once, but Claude Code prints
/// "PreToolUse:Bash hook error ... ECONNREFUSED" into the transcript on every
/// tool call (2.1.289), so a stale block is worse than none.
pub fn emit_for_agent(
    recipe_command: &str,
    worktree: &Path,
    repo_root: &Path,
    port: Option<u16>,
) -> Result<bool> {
    if !agent_supported(recipe_command) {
        return Ok(false);
    }
    write(worktree, repo_root, port)
}

/// Take our hooks out of the worktree, whichever agent they were written for:
/// for a run whose agents are all gone, so nothing is left to post to them.
pub fn withdraw(worktree: &Path, repo_root: &Path) -> Result<bool> {
    write(worktree, repo_root, None)
}

fn write(worktree: &Path, repo_root: &Path, port: Option<u16>) -> Result<bool> {
    let rel = CLAUDE_SETTINGS.join("/");
    if crate::mcp::tracked(worktree, &rel) {
        log::info!("{rel} is tracked in {}: not writing state hooks into it", worktree.display());
        return Ok(false);
    }
    let path = settings_path(worktree);
    // Only a missing file is an absent one. Read as absent, a file we could
    // not read (not ours to read, or holding a byte that is not UTF-8) was
    // merged from `{}`, and the swap below replaced the user's permission
    // rules and hooks with ours alone.
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context(format!("reading {}: leaving it alone", path.display())))
        }
    };
    if existing.is_none() && port.is_none() {
        return Ok(false);
    }
    let Some(root) = merged(existing.as_deref(), port) else {
        if existing.as_deref().is_some_and(|t| serde_json::from_str::<Value>(t).is_err()) {
            log::warn!("{} does not parse: leaving it alone, no state hooks", path.display());
        }
        return Ok(false);
    };
    crate::issuefs::atomic_write(&path, &(serde_json::to_string_pretty(&root)? + "\n"))?;
    // Excluded for the reason AGE-115 recorded against `.mcp.json`: the one
    // worktree file Agency wrote without an exclude was the one an agent's
    // `git add -A` staged and the merge carried into the project.
    if port.is_some() {
        if let Err(e) = crate::worktree::ensure_exclude_pattern(repo_root, &format!("/{rel}")) {
            log::warn!("excluding {rel} in {}: {e}", repo_root.display());
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours_in(v: &Value) -> Vec<(String, Option<String>, String)> {
        let mut out = Vec::new();
        for (event, groups) in v["hooks"].as_object().unwrap() {
            for g in groups.as_array().unwrap() {
                for h in g["hooks"].as_array().unwrap() {
                    if is_ours(h) {
                        out.push((
                            event.clone(),
                            g.get("matcher").and_then(Value::as_str).map(str::to_string),
                            h["url"].as_str().unwrap().to_string(),
                        ));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn a_fresh_file_gets_every_event_pointed_at_the_port() {
        let v = merged(None, Some(5249)).unwrap();
        let ours = ours_in(&v);
        assert_eq!(ours.len(), CLAUDE_EVENTS.len());
        assert!(ours.iter().all(|(_, _, url)| url == "http://127.0.0.1:5249/__agency__/state"));
        let blocked = ours.iter().find(|(e, _, _)| e == "PermissionRequest").unwrap();
        assert_eq!(blocked.1.as_deref(), Some("*"));
        let idle = ours.iter().find(|(e, _, _)| e == "Notification").unwrap();
        assert_eq!(idle.1.as_deref(), Some("idle_prompt"));
        let stop = ours.iter().find(|(e, _, _)| e == "Stop").unwrap();
        assert_eq!(stop.1, None, "Stop takes no matcher");
    }

    #[test]
    fn the_entry_carries_the_session_through_an_allowed_variable() {
        let v = merged(None, Some(5249)).unwrap();
        let h = &v["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(h["type"], "http");
        assert_eq!(h["headers"]["X-Agency-Session"], "$AGENCY_SESSION_ID");
        assert_eq!(h["headers"]["X-Agency-Token"], "$AGENCY_STATE_TOKEN");
        assert_eq!(h["allowedEnvVars"], json!(["AGENCY_SESSION_ID", "AGENCY_STATE_TOKEN"]));
        assert_eq!(h["timeout"], 2);
    }

    #[test]
    fn the_users_own_settings_and_hooks_survive() {
        let theirs = json!({
            "permissions": { "allow": ["Bash(npm test)"] },
            "hooks": {
                "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }],
                "PreCompact": [{ "hooks": [{ "type": "command", "command": "true" }] }],
            },
        });
        let v = merged(Some(&theirs.to_string()), Some(5249)).unwrap();
        assert_eq!(v["permissions"], theirs["permissions"]);
        assert_eq!(v["hooks"]["PreCompact"], theirs["hooks"]["PreCompact"]);
        assert_eq!(v["hooks"]["Stop"][0], theirs["hooks"]["Stop"][0], "theirs stays first");
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn emitting_twice_changes_nothing_and_a_new_port_replaces_the_old() {
        let once = merged(None, Some(5249)).unwrap();
        let text = once.to_string();
        assert_eq!(merged(Some(&text), Some(5249)), None, "already says that");
        let moved = merged(Some(&text), Some(5259)).unwrap();
        let ours = ours_in(&moved);
        assert_eq!(ours.len(), CLAUDE_EVENTS.len(), "replaced, not added beside");
        assert!(ours.iter().all(|(_, _, url)| url.contains(":5259/")));
    }

    #[test]
    fn a_users_group_after_ours_is_not_a_change() {
        let mut v = merged(None, Some(5249)).unwrap();
        v["hooks"]["Stop"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "hooks": [{ "type": "command", "command": "say done" }] }));
        assert_eq!(merged(Some(&v.to_string()), Some(5249)), None, "nothing to rewrite");
        // A port move still rewrites, and keeps the user's group.
        let moved = merged(Some(&v.to_string()), Some(5259)).unwrap();
        assert_eq!(moved["hooks"]["Stop"][0]["hooks"][0]["command"], "say done");
        assert!(ours_in(&moved).iter().all(|(_, _, url)| url.contains(":5259/")));
    }

    #[test]
    fn a_hand_edited_hook_of_ours_is_put_right() {
        let mut v = merged(None, Some(5249)).unwrap();
        v["hooks"]["Stop"][0]["hooks"][0]["timeout"] = json!(30);
        let fixed = merged(Some(&v.to_string()), Some(5249)).unwrap();
        assert_eq!(fixed["hooks"]["Stop"][0]["hooks"][0]["timeout"], 2);
        // One event dropped by hand comes back.
        let mut v = merged(None, Some(5249)).unwrap();
        v["hooks"].as_object_mut().unwrap().remove("SessionEnd");
        let fixed = merged(Some(&v.to_string()), Some(5249)).unwrap();
        assert_eq!(ours_in(&fixed).len(), CLAUDE_EVENTS.len());
    }

    #[test]
    fn taking_them_out_leaves_only_what_the_user_had() {
        let theirs = json!({
            "permissions": { "allow": [] },
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }] },
        });
        let with = merged(Some(&theirs.to_string()), Some(5249)).unwrap();
        let without = merged(Some(&with.to_string()), None).unwrap();
        assert_eq!(without, theirs);
        // A file that only ever held ours goes back to holding nothing.
        let only = merged(None, Some(5249)).unwrap();
        assert_eq!(merged(Some(&only.to_string()), None).unwrap(), json!({}));
        assert_eq!(merged(Some("{}"), None), None, "nothing of ours to take out");
    }

    #[test]
    fn a_user_hook_sharing_a_group_with_ours_keeps_the_group() {
        let mut v = merged(None, Some(5249)).unwrap();
        v["hooks"]["Stop"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "type": "command", "command": "say done" }));
        let out = merged(Some(&v.to_string()), None).unwrap();
        assert_eq!(
            out["hooks"]["Stop"],
            json!([{ "hooks": [{ "type": "command", "command": "say done" }] }])
        );
    }

    #[test]
    fn a_file_we_cannot_read_safely_is_left_alone() {
        assert_eq!(merged(Some("{ not json"), Some(5249)), None);
        assert_eq!(merged(Some("[]"), Some(5249)), None, "not an object");
        assert_eq!(merged(Some(r#"{"hooks": []}"#), Some(5249)), None, "hooks not an object");
        assert_eq!(merged(Some(r#"{"hooks": {"Stop": {}}}"#), Some(5249)), None);
    }

    #[test]
    fn a_hook_elsewhere_on_the_same_path_is_not_ours() {
        let theirs = json!({ "hooks": { "Stop": [{ "hooks": [
            { "type": "http", "url": "https://example.com/__agency__/state" },
        ] }] } });
        let out = merged(Some(&theirs.to_string()), None);
        assert_eq!(out, None, "nothing of ours in it");
    }

    #[test]
    fn only_claude_gets_hooks() {
        assert!(agent_supported("claude"));
        for other in ["codex", "copilot", "cursor-agent", "opencode", "gemini", "pi"] {
            assert!(!agent_supported(other), "{other}");
        }
    }

    #[test]
    fn emit_writes_excludes_and_reports_the_file_as_emitted() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        std::process::Command::new("git").args(["init", "-q"]).current_dir(repo).status().unwrap();
        assert!(!emitted(repo));
        assert!(emit_for_agent("claude", repo, repo, Some(5249)).unwrap());
        assert!(emitted(repo));
        let exclude = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
        assert!(exclude.lines().any(|l| l == "/.claude/settings.local.json"), "{exclude}");
        assert!(!emit_for_agent("claude", repo, repo, Some(5249)).unwrap(), "no rewrite");
        assert!(withdraw(repo, repo).unwrap());
        assert!(!emitted(repo));
        assert!(!withdraw(repo, repo).unwrap(), "nothing left to take out");
        assert!(!emit_for_agent("codex", repo, repo, Some(5249)).unwrap());
    }

    #[test]
    fn emit_leaves_a_tracked_settings_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git").args(args).current_dir(repo).output().unwrap()
        };
        git(&["init", "-q"]);
        std::fs::create_dir_all(repo.join(".claude")).unwrap();
        std::fs::write(repo.join(".claude/settings.local.json"), "{}\n").unwrap();
        git(&["add", "-f", ".claude/settings.local.json"]);
        assert!(!emit_for_agent("claude", repo, repo, Some(5249)).unwrap());
        assert_eq!(
            std::fs::read_to_string(repo.join(".claude/settings.local.json")).unwrap(),
            "{}\n"
        );
    }

    #[test]
    fn the_token_is_kept_across_starts_and_compared_whole() {
        let dir = tempfile::tempdir().unwrap();
        let t = token(dir.path());
        assert_eq!(t.len(), 32);
        assert_eq!(token(dir.path()), t, "an agent from the last start still matches");
        let mode = std::fs::metadata(dir.path().join(TOKEN_FILE)).unwrap().permissions();
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777, 0o600);
        assert!(token_matches(&t, &t));
        assert!(!token_matches("", &t));
        assert!(!token_matches(&t[..31], &t));
        assert!(!token_matches(&format!("{}x", &t[..31]), &t));
    }

    #[test]
    fn a_settings_file_that_is_not_utf8_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        std::process::Command::new("git").args(["init", "-q"]).current_dir(repo).status().unwrap();
        std::fs::create_dir_all(repo.join(".claude")).unwrap();
        let path = repo.join(".claude/settings.local.json");
        let original = b"{\"permissions\":{\"allow\":[\"Bash(\xff)\"]}}\n".to_vec();
        std::fs::write(&path, &original).unwrap();
        assert!(emit_for_agent("claude", repo, repo, Some(5249)).is_err());
        assert!(withdraw(repo, repo).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn emit_with_no_port_and_no_file_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!emit_for_agent("claude", dir.path(), dir.path(), None).unwrap());
        assert!(!dir.path().join(".claude").exists());
    }
}
