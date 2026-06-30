# Resume-Failure Fallback: Fresh Session in the Same Pane — Design

**Date:** 2026-06-29
**Branch:** `feat/termd` (extends the resume-stopped-runs feature)

## Goal

**No dead panes.** Today, reactivating a stopped agent that never started a
conversation runs e.g. `claude --continue`, which prints "no conversation to
resume" and exits — leaving a dead pane the user can't use. When a resume fails,
the session must instead fall back to a **fresh** agent in the **same worktree and
pane**, so the message is shown and then a usable session appears.

## Background

`ensure_run_active` (added in the resume feature) respawns a `Gone` run. For a
resume-capable agent it launches the resume recipe (e.g. `claude --continue`) in
the run's worktree. That recipe fails when there is nothing to resume.

Key behavioral fact (verified): a **successful** resume leaves the interactive
agent running indefinitely; a **failed** resume exits within ~1s of launch, before
the user types anything. Exit codes vary by agent (claude exits non-zero), so the
robust, agent-agnostic signal is **"the process exited within a short grace window
and no input was sent"** — not the exit code.

The daemon `Session` (`crates/agency-core/src/term/session.rs`) currently owns one
`PtyProcess` for its lifetime; its `on_output`/`on_exit` closures capture only the
`emu`/`subs` Arcs (never the pty), so there is no reference cycle and kill-on-drop
works. The design must preserve that.

## Approach: daemon-level early-exit fallback

Generalize a session to "**run a primary command; if it exits within `grace` with
no input, run a fallback command in the same session.**" `ensure_run_active` wires
the resume recipe as primary and the fresh recipe as fallback.

This was chosen over a frontend "detect-exit-then-rerun" approach (re-attach
flicker, heuristic in TS, GUI-only testable) and a shell `||` chain (fragile across
agents' exit codes, messy quoting): the daemon approach gives the cleanest UX
(message then fresh, one pane, no flicker), is agent-agnostic, and is
backend-testable.

## Daemon `Session` changes

### Shared context + recursive spawn

Introduce a private shared context holding the pieces a respawn needs:

```
struct SpawnCtx {
    id: String,
    emu: Arc<Mutex<Emulator>>,
    subs: Subs,                                  // existing type alias
    pty: Arc<Mutex<Option<PtyProcess>>>,         // swappable slot
    cwd: PathBuf,
    env: Vec<(String, String)>,
    cols: u16,
    rows: u16,
    input_seen: Arc<AtomicBool>,
}

pub struct Fallback { pub command: String, pub args: Vec<String>, pub grace: Duration }
```

A private free function does the spawn and wires the fallback:

```
fn spawn_into(ctx: &Arc<SpawnCtx>, command: String, args: Vec<String>, fallback: Option<Fallback>) -> Result<()>
```

- Captures `spawn_at = Instant::now()`.
- `on_output`: holds `emu` + `subs` **strongly** (exactly as today) — feed emulator, fan `Output`. (No cycle: emu/subs do not hold the pty.)
- `on_exit`: holds a **`Weak<SpawnCtx>`** (this is the load-bearing cycle-break) plus the captured `fallback` and `spawn_at`. On fire:
  - `let Some(ctx) = weak.upgrade() else { return };` — if the session was dropped, do nothing.
  - If `fallback` is `Some`, `spawn_at.elapsed() < fallback.grace`, and `!ctx.input_seen.load(Relaxed)` → call `spawn_into(&ctx, fallback.command, fallback.args, None)` (fallback fires once; no further fallback → no loop) and `return`.
  - Otherwise fan the normal `Exited { id, code }` frame to `ctx.subs`.
- Spawns the `PtyProcess` via `spawn_pty(&command, &args, &ctx.cwd, &ctx.env, ctx.cols, ctx.rows, on_output, on_exit)` and stores it: `*ctx.pty.lock().unwrap() = Some(pty)`.

**Why `Weak`:** if `on_exit` held a strong `Arc<SpawnCtx>`, then `ctx → pty (slot) → pty wait-thread → on_exit → ctx` would be a cycle, so dropping the `Session` would never drop the pty and the child would never be killed. `on_exit` holding `Weak` breaks the cycle: the `Session` holds the only strong `SpawnCtx`; when it drops, `SpawnCtx` drops → the slot's `PtyProcess` drops → `killer.kill()` runs → child dies → threads end.

### Session struct + methods

```
pub struct Session { ctx: Arc<SpawnCtx> }
```

- `start(id, cwd, command, args, env, cols, rows, fallback: Option<Fallback>) -> Result<Arc<Session>>` — builds `SpawnCtx` (emulator sized cols×rows, empty subs, `pty: None`, `input_seen: false`), calls `spawn_into`, returns the `Session`.
- `input(&self, bytes)`: if `!bytes.is_empty()` set `ctx.input_seen` true (so the `kill()` empty-write does not count); then `if let Some(p) = ctx.pty.lock().unwrap().as_ref() { let _ = p.write_input(bytes); }`.
- `resize(&self, cols, rows)`: resize the current pty in the slot, then `ctx.emu.lock().resize(...)`.
- `status(&self)`: map the current slot pty's `ProcStatus` (None slot → treat as `Running`, since a spawn is in flight; in practice the slot is populated before `start` returns).
- `capture`/`subscribe`/`unsubscribe`: unchanged logic against `ctx.emu`/`ctx.subs` (same lock order emu→subs).
- `kill`: unchanged intent (real teardown is dropping the `Session`/`SpawnCtx`).

**Lock discipline:** `emu`→`subs` ordering in `on_output` and `subscribe` is preserved unchanged. The `pty` slot is a separate mutex always taken alone (never while holding emu or subs), so it cannot deadlock against them.

## Protocol / registry / server / client wiring

- **`protocol.rs`:** add to `ClientMsg::StartSession` a field `#[serde(default)] fallback: Option<FallbackSpec>` where `FallbackSpec { command: String, args: Vec<String>, grace_ms: u64 }`. Additive; existing senders/decoders unaffected.
- **`registry.rs`:** `Registry::start` gains a `fallback: Option<Fallback>` param, passed to `Session::start`.
- **`server.rs`:** `dispatch` maps the message's `FallbackSpec` → `Fallback { command, args, grace: Duration::from_millis(grace_ms) }` and passes it to `registry.start`.
- **`client.rs`:** add `TermClient::start_session_with_fallback(id, cwd, command, args, env, cols, rows, fallback: Option<FallbackSpec>)`. Existing `start_session(...)` delegates to it with `None` — the ~5 current call sites are unchanged.

## `ensure_run_active` wiring (`state.rs`)

For an agent run whose profile **has** `resume_args`:

```
let (command, args) = agent_argv(&profile, &run.prompt, true, setup);   // resume (as today)
let fallback = {
    let (fc, fa) = agent_argv(&profile, &run.prompt, false, setup);     // fresh
    Some(FallbackSpec { command: fc, args: fa, grace_ms: 3000 })
};
self.term.read().unwrap().start_session_with_fallback(&session_name(id), &worktree, &command, &args, &env, 220, 50, fallback)?;
```

For an agent **without** `resume_args` (cursor/hermes) and for terminals: unchanged
(`start_session`, no fallback). `create_run` and `rerun` are unchanged.

Grace = 3000 ms. A successful-but-fast-finishing resume is the only false positive
and merely yields a fresh session — acceptable.

## Error handling / edge cases

- Fallback fires **at most once** (the fallback spawn carries `None`), so a
  genuinely broken agent (e.g. missing binary) does not loop — its second exit fans
  the normal `Exited` frame and the pane shows the honest error. This is the only
  remaining "dead" case and it reflects a real broken agent, not the resume bug.
- If the user types during the grace window, `input_seen` suppresses the fallback
  (their input, their session).
- `Session` drop still kills the child via the slot's `PtyProcess::Drop` (the
  `Weak` ensures the slot is reachable for dropping).

