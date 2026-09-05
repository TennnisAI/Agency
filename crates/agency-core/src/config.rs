use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    #[serde(default)]
    pub preview: PreviewConfig,
    #[serde(default)]
    pub issues: IssuesConfig,
}

/// Where this project's backlog lives. Off by default: a tracker that starts
/// pushing itself somewhere the user did not ask for is not a default anyone
/// wants.
///
/// The two fields deliberately belong in different files, and `load` already
/// merges them that way (`agency.toml` under `agency.local.toml`):
///
/// - `sync` is a fact about the repo and its team, so it belongs in the tracked
///   `agency.toml` and travels with a clone. A teammate then needs no setup.
/// - `remote` is a fact about this machine, so it belongs in the gitignored
///   `agency.local.toml`. A private tracker remote is not something to ship to
///   people who cannot push to it, and this repo is the case in point: the code
///   is public and the backlog is not.
///
/// The three states a user picks between are "this machine only" (`sync =
/// false`), "sync with the repo" (`sync = true`, the default `origin`), and
/// "sync to another remote" (`sync = true` with `remote` set). See
/// `docs/tracked-issues.md`.
#[derive(Debug, Clone, Deserialize)]
pub struct IssuesConfig {
    #[serde(default)]
    pub sync: bool,
    /// Run a pass without being asked, rather than only from the Sync button.
    ///
    /// Per machine and off by default, and deliberately not part of the
    /// `sync`/`remote` pair: whether the backlog is shared is a fact about the
    /// project that a team may commit into `agency.toml`, while how often this
    /// laptop talks to the network is nobody else's decision. It stays off
    /// until asked for because a merge decides a field both sides changed by
    /// `updated`, and the first version of that judgement to run unattended
    /// should be one someone has already watched run by hand.
    #[serde(default)]
    pub auto: bool,
    /// A remote name, or a URL for a tracker that lives somewhere the code
    /// does not.
    #[serde(default = "default_remote")]
    pub remote: String,
}

impl Default for IssuesConfig {
    fn default() -> Self {
        IssuesConfig { sync: false, auto: false, remote: default_remote() }
    }
}

fn default_remote() -> String {
    "origin".to_string()
}

/// Did the *tracked* `agency.toml` ask for issue sync, rather than this
/// machine's local file? Read on its own, and not through [`load`], because the
/// two say different things to a user: a repo that turns sync on is a decision
/// their team made and shares, and switching it off locally is an override of
/// that rather than a change anyone else sees.
pub fn issue_sync_declared_by_repo(repo_path: &Path) -> bool {
    read_value(&repo_path.join(".agency").join("agency.toml"))
        .and_then(|v| Some(v.get("issues")?.get("sync")?.as_bool()?))
        .unwrap_or(false)
}

/// The remote this project's issues sync to, or `None` when the backlog stays
/// on this machine. One place to ask, so no caller has to remember that an
/// empty `remote` means the same thing as `sync = false`.
pub fn issue_sync_remote(repo_path: &Path) -> Option<String> {
    let cfg = load(repo_path).issues;
    let remote = cfg.remote.trim().to_string();
    (cfg.sync && !remote.is_empty()).then_some(remote)
}

/// The Run tab preview's agent-facing side (AGE-143). On by default because it
/// only ever exists where the user has already configured a web run script,
/// and every run that carries the tools says so in its Run tab — the gate is
/// the script the user set up, not a hidden flag. `agent_tools = false` in
/// `[preview]` turns the server off project-wide for the users who want that.
#[derive(Debug, Clone, Deserialize)]
pub struct PreviewConfig {
    #[serde(default = "default_true")]
    pub agent_tools: bool,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        PreviewConfig { agent_tools: true }
    }
}

fn default_true() -> bool {
    true
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
    /// Agent CLIs this server is registered with at their own user scope; not
    /// re-emitted into those agents' per-worktree config (see mcp.rs).
    #[serde(default)]
    pub user_scope_agents: Vec<String>,
    /// Legacy global form of the field above, folded into it on load.
    #[serde(default)]
    pub user_scope: bool,
}

/// Knowledge-graph integration (graphify). When `graph = true`, the serve
/// command is auto-registered as an MCP server for every agent workspace and
/// the graph is rebuilt in the background after each clean merge.
#[derive(Debug, Clone, Deserialize)]
pub struct KnowledgeConfig {
    #[serde(default)]
    pub graph: bool,
    /// Override for the MCP serve command. Default: [`default_serve_command`].
    pub serve_command: Option<String>,
    /// Override for the rebuild command run after merges, and the place a user
    /// picks a different LLM backend. Default: [`default_build_command`].
    pub build_command: Option<String>,
    /// Rebuild the graph in the background after every clean merge.
    ///
    /// On, because a graph that stops matching the code is worse than no graph.
    /// A switch and not a constant, because it is the one thing here that
    /// spends a user's model budget without them pressing anything: a build
    /// reads every file in the project, and merges land all day.
    #[serde(default = "default_true")]
    pub rebuild_on_merge: bool,
}

impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self { graph: false, serve_command: None, build_command: None, rebuild_on_merge: true }
    }
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
    /// The single run command, as projects configured before named scripts
    /// spell it. Superseded by `runs` whenever that list has entries; still
    /// read (and still the shorthand a hand-written `agency.toml` can use).
    pub run: Option<String>,
    pub archive: Option<String>,
    #[serde(default)]
    pub run_mode: RunMode,
    /// Named run scripts, `[[scripts.runs]]`. A project usually has more than
    /// one thing worth starting — a dev server, a release build, a test watch
    /// — so the Run tab lists them rather than holding one command.
    #[serde(default)]
    pub runs: Vec<RunScript>,
}

/// One entry in a project's run list: a named command the Run tab can start in
/// an agent's workspace or in the project's own checkout.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RunScript {
    pub name: String,
    pub command: String,
    /// The command serves something over HTTP on `$AGENCY_PORT`. Only these
    /// get a URL, the preview pane and "Open in browser": a release build or a
    /// test run has nothing to point a browser at, and opening one for it was
    /// the whole complaint behind AGE-34.
    #[serde(default)]
    pub web: bool,
    /// Starting this one stops every other run script in the project. For
    /// commands that bind a fixed port instead of `$AGENCY_PORT`.
    #[serde(default)]
    pub nonconcurrent: bool,
}

/// The name given to a legacy single `run =` command when it is lifted into
/// the list.
pub const DEFAULT_RUN_NAME: &str = "Run";

/// Whether a command looks like it serves a web app — it threads the port
/// Agency hands it. Only ever a prefill for the "opens in a browser" toggle;
/// what gets saved is the answer the user actually left on screen.
pub fn command_looks_web(command: &str) -> bool {
    command.contains("AGENCY_PORT")
}

