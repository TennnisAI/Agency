//! Cross-harness MCP configuration.
//!
//! Agency keeps one canonical list of MCP servers (global app settings merged
//! with the project's `[mcp.servers.*]` in `.agency/agency.toml`) and emits it
//! into each new worktree in the *native* format of the agent that will run
//! there — `.mcp.json` for Claude Code and Copilot CLI, `.cursor/mcp.json` for
//! Cursor, `opencode.json` for OpenCode. An untracked file already sitting in
//! the worktree is merged into (our entries upserted by name), never replaced;
//! a file the repo *tracks* is left alone entirely, and the config is excluded
//! from git either way — see [`emit_for_agent`]. Agents whose MCP config is
//! global-only (Codex's ~/.codex/config.toml) are intentionally skipped: Agency
//! never mutates files outside the workspace. So is DeepSeek Harness, whose
//! MCP servers are plugin rows in its Cordis composition rather than any
//! config file — there is no per-worktree artifact to write, and emitting a
//! composition patch would be writing against a plugin API its README promises
//! to break.
//!
//! Writing the file is not always enough to have it read: [`launch_args`] adds
//! whatever the agent needs on its command line to actually load what we wrote
//! (Copilot, whose workspace config is gated behind folder trust, is handed the
//! file with `--additional-mcp-config`).
//!
//! OAuth servers can't live in per-worktree config — their signed-in session is
//! held by the agent CLI at *its* user scope. The Authenticate flow registers
//! such a server with one agent CLI and records that in `user_scope_agents`, so
//! emission is suppressed **only for that agent**. Every other agent keeps
//! getting the server written into its workspace config.
//!
//! This module only ever emits config; the one MCP server Agency *runs* is the
//! per-run preview server ([`crate::preview`]), whose entry rides into the same
//! merged list rather than through a second emission path.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The wire transport a remote MCP server speaks. `Stdio` is the local
/// command-spawned case. When unset on a server we infer it (command → stdio,
/// url → http) so older configs keep working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Http,
    Sse,
}

/// One MCP server definition. Exactly one of `command` (stdio transport) or
/// `url` (remote transport) should be set.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub name: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
    /// Explicit transport; inferred from command/url when `None`.
    #[serde(default)]
    pub transport: Option<McpTransport>,
    /// HTTP headers for remote transports (e.g. `Authorization`). Ignored for stdio.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Agent CLIs this server is registered with at *their* own user scope (via
    /// the Authenticate flow). The OAuth session lives there and persists across
    /// worktrees, so Agency must not re-emit the server into that agent's
    /// per-worktree config — but every other agent still gets it.
    #[serde(default)]
    pub user_scope_agents: Vec<String>,
    /// Legacy shape of the field above, from when Agency could only authenticate
    /// with Claude and the flag was global. Read on load and folded into
    /// `user_scope_agents` by [`normalize`]; never written back.
    #[serde(default, skip_serializing_if = "is_false")]
    pub user_scope: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl McpServer {
    /// Whether `agent` gets this server from its own user-scope config rather
    /// than from the per-worktree file Agency writes.
    pub fn is_user_scope_for(&self, agent: &str) -> bool {
        self.user_scope_agents.iter().any(|a| a == agent)
    }

    /// Fold the legacy global `user_scope` flag into the per-agent list. A
    /// server saved before per-agent scope existed was necessarily registered
    /// with Claude — that was the only agent Authenticate supported.
    pub fn normalize(&mut self) {
        if self.user_scope {
            self.user_scope = false;
            if !self.is_user_scope_for("claude") {
                self.user_scope_agents.push("claude".into());
            }
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("MCP server needs a name");
        }
        match (&self.command, &self.url) {
            (Some(c), None) if !c.trim().is_empty() => Ok(()),
            (None, Some(u)) if !u.trim().is_empty() => Ok(()),
            _ => bail!("MCP server '{}' needs either a command or a url (not both)", self.name),
        }
    }

    /// The effective transport: explicit if set, else inferred from which of
    /// command/url is present. Defaults to stdio when neither is set.
    pub fn effective_transport(&self) -> McpTransport {
        self.transport.unwrap_or(if self.url.is_some() {
            McpTransport::Http
        } else {
            McpTransport::Stdio
        })
    }
}

