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
    #[serde(default)]
    pub transport: Option<crate::mcp::McpTransport>,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
    /// Registered at the agent CLI's user scope; not re-emitted per worktree.
    #[serde(default)]
    pub user_scope: bool,
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

/// The default graphify MCP serve command for a repo. graphify's server has no
/// console-script entry point — it runs as `python -m graphify.serve <graph.json>`
/// inside the uv tool venv, and the graph lives in the primary repo's untracked
/// `graphify-out/`. Shared by the MCP injector and the settings UI so the UI's
/// placeholder matches what actually runs.
pub fn default_serve_command(repo_path: &Path) -> String {
    format!(
        "uv tool run --from graphifyy python -m graphify.serve {}",
        repo_path.join("graphify-out").join("graph.json").display()
    )
}

/// The default graphify MCP serve command as an argv vector. The graph path is
/// a single element, so a repo path containing spaces survives intact — unlike
/// splitting the string form of [`default_serve_command`] on whitespace.
pub fn default_serve_argv(repo_path: &Path) -> Vec<String> {
    vec![
        "uv".to_string(),
        "tool".to_string(),
        "run".to_string(),
        "--from".to_string(),
        "graphifyy".to_string(),
        "python".to_string(),
        "-m".to_string(),
        "graphify.serve".to_string(),
        repo_path.join("graphify-out").join("graph.json").display().to_string(),
    ]
}

/// Split a shell-style command string into an argv, honoring single/double
/// quotes so a user-configured serve command can carry a path containing
/// spaces (`... "/my repo/graph.json"`). No escape processing — quotes are
/// enough for the paths this guards.
pub fn split_command(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in s.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    cur.push(c);
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    started = true;
                }
                c if c.is_whitespace() => {
                    if started {
                        args.push(std::mem::take(&mut cur));
                        started = false;
                    }
                }
                c => {
                    cur.push(c);
                    started = true;
                }
            },
        }
    }
    if started {
        args.push(cur);
    }
    args
}

/// The default rebuild command run after a clean merge.
pub fn default_build_command() -> &'static str {
    "graphify ."
}

/// Persist the `[knowledge]` section into `.agency/agency.local.toml` — the
/// gitignored, per-machine override file (`load` merges it over the tracked
/// `agency.toml`). Any other config already in that file is preserved. `graph`
/// is always written so toggling off is durable; empty command overrides are
/// omitted so the runtime defaults apply.
pub fn save_knowledge(repo_path: &Path, k: &KnowledgeConfig) -> std::io::Result<()> {
    let dir = repo_path.join(".agency");
    let path = dir.join("agency.local.toml");
    let mut doc = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| toml::from_str::<toml::Value>(&t).ok())
        .and_then(|v| v.as_table().cloned())
        .unwrap_or_default();

    let mut table = toml::value::Table::new();
    table.insert("graph".into(), toml::Value::Boolean(k.graph));
    for (key, val) in [("serve_command", &k.serve_command), ("build_command", &k.build_command)] {
        if let Some(s) = val.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            table.insert(key.into(), toml::Value::String(s.to_string()));
        }
    }
    doc.insert("knowledge".into(), toml::Value::Table(table));

    let text = toml::to_string_pretty(&toml::Value::Table(doc))
        .map_err(std::io::Error::other)?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&path, text)
}

/// Persist the `[files]` section (the `copy` list) into `.agency/agency.local.toml`
/// — the gitignored, per-machine override file. Paths are trimmed and blanks
/// dropped; any other config already in that file is preserved. Mirrors
/// [`save_knowledge`]. The list lives in the local file because worktree-copy
/// targets (`.env`, local certs) are inherently machine-specific.
pub fn save_files(repo_path: &Path, f: &FilesConfig) -> std::io::Result<()> {
    let dir = repo_path.join(".agency");
    let path = dir.join("agency.local.toml");
    let mut doc = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| toml::from_str::<toml::Value>(&t).ok())
        .and_then(|v| v.as_table().cloned())
        .unwrap_or_default();

    let copy: Vec<toml::Value> = f
        .copy
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| toml::Value::String(s.to_string()))
        .collect();
    // `copy` is the only `[files]` key today, so replacing the whole table is
    // safe. If more keys are added here, merge into the existing table instead of
    // overwriting so sibling keys aren't dropped.
    let mut table = toml::value::Table::new();
    table.insert("copy".into(), toml::Value::Array(copy));
    doc.insert("files".into(), toml::Value::Table(table));

    let text = toml::to_string_pretty(&toml::Value::Table(doc))
        .map_err(std::io::Error::other)?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&path, text)
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
    fn default_serve_argv_keeps_spaced_path_as_one_arg() {
        let argv = default_serve_argv(Path::new("/Users/me/My Repos/proj"));
        assert_eq!(argv.last().unwrap(), "/Users/me/My Repos/proj/graphify-out/graph.json");
        // The command itself is the first element; the path is not split.
        assert_eq!(argv[0], "uv");
        assert_eq!(argv.len(), 9);
    }

    #[test]
    fn split_command_honors_quotes() {
        assert_eq!(split_command("uv run python"), vec!["uv", "run", "python"]);
        assert_eq!(
            split_command("python -m graphify.serve \"/my repo/graph.json\""),
            vec!["python", "-m", "graphify.serve", "/my repo/graph.json"]
        );
        assert_eq!(
            split_command("a 'b c' d"),
            vec!["a", "b c", "d"]
        );
        // Extra whitespace collapses; no empty trailing arg.
        assert_eq!(split_command("  a   b  "), vec!["a", "b"]);
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
    fn save_knowledge_writes_local_and_preserves_other_config() {
        let dir = tempdir().unwrap();
        // Pre-existing local config in an unrelated section must survive.
        write(dir.path(), "agency.local.toml", "[ports]\nbase = 4100\n");
        save_knowledge(
            dir.path(),
            &KnowledgeConfig {
                graph: true,
                serve_command: None,
                build_command: Some("  graphify . --skip-html  ".to_string()),
            },
        )
        .unwrap();
        let c = load(dir.path());
        assert!(c.knowledge.graph);
        // Trimmed, and the empty serve override is omitted (default applies).
        assert_eq!(c.knowledge.build_command.as_deref(), Some("graphify . --skip-html"));
        assert_eq!(c.knowledge.serve_command, None);
        assert_eq!(c.ports.base, 4100);

        // Toggling off is durable (graph = false is written, not dropped).
        save_knowledge(
            dir.path(),
            &KnowledgeConfig { graph: false, serve_command: None, build_command: None },
        )
        .unwrap();
        assert!(!load(dir.path()).knowledge.graph);
    }

    #[test]
    fn save_files_writes_local_trims_and_preserves_other_config() {
        let dir = tempdir().unwrap();
        write(dir.path(), "agency.local.toml", "[ports]\nbase = 4100\n");
        save_files(
            dir.path(),
            &FilesConfig {
                copy: vec![" .env ".to_string(), "".to_string(), "config/certs".to_string()],
            },
        )
        .unwrap();
        let c = load(dir.path());
        // Trimmed, blanks dropped, order preserved.
        assert_eq!(c.files.copy, vec![".env".to_string(), "config/certs".to_string()]);
        // Unrelated local section survives.
        assert_eq!(c.ports.base, 4100);

        // Clearing the list is durable (empty array written, not dropped).
        save_files(dir.path(), &FilesConfig { copy: vec![] }).unwrap();
        assert!(load(dir.path()).files.copy.is_empty());
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