impl ScriptsConfig {
    /// The project's run scripts. The `[[scripts.runs]]` list when it has
    /// entries, else the legacy single `run =` command lifted into a one-entry
    /// list so a project configured before named scripts keeps working with no
    /// edit at all.
    pub fn run_list(&self) -> Vec<RunScript> {
        if !self.runs.is_empty() {
            return dedupe_by_name(self.runs.clone());
        }
        self.run
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|command| {
                vec![RunScript {
                    name: DEFAULT_RUN_NAME.to_string(),
                    command: command.to_string(),
                    web: command_looks_web(command),
                    nonconcurrent: self.run_mode == RunMode::Nonconcurrent,
                }]
            })
            .unwrap_or_default()
    }
}

/// Drop blank entries and later duplicates of a name. The name keys the script's
/// daemon session, so two entries sharing one would fight over the same running
/// process; first-wins keeps that impossible however the TOML was written.
fn dedupe_by_name(scripts: Vec<RunScript>) -> Vec<RunScript> {
    let mut seen = std::collections::HashSet::new();
    scripts
        .into_iter()
        .map(|mut s| {
            s.name = s.name.trim().to_string();
            s.command = s.command.trim().to_string();
            s
        })
        .filter(|s| !s.name.is_empty() && !s.command.is_empty())
        .filter(|s| seen.insert(s.name.clone()))
        .collect()
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

impl RunMode {
    /// The TOML spelling, as `load` expects to read it back.
    pub fn as_str(self) -> &'static str {
        match self {
            RunMode::Concurrent => "concurrent",
            RunMode::Nonconcurrent => "nonconcurrent",
        }
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
    let base = read_value(&repo_path.join(".agency").join("agency.toml")).map(strip_machine_only);
    let local = read_value(&repo_path.join(".agency").join("agency.local.toml"));
    let merged = match (base, local) {
        (Some(b), Some(l)) => merge_values(b, l),
        (Some(b), None) => b,
        (None, Some(l)) => l,
        (None, None) => return AgencyConfig::default(),
    };
    merged.try_into().unwrap_or_default()
}

/// Drop the fields a repo is not allowed to decide for the machine that clones
/// it, before the tracked file is merged under the local one.
///
/// `[issues] auto` is the only one so far. Whether a backlog is shared is a
/// fact about the project and a team may commit `sync = true`; how often this
/// laptop talks to the network is not, and a tracked `auto = true` would have
/// every clone start fetching and pushing on a timer with nobody here having
/// asked. The merge alone cannot stop it: a fresh clone has no `[issues]` in
/// `agency.local.toml` at all, so there is no local `false` for the merge to
/// prefer. Stripped from the tracked value rather than special-cased in
/// `merge_values`, which has no business knowing which keys these are.
fn strip_machine_only(mut v: toml::Value) -> toml::Value {
    if let Some(issues) = v.get_mut("issues").and_then(|i| i.as_table_mut()) {
        issues.remove("auto");
    }
    v
}

fn read_value(path: &Path) -> Option<toml::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str::<toml::Value>(&text).ok()
}

/// The directory a graph build writes into, at the repo root: `graph.json`
/// plus a `cache/` the next build reuses.
pub const GRAPH_OUT_DIR: &str = "graphify-out";

/// Where the build command leaves the graph: the primary repo's untracked
/// `graphify-out/graph.json`. The serve command points at this file, and its
/// presence is what tells us a graph has actually been built (worktrees never
/// carry `graphify-out/`, so the path is always the primary repo's).
pub fn graph_path(repo_path: &Path) -> PathBuf {
    repo_path.join(GRAPH_OUT_DIR).join("graph.json")
}

/// Keep a graph build's output out of git, in the repo's `.git/info/exclude`.
///
/// AGE-170: a build in the primary checkout put
/// `graphify-out/cache/stat-index.json` in the user's untracked changes, where
/// it sits in front of every commit they make and rides along in any `git add
/// -A`. The graph is a machine-local artifact of a build the user re-runs, not
/// project content, and Agency is what put it there.
///
/// Best-effort: failing to write the exclude file is no reason to refuse a
/// build. Root-anchored, because only a build run from the repo root (as
/// `graphify .` is) lands here, and `git add -f` still wins for a user who
/// wants the graph tracked after all.
pub fn exclude_graph_output(repo_path: &Path) {
    let pattern = format!("/{GRAPH_OUT_DIR}/");
    if let Err(e) = crate::worktree::ensure_exclude_pattern(repo_path, &pattern) {
        log::warn!("excluding {pattern} in {}: {e}", repo_path.display());
    }
}

/// The default graphify MCP serve command for a repo. graphify's server has no
/// console-script entry point — it runs as `python -m graphify.serve <graph.json>`
/// inside the uv tool venv, and the graph lives in the primary repo's untracked
/// `graphify-out/`. Shared by the MCP injector and the settings UI so the UI's
/// placeholder matches what actually runs.
pub fn default_serve_command(repo_path: &Path) -> String {
    format!(
        "uv tool run --from graphifyy python -m graphify.serve {}",
        graph_path(repo_path).display()
    )
}

