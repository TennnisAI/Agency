# Agency Phase 2 Implementation Plan — Tauri Shell + Live Terminal

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a runnable Agency desktop app: a Tauri v2 window with a project sidebar, a task board, and a live xterm.js terminal that spawns a process in a per-task git worktree (via the Phase 1 `agency-core` engine) and streams it bidirectionally to the UI.

**Architecture:** A new workspace member `agency-app` (Tauri v2, Rust) holds all app state in a Tauri-free, unit-tested `AppState` (wraps the Phase 1 `Registry`, `WorktreeManager`, `AgentProfile`, and PTY `spawn_agent`). Thin `#[tauri::command]` wrappers adapt `AppState` to the frontend and stream PTY output over a Tauri `Channel`. The frontend is a Vite + React + TypeScript app in `ui/`, using `@xterm/xterm` for the terminal.

**Tech Stack:** Tauri v2, Rust, `base64`; Vite, React, TypeScript, `@tauri-apps/api`, `@xterm/xterm` + `@xterm/addon-fit`; Vitest for frontend unit tests.

## Global Constraints

- **Local-only / zero first-party data collection:** `agency-app`'s own code makes no network calls. Only user-initiated agent/provider/git-remote calls are allowed (none in Phase 2 — the default profile is a local shell).
- **Do not modify the `agency-core` crate.** Phase 2 consumes it as-is. If a change to `agency-core` seems required, STOP and report it — it is a plan decision, not an implementer choice.
- **Testable logic lives in `AppState` (Tauri-free).** `#[tauri::command]` functions must be thin adapters with no business logic, so the logic is unit-testable without a webview.
- **Default agent profile** in Phase 2 is an interactive login shell: command = `$SHELL` (fallback `/bin/zsh`), args = `["-l"]`. Real agent profiles (Claude Code/Pi/Hermes) are Phase 5.
- **Worktree/branch conventions** are owned by `agency-core` (`<repo>/.agency/worktrees/<task-id>`, branch `agent/<task-id>`); do not reimplement them.
- **Frontend dev server:** Vite on port **1420**, `strictPort: true`; Tauri `devUrl` = `http://localhost:1420`.
- **Tauri identifier:** `build.agency.app`. **Product name:** `Agency`.
- **Bundling is disabled in Phase 2** (`bundle.active = false`) — we run via `tauri dev`; packaging/icons are Phase 5.
- **Frontend↔Rust naming:** types defined in `agency-app` use `#[serde(rename_all = "camelCase")]`. The reused `agency_core::registry::Project` serializes with its Rust field names (`repo_path`, `default_agent`, `default_provider`) — the TS `Project` interface MUST use those snake_case keys to match. Tauri converts camelCase JS command args to snake_case Rust params automatically.

---

## File Structure

```
crates/agency-app/
├── Cargo.toml                 # Tauri app crate (workspace member)
├── build.rs                   # tauri_build::build()
├── tauri.conf.json            # Tauri v2 config (dev-only, bundle off)
├── capabilities/default.json  # grants core:default to the main window
├── src/
│   ├── main.rs                # thin: calls agency_app_lib::run()
│   ├── lib.rs                 # Tauri Builder, state setup, command registration
│   ├── state.rs               # AppState + Session + TaskInfo (Tauri-free, tested)
│   └── commands.rs            # #[tauri::command] thin wrappers + DTOs
└── tests/
    └── state.rs               # AppState unit/integration tests (fake_agent fixture)

ui/
├── package.json
├── vite.config.ts
├── tsconfig.json
├── tsconfig.node.json
├── index.html
├── vitest.config.ts
└── src/
    ├── main.tsx
    ├── App.tsx
    ├── api.ts                 # typed invoke wrappers + Channel wiring
    ├── b64.ts                 # base64→Uint8Array helper (unit-tested)
    ├── b64.test.ts
    ├── styles.css
    └── components/
        ├── ProjectSidebar.tsx
        ├── TaskBoard.tsx
        └── TerminalPane.tsx
```

Reused unchanged from Phase 1: `crates/agency-core/tests/fixtures/fake_agent.sh` (used by `agency-app` tests).

---

### Task 1: Scaffold the Tauri v2 app crate and the Vite/React frontend

**Files:**
- Create: `crates/agency-app/Cargo.toml`, `crates/agency-app/build.rs`, `crates/agency-app/tauri.conf.json`, `crates/agency-app/capabilities/default.json`, `crates/agency-app/src/main.rs`, `crates/agency-app/src/lib.rs`
- Create: `crates/agency-app/src/state.rs`, `crates/agency-app/src/commands.rs` (minimal stubs this task)
- Create: `ui/package.json`, `ui/vite.config.ts`, `ui/tsconfig.json`, `ui/tsconfig.node.json`, `ui/index.html`, `ui/src/main.tsx`, `ui/src/App.tsx`, `ui/src/styles.css`
- Modify: root `Cargo.toml` (add `crates/agency-app` to workspace members), `.gitignore` (add `ui/node_modules`, `ui/dist`)
- Test: `crates/agency-app/tests/smoke.rs`