## Scope boundaries

**In:** the daemon early-exit fallback; protocol/registry/server/client plumbing;
`ensure_run_active` using it for resume-capable agents.

**Out:** changing cursor/hermes behavior (still fresh, no fallback needed); any
frontend change (the same pane updates through the existing subscription); making a
genuinely-broken-agent pane non-dead (honest error is correct).

## Testing

- **`session.rs` unit tests** (mirror the existing subscribe/input tests):
  - primary exits fast → fallback runs in the same session: subscribe, feed a
    primary like `/bin/sh -c "printf NOPE; exit 1"` with fallback
    `/bin/sh -c "printf FRESH; sleep 3"` (grace 2s); assert the subscriber/`capture`
    sees both `NOPE` and `FRESH` and `status()` stays `Running`.
  - primary stays running → fallback never fires: primary `/bin/sh -c "sleep 3"` +
    a fallback; assert no fallback output and status `Running`.
  - primary exits after grace → normal `Exited`: primary
    `/bin/sh -c "printf BYE; exit 0"` with grace `0` (so elapsed ≥ grace) and a
    fallback; assert an `Exited` frame is delivered and the fallback did not run.
- **`protocol.rs`:** `StartSession` with a `fallback` round-trips through
  `encode_json`/`decode_client`.
- **`ensure_run_active` integration** (`crates/agency-app/tests/state.rs`): register
  a fake agent profile with `resume_args = ["-c","printf no-conv; exit 1"]` and
  `args = ["-c","printf FRESH; sleep 5"]` (so fresh = render_args), `command =
  "/bin/sh"`; `create_run` → `stop_run` (→ `Gone`) → `ensure_run_active` → poll until
  `run_status` is `Running` and `run_preview`/capture contains `FRESH`.
