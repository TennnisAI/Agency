# Resume-Failure Fallback (Fresh Session in Same Pane) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When reactivating a stopped agent whose resume fails (nothing to continue), the daemon session automatically falls back to a fresh agent in the SAME worktree and pane — no dead panes.

**Architecture:** Generalize a daemon `Session` to "run a primary command; if it exits within a grace window with no input, respawn a fallback command into the same session (same emulator + subscribers)." `ensure_run_active` wires the resume recipe as primary and the fresh recipe as fallback for resume-capable agents.

**Tech Stack:** Rust (`agency-core` daemon: protocol/session/registry/server/client; `agency-app` state).

## Global Constraints

- Branch: `feat/termd`. Never touch `main`.
- `on_output` keeps holding `emu` + `subs` **strongly** (as today). `on_exit` holds a **`Weak<SpawnCtx>`** — this is the load-bearing cycle-break: a strong ref would make `ctx → pty slot → pty wait-thread → on_exit → ctx` a cycle and defeat kill-on-drop. On fire, `on_exit` upgrades the `Weak`; if it fails (session dropped), it does nothing.
- Lock order `emu → subs` is preserved unchanged in `on_output` and `subscribe`. The `pty` slot mutex is always taken alone (never while holding `emu` or `subs`).
- The fallback fires **at most once**: the fallback spawn is passed `None` as its own fallback, so a genuinely broken agent does not loop — its exit fans the normal `Exited` frame.
- `input_seen` is set on **non-empty** input only (so `kill()`'s empty write does not count) and suppresses the fallback when set.
- `ClientMsg::StartSession` gains `#[serde(default)] fallback: Option<FallbackSpec>` (additive). `FallbackSpec { command: String, args: Vec<String>, grace_ms: u64 }`. The daemon maps it to `Fallback { command, args, grace: Duration::from_millis(grace_ms) }`.
- `ensure_run_active`: an agent **with** `resume_args` → primary = resume argv, fallback = fresh argv (`agent_argv(profile, prompt, false, setup)`), `grace_ms = 3000`. cursor/hermes (no `resume_args`) and terminals → no fallback. `create_run`/`rerun` unchanged.
- Run from `<home>/agency`: `cargo test -p agency-core ...`, `cargo test -p agency-app ...`, `cargo build`.

---

## File Structure

- `crates/agency-core/src/term/protocol.rs` — `FallbackSpec` struct + `fallback` field on `StartSession`.
- `crates/agency-core/src/term/session.rs` — `Fallback` type; `Session::start` gains `fallback` param; the `SpawnCtx`/`spawn_into`/`Weak` rewrite implementing early-exit fallback.
- `crates/agency-core/src/term/registry.rs` — `Registry::start` threads `Option<Fallback>`.
- `crates/agency-core/src/term/server.rs` — dispatch maps `FallbackSpec` → `Fallback`.
- `crates/agency-core/src/term/client.rs` — `start_session_with_fallback`; `start_session` delegates with `None`.
- `crates/agency-core/tests/termd.rs` — e2e fallback test.
- `crates/agency-app/src/state.rs` — `ensure_run_active` wires the fallback.
- `crates/agency-app/tests/state.rs` — integration test.

---

### Task 1: Thread `fallback` through the pipe (inert)

**Files:**
- Modify: `crates/agency-core/src/term/protocol.rs`
- Modify: `crates/agency-core/src/term/session.rs` (add `Fallback` type; `Session::start` accepts but IGNORES `fallback` for now)
- Modify: `crates/agency-core/src/term/registry.rs`
- Modify: `crates/agency-core/src/term/server.rs`
- Modify: `crates/agency-core/src/term/client.rs`
- Test: `protocol.rs` inline test

**Interfaces:**
- Produces:
  - `pub struct FallbackSpec { pub command: String, pub args: Vec<String>, pub grace_ms: u64 }` (protocol.rs, serde)
  - `ClientMsg::StartSession { ..., fallback: Option<FallbackSpec> }`
  - `pub struct Fallback { pub command: String, pub args: Vec<String>, pub grace: Duration }` (session.rs)
  - `Session::start(id, cwd, command, args, env, cols, rows, fallback: Option<Fallback>)`
  - `Registry::start(id, cwd, command, args, env, cols, rows, fallback: Option<Fallback>)`
  - `TermClient::start_session_with_fallback(id, cwd, command, args, env, cols, rows, fallback: Option<FallbackSpec>)`; `start_session(...)` delegates with `None`.

- [ ] **Step 1: Write the failing protocol test**

Add to the `#[cfg(test)] mod tests` in `crates/agency-core/src/term/protocol.rs`:

```rust
#[test]
fn start_session_carries_fallback() {
    let msg = ClientMsg::StartSession {
        id: "r".into(), cwd: "/tmp".into(), command: "claude".into(),
        args: vec!["--continue".into()], env: vec![], cols: 80, rows: 24,
        fallback: Some(FallbackSpec {
            command: "claude".into(), args: vec![], grace_ms: 3000,
        }),
    };
    let payload = encode_json(&msg);
    match decode_client(&payload).unwrap() {
        ClientFrame::Msg(ClientMsg::StartSession { fallback: Some(fb), .. }) => {
            assert_eq!((fb.command.as_str(), fb.grace_ms), ("claude", 3000));
        }
        other => panic!("wrong decode: {other:?}"),
    }
}

#[test]
fn start_session_fallback_defaults_to_none_when_absent() {
    // A JSON StartSession payload WITHOUT a `fallback` field must decode to None.
    let json = br#"{"StartSession":{"id":"r","cwd":"/tmp","command":"c","args":[],"env":[],"cols":80,"rows":24}}"#;
    let mut payload = vec![0u8]; // type 0 = JSON
    payload.extend_from_slice(json);
    match decode_client(&payload).unwrap() {
        ClientFrame::Msg(ClientMsg::StartSession { fallback: None, .. }) => {}
        other => panic!("expected fallback None, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core protocol::tests::start_session -- --nocapture`
Expected: FAIL to compile (`FallbackSpec` / `fallback` missing).

- [ ] **Step 3: Add `FallbackSpec` + the field**

In `protocol.rs` add the struct (near the other message types):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackSpec {
    pub command: String,
    pub args: Vec<String>,
    pub grace_ms: u64,
}
```

Add the field to the `StartSession` variant of `ClientMsg`:

```rust
    StartSession {
        id: String,
        cwd: String,
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cols: u16,
        rows: u16,
        #[serde(default)]
        fallback: Option<FallbackSpec>,
    },
```

- [ ] **Step 4: Add `Fallback` + accept (ignore) it in `Session::start`**

In `session.rs`, add the type (near the top, after the `Subs` alias) and import `Duration`:

```rust
use std::time::Duration;

/// A command to run in the same session if the primary exits within `grace`
/// before any input. Wired by `ensure_run_active` so a failed resume falls back
/// to a fresh agent. (Behavior implemented in Task 2; this task only threads it.)
#[derive(Clone)]
pub struct Fallback {
    pub command: String,
    pub args: Vec<String>,
    pub grace: Duration,
}
```

Change `Session::start`'s signature to accept it, ignoring it for now (prefix `_`):

```rust
    pub fn start(
        id: String,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cols: u16,
        rows: u16,
        _fallback: Option<Fallback>,
    ) -> Result<Arc<Session>> {
        // ...existing body unchanged...
    }
```

Update the two existing session unit tests (`subscribe_delivers_a_snapshot_first`, `input_is_echoed_to_subscribers_and_capture`) to pass `None` as the new last arg to `Session::start`.

- [ ] **Step 5: Thread through registry + server**

In `registry.rs`, `Registry::start` (add `use crate::term::session::Fallback;`):

```rust
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &self,
        id: String,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cols: u16,
        rows: u16,
        fallback: Option<Fallback>,
    ) -> Result<()> {
        let session = Session::start(id.clone(), cwd, command, args, env, cols, rows, fallback)?;
        self.sessions.lock().unwrap().insert(id, session);
        Ok(())
    }
