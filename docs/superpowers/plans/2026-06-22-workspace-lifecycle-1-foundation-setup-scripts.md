# Workspace Lifecycle — Plan 1: Foundation + Setup Scripts

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add committed per-project config (`.agency/agency.toml`) and run a configured `setup` script when a workspace is created, so an agent's worktree has its dependencies before the agent starts.

**Architecture:** A new `agency-core::config` module loads and merges `.agency/agency.toml` (tracked) with `.agency/agency.local.toml` (machine-local) into an `AgencyConfig`. A new `agency-core::scripts` module builds the injected script environment and chains the setup script in front of the agent command (`sh -lc '<setup> && exec <agent>'`) so the agent starts only if setup succeeds. `AppState::create_run` / `rerun` call these. The `.git/info/exclude` entry narrows from `.agency/` to `.agency/worktrees/` so config is trackable.

**Tech Stack:** Rust (agency-core, agency-app/Tauri), `toml` crate (new), `serde`, `tempfile` (dev).

**Spec:** `docs/superpowers/specs/2026-06-22-workspace-lifecycle-design.md` (Foundation + Feature 1).

## Global Constraints

- Rust edition 2021; both crates already use anyhow + serde.
- No AI/Claude attribution in any commit message (user standing rule).
- Default port base = `5200`, default port block_size = `10` (verbatim from spec). Ports are **defined** in config here but not yet allocated/used — that lands in Plan 2. `script_env` takes `port: Option<u16>` and this plan always passes `None`.
- When no `.agency/agency.toml` exists, behavior must be identical to today.
- TOML config holds no secrets; secrets stay in the SQLite `settings` table.
- Default `run_mode` = `concurrent`.

## File Structure

- **Create** `crates/agency-core/src/config.rs` — `AgencyConfig`, `ScriptsConfig`, `PortsConfig`, `RunMode`, `load(repo_path)`. One responsibility: read + merge project config.
- **Create** `crates/agency-core/src/scripts.rs` — `script_env(...)`, `wrap_setup(...)`, `shell_quote(...)`. One responsibility: turn config + a worktree into the env + command that tmux runs.
- **Modify** `crates/agency-core/Cargo.toml` — add `toml = "0.8"`.
- **Modify** `crates/agency-core/src/lib.rs:1-8` — declare the two new modules.
- **Modify** `crates/agency-core/src/worktree.rs:50-66` — narrow the git-exclude entry.
- **Modify** `crates/agency-app/src/state.rs:308-340` (`create_run`) and `:402-417` (`rerun`) — load config, inject script env, chain setup.

---

### Task 1: Config module — load and merge `.agency/agency.toml`

**Files:**
- Modify: `crates/agency-core/Cargo.toml`
- Modify: `crates/agency-core/src/lib.rs:1-8`
- Create: `crates/agency-core/src/config.rs`

**Interfaces:**
- Produces:
  - `agency_core::config::AgencyConfig { scripts: ScriptsConfig, ports: PortsConfig }`
  - `ScriptsConfig { setup: Option<String>, run: Option<String>, archive: Option<String>, run_mode: RunMode }`
  - `PortsConfig { base: u16, block_size: u16 }`
  - `enum RunMode { Concurrent, Nonconcurrent }` (Default = `Concurrent`)
  - `pub fn load(repo_path: &std::path::Path) -> AgencyConfig`

- [ ] **Step 1: Add the `toml` dependency**

In `crates/agency-core/Cargo.toml`, under `[dependencies]`, add the line after `serde_json = "1"`:

```toml
toml = "0.8"
```

- [ ] **Step 2: Declare the module**

In `crates/agency-core/src/lib.rs`, add to the module list (keep alphabetical):

```rust
pub mod config;
pub mod git;
pub mod merge;
pub mod profile;
pub mod registry;
pub mod scripts;
pub mod setup;
pub mod supervisor;
pub mod tmux;
pub mod worktree;
```

(Note: `scripts` is added now too so the crate compiles after Task 2; if you implement strictly in order, add only `config` here and add `scripts` in Task 2.)

- [ ] **Step 3: Write the failing tests**

Create `crates/agency-core/src/config.rs` with only the tests first:

```rust
use serde::Deserialize;
use std::path::Path;

// (types + load go here in Step 5)

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
            "#,
        );
        let c = load(dir.path());
        assert_eq!(c.scripts.setup.as_deref(), Some("pnpm install"));
        assert_eq!(c.scripts.run.as_deref(), Some("pnpm dev --port $AGENCY_PORT"));
        assert_eq!(c.scripts.archive.as_deref(), Some("./cleanup.sh"));
        assert_eq!(c.scripts.run_mode, RunMode::Nonconcurrent);
        assert_eq!(c.ports.base, 4000);
        assert_eq!(c.ports.block_size, 5);
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
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p agency-core config::`
Expected: FAIL to compile — `load`, `AgencyConfig`, `RunMode` not found.

- [ ] **Step 5: Implement the module**

Add above the `#[cfg(test)]` block in `crates/agency-core/src/config.rs`:

```rust
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgencyConfig {
    #[serde(default)]
    pub scripts: ScriptsConfig,
    #[serde(default)]
    pub ports: PortsConfig,
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
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p agency-core config::`
Expected: PASS (4 tests).

- [ ] **Step 7: Commit**

```bash
git add crates/agency-core/Cargo.toml crates/agency-core/src/lib.rs crates/agency-core/src/config.rs
git commit -m "Add agency-core config module for .agency/agency.toml"
```

---

### Task 2: Scripts module — env injection + setup chaining

**Files:**
- Create: `crates/agency-core/src/scripts.rs`
- Modify: `crates/agency-core/src/lib.rs` (add `pub mod scripts;` if not already added in Task 1)

**Interfaces:**
- Consumes: nothing from Task 1 (independent).
- Produces:
  - `pub fn script_env(workspace_path: &Path, root_path: &Path, workspace_name: &str, port: Option<u16>) -> Vec<(String, String)>`
  - `pub fn wrap_setup(setup: Option<&str>, command: &str, args: &[String]) -> (String, Vec<String>)`

- [ ] **Step 1: Write the failing tests**

Create `crates/agency-core/src/scripts.rs` with the tests first:

```rust
use std::path::Path;

// (implementation goes here in Step 3)

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn script_env_sets_workspace_vars_without_port() {
        let env = script_env(
            Path::new("/repo/.agency/worktrees/fix-login-a3k2"),
            Path::new("/repo"),
            "fix-login-a3k2",
            None,
        );
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("AGENCY_WORKSPACE_PATH").as_deref(), Some("/repo/.agency/worktrees/fix-login-a3k2"));
        assert_eq!(get("AGENCY_ROOT_PATH").as_deref(), Some("/repo"));
        assert_eq!(get("AGENCY_WORKSPACE_NAME").as_deref(), Some("fix-login-a3k2"));
        assert!(get("AGENCY_PORT").is_none());
    }

    #[test]
    fn script_env_includes_port_when_present() {
        let env = script_env(Path::new("/w"), Path::new("/r"), "x", Some(5210));
        let port = env.iter().find(|(n, _)| n == "AGENCY_PORT").map(|(_, v)| v.clone());
        assert_eq!(port.as_deref(), Some("5210"));
    }

    #[test]
    fn wrap_setup_none_returns_command_unchanged() {
        let (cmd, args) = wrap_setup(None, "claude", &["--print".to_string(), "hi there".to_string()]);
        assert_eq!(cmd, "claude");
        assert_eq!(args, vec!["--print".to_string(), "hi there".to_string()]);
    }

    #[test]
    fn wrap_setup_blank_setup_returns_command_unchanged() {
        let (cmd, _) = wrap_setup(Some("   "), "claude", &[]);
        assert_eq!(cmd, "claude");
    }

    #[test]
    fn wrap_setup_chains_and_quotes() {
        let (cmd, args) = wrap_setup(
            Some("pnpm install"),
            "claude",
            &["--print".to_string(), "fix the bug".to_string()],
        );
        assert_eq!(cmd, "sh");
        assert_eq!(
            args,
            vec![
                "-lc".to_string(),
                "pnpm install && exec claude --print 'fix the bug'".to_string(),
            ]
        );
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("plain"), "plain");
        assert_eq!(shell_quote(""), "''");
    }

    // keep PathBuf import used
    #[allow(dead_code)]
    fn _p() -> PathBuf { PathBuf::new() }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p agency-core scripts::`
Expected: FAIL to compile — `script_env`, `wrap_setup`, `shell_quote` not found.

