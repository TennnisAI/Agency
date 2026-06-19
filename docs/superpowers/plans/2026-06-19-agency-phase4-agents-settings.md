# Agency Phase 4 Implementation Plan — Real Agents, Providers & Settings

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn Agency's hardcoded "shell" task runner into a configurable agent system: persisted agent profiles, provider settings (Anthropic API key, LM Studio base URL) injected into every agent's environment, a settings UI to manage both, and a profile picker when starting a task. Seed with a `shell` profile and a `claude` (Claude Code) profile.

**Architecture:** Persistence lives in the existing SQLite store (the `Registry`, extended with `profiles` and `settings` tables). `AppState` loads/seeds profiles on startup, exposes profile/settings CRUD, and — when spawning a task — merges provider settings into the agent's environment. Thin Tauri commands expose CRUD; the frontend adds a Settings view and a profile dropdown.

**Tech Stack:** Rust (rusqlite, serde_json), Tauri v2, React + TypeScript.

## Global Constraints

- **Local-only / zero first-party data collection:** Agency's own code makes no network calls. Provider settings (API key, base URL) are only ever passed as environment variables to agent processes the user starts; the key is never sent anywhere by Agency itself.
- **Generic profiles:** an agent profile is `{ name, command, args, env }` (the existing `agency_core::profile::AgentProfile`). `{{prompt}}` in args is replaced at spawn time. No agent-specific code paths.
- **Provider env mapping (fixed):** when spawning any task, merge these into the profile's env, after the profile's own env (so profile env wins on conflict is NOT desired — see note): set `ANTHROPIC_API_KEY` (only if the stored key is non-empty), `OPENAI_BASE_URL` = stored LM Studio URL, `OPENAI_API_KEY` = `"lm-studio"` (placeholder many local servers accept). The profile's own `env` entries take precedence (apply provider env first, then the profile's env on top).
- **Seed profiles** (only when the profiles table is empty): `shell` (command `$SHELL` or `/bin/zsh`, args `["-l"]`) and `claude` (command `"claude"`, args `["{{prompt}}"]`, env `[]`).
- **Settings keys:** `anthropic_api_key`, `lm_studio_base_url` (default `"http://localhost:1234/v1"`).
- **Frontend↔Rust naming:** `agency-app` DTOs use `#[serde(rename_all = "camelCase")]`; `AgentProfile` (from agency-core) serializes with its plain field names (`name`, `command`, `args`, `env`) — the TS interface matches those.
- Do not break existing behavior: the default profile a task runs is still `shell` unless the user picks another.
- TDD for Rust; commit after each green task.

---

## File Structure

```
crates/agency-core/
├── src/registry.rs   # MODIFY: profiles + settings tables; profile/settings CRUD
└── tests/registry.rs # MODIFY: tests for profile + settings persistence

crates/agency-app/
├── src/state.rs      # MODIFY: load/seed profiles from registry; provider settings; env merge on spawn; CRUD methods
├── src/commands.rs   # MODIFY: profile/settings commands + ProviderSettings DTO
├── src/lib.rs        # MODIFY: register new commands
└── tests/state.rs    # MODIFY: tests for seeding, persistence, env injection

ui/src/
├── api.ts                  # MODIFY: AgentProfile/ProviderSettings types + wrappers
├── components/Settings.tsx # NEW: provider + profile management UI
├── components/TaskBoard.tsx# MODIFY: profile picker; pass profile to TerminalPane
├── components/TerminalPane.tsx # MODIFY: accept a `profile` prop (instead of hardcoded "shell")
├── App.tsx                 # MODIFY: a Settings toggle
└── styles.css              # MODIFY: settings + picker styles
```

---

### Task 1: Registry — profile & settings persistence

**Files:**
- Modify: `crates/agency-core/src/registry.rs`
- Test: `crates/agency-core/tests/registry.rs`