/// The shell script that installs the graphify tooling, shown (and offered) by
/// the settings UI when the serve/build commands aren't on PATH. `graphifyy` is
/// the PyPI distribution; it ships both the `graphify` console script the build
/// command runs and the `graphify` Python package `graphify.serve` lives in, so
/// this one install covers serve and build together.
///
/// It installs *through* uv, so when uv itself is missing the script installs
/// that first, from Astral's own installer. uv lands in `~/.local/bin`, which a
/// shell that started before the install doesn't have on its PATH yet, hence the
/// export: without it the very next line fails on a uv that is right there.
///
/// The extras are not optional here, and a bare `uv tool install graphifyy`
/// installs none of them. `mcp` is what the serve command imports: without it
/// `python -m graphify.serve` exits on `ImportError: mcp not installed`, so
/// every agent gets a knowledge-graph server that dies on startup. `openai`
/// (the SDK the openai, gemini, kimi, deepseek and ollama backends all share)
/// and `anthropic` are what the *build* imports once the corpus has a doc or an
/// image in it: on graphifyy 0.9.51 a build of an 85-file corpus failed every
/// chunk with "the 'openai' package is required for this backend but is not
/// installed" and wrote no graph.json at all. `--force` because the install
/// this repairs usually already exists: uv treats an installed `graphifyy` as
/// satisfying the request and would otherwise leave the broken one in place.
pub fn graphify_install_script(uv_installed: bool) -> String {
    let install_graphify = "uv tool install --force \"graphifyy[mcp,openai,anthropic]\"";
    if uv_installed {
        return install_graphify.to_string();
    }
    format!(
        "curl -LsSf https://astral.sh/uv/install.sh | sh\n\
         export PATH=\"$HOME/.local/bin:$PATH\"\n\
         {install_graphify}"
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
        graph_path(repo_path).display().to_string(),
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

/// The choice that runs no LLM at all: tree-sitter over the code, docs skipped.
pub const CODE_ONLY: &str = "code-only";

/// The `claude` CLI, used as the build's model. Not an API key: it bills the
/// user's existing Claude plan.
pub const CLAUDE_CLI: &str = "claude-cli";

/// The model a claude-CLI build runs on when the user names none.
///
/// Not the CLI's own default, which is what a build got before this existed:
/// graphify passes `claude -p` once per chunk, the CLI answered on the plan's
/// default (a large model), and one first build of one project spent an entire
/// 5-hour usage window in about 30 minutes. It is named in the command so it
/// can never silently follow whatever the CLI defaults to next.
///
/// Small on purpose, and graphify agrees in its own source (`llm.py`,
/// `_call_claude_cli`): "claude-cli defaults to Opus, which is overkill for the
/// structured-JSON extraction graphify performs". The model never sees code at
/// all. Code files go through tree-sitter locally; the LLM pass runs over docs,
/// papers and images only, at temperature 0, against a fixed node/edge schema
/// that newer Claude Code releases pin structurally with `--json-schema`. That
/// is a format-following job, not a reasoning one.
pub const CLAUDE_CLI_DEFAULT_MODEL: &str = "haiku";

/// graphify's `openai` backend pointed at the local, OpenAI-protocol server
/// from Agency's own settings (LM Studio and friends). Its own id here because
/// nothing about it is OpenAI: the corpus never leaves the machine.
pub const LOCAL_MODEL: &str = "lmstudio";

/// A saved build command this panel didn't write, and can't take apart.
pub const CUSTOM_BACKEND: &str = "custom";

/// The cloud backends graphify can run, each with the environment variable that
/// switches it on, where the corpus ends up, and the model it uses unasked.
/// Ordered as graphify's own auto-detection orders them, so the first one
/// offered is the one a bare `graphify .` would have picked anyway.
const CLOUD_BACKENDS: &[(&str, &str, &str, &str, &str)] = &[
    // id, label, env var, vendor, default model
    ("gemini", "Gemini", "GEMINI_API_KEY", "Google", "gemini-3-flash-preview"),
    ("kimi", "Kimi", "MOONSHOT_API_KEY", "Moonshot", "kimi-k2.6"),
    ("claude", "Claude API", "ANTHROPIC_API_KEY", "Anthropic", "claude-sonnet-4-6"),
    ("openai", "OpenAI", "OPENAI_API_KEY", "OpenAI", "gpt-4.1-mini"),
    ("deepseek", "DeepSeek", "DEEPSEEK_API_KEY", "DeepSeek", "deepseek-v4-flash"),
];

/// What Agency can see of a machine when it works out which models the graph
/// build could run on. Data, so the offer itself stays pure: the app fills this
/// from PATH, its own settings and the environment it was started in.
#[derive(Debug, Clone, Default)]
pub struct BackendProbe {
    pub claude_on_path: bool,
    /// Ollama is installed or pointed at (on PATH, or `OLLAMA_BASE_URL` /
    /// `OLLAMA_HOST` set).
    pub ollama: bool,
    /// Agency's own "Local model" base URL, when one is configured.
    pub local_model_url: Option<String>,
    /// Ids from [`CLOUD_BACKENDS`] whose API key is set in the app's
    /// environment.
    pub env_keys: Vec<String>,
}

/// One model the graph build can run on, as offered by settings.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KnowledgeBackend {
    pub id: String,
    pub label: String,
    /// What it costs and who ends up with the corpus, in one line. A build
    /// reads every doc in the project, so this is what a user has to be told
    /// *before* one runs rather than after it has already been paid for.
    pub note: String,
    /// The model this backend uses when none is named: the model field's
    /// placeholder. Empty where the user has to name one themselves.
    pub default_model: String,
}

/// Which cloud backends have an API key to run on, given a lookup into the
/// environment the app was started in.
///
/// `openai` is skipped whenever `OPENAI_BASE_URL` is set alongside the key.
/// That pair is a local, OpenAI-protocol server, not a paid OpenAI account:
/// Agency writes exactly it into every agent session when the user has
/// configured a local model, so an app started from one inherits it. Offering
/// it as "OpenAI"
/// would name the wrong vendor, and graphify's own auto-detection making that
/// mistake is what sent an entire corpus at an LM Studio that wasn't running.
pub fn env_backend_keys(env: impl Fn(&str) -> Option<String>) -> Vec<String> {
    CLOUD_BACKENDS
        .iter()
        .filter(|(id, _, var, _, _)| {
            env(var).is_some() && !(*id == "openai" && env("OPENAI_BASE_URL").is_some())
        })
        .map(|(id, ..)| id.to_string())
        .collect()
}

/// The models this machine can actually build a graph with, best first.
///
/// Ordered by how much of a user's setup they take for granted, not by
/// quality: the CLI they already run, then a key they have already provisioned,
/// then a local server, then no LLM at all. Every machine gets at least
/// [`CODE_ONLY`], so a user with no agent, no key and no local model still has
/// a graph they can build (code, via tree-sitter, and nothing sent anywhere).
pub fn knowledge_backends(probe: &BackendProbe) -> Vec<KnowledgeBackend> {
    let mut out = Vec::new();
    if probe.claude_on_path {
        out.push(KnowledgeBackend {
            id: CLAUDE_CLI.to_string(),
            label: "Claude Code".to_string(),
            note: "Runs the claude CLI you already have, billed to your Claude plan rather than \
                   an API key. Your docs go to Anthropic. A build reads every file in the \
                   project, so the model is the cost: a large one can spend a whole usage \
                   window on a single build."
                .to_string(),
            default_model: CLAUDE_CLI_DEFAULT_MODEL.to_string(),
        });
    }
    for (id, label, env, vendor, model) in CLOUD_BACKENDS {
        if probe.env_keys.iter().any(|k| k == id) {
            out.push(KnowledgeBackend {
                id: id.to_string(),
                label: label.to_string(),
                note: format!(
                    "Uses the {env} from the environment Agency started in. Your docs go to \
                     {vendor}, billed to that key."
                ),
                default_model: model.to_string(),
            });
        }
    }
    if probe.ollama {
        out.push(KnowledgeBackend {
            id: "ollama".to_string(),
            label: "Ollama".to_string(),
            note: "Runs the model on this machine. Nothing is sent anywhere and nothing is \
                   billed. Name a model you have pulled, and not a small one: below about 14b \
                   the answers come back as prose or half-formed JSON, and those chunks are \
                   retried and then dropped from the graph."
                .to_string(),
            // 14b and not 7b because graphify's own ollama warning says so:
            // "model too small for JSON instruction following - try a larger
            // model with --model (e.g. --model qwen2.5-coder:14b)".
            default_model: "qwen2.5-coder:14b".to_string(),
        });
    }
    if let Some(url) = probe.local_model_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        out.push(KnowledgeBackend {
            id: LOCAL_MODEL.to_string(),
            label: "Local model".to_string(),
            note: format!(
                "Sends your docs to {url}, the local server from settings. Nothing leaves your \
                 machine. Name the model it serves, and start it before you build."
            ),
            default_model: String::new(),
        });
    }
    out.push(KnowledgeBackend {
        id: CODE_ONLY.to_string(),
        label: "No LLM".to_string(),
        note: "Indexes code only, with tree-sitter, on this machine. Docs, papers and images are \
               skipped. Costs nothing and sends nothing."
            .to_string(),
        default_model: String::new(),
    });
    out
}