**Interfaces:**
- Consumes: `agency-core` (path dependency).
- Produces: a buildable `agency-app` crate exposing `agency_app_lib::run()`, and a buildable frontend. `state.rs` exposes `pub struct AppState;` with `AppState::version() -> &'static str` as a placeholder until Task 2; `commands.rs` is an empty module.

- [ ] **Step 1: Write the failing app-crate smoke test**

Create `crates/agency-app/tests/smoke.rs`:

```rust
#[test]
fn app_lib_exposes_state_version() {
    assert_eq!(agency_app_lib::AppState::version(), "0.1.0");
}
```

- [ ] **Step 2: Run it to verify failure**

Run: `cargo test -p agency-app --test smoke`
Expected: FAIL — package `agency-app` does not exist yet.

- [ ] **Step 3: Add the app crate to the workspace**

Edit root `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/agency-core", "crates/agency-app"]
```

- [ ] **Step 4: Create the Tauri crate manifest and build script**

`crates/agency-app/Cargo.toml`:

```toml
[package]
name = "agency-app"
version = "0.1.0"
edition = "2021"

[lib]
name = "agency_app_lib"
crate-type = ["lib", "cdylib", "staticlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = [] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
base64 = "0.22"
agency-core = { path = "../agency-core" }

[dev-dependencies]
tempfile = "3"
```

`crates/agency-app/build.rs`:

```rust
fn main() {
    tauri_build::build();
}
```

- [ ] **Step 5: Create the Tauri config and capability**

`crates/agency-app/tauri.conf.json`:

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Agency",
  "version": "0.1.0",
  "identifier": "build.agency.app",
  "build": {
    "frontendDist": "../../ui/dist",
    "devUrl": "http://localhost:1420",
    "beforeDevCommand": "pnpm --dir ../../ui dev",
    "beforeBuildCommand": "pnpm --dir ../../ui build"
  },
  "app": {
    "windows": [
      { "title": "Agency", "width": 1280, "height": 832 }
    ],
    "security": { "csp": null }
  },
  "bundle": { "active": false }
}
```

`crates/agency-app/capabilities/default.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Default capability for the main window",
  "windows": ["main"],
  "permissions": ["core:default"]
}
```

- [ ] **Step 6: Create the Rust entrypoints and stubs**

`crates/agency-app/src/main.rs`:

```rust
// Prevents an extra console window on Windows in release; harmless on macOS.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    agency_app_lib::run();
}
```

`crates/agency-app/src/lib.rs`:

```rust
mod commands;
mod state;

pub use state::AppState;

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running Agency");
}
```

`crates/agency-app/src/state.rs`:

```rust
pub struct AppState;

impl AppState {
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
}
```

`crates/agency-app/src/commands.rs`:

```rust
// Tauri command wrappers — implemented in later tasks.
```

- [ ] **Step 7: Create the frontend project**

`ui/package.json`:

```json
{
  "name": "agency-ui",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview",
    "test": "vitest run"
  },
  "dependencies": {
    "@tauri-apps/api": "^2",
    "@xterm/xterm": "^5.5.0",
    "@xterm/addon-fit": "^0.10.0",
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2",
    "@types/react": "^18.3.12",
    "@types/react-dom": "^18.3.1",
    "@vitejs/plugin-react": "^4.3.4",
    "typescript": "^5.6.3",
    "vite": "^6.0.3",
    "vitest": "^2.1.8"
  }
}
```

`ui/vite.config.ts`:

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
});
```

`ui/vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: { environment: "node" },
});
```

`ui/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2020",
    "useDefineForClassFields": true,
    "lib": ["ES2020", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true
  },
  "include": ["src"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
```

`ui/tsconfig.node.json`:

```json
{
  "compilerOptions": {
    "composite": true,
    "skipLibCheck": true,
    "module": "ESNext",
    "moduleResolution": "bundler",
    "allowSyntheticDefaultImports": true
  },
  "include": ["vite.config.ts", "vitest.config.ts"]
}
```

