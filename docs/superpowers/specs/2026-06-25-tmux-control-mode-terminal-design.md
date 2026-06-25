# tmux Control Mode for Terminal/Agent Attach — Design

**Date:** 2026-06-25

## Goal

Replace the raw `tmux attach-session`-in-a-PTY attach with a **tmux control-mode**
(`tmux -CC attach`) client. This removes the documented fragility where abruptly
killing the attach client can take the session's pane process down with it — the
root cause of the "Pane is dead (status 0)" terminal-death bug. A graceful
`detach-client` already mitigates it; control mode is the structural cure and the
architecture the tmux docs recommend for GUI embedding.

## Background

Each agency "run" (agent or plain terminal) is its own tmux session
(`agency-<id>` / `agency-run-<id>`) with one window and one pane running the agent
command or `$SHELL -l`. The UI streams that pane to xterm.js:

- `Tmux::attach(name, on_output)` spawns `tmux attach-session` in a PTY (via
  `spawn_agent`/`portable-pty`) and returns an `AgentHandle`. Raw PTY bytes flow
  to `on_output`; `AgentHandle::write_input` writes keystrokes to the client's PTY
  stdin; resize is a `SIGWINCH` via `master.resize`.
- `state.rs` holds handles in `attaches` (agent) and `run_attaches` (run-script)
  maps. `attach_run`/`detach_run`/`run_input`/`resize_run` (and the `*_script`
  variants) drive them. `commands.rs` streams bytes to the UI as base64 over a
  Tauri `Channel`. `FocusTerminal.tsx` renders via xterm and sends `onData` back.
- On navigation, `FocusTerminal` unmounts → `detach_run` → the handle drops →
  `AgentHandle::Drop` SIGKILLs the `tmux attach` client. That abrupt kill is what
  can EOF the pane's line-mode shell (raw-mode programs like claude survive).

Control-mode facts that shape this design (tmux Control Mode wiki, tmux(1)):

- `tmux -CC attach -t <session>` speaks a **line-based text protocol over plain
  pipes — no PTY required**.
