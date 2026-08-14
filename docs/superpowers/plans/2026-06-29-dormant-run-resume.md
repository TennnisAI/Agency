# Resume Stopped Runs on Reopen Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After the app is quit and reopened, a stopped run is dormant (not a dead pane) and clicking it immediately brings it back — resuming the prior agent conversation where the CLI supports it, or starting fresh otherwise.

**Architecture:** Agent profiles gain an optional `resume_args` recipe. A new `ensure_run_active(id)` backend command transparently respawns a run whose daemon session is gone (resume args for resume-capable agents, fresh prompt otherwise, a shell for terminals). The frontend calls `ensure_run_active` before attaching, so it never subscribes to a missing session. The run list already shows per-run status, so a gone session renders as "stopped".

**Tech Stack:** Rust (`agency-core` rusqlite registry + `agency-app` Tauri state/commands), TypeScript/React (`ui`).

## Global Constraints

- Branch: `feat/termd` (this continues the terminal-daemon work; do not branch off).
- `AgentProfile` gains `pub resume_args: Option<Vec<String>>` with `#[serde(default)]` (so older serialized profiles / frontend JSON without the field deserialize to `None`).
- Profiles DB migration: guard with the existing `column_exists` helper, then `ALTER TABLE profiles ADD COLUMN resume_args TEXT` (nullable JSON). Add the column to the `CREATE TABLE IF NOT EXISTS profiles` block too, for fresh DBs.
- Built-in resume recipes (exact): claude `["--continue"]`; codex `["resume","--last"]`; pi `["--continue"]`; opencode `["--continue"]`; copilot `["--continue"]`; cursor (command `cursor-agent`) `None`; hermes `None`. Terminals never resume.
- Retrofitting must NOT clobber a user's customized `command`/`args`/`env`: only set `resume_args` on an existing built-in profile when it is currently unset.
- `ensure_run_active` no-ops unless the session status is `SessionStatus::Gone`; terminal kind → fresh `$SHELL -l` in the project repo root; agent kind → resume args if the profile has them, else the rendered prompt; the optional setup script wraps the command in both agent cases (same as `create_run`/`rerun`).
- v1 assumes the run's git worktree still exists on disk (worktrees survive a quit). Restoring a deleted worktree is out of scope; a missing worktree surfaces as a spawn error (not a silent blank pane).
- `SessionStatus` enum is unchanged (`Running`/`Exited{code}`/`Gone`); `Gone` already means "no live session" = stopped.
- Run from the repo root: `cargo test -p agency-core`, `cargo test -p agency-app`. Frontend typecheck/build via the direct binaries in `ui/`: `./node_modules/.bin/tsc --noEmit` and `./node_modules/.bin/vite build` (per project memory: pnpm 11 `pnpm build` can mis-exit; use the direct binaries).

---

## File Structure

- `crates/agency-core/src/profile.rs` — add `resume_args` field to `AgentProfile`.
- `crates/agency-core/src/registry.rs` — profiles migration + `resume_args` in CREATE TABLE + read/write it in `upsert_profile`/`get_profile`/`list_profiles`/`row_to_profile`; add `ensure_profile_resume_args`.
- `crates/agency-app/src/state.rs` — seed/retrofit the 7 built-in profiles; add the pure `agent_argv` helper and the `ensure_run_active` method; fix `AgentProfile` literal constructors for the new field.
- `crates/agency-app/src/commands.rs` — `ensure_run_active` Tauri command.
- `crates/agency-app/src/lib.rs` — register the command.
- `ui/src/api.ts` — `ensureRunActive` binding.
- `ui/src/components/FocusTerminal.tsx` — call `ensureRunActive` before attach (run stream only).

---

### Task 1: `resume_args` on AgentProfile + DB migration + persistence

**Files:**
- Modify: `crates/agency-core/src/profile.rs`
- Modify: `crates/agency-core/src/registry.rs`
- Test: inline `#[cfg(test)]` in `registry.rs`

**Interfaces:**
- Produces:
  - `AgentProfile { name, command, args, env, resume_args: Option<Vec<String>> }`
  - `Registry::ensure_profile_resume_args(&self, name: &str, resume_args: &Option<Vec<String>>) -> Result<()>` (sets `resume_args` only when currently NULL)
  - `upsert_profile`/`get_profile`/`list_profiles` round-trip `resume_args`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `crates/agency-core/src/registry.rs`:

```rust
#[test]
fn profile_resume_args_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(&dir.path().join("a.db")).unwrap();
    reg.upsert_profile(&AgentProfile {
        name: "claude".into(),
        command: "claude".into(),
        args: vec![],
        env: vec![],
        resume_args: Some(vec!["--continue".into()]),
    })
    .unwrap();
    reg.upsert_profile(&AgentProfile {
        name: "cursor".into(),
        command: "cursor-agent".into(),
        args: vec![],
        env: vec![],
        resume_args: None,
    })
    .unwrap();
    assert_eq!(
        reg.get_profile("claude").unwrap().unwrap().resume_args,
        Some(vec!["--continue".into()])
    );
    assert_eq!(reg.get_profile("cursor").unwrap().unwrap().resume_args, None);
}

#[test]
fn ensure_profile_resume_args_only_sets_when_unset() {
    let dir = tempfile::tempdir().unwrap();
    let reg = Registry::open(&dir.path().join("a.db")).unwrap();
    reg.upsert_profile(&AgentProfile {
        name: "claude".into(), command: "claude".into(),
        args: vec![], env: vec![], resume_args: None,
    }).unwrap();
    // Unset -> gets set.
    reg.ensure_profile_resume_args("claude", &Some(vec!["--continue".into()])).unwrap();
    assert_eq!(reg.get_profile("claude").unwrap().unwrap().resume_args, Some(vec!["--continue".into()]));
    // Already set -> not clobbered.
    reg.ensure_profile_resume_args("claude", &Some(vec!["--other".into()])).unwrap();
    assert_eq!(reg.get_profile("claude").unwrap().unwrap().resume_args, Some(vec!["--continue".into()]));
}

#[test]
fn migrates_legacy_profiles_table_without_resume_args() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE profiles (name TEXT PRIMARY KEY, command TEXT NOT NULL, args TEXT NOT NULL, env TEXT NOT NULL);
             INSERT INTO profiles (name, command, args, env) VALUES ('claude','claude','[]','[]');",
        ).unwrap();
    }
    // Opening must add the column and read the legacy row as resume_args = None.
    let reg = Registry::open(&path).unwrap();
    assert_eq!(reg.get_profile("claude").unwrap().unwrap().resume_args, None);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core registry::tests::profile_resume -- --nocapture`
Expected: FAIL to compile (`resume_args` field missing).

- [ ] **Step 3: Add the field**