**Interfaces:**
- Consumes: `agency_core::profile::AgentProfile`.
- Produces on `Registry` (tables created in `open()` via `CREATE TABLE IF NOT EXISTS`):
  - `fn upsert_profile(&self, p: &AgentProfile) -> anyhow::Result<()>` — keyed by `name`.
  - `fn get_profile(&self, name: &str) -> anyhow::Result<Option<AgentProfile>>`
  - `fn list_profiles(&self) -> anyhow::Result<Vec<AgentProfile>>` (ordered by name)
  - `fn delete_profile(&self, name: &str) -> anyhow::Result<()>`
  - `fn get_setting(&self, key: &str) -> anyhow::Result<Option<String>>`
  - `fn set_setting(&self, key: &str, value: &str) -> anyhow::Result<()>` (upsert)
  - `args` and `env` are stored as JSON text (via `serde_json`).

- [ ] **Step 1: Write the failing test**

Append to `crates/agency-core/tests/registry.rs`:

```rust
use agency_core::profile::AgentProfile;

#[test]
fn profiles_persist_and_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agency.db");
    let p = AgentProfile {
        name: "claude".into(),
        command: "claude".into(),
        args: vec!["{{prompt}}".into()],
        env: vec![("FOO".into(), "bar".into())],
    };
    {
        let reg = Registry::open(&db).unwrap();
        reg.upsert_profile(&p).unwrap();
    }
    let reg = Registry::open(&db).unwrap();
    assert_eq!(reg.get_profile("claude").unwrap().unwrap(), p);
    assert_eq!(reg.list_profiles().unwrap(), vec![p.clone()]);

    // upsert replaces by name
    let p2 = AgentProfile { command: "claude2".into(), ..p.clone() };
    reg.upsert_profile(&p2).unwrap();
    assert_eq!(reg.get_profile("claude").unwrap().unwrap().command, "claude2");
    assert_eq!(reg.list_profiles().unwrap().len(), 1);

    reg.delete_profile("claude").unwrap();
    assert!(reg.get_profile("claude").unwrap().is_none());
}

#[test]
fn settings_persist_and_upsert() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agency.db");
    let reg = Registry::open(&db).unwrap();
    assert!(reg.get_setting("anthropic_api_key").unwrap().is_none());
    reg.set_setting("anthropic_api_key", "sk-test").unwrap();
    reg.set_setting("anthropic_api_key", "sk-updated").unwrap();
    assert_eq!(reg.get_setting("anthropic_api_key").unwrap().unwrap(), "sk-updated");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core --test registry`
Expected: FAIL — `upsert_profile`/`get_setting` not found.

- [ ] **Step 3: Implement the persistence**

In `crates/agency-core/src/registry.rs`, extend the `CREATE TABLE` batch in `open()` (add to the existing `execute_batch` string):

```rust
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                repo_path TEXT NOT NULL,
                default_agent TEXT,
                default_provider TEXT
            );
            CREATE TABLE IF NOT EXISTS profiles (
                name TEXT PRIMARY KEY,
                command TEXT NOT NULL,
                args TEXT NOT NULL,
                env TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;
```

Add `use agency_core...`? No — `AgentProfile` is in the same crate. Add at the top of registry.rs:

```rust
use crate::profile::AgentProfile;
```

Add these methods to `impl Registry`:

```rust
    pub fn upsert_profile(&self, p: &AgentProfile) -> Result<()> {
        let args = serde_json::to_string(&p.args)?;
        let env = serde_json::to_string(&p.env)?;
        self.conn.execute(
            "INSERT INTO profiles (name, command, args, env) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET command = ?2, args = ?3, env = ?4",
            rusqlite::params![p.name, p.command, args, env],
        )?;
        Ok(())
    }

    pub fn get_profile(&self, name: &str) -> Result<Option<AgentProfile>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, command, args, env FROM profiles WHERE name = ?1")?;
        let mut rows = stmt.query([name])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_profile(row)?)),
            None => Ok(None),
        }
    }

    pub fn list_profiles(&self) -> Result<Vec<AgentProfile>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, command, args, env FROM profiles ORDER BY name")?;
        let rows = stmt.query_map([], |row| Ok(row_to_profile(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    pub fn delete_profile(&self, name: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM profiles WHERE name = ?1", [name])?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query([key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }
```

Add this free function near `row_to_project`:

```rust
fn row_to_profile(row: &rusqlite::Row) -> Result<AgentProfile> {
    let args: String = row.get(2)?;
    let env: String = row.get(3)?;
    Ok(AgentProfile {
        name: row.get(0)?,
        command: row.get(1)?,
        args: serde_json::from_str(&args)?,
        env: serde_json::from_str(&env)?,
    })
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-core --test registry`
Expected: PASS (existing + 2 new), no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/registry.rs crates/agency-core/tests/registry.rs
git commit -m "Persist agent profiles and settings in the registry"
```

---

### Task 2: AppState — seed/load profiles, provider settings, env injection

**Files:**
- Modify: `crates/agency-app/src/state.rs`
- Test: `crates/agency-app/tests/state.rs`

**Interfaces:**
- Produces on `AppState`:
  - `new()` now: opens the registry, and if `list_profiles()` is empty, seeds `shell` + `claude` (persisting them). Profiles are no longer held in a separate `Mutex<Vec<AgentProfile>>` — the registry is the source of truth. (Remove the `profiles` field; `register_profile`, `profile_names`, and profile lookups go through the registry.)
  - `register_profile(&self, p: AgentProfile) -> anyhow::Result<()>` — upserts via registry (signature gains a `Result`).
  - `profile_names(&self) -> anyhow::Result<Vec<String>>`
  - `list_profiles(&self) -> anyhow::Result<Vec<AgentProfile>>`
  - `delete_profile(&self, name: &str) -> anyhow::Result<()>`
  - `struct ProviderSettings { pub anthropic_api_key: String, pub lm_studio_base_url: String }` (Debug, Clone, PartialEq, serde Serialize/Deserialize, `#[serde(rename_all = "camelCase")]`).
  - `get_settings(&self) -> anyhow::Result<ProviderSettings>` (reads keys, applies the LM Studio default).
  - `save_settings(&self, s: &ProviderSettings) -> anyhow::Result<()>`
  - `start_task` now: looks up the profile from the registry; builds the effective env = provider env (from settings) followed by the profile's own env; spawns with that.

- [ ] **Step 1: Update existing tests + add new ones**

The existing tests call `AppState::new(...)` then `register_profile(AgentProfile{..})` (now returns `Result`, so add `.unwrap()`), and the seeding test asserted `profile_names()` contains `"shell"` (now returns `Result`). Update those call sites, then append:

```rust
#[test]
fn new_seeds_shell_and_claude_when_empty() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    let names = state.profile_names().unwrap();
    assert!(names.contains(&"shell".to_string()));
    assert!(names.contains(&"claude".to_string()));
}

#[test]
fn settings_default_and_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    let s = state.get_settings().unwrap();
    assert_eq!(s.lm_studio_base_url, "http://localhost:1234/v1");
    assert_eq!(s.anthropic_api_key, "");

    state
        .save_settings(&agency_app_lib::ProviderSettings {
            anthropic_api_key: "sk-x".into(),
            lm_studio_base_url: "http://localhost:9999/v1".into(),
        })
        .unwrap();
    let s2 = state.get_settings().unwrap();
    assert_eq!(s2.anthropic_api_key, "sk-x");
    assert_eq!(s2.lm_studio_base_url, "http://localhost:9999/v1");
}

#[test]
fn start_task_injects_provider_env() {
    // Use a fake agent that echoes an env var so we can prove injection.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    state.save_settings(&agency_app_lib::ProviderSettings {
        anthropic_api_key: "sk-secret".into(),
        lm_studio_base_url: "http://localhost:1234/v1".into(),
    }).unwrap();
    // Profile prints $ANTHROPIC_API_KEY via a shell command.
    state.register_profile(AgentProfile {
        name: "envcheck".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "echo KEY=$ANTHROPIC_API_KEY; echo BASE=$OPENAI_BASE_URL".into()],
        env: vec![],
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();

    let buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let b = buf.clone();
    let info = state.start_task(&project.id, "p", "envcheck", "HEAD", move |bytes| {
        b.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes));
    }).unwrap();

    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        if buf.lock().unwrap().contains("KEY=sk-secret") { break; }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = buf.lock().unwrap().clone();
    assert!(out.contains("KEY=sk-secret"), "got: {out}");
    assert!(out.contains("BASE=http://localhost:1234/v1"), "got: {out}");
    let _ = info;
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-app --test state`
Expected: FAIL — `get_settings`/`ProviderSettings`/seeding not present; existing calls may not compile yet.