- Pane output arrives as `%output %<paneid> <data>` where bytes < ASCII 32 and `\`
  are **octal-escaped** (`\` → `\134`). Recover raw bytes by reversing the octal
  escaping. `%extended-output %<paneid> <age> : <data>` is the same with an age
  field to ignore.
- Other notifications: `%begin`/`%end`/`%error` (command-response framing),
  `%exit`, `%window-*`, `%session-*`, a startup banner. A control client receives
  output only for its **attached session's** panes — which is exactly one pane
  per run, so no cross-session routing is needed.
- Input is sent as `send-keys` commands written to the client's stdin (`-H` takes
  hex byte values). Size is set with `refresh-client -C`.
- Control mode does **not** repaint the screen on attach (raw attach does); only
  new output streams. Reattach must reconstruct the screen another way.

## Approach (chosen)

**A — Pipe-based control client, one per session.** A new `ControlClient` in
`agency-core` spawns `tmux -CC attach -t <session>` with plain piped stdin/stdout
(no PTY). A reader thread parses the protocol and un-escapes `%output` to raw bytes
fed to the existing `on_output` callback. Input → `send-keys -H`; resize →
`refresh-client -C`; detach → close stdin (client exits cleanly, never disturbing
the pane). This keeps the one-session-per-run model and leaves `commands.rs` and
`FocusTerminal`/xterm unchanged (they still receive raw bytes).

Rejected: **B — PTY-based control client** (retains the PTY-kill fragility we are
escaping); **C — single global control client multiplexing all sessions** (control
mode streams only the attached session's panes, so this needs session-switching
and a routing layer — a much larger change for no current benefit, YAGNI).

## Components

### 1. `Attachment` trait + handle boundary (`agency-core`)

Introduce a trait abstracting the two attach implementations:

```rust
pub trait Attachment: Send {
    fn write_input(&self, data: &[u8]) -> Result<()>;
    fn resize(&self, rows: u16, cols: u16) -> Result<()>;
    fn status(&self) -> AgentStatus;
}
```

`AgentHandle` (raw PTY path) implements `Attachment`. `ControlClient` (new)
implements it. `state.rs`'s `attaches`/`run_attaches` maps change from
`HashMap<String, AgentHandle>` to `HashMap<String, Box<dyn Attachment>>`. The merge
**resolver** path (direct `spawn_agent`, held in the separate `resolvers` map) is
untouched and keeps using `AgentHandle` concretely.

### 2. `ControlClient` (`agency-core`, new module e.g. `control.rs`)

- Spawns `tmux -L agency -CC attach-session -t <session>` via
  `std::process::Command` with `stdin`/`stdout` piped (no PTY). `stderr` inherited
  or piped for diagnostics.
- **Reader thread** reads stdout, buffers partial lines, and per full line:
  - `%output %<pane> <data>` / `%extended-output %<pane> <age> : <data>` →
    `unescape_octal(data)` → `on_output(bytes)`. The pane id is not tracked: each
    run-session has exactly one pane, so input targets the session directly (see
    Input).
  - `%begin`/`%end` → discard the wrapped block (fire-and-forget command output).
  - `%error ...` → log via `pty_debug`.
  - `%exit` → set status, end the thread.
  - anything else → ignore.
- **Wait thread** records the child's exit code into a shared `AgentStatus` (mirrors
  `spawn_agent`).
- Holds a mutex-guarded `ChildStdin` writer.
- `write_input(data)` → write `send-keys -t <session> -H <hex...>\n`.
- `resize(rows, cols)` → write `refresh-client -C <cols>x<rows>\n` (exact `-C`
  argument format to be confirmed against the installed tmux 3.6b during
  implementation).
- `Drop`: close stdin (graceful exit), then `kill()` as a leak-prevention fallback.

### 3. `Tmux::attach` variant selection

`Tmux` gains a control-mode attach builder (e.g. `attach_control(name, on_output)
-> ControlClient`). `state.rs`'s `attach_run`/`attach_run_script` pick raw vs
control based on a flag and insert a `Box<dyn Attachment>`.

### 4. Flag / rollout

`AGENCY_TMUX_CONTROL=1` env var selects control mode; unset → current raw path,
fully unchanged. After live verification, flip the default and remove the raw
attach path and now-unused PTY-attach plumbing.

### 5. Reattach screen reconstruction (`FocusTerminal.tsx` + `Tmux::capture`)

Control mode does not repaint on attach, so reconstruct the current screen via:
- **Seed:** the existing `preview()`→`capture-pane` snapshot, upgraded to
  `capture-pane -e -p` so it carries colors/attributes. `FocusTerminal` already
  writes the seed and `term.reset()`s when the live stream takes over (Bug-3 fix).
- **Redraw nudge:** the `doFit()` resize on attach issues `refresh-client -C`,
  prompting tmux to re-send visible content for size-sensitive TUIs (claude).

No other `FocusTerminal` changes; `commands.rs` and the `Channel` stay as-is.

## Data flow (control mode)

```
pane process ──▶ tmux server ──(%output, octal-escaped, over pipe)──▶ ControlClient
  reader thread: parse line, un-escape ──▶ on_output(raw bytes)
  ──▶ commands.rs Channel (base64) ──▶ FocusTerminal ──▶ xterm.write

xterm.onData ──▶ run_input ──▶ ControlClient.write_input
  ──▶ "send-keys -t <session> -H <hex>\n" on client stdin ──▶ tmux ──▶ pane

doFit ──▶ resize_run ──▶ ControlClient.resize ──▶ "refresh-client -C WxH\n"

detach_run ──▶ drop Box<dyn Attachment> ──▶ ControlClient::Drop
  ──▶ close stdin (client EOFs and exits cleanly; pane untouched) + kill fallback
```

## Error handling

- Malformed/unknown `%` lines: ignored (forward-compatible with tmux additions).
- `%error`: logged via `pty_debug`, not surfaced as a hard failure.
- Octal un-escape on malformed input: pass through the literal bytes rather than
  panicking.
- Spawn failure (tmux missing / session gone): `attach_run` returns `Err` exactly
  as the raw path does today.
- Detach is best-effort and idempotent; double-detach is safe.

## Testing

- **Unit (no tmux):** `unescape_octal` round-trips (`\015`, `\012`, `\134`, mixed
  printable+escaped); line parser handles `%output`, `%extended-output`, `%begin`/
  `%end` blocks, `%error`, `%exit`, partial-line buffering, and unknown lines.
- **Integration (real tmux, like `crates/agency-core/tests/tmux.rs`):**
  - control-attach a session running a known command → streamed output arrives
    un-escaped and correct;
  - `write_input` reaches the pane (echo round-trip, mirroring
    `attach_streams_output_and_input`);
  - **survives detach → reattach** with the pane still `Running` (the regression
    that matters);
  - `resize` issues without error.
- The existing `terminal_survives_attach_detach_reattach` AppState test runs under
  the flag too.

## Out of scope / future

- Single global multiplexed control client (Approach C).
- Removing `portable-pty` entirely — the resolver path still uses `spawn_agent`.
- Persisting attach across navigation (keeping the client alive while unmounted) —
  control mode makes this feasible later, but it is not part of this change.
```