/// Flatten the project config's `[mcp.servers.<name>]` tables into servers.
pub fn from_config(cfg: &crate::config::McpConfig) -> Vec<McpServer> {
    cfg.servers
        .iter()
        .map(|(name, def)| {
            let mut s = McpServer {
                name: name.clone(),
                command: def.command.clone(),
                args: def.args.clone(),
                env: def.env.clone(),
                url: def.url.clone(),
                transport: def.transport,
                headers: def.headers.clone(),
                user_scope_agents: def.user_scope_agents.clone(),
                user_scope: def.user_scope,
            };
            s.normalize();
            s
        })
        .collect()
}

/// Parse a standard / VS Code `mcp.json` document into servers. Accepts either
/// root key (`mcpServers`, as Claude/Cursor use, or `servers`, as VS Code uses),
/// or a bare `{ name: def }` map. Each entry's `type`/`command`/`args`/`env`/
/// `url`/`headers` map onto [`McpServer`]; `inputs` and unknown keys are ignored.
/// Entries with neither a command nor a url are skipped.
pub fn import_json(text: &str) -> Result<Vec<McpServer>> {
    let root: serde_json::Value = serde_json::from_str(text)?;
    let obj = root.as_object().ok_or_else(|| anyhow::anyhow!("mcp.json must be a JSON object"))?;
    // The server map is under mcpServers/servers, or the object is the map itself.
    let servers = obj
        .get("mcpServers")
        .or_else(|| obj.get("servers"))
        .and_then(|v| v.as_object())
        .unwrap_or(obj);

    let mut out = Vec::new();
    for (name, def) in servers {
        let Some(def) = def.as_object() else { continue };
        let str_at = |k: &str| def.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let map_at = |k: &str| -> BTreeMap<String, String> {
            def.get(k)
                .and_then(|v| v.as_object())
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default()
        };
        let args = def
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let transport = match def.get("type").and_then(|v| v.as_str()) {
            Some("sse") => Some(McpTransport::Sse),
            Some("http") | Some("streamable-http") => Some(McpTransport::Http),
            Some("stdio") => Some(McpTransport::Stdio),
            _ => None,
        };
        let server = McpServer {
            name: name.clone(),
            command: str_at("command"),
            args,
            env: map_at("env"),
            url: str_at("url"),
            transport,
            headers: map_at("headers"),
            ..Default::default()
        };
        if server.command.is_some() || server.url.is_some() {
            out.push(server);
        }
    }
    Ok(out)
}

/// Merge server lists; later lists win on name conflicts (callers pass
/// global first, then project, then auto-injected entries).
pub fn merge(lists: &[Vec<McpServer>]) -> Vec<McpServer> {
    let mut by_name: BTreeMap<String, McpServer> = BTreeMap::new();
    for list in lists {
        for s in list {
            by_name.insert(s.name.clone(), s.clone());
        }
    }
    by_name.into_values().collect()
}

/// Where and how `agent`'s per-workspace MCP config is written: relative path
/// segments, root key, and per-server formatter. `None` means the agent has no
/// per-workspace config format Agency can emit.
fn emit_target(
    agent: &str,
) -> Option<(&'static [&'static str], &'static str, fn(&McpServer) -> serde_json::Value)> {
    match agent {
        "claude" | "copilot" => Some((&[".mcp.json"], "mcpServers", claude_entry)),
        "cursor" => Some((&[".cursor", "mcp.json"], "mcpServers", cursor_entry)),
        "opencode" => Some((&["opencode.json"], "mcp", opencode_entry)),
        _ => None,
    }
}

/// Whether Agency can emit per-workspace MCP config for this agent.
pub fn agent_supported(agent: &str) -> bool {
    emit_target(agent).is_some()
}

/// What keeps `agent`'s emitted config out of the run's diff, derived from the
/// same [`emit_target`] entry that decided where to write it, so a new agent
/// format cannot land without its exclude. Anchored (`/.mcp.json`,
/// `/.cursor/mcp.json`, `/opencode.json`) so it only ever hides the repo root's
/// copy, never a same-named file somewhere down the tree.
fn exclude_pattern(segments: &[&str]) -> String {
    format!("/{}", segments.join("/"))
}