/// The model a backend runs on when the user picks the backend and names no
/// model: the same string its offer shows as the placeholder.
///
/// Empty for the backends only the user can name a model for (a local server,
/// and the build that runs no model at all). Everywhere else the choice is
/// written into the command, so the panel and the build can never disagree
/// about which model reads the project, and a vendor moving its own default
/// cannot move what a build costs.
pub fn default_model_for(backend: &str, probe: &BackendProbe) -> String {
    knowledge_backends(probe)
        .into_iter()
        .find(|b| b.id == backend)
        .map(|b| b.default_model)
        .unwrap_or_default()
}

/// The build command a (backend, model) choice writes into `[knowledge]`. The
/// whole choice lives in this one string, so what runs is always exactly what
/// the settings panel shows, and a user who wants something else can type it.
///
/// `--max-concurrency 1` on the two local backends is graphify's own guidance
/// for a local server: four chunks in flight is four copies of the model's
/// context on one machine.
pub fn build_command_for(backend: &str, model: &str, local_model_url: &str) -> String {
    let model = model.trim();
    let named = |base: String| match model.is_empty() {
        true => base,
        false => format!("{base} --model {model}"),
    };
    match backend {
        CODE_ONLY => "graphify . --code-only".to_string(),
        // The claude CLI takes its model from graphify's env var and ignores
        // `--model` (graphify never passes one through to `claude -p`).
        CLAUDE_CLI if model.is_empty() => "graphify . --backend claude-cli".to_string(),
        CLAUDE_CLI => format!("GRAPHIFY_CLAUDE_CLI_MODEL={model} graphify . --backend claude-cli"),
        LOCAL_MODEL => named(format!(
            "OPENAI_BASE_URL={local_model_url} OPENAI_API_KEY=local \
             graphify . --backend openai --max-concurrency 1"
        )),
        "ollama" => named("graphify . --backend ollama --max-concurrency 1".to_string()),
        other => named(format!("graphify . --backend {other}")),
    }
}

/// A saved build command that leaves the model to the CLI, rewritten to name
/// the backend's own default. `None` when there is nothing to repair.
///
/// The one migration this panel performs, and it is deliberately narrow: only a
/// command this picker itself wrote (it has to round-trip), only where the
/// model is missing, and only to the model the panel already shows as that
/// backend's default. A hand-written command is never touched, and neither is
/// one that names a model.
///
/// It exists because the model was not always part of the choice. A command
/// saved as `graphify . --backend claude-cli` runs `claude -p` per file on
/// whatever the CLI defaults to, which is how one project's builds spent a
/// 5-hour usage window in about 30 minutes. Leaving that saved command alone
/// and only warning about it would leave the post-merge rebuild firing it all
/// day for anyone who never opens this panel.
pub fn name_the_default_model(
    command: &str,
    probe: &BackendProbe,
    local_model_url: &str,
) -> Option<String> {
    let (backend, model) = build_selection(command);
    if backend == CUSTOM_BACKEND || !model.is_empty() {
        return None;
    }
    let default = default_model_for(&backend, probe);
    match default.is_empty() {
        true => None,
        false => Some(build_command_for(&backend, &default, local_model_url)),
    }
}

/// The (backend, model) a build command expresses, or [`CUSTOM_BACKEND`] when
/// it isn't one this panel wrote.
///
/// Recognition is a round trip rather than a parse: whatever is read out of the
/// command is composed back with [`build_command_for`] and compared. A command
/// the picker can't reproduce exactly is reported as custom and left alone,
/// which is the only way a hand-written command survives being looked at.
pub fn build_selection(command: &str) -> (String, String) {
    let argv = split_command(command);
    let flag = |name: &str| {
        argv.iter()
            .position(|a| a == name)
            .and_then(|i| argv.get(i + 1))
            .cloned()
            .unwrap_or_default()
    };
    let assignment = |name: &str| {
        argv.iter()
            .find_map(|a| a.strip_prefix(&format!("{name}=")))
            .unwrap_or_default()
            .to_string()
    };
    let local_url = assignment("OPENAI_BASE_URL");
    let backend = match () {
        _ if argv.iter().any(|a| a == "--code-only") => CODE_ONLY.to_string(),
        _ if !local_url.is_empty() => LOCAL_MODEL.to_string(),
        _ => flag("--backend"),
    };
    let model = match backend.as_str() {
        CLAUDE_CLI => assignment("GRAPHIFY_CLAUDE_CLI_MODEL"),
        _ => flag("--model"),
    };
    match build_command_for(&backend, &model, &local_url) == command.trim() {
        true => (backend, model),
        false => (CUSTOM_BACKEND.to_string(), String::new()),
    }
}

/// The build command a project gets before anyone has chosen one.
///
/// Only two of the offers are safe to pick on someone's behalf, and this picks
/// between them: the CLI that is sitting right there on the PATH, or no LLM at
/// all. An API key can be a placeholder (Agency injects `OPENAI_API_KEY` for a
/// configured local model, and graphify's auto-detection reads that as a paid
/// OpenAI account), and a local server that isn't running fails the same
/// way. Both are real choices, but a user makes them, having read what they
/// cost. Auto-detection is never left to graphify: a build of an 85-file corpus
/// went to an LM Studio that wasn't listening and wrote no graph at all.
pub fn default_build_command(claude_on_path: bool) -> String {
    match claude_on_path {
        true => build_command_for(CLAUDE_CLI, CLAUDE_CLI_DEFAULT_MODEL, ""),
        false => build_command_for(CODE_ONLY, "", ""),
    }
}

/// The binary a command line runs, skipping the `VAR=value` assignments a
/// command can carry in front of it (the local-model and claude-cli choices
/// both write those, and a bare first-token check reads the assignment as the
/// program and reports the tooling missing).
pub fn command_binary(command_line: &str) -> Option<String> {
    split_command(command_line).into_iter().find(|token| !is_env_assignment(token))
}

/// `NAME=value`, in the shell's own sense: a name of word characters, an `=`,
/// and anything after it.
fn is_env_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else { return false };
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
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
    table.insert("rebuild_on_merge".into(), toml::Value::Boolean(k.rebuild_on_merge));
    for (key, val) in [("serve_command", &k.serve_command), ("build_command", &k.build_command)] {
        if let Some(s) = val.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            table.insert(key.into(), toml::Value::String(s.to_string()));
        }
    }
    doc.insert("knowledge".into(), toml::Value::Table(table));

    let text = toml::to_string_pretty(&toml::Value::Table(doc)).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&path, text)
}