- [ ] **Step 3: Implement the module**

Add above the `#[cfg(test)]` block in `crates/agency-core/src/scripts.rs`:

```rust
/// Environment variables made available to every lifecycle script and to the
/// agent process. `port` is `None` until the port allocator lands (Plan 2).
pub fn script_env(
    workspace_path: &Path,
    root_path: &Path,
    workspace_name: &str,
    port: Option<u16>,
) -> Vec<(String, String)> {
    let mut env = vec![
        ("AGENCY_WORKSPACE_PATH".to_string(), workspace_path.to_string_lossy().to_string()),
        ("AGENCY_ROOT_PATH".to_string(), root_path.to_string_lossy().to_string()),
        ("AGENCY_WORKSPACE_NAME".to_string(), workspace_name.to_string()),
    ];
    if let Some(p) = port {
        env.push(("AGENCY_PORT".to_string(), p.to_string()));
    }
    env
}

/// If a non-blank `setup` script is configured, return a command that runs setup
/// first and only then `exec`s the agent (`sh -lc '<setup> && exec <agent>'`), so
/// the agent starts only when setup succeeds. Otherwise return the agent command
/// unchanged. The agent command and args are shell-quoted for safe embedding;
/// the setup string is passed through verbatim (it is the user's own shell line).
pub fn wrap_setup(setup: Option<&str>, command: &str, args: &[String]) -> (String, Vec<String>) {
    match setup {
        Some(s) if !s.trim().is_empty() => {
            let mut quoted = vec![shell_quote(command)];
            quoted.extend(args.iter().map(|a| shell_quote(a)));
            let agent = quoted.join(" ");
            let line = format!("{} && exec {}", s.trim(), agent);
            ("sh".to_string(), vec!["-lc".to_string(), line])
        }
        _ => (command.to_string(), args.to_vec()),
    }
}

/// Minimal POSIX shell single-quoting. Safe characters pass through unquoted;
/// everything else is wrapped in single quotes with embedded `'` escaped.
pub fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p agency-core scripts::`
Expected: PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/scripts.rs crates/agency-core/src/lib.rs
git commit -m "Add agency-core scripts module: env injection and setup chaining"
```

---

### Task 3: Narrow the git-exclude entry for `.agency/`

**Files:**
- Modify: `crates/agency-core/src/worktree.rs:50-66`

**Interfaces:**
- Consumes: nothing.
- Produces: `ensure_excluded` now writes `.agency/worktrees/` and `.agency/agency.local.toml` and removes a legacy `.agency/` line.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/agency-core/src/worktree.rs` (create a `#[cfg(test)] mod tests` block if none exists):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn exclude_path(repo: &std::path::Path) -> std::path::PathBuf {
        repo.join(".git").join("info").join("exclude")
    }

    #[test]
    fn ensure_excluded_writes_narrow_entries() {
        let dir = tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        fs::create_dir_all(repo.join(".git").join("info")).unwrap();
        let mgr = WorktreeManager::new(repo.clone());
        mgr.ensure_excluded().unwrap();
        let body = fs::read_to_string(exclude_path(&repo)).unwrap();
        let lines: Vec<&str> = body.lines().map(|l| l.trim()).collect();
        assert!(lines.contains(&".agency/worktrees/"));
        assert!(lines.contains(&".agency/agency.local.toml"));
        assert!(!lines.contains(&".agency/"));
    }

    #[test]
    fn ensure_excluded_migrates_legacy_entry() {
        let dir = tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let info = repo.join(".git").join("info");
        fs::create_dir_all(&info).unwrap();
        fs::write(info.join("exclude"), "# existing\n.agency/\n").unwrap();
        let mgr = WorktreeManager::new(repo.clone());
        mgr.ensure_excluded().unwrap();
        let body = fs::read_to_string(exclude_path(&repo)).unwrap();
        let lines: Vec<&str> = body.lines().map(|l| l.trim()).collect();
        assert!(lines.contains(&"# existing"), "preserves unrelated lines");
        assert!(!lines.contains(&".agency/"), "drops legacy broad ignore");
        assert!(lines.contains(&".agency/worktrees/"));
    }

    #[test]
    fn ensure_excluded_is_idempotent() {
        let dir = tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        fs::create_dir_all(repo.join(".git").join("info")).unwrap();
        let mgr = WorktreeManager::new(repo.clone());
        mgr.ensure_excluded().unwrap();
        let first = fs::read_to_string(exclude_path(&repo)).unwrap();
        mgr.ensure_excluded().unwrap();
        let second = fs::read_to_string(exclude_path(&repo)).unwrap();
        assert_eq!(first, second);
    }
}
```

Note: `ensure_excluded` is currently private. Change its signature from `fn ensure_excluded` to `pub(crate) fn ensure_excluded` so the test (same crate) can call it.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p agency-core worktree::`
Expected: FAIL — assertions on `.agency/worktrees/` fail (current code writes `.agency/`).