/// Whether git tracks `rel` in this worktree. A tracked config is the repo's
/// own; see [`crate::briefing`] and [`crate::skills`], which skip on the same
/// test.
fn tracked(worktree: &Path, rel: &str) -> bool {
    std::process::Command::new("git")
        .args(["ls-files", "--", rel])
        .current_dir(worktree)
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}

/// The flag `agent` takes to be handed an MCP config file outright, instead of
/// discovering the workspace file on its own. `None` (every agent but Copilot)
/// means the emitted file is picked up without help.
///
/// Copilot CLI reads workspace config (`.mcp.json`) only once the user approves
/// folder trust for that directory, and Agency cuts a fresh, never-trusted
/// worktree per run — so the servers it wrote sit there ignored until the user
/// notices the trust prompt and accepts it, which reads as "MCP isn't working".
/// `--additional-mcp-config` takes "JSON string or file path (prefix with @)"
/// per `copilot --help`, is honoured regardless of trust, and takes precedence
/// over the file sources.
///
/// Read out of Copilot CLI 1.0.78's own bundle, where one config load takes both
/// and gates only the workspace half on trust:
/// `includeWorkspaceSources: COPILOT_ALLOW_ALL === "true" || folderTrustIsTrusted(cwd)`,
/// while `additionalConfig` is passed unconditionally and merged in last. (The
/// env var is the other way to unlock the workspace file, and not one worth
/// taking: it turns off every permission prompt Copilot has.)
fn launch_flag(agent: &str) -> Option<&'static str> {
    match agent {
        "copilot" => Some("--additional-mcp-config"),
        _ => None,
    }
}

/// Extra launch arguments that point `agent` at the workspace MCP config in
/// `worktree`, for the agents that need to be told (see [`launch_flag`]). Empty
/// for everyone else, and empty when the file isn't there — an `@path` naming a
/// missing file is worse than no flag at all: Copilot reads it eagerly at
/// startup and dies on the read error ("Failed to read MCP config file").
///
/// That file is the one [`emit_for_agent`] writes, so it holds Agency's servers
/// and any the repo committed alongside them; handing it over covers both,
/// which is what the agent would have read anyway once trust was granted.
///
/// This is Agency's own slice of the argv, kept apart from the profile's
/// user-edited `args` so neither can clobber the other. Callers skip it when
/// the user already passes the same flag themselves; see
/// [`launch_args_unless_set`].
pub fn launch_args(agent: &str, worktree: &Path) -> Vec<String> {
    let Some(flag) = launch_flag(agent) else {
        return Vec::new();
    };
    let Some((segments, ..)) = emit_target(agent) else {
        return Vec::new();
    };
    let path = segments.iter().fold(worktree.to_path_buf(), |p, s| p.join(s));
    if !path.is_file() {
        return Vec::new();
    }
    vec![flag.to_string(), format!("@{}", path.display())]
}

/// [`launch_args`], unless `existing` (the profile's own args, a resume recipe,
/// a loop recipe) already carries the flag — a user who set it by hand meant
/// their value, and a second copy would either be ignored or fight it.
pub fn launch_args_unless_set(agent: &str, worktree: &Path, existing: &[String]) -> Vec<String> {
    match launch_flag(agent) {
        // Matches `--flag=value` as well as the separate-argument spelling.
        Some(flag) if existing.iter().any(|a| a == flag || a.starts_with(&format!("{flag}="))) => {
            Vec::new()
        }
        _ => launch_args(agent, worktree),
    }
}

/// Agent CLIs whose own user-scope MCP registration Agency can drive (the
/// Authenticate flow). Both take a remote server on the command line and hold
/// the resulting OAuth session in their user config; the interactive sign-in
/// itself happens in the agent's own `/mcp` UI.
pub const AUTH_AGENTS: &[&str] = &["claude", "copilot"];

/// Whether [`auth_argv`] knows how to register a server with `agent`'s CLI.
pub fn auth_supported(agent: &str) -> bool {
    AUTH_AGENTS.contains(&agent)
}