In `crates/agency-core/src/profile.rs`, change the struct to:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentProfile {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    #[serde(default)]
    pub resume_args: Option<Vec<String>>,
}
```

(Leave `render_args` unchanged.)

- [ ] **Step 4: Migration + CREATE TABLE column**

In `crates/agency-core/src/registry.rs`, in the `CREATE TABLE IF NOT EXISTS profiles (...)` block, add the column:

```sql
CREATE TABLE IF NOT EXISTS profiles (
    name TEXT PRIMARY KEY,
    command TEXT NOT NULL,
    args TEXT NOT NULL,
    env TEXT NOT NULL,
    resume_args TEXT
);
```

Then, alongside the other `column_exists` migrations (after the `runs` migrations), add:

```rust
if !column_exists(&conn, "profiles", "resume_args")? {
    conn.execute("ALTER TABLE profiles ADD COLUMN resume_args TEXT", [])?;
}
```

- [ ] **Step 5: Read/write `resume_args` in the CRUD**

Update `upsert_profile`:

```rust
pub fn upsert_profile(&self, p: &AgentProfile) -> Result<()> {
    let args = serde_json::to_string(&p.args)?;
    let env = serde_json::to_string(&p.env)?;
    let resume = match &p.resume_args {
        Some(r) => Some(serde_json::to_string(r)?),
        None => None,
    };
    self.conn.execute(
        "INSERT INTO profiles (name, command, args, env, resume_args) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(name) DO UPDATE SET command = ?2, args = ?3, env = ?4, resume_args = ?5",
        rusqlite::params![p.name, p.command, args, env, resume],
    )?;
    Ok(())
}
```

Add the setter:

```rust
/// Set a profile's resume recipe ONLY if it is currently NULL (so a user's
/// customization is never clobbered). No-op if the profile does not exist.
pub fn ensure_profile_resume_args(&self, name: &str, resume_args: &Option<Vec<String>>) -> Result<()> {
    let resume = match resume_args {
        Some(r) => Some(serde_json::to_string(r)?),
        None => None,
    };
    self.conn.execute(
        "UPDATE profiles SET resume_args = ?2 WHERE name = ?1 AND resume_args IS NULL",
        rusqlite::params![name, resume],
    )?;
    Ok(())
}
```

Update both SELECTs to include the column (`get_profile` and `list_profiles`):

```rust
// get_profile:
.prepare("SELECT name, command, args, env, resume_args FROM profiles WHERE name = ?1")?
// list_profiles:
.prepare("SELECT name, command, args, env, resume_args FROM profiles ORDER BY name")?
```

Update `row_to_profile`:

```rust
fn row_to_profile(row: &rusqlite::Row) -> Result<AgentProfile> {
    let args: String = row.get(2)?;
    let env: String = row.get(3)?;
    let resume_args: Option<String> = row.get(4)?;
    Ok(AgentProfile {
        name: row.get(0)?,
        command: row.get(1)?,
        args: serde_json::from_str(&args)?,
        env: serde_json::from_str(&env)?,
        resume_args: match resume_args {
            Some(s) => Some(serde_json::from_str(&s)?),
            None => None,
        },
    })
}
```

- [ ] **Step 6: Fix `AgentProfile` literal constructions in this crate**

Search the crate for `AgentProfile {` and add `resume_args: None,` to any literal that lacks it (e.g. test fixtures in `registry.rs`). Run: `rg "AgentProfile \{" crates/agency-core/src` and fix each.

- [ ] **Step 7: Run tests**

Run: `cargo test -p agency-core registry -- --nocapture`
Expected: PASS (the 3 new tests + existing registry tests). Also `cargo build -p agency-core` clean.

- [ ] **Step 8: Commit**

```bash
git add crates/agency-core/src/profile.rs crates/agency-core/src/registry.rs
git commit -m "feat(resume): add resume_args to AgentProfile with migration + CRUD"
```

---

### Task 2: Seed/retrofit the 7 built-in agent profiles with resume recipes

**Files:**
- Modify: `crates/agency-app/src/state.rs` (the seeding block in `AppState::new`, ~line 270-289, and the `agent_profile` helper ~243)
- Test: `crates/agency-app/tests/profiles.rs` (integration test — `AppState` is constructed via its public `new`, and assertions use the public `list_profiles`; the `registry` field is private and not reachable from a `tests/` crate)

**Interfaces:**
- Consumes: `Registry::{upsert_profile, get_profile, ensure_profile_resume_args}`, `AgentProfile { ..., resume_args }`.
- Produces: after `AppState::new`, the 7 built-ins exist (via `list_profiles`) with the correct `resume_args`.

- [ ] **Step 1: Write the failing test**

Add to `crates/agency-app/tests/profiles.rs` (this file already builds `AppState` via `AppState::new(&dir.path().join("agency.db"), dir.path())` — reuse that exact pattern and the file's existing imports; `AgentProfile` is returned by `list_profiles`):

```rust
#[test]
fn seeds_builtin_resume_recipes() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(&dir.path().join("agency.db"), dir.path()).unwrap();
    let profiles = state.list_profiles().unwrap();
    let get = |name: &str| profiles.iter().find(|p| p.name == name).cloned().unwrap();
    assert_eq!(get("claude").resume_args, Some(vec!["--continue".into()]));
    assert_eq!(get("codex").resume_args, Some(vec!["resume".into(), "--last".into()]));
    assert_eq!(get("pi").resume_args, Some(vec!["--continue".into()]));
    assert_eq!(get("opencode").resume_args, Some(vec!["--continue".into()]));
    assert_eq!(get("copilot").resume_args, Some(vec!["--continue".into()]));
    assert_eq!(get("cursor").command, "cursor-agent");
    assert_eq!(get("cursor").resume_args, None);
    assert_eq!(get("hermes").resume_args, None);
}
```

(Match `tests/profiles.rs`'s existing `use` imports — it already imports `AppState`. Add an `AgentProfile` import only if field access needs it; `.cloned()` works since `AgentProfile: Clone`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-app --test profiles seeds_builtin_resume_recipes -- --nocapture`
Expected: FAIL (codex/opencode/copilot/cursor not seeded; existing ones lack resume_args).