- [ ] **Step 3: Implement**

In `crates/agency-app/src/state.rs`:

Remove the `profiles: Mutex<Vec<AgentProfile>>` field and its imports if now unused. Add the settings struct and constants:

```rust
const SETTING_ANTHROPIC_KEY: &str = "anthropic_api_key";
const SETTING_LM_STUDIO_URL: &str = "lm_studio_base_url";
const DEFAULT_LM_STUDIO_URL: &str = "http://localhost:1234/v1";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettings {
    pub anthropic_api_key: String,
    pub lm_studio_base_url: String,
}
```

Rewrite `new()` to seed via the registry:

```rust
    pub fn new(db_path: &Path) -> Result<AppState> {
        let registry = Registry::open(db_path)?;
        if registry.list_profiles()?.is_empty() {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
            registry.upsert_profile(&AgentProfile {
                name: "shell".to_string(),
                command: shell,
                args: vec!["-l".to_string()],
                env: vec![],
            })?;
            registry.upsert_profile(&AgentProfile {
                name: "claude".to_string(),
                command: "claude".to_string(),
                args: vec!["{{prompt}}".to_string()],
                env: vec![],
            })?;
        }
        Ok(AppState {
            registry: Mutex::new(registry),
            sessions: Mutex::new(HashMap::new()),
        })
    }
```

Update the profile methods to delegate to the registry:

```rust
    pub fn register_profile(&self, profile: AgentProfile) -> Result<()> {
        self.registry.lock().unwrap().upsert_profile(&profile)
    }

    pub fn profile_names(&self) -> Result<Vec<String>> {
        Ok(self
            .registry
            .lock()
            .unwrap()
            .list_profiles()?
            .into_iter()
            .map(|p| p.name)
            .collect())
    }

    pub fn list_profiles(&self) -> Result<Vec<AgentProfile>> {
        self.registry.lock().unwrap().list_profiles()
    }

    pub fn delete_profile(&self, name: &str) -> Result<()> {
        self.registry.lock().unwrap().delete_profile(name)
    }

    pub fn get_settings(&self) -> Result<ProviderSettings> {
        let reg = self.registry.lock().unwrap();
        Ok(ProviderSettings {
            anthropic_api_key: reg.get_setting(SETTING_ANTHROPIC_KEY)?.unwrap_or_default(),
            lm_studio_base_url: reg
                .get_setting(SETTING_LM_STUDIO_URL)?
                .unwrap_or_else(|| DEFAULT_LM_STUDIO_URL.to_string()),
        })
    }

    pub fn save_settings(&self, s: &ProviderSettings) -> Result<()> {
        let reg = self.registry.lock().unwrap();
        reg.set_setting(SETTING_ANTHROPIC_KEY, &s.anthropic_api_key)?;
        reg.set_setting(SETTING_LM_STUDIO_URL, &s.lm_studio_base_url)?;
        Ok(())
    }
```

Update `start_task` to look up the profile from the registry and merge provider env. Replace the profile-resolution block and the env handling:

```rust
        // Resolve project repo path.
        let repo_path = {
            let reg = self.registry.lock().unwrap();
            reg.get_project(project_id)?
                .ok_or_else(|| anyhow!("unknown project: {project_id}"))?
                .repo_path
        };

        // Resolve the profile and provider settings; build the effective env.
        let (mut profile, settings) = {
            let reg = self.registry.lock().unwrap();
            let profile = reg
                .get_profile(profile_name)?
                .ok_or_else(|| anyhow!("unknown profile: {profile_name}"))?;
            let settings = ProviderSettings {
                anthropic_api_key: reg.get_setting(SETTING_ANTHROPIC_KEY)?.unwrap_or_default(),
                lm_studio_base_url: reg
                    .get_setting(SETTING_LM_STUDIO_URL)?
                    .unwrap_or_else(|| DEFAULT_LM_STUDIO_URL.to_string()),
            };
            (profile, settings)
        };

        // Provider env first, then the profile's own env on top.
        let mut env: Vec<(String, String)> = Vec::new();
        if !settings.anthropic_api_key.is_empty() {
            env.push(("ANTHROPIC_API_KEY".into(), settings.anthropic_api_key.clone()));
        }
        env.push(("OPENAI_BASE_URL".into(), settings.lm_studio_base_url.clone()));
        env.push(("OPENAI_API_KEY".into(), "lm-studio".into()));
        env.extend(profile.env.iter().cloned());
        profile.env = env;
```