/// The argv that registers `server` with `agent`'s CLI at that CLI's user
/// scope, ready to spawn without a shell (so user-supplied values need no
/// quoting). Errors for agents with no such command, or for a stdio server —
/// Authenticate exists for remote servers that need an interactive sign-in.
pub fn auth_argv(agent: &str, server: &McpServer) -> Result<(String, Vec<String>)> {
    let url = server.url.as_deref().map(str::trim).filter(|u| !u.is_empty()).ok_or_else(|| {
        anyhow::anyhow!(
            "Authenticate is for remote (url) servers; '{}' is a stdio server",
            server.name
        )
    })?;
    let transport = match server.effective_transport() {
        McpTransport::Sse => "sse",
        _ => "http",
    };
    let (command, mut args) = match agent {
        // `--scope user` is Claude's opt-in; Copilot's `mcp add` is user-scoped
        // already (its help: "Add a new MCP server to the user configuration").
        "claude" => (
            "claude",
            vec![
                "mcp".to_string(),
                "add".into(),
                "--scope".into(),
                "user".into(),
                "--transport".into(),
                transport.into(),
            ],
        ),
        "copilot" => (
            "copilot",
            vec!["mcp".to_string(), "add".into(), "--transport".into(), transport.into()],
        ),
        _ => bail!(
            "Agency can't register MCP servers with {agent}; add '{}' using {agent}'s own CLI",
            server.name
        ),
    };
    args.push(server.name.clone());
    args.push(url.to_string());
    for (k, v) in &server.headers {
        args.push("--header".into());
        args.push(format!("{k}: {v}"));
    }
    Ok((command.to_string(), args))
}

/// Write the servers into `worktree` in `agent`'s native config format, and
/// keep the file out of git so no agent commits it into the project.
///
/// Returns false (and writes nothing) for agents Agency can't configure
/// per-workspace, and for a repo that tracks the config file itself: a diff on
/// a tracked file rides into the project's main branch at merge, which is not a
/// change Agency gets to make. Invalid entries are skipped rather than failing
/// the run.
pub fn emit_for_agent(
    agent: &str,
    worktree: &Path,
    repo_root: &Path,
    servers: &[McpServer],
) -> Result<bool> {
    // Skip servers registered with *this* agent's CLI directly (Authenticate
    // flow) so its persisted OAuth session applies, and re-emitting a
    // project-scoped copy would only trigger an untrusted-server approval.
    // Servers authenticated with a *different* agent still get emitted here.
    let valid: Vec<&McpServer> =
        servers.iter().filter(|s| !s.is_user_scope_for(agent) && s.validate().is_ok()).collect();
    if valid.is_empty() {
        return Ok(false);
    }
    let Some((segments, root_key, entry)) = emit_target(agent) else {
        return Ok(false);
    };
    let rel = segments.join("/");
    if tracked(worktree, &rel) {
        log::info!(
            "{rel} is tracked in {}: leaving the repo's own MCP config alone",
            worktree.display()
        );
        return Ok(false);
    }
    let path = segments.iter().fold(worktree.to_path_buf(), |p, s| p.join(s));
    let wrote = upsert_json(&path, root_key, &valid, entry)?;
    // AGE-115: this was the only one of Agency's three worktree file drops with no
    // exclude, so on a repo that did not already track `.mcp.json` the first
    // `git add -A` an agent ran staged Agency's MCP config and merging the run
    // put it in the project. Excluding costs the user nothing: `git add -f`
    // still wins, and an exclude has no say over a file once it is tracked
    // (which is why the skip above, not the exclude, is what protects a repo's
    // own committed config).
    if wrote {
        if let Err(e) =
            crate::worktree::ensure_exclude_pattern(repo_root, &exclude_pattern(segments))
        {
            log::warn!("excluding {rel} in {}: {e}", repo_root.display());
        }
    }
    Ok(wrote)
}

/// The `"type"` string Claude/standard `.mcp.json` uses for a remote transport.
fn remote_type_str(t: McpTransport) -> &'static str {
    match t {
        McpTransport::Sse => "sse",
        // http is the modern streamable-HTTP transport; stdio never reaches here.
        _ => "http",
    }
}