- [ ] **Step 3: Replace the seeding block**

In `AppState::new`, replace the existing built-in agent loop (the `for (name, command) in [("claude", "claude"), ("pi", "pi"), ("hermes", "hermes")]` block) with a definition-driven seed that both creates missing profiles and retrofits recipes onto existing ones:

```rust
// Built-in agent profiles and their resume recipes. Resume recipes are seeded
// for missing profiles and retrofitted onto existing ones only when unset, so a
// user's customized command/args/env is never clobbered. cursor/hermes are
// id-keyed (not cwd-keyed) so they start fresh rather than risk resuming the
// wrong global session.
let builtins: [(&str, &str, Option<Vec<String>>); 7] = [
    ("claude", "claude", Some(vec!["--continue".into()])),
    ("codex", "codex", Some(vec!["resume".into(), "--last".into()])),
    ("pi", "pi", Some(vec!["--continue".into()])),
    ("opencode", "opencode", Some(vec!["--continue".into()])),
    ("copilot", "copilot", Some(vec!["--continue".into()])),
    ("cursor", "cursor-agent", None),
    ("hermes", "hermes", None),
];
for (name, command, resume_args) in builtins {
    if registry.get_profile(name)?.is_none() {
        registry.upsert_profile(&AgentProfile {
            name: name.to_string(),
            command: command.to_string(),
            args: vec![],
            env: vec![],
            resume_args: resume_args.clone(),
        })?;
    } else {
        registry.ensure_profile_resume_args(name, &resume_args)?;
    }
}
```

- [ ] **Step 4: Fix the `agent_profile` helper + any other `AgentProfile` literals**

The `agent_profile` helper (`state.rs:243`) and the `shell` seed literal (`state.rs:276`) construct `AgentProfile` without `resume_args` — add `resume_args: None` to each. Run `rg "AgentProfile \{" crates/agency-app/src` and fix any remaining literal. If `agent_profile` is now unused after Step 3, delete it.

- [ ] **Step 5: Run tests**

Run: `cargo test -p agency-app --test profiles -- --nocapture` then `cargo test -p agency-app`
Expected: PASS (new test + the existing 45 tests). `cargo build` clean.

- [ ] **Step 6: Commit**

```bash
git add crates/agency-app/src/state.rs
git commit -m "feat(resume): seed 7 built-in agents with resume recipes (retrofit-safe)"
```

---

### Task 3: `ensure_run_active` backend (resume / fresh / shell)

**Files:**
- Modify: `crates/agency-app/src/state.rs`
- Test: pure `agent_argv` tests in the `#[cfg(test)] mod tests` module of `state.rs` (it can see the private free fn via `super::`); `ensure_run_active` integration tests in `crates/agency-app/tests/state.rs` (needs `AppState` + the real daemon/repo harness)

**Interfaces:**
- Consumes: `run_status`, `run_record`, `project_repo`, `provider_env`, `session_name`, `agency_core::scripts::{script_env, wrap_setup}`, `agency_core::config::load`, `Registry::get_profile`, `term...start_session`, `AgentProfile.resume_args`, and (in the integration test) the public `register_profile`, `add_project`, `create_run`, `stop_run`, `discard_run`, `run_status`.
- Produces:
  - free fn `agent_argv(profile: &AgentProfile, prompt: &str, use_resume: bool, setup: Option<&str>) -> (String, Vec<String>)`
  - `AppState::ensure_run_active(&self, id: &str) -> Result<()>`

- [ ] **Step 1a: Write the failing pure-logic tests**

Add to the existing `#[cfg(test)] mod tests` in `crates/agency-app/src/state.rs`. Add `use super::agent_argv;` and `use agency_core::profile::AgentProfile;` to that module's imports.