`ui/index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Agency</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

`ui/src/main.tsx`:

```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
```

`ui/src/App.tsx`:

```tsx
export default function App() {
  return <h1>Agency</h1>;
}
```

`ui/src/styles.css`:

```css
:root { color-scheme: dark; }
body { margin: 0; font-family: ui-sans-serif, system-ui, sans-serif; background: #14161a; color: #e6e6e6; }
```

- [ ] **Step 8: Install deps, then verify both halves build and the test passes**

```bash
pnpm --dir ui install
cargo test -p agency-app --test smoke
pnpm --dir ui build
```
Expected: smoke test PASS (1 passed); `pnpm build` produces `ui/dist/` with no TypeScript errors.

- [ ] **Step 9: Update .gitignore**

Append to `.gitignore`:

```
ui/node_modules
ui/dist
crates/agency-app/gen
```

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "Scaffold Tauri v2 app crate and React frontend"
```

---

### Task 2: AppState + project operations (Tauri-free, tested)

**Files:**
- Modify: `crates/agency-app/src/state.rs`
- Test: `crates/agency-app/tests/state.rs`

**Interfaces:**
- Consumes: `agency_core::registry::{Registry, Project}`, `agency_core::profile::AgentProfile`.
- Produces:
  - `struct AppState` holding `registry: std::sync::Mutex<Registry>`, `sessions: std::sync::Mutex<std::collections::HashMap<String, Session>>`, `profiles: std::sync::Mutex<Vec<AgentProfile>>`.
  - `struct Session { pub handle: agency_core::supervisor::AgentHandle, pub worktree: agency_core::worktree::Worktree, pub project_id: String }`
  - `AppState::new(db_path: &std::path::Path) -> anyhow::Result<AppState>` — opens the registry and seeds the default shell profile named `"shell"`.
  - `AppState::register_profile(&self, profile: AgentProfile)` — adds/replaces a profile by name.
  - `AppState::add_project(&self, name: &str, repo_path: &std::path::Path) -> anyhow::Result<Project>`
  - `AppState::list_projects(&self) -> anyhow::Result<Vec<Project>>`
  - `AppState::remove_project(&self, id: &str) -> anyhow::Result<()>`
  - Keep `AppState::version()` (used by Task 1's smoke test).

- [ ] **Step 1: Write the failing tests**

Create `crates/agency-app/tests/state.rs`:

```rust
use agency_app_lib::AppState;
use std::path::Path;

#[test]
fn new_seeds_default_shell_profile_and_version_holds() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    assert_eq!(AppState::version(), "0.1.0");
    assert!(state.profile_names().contains(&"shell".to_string()));
}

#[test]
fn project_crud_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(&dir.path().join("agency.db")).unwrap();

    let p = state.add_project("demo", Path::new("/tmp/demo-repo")).unwrap();
    assert_eq!(p.name, "demo");

    let all = state.list_projects().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, p.id);

    state.remove_project(&p.id).unwrap();
    assert_eq!(state.list_projects().unwrap().len(), 0);
}
```

Note: `profile_names()` is a small test-support accessor; add it to the public API (it is also useful to the settings UI later).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-app --test state`
Expected: FAIL — `AppState::new` / `add_project` / `profile_names` not found.

- [ ] **Step 3: Implement AppState (project operations + profile seeding)**

Replace `crates/agency-app/src/state.rs` with:

```rust
use agency_core::profile::AgentProfile;
use agency_core::registry::{Project, Registry};
use agency_core::supervisor::AgentHandle;
use agency_core::worktree::Worktree;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

pub struct Session {
    pub handle: AgentHandle,
    pub worktree: Worktree,
    pub project_id: String,
}

pub struct AppState {
    registry: Mutex<Registry>,
    sessions: Mutex<HashMap<String, Session>>,
    profiles: Mutex<Vec<AgentProfile>>,
}

impl AppState {
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn new(db_path: &Path) -> Result<AppState> {
        let registry = Registry::open(db_path)?;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let default_profile = AgentProfile {
            name: "shell".to_string(),
            command: shell,
            args: vec!["-l".to_string()],
            env: vec![],
        };
        Ok(AppState {
            registry: Mutex::new(registry),
            sessions: Mutex::new(HashMap::new()),
            profiles: Mutex::new(vec![default_profile]),
        })
    }

    pub fn register_profile(&self, profile: AgentProfile) {
        let mut profiles = self.profiles.lock().unwrap();
        if let Some(slot) = profiles.iter_mut().find(|p| p.name == profile.name) {
            *slot = profile;
        } else {
            profiles.push(profile);
        }
    }

    pub fn profile_names(&self) -> Vec<String> {
        self.profiles
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.name.clone())
            .collect()
    }

    pub fn add_project(&self, name: &str, repo_path: &Path) -> Result<Project> {
        self.registry.lock().unwrap().add_project(name, repo_path)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        self.registry.lock().unwrap().list_projects()
    }

    pub fn remove_project(&self, id: &str) -> Result<()> {
        self.registry.lock().unwrap().remove_project(id)
    }
}
```

Note: `Session`, `sessions`, `Worktree`, and `AgentHandle` imports are unused until Task 3. Add `#[allow(dead_code)]` on the `Session` struct and the `sessions` field for THIS task only, and remove it in Task 3 when they are used. Do not silence other warnings.

Apply the allow:

```rust
#[allow(dead_code)]
pub struct Session {
    pub handle: AgentHandle,
    pub worktree: Worktree,
    pub project_id: String,
}
```

and on the field:

```rust
    #[allow(dead_code)]
    sessions: Mutex<HashMap<String, Session>>,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agency-app --test state`
Expected: PASS (2 passed), no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app/src/state.rs crates/agency-app/tests/state.rs
git commit -m "Add AppState with project operations and default shell profile"
```

---

### Task 3: Session operations — start/input/status/stop (Tauri-free, tested)

**Files:**
- Modify: `crates/agency-app/src/state.rs`
- Test: `crates/agency-app/tests/state.rs`

**Interfaces:**
- Consumes: from Task 2 `AppState`, plus `agency_core::worktree::WorktreeManager`, `agency_core::supervisor::{spawn_agent, AgentStatus, AgentHandle}`, `agency_core::profile::AgentProfile`, `uuid` via `agency-core`'s dependency is NOT available here — generate ids with a small helper (see below).
- Produces:
  - `struct TaskInfo { pub task_id: String, pub branch: String }` (derives `Debug, Clone, PartialEq`).
  - `AppState::start_task<F>(&self, project_id: &str, prompt: &str, profile_name: &str, base: &str, on_output: F) -> anyhow::Result<TaskInfo>` where `F: Fn(Vec<u8>) + Send + 'static`. Looks up the project's repo path, looks up the profile by name, creates a worktree with a fresh task id, spawns the agent in the worktree with `on_output`, stores the `Session`, returns `TaskInfo`.
  - `AppState::send_input(&self, task_id: &str, data: &[u8]) -> anyhow::Result<()>`
  - `AppState::task_status(&self, task_id: &str) -> anyhow::Result<AgentStatus>`
  - `AppState::stop_task(&self, task_id: &str) -> anyhow::Result<()>` — drops the session (closing the PTY) and removes the worktree.

- [ ] **Step 1: Write the failing test**

Append to `crates/agency-app/tests/state.rs`:

```rust
use agency_app_lib::TaskInfo;
use agency_core::profile::AgentProfile;
use agency_core::supervisor::AgentStatus;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn init_repo(dir: &Path) {
    let run = |args: &[&str]| {
        assert!(
            Command::new("git").args(args).current_dir(dir).status().unwrap().success(),
            "git {:?}",
            args
        );
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "t@e.com"]);
    run(&["config", "user.name", "T"]);
    std::fs::write(dir.join("README.md"), "hi").unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "init"]);
}

fn fake_agent_command() -> String {
    // Reuse the Phase 1 fixture from agency-core.
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("agency-core")
        .join("tests")
        .join("fixtures")
        .join("fake_agent.sh");
    p.to_string_lossy().to_string()
}

fn wait_for(buf: &Arc<Mutex<String>>, needle: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if buf.lock().unwrap().contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn start_task_spawns_in_worktree_streams_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let state = AppState::new(&dir.path().join("agency.db")).unwrap();
    state.register_profile(AgentProfile {
        name: "fake".into(),
        command: fake_agent_command(),
        args: vec!["{{prompt}}".into()],
        env: vec![],
    });

    let project = state.add_project("demo", &repo).unwrap();

    let buf = Arc::new(Mutex::new(String::new()));
    let buf_cb = buf.clone();
    let info: TaskInfo = state
        .start_task(&project.id, "do-the-thing", "fake", "HEAD", move |bytes| {
            buf_cb.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes));
        })
        .unwrap();

    assert_eq!(info.branch, format!("agent/{}", info.task_id));

    // Worktree exists on disk.
    let wt_path = repo.join(".agency").join("worktrees").join(&info.task_id);
    assert!(wt_path.exists());

    // Streaming + the rendered prompt arg.
    assert!(wait_for(&buf, "AGENT_READY", Duration::from_secs(5)));
    assert!(wait_for(&buf, "PROMPT:do-the-thing", Duration::from_secs(5)));

    // Input injection.
    state.send_input(&info.task_id, b"ping\n").unwrap();
    assert!(wait_for(&buf, "GOT:ping", Duration::from_secs(5)));

    // Status reaches Exited(0).
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if matches!(state.task_status(&info.task_id).unwrap(), AgentStatus::Exited(_)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(state.task_status(&info.task_id).unwrap(), AgentStatus::Exited(0));

    // Stop removes the worktree.
    state.stop_task(&info.task_id).unwrap();
    assert!(!wt_path.exists());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-app --test state`
Expected: FAIL — `start_task` / `TaskInfo` / `send_input` not found.

- [ ] **Step 3: Implement the session operations**

Add a small id helper and the methods. First, add these imports at the top of `state.rs` (merge with existing imports):

```rust
use agency_core::supervisor::{spawn_agent, AgentStatus};
use agency_core::worktree::WorktreeManager;
use anyhow::{anyhow, Result};
```

Remove the `#[allow(dead_code)]` attributes added in Task 2 (the fields are now used).

Add the `TaskInfo` type:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct TaskInfo {
    pub task_id: String,
    pub branch: String,
}
```

Add a private id generator (avoids adding a new dependency — derives a 32-hex-char id from the system clock and a counter):

```rust
fn new_task_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:032x}", nanos ^ ((n as u128) << 96))
}
```

Add the methods to `impl AppState`:

```rust
    pub fn start_task<F>(
        &self,
        project_id: &str,
        prompt: &str,
        profile_name: &str,
        base: &str,
        on_output: F,
    ) -> Result<TaskInfo>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        // Resolve project repo path (lock released before spawning).
        let repo_path = {
            let reg = self.registry.lock().unwrap();
            reg.get_project(project_id)?
                .ok_or_else(|| anyhow!("unknown project: {project_id}"))?
                .repo_path
        };

        // Resolve the profile (clone so we don't hold the lock during spawn).
        let profile = {
            let profiles = self.profiles.lock().unwrap();
            profiles
                .iter()
                .find(|p| p.name == profile_name)
                .cloned()
                .ok_or_else(|| anyhow!("unknown profile: {profile_name}"))?
        };

        let task_id = new_task_id();
        let manager = WorktreeManager::new(repo_path);
        let worktree = manager.create(&task_id, base)?;

        let handle = spawn_agent(&profile, &worktree.path, prompt, on_output)?;
        let branch = worktree.branch.clone();

        self.sessions.lock().unwrap().insert(
            task_id.clone(),
            Session {
                handle,
                worktree,
                project_id: project_id.to_string(),
            },
        );

        Ok(TaskInfo { task_id, branch })
    }

    pub fn send_input(&self, task_id: &str, data: &[u8]) -> Result<()> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(task_id)
            .ok_or_else(|| anyhow!("unknown task: {task_id}"))?;
        session.handle.write_input(data)
    }

    pub fn task_status(&self, task_id: &str) -> Result<AgentStatus> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(task_id)
            .ok_or_else(|| anyhow!("unknown task: {task_id}"))?;
        Ok(session.handle.status())
    }

    pub fn stop_task(&self, task_id: &str) -> Result<()> {
        // Remove (and drop) the session first so the PTY/handle is released.
        let session = self
            .sessions
            .lock()
            .unwrap()
            .remove(task_id)
            .ok_or_else(|| anyhow!("unknown task: {task_id}"))?;
        let repo_path = {
            let reg = self.registry.lock().unwrap();
            reg.get_project(&session.project_id)?
                .map(|p| p.repo_path)
        };
        drop(session); // close PTY before removing the worktree
        if let Some(repo_path) = repo_path {
            WorktreeManager::new(repo_path).remove(task_id)?;
        }
        Ok(())
    }
