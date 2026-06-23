# Workspace Lifecycle — Plan 4: Notifications

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fire desktop notifications when an agent finishes, goes idle (waiting on you), a run script crashes, or a merge hits conflicts — suppressed for the run you're actively viewing and (optionally) when the app is focused.

**Architecture:** A background watcher thread in `agency-app` polls every 2s, snapshots each run's agent/run-script session status plus a hash of the agent pane (for idle detection), and feeds them to a **pure** `notifier::step` function that detects edge transitions and latches idle. The watcher applies per-event toggles + focus/active-run suppression, then shows OS notifications via `tauri-plugin-notification`. Merge-conflict notifications fire from the `merge_task` command path. The frontend reports window focus + the focused run to the backend.

**Tech Stack:** Rust (agency-app/Tauri, std threads), `tauri-plugin-notification` (new), `serde_json`, React/TS.

**Spec:** `docs/superpowers/specs/2026-06-22-workspace-lifecycle-design.md` (Feature 5 — Notifications). **Builds on Plans 1–3** (merged).

## Global Constraints

- Rust edition 2021. No AI/Claude attribution in any commit message (hard rule).
- Four events: **Finished** (agent session running→exited), **Idle** (agent running + pane unchanged ≥ `idle_secs`, default 30s, fired once per idle stretch), **RunCrashed** (run-script session running→exited with non-zero code), **MergeConflict** (`merge_task` returns conflicts).
- Suppression: an event is shown only if its per-event toggle is on AND not `(only_when_unfocused && app_focused)` AND the run is not the actively-viewed run. (MergeConflict respects the toggle + `only_when_unfocused` only — it is a discrete foreground action, so active-run suppression does not apply to it.)
- Defaults: all four event toggles ON, `only_when_unfocused` ON, `idle_secs` 30. Stored as JSON under the SQLite settings key `notification_settings`.
- The edge-detection/idle-latch logic lives in a pure, unit-tested `notifier::step`. The thread is the only impure part.
- Poll interval 2s. Idle is computed in whole poll ticks: idle when `(now_tick - quiet_since_tick) * poll_secs >= idle_secs`.

## File Structure

**Backend**
- **Create** `crates/agency-app/src/notifier.rs` — `NotifSettings`, `RunSnapshot`, `RunWatch`, `NotifyKind`, pure `step`, `message`.
- **Modify** `crates/agency-app/src/state.rs` — `UiState` + `set_ui_state`/`ui_snapshot`; `notif_settings`/`save_notif_settings`; `watch_snapshot`.
- **Modify** `crates/agency-app/src/commands.rs` — `set_ui_state`, `get_notif_settings`, `save_notif_settings`; merge-conflict notify in `merge_task`.
- **Modify** `crates/agency-app/src/lib.rs` — declare `notifier` module; init notification plugin; register 3 commands; spawn the watcher thread.
- **Modify** `crates/agency-app/Cargo.toml` — `tauri-plugin-notification = "2"`.
- **Modify** `crates/agency-app/capabilities/default.json` — add `notification:default`.

**Frontend**
- **Modify** `ui/src/api.ts` — `NotifSettings` type; `setUiState`, `getNotifSettings`, `saveNotifSettings`.
- **Modify** `ui/src/App.tsx` — report focus + focused run via `setUiState`.
- **Modify** `ui/src/components/Settings.tsx` — Notifications section (toggles + idle secs).

---

### Task 1: Notifier core (pure edge detection + settings)

**Files:**
- Create: `crates/agency-app/src/notifier.rs`
- Modify: `crates/agency-app/src/lib.rs` (add `mod notifier;`)

**Interfaces:**
- Produces:
  - `NotifSettings { agent_finished, agent_idle, run_crashed, merge_attention, only_when_unfocused: bool, idle_secs: u64 }` (serde camelCase; `Default` = all bools true, idle_secs 30)
  - `RunSnapshot { id, label: String, agent: SessionStatus, run_script: SessionStatus, pane_hash: u64 }`
  - `RunWatch { agent, run_script: SessionStatus, pane_hash: u64, quiet_since_tick: u64, idle_fired: bool }` (Clone)
  - `enum NotifyKind { Finished, RunCrashed, Idle }`
  - `fn step(prev: Option<&RunWatch>, snap: &RunSnapshot, now_tick: u64, poll_secs: u64, idle_secs: u64) -> (RunWatch, Vec<NotifyKind>)`
  - `fn message(kind: &NotifyKind, label: &str) -> (String, String)`