```rust
#[test]
fn agent_argv_uses_resume_args_when_available() {
    let p = AgentProfile {
        name: "claude".into(), command: "claude".into(),
        args: vec!["{{prompt}}".into()], env: vec![],
        resume_args: Some(vec!["--continue".into()]),
    };
    let (cmd, args) = agent_argv(&p, "do the thing", true, None);
    assert_eq!(cmd, "claude");
    assert_eq!(args, vec!["--continue".to_string()]);
}

#[test]
fn agent_argv_falls_back_to_prompt_without_resume_args() {
    let p = AgentProfile {
        name: "cursor".into(), command: "cursor-agent".into(),
        args: vec!["{{prompt}}".into()], env: vec![],
        resume_args: None,
    };
    let (cmd, args) = agent_argv(&p, "hello", true, None);
    assert_eq!(cmd, "cursor-agent");
    assert_eq!(args, vec!["hello".to_string()]);
}

#[test]
fn agent_argv_fresh_ignores_resume_args() {
    let p = AgentProfile {
        name: "claude".into(), command: "claude".into(),
        args: vec!["{{prompt}}".into()], env: vec![],
        resume_args: Some(vec!["--continue".into()]),
    };
    let (_cmd, args) = agent_argv(&p, "fresh prompt", false, None);
    assert_eq!(args, vec!["fresh prompt".to_string()]);
}
```

- [ ] **Step 1b: Write the failing integration test**

Add to `crates/agency-app/tests/state.rs`, reusing that file's existing `init_repo` helper, `AppState::new(db, data_dir)` pattern, and status-poll style. Ensure `AgentProfile` and `SessionStatus` are imported there (match the file's existing imports):

```rust
#[test]
fn ensure_run_active_respawns_a_stopped_agent_run() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo); // repo on `main` with a commit
    let state = AppState::new(&dir.path().join("agency.db"), dir.path()).unwrap();
    // A fake agent whose command just sleeps, so a respawn is observable as Running.
    state.register_profile(AgentProfile {
        name: "sleeper".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "sleep 5".into()],
        env: vec![],
        resume_args: None,
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "sleeper", "HEAD", None).unwrap();

    // Stop the run's session (record stays) -> status becomes Gone.
    state.stop_run(&info.id).unwrap();
    let mut gone = false;
    for _ in 0..75 {
        if matches!(state.run_status(&info.id).unwrap(), SessionStatus::Gone) { gone = true; break; }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(gone, "session did not become Gone after stop_run");

    // Reactivate -> a new session comes up (fresh fallback, since resume_args is None).
    state.ensure_run_active(&info.id).unwrap();
    let mut back = false;
    for _ in 0..75 {
        if !matches!(state.run_status(&info.id).unwrap(), SessionStatus::Gone) { back = true; break; }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(back, "ensure_run_active did not respawn the session");

    state.discard_run(&info.id).unwrap();
}

#[test]
fn ensure_run_active_is_noop_when_session_present() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = AppState::new(&dir.path().join("agency.db"), dir.path()).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_terminal(&project.id).unwrap();
    // Session is live; ensure_run_active must not error or take it down.
    state.ensure_run_active(&info.id).unwrap();
    assert!(!matches!(state.run_status(&info.id).unwrap(), SessionStatus::Gone));
    state.discard_run(&info.id).unwrap();
}
```