```

Note on imports: keep the single `use anyhow::{anyhow, Result};` line and remove the earlier `use anyhow::Result;` to avoid a duplicate import.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agency-app --test state`
Expected: PASS (3 passed), no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app/src/state.rs crates/agency-app/tests/state.rs
git commit -m "Add session lifecycle operations to AppState"
```

---

### Task 4: Tauri command wrappers + Channel streaming + state wiring

**Files:**
- Modify: `crates/agency-app/src/commands.rs`, `crates/agency-app/src/lib.rs`

**Interfaces:**
- Consumes: `AppState` methods from Tasks 2-3.
- Produces these `#[tauri::command]` functions (thin adapters, mapping `anyhow::Error` to `String`):
  - `list_projects(state) -> Result<Vec<Project>, String>`
  - `add_project(state, name: String, repo_path: String) -> Result<Project, String>`
  - `remove_project(state, id: String) -> Result<(), String>`
  - `start_task(state, project_id: String, prompt: String, profile: String, on_chunk: Channel<TerminalChunk>) -> Result<TaskInfo, String>` — streams PTY bytes base64-encoded over the channel.
  - `send_input(state, task_id: String, data: String) -> Result<(), String>`
  - `task_status(state, task_id: String) -> Result<StatusDto, String>`
  - `stop_task(state, task_id: String) -> Result<(), String>`
  - DTOs: `TerminalChunk { b64: String }` and `StatusDto { state: String, code: Option<i32> }`, both `#[serde(rename_all = "camelCase")]` + `Serialize` + `Clone`.