fn claude_entry(s: &McpServer) -> serde_json::Value {
    match &s.url {
        Some(url) => {
            let mut v =
                serde_json::json!({ "type": remote_type_str(s.effective_transport()), "url": url });
            if !s.headers.is_empty() {
                v["headers"] = serde_json::json!(s.headers);
            }
            v
        }
        // Explicit "type" so Copilot (which shares this file) never has to
        // rely on its default; "stdio" is the standard name both CLIs accept.
        None => serde_json::json!({
            "type": "stdio",
            "command": s.command.clone().unwrap_or_default(),
            "args": s.args,
            "env": s.env,
        }),
    }
}

fn cursor_entry(s: &McpServer) -> serde_json::Value {
    match &s.url {
        Some(url) => {
            let mut v = serde_json::json!({ "url": url });
            if !s.headers.is_empty() {
                v["headers"] = serde_json::json!(s.headers);
            }
            v
        }
        None => serde_json::json!({
            "command": s.command.clone().unwrap_or_default(),
            "args": s.args,
            "env": s.env,
        }),
    }
}

fn opencode_entry(s: &McpServer) -> serde_json::Value {
    match &s.url {
        Some(url) => {
            let mut v = serde_json::json!({ "type": "remote", "url": url, "enabled": true });
            if !s.headers.is_empty() {
                v["headers"] = serde_json::json!(s.headers);
            }
            v
        }
        None => {
            let mut command = vec![s.command.clone().unwrap_or_default()];
            command.extend(s.args.iter().cloned());
            serde_json::json!({
                "type": "local",
                "command": command,
                "environment": s.env,
                "enabled": true,
            })
        }
    }
}