/// Persist the `[issues]` section into `.agency/agency.local.toml`, the
/// gitignored per-machine file. Mirrors [`save_knowledge`].
///
/// The local file and not the tracked `agency.toml`, even though `sync` is
/// conceptually a fact about the repo: Agency has never written a tracked file,
/// and starting here would leave the user's checkout dirty every time they
/// touched this toggle — the exact condition the whole untracked-issues design
/// exists to avoid. A team that wants the decision to travel with the repo
/// commits `[issues] sync = true` into `agency.toml` by hand, and `load` already
/// merges that under whatever this writes. See `docs/tracked-issues.md`.
pub fn save_issues(repo_path: &Path, i: &IssuesConfig) -> std::io::Result<()> {
    let dir = repo_path.join(".agency");
    let path = dir.join("agency.local.toml");
    let mut doc = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| toml::from_str::<toml::Value>(&t).ok())
        .and_then(|v| v.as_table().cloned())
        .unwrap_or_default();

    let mut table = toml::value::Table::new();
    // Always written, so turning either back off is durable rather than
    // falling through to whatever the tracked file says.
    table.insert("sync".into(), toml::Value::Boolean(i.sync));
    table.insert("auto".into(), toml::Value::Boolean(i.auto));
    // The remote is written even when it is the default, for the same reason:
    // omitting `origin` left a tracked `remote = "tracker"` to win the merge,
    // so a user who picked `origin` here got the repo's choice back on the next
    // read and no way to say otherwise from the UI.
    let remote = i.remote.trim();
    let remote = if remote.is_empty() { default_remote() } else { remote.to_string() };
    table.insert("remote".into(), toml::Value::String(remote));
    doc.insert("issues".into(), toml::Value::Table(table));

    let text = toml::to_string_pretty(&toml::Value::Table(doc)).map_err(std::io::Error::other)?;
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

    let text = toml::to_string_pretty(&toml::Value::Table(doc)).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&path, text)
}

/// True when the effective run list comes from the tracked `agency.toml`
/// rather than the per-machine `agency.local.toml` — i.e. the whole team sees
/// these commands. False when they are local-only or unset. The UI uses it to
/// say where the scripts being edited actually live.
pub fn run_scripts_are_shared(repo_path: &Path) -> bool {
    let defines_runs = |name: &str| {
        read_value(&repo_path.join(".agency").join(name))
            .and_then(|v| v.get("scripts").cloned())
            .map(|s| {
                let listed =
                    s.get("runs").and_then(|r| r.as_array()).is_some_and(|a| !a.is_empty());
                listed || s.get("run").and_then(|r| r.as_str()).is_some()
            })
            .unwrap_or(false)
    };
    !defines_runs("agency.local.toml") && defines_runs("agency.toml")
}

/// Persist the run list into the `[scripts]` section of
/// `.agency/agency.local.toml` — the gitignored, per-machine override file, so
/// configuring the Run tab never dirties the tracked `agency.toml` (and never
/// rewrites its comments). An empty list removes the keys entirely instead of
/// writing an empty array, which lets the tracked file's scripts apply again.
/// Sibling `[scripts]` keys (`setup`, `archive`) are preserved.
///
/// The legacy single-command keys are dropped from the local file whenever a
/// list is written: leaving `run =` behind would resurrect the old command the
/// moment the list was emptied.
pub fn save_run_scripts(repo_path: &Path, scripts: &[RunScript]) -> std::io::Result<()> {
    let dir = repo_path.join(".agency");
    let path = dir.join("agency.local.toml");
    let mut doc = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| toml::from_str::<toml::Value>(&t).ok())
        .and_then(|v| v.as_table().cloned())
        .unwrap_or_default();

    let mut table = doc.get("scripts").and_then(|v| v.as_table().cloned()).unwrap_or_default();
    table.remove("run");
    table.remove("run_mode");

    let entries: Vec<toml::Value> = dedupe_by_name(scripts.to_vec())
        .into_iter()
        .map(|s| {
            let mut t = toml::value::Table::new();
            t.insert("name".into(), toml::Value::String(s.name));
            t.insert("command".into(), toml::Value::String(s.command));
            t.insert("web".into(), toml::Value::Boolean(s.web));
            t.insert("nonconcurrent".into(), toml::Value::Boolean(s.nonconcurrent));
            toml::Value::Table(t)
        })
        .collect();
    if entries.is_empty() {
        table.remove("runs");
    } else {
        table.insert("runs".into(), toml::Value::Array(entries));
    }

    if table.is_empty() {
        doc.remove("scripts");
    } else {
        doc.insert("scripts".into(), toml::Value::Table(table));
    }

    let text = toml::to_string_pretty(&toml::Value::Table(doc)).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&path, text)
}