- `lib.rs::run()` builds the app state in `.setup(...)` (db at the app-data dir) and registers all commands.

- [ ] **Step 1: Implement the command wrappers and DTOs**

Replace `crates/agency-app/src/commands.rs` with:

```rust
use agency_core::registry::Project;
use agency_core::supervisor::AgentStatus;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::state::{AppState, TaskInfo};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalChunk {
    pub b64: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    pub state: String,
    pub code: Option<i32>,
}

fn status_dto(status: AgentStatus) -> StatusDto {
    match status {
        AgentStatus::Running => StatusDto { state: "running".into(), code: None },
        AgentStatus::Idle => StatusDto { state: "idle".into(), code: None },
        AgentStatus::Exited(c) => StatusDto { state: "exited".into(), code: Some(c) },
        AgentStatus::Crashed => StatusDto { state: "crashed".into(), code: None },
    }
}

#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    state.list_projects().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_project(
    state: State<'_, AppState>,
    name: String,
    repo_path: String,
) -> Result<Project, String> {
    state
        .add_project(&name, std::path::Path::new(&repo_path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_project(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.remove_project(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn start_task(
    state: State<'_, AppState>,
    project_id: String,
    prompt: String,
    profile: String,
    on_chunk: Channel<TerminalChunk>,
) -> Result<TaskInfo, String> {
    state
        .start_task(&project_id, &prompt, &profile, "HEAD", move |bytes| {
            let _ = on_chunk.send(TerminalChunk { b64: STANDARD.encode(&bytes) });
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn send_input(
    state: State<'_, AppState>,
    task_id: String,
    data: String,
) -> Result<(), String> {
    state
        .send_input(&task_id, data.as_bytes())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn task_status(state: State<'_, AppState>, task_id: String) -> Result<StatusDto, String> {
    state
        .task_status(&task_id)
        .map(status_dto)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_task(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    state.stop_task(&task_id).map_err(|e| e.to_string())
}
```