Then the existing worktree-create + spawn lines follow, using the modified `profile`. (Keep `let task_id = new_task_id();` etc.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-app --test state`
Expected: PASS (existing updated + new), no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app/src/state.rs crates/agency-app/tests/state.rs
git commit -m "Seed/persist profiles, provider settings, and inject provider env"
```

---

### Task 3: Tauri commands for profiles & settings

**Files:**
- Modify: `crates/agency-app/src/commands.rs`, `crates/agency-app/src/lib.rs`

**Interfaces:**
- Produces (thin; map anyhow→String): `list_profiles() -> Vec<AgentProfile>`, `save_profile(profile: AgentProfile)`, `delete_profile(name: String)`, `get_settings() -> ProviderSettings`, `save_settings(settings: ProviderSettings)`. (`AgentProfile` and `ProviderSettings` derive Serialize/Deserialize already.)
- Registers all five in `generate_handler!`.
- `lib.rs` re-exports `ProviderSettings` (so tests can `agency_app_lib::ProviderSettings`).

- [ ] **Step 1: Add the command wrappers**

In `crates/agency-app/src/commands.rs`, add (importing `AgentProfile` and `ProviderSettings`):

```rust
use agency_core::profile::AgentProfile;
use crate::state::ProviderSettings;

#[tauri::command]
pub fn list_profiles(state: State<'_, AppState>) -> Result<Vec<AgentProfile>, String> {
    state.list_profiles().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_profile(state: State<'_, AppState>, profile: AgentProfile) -> Result<(), String> {
    state.register_profile(profile).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_profile(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state.delete_profile(&name).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<ProviderSettings, String> {
    state.get_settings().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: ProviderSettings) -> Result<(), String> {
    state.save_settings(&settings).map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Re-export ProviderSettings + register commands**

In `crates/agency-app/src/lib.rs`, change the re-export line to include `ProviderSettings`:

```rust
pub use state::{AppState, ProviderSettings, TaskInfo};
```

And add the five commands to `generate_handler!`:

```rust
            commands::list_profiles,
            commands::save_profile,
            commands::delete_profile,
            commands::get_settings,
            commands::save_settings,
```

- [ ] **Step 3: Build & test**

Run: `cargo build -p agency-app` (warning-free) and `cargo test -p agency-app`.
Expected: clean; all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs
git commit -m "Add Tauri commands for profile and settings management"
```

---

### Task 4: Frontend — Settings view + API

**Files:**
- Modify: `ui/src/api.ts`
- Create: `ui/src/components/Settings.tsx`
- Modify: `ui/src/styles.css`

**Interfaces:**
- Produces in `api.ts`: `AgentProfile { name; command; args: string[]; env: [string, string][] }`, `ProviderSettings { anthropicApiKey; lmStudioBaseUrl }`, and wrappers `listProfiles()`, `saveProfile(profile)`, `deleteProfile(name)`, `getSettings()`, `saveSettings(settings)`.
- `Settings({ onClose })`: edits provider fields (Anthropic key as a password field, LM Studio URL) and manages profiles (list with delete; a form to add/edit a profile: name, command, args as a comma/space list, env as `KEY=VALUE` lines).

- [ ] **Step 1: Add API wrappers**

Append to `ui/src/api.ts`:

```ts
export interface AgentProfile {
  name: string;
  command: string;
  args: string[];
  env: [string, string][];
}

export interface ProviderSettings {
  anthropicApiKey: string;
  lmStudioBaseUrl: string;
}

export const listProfiles = () => invoke<AgentProfile[]>("list_profiles");
export const saveProfile = (profile: AgentProfile) =>
  invoke<void>("save_profile", { profile });
export const deleteProfile = (name: string) => invoke<void>("delete_profile", { name });
export const getSettings = () => invoke<ProviderSettings>("get_settings");
export const saveSettings = (settings: ProviderSettings) =>
  invoke<void>("save_settings", { settings });
```

- [ ] **Step 2: Create the Settings component**

Create `ui/src/components/Settings.tsx`:

```tsx
import { useEffect, useState } from "react";
import {
  AgentProfile,
  ProviderSettings,
  deleteProfile,
  getSettings,
  listProfiles,
  saveProfile,
  saveSettings,
} from "../api";

export default function Settings({ onClose }: { onClose: () => void }) {
  const [settings, setSettings] = useState<ProviderSettings>({
    anthropicApiKey: "",
    lmStudioBaseUrl: "",
  });
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [draft, setDraft] = useState({ name: "", command: "", args: "", env: "" });
  const [error, setError] = useState("");

  async function refresh() {
    try {
      setSettings(await getSettings());
      setProfiles(await listProfiles());
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    refresh();
  }, []);

  async function persistSettings() {
    try {
      await saveSettings(settings);
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }

  async function addProfile() {
    if (!draft.name.trim() || !draft.command.trim()) return;
    const args = draft.args.trim() ? draft.args.trim().split(/\s+/) : [];
    const env: [string, string][] = draft.env
      .split("\n")
      .map((l) => l.trim())
      .filter(Boolean)
      .map((l) => {
        const i = l.indexOf("=");
        return [l.slice(0, i), l.slice(i + 1)] as [string, string];
      })
      .filter(([k]) => k);
    try {
      await saveProfile({ name: draft.name.trim(), command: draft.command.trim(), args, env });
      setDraft({ name: "", command: "", args: "", env: "" });
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  function editProfile(p: AgentProfile) {
    setDraft({
      name: p.name,
      command: p.command,
      args: p.args.join(" "),
      env: p.env.map(([k, v]) => `${k}=${v}`).join("\n"),
    });
  }

  return (
    <div className="settings-overlay">
      <div className="settings">
        <div className="settings-head">
          <h2>Settings</h2>
          <button onClick={onClose}>Close</button>
        </div>
        {error && <div className="git-error">{error}</div>}

        <section>
          <h3>Providers</h3>
          <label>Anthropic API key</label>
          <input
            type="password"
            value={settings.anthropicApiKey}
            onChange={(e) => setSettings({ ...settings, anthropicApiKey: e.target.value })}
          />
          <label>LM Studio base URL</label>
          <input
            value={settings.lmStudioBaseUrl}
            onChange={(e) => setSettings({ ...settings, lmStudioBaseUrl: e.target.value })}
          />
          <button onClick={persistSettings}>Save providers</button>
        </section>

        <section>
          <h3>Agent profiles</h3>
          <ul className="profile-list">
            {profiles.map((p) => (
              <li key={p.name}>
                <span className="profile-name" onClick={() => editProfile(p)}>
                  {p.name}
                </span>
                <code>{p.command}</code>
                <button onClick={() => deleteProfile(p.name).then(refresh)}>Delete</button>
              </li>
            ))}
          </ul>
          <div className="profile-form">
            <input
              placeholder="name"
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            />
            <input
              placeholder="command (e.g. claude)"
              value={draft.command}
              onChange={(e) => setDraft({ ...draft, command: e.target.value })}
            />
            <input
              placeholder="args (space-separated, use {{prompt}})"
              value={draft.args}
              onChange={(e) => setDraft({ ...draft, args: e.target.value })}
            />
            <textarea
              placeholder="env, one KEY=VALUE per line"
              value={draft.env}
              onChange={(e) => setDraft({ ...draft, env: e.target.value })}
            />
            <button onClick={addProfile}>Save profile</button>
          </div>
        </section>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Add styles**

Append to `ui/src/styles.css`:

```css
.settings-overlay { position: fixed; inset: 0; background: rgba(0,0,0,0.5); display: flex; align-items: center; justify-content: center; z-index: 10; }
.settings { background: #14161a; border: 1px solid #2a2e36; border-radius: 10px; padding: 16px; width: 560px; max-height: 86vh; overflow: auto; display: flex; flex-direction: column; gap: 14px; }
.settings-head { display: flex; justify-content: space-between; align-items: center; }
.settings section { display: flex; flex-direction: column; gap: 6px; }
.settings h3 { margin: 0; font-size: 13px; text-transform: uppercase; letter-spacing: 0.06em; color: #9aa3b2; }
.settings label { font-size: 12px; color: #8893a5; }
.settings input, .settings textarea { background: #1b1f27; border: 1px solid #2a2e36; color: #e6e6e6; border-radius: 6px; padding: 8px; }
.settings textarea { min-height: 64px; resize: vertical; }
.profile-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.profile-list li { display: flex; align-items: center; gap: 8px; }
.profile-name { cursor: pointer; min-width: 90px; }
.profile-name:hover { text-decoration: underline; }
.profile-list code { color: #8893a5; flex: 1; font-size: 12px; }
.profile-form { display: flex; flex-direction: column; gap: 6px; margin-top: 6px; }
```

- [ ] **Step 4: Build**

Run: `pnpm --dir ui build`
Expected: clean (Settings not mounted yet; verifies types). `pnpm --dir ui test` still green.

- [ ] **Step 5: Commit**

```bash
git add ui/src/api.ts ui/src/components/Settings.tsx ui/src/styles.css
git commit -m "Add settings view and profile/provider API"
```

---

### Task 5: Frontend — profile picker + settings toggle

**Files:**
- Modify: `ui/src/components/TerminalPane.tsx` (accept a `profile` prop)
- Modify: `ui/src/components/TaskBoard.tsx` (profile dropdown)
- Modify: `ui/src/App.tsx` (open Settings)

**Interfaces:**
- `TerminalPane` gains a required `profile: string` prop, passed to `startTask` instead of the hardcoded `"shell"`.
- `TaskBoard` loads profiles via `listProfiles()`, shows a `<select>` to pick one (default `shell` if present, else the first), and passes it to `TerminalPane`.
- `App` shows a "Settings" button (in the sidebar area) that opens the `Settings` overlay.

- [ ] **Step 1: TerminalPane takes a profile prop**

In `ui/src/components/TerminalPane.tsx`, add `profile: string` to `Props`, destructure it, and change the `startTask` call from `"shell"` to `profile`:

```tsx
interface Props {
  projectId: string;
  prompt: string;
  profile: string;
  onStatus: (label: string) => void;
  onStarted?: (taskId: string) => void;
}
```

```tsx
    startTask(projectId, prompt, profile, (bytes) => term.write(bytes)).then((i) => {
```

Add `profile` to the effect dependency array.

- [ ] **Step 2: TaskBoard profile dropdown**

Update `ui/src/components/TaskBoard.tsx`: load profiles, add a `profile` state, render a select in the new-task form, and pass `profile` to `TerminalPane`.

```tsx
import { useEffect, useState } from "react";
import { AgentProfile, listProfiles, Project } from "../api";
import TerminalPane from "./TerminalPane";
import GitPanel from "./GitPanel";

interface Props {
  project: Project;
}

export default function TaskBoard({ project }: Props) {
  const [prompt, setPrompt] = useState("");
  const [activePrompt, setActivePrompt] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const [taskId, setTaskId] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [profile, setProfile] = useState("shell");
  const [activeProfile, setActiveProfile] = useState("shell");

  useEffect(() => {
    listProfiles().then((ps) => {
      setProfiles(ps);
      if (!ps.find((p) => p.name === "shell") && ps[0]) setProfile(ps[0].name);
    });
  }, []);

  function closeTask() {
    setActivePrompt(null);
    setTaskId(null);
    setStatus("");
  }

  function startTask() {
    setActiveProfile(profile);
    setActivePrompt(prompt);
  }

  return (
    <main className="board">
      <header className="board-header">
        <h2>{project.name}</h2>
        <code>{project.repo_path}</code>
      </header>
      {activePrompt === null ? (
        <div className="new-task">
          <select value={profile} onChange={(e) => setProfile(e.target.value)}>
            {profiles.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}
              </option>
            ))}
          </select>
          <textarea
            placeholder="Task prompt"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
          />
          <button onClick={startTask}>Start task</button>
        </div>
      ) : (
        <div className="task-running">
          <div className="task-status">
            status: {status || "starting…"} · agent: {activeProfile}
          </div>
          <div className="task-split">
            <TerminalPane
              key={`${project.id}:${activeProfile}:${activePrompt}`}
              projectId={project.id}
              prompt={activePrompt}
              profile={activeProfile}
              onStatus={setStatus}
              onStarted={setTaskId}
            />
            {taskId && <GitPanel taskId={taskId} />}
          </div>
          <button onClick={closeTask}>Close terminal</button>
        </div>
      )}
    </main>
  );
}
```

- [ ] **Step 3: App opens Settings**

Update `ui/src/App.tsx`:

```tsx
import { useState } from "react";
import ProjectSidebar from "./components/ProjectSidebar";
import TaskBoard from "./components/TaskBoard";
import Settings from "./components/Settings";
import { Project } from "./api";

export default function App() {
  const [project, setProject] = useState<Project | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  return (
    <div className="app">
      <div className="sidebar-wrap">
        <ProjectSidebar selectedId={project?.id ?? null} onSelect={setProject} />
        <button className="settings-btn" onClick={() => setShowSettings(true)}>
          Settings
        </button>
      </div>
      {project ? (
        <TaskBoard project={project} />
      ) : (
        <main className="board empty">Select or add a project to begin.</main>
      )}
      {showSettings && <Settings onClose={() => setShowSettings(false)} />}
    </div>
  );
}
```

Append to `ui/src/styles.css`:

```css
.sidebar-wrap { display: flex; flex-direction: column; }
.settings-btn { margin: 8px; background: #2a2e36; }
.new-task select { background: #1b1f27; border: 1px solid #2a2e36; color: #e6e6e6; border-radius: 6px; padding: 8px; max-width: 200px; }
```

- [ ] **Step 4: Build + test**

Run: `pnpm --dir ui build` and `pnpm --dir ui test`.
Expected: clean build, vitest green.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/TerminalPane.tsx ui/src/components/TaskBoard.tsx ui/src/App.tsx ui/src/styles.css
git commit -m "Add agent profile picker and settings toggle"
```

---

## Self-Review

**Spec coverage (Phase 4 scope):**
- Persisted profiles + settings → Task 1. ✓
- Seed shell + claude; provider env injection → Task 2 (env-injection test proves `ANTHROPIC_API_KEY`/`OPENAI_BASE_URL` reach the agent). ✓
- Profile/settings Tauri commands → Task 3. ✓
- Settings UI (providers + profile management) → Task 4. ✓
- Profile picker when starting a task → Task 5. ✓
- Deferred: per-profile provider selection (all agents get the same provider env — fine since each agent reads only what it needs); editing a profile is "load into the add form and re-save" (upsert by name), not a separate edit modal.

**Placeholder scan:** No TBD/TODO; every code step is complete.

**Type consistency:** `ProviderSettings` defined in `state.rs`, re-exported from `lib.rs`, used by commands and tests; `AgentProfile` flows from `agency_core::profile` through commands to the TS `AgentProfile` (plain field names). `register_profile`/`profile_names` now return `Result` — Task 2 updates the existing call sites that used them. `TerminalPane.profile` (Task 5) matches the `startTask(projectId, prompt, profile, ...)` signature.

**Notes for the executor:**
- The seed `claude` profile assumes the `claude` CLI is installed for real use; tests never spawn it (they use `/bin/sh` / the fake agent). Absence of `claude` only affects running that profile at runtime, not the build/tests.
- `AppState` loses its `profiles` field this phase; ensure no remaining references and no unused imports (warning-free).
