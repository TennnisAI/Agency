//! Built-in agent catalog: known CLI profiles with resume/loop recipes.
//!
//! Distinct from enabled profiles in the DB — the catalog is the menu of
//! agents Agency knows how to configure; users enable a subset via onboarding
//! or Settings. `shell` is seeded separately and is not part of this catalog.

use agency_core::profile::AgentProfile;
use serde::Serialize;
use std::sync::OnceLock;

type Recipe = Option<Vec<String>>;

/// One catalog entry: id, binary name, and optional resume/loop recipes.
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub command: &'static str,
    pub resume_args: Recipe,
    pub loop_args: Recipe,
}

/// Catalog entry as sent to the UI (includes whether it's already enabled).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntryInfo {
    pub id: String,
    pub command: String,
    pub resume_args: Option<Vec<String>>,
    pub loop_args: Option<Vec<String>>,
    pub enabled: bool,
    /// Whether `command` resolves on PATH (usable without a profile row).
    pub installed: bool,
    /// Whether Agency emits per-workspace MCP config for this agent.
    pub supports_mcp: bool,
    /// Whether Agency can register an OAuth MCP server with this agent's own
    /// CLI at user scope (the Authenticate flow).
    pub supports_mcp_auth: bool,
}

/// All built-in agent profiles Agency ships recipes for.
pub fn builtins() -> &'static [CatalogEntry] {
    // cursor/hermes/gemini/kimi/crush are id-keyed (not cwd-keyed) so they
    // start fresh rather than risk resuming the wrong global session. Loop
    // recipes are the agents' headless one-shot modes; agents without a known
    // headless mode get None and simply can't loop. claude gets acceptEdits so
    // unattended attempts don't die on the first file-edit permission prompt.
    static LIST: OnceLock<Vec<CatalogEntry>> = OnceLock::new();
    LIST.get_or_init(|| {
        vec![
            CatalogEntry {
                id: "claude",
                command: "claude",
                resume_args: Some(vec!["--continue".into()]),
                loop_args: Some(vec![
                    "-p".into(),
                    "{{prompt}}".into(),
                    "--permission-mode".into(),
                    "acceptEdits".into(),
                ]),
            },
            CatalogEntry {
                id: "codex",
                command: "codex",
                resume_args: Some(vec!["resume".into(), "--last".into()]),
                loop_args: Some(vec![
                    "exec".into(),
                    "--full-auto".into(),
                    "{{prompt}}".into(),
                ]),
            },
            CatalogEntry {
                id: "pi",
                command: "pi",
                resume_args: Some(vec!["--continue".into()]),
                loop_args: None,
            },
            CatalogEntry {
                id: "opencode",
                command: "opencode",
                resume_args: Some(vec!["--continue".into()]),
                loop_args: Some(vec!["run".into(), "{{prompt}}".into()]),
            },
            CatalogEntry {
                id: "copilot",
                command: "copilot",
                resume_args: Some(vec!["--continue".into()]),
                loop_args: None,
            },
            CatalogEntry {
                id: "cursor",
                command: "cursor-agent",
                resume_args: None,
                loop_args: Some(vec!["-p".into(), "{{prompt}}".into()]),
            },
            CatalogEntry {
                id: "hermes",
                command: "hermes",
                resume_args: None,
                loop_args: None,
            },
            CatalogEntry {
                id: "gemini",
                command: "gemini",
                resume_args: None,
                loop_args: None,
            },
            CatalogEntry {
                id: "kimi",
                command: "kimi",
                resume_args: None,
                loop_args: None,
            },
            CatalogEntry {
                id: "crush",
                command: "crush",
                resume_args: None,
                loop_args: None,
            },
        ]
    })
    .as_slice()
}

/// Look up a catalog entry by id.
pub fn find(id: &str) -> Option<&'static CatalogEntry> {
    builtins().iter().find(|e| e.id == id)
}

/// Build an `AgentProfile` from a catalog entry (empty args/env).
pub fn profile_for(entry: &CatalogEntry) -> AgentProfile {
    AgentProfile {
        name: entry.id.to_string(),
        command: entry.command.to_string(),
        args: vec![],
        env: vec![],
        resume_args: entry.resume_args.clone(),
        loop_args: entry.loop_args.clone(),
    }
}