Note: `TaskInfo` must serialize for the command return. Add `serde::Serialize` (with `#[serde(rename_all = "camelCase")]`) to the `TaskInfo` derive in `state.rs`:

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInfo {
    pub task_id: String,
    pub branch: String,
}
```

(This is an additive derive on an `agency-app` type — allowed; it is not in `agency-core`.)

- [ ] **Step 2: Wire state setup and command registration in lib.rs**

Replace `crates/agency-app/src/lib.rs` with:

```rust
mod commands;
mod state;

pub use state::{AppState, TaskInfo};

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            use tauri::Manager;
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let state = AppState::new(&data_dir.join("agency.db"))?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_projects,
            commands::add_project,
            commands::remove_project,
            commands::start_task,
            commands::send_input,
            commands::task_status,
            commands::stop_task,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Agency");
}
```

- [ ] **Step 3: Verify the crate compiles and existing tests still pass**

Run: `cargo build -p agency-app`
Expected: builds cleanly, no warnings.

Run: `cargo test -p agency-app`
Expected: PASS (smoke + 3 state tests), no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs crates/agency-app/src/state.rs
git commit -m "Add Tauri command layer with channel-streamed terminal output"
```

---

### Task 5: Frontend — projects, task board, live terminal

**Files:**
- Create: `ui/src/api.ts`, `ui/src/b64.ts`, `ui/src/b64.test.ts`, `ui/src/components/ProjectSidebar.tsx`, `ui/src/components/TaskBoard.tsx`, `ui/src/components/TerminalPane.tsx`
- Modify: `ui/src/App.tsx`, `ui/src/styles.css`

**Interfaces:**
- Consumes: the Task 4 Tauri commands.
- Produces: a working UI. `App` holds the selected project and the active task; `ProjectSidebar` lists/creates/removes projects; `TaskBoard` creates a task and shows status; `TerminalPane` renders the live xterm terminal and wires input.

- [ ] **Step 1: Write the failing base64 helper test**

Create `ui/src/b64.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { b64ToBytes } from "./b64";

describe("b64ToBytes", () => {
  it("decodes ascii", () => {
    const bytes = b64ToBytes(btoa("hi\n"));
    expect(Array.from(bytes)).toEqual([104, 105, 10]);
  });

  it("decodes a high byte", () => {
    // base64 of [0xff, 0x00]
    const bytes = b64ToBytes("/wA=");
    expect(Array.from(bytes)).toEqual([255, 0]);
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `pnpm --dir ui test`
Expected: FAIL — cannot resolve `./b64`.

- [ ] **Step 3: Implement the base64 helper**

Create `ui/src/b64.ts`:

```ts
export function b64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `pnpm --dir ui test`
Expected: PASS (2 passed).

- [ ] **Step 5: Create the typed API layer**

Create `ui/src/api.ts`:

```ts
import { invoke, Channel } from "@tauri-apps/api/core";
import { b64ToBytes } from "./b64";

// NOTE: Project comes from agency_core (snake_case serde field names).
export interface Project {
  id: string;
  name: string;
  repo_path: string;
  default_agent: string | null;
  default_provider: string | null;
}

export interface TaskInfo {
  taskId: string;
  branch: string;
}

export interface StatusDto {
  state: string;
  code: number | null;
}

export const listProjects = () => invoke<Project[]>("list_projects");

export const addProject = (name: string, repoPath: string) =>
  invoke<Project>("add_project", { name, repoPath });

export const removeProject = (id: string) =>
  invoke<void>("remove_project", { id });

export const sendInput = (taskId: string, data: string) =>
  invoke<void>("send_input", { taskId, data });

export const taskStatus = (taskId: string) =>
  invoke<StatusDto>("task_status", { taskId });

export const stopTask = (taskId: string) => invoke<void>("stop_task", { taskId });

export function startTask(
  projectId: string,
  prompt: string,
  profile: string,
  onBytes: (bytes: Uint8Array) => void,
): Promise<TaskInfo> {
  const onChunk = new Channel<{ b64: string }>();
  onChunk.onmessage = (msg) => onBytes(b64ToBytes(msg.b64));
  return invoke<TaskInfo>("start_task", { projectId, prompt, profile, onChunk });
}
```

- [ ] **Step 6: Create ProjectSidebar**

Create `ui/src/components/ProjectSidebar.tsx`:

```tsx
import { useEffect, useState } from "react";
import { addProject, listProjects, removeProject, Project } from "../api";

interface Props {
  selectedId: string | null;
  onSelect: (project: Project) => void;
}

export default function ProjectSidebar({ selectedId, onSelect }: Props) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [name, setName] = useState("");
  const [repoPath, setRepoPath] = useState("");

  async function refresh() {
    setProjects(await listProjects());
  }

  useEffect(() => {
    refresh();
  }, []);

  async function handleAdd() {
    if (!name.trim() || !repoPath.trim()) return;
    await addProject(name.trim(), repoPath.trim());
    setName("");
    setRepoPath("");
    await refresh();
  }

  async function handleRemove(id: string) {
    await removeProject(id);
    await refresh();
  }

  return (
    <aside className="sidebar">
      <h2>Projects</h2>
      <ul className="project-list">
        {projects.map((p) => (
          <li
            key={p.id}
            className={p.id === selectedId ? "selected" : ""}
            onClick={() => onSelect(p)}
          >
            <span>{p.name}</span>
            <button
              onClick={(e) => {
                e.stopPropagation();
                handleRemove(p.id);
              }}
            >
              ×
            </button>
          </li>
        ))}
      </ul>
      <div className="add-project">
        <input placeholder="name" value={name} onChange={(e) => setName(e.target.value)} />
        <input
          placeholder="/path/to/repo"
          value={repoPath}
          onChange={(e) => setRepoPath(e.target.value)}
        />
        <button onClick={handleAdd}>Add project</button>
      </div>
    </aside>
  );
}
```

- [ ] **Step 7: Create TerminalPane**

Create `ui/src/components/TerminalPane.tsx`:

```tsx
import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { sendInput, startTask, taskStatus, TaskInfo } from "../api";

interface Props {
  projectId: string;
  prompt: string;
  onStatus: (label: string) => void;
}

export default function TerminalPane({ projectId, prompt, onStatus }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const term = new Terminal({ convertEol: true, fontSize: 13 });
    const fit = new FitAddon();
    term.loadAddon(fit);
    if (containerRef.current) {
      term.open(containerRef.current);
      fit.fit();
    }

    let info: TaskInfo | null = null;
    let statusTimer: number | undefined;
    let disposed = false;

    startTask(projectId, prompt, "shell", (bytes) => term.write(bytes)).then((i) => {
      if (disposed) return;
      info = i;
      term.onData((data) => {
        sendInput(i.taskId, data);
      });
      statusTimer = window.setInterval(async () => {
        try {
          const s = await taskStatus(i.taskId);
          onStatus(s.code != null ? `${s.state} (${s.code})` : s.state);
        } catch {
          /* task gone */
        }
      }, 1000);
    });

    return () => {
      disposed = true;
      if (statusTimer) window.clearInterval(statusTimer);
      term.dispose();
      void info; // session teardown handled by App via stopTask
    };
  }, [projectId, prompt, onStatus]);

  return <div className="terminal" ref={containerRef} />;
}
```

- [ ] **Step 8: Create TaskBoard**

Create `ui/src/components/TaskBoard.tsx`:

```tsx
import { useState } from "react";
import { Project } from "../api";
import TerminalPane from "./TerminalPane";

interface Props {
  project: Project;
}

export default function TaskBoard({ project }: Props) {
  const [prompt, setPrompt] = useState("");
  const [activePrompt, setActivePrompt] = useState<string | null>(null);
  const [status, setStatus] = useState("");

  return (
    <main className="board">
      <header className="board-header">
        <h2>{project.name}</h2>
        <code>{project.repo_path}</code>
      </header>
      {activePrompt === null ? (
        <div className="new-task">
          <textarea
            placeholder="Task prompt (the shell profile ignores it for now; real agents will use it)"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
          />
          <button onClick={() => setActivePrompt(prompt)}>Start task</button>
        </div>
      ) : (
        <div className="task-running">
          <div className="task-status">status: {status || "starting…"}</div>
          <TerminalPane
            key={`${project.id}:${activePrompt}`}
            projectId={project.id}
            prompt={activePrompt}
            onStatus={setStatus}
          />
          <button onClick={() => setActivePrompt(null)}>Close terminal</button>
        </div>
      )}
    </main>
  );
}
```

- [ ] **Step 9: Wire App and styles**

Replace `ui/src/App.tsx`:

```tsx
import { useState } from "react";
import ProjectSidebar from "./components/ProjectSidebar";
import TaskBoard from "./components/TaskBoard";
import { Project } from "./api";

export default function App() {
  const [project, setProject] = useState<Project | null>(null);
  return (
    <div className="app">
      <ProjectSidebar selectedId={project?.id ?? null} onSelect={setProject} />
      {project ? (
        <TaskBoard project={project} />
      ) : (
        <main className="board empty">Select or add a project to begin.</main>
      )}
    </div>
  );
}
```

Replace `ui/src/styles.css`:

```css
:root { color-scheme: dark; }
* { box-sizing: border-box; }
body { margin: 0; font-family: ui-sans-serif, system-ui, sans-serif; background: #14161a; color: #e6e6e6; }
.app { display: flex; height: 100vh; }
.sidebar { width: 260px; border-right: 1px solid #2a2e36; padding: 12px; display: flex; flex-direction: column; gap: 12px; }
.sidebar h2 { font-size: 14px; text-transform: uppercase; letter-spacing: 0.08em; color: #9aa3b2; }
.project-list { list-style: none; margin: 0; padding: 0; flex: 1; overflow: auto; }
.project-list li { display: flex; justify-content: space-between; padding: 8px; border-radius: 6px; cursor: pointer; }
.project-list li:hover { background: #1d2128; }
.project-list li.selected { background: #243042; }
.project-list button { background: none; border: none; color: #8893a5; cursor: pointer; }
.add-project { display: flex; flex-direction: column; gap: 6px; }
.add-project input, .new-task textarea { background: #1b1f27; border: 1px solid #2a2e36; color: #e6e6e6; border-radius: 6px; padding: 8px; }
button { background: #2d6cdf; border: none; color: white; padding: 8px 12px; border-radius: 6px; cursor: pointer; }
.board { flex: 1; display: flex; flex-direction: column; padding: 16px; gap: 12px; }
.board.empty { align-items: center; justify-content: center; color: #6b7280; }
.board-header code { color: #8893a5; font-size: 12px; }
.new-task { display: flex; flex-direction: column; gap: 8px; max-width: 640px; }
.new-task textarea { min-height: 120px; resize: vertical; }
.task-running { display: flex; flex-direction: column; gap: 8px; flex: 1; }
.task-status { color: #9aa3b2; font-size: 13px; }
.terminal { flex: 1; min-height: 320px; background: #000; border-radius: 8px; padding: 6px; overflow: hidden; }
```

- [ ] **Step 10: Verify the frontend builds and unit tests pass**

```bash
pnpm --dir ui test
pnpm --dir ui build
```
Expected: vitest 2 passed; `tsc && vite build` completes with no type errors and writes `ui/dist/`.

- [ ] **Step 11: Manual smoke check (record result in the commit/report, not automated)**

Run the app: `pnpm --dir ui exec tauri dev --config ../crates/agency-app/tauri.conf.json` (or from `crates/agency-app`: `cargo tauri dev` if `tauri-cli` is installed). Confirm: window opens, you can add a project pointing at a real local git repo, click it, click "Start task", and an interactive shell terminal appears in the worktree and accepts input (e.g. `ls`, `pwd` shows the `.agency/worktrees/<id>` path). Note the outcome in your report. This step has no automated assertion — it is a human/agent visual confirmation.

- [ ] **Step 12: Commit**

```bash
git add ui/src
git commit -m "Add project sidebar, task board, and live terminal UI"
```

---

## Self-Review

**Spec coverage (Phase 2 scope):**
- Tauri desktop window → Task 1 (scaffold) + Task 4 (`run()` setup). ✓
- Project sidebar (multi-project dashboard) → Task 5 `ProjectSidebar` over Task 2/4 project commands. ✓
- Task board → Task 5 `TaskBoard`. ✓
- Live PTY terminal pane (xterm) bound to a task's PTY, with interject (input) → Task 5 `TerminalPane` + Task 4 `start_task`/`send_input` + Channel streaming. ✓
- Worktree-per-task isolation via the engine → Task 3 `start_task` uses `WorktreeManager` + `spawn_agent`. ✓
- Status surfaced → Task 4 `task_status` + Task 5 polling. ✓
- Local-only → no network code; default profile is a local shell. ✓
- Deferred (correctly out of scope): git diff/stage/commit/push panel (Phase 3), merge-resolver (Phase 4), real agent profiles + provider/settings UI + packaging/icons (Phase 5), live PTY reattach across restarts, structured stream-json sidecar.

**Placeholder scan:** No TBD/TODO; every code step has complete code; commands have expected output. The one non-automated step (Task 5 Step 11) is explicitly labeled a manual visual check, not a silent gap.

**Type consistency:** `AppState`, `Session`, `TaskInfo`, `start_task`/`send_input`/`task_status`/`stop_task`, `TerminalChunk`, `StatusDto` names/signatures match across state.rs ↔ commands.rs ↔ api.ts. The `dead_code` allow added in Task 2 is explicitly removed in Task 3. TS `Project` uses snake_case keys to match `agency_core`'s serde output (called out in Global Constraints).

**Notes for the executor:**
- Requires `git` and a POSIX shell on PATH (present on this macOS machine), plus Node/pnpm (verified: Node 25, pnpm 11) and the Rust toolchain.
- The first `cargo build -p agency-app` compiles Tauri and its macro crates — expect a longer first build.
- If `cargo tauri` is not installed, use the frontend-local CLI: `pnpm --dir ui exec tauri ...`, or `cargo install tauri-cli --version "^2"`.
- Do not modify `agency-core`. Adding `Serialize` to the `agency-app`-owned `TaskInfo` is fine; needing a change in `agency-core` is a STOP-and-report condition.