/// Deep-merge `local` over `base`: tables merge recursively, every other value
/// is replaced by `local`.
fn merge_values(mut base: toml::Value, local: toml::Value) -> toml::Value {
    if let (Some(bt), Some(lt)) = (base.as_table_mut(), local.as_table()) {
        for (k, lv) in lt {
            let merged = match bt.get(k) {
                Some(bv) if bv.is_table() && lv.is_table() => merge_values(bv.clone(), lv.clone()),
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

    /// The split that makes one setting serve a team and one person: the repo
    /// decides *that* the backlog is shared, this machine decides *where*.
    #[test]
    fn issue_sync_is_off_until_the_repo_says_otherwise() {
        let dir = tempdir().unwrap();
        // Nothing configured is today's behavior: the tracker stays here.
        assert_eq!(issue_sync_remote(dir.path()), None);

        // The tracked file turns it on; the default target is the repo's own
        // remote, so a teammate cloning needs to configure nothing.
        write(dir.path(), "agency.toml", "[issues]\nsync = true\n");
        assert_eq!(issue_sync_remote(dir.path()).as_deref(), Some("origin"));

        // The gitignored file redirects it, which is what a public repo with a
        // private backlog needs. It must not have to edit the tracked file to
        // do that, or the private URL ships to everyone who clones.
        write(dir.path(), "agency.local.toml", "[issues]\nremote = \"tracker\"\n");
        assert_eq!(issue_sync_remote(dir.path()).as_deref(), Some("tracker"));

        // And a local override can switch it back off for one machine.
        write(dir.path(), "agency.local.toml", "[issues]\nsync = false\n");
        assert_eq!(issue_sync_remote(dir.path()), None);
    }

    #[test]
    fn saving_the_backlog_setting_writes_local_and_keeps_the_rest() {
        let dir = tempdir().unwrap();
        write(dir.path(), "agency.local.toml", "[knowledge]\ngraph = true\n");
        // The tracked file must never be touched by a save: writing it would
        // dirty the checkout, which is the condition this whole design avoids.
        write(dir.path(), "agency.toml", "[issues]\nsync = true\n");
        let tracked_before =
            fs::read_to_string(dir.path().join(".agency").join("agency.toml")).unwrap();

        save_issues(
            dir.path(),
            &IssuesConfig { sync: true, auto: false, remote: "tracker".into() },
        )
        .unwrap();
        assert_eq!(issue_sync_remote(dir.path()).as_deref(), Some("tracker"));
        assert!(load(dir.path()).knowledge.graph, "an unrelated section was dropped");
        assert_eq!(
            fs::read_to_string(dir.path().join(".agency").join("agency.toml")).unwrap(),
            tracked_before
        );

        // Turning it off is durable: it has to beat the tracked `sync = true`,
        // so it cannot be written by omission.
        save_issues(
            dir.path(),
            &IssuesConfig { sync: false, auto: false, remote: "tracker".into() },
        )
        .unwrap();
        assert_eq!(issue_sync_remote(dir.path()), None);
    }

    /// Automatic sync is a per-machine choice, so a repo that shares its
    /// backlog does not also decide that everyone's laptop polls for it. A
    /// tracked `auto = true` is dropped before the merge rather than merely
    /// losing it: a fresh clone has no `[issues]` in the local file at all, so
    /// there would be no local `false` to prefer, and cloning a repo would
    /// start unattended fetching and pushing here with nobody having asked.
    #[test]
    fn only_this_machine_can_turn_automatic_sync_on() {
        let dir = tempdir().unwrap();
        write(dir.path(), "agency.toml", "[issues]\nsync = true\n");
        assert!(!load(dir.path()).issues.auto);

        // The state a clone actually arrives in: the tracked file asks for it
        // and there is no local file to answer with.
        write(dir.path(), "agency.toml", "[issues]\nsync = true\nauto = true\n");
        let cfg = load(dir.path()).issues;
        assert!(!cfg.auto, "the repo does not get to decide this one");
        assert!(cfg.sync, "but sharing itself still travels with the repo");

        // Only the gitignored file can turn it on, and off again after.
        save_issues(dir.path(), &IssuesConfig { sync: true, auto: true, remote: "origin".into() })
            .unwrap();
        assert!(load(dir.path()).issues.auto);
        save_issues(dir.path(), &IssuesConfig { sync: true, auto: false, remote: "origin".into() })
            .unwrap();
        assert!(!load(dir.path()).issues.auto);
    }

    /// A local `origin` has to beat a tracked non-default remote, so the field
    /// cannot be written by omission either. Before this, choosing `origin` in
    /// Settings wrote nothing at all and the tracked `remote = "tracker"` won
    /// the merge, so the choice silently did not take.
    #[test]
    fn choosing_the_default_remote_beats_a_tracked_one() {
        let dir = tempdir().unwrap();
        write(dir.path(), "agency.toml", "[issues]\nsync = true\nremote = \"tracker\"\n");
        assert_eq!(issue_sync_remote(dir.path()).as_deref(), Some("tracker"));

        save_issues(dir.path(), &IssuesConfig { sync: true, auto: false, remote: "origin".into() })
            .unwrap();
        assert_eq!(issue_sync_remote(dir.path()).as_deref(), Some("origin"));
    }

    /// AGE-170, with the file git actually offered the user:
    /// `graphify-out/cache/stat-index.json` in Untracked Changes on main.
    /// Asserted through real `git status`, since what matters is git's reading
    /// of the pattern, not the line we wrote.
    #[test]
    fn a_built_graph_stays_out_of_the_users_changes() {
        let dir = tempdir().unwrap();
        let repo = dir.path();
        let git = |args: &[&str]| {
            let out =
                std::process::Command::new("git").args(args).current_dir(repo).output().unwrap();
            assert!(out.status.success(), "git {args:?}");
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        git(&["init", "-q", "-b", "main"]);

        exclude_graph_output(repo);
        let out = repo.join(GRAPH_OUT_DIR);
        fs::create_dir_all(out.join("cache")).unwrap();
        fs::write(out.join("cache").join("stat-index.json"), "{}").unwrap();
        fs::write(graph_path(repo), "{}").unwrap();

        assert_eq!(git(&["status", "--porcelain"]).trim(), "");
        // Idempotent: the settings toggle and every build both call it.
        exclude_graph_output(repo);
        let excludes = fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
        assert_eq!(excludes.lines().filter(|l| l.trim() == "/graphify-out/").count(), 1);
    }

    #[test]
    fn default_serve_argv_keeps_spaced_path_as_one_arg() {
        let argv = default_serve_argv(Path::new("/Users/me/My Repos/proj"));
        assert_eq!(argv.last().unwrap(), "/Users/me/My Repos/proj/graphify-out/graph.json");
        // The command itself is the first element; the path is not split.
        assert_eq!(argv[0], "uv");
        assert_eq!(argv.len(), 9);
    }

    /// AGE-83: the settings panel offers to install the tooling, so the script
    /// has to work on a machine with no uv at all — the case that made the
    /// feature look broken in the first place.
    #[test]
    fn install_script_picks_up_uv_when_it_is_missing() {
        let direct = graphify_install_script(true);
        assert!(direct.starts_with("uv tool install"), "{direct}");

        let bootstrap = graphify_install_script(false);
        let lines: Vec<&str> = bootstrap.lines().collect();
        assert!(lines[0].contains("astral.sh/uv/install.sh"), "{bootstrap}");
        assert!(
            lines[1].contains(".local/bin"),
            "a uv installed a moment ago is not on this shell's PATH yet: {bootstrap}"
        );
        assert_eq!(lines[2], direct);
        // Both halves of the integration come from this one distribution.
        assert!(default_serve_argv(Path::new("/r")).contains(&"graphifyy".to_string()));
        // Read past the model assignment the claude-CLI default carries in
        // front of it, which is not the program being run.
        assert_eq!(command_binary(&default_build_command(true)).as_deref(), Some("graphify"));
    }

    /// The serve command imports `mcp` and the build imports an LLM SDK; a
    /// bare `graphifyy` ships neither, so an install without the extras leaves
    /// both halves of the feature broken in a way nothing else here catches.
    #[test]
    fn install_script_carries_the_extras_both_halves_need() {
        for script in [graphify_install_script(true), graphify_install_script(false)] {
            let install = script.lines().last().unwrap();
            assert!(install.contains("--force"), "{install}");
            // Quoted: the brackets are a glob in zsh, where an unquoted spec
            // dies on "no matches found" before uv ever runs.
            assert!(install.contains("\"graphifyy[mcp,openai,anthropic]\""), "{install}");
        }
    }

    /// The build's backend is a choice about who gets the source and who pays
    /// for reading it, so it is pinned to one of the two answers that can't
    /// surprise anyone, never inferred from an exported API key.
    #[test]
    fn build_command_pins_a_backend_that_cannot_surprise_anyone() {
        assert_eq!(
            default_build_command(true),
            "GRAPHIFY_CLAUDE_CLI_MODEL=haiku graphify . --backend claude-cli"
        );
        assert_eq!(default_build_command(false), "graphify . --code-only");
    }

    /// The model is half the choice, not a detail under it: a claude-CLI build
    /// that names no model runs on whatever the CLI defaults to, which is how
    /// one first build spent a 5-hour usage window in 30 minutes. Every offer
    /// that has a default model says which, and the default command names it.
    #[test]
    fn a_build_never_leaves_the_model_to_the_cli() {
        let probe = BackendProbe {
            claude_on_path: true,
            local_model_url: Some("http://localhost:1234/v1".into()),
            env_keys: vec!["gemini".into()],
            ..BackendProbe::default()
        };
        assert_eq!(default_model_for(CLAUDE_CLI, &probe), CLAUDE_CLI_DEFAULT_MODEL);
        assert_eq!(default_model_for("gemini", &probe), "gemini-3-flash-preview");
        // Nothing to name: no model runs, or only the user knows what the
        // local server serves.
        assert_eq!(default_model_for(CODE_ONLY, &probe), "");
        assert_eq!(default_model_for(LOCAL_MODEL, &probe), "");
        // A backend this machine can't run has no default to offer.
        assert_eq!(default_model_for("kimi", &probe), "");

        // The default model is the one that reads a doc cheapest, not the one
        // the plan would otherwise reach for.
        assert!(default_build_command(true).contains(CLAUDE_CLI_DEFAULT_MODEL));
        for b in knowledge_backends(&probe) {
            let cmd = build_command_for(&b.id, &b.default_model, "http://localhost:1234/v1");
            assert_eq!(build_selection(&cmd), (b.id.clone(), b.default_model.clone()), "{cmd}");
        }
    }

    /// Every machine can build *something*: a user with no agent CLI, no API
    /// key and no local server still gets the code-only offer, which is the
    /// whole answer to "what if I only run local agents".
    #[test]
    fn a_bare_machine_is_still_offered_a_graph_it_can_build() {
        let bare = knowledge_backends(&BackendProbe::default());
        assert_eq!(bare.len(), 1, "{bare:?}");
        assert_eq!(bare[0].id, CODE_ONLY);
        assert_eq!(default_build_command(false), build_command_for(CODE_ONLY, "", ""));
    }

    /// The offer is ordered by what it takes for granted, and every entry says
    /// what it costs: the note is the "before it runs" half of the feature.
    #[test]
    fn backends_are_offered_in_order_and_all_say_what_they_cost() {
        let probe = BackendProbe {
            claude_on_path: true,
            ollama: true,
            local_model_url: Some("http://localhost:1234/v1".into()),
            env_keys: vec!["gemini".into(), "openai".into()],
        };
        let offered = knowledge_backends(&probe);
        let ids: Vec<&str> = offered.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["claude-cli", "gemini", "openai", "ollama", "lmstudio", "code-only"]);
        for b in &offered {
            assert!(!b.note.trim().is_empty(), "{} has no note", b.id);
        }
        // The local one names the server it will talk to, since a URL from
        // settings is the whole difference between local and not.
        let local = offered.iter().find(|b| b.id == LOCAL_MODEL).unwrap();
        assert!(local.note.contains("http://localhost:1234/v1"), "{}", local.note);
    }

    /// A chosen backend round-trips: the command the picker writes is the
    /// command it reads back, or the picker has to say "custom" and keep its
    /// hands off. Anything less silently rewrites a hand-edited command.
    #[test]
    fn a_backend_choice_round_trips_through_the_command() {
        let url = "http://localhost:1234/v1";
        let cases = [
            (CLAUDE_CLI, ""),
            (CLAUDE_CLI, "haiku"),
            (CODE_ONLY, ""),
            (LOCAL_MODEL, "qwen3-coder-30b"),
            ("ollama", "qwen2.5-coder:7b"),
            ("gemini", ""),
        ];
        for (backend, model) in cases {
            let cmd = build_command_for(backend, model, url);
            assert_eq!(build_selection(&cmd), (backend.to_string(), model.to_string()), "{cmd}");
            assert_eq!(command_binary(&cmd).as_deref(), Some("graphify"), "{cmd}");
        }
        // Anything else is left alone rather than reinterpreted.
        assert_eq!(build_selection("graphify . --mode deep").0, CUSTOM_BACKEND);
        assert_eq!(build_selection("graphify .").0, CUSTOM_BACKEND);
    }

    /// A command saved before the model was part of the choice runs on whatever
    /// the CLI defaults to. It is repaired to name the backend's default, and
    /// nothing else is: a hand-written command means the user knows what they
    /// wrote, and one that already names a model has already been decided.
    #[test]
    fn a_command_that_names_no_model_is_repaired_and_nothing_else_is() {
        let url = "http://localhost:1234/v1";
        let probe = BackendProbe {
            claude_on_path: true,
            local_model_url: Some(url.into()),
            env_keys: vec!["gemini".into()],
            ..BackendProbe::default()
        };
        let repair = |cmd: &str| name_the_default_model(cmd, &probe, url);

        assert_eq!(
            repair("graphify . --backend claude-cli").as_deref(),
            Some("GRAPHIFY_CLAUDE_CLI_MODEL=haiku graphify . --backend claude-cli")
        );
        assert_eq!(
            repair("graphify . --backend gemini").as_deref(),
            Some("graphify . --backend gemini --model gemini-3-flash-preview")
        );
        // Already named: the user's choice, left exactly as it is.
        assert_eq!(repair("GRAPHIFY_CLAUDE_CLI_MODEL=opus graphify . --backend claude-cli"), None);
        // Hand-written, so not this panel's to rewrite.
        assert_eq!(repair("graphify . --mode deep"), None);
        // Nothing to name: no model runs, or only the user knows what the local
        // server serves.
        assert_eq!(repair("graphify . --code-only"), None);
        assert_eq!(repair(&build_command_for(LOCAL_MODEL, "", url)), None);
        // A backend this machine cannot run is not repaired into one it can.
        assert_eq!(repair("graphify . --backend kimi"), None);
        // Repairing twice changes nothing.
        let once = repair("graphify . --backend claude-cli").unwrap();
        assert_eq!(repair(&once), None);
    }

    /// Agency injects `OPENAI_API_KEY=lm-studio` + `OPENAI_BASE_URL` into every
    /// agent session, so an app started from one inherits a key that is not an
    /// OpenAI account at all. Offering that as "OpenAI" would name the wrong
    /// vendor on the one line the user reads before spending anything.
    #[test]
    fn a_local_server_is_never_offered_as_a_cloud_account() {
        let env = |pairs: Vec<(&'static str, &'static str)>| {
            move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        assert_eq!(env_backend_keys(env(vec![("OPENAI_API_KEY", "sk-real")])), ["openai"]);
        assert!(env_backend_keys(env(vec![
            ("OPENAI_API_KEY", "lm-studio"),
            ("OPENAI_BASE_URL", "http://localhost:1234/v1"),
        ]))
        .is_empty());
        // A real key elsewhere still counts, base URL or not.
        assert_eq!(
            env_backend_keys(env(vec![
                ("GEMINI_API_KEY", "k"),
                ("OPENAI_API_KEY", "lm-studio"),
                ("OPENAI_BASE_URL", "http://localhost:1234/v1"),
            ])),
            ["gemini"]
        );
    }

    /// The claude CLI reads its model from graphify's env var, so that choice
    /// lands in front of the command, where a first-token PATH check used to
    /// see `GRAPHIFY_CLAUDE_CLI_MODEL=haiku` and call the tooling missing.
    #[test]
    fn command_binary_looks_past_the_env_assignments() {
        assert_eq!(command_binary("A=1 B=2 graphify .").as_deref(), Some("graphify"));
        assert_eq!(command_binary("graphify .").as_deref(), Some("graphify"));
        assert_eq!(command_binary("./scripts/build.sh").as_deref(), Some("./scripts/build.sh"));
        // Not an assignment: no name in front of the `=`.
        assert_eq!(command_binary("=x graphify").as_deref(), Some("=x"));
        assert_eq!(command_binary("   "), None);
    }

    #[test]
    fn split_command_honors_quotes() {
        assert_eq!(split_command("uv run python"), vec!["uv", "run", "python"]);
        assert_eq!(
            split_command("python -m graphify.serve \"/my repo/graph.json\""),
            vec!["python", "-m", "graphify.serve", "/my repo/graph.json"]
        );
        assert_eq!(split_command("a 'b c' d"), vec!["a", "b c", "d"]);
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
        assert!(c.preview.agent_tools, "preview tools are on unless switched off");
    }

    #[test]
    fn preview_agent_tools_can_be_switched_off() {
        let dir = tempdir().unwrap();
        write(dir.path(), "agency.toml", "[preview]\nagent_tools = false\n");
        assert!(!load(dir.path()).preview.agent_tools);
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
                build_command: Some("  graphify . --skip-html  ".to_string()),
                ..KnowledgeConfig::default()
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
        save_knowledge(dir.path(), &KnowledgeConfig { graph: false, ..KnowledgeConfig::default() })
            .unwrap();
        assert!(!load(dir.path()).knowledge.graph);

        // The post-merge rebuild is on for a config that predates the switch,
        // and off is durable the same way `graph = false` is.
        assert!(load(dir.path()).knowledge.rebuild_on_merge);
        save_knowledge(
            dir.path(),
            &KnowledgeConfig { graph: true, rebuild_on_merge: false, ..KnowledgeConfig::default() },
        )
        .unwrap();
        assert!(!load(dir.path()).knowledge.rebuild_on_merge);
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

    fn script(name: &str, command: &str) -> RunScript {
        RunScript { name: name.into(), command: command.into(), web: false, nonconcurrent: false }
    }

    #[test]
    fn save_run_scripts_writes_local_and_keeps_sibling_scripts() {
        let dir = tempdir().unwrap();
        write(dir.path(), "agency.local.toml", "[scripts]\nsetup = \"pnpm install\"\n");
        save_run_scripts(
            dir.path(),
            &[
                RunScript {
                    name: "  dev  ".into(),
                    command: "  pnpm dev  ".into(),
                    web: true,
                    nonconcurrent: true,
                },
                script("build mac", "./scripts/release.sh"),
            ],
        )
        .unwrap();
        let c = load(dir.path());
        let list = c.scripts.run_list();
        assert_eq!(list.len(), 2);
        // Names and commands are trimmed on the way in.
        assert_eq!(
            list[0],
            RunScript {
                name: "dev".into(),
                command: "pnpm dev".into(),
                web: true,
                nonconcurrent: true,
            }
        );
        assert_eq!(list[1].name, "build mac");
        assert!(!list[1].web, "a build script must not claim a browser preview");
        assert_eq!(c.scripts.setup.as_deref(), Some("pnpm install"));
    }

    #[test]
    fn a_legacy_single_run_command_becomes_a_one_entry_list() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "agency.toml",
            "[scripts]\nrun = \"pnpm dev --port $AGENCY_PORT\"\nrun_mode = \"nonconcurrent\"\n",
        );
        let list = load(dir.path()).scripts.run_list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, DEFAULT_RUN_NAME);
        assert_eq!(list[0].command, "pnpm dev --port $AGENCY_PORT");
        // It threads AGENCY_PORT, so it is a server worth previewing.
        assert!(list[0].web);
        assert!(list[0].nonconcurrent);
    }

    #[test]
    fn the_list_supersedes_the_legacy_command() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "agency.toml",
            r#"
                [scripts]
                run = "old-single"

                [[scripts.runs]]
                name = "dev"
                command = "pnpm dev"
            "#,
        );
        let list = load(dir.path()).scripts.run_list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].command, "pnpm dev");
    }

    #[test]
    fn saving_a_list_drops_the_local_legacy_command() {
        let dir = tempdir().unwrap();
        write(dir.path(), "agency.local.toml", "[scripts]\nrun = \"old-single\"\n");
        save_run_scripts(dir.path(), &[script("dev", "pnpm dev")]).unwrap();
        let c = load(dir.path());
        assert_eq!(c.scripts.run, None, "the superseded key must not linger");
        assert_eq!(c.scripts.run_list().len(), 1);
    }

    #[test]
    fn clearing_the_run_scripts_unshadows_the_tracked_ones() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "agency.toml",
            "[[scripts.runs]]\nname = \"dev\"\ncommand = \"base-run\"\n",
        );
        save_run_scripts(dir.path(), &[script("dev", "local-run")]).unwrap();
        assert_eq!(load(dir.path()).scripts.run_list()[0].command, "local-run");

        // An empty list removes the local key rather than persisting `[]`, so
        // the tracked scripts apply again.
        save_run_scripts(dir.path(), &[]).unwrap();
        assert_eq!(load(dir.path()).scripts.run_list()[0].command, "base-run");
    }

    #[test]
    fn duplicate_and_blank_entries_are_dropped() {
        let dir = tempdir().unwrap();
        save_run_scripts(
            dir.path(),
            &[
                script("dev", "first"),
                script("dev", "second"),
                script("", "nameless"),
                script("empty", "   "),
            ],
        )
        .unwrap();
        let list = load(dir.path()).scripts.run_list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].command, "first", "first entry wins its name");
    }

    #[test]
    fn shared_tracks_which_file_defines_the_scripts() {
        let dir = tempdir().unwrap();
        assert!(!run_scripts_are_shared(dir.path()), "nothing configured is not shared");

        write(dir.path(), "agency.toml", "[[scripts.runs]]\nname = \"dev\"\ncommand = \"x\"\n");
        assert!(run_scripts_are_shared(dir.path()));

        save_run_scripts(dir.path(), &[script("dev", "mine")]).unwrap();
        assert!(!run_scripts_are_shared(dir.path()), "a local override is not shared");
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