- [ ] **Step 3: Replace `ensure_excluded`**

Replace the existing method (worktree.rs:50-66) with:

```rust
    /// Ensure agency's local artifacts are git-excluded without ignoring the
    /// tracked `.agency/agency.toml`. Writes `.agency/worktrees/` and
    /// `.agency/agency.local.toml`, and migrates away the legacy broad
    /// `.agency/` entry if present.
    pub(crate) fn ensure_excluded(&self) -> Result<()> {
        let exclude = self.repo_path.join(".git").join("info").join("exclude");
        let current = std::fs::read_to_string(&exclude).unwrap_or_default();
        let wanted = [".agency/worktrees/", ".agency/agency.local.toml"];

        let had_legacy = current.lines().any(|l| l.trim() == ".agency/");
        let mut lines: Vec<String> = current
            .lines()
            .filter(|l| l.trim() != ".agency/")
            .map(|l| l.to_string())
            .collect();

        let mut changed = had_legacy;
        for w in wanted {
            if !lines.iter().any(|l| l.trim() == w) {
                lines.push(w.to_string());
                changed = true;
            }
        }

        if changed {
            if let Some(parent) = exclude.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut out = lines.join("\n");
            out.push('\n');
            std::fs::write(&exclude, out)?;
        }
        Ok(())
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p agency-core worktree::`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/worktree.rs
git commit -m "Narrow .agency git-exclude so agency.toml is trackable"
```

---

### Task 4: Wire setup into run creation and rerun

**Files:**
- Modify: `crates/agency-app/src/state.rs:308-340` (`create_run`)
- Modify: `crates/agency-app/src/state.rs:402-417` (`rerun`)

**Interfaces:**
- Consumes: `agency_core::config::load`, `agency_core::scripts::{script_env, wrap_setup}` (Tasks 1–2).
- Produces: no new public API. Behavior change only.

The composition logic (`wrap_setup`, `script_env`) is already unit-tested in Task 2. This task is the wiring; it is verified by build + a manual smoke test because it needs a real git repo + tmux.

- [ ] **Step 1: Update `create_run`**

Replace the body of `create_run` (state.rs:308-340) from the `let repo = ...` line through the `start_session` call. Current:

```rust
    pub fn create_run(&self, project_id: &str, prompt: &str, agent: &str, base: &str) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {agent}"))?
        };
        let id = new_task_id(prompt);
        let worktree = WorktreeManager::new(repo).create(&id, base)?;

        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        let args: Vec<String> = profile
            .render_args(prompt)
            .into_iter()
            .filter(|a| !a.is_empty())
            .collect();

        self.tmux
            .start_session(&session_name(&id), &worktree.path, &profile.command, &args, &env)?;
```

Replace with:

```rust
    pub fn create_run(&self, project_id: &str, prompt: &str, agent: &str, base: &str) -> Result<RunInfo> {
        let repo = self.project_repo(project_id)?;
        let config = agency_core::config::load(&repo);
        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {agent}"))?
        };
        let id = new_task_id(prompt);
        let worktree = WorktreeManager::new(repo.clone()).create(&id, base)?;

        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        env.extend(agency_core::scripts::script_env(&worktree.path, &repo, &id, None));
        let args: Vec<String> = profile
            .render_args(prompt)
            .into_iter()
            .filter(|a| !a.is_empty())
            .collect();
        let (command, args) =
            agency_core::scripts::wrap_setup(config.scripts.setup.as_deref(), &profile.command, &args);

        self.tmux
            .start_session(&session_name(&id), &worktree.path, &command, &args, &env)?;