```

Update any `registry.start(...)` calls in `registry.rs`'s own tests to pass `None`.

In `server.rs` `dispatch`, the `StartSession` arm — destructure `fallback` and map it (add `use crate::term::session::Fallback;` and `use std::time::Duration;` if needed):

```rust
        ClientFrame::Msg(ClientMsg::StartSession { id, cwd, command, args, env, cols, rows, fallback }) => {
            let fb = fallback.map(|f| Fallback {
                command: f.command,
                args: f.args,
                grace: Duration::from_millis(f.grace_ms),
            });
            match registry.start(id.clone(), Path::new(&cwd), &command, &args, &env, cols, rows, fb) {
                Ok(()) => reply(ServerMsg::Started { id }),
                Err(e) => reply(ServerMsg::Error { id: Some(id), message: e.to_string() }),
            }
        }
```

- [ ] **Step 6: Client `start_session_with_fallback`**

In `client.rs`, refactor `start_session` to delegate to a new method that carries the fallback. Replace the existing `start_session` with:

```rust
    pub fn start_session(
        &self,
        id: &str,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cols: u16,
        rows: u16,
    ) -> Result<()> {
        self.start_session_with_fallback(id, cwd, command, args, env, cols, rows, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_session_with_fallback(
        &self,
        id: &str,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cols: u16,
        rows: u16,
        fallback: Option<crate::term::protocol::FallbackSpec>,
    ) -> Result<()> {
        self.request(ClientMsg::StartSession {
            id: id.into(),
            cwd: cwd.to_string_lossy().into(),
            command: command.into(),
            args: args.to_vec(),
            env: env.to_vec(),
            cols,
            rows,
            fallback,
        })
        .map(|_| ())
    }
```

(If `client.rs` already `use crate::term::protocol::*;`, you may write `FallbackSpec` unqualified.)

- [ ] **Step 7: Run tests + build**

Run: `cargo test -p agency-core protocol -- --nocapture` (new tests pass), then `cargo test -p agency-core` (whole crate green — fallback is threaded but inert, so all existing behavior is unchanged), then `cargo build`.
Expected: all pass, clean build.

- [ ] **Step 8: Commit**

```bash
git add crates/agency-core/src/term/protocol.rs crates/agency-core/src/term/session.rs crates/agency-core/src/term/registry.rs crates/agency-core/src/term/server.rs crates/agency-core/src/term/client.rs
git commit -m "feat(resume): thread fallback through StartSession pipe (inert)"
```

---

### Task 2: Implement the Session early-exit fallback

**Files:**
- Modify: `crates/agency-core/src/term/session.rs` (the core rewrite + 3 unit tests)
- Test: `crates/agency-core/tests/termd.rs` (e2e fallback test)

**Interfaces:**
- Consumes: `Fallback` (from Task 1), `spawn_pty`, `Emulator`, `encode_output`/`encode_json`/`encode_snapshot`, `TermClient::start_session_with_fallback` (for the e2e test).
- Produces: `Session` whose primary, on exiting within `fallback.grace` with no input, respawns `fallback.command`/`args` into the same session.

- [ ] **Step 1: Write the failing unit tests**

Add to the `#[cfg(test)] mod tests` in `crates/agency-core/src/term/session.rs` (the module already has `use super::*;`, `mpsc`, `Duration`, `drain_until`, `decode_server`, `ServerFrame`):

```rust
#[test]
fn fallback_runs_in_same_session_when_primary_exits_fast() {
    let s = Session::start(
        "fb1".into(),
        std::env::temp_dir().as_path(),
        "/bin/sh",
        &["-c".into(), "printf NOPE; exit 1".into()],
        &[],
        80, 24,
        Some(Fallback {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "printf FRESH; sleep 3".into()],
            grace: Duration::from_secs(2),
        }),
    ).unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut cap = String::new();
    while std::time::Instant::now() < deadline {
        cap = s.capture(10);
        if cap.contains("FRESH") { break; }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(cap.contains("NOPE"), "primary output missing: {cap:?}");
    assert!(cap.contains("FRESH"), "fallback output missing: {cap:?}");
    assert!(matches!(s.status(), SessionStatus::Running), "fallback should be running");
}

#[test]
fn no_fallback_when_primary_keeps_running() {
    let s = Session::start(
        "fb2".into(),
        std::env::temp_dir().as_path(),
        "/bin/sh",
        &["-c".into(), "printf ALIVE; sleep 3".into()],
        &[],
        80, 24,
        Some(Fallback {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "printf SHOULD_NOT_RUN; sleep 3".into()],
            grace: Duration::from_secs(1),
        }),
    ).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    let cap = s.capture(10);
    assert!(cap.contains("ALIVE"));
    assert!(!cap.contains("SHOULD_NOT_RUN"), "fallback should not have run: {cap:?}");
    assert!(matches!(s.status(), SessionStatus::Running));
}

#[test]
fn exit_after_grace_emits_exited_and_no_fallback() {
    let (tx, rx) = mpsc::channel();
    let s = Session::start(
        "fb3".into(),
        std::env::temp_dir().as_path(),
        "/bin/sh",
        // stays alive long enough for subscribe to register, then exits past grace
        &["-c".into(), "printf BYE; sleep 1; exit 0".into()],
        &[],
        80, 24,
        Some(Fallback {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "printf SHOULD_NOT_RUN".into()],
            grace: Duration::from_millis(300),
        }),
    ).unwrap();
    s.subscribe(1, tx);
    let got = drain_until(&rx, |f| matches!(f, ServerFrame::Msg(ServerMsg::Exited { .. })));
    assert!(matches!(got, ServerFrame::Msg(ServerMsg::Exited { .. })));
    assert!(!s.capture(10).contains("SHOULD_NOT_RUN"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core session -- --nocapture`
Expected: FAIL — `fallback_runs_...` fails because the current `Session` ignores the fallback (no FRESH appears).

- [ ] **Step 3: Rewrite `session.rs` (non-test portion)**

Replace everything in `session.rs` above the `#[cfg(test)]` module with:

```rust
//! A single terminal session: a PTY child (with an optional early-exit
//! fallback), an emulator, and N subscribers.
use crate::term::emulator::Emulator;
use crate::term::protocol::{encode_json, encode_output, encode_snapshot, ServerMsg, SessionStatus};
use crate::term::pty::{spawn_pty, ProcStatus, PtyProcess};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

type Subs = Arc<Mutex<HashMap<u64, Sender<Vec<u8>>>>>;

/// A command to run in the same session if the primary exits within `grace`
/// before any input — so a failed resume falls back to a fresh agent.
#[derive(Clone)]
pub struct Fallback {
    pub command: String,
    pub args: Vec<String>,
    pub grace: Duration,
}

/// Respawn-able shared state. Held strongly by `Session`; the pty `on_exit`
/// closure holds only a `Weak` of this (see `spawn_into`) to avoid a cycle.
struct SpawnCtx {
    id: String,
    emu: Arc<Mutex<Emulator>>,
    subs: Subs,
    pty: Mutex<Option<PtyProcess>>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    cols: u16,
    rows: u16,
    input_seen: AtomicBool,
}

pub struct Session {
    ctx: Arc<SpawnCtx>,
}

/// Spawn `command` into the session. If `fallback` is set and the process exits
/// within `fallback.grace` with no input seen, the fallback command is spawned
/// into the SAME session (same emulator + subscribers), once.
fn spawn_into(
    ctx: &Arc<SpawnCtx>,
    command: String,
    args: Vec<String>,
    fallback: Option<Fallback>,
) -> Result<()> {
    let spawn_at = Instant::now();

    // on_output holds emu + subs STRONGLY (as before); neither references the
    // pty, so there is no cycle. Lock order is emu -> subs.
    let emu_o = ctx.emu.clone();
    let subs_o = ctx.subs.clone();
    let id_o = ctx.id.clone();
    let on_output = move |bytes: Vec<u8>| {
        let mut e = emu_o.lock().unwrap();
        e.feed(&bytes);
        let frame = encode_output(&id_o, &bytes);
        for tx in subs_o.lock().unwrap().values() {
            let _ = tx.send(frame.clone());
        }
    };

    // on_exit holds a WEAK ctx: a strong ref would form
    // ctx -> pty slot -> wait-thread -> on_exit -> ctx and defeat kill-on-drop.
    let weak: Weak<SpawnCtx> = Arc::downgrade(ctx);
    let on_exit = move |code: i32| {
        let Some(ctx) = weak.upgrade() else { return };
        if let Some(fb) = &fallback {
            if spawn_at.elapsed() < fb.grace && !ctx.input_seen.load(Ordering::SeqCst) {
                // Primary died fast with no input -> run the fallback once.
                let _ = spawn_into(&ctx, fb.command.clone(), fb.args.clone(), None);
                return;
            }
        }
        let frame = encode_json(&ServerMsg::Exited { id: ctx.id.clone(), code });
        for tx in ctx.subs.lock().unwrap().values() {
            let _ = tx.send(frame.clone());
        }
    };

    let pty = spawn_pty(&command, &args, &ctx.cwd, &ctx.env, ctx.cols, ctx.rows, on_output, on_exit)?;
    *ctx.pty.lock().unwrap() = Some(pty);
    Ok(())
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        id: String,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cols: u16,
        rows: u16,
        fallback: Option<Fallback>,
    ) -> Result<Arc<Session>> {
        let ctx = Arc::new(SpawnCtx {
            id: id.clone(),
            emu: Arc::new(Mutex::new(Emulator::new(cols, rows))),
            subs: Arc::new(Mutex::new(HashMap::new())),
            pty: Mutex::new(None),
            cwd: cwd.to_path_buf(),
            env: env.to_vec(),
            cols,
            rows,
            input_seen: AtomicBool::new(false),
        });
        spawn_into(&ctx, command.to_string(), args.to_vec(), fallback)?;
        Ok(Arc::new(Session { ctx }))
    }

    pub fn input(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.ctx.input_seen.store(true, Ordering::SeqCst);
        }
        if let Some(p) = self.ctx.pty.lock().unwrap().as_ref() {
            let _ = p.write_input(bytes);
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if let Some(p) = self.ctx.pty.lock().unwrap().as_ref() {
            let _ = p.resize(rows, cols);
        }
        self.ctx.emu.lock().unwrap().resize(cols, rows);
    }

    pub fn capture(&self, lines: usize) -> String {
        self.ctx.emu.lock().unwrap().capture(lines)
    }

    pub fn status(&self) -> SessionStatus {
        match self.ctx.pty.lock().unwrap().as_ref().map(|p| p.status()) {
            Some(ProcStatus::Running) | None => SessionStatus::Running,
            Some(ProcStatus::Exited(code)) => SessionStatus::Exited { code },
            Some(ProcStatus::Crashed) => SessionStatus::Exited { code: -1 },
        }
    }

    /// Register a subscriber and hand it the current screen as a snapshot,
    /// atomically: the emulator lock is held across snapshot + registration +
    /// snapshot-send so no output frame can jump ahead of the snapshot.
    pub fn subscribe(&self, client_id: u64, out: Sender<Vec<u8>>) {
        let emu = self.ctx.emu.lock().unwrap();
        let snap = emu.snapshot();
        let frame = encode_snapshot(&self.ctx.id, snap.cols, snap.rows, snap.cx, snap.cy, &snap.data);
        let mut subs = self.ctx.subs.lock().unwrap();
        let _ = out.send(frame);
        subs.insert(client_id, out);
    }

    pub fn unsubscribe(&self, client_id: u64) {
        self.ctx.subs.lock().unwrap().remove(&client_id);
    }

    pub fn kill(&self) {
        // Real teardown is dropping the Session (SpawnCtx -> pty slot -> child kill).
        if let Some(p) = self.ctx.pty.lock().unwrap().as_ref() {
            let _ = p.write_input(&[]);
        }
    }
}
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo test -p agency-core session -- --nocapture`
Expected: PASS — the 3 new fallback tests plus the 2 pre-existing tests (`subscribe_delivers_a_snapshot_first`, `input_is_echoed_to_subscribers_and_capture`).

- [ ] **Step 5: Add the e2e fallback test**

Append to `crates/agency-core/tests/termd.rs` (reuse its `server_and_client()` helper):

```rust
#[test]
fn start_session_with_fallback_runs_fresh_on_fast_primary_exit() {
    let (_dir, client) = server_and_client();
    client.start_session_with_fallback(
        "fb",
        std::env::temp_dir().as_path(),
        "/bin/sh",
        &["-c".into(), "printf NOPE; exit 1".into()],
        &[],
        80, 24,
        Some(agency_core::term::protocol::FallbackSpec {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "printf FRESH; sleep 3".into()],
            grace_ms: 2000,
        }),
    ).unwrap();

    let mut cap = String::new();
    for _ in 0..60 {
        cap = client.capture("fb", 10).unwrap();
        if cap.contains("FRESH") { break; }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(cap.contains("FRESH"), "fallback did not run through the daemon: {cap:?}");
    assert!(matches!(
        client.status("fb").unwrap(),
        agency_core::term::SessionStatus::Running
    ));
}
```

- [ ] **Step 6: Run the e2e test + whole crate**

Run: `cargo test -p agency-core --test termd -- --nocapture` then `cargo test -p agency-core`.
Expected: PASS (including the new e2e test) and the whole crate green.

- [ ] **Step 7: Commit**

```bash
git add crates/agency-core/src/term/session.rs crates/agency-core/tests/termd.rs
git commit -m "feat(resume): session early-exit fallback respawns fresh in same pane"
```

---

### Task 3: Wire `ensure_run_active` to use the fallback

**Files:**
- Modify: `crates/agency-app/src/state.rs` (the agent branch of `ensure_run_active`)
- Test: `crates/agency-app/tests/state.rs`

**Interfaces:**
- Consumes: `agent_argv`, `TermClient::start_session_with_fallback`, `agency_core::term::protocol::FallbackSpec`.

- [ ] **Step 1: Write the failing integration test**

Add to `crates/agency-app/tests/state.rs` (reuse `init_repo`, `AppState::new`, and the status-poll style; ensure `AgentProfile` + `SessionStatus` are imported as the file already does):

```rust
#[test]
fn ensure_run_active_falls_back_to_fresh_when_resume_fails() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let state = AppState::new(&dir.path().join("agency.db"), dir.path()).unwrap();
    // Fake agent: resume fails fast; fresh (render_args of `args`) prints FRESH and stays.
    state.register_profile(AgentProfile {
        name: "flaky".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "printf FRESH; sleep 5".into()],
        env: vec![],
        resume_args: Some(vec!["-c".into(), "printf NO-CONV; exit 1".into()]),
    }).unwrap();
    let project = state.add_project("demo", &repo).unwrap();
    let info = state.create_run(&project.id, "p", "flaky", "HEAD", None).unwrap();

    state.stop_run(&info.id).unwrap();
    let mut gone = false;
    for _ in 0..75 {
        if matches!(state.run_status(&info.id).unwrap(), SessionStatus::Gone) { gone = true; break; }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(gone, "session did not become Gone after stop_run");

    // Reactivate: resume (NO-CONV; exit 1) fails fast -> fallback fresh (FRESH; sleep 5).
    state.ensure_run_active(&info.id).unwrap();
    let mut fresh = false;
    for _ in 0..150 {
        let cap = state.run_preview(&info.id, 10).unwrap_or_default();
        if cap.contains("FRESH") && matches!(state.run_status(&info.id).unwrap(), SessionStatus::Running) {
            fresh = true; break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    assert!(fresh, "fallback fresh session did not come up");
    state.discard_run(&info.id).unwrap();
}
```

(If the public capture/preview method is not named `run_preview`, use the real one — check the methods used by the existing `tests/state.rs` and `api.ts`'s `runPreview` → its backing command.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-app --test state ensure_run_active_falls_back -- --nocapture`
Expected: FAIL — without the fallback, reactivation runs only the failing resume; the session goes Exited and `FRESH` never appears.

- [ ] **Step 3: Wire the fallback in `ensure_run_active`**

In `crates/agency-app/src/state.rs`, in `ensure_run_active`'s AGENT branch, replace the existing argv + `start_session` call. The current code is roughly:

```rust
    let (command, args) = agent_argv(&profile, &run.prompt, true, config.scripts.setup.as_deref());
    self.term.read().unwrap().start_session(
        &session_name(id), &worktree, &command, &args, &env, 220, 50,
    )?;
    Ok(())
```

Change it to compute a fallback (fresh argv) when the profile has a resume recipe:

```rust
    let setup = config.scripts.setup.as_deref();
    let (command, args) = agent_argv(&profile, &run.prompt, true, setup);
    let fallback = if profile.resume_args.is_some() {
        let (fresh_cmd, fresh_args) = agent_argv(&profile, &run.prompt, false, setup);
        Some(agency_core::term::protocol::FallbackSpec {
            command: fresh_cmd,
            args: fresh_args,
            grace_ms: 3000,
        })
    } else {
        None
    };
    self.term.read().unwrap().start_session_with_fallback(
        &session_name(id), &worktree, &command, &args, &env, 220, 50, fallback,
    )?;
    Ok(())
```

(The terminal branch and the early `Gone` guard are unchanged.)

- [ ] **Step 4: Run tests + build**

Run: `cargo test -p agency-app --test state -- --nocapture` then `cargo test -p agency-app` then `cargo build`.
Expected: PASS (new test + the rest) and clean build.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app/src/state.rs
git commit -m "feat(resume): ensure_run_active falls back to fresh on resume failure"
```

---

## Self-Review notes

- **Spec coverage:** protocol `FallbackSpec` + field (Task 1); `Session` early-exit behavior with `Weak` cycle-break + input_seen + once-only (Task 2); registry/server/client plumbing (Task 1); `ensure_run_active` resume-capable wiring at grace 3000ms (Task 3). Daemon unit tests + protocol round-trip + e2e + state integration all present.
- **Out of scope (unchanged):** cursor/hermes/terminal behavior; `create_run`/`rerun`; frontend (the pane updates through the existing subscription); a genuinely-broken-agent honest `Exited`.
- **Type consistency:** `Fallback { command, args, grace: Duration }` (daemon), `FallbackSpec { command, args, grace_ms }` (protocol/client/app), `Session::start(..., Option<Fallback>)`, `Registry::start(..., Option<Fallback>)`, `TermClient::start_session_with_fallback(..., Option<FallbackSpec>)`, and the `serde(default)` field are used consistently across tasks.
