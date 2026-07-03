use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgencyConfig {
    #[serde(default)]
    pub scripts: ScriptsConfig,
    #[serde(default)]
    pub ports: PortsConfig,
    #[serde(default)]
    pub files: FilesConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub knowledge: KnowledgeConfig,
}

/// Project-level MCP servers (`[mcp.servers.<name>]` tables), merged over the
/// app-global list and emitted per-harness into each worktree (see mcp.rs).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: std::collections::BTreeMap<String, McpServerDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpServerDef {
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    pub url: Option<String>,
}

/// Knowledge-graph integration (graphify). When `graph = true`, the serve
/// command is auto-registered as an MCP server for every agent workspace and
/// the graph is rebuilt in the background after each clean merge.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct KnowledgeConfig {
    #[serde(default)]
    pub graph: bool,
    /// Override for the MCP serve command. Default: "graphify serve".
    pub serve_command: Option<String>,
    /// Override for the rebuild command run after merges. Default: "graphify .".
    pub build_command: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FilesConfig {
    /// Repo-root-relative paths copied into every new (or restored) worktree.
    /// Worktrees only materialize tracked files, so untracked essentials like
    /// `.env` must be listed here to be present in agent workspaces.
    #[serde(default)]
    pub copy: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScriptsConfig {
    pub setup: Option<String>,
    pub run: Option<String>,
    pub archive: Option<String>,
    #[serde(default)]
    pub run_mode: RunMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    Concurrent,
    Nonconcurrent,
}

impl Default for RunMode {
    fn default() -> Self {
        RunMode::Concurrent
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PortsConfig {
    #[serde(default = "default_port_base")]
    pub base: u16,
    #[serde(default = "default_block_size")]
    pub block_size: u16,
}

impl Default for PortsConfig {
    fn default() -> Self {
        PortsConfig { base: default_port_base(), block_size: default_block_size() }
    }
}

fn default_port_base() -> u16 {
    5200
}
fn default_block_size() -> u16 {
    10
}

/// Load `.agency/agency.toml` merged under `.agency/agency.local.toml`
/// (local overrides repo). Missing files / malformed TOML resolve to defaults.
pub fn load(repo_path: &Path) -> AgencyConfig {
    let base = read_value(&repo_path.join(".agency").join("agency.toml"));
    let local = read_value(&repo_path.join(".agency").join("agency.local.toml"));
    let merged = match (base, local) {
        (Some(b), Some(l)) => merge_values(b, l),
        (Some(b), None) => b,
        (None, Some(l)) => l,
        (None, None) => return AgencyConfig::default(),
    };
    merged.try_into().unwrap_or_default()
}

fn read_value(path: &Path) -> Option<toml::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str::<toml::Value>(&text).ok()
}

/// Deep-merge `local` over `base`: tables merge recursively, every other value
/// is replaced by `local`.
fn merge_values(mut base: toml::Value, local: toml::Value) -> toml::Value {
    if let (Some(bt), Some(lt)) = (base.as_table_mut(), local.as_table()) {
        for (k, lv) in lt {
            let merged = match bt.get(k) {
                Some(bv) if bv.is_table() && lv.is_table() => {
                    merge_values(bv.clone(), lv.clone())
                }
                _ => lv.clone(),
            };
            bt.insert(k.clone(), merged);
        }
        base
    } else {
        local
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(dir: &Path, name: &str, body: &str) {
        let agency = dir.join(".agency");
        fs::create_dir_all(&agency).unwrap();
        fs::write(agency.join(name), body).unwrap();
    }

    #[test]
    fn defaults_when_no_file() {
        let dir = tempdir().unwrap();
        let c = load(dir.path());
        assert_eq!(c.scripts.setup, None);
        assert_eq!(c.scripts.run_mode, RunMode::Concurrent);
        assert_eq!(c.ports.base, 5200);
        assert_eq!(c.ports.block_size, 10);
    }

    #[test]
    fn parses_full_config() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "agency.toml",
            r#"
                [scripts]
                setup = "pnpm install"
                run = "pnpm dev --port $AGENCY_PORT"
                archive = "./cleanup.sh"
                run_mode = "nonconcurrent"

                [ports]
                base = 4000
                block_size = 5

                [files]
                copy = [".env", "config/certs"]
            "#,
        );
        let c = load(dir.path());
        assert_eq!(c.scripts.setup.as_deref(), Some("pnpm install"));
        assert_eq!(c.scripts.run.as_deref(), Some("pnpm dev --port $AGENCY_PORT"));
        assert_eq!(c.scripts.archive.as_deref(), Some("./cleanup.sh"));
        assert_eq!(c.scripts.run_mode, RunMode::Nonconcurrent);
        assert_eq!(c.ports.base, 4000);
        assert_eq!(c.ports.block_size, 5);
        assert_eq!(c.files.copy, vec![".env".to_string(), "config/certs".to_string()]);
    }

    #[test]
    fn files_copy_defaults_to_empty() {
        let dir = tempdir().unwrap();
        write(dir.path(), "agency.toml", "[ports]\nbase = 4000\n");
        let c = load(dir.path());
        assert!(c.files.copy.is_empty());
        assert!(c.mcp.servers.is_empty());
        assert!(!c.knowledge.graph);
    }

    #[test]
    fn parses_mcp_servers_and_knowledge() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "agency.toml",
            r#"
                [mcp.servers.context7]
                command = "npx"
                args = ["-y", "@upstash/context7-mcp"]
                env = { API_KEY = "abc" }

                [mcp.servers.linear]
                url = "https://mcp.linear.app/sse"

                [knowledge]
                graph = true
                build_command = "graphify . --skip-html"
            "#,
        );
        let c = load(dir.path());
        let ctx = &c.mcp.servers["context7"];
        assert_eq!(ctx.command.as_deref(), Some("npx"));
        assert_eq!(ctx.args, vec!["-y", "@upstash/context7-mcp"]);
        assert_eq!(ctx.env["API_KEY"], "abc");
        assert_eq!(c.mcp.servers["linear"].url.as_deref(), Some("https://mcp.linear.app/sse"));
        assert!(c.knowledge.graph);
        assert_eq!(c.knowledge.build_command.as_deref(), Some("graphify . --skip-html"));
        assert_eq!(c.knowledge.serve_command, None);
    }

    #[test]
    fn local_overrides_base_per_field() {
        let dir = tempdir().unwrap();
        write(dir.path(), "agency.toml", "[scripts]\nsetup = \"base-setup\"\nrun = \"base-run\"\n");
        write(dir.path(), "agency.local.toml", "[scripts]\nsetup = \"local-setup\"\n");
        let c = load(dir.path());
        // local wins for setup, base survives for run
        assert_eq!(c.scripts.setup.as_deref(), Some("local-setup"));
        assert_eq!(c.scripts.run.as_deref(), Some("base-run"));
    }

    #[test]
    fn malformed_toml_falls_back_to_defaults() {
        let dir = tempdir().unwrap();
        write(dir.path(), "agency.toml", "this is not valid = = toml [[[");
        let c = load(dir.path());
        assert_eq!(c.ports.base, 5200);
        assert_eq!(c.scripts.setup, None);
    }
}
