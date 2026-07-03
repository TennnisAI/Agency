//! Cross-harness MCP configuration.
//!
//! Agency keeps one canonical list of MCP servers (global app settings merged
//! with the project's `[mcp.servers.*]` in `.agency/agency.toml`) and emits it
//! into each new worktree in the *native* format of the agent that will run
//! there — `.mcp.json` for Claude Code, `.cursor/mcp.json` for Cursor,
//! `opencode.json` for OpenCode. Existing files are merged into (our entries
//! upserted by name), never replaced, so repo-committed server definitions
//! survive. Agents whose MCP config is global-only (Codex's ~/.codex/config.toml,
//! Copilot's ~/.copilot) are intentionally skipped: Agency never mutates
//! files outside the workspace.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// One MCP server definition. Exactly one of `command` (stdio transport) or
/// `url` (remote transport) should be set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
}

impl McpServer {
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
}

/// Flatten the project config's `[mcp.servers.<name>]` tables into servers.
pub fn from_config(cfg: &crate::config::McpConfig) -> Vec<McpServer> {
    cfg.servers
        .iter()
        .map(|(name, def)| McpServer {
            name: name.clone(),
            command: def.command.clone(),
            args: def.args.clone(),
            env: def.env.clone(),
            url: def.url.clone(),
        })
        .collect()
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

/// Write the servers into `worktree` in `agent`'s native config format.
/// Returns false (and writes nothing) for agents Agency can't configure
/// per-workspace. Invalid entries are skipped rather than failing the run.
pub fn emit_for_agent(agent: &str, worktree: &Path, servers: &[McpServer]) -> Result<bool> {
    let valid: Vec<&McpServer> = servers.iter().filter(|s| s.validate().is_ok()).collect();
    if valid.is_empty() {
        return Ok(false);
    }
    match agent {
        "claude" => upsert_json(&worktree.join(".mcp.json"), "mcpServers", &valid, claude_entry),
        "cursor" => upsert_json(&worktree.join(".cursor").join("mcp.json"), "mcpServers", &valid, cursor_entry),
        "opencode" => upsert_json(&worktree.join("opencode.json"), "mcp", &valid, opencode_entry),
        _ => Ok(false),
    }
}

fn claude_entry(s: &McpServer) -> serde_json::Value {
    match &s.url {
        Some(url) => serde_json::json!({ "type": "http", "url": url }),
        None => serde_json::json!({
            "command": s.command.clone().unwrap_or_default(),
            "args": s.args,
            "env": s.env,
        }),
    }
}

fn cursor_entry(s: &McpServer) -> serde_json::Value {
    match &s.url {
        Some(url) => serde_json::json!({ "url": url }),
        None => serde_json::json!({
            "command": s.command.clone().unwrap_or_default(),
            "args": s.args,
            "env": s.env,
        }),
    }
}

fn opencode_entry(s: &McpServer) -> serde_json::Value {
    match &s.url {
        Some(url) => serde_json::json!({ "type": "remote", "url": url, "enabled": true }),
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
    let bucket = obj
        .entry(root_key.to_string())
        .or_insert_with(|| serde_json::json!({}));
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
            url: None,
        }
    }

    fn remote(name: &str, url: &str) -> McpServer {
        McpServer { name: name.into(), command: None, args: vec![], env: BTreeMap::new(), url: Some(url.into()) }
    }

    #[test]
    fn validate_requires_exactly_one_transport() {
        assert!(stdio("a", "cmd").validate().is_ok());
        assert!(remote("b", "https://x").validate().is_ok());
        let neither = McpServer { name: "n".into(), command: None, args: vec![], env: BTreeMap::new(), url: None };
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

        let wrote = emit_for_agent("claude", dir.path(), &[stdio("mine", "my-cmd"), remote("api", "https://mcp.example")]).unwrap();
        assert!(wrote);

        let root: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["other"], 1, "unrelated keys survive");
        assert_eq!(root["mcpServers"]["repo-own"]["command"], "keep-me", "existing servers survive");
        assert_eq!(root["mcpServers"]["mine"]["command"], "my-cmd");
        assert_eq!(root["mcpServers"]["mine"]["env"]["KEY"], "V");
        assert_eq!(root["mcpServers"]["api"]["type"], "http");
        assert_eq!(root["mcpServers"]["api"]["url"], "https://mcp.example");
    }

    #[test]
    fn emits_cursor_into_nested_dir() {
        let dir = tempfile::tempdir().unwrap();
        emit_for_agent("cursor", dir.path(), &[remote("api", "https://mcp.example")]).unwrap();
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(".cursor/mcp.json")).unwrap()).unwrap();
        assert_eq!(root["mcpServers"]["api"]["url"], "https://mcp.example");
    }

    #[test]
    fn emits_opencode_local_command_vector() {
        let dir = tempfile::tempdir().unwrap();
        emit_for_agent("opencode", dir.path(), &[stdio("kg", "graphify")]).unwrap();
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("opencode.json")).unwrap()).unwrap();
        assert_eq!(root["mcp"]["kg"]["type"], "local");
        assert_eq!(root["mcp"]["kg"]["command"][0], "graphify");
        assert_eq!(root["mcp"]["kg"]["command"][1], "--flag");
        assert_eq!(root["mcp"]["kg"]["enabled"], true);
    }

    #[test]
    fn unsupported_agents_and_invalid_servers_write_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!emit_for_agent("codex", dir.path(), &[stdio("x", "cmd")]).unwrap());
        let invalid = McpServer { name: "bad".into(), command: None, args: vec![], env: BTreeMap::new(), url: None };
        assert!(!emit_for_agent("claude", dir.path(), &[invalid]).unwrap());
        assert!(!dir.path().join(".mcp.json").exists());
    }
}