- [ ] **Step 1: Declare the module**

In `crates/agency-app/src/lib.rs`, add near the top with the other `mod` lines:

```rust
mod notifier;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/agency-app/src/notifier.rs` with the tests first (implementation added in Step 3):

```rust
use agency_core::tmux::SessionStatus;
use serde::{Deserialize, Serialize};

// (types + step + message go here in Step 3)

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(agent: SessionStatus, run_script: SessionStatus, pane_hash: u64) -> RunSnapshot {
        RunSnapshot { id: "x".into(), label: "claude: fix".into(), agent, run_script, pane_hash }
    }
    fn running() -> SessionStatus { SessionStatus::Running }
    fn exited(code: i32) -> SessionStatus { SessionStatus::Exited { code } }

    #[test]
    fn settings_default_is_all_on_idle_30() {
        let s = NotifSettings::default();
        assert!(s.agent_finished && s.agent_idle && s.run_crashed && s.merge_attention && s.only_when_unfocused);
        assert_eq!(s.idle_secs, 30);
    }

    #[test]
    fn empty_json_deserializes_to_defaults() {
        let s: NotifSettings = serde_json::from_str("{}").unwrap();
        assert!(s.agent_finished);
        assert_eq!(s.idle_secs, 30);
    }

    #[test]
    fn first_observation_emits_nothing() {
        let (_w, events) = step(None, &snap(running(), SessionStatus::Gone, 1), 0, 2, 30);
        assert!(events.is_empty());
    }

    #[test]
    fn agent_running_to_exited_emits_finished() {
        let (prev, _) = step(None, &snap(running(), SessionStatus::Gone, 1), 0, 2, 30);
        let (_w, events) = step(Some(&prev), &snap(exited(0), SessionStatus::Gone, 1), 1, 2, 30);
        assert_eq!(events, vec![NotifyKind::Finished]);
    }

    #[test]
    fn run_script_nonzero_exit_emits_crash_but_zero_does_not() {
        let (p1, _) = step(None, &snap(running(), running(), 1), 0, 2, 30);
        let (_w, events) = step(Some(&p1), &snap(running(), exited(1), 1), 1, 2, 30);
        assert_eq!(events, vec![NotifyKind::RunCrashed]);

        let (p2, _) = step(None, &snap(running(), running(), 1), 0, 2, 30);
        let (_w2, events2) = step(Some(&p2), &snap(running(), exited(0), 1), 1, 2, 30);
        assert!(events2.is_empty());
    }

    #[test]
    fn idle_fires_once_after_threshold_then_resets_on_activity() {
        // tick 0: first obs, pane_hash 1, quiet_since 0
        let (mut w, _) = step(None, &snap(running(), SessionStatus::Gone, 1), 0, 2, 30);
        // ticks 1..=14 same pane → at tick 15, 15*2=30 >= 30 → Idle
        let mut fired_at = None;
        for t in 1..=15 {
            let (nw, ev) = step(Some(&w), &snap(running(), SessionStatus::Gone, 1), t, 2, 30);
            w = nw;
            if ev.contains(&NotifyKind::Idle) { fired_at = Some(t); }
        }
        assert_eq!(fired_at, Some(15));
        // already idle → no repeat next tick
        let (w2, ev2) = step(Some(&w), &snap(running(), SessionStatus::Gone, 1), 16, 2, 30);
        assert!(ev2.is_empty());
        // pane changes → idle latch clears, quiet resets
        let (w3, ev3) = step(Some(&w2), &snap(running(), SessionStatus::Gone, 999), 17, 2, 30);
        assert!(ev3.is_empty());
        assert!(!w3.idle_fired);
        assert_eq!(w3.quiet_since_tick, 17);
    }

    #[test]
    fn exited_agent_does_not_go_idle() {
        let (p, _) = step(None, &snap(exited(0), SessionStatus::Gone, 1), 0, 2, 30);
        let (_w, ev) = step(Some(&p), &snap(exited(0), SessionStatus::Gone, 1), 100, 2, 30);
        assert!(!ev.contains(&NotifyKind::Idle));
    }

    #[test]
    fn message_uses_label() {
        let (title, body) = message(&NotifyKind::Finished, "claude: fix login");
        assert!(!title.is_empty());
        assert!(body.contains("claude: fix login"));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p agency-app notifier::`