```

(The rest of the method — building the `Run`, inserting it, returning `run_info` — is unchanged.)

- [ ] **Step 2: Update `rerun`**

Replace the body of `rerun` (state.rs:402-417). Current:

```rust
    pub fn rerun(&self, id: &str) -> Result<RunInfo> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);
        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(&run.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {}", run.agent))?
        };
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        let args = profile.render_args(&run.prompt);
        self.tmux.kill_session(&session_name(id)).ok();
        self.tmux.start_session(&session_name(id), &worktree, &profile.command, &args, &env)?;
        Ok(self.run_info(&run))
    }
```

Replace with:

```rust
    pub fn rerun(&self, id: &str) -> Result<RunInfo> {
        let run = self.run_record(id)?;
        let repo = self.project_repo(&run.project_id)?;
        let config = agency_core::config::load(&repo);
        let worktree = repo.join(".agency").join("worktrees").join(&run.id);
        let profile = {
            let reg = self.registry.lock().unwrap();
            reg.get_profile(&run.agent)?
                .ok_or_else(|| anyhow!("unknown agent profile: {}", run.agent))?
        };
        let mut env = self.provider_env()?;
        env.extend(profile.env.iter().cloned());
        env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, None));
        let args = profile.render_args(&run.prompt);
        let (command, args) =
            agency_core::scripts::wrap_setup(config.scripts.setup.as_deref(), &profile.command, &args);
        self.tmux.kill_session(&session_name(id)).ok();
        self.tmux.start_session(&session_name(id), &worktree, &command, &args, &env)?;
        Ok(self.run_info(&run))
    }
```

- [ ] **Step 3: Build the workspace**

Run: `cargo build`
Expected: compiles cleanly (no warnings about unused `config`).

- [ ] **Step 4: Run the full core test suite**

Run: `cargo test -p agency-core`
Expected: PASS (all existing + new tests).

- [ ] **Step 5: Manual smoke test**

1. In a scratch git repo with at least one commit, create `.agency/agency.toml`:

```toml
[scripts]
setup = "echo SETUP_RAN > setup-marker.txt"
```

2. Add the repo as a project in agency, spawn an agent with the `shell` profile.
3. Confirm the agent's worktree contains `setup-marker.txt` with `SETUP_RAN` (setup ran before the shell), and the shell session is live.
4. Edit the setup line to `false` (a command that fails); spawn another agent; confirm the agent shell does **not** start (the pane shows the failed setup and stays dead via `remain-on-exit`).
5. Remove `.agency/agency.toml`; spawn again; confirm behavior is identical to before this change (agent starts immediately, no setup).

- [ ] **Step 6: Commit**

```bash
git add crates/agency-app/src/state.rs
git commit -m "Run configured setup script before agent starts in a workspace"
```

---

## Self-Review

**Spec coverage (Foundation + Feature 1):**
- Committed `.agency/agency.toml` + `.agency/agency.local.toml` merge → Task 1. ✓
- Secrets stay in SQLite (no secret fields in config types) → Task 1 (by omission). ✓
- Exclude narrowing + legacy migration → Task 3. ✓
- Injected env (`AGENCY_WORKSPACE_PATH`/`ROOT_PATH`/`WORKSPACE_NAME`; `PORT` deferred) → Task 2 + Task 4. ✓
- Chained `setup && exec agent` single-session model → Task 2 (`wrap_setup`) + Task 4 wiring. ✓
- `rerun` also prepares the workspace → Task 4 Step 2. ✓
- No-config behavior identical to today → Task 1 defaults + Task 2 `wrap_setup` passthrough; verified in Task 4 Step 5.5. ✓

**Deferred to later plans (intentional, noted in spec sequencing):**
- Port allocation + `AGENCY_PORT` value → Plan 2 (`script_env` already accepts `port: Option<u16>`).
- `run`/`archive` scripts execution, `run_mode` → Plans 2 / 3 (parsed here, unused).

**Placeholder scan:** none — every step has concrete code/commands.

**Type consistency:** `script_env(workspace_path, root_path, workspace_name, port)` and `wrap_setup(setup, command, args)` signatures are identical in Task 2 (definition), Task 4 (call sites), and the interface blocks. `ScriptsConfig.setup: Option<String>` consumed via `.as_deref()` at both call sites. `ensure_excluded` made `pub(crate)` in Task 3 to allow the same-crate test.