/// Upsert our servers under `root_key` of the JSON file at `path`, preserving
/// everything else in the file (repo-committed servers, unrelated top-level
/// keys). A missing or unparseable file starts from an empty object.
fn upsert_json(
    path: &Path,
    root_key: &str,
    servers: &[&McpServer],
    entry: fn(&McpServer) -> serde_json::Value,
) -> Result<bool> {
    let mut root: serde_json::Value = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let obj = root.as_object_mut().unwrap();
    let bucket = obj.entry(root_key.to_string()).or_insert_with(|| serde_json::json!({}));
    if !bucket.is_object() {
        *bucket = serde_json::json!({});
    }
    let bucket = bucket.as_object_mut().unwrap();
    for s in servers {
        bucket.insert(s.name.clone(), entry(s));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&root)? + "\n")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio(name: &str, command: &str) -> McpServer {
        McpServer {
            name: name.into(),
            command: Some(command.into()),
            args: vec!["--flag".into()],
            env: BTreeMap::from([("KEY".to_string(), "V".to_string())]),
            ..Default::default()
        }
    }

    fn remote(name: &str, url: &str) -> McpServer {
        McpServer { name: name.into(), url: Some(url.into()), ..Default::default() }
    }

    #[test]
    fn validate_requires_exactly_one_transport() {
        assert!(stdio("a", "cmd").validate().is_ok());
        assert!(remote("b", "https://x").validate().is_ok());
        let neither = McpServer { name: "n".into(), ..Default::default() };
        assert!(neither.validate().is_err());
        let mut both = stdio("c", "cmd");
        both.url = Some("https://x".into());
        assert!(both.validate().is_err());
    }

    #[test]
    fn merge_later_lists_win_by_name() {
        let global = vec![stdio("shared", "global-cmd"), stdio("only-global", "g")];
        let project = vec![stdio("shared", "project-cmd")];
        let merged = merge(&[global, project]);
        assert_eq!(merged.len(), 2);
        let shared = merged.iter().find(|s| s.name == "shared").unwrap();
        assert_eq!(shared.command.as_deref(), Some("project-cmd"));
    }

    #[test]
    fn emits_claude_format_and_preserves_existing_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(
            &path,
            r#"{"mcpServers":{"repo-own":{"command":"keep-me","args":[]}},"other":1}"#,
        )
        .unwrap();

        let wrote = emit_for_agent(
            "claude",
            dir.path(),
            dir.path(),
            &[stdio("mine", "my-cmd"), remote("api", "https://mcp.example")],
        )
        .unwrap();
        assert!(wrote);

        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["other"], 1, "unrelated keys survive");
        assert_eq!(
            root["mcpServers"]["repo-own"]["command"], "keep-me",
            "existing servers survive"
        );
        assert_eq!(root["mcpServers"]["mine"]["type"], "stdio");
        assert_eq!(root["mcpServers"]["mine"]["command"], "my-cmd");
        assert_eq!(root["mcpServers"]["mine"]["env"]["KEY"], "V");
        assert_eq!(root["mcpServers"]["api"]["type"], "http");
        assert_eq!(root["mcpServers"]["api"]["url"], "https://mcp.example");
    }

    #[test]
    fn emits_copilot_into_project_mcp_json() {
        let dir = tempfile::tempdir().unwrap();
        let wrote = emit_for_agent(
            "copilot",
            dir.path(),
            dir.path(),
            &[stdio("kg", "graphify"), remote("api", "https://mcp.example")],
        )
        .unwrap();
        assert!(wrote);
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap())
                .unwrap();
        assert_eq!(root["mcpServers"]["kg"]["type"], "stdio");
        assert_eq!(root["mcpServers"]["kg"]["command"], "graphify");
        assert_eq!(root["mcpServers"]["api"]["type"], "http");
        assert_eq!(root["mcpServers"]["api"]["url"], "https://mcp.example");
    }

    /// AGE-71: Copilot gates workspace `.mcp.json` behind folder trust, and a
    /// fresh worktree is never trusted, so the file has to be handed to it.
    #[test]
    fn copilot_launch_args_point_at_the_emitted_file() {
        let dir = tempfile::tempdir().unwrap();
        // Nothing written yet: an `@path` to a missing file is worse than no flag.
        assert!(launch_args("copilot", dir.path()).is_empty());

        emit_for_agent("copilot", dir.path(), dir.path(), &[stdio("kg", "graphify")]).unwrap();
        let path = dir.path().join(".mcp.json");
        assert_eq!(
            launch_args("copilot", dir.path()),
            vec!["--additional-mcp-config".to_string(), format!("@{}", path.display())]
        );

        // Every other agent reads what Agency wrote without being told.
        for agent in ["claude", "cursor", "opencode", "codex", "gemini"] {
            assert!(launch_args(agent, dir.path()).is_empty(), "{agent}");
        }
    }

    #[test]
    fn launch_args_yield_to_a_flag_the_user_set_themselves() {
        let dir = tempfile::tempdir().unwrap();
        emit_for_agent("copilot", dir.path(), dir.path(), &[stdio("kg", "graphify")]).unwrap();
        assert!(!launch_args_unless_set("copilot", dir.path(), &[]).is_empty());
        for existing in [
            vec!["--additional-mcp-config".to_string(), "@/my/own.json".to_string()],
            vec!["--additional-mcp-config=@/my/own.json".to_string()],
        ] {
            assert!(
                launch_args_unless_set("copilot", dir.path(), &existing).is_empty(),
                "{existing:?}"
            );
        }
        // An unrelated flag is no reason to stand down.
        let unrelated = vec!["--allow-all-tools".to_string()];
        assert!(!launch_args_unless_set("copilot", dir.path(), &unrelated).is_empty());
    }

    #[test]
    fn agent_supported_matches_emit_targets() {
        for agent in ["claude", "copilot", "cursor", "opencode"] {
            assert!(agent_supported(agent), "{agent} should be supported");
        }
        // dsh is deliberate, not an omission: its MCP servers are rows in a
        // Cordis plugin composition, not a config file, so there is no
        // per-worktree artifact Agency could write for it.
        for agent in ["codex", "gemini", "kimi", "crush", "hermes", "pi", "dsh", "custom-profile"] {
            assert!(!agent_supported(agent), "{agent} should not be supported");
        }
    }

    #[test]
    fn claude_remote_honors_sse_transport_and_headers() {
        let dir = tempfile::tempdir().unwrap();
        let server = McpServer {
            name: "atlassian".into(),
            url: Some("https://mcp.atlassian.com/v1/sse".into()),
            transport: Some(McpTransport::Sse),
            headers: BTreeMap::from([("Authorization".to_string(), "Bearer tok".to_string())]),
            ..Default::default()
        };
        emit_for_agent("claude", dir.path(), dir.path(), &[server]).unwrap();
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap())
                .unwrap();
        assert_eq!(root["mcpServers"]["atlassian"]["type"], "sse");
        assert_eq!(root["mcpServers"]["atlassian"]["headers"]["Authorization"], "Bearer tok");
    }

    #[test]
    fn user_scope_servers_are_not_emitted_per_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let server = McpServer {
            name: "atlassian".into(),
            url: Some("https://mcp.atlassian.com/v1/sse".into()),
            transport: Some(McpTransport::Sse),
            user_scope_agents: vec!["claude".into()],
            ..Default::default()
        };
        // Only the user-scope server is present → nothing to write.
        assert!(!emit_for_agent("claude", dir.path(), dir.path(), &[server.clone()]).unwrap());
        assert!(!dir.path().join(".mcp.json").exists());
        // Mixed with a normal server → the user-scope one is filtered out.
        let wrote =
            emit_for_agent("claude", dir.path(), dir.path(), &[server, stdio("kg", "graphify")])
                .unwrap();
        assert!(wrote);
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap())
                .unwrap();
        assert!(root["mcpServers"].get("atlassian").is_none());
        assert_eq!(root["mcpServers"]["kg"]["command"], "graphify");
    }

    /// The AGE-61 bug: authenticating a server with Claude used to set one
    /// global flag, which silently stopped Agency emitting that server for
    /// *every* other agent — so a Copilot user who pressed Authenticate lost
    /// the server entirely. Scope is per-agent now.
    #[test]
    fn user_scope_for_one_agent_still_emits_for_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let server = McpServer {
            name: "atlassian".into(),
            url: Some("https://mcp.atlassian.com/v1/mcp".into()),
            user_scope_agents: vec!["claude".into()],
            ..Default::default()
        };
        // Claude reads it from its own user config → nothing to emit.
        assert!(!emit_for_agent("claude", dir.path(), dir.path(), &[server.clone()]).unwrap());
        assert!(!dir.path().join(".mcp.json").exists());
        // Copilot never registered it → it must still land in the worktree.
        assert!(emit_for_agent("copilot", dir.path(), dir.path(), &[server.clone()]).unwrap());
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap())
                .unwrap();
        assert_eq!(root["mcpServers"]["atlassian"]["url"], "https://mcp.atlassian.com/v1/mcp");
        // …as must Cursor, in its own format.
        assert!(emit_for_agent("cursor", dir.path(), dir.path(), &[server]).unwrap());
        assert!(dir.path().join(".cursor/mcp.json").exists());
    }

    #[test]
    fn legacy_global_user_scope_migrates_to_claude_only() {
        // Servers persisted before per-agent scope existed carry `userScope`.
        let mut s: McpServer =
            serde_json::from_str(r#"{"name":"atlassian","url":"https://x/mcp","userScope":true}"#)
                .unwrap();
        assert!(s.user_scope);
        s.normalize();
        assert!(!s.user_scope, "legacy flag is consumed");
        assert_eq!(s.user_scope_agents, vec!["claude".to_string()]);
        assert!(s.is_user_scope_for("claude"));
        assert!(!s.is_user_scope_for("copilot"));
        // Normalizing twice must not duplicate the entry.
        s.normalize();
        assert_eq!(s.user_scope_agents, vec!["claude".to_string()]);
        // The legacy flag is not written back out.
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("userScope\":true"), "{json}");
    }

    #[test]
    fn auth_argv_is_agent_specific() {
        let mut server = remote("atlassian", "https://mcp.atlassian.com/v1/mcp");
        server.headers.insert("Authorization".into(), "Bearer tok".into());

        let (cmd, args) = auth_argv("claude", &server).unwrap();
        assert_eq!(cmd, "claude");
        assert_eq!(
            args,
            vec![
                "mcp",
                "add",
                "--scope",
                "user",
                "--transport",
                "http",
                "atlassian",
                "https://mcp.atlassian.com/v1/mcp",
                "--header",
                "Authorization: Bearer tok",
            ]
        );

        // Copilot's `mcp add` writes its user config already — no scope flag.
        let (cmd, args) = auth_argv("copilot", &server).unwrap();
        assert_eq!(cmd, "copilot");
        assert_eq!(
            args,
            vec![
                "mcp",
                "add",
                "--transport",
                "http",
                "atlassian",
                "https://mcp.atlassian.com/v1/mcp",
                "--header",
                "Authorization: Bearer tok",
            ]
        );

        // SSE servers keep their transport.
        let sse = McpServer { transport: Some(McpTransport::Sse), ..remote("a", "https://x/sse") };
        let (_, args) = auth_argv("copilot", &sse).unwrap();
        assert!(args.windows(2).any(|w| w == ["--transport", "sse"]), "{args:?}");

        // Agents with no known command, and stdio servers, are rejected.
        assert!(auth_argv("cursor", &server).is_err());
        assert!(auth_argv("claude", &stdio("kg", "graphify")).is_err());
    }

    #[test]
    fn auth_supported_matches_auth_argv() {
        let server = remote("a", "https://x/mcp");
        for agent in AUTH_AGENTS {
            assert!(auth_supported(agent));
            assert!(auth_argv(agent, &server).is_ok(), "{agent}");
        }
        for agent in ["cursor", "opencode", "codex", "gemini"] {
            assert!(!auth_supported(agent));
            assert!(auth_argv(agent, &server).is_err(), "{agent}");
        }
    }

    #[test]
    fn import_json_reads_both_root_keys_and_maps_fields() {
        // Claude/Cursor style: mcpServers.
        let a = import_json(
            r#"{"mcpServers":{"ctx":{"command":"npx","args":["-y","x"],"env":{"K":"V"}}}}"#,
        )
        .unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].name, "ctx");
        assert_eq!(a[0].command.as_deref(), Some("npx"));
        assert_eq!(a[0].args, vec!["-y", "x"]);
        assert_eq!(a[0].env["K"], "V");

        // VS Code style: servers + type + headers + inputs (ignored).
        let b = import_json(
            r#"{"servers":{"jira":{"type":"sse","url":"https://x/sse","headers":{"Authorization":"Bearer t"}}},"inputs":[{"id":"tok"}]}"#,
        )
        .unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].transport, Some(McpTransport::Sse));
        assert_eq!(b[0].url.as_deref(), Some("https://x/sse"));
        assert_eq!(b[0].headers["Authorization"], "Bearer t");

        // Entries with neither command nor url are dropped.
        let c = import_json(r#"{"mcpServers":{"broken":{"type":"http"}}}"#).unwrap();
        assert!(c.is_empty());
    }

    #[test]
    fn emits_cursor_into_nested_dir() {
        let dir = tempfile::tempdir().unwrap();
        emit_for_agent("cursor", dir.path(), dir.path(), &[remote("api", "https://mcp.example")])
            .unwrap();
        let root: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".cursor/mcp.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(root["mcpServers"]["api"]["url"], "https://mcp.example");
    }

    #[test]
    fn emits_opencode_local_command_vector() {
        let dir = tempfile::tempdir().unwrap();
        emit_for_agent("opencode", dir.path(), dir.path(), &[stdio("kg", "graphify")]).unwrap();
        let root: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("opencode.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(root["mcp"]["kg"]["type"], "local");
        assert_eq!(root["mcp"]["kg"]["command"][0], "graphify");
        assert_eq!(root["mcp"]["kg"]["command"][1], "--flag");
        assert_eq!(root["mcp"]["kg"]["enabled"], true);
    }

    #[test]
    fn unsupported_agents_and_invalid_servers_write_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!emit_for_agent("codex", dir.path(), dir.path(), &[stdio("x", "cmd")]).unwrap());
        let invalid = McpServer { name: "bad".into(), ..Default::default() };
        assert!(!emit_for_agent("claude", dir.path(), dir.path(), &[invalid]).unwrap());
        assert!(!dir.path().join(".mcp.json").exists());
    }
}