Expected: FAIL to compile — types/functions undefined.

- [ ] **Step 4: Implement the module**

Add above the `#[cfg(test)]` block in `notifier.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifSettings {
    #[serde(default = "d_true")]
    pub agent_finished: bool,
    #[serde(default = "d_true")]
    pub agent_idle: bool,
    #[serde(default = "d_true")]
    pub run_crashed: bool,
    #[serde(default = "d_true")]
    pub merge_attention: bool,
    #[serde(default = "d_true")]
    pub only_when_unfocused: bool,
    #[serde(default = "d_idle")]
    pub idle_secs: u64,
}

fn d_true() -> bool { true }
fn d_idle() -> u64 { 30 }

impl Default for NotifSettings {
    fn default() -> Self {
        NotifSettings {
            agent_finished: true,
            agent_idle: true,
            run_crashed: true,
            merge_attention: true,
            only_when_unfocused: true,
            idle_secs: 30,
        }
    }
}

pub struct RunSnapshot {
    pub id: String,
    pub label: String,
    pub agent: SessionStatus,
    pub run_script: SessionStatus,
    pub pane_hash: u64,
}

#[derive(Clone)]
pub struct RunWatch {
    pub agent: SessionStatus,
    pub run_script: SessionStatus,
    pub pane_hash: u64,
    pub quiet_since_tick: u64,
    pub idle_fired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifyKind {
    Finished,
    RunCrashed,
    Idle,
}

/// Pure edge detector. Given the previous watch state (if any) and a fresh
/// snapshot at `now_tick`, return the updated watch and any notification events.
pub fn step(
    prev: Option<&RunWatch>,
    snap: &RunSnapshot,
    now_tick: u64,
    poll_secs: u64,
    idle_secs: u64,
) -> (RunWatch, Vec<NotifyKind>) {
    let mut events = Vec::new();

    // Pane-change tracking for idle.
    let pane_changed = prev.map(|p| p.pane_hash != snap.pane_hash).unwrap_or(true);
    let quiet_since_tick = if pane_changed { now_tick } else { prev.unwrap().quiet_since_tick };

    if let Some(p) = prev {
        // Agent finished: running -> exited.
        if matches!(p.agent, SessionStatus::Running) {
            if let SessionStatus::Exited { .. } = snap.agent {
                events.push(NotifyKind::Finished);
            }
        }
        // Run script crashed: running -> exited non-zero.
        if matches!(p.run_script, SessionStatus::Running) {
            if let SessionStatus::Exited { code } = snap.run_script {
                if code != 0 {
                    events.push(NotifyKind::RunCrashed);
                }
            }
        }
    }

    // Idle latch.
    let agent_running = matches!(snap.agent, SessionStatus::Running);
    let mut idle_fired = prev.map(|p| p.idle_fired).unwrap_or(false);
    if !agent_running || pane_changed {
        idle_fired = false;
    }
    if agent_running && !idle_fired {
        let quiet_ticks = now_tick.saturating_sub(quiet_since_tick);
        if quiet_ticks.saturating_mul(poll_secs) >= idle_secs {
            events.push(NotifyKind::Idle);
            idle_fired = true;
        }
    }

    let watch = RunWatch {
        agent: snap.agent.clone(),
        run_script: snap.run_script.clone(),
        pane_hash: snap.pane_hash,
        quiet_since_tick,
        idle_fired,
    };
    (watch, events)
}

/// Notification (title, body) for an event about the run labelled `label`.
pub fn message(kind: &NotifyKind, label: &str) -> (String, String) {
    match kind {
        NotifyKind::Finished => ("Agent finished".to_string(), format!("{label} — done")),
        NotifyKind::RunCrashed => ("Run script crashed".to_string(), format!("{label} — dev server exited")),
        NotifyKind::Idle => ("Agent needs you".to_string(), format!("{label} — waiting for input")),
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p agency-app notifier::`
Expected: PASS (9 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/agency-app/src/notifier.rs crates/agency-app/src/lib.rs
git commit -m "Add notifier module: settings and pure edge detection"
```

---

### Task 2: AppState — UI state, settings, and watch snapshots

**Files:**
- Modify: `crates/agency-app/src/state.rs`

**Interfaces:**
- Consumes: `notifier::{NotifSettings, RunSnapshot}` (Task 1).
- Produces (on `AppState`):
  - `set_ui_state(&self, focused: bool, active_run: Option<String>)`
  - `ui_snapshot(&self) -> (bool, Option<String>)`
  - `notif_settings(&self) -> Result<NotifSettings>`
  - `save_notif_settings(&self, s: &NotifSettings) -> Result<()>`
  - `watch_snapshot(&self) -> Result<Vec<RunSnapshot>>`

- [ ] **Step 1: Add the UiState field**

Near the top of `state.rs`, add a settings-key constant alongside the others:

```rust
const SETTING_NOTIF: &str = "notification_settings";
```

Add a small struct above `AppState`:

```rust
#[derive(Default)]
struct UiState {
    focused: bool,
    active_run: Option<String>,
}
```

Add a field to `AppState`:

```rust
    ui: Mutex<UiState>,
```

In `AppState::new`, initialise it assuming the window starts focused (so we don't notify before the UI first reports):

```rust
            ui: Mutex::new(UiState { focused: true, active_run: None }),
```

- [ ] **Step 2: Add the methods**

Add to `impl AppState`:

```rust
    pub fn set_ui_state(&self, focused: bool, active_run: Option<String>) {
        let mut ui = self.ui.lock().unwrap();
        ui.focused = focused;
        ui.active_run = active_run;
    }

    pub fn ui_snapshot(&self) -> (bool, Option<String>) {
        let ui = self.ui.lock().unwrap();
        (ui.focused, ui.active_run.clone())
    }

    pub fn notif_settings(&self) -> Result<notifier::NotifSettings> {
        let raw = self.registry.lock().unwrap().get_setting(SETTING_NOTIF)?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    pub fn save_notif_settings(&self, s: &notifier::NotifSettings) -> Result<()> {
        let json = serde_json::to_string(s)?;
        self.registry.lock().unwrap().set_setting(SETTING_NOTIF, &json)
    }

    /// Snapshot every non-archived run across all projects for the watcher:
    /// agent + run-script session status and a hash of the agent pane (for idle).
    pub fn watch_snapshot(&self) -> Result<Vec<notifier::RunSnapshot>> {
        let projects = self.registry.lock().unwrap().list_projects()?;
        let mut out = Vec::new();
        for proj in projects {
            let runs = self.registry.lock().unwrap().list_runs(&proj.id)?;
            for run in runs {
                let agent = self.tmux.session_status(&session_name(&run.id)).unwrap_or(SessionStatus::Gone);
                let run_script = self.tmux.session_status(&run_session_name(&run.id)).unwrap_or(SessionStatus::Gone);
                let pane = self.tmux.capture(&session_name(&run.id), 50).unwrap_or_default();
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                std::hash::Hash::hash(&pane, &mut hasher);
                let pane_hash = std::hash::Hasher::finish(&hasher);
                let label = format!(
                    "{}: {}",
                    run.agent,
                    if run.prompt.is_empty() { run.branch.clone() } else { run.prompt.clone() }
                );
                out.push(notifier::RunSnapshot { id: run.id, label, agent, run_script, pane_hash });
            }
        }
        Ok(out)
    }
```

Add `use crate::notifier;` near the top of `state.rs` if the module path isn't already in scope (it is a sibling module; reference as `crate::notifier` or add the `use`).

- [ ] **Step 3: Build + test**

Run: `cargo build && cargo test -p agency-app`
Expected: compiles; existing tests still pass.

- [ ] **Step 4: Commit**

```bash
git add crates/agency-app/src/state.rs
git commit -m "Add UI state, notification settings, and watch snapshots to AppState"
```

---

### Task 3: Notification plugin + watcher thread + commands

**Files:**
- Modify: `crates/agency-app/Cargo.toml`
- Modify: `crates/agency-app/capabilities/default.json`
- Modify: `crates/agency-app/src/commands.rs`
- Modify: `crates/agency-app/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 (`notifier::{step, message, NotifSettings, RunWatch}`), Task 2 (`AppState` methods).
- Produces: commands `set_ui_state`, `get_notif_settings`, `save_notif_settings`; a background watcher thread.

- [ ] **Step 1: Add the dependency**

In `crates/agency-app/Cargo.toml`, under `[dependencies]` after `tauri-plugin-opener = "2"`:

```toml
tauri-plugin-notification = "2"
```

- [ ] **Step 2: Grant the permission**

In `crates/agency-app/capabilities/default.json`, add `"notification:default"` to `permissions`:

```json
  "permissions": ["core:default", "dialog:allow-open", "opener:allow-open-url", "notification:default"]
```

- [ ] **Step 3: Add the commands**

Append to `crates/agency-app/src/commands.rs`:

```rust
use crate::notifier::NotifSettings;

#[tauri::command]
pub fn set_ui_state(state: State<'_, AppState>, focused: bool, active_run: Option<String>) {
    state.set_ui_state(focused, active_run);
}

#[tauri::command]
pub fn get_notif_settings(state: State<'_, AppState>) -> Result<NotifSettings, String> {
    state.notif_settings().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_notif_settings(state: State<'_, AppState>, settings: NotifSettings) -> Result<(), String> {
    state.save_notif_settings(&settings).map_err(|e| e.to_string())
}
```

- [ ] **Step 4: Init plugin, register commands, spawn the watcher**

In `crates/agency-app/src/lib.rs`:

Add the plugin after the opener plugin:

```rust
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
```

Register the three commands in the `generate_handler!` list (after `commands::list_archived_runs,`):

```rust
            commands::set_ui_state,
            commands::get_notif_settings,
            commands::save_notif_settings,
```

In the `.setup(|app| { … })` closure, after `app.manage(state);`, spawn the watcher (keep the existing `Ok(())` at the end):

```rust
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                use std::collections::{HashMap, HashSet};
                use tauri::Manager;
                use tauri_plugin_notification::NotificationExt;

                let poll_secs: u64 = 2;
                let mut watches: HashMap<String, crate::notifier::RunWatch> = HashMap::new();
                let mut tick: u64 = 0;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(poll_secs));
                    tick += 1;
                    let state = handle.state::<AppState>();
                    let settings = state.notif_settings().unwrap_or_default();
                    let (focused, active) = state.ui_snapshot();
                    let snaps = match state.watch_snapshot() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    let mut seen: HashSet<String> = HashSet::new();
                    for snap in &snaps {
                        seen.insert(snap.id.clone());
                        let (watch, events) =
                            crate::notifier::step(watches.get(&snap.id), snap, tick, poll_secs, settings.idle_secs);
                        for ev in &events {
                            let enabled = match ev {
                                crate::notifier::NotifyKind::Finished => settings.agent_finished,
                                crate::notifier::NotifyKind::RunCrashed => settings.run_crashed,
                                crate::notifier::NotifyKind::Idle => settings.agent_idle,
                            };
                            let suppressed = (settings.only_when_unfocused && focused)
                                || active.as_deref() == Some(snap.id.as_str());
                            if enabled && !suppressed {
                                let (title, body) = crate::notifier::message(ev, &snap.label);
                                let _ = handle.notification().builder().title(title).body(body).show();
                            }
                        }
                        watches.insert(snap.id.clone(), watch);
                    }
                    watches.retain(|id, _| seen.contains(id));
                }
            });
```

- [ ] **Step 5: Build**

Run: `cargo build`
Expected: compiles; `tauri-plugin-notification` resolves; handler list valid.

- [ ] **Step 6: Commit**

```bash
git add crates/agency-app/Cargo.toml crates/agency-app/capabilities/default.json crates/agency-app/src/commands.rs crates/agency-app/src/lib.rs Cargo.lock
git commit -m "Spawn notification watcher thread and expose its settings commands"
```

---

### Task 4: Merge-conflict notification

**Files:**
- Modify: `crates/agency-app/src/commands.rs`

**Interfaces:**
- Consumes: `AppState::{merge_task, notif_settings, ui_snapshot}`; `notifier`; the `MergeOutcome` enum.
- Produces: the `merge_task` command now also shows a notification on conflicts.

- [ ] **Step 1: Update the `merge_task` command**

Find the existing `merge_task` command in `commands.rs`. It currently looks like:

```rust
#[tauri::command]
pub fn merge_task(state: State<'_, AppState>, task_id: String) -> Result<MergeOutcome, String> {
    state.merge_task(&task_id).map_err(|e| e.to_string())
}
```

Replace it with a version that also takes the `AppHandle` and notifies on conflicts (respecting the toggle + `only_when_unfocused`, but NOT active-run suppression — merge is a deliberate foreground action the user should be told about even on the focused run):

```rust
#[tauri::command]
pub fn merge_task(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<MergeOutcome, String> {
    let outcome = state.merge_task(&task_id).map_err(|e| e.to_string())?;
    if let MergeOutcome::Conflicts { files } = &outcome {
        let settings = state.notif_settings().unwrap_or_default();
        let (focused, _active) = state.ui_snapshot();
        if settings.merge_attention && !(settings.only_when_unfocused && focused) {
            use tauri_plugin_notification::NotificationExt;
            let _ = app
                .notification()
                .builder()
                .title("Merge needs attention")
                .body(format!("{} file(s) conflict", files.len()))
                .show();
        }
    }
    Ok(outcome)
}
```

(Confirm the `MergeOutcome::Conflicts` variant field is named `files` — it is, per `ui/src/api.ts`'s `{ kind: "conflicts"; files: string[] }`. If the Rust variant differs, match its actual shape.)

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: compiles. (Tauri allows a command to take both `AppHandle` and `State`.)

- [ ] **Step 3: Commit**

```bash
git add crates/agency-app/src/commands.rs
git commit -m "Notify on merge conflicts from the merge command"
```

---

### Task 5: Frontend — focus reporting + notification settings UI

**Files:**
- Modify: `ui/src/api.ts`
- Modify: `ui/src/App.tsx`
- Modify: `ui/src/components/Settings.tsx`

**Interfaces:**
- Consumes: Task 3 commands.
- Produces: `NotifSettings` type; `setUiState`, `getNotifSettings`, `saveNotifSettings`; focus reporting; a Notifications settings section.

- [ ] **Step 1: Add api wrappers**

In `ui/src/api.ts`, add:

```ts
export interface NotifSettings {
  agentFinished: boolean;
  agentIdle: boolean;
  runCrashed: boolean;
  mergeAttention: boolean;
  onlyWhenUnfocused: boolean;
  idleSecs: number;
}

export const setUiState = (focused: boolean, activeRun: string | null) =>
  invoke<void>("set_ui_state", { focused, activeRun });
export const getNotifSettings = () => invoke<NotifSettings>("get_notif_settings");
export const saveNotifSettings = (settings: NotifSettings) =>
  invoke<void>("save_notif_settings", { settings });
```

- [ ] **Step 2: Report focus + focused run from the Shell**

In `ui/src/App.tsx`:

Add to the imports:

```ts
import { useEffect } from "react";
import { Project, setUiState } from "./api";
```

(Merge with the existing `import { useState } from "react";` → `import { useEffect, useState } from "react";`, and extend the existing `import { Project } from "./api";` to include `setUiState`.)

In `Shell`, after the `useShortcuts({ … })` call, add:

```ts
  useEffect(() => {
    const report = () => setUiState(document.hasFocus(), focusedRunId).catch(() => {});
    report();
    window.addEventListener("focus", report);
    window.addEventListener("blur", report);
    return () => {
      window.removeEventListener("focus", report);
      window.removeEventListener("blur", report);
    };
  }, [focusedRunId]);
```

- [ ] **Step 3: Add the Notifications settings section**

In `ui/src/components/Settings.tsx`:

Extend the api import to add the notification helpers and type:

```ts
import {
  AgentProfile,
  ProviderSettings,
  NotifSettings,
  deleteProfile,
  getSettings,
  getNotifSettings,
  listProfiles,
  saveProfile,
  saveSettings,
  saveNotifSettings,
} from "../api";
```

Add state + a default near the other `useState` calls:

```ts
  const [notif, setNotif] = useState<NotifSettings>({
    agentFinished: true,
    agentIdle: true,
    runCrashed: true,
    mergeAttention: true,
    onlyWhenUnfocused: true,
    idleSecs: 30,
  });
```

In `refresh`, after `setProfiles(await listProfiles());`, add:

```ts
      setNotif(await getNotifSettings());
```

Add a persist function next to `persistSettings`:

```ts
  async function persistNotif(next: NotifSettings) {
    setNotif(next);
    try {
      await saveNotifSettings(next);
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }
```

Add a new `<section>` after the Model providers section (before the closing `</div>`s):

```tsx
        <section className="settings-section">
          <div className="settings-section-label">Notifications</div>
          <div className="settings-notif">
            {([
              ["agentFinished", "Agent finished"],
              ["agentIdle", "Agent needs input / idle"],
              ["runCrashed", "Run script crashed"],
              ["mergeAttention", "Merge needs attention"],
              ["onlyWhenUnfocused", "Only when app is not focused"],
            ] as [keyof NotifSettings, string][]).map(([key, label]) => (
              <label key={key} className="settings-notif-row">
                <input
                  type="checkbox"
                  checked={notif[key] as boolean}
                  onChange={(e) => persistNotif({ ...notif, [key]: e.target.checked })}
                />
                {label}
              </label>
            ))}
            <label className="settings-notif-row">
              Idle after (seconds)
              <input
                className="settings-input settings-notif-secs"
                type="number"
                min={5}
                value={notif.idleSecs}
                onChange={(e) => persistNotif({ ...notif, idleSecs: Number(e.target.value) || 30 })}
              />
            </label>
          </div>
        </section>
```

Append minimal styles to `ui/src/styles.css`:

```css
.settings-notif { display: flex; flex-direction: column; gap: 6px; }
.settings-notif-row { display: flex; align-items: center; gap: 8px; color: var(--fg, #e6e6e6); }
.settings-notif-secs { width: 80px; }
```

- [ ] **Step 4: Typecheck + build**

Run: `cd ui && pnpm exec tsc --noEmit && pnpm build`
Expected: no type errors; build succeeds.

- [ ] **Step 5: Manual smoke test**

1. Spawn an agent, switch focus to another app (or another run), and let the agent finish → a "Agent finished" notification appears.
2. With an agent waiting at a prompt and the app unfocused, after ~30s → "Agent needs you".
3. Configure a `run` script that exits non-zero; start it → "Run script crashed".
4. Approve a run that conflicts on merge → "Merge needs attention".
5. While **viewing** the run that finishes, with the app focused → no notification (suppressed). Toggle "Only when app is not focused" off in Settings → finishing a non-viewed run notifies even when focused.

- [ ] **Step 6: Commit**

```bash
git add ui/src/api.ts ui/src/App.tsx ui/src/components/Settings.tsx ui/src/styles.css
git commit -m "Report focus to backend and add notification settings UI"
```

---

## Self-Review

**Spec coverage (Feature 5):**
- `tauri-plugin-notification` + watcher thread polling 2s → Task 3. ✓
- Finished / Idle (tmux-quiescence heuristic) / RunCrashed events → Tasks 1 (logic) + 3 (thread). ✓
- Merge needs attention → Task 4. ✓
- Per-event toggles + only-when-unfocused + suppress actively-viewed run → Tasks 1/3 (logic), 5 (UI + focus reporting). ✓
- Settings persisted (SQLite JSON) → Tasks 1/2 (`NotifSettings`, storage), 5 (UI). ✓
- Frontend reports window focus + active run → Task 5. ✓

**Placeholder scan:** none — every step carries concrete code/commands.

**Type consistency:** `NotifSettings` is one shape across Rust (`notifier.rs`, camelCase serde) and TS (`api.ts`): `agentFinished/agentIdle/runCrashed/mergeAttention/onlyWhenUnfocused/idleSecs`. `step`/`message`/`RunWatch`/`RunSnapshot`/`NotifyKind` defined in Task 1 are consumed by the watcher in Task 3. The 3 command names match across `commands.rs` (Task 3), `lib.rs` (Task 3), `api.ts` (Task 5): `set_ui_state`/`get_notif_settings`/`save_notif_settings`. `watch_snapshot`/`ui_snapshot`/`notif_settings` (Task 2) are consumed by the watcher (Task 3).

**Task dependencies:** 1→2 (types) → 3 (thread+commands) → 4 (merge notify reuses settings) → 5 (frontend). Sequential.

**Idle heuristic note (accepted, per spec):** a long quiet build with no agent output reads as "idle" after `idle_secs`; the threshold is user-configurable. Not a bug.

**Deferred:** resolver-agent-finished notifications (the resolver runs in-view in MergeModal); only the conflict-detected event is notified here.