(If `stop_run` does not drive a run to `Gone` in practice, fall back to `discard_run` is NOT acceptable — it deletes the record; instead inspect what `stop_run` does and use whatever public method kills the session while keeping the run record. The intent is: record present, session `Gone`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-app agent_argv -- --nocapture` and `cargo test -p agency-app --test state ensure_run_active -- --nocapture`
Expected: FAIL to compile (`agent_argv`/`ensure_run_active` missing).

- [ ] **Step 3: Implement `agent_argv` + `ensure_run_active`**

Add the free function near `rerun` in `state.rs`:

```rust
/// Decide the (command, args) to launch for an agent run. With `use_resume` and a
/// resume recipe present, launch the resume args (no prompt). Otherwise launch a
/// fresh session from the rendered prompt. The optional setup script wraps the
/// command in both cases (same as create_run/rerun).
fn agent_argv(
    profile: &AgentProfile,
    prompt: &str,
    use_resume: bool,
    setup: Option<&str>,
) -> (String, Vec<String>) {
    let base_args: Vec<String> = match (use_resume, &profile.resume_args) {
        (true, Some(resume)) => resume.clone(),
        _ => profile
            .render_args(prompt)
            .into_iter()
            .filter(|a| !a.is_empty())
            .collect(),
    };
    agency_core::scripts::wrap_setup(setup, &profile.command, &base_args)
}
```

Add the method on `AppState` (near `rerun`):

```rust
/// Ensure the run has a live daemon session, transparently respawning it if the
/// previous session is gone (e.g. after the app was quit). Prefers resuming the
/// agent's prior context; falls back to a fresh start; terminals get a fresh
/// shell. No-op if a session already exists.
pub fn ensure_run_active(&self, id: &str) -> Result<()> {
    if !matches!(self.run_status(id)?, SessionStatus::Gone) {
        return Ok(());
    }
    let run = self.run_record(id)?;
    let repo = self.project_repo(&run.project_id)?;

    if run.kind == "terminal" {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        self.term.read().unwrap().start_session(
            &session_name(id), &repo, &shell, &["-l".to_string()], &[], 220, 50,
        )?;
        return Ok(());
    }

    let config = agency_core::config::load(&repo);
    let worktree = repo.join(".agency").join("worktrees").join(&run.id);
    let profile = {
        let reg = self.registry.lock().unwrap();
        reg.get_profile(&run.agent)?
            .ok_or_else(|| anyhow!("unknown agent profile: {}", run.agent))?
    };
    let mut env = self.provider_env()?;
    env.extend(profile.env.iter().cloned());
    env.extend(agency_core::scripts::script_env(&worktree, &repo, &run.id, run.port_base));
    let (command, args) = agent_argv(&profile, &run.prompt, true, config.scripts.setup.as_deref());
    self.term.read().unwrap().start_session(
        &session_name(id), &worktree, &command, &args, &env, 220, 50,
    )?;
    Ok(())
}
```

(Confirm `run_record`, `project_repo`, `provider_env`, `session_name` exist with these names — they are used by `rerun`/`create_run` already. If `run_record` is named differently, match the real name.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-app -- --nocapture` (runs the new argv unit tests + the two ensure_run_active integration tests + existing suite).
Expected: PASS. `cargo build` clean.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app/src/state.rs
git commit -m "feat(resume): ensure_run_active respawns gone runs (resume/fresh/shell)"
```

---

### Task 4: `ensure_run_active` Tauri command + frontend binding

**Files:**
- Modify: `crates/agency-app/src/commands.rs`
- Modify: `crates/agency-app/src/lib.rs`
- Modify: `ui/src/api.ts`

**Interfaces:**
- Consumes: `AppState::ensure_run_active`.
- Produces: Tauri command `ensure_run_active(id)`; `ui/src/api.ts` export `ensureRunActive(id): Promise<void>`.

- [ ] **Step 1: Add the command**

In `crates/agency-app/src/commands.rs`, mirroring the style of the existing run commands (e.g. `rerun`):

```rust
#[tauri::command]
pub fn ensure_run_active(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.ensure_run_active(&id).map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Register it**

In `crates/agency-app/src/lib.rs`, add `commands::ensure_run_active` to the `tauri::generate_handler![...]` list (next to the other run commands like `rerun`/`attach_run`).

- [ ] **Step 3: Add the frontend binding**

In `ui/src/api.ts`, next to `rerun`/`attachRun`:

```ts
export const ensureRunActive = (id: string) => invoke<void>("ensure_run_active", { id });
```

- [ ] **Step 4: Build**

Run: `cargo build -p agency-app` (clean) and from `ui/`: `./node_modules/.bin/tsc --noEmit` (clean).
Expected: both succeed. (Behavior is exercised by Task 3's tests + Task 5's manual check.)

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs ui/src/api.ts
git commit -m "feat(resume): expose ensure_run_active command + api binding"
```

---

### Task 5: Frontend — resume-on-focus (and confirm stopped status renders)

**Files:**
- Modify: `ui/src/components/FocusTerminal.tsx`
- Test: manual (GUI) + typecheck/build

**Interfaces:**
- Consumes: `ensureRunActive` from `../api`.

The bug today: `FocusTerminal` attaches via a `stream` abstraction (the `TerminalStream` interface, ~line 12, with `attach`/`detach`/`resize`/`input`/`preview`). On mount it calls `stream.attach(runId, ...)` directly; if the daemon has no session the subscribe fails and the pane is blank. Fix: ensure the run is active before attaching — but ONLY for the main run stream, NOT the run-script stream (run-script sessions are not reactivatable here).

- [ ] **Step 1: Add `ensureActive` to the stream abstraction**

In `FocusTerminal.tsx`, add an optional method to the `TerminalStream` interface and wire it on the run stream only:

```ts
import { attachRun, detachRun, resizeRun, runInput, runPreview, ensureRunActive,
  attachRunScript, detachRunScript, resizeRunScript, runScriptInput, runScriptPreview } from "../api";

interface TerminalStream {
  // ...existing fields...
  ensureActive?: (id: string) => Promise<void>;
}

const runStream: TerminalStream = {
  attach: attachRun, detach: detachRun, resize: resizeRun, input: runInput, preview: runPreview,
  ensureActive: ensureRunActive,
};

const runScriptStream: TerminalStream = {
  attach: attachRunScript, detach: detachRunScript, resize: resizeRunScript,
  input: runScriptInput, preview: runScriptPreview,
  // no ensureActive — run-script sessions are not reactivated here
};
```

(Match the real shape of the existing `runStream`/`runScriptStream` objects in the file.)

- [ ] **Step 2: Call it before attach, surfacing errors**

In the mount/attach `useEffect`, before `stream.attach(runId, ...)`, await reactivation and surface failure into the pane instead of attaching to nothing:

```ts
let cancelled = false;
(async () => {
  try {
    await stream.ensureActive?.(runId);
  } catch (e) {
    if (!cancelled) term.write(`\r\n\x1b[31mCould not resume this run: ${e}\x1b[0m\r\n`);
    return; // don't attach to a session that failed to come up
  }
  if (cancelled) return;
  stream.attach(runId, onBytes);
  // ...the rest of the existing attach setup (resize/preview/input wiring)...
})();
```

Adapt to the effect's actual structure (it currently calls `stream.attach` and sets up resize/preview/input). Keep the existing cleanup (`detach`) on unmount, and the `cancelled` guard so a fast unmount doesn't attach. The brief "resuming…" state is simply the gap until the first bytes/snapshot arrive.

- [ ] **Step 3: Confirm stopped status renders (no code change expected)**

The run list already shows per-run status dots from `RunInfo.status` (`AgentFocus.tsx:43` uses `r.status.state === "running"`; `ProjectTree.tsx:148` uses `statusClass(r.status)`; `AgentTile.tsx:39` uses `statusLabel(run.status)`). After this feature a stopped run reports `Gone`. Verify `statusClass`/`statusLabel` render `Gone` as a clearly-stopped state (not blank). If `Gone` falls through to an empty/`undefined` class or label, add a `gone` case that styles/labels it as "stopped" — matching the existing `exited` styling. Make this minimal and only if needed.

- [ ] **Step 4: Typecheck + build**

From `ui/`: `./node_modules/.bin/tsc --noEmit` and `./node_modules/.bin/vite build`.
Expected: both clean.

- [ ] **Step 5: Manual GUI verification**

Run `./dev.sh`. Create a couple of agent runs (e.g. claude and cursor) and a terminal; let them produce output. Quit from the menu bar (stops everything), relaunch with `./dev.sh`. Expected:
- The runs appear in the list marked stopped (not running).
- Clicking the **claude** run resumes it — the prior conversation comes back (Claude `--continue` in its worktree).
- Clicking the **cursor** run starts a fresh cursor session (no resume).
- Clicking the **terminal** opens a fresh shell in the project root.
- No pane is a dead blank cursor.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/FocusTerminal.tsx
git commit -m "feat(resume): reactivate a stopped run on focus before attaching"
```

---

## Self-Review notes

- **Spec coverage:** resume_args field + migration + seeds (Tasks 1-2); `ensure_run_active` resume/fresh/shell (Task 3); command + binding (Task 4); resume-on-focus + status indicator (Task 5). The spec's "restore worktree if missing" is intentionally descoped per Global Constraints (rerun has no such path today; worktrees persist across quit) — a missing worktree surfaces as a spawn error.
- **Out of scope (unchanged):** per-run session-id assignment for cursor/hermes precise resume; daemon-crash live-terminal-freeze surfacing; transcript replay; bulk resume-all.
- **Type consistency:** `agent_argv(profile, prompt, use_resume, setup) -> (String, Vec<String>)`, `ensure_run_active(&self, id: &str) -> Result<()>`, `ensure_profile_resume_args(&self, name, &Option<Vec<String>>)`, `resume_args: Option<Vec<String>>`, recipe values, and the `ensureRunActive(id)` binding are used consistently across tasks.
