# Server-Side Terminal Emulator Daemon (`agency-termd`) — Design

**Date:** 2026-06-28
**Branch:** `feat/termd`

## Goal

Remove tmux entirely and replace it with a purpose-built terminal session daemon,
`agency-termd`, that the Agency app drives as a client over a Unix socket. The
daemon owns each agent/terminal PTY and runs a headless terminal emulator
(`alacritty_terminal`) over it, maintaining the screen grid + scrollback as the
source of truth.

This is the structural cure for the entire class of tmux bugs we keep hitting,
all of which live in the **attach seam** — driving `tmux attach-session` over a
PTY and forwarding a raw terminal stream:

- "Pane is dead (status 0)" — abruptly killing the attach client EOFs the pane's
  shell (see `2026-06-23` graceful-detach fix, which mitigated but did not cure).
- No-repaint-on-reattach — the reason the control-mode rewrite
  (`2026-06-25-tmux-control-mode-terminal-design.md`) was built and then reverted:
  control mode does not repaint the screen on attach, and there was no clean way
  to reconstruct it.

Owning the emulator fixes both at the root: reattach becomes "serve the grid we
already hold," and there is no attach-client process to kill.

## Why a daemon (not an in-process emulator)

Hard requirement: **running agents must survive an Agency app restart or crash.**
Agents are long-running and autonomous; losing them when the GUI quits is
unacceptable. That rules out a pure in-process emulator (the PTYs would be
children of the app and die with it). The PTYs must live in a process that
outlives the app — hence a daemon.

This is *not* reinventing tmux. The thing tmux gets wrong for us is the wire
format: it forwards a raw terminal stream over a PTY-attach, which is the fragile
seam. We own the protocol between app and daemon, so the daemon serves the app
**structured data** — a screen-grid snapshot on attach, then output deltas — over
a real IPC socket. Reattach-repaint stops being a problem because reattach
literally means "send me the grid you are already holding."

We also gain: no bundled external binary to resolve/ship (tmux today is resolved
via `Tmux::resolved()` and slated for bundling), and `capture`/`send` become
first-class calls on our own data instead of shell-outs.

## Background: how tmux is used today

Each run is its own tmux session on a private socket (`tmux -L agency`):

- **Agent session** `agency-<id>` — the agent command, one pane
  (`state.rs:458`).
- **Run-script overlay** `agency-run-<id>` — execution scripts (`state.rs:682`).
- **Interactive shell** — a plain `$SHELL -l` session (`state.rs:494`).

The `Tmux` wrapper (`crates/agency-core/src/tmux.rs`) shells out for every
operation: `start_session`, `attach` (spawns `tmux attach-session` in a PTY via
`supervisor.rs::spawn_agent`), `capture` (`capture-pane -p`, used for idle
detection + previews), `send_text` (`send-keys`), `session_status`
(`list-panes -F '#{pane_dead} #{pane_dead_status}'`), `kill_session`,
`detach_clients`. Session options set at creation: `remain-on-exit on`,
`window-size latest`, `status off`, truecolor `terminal-overrides`, plus
`COLORTERM`/`TERM` defaults for Finder-launched bundles.

`supervisor.rs::spawn_agent` already spawns a command directly into a
`portable-pty` PTY with a reader thread (pumps output to a callback regardless of
any viewer) and a wait thread (records exit code). Today the command it spawns
happens to be `tmux attach-session`; the daemon will spawn the **agent command
itself** there instead. This module relocates into the daemon almost verbatim.

App ↔ UI transport is unchanged: `commands.rs` streams bytes to the UI as base64
over a Tauri `Channel`; `FocusTerminal.tsx` renders via xterm.js and sends
`onData` back. Only the app ↔ backend layer (tmux → daemon socket) changes.

## Architecture: two processes

**`agency-termd`** — a new lightweight bin target built from `agency-core` (not
the Tauri binary, so no webview weight). Owns every PTY + emulator; outlives the
app. Listens on a per-user Unix socket at
`~/Library/Application Support/agency/termd.sock`.

**Agency app** (Tauri) — a thin client of the daemon over that socket.

On launch the app tries to connect; if nothing answers, it spawns the daemon
**self-daemonized** (double-fork + `setsid`, fds closed) so the daemon reparents
away from the app and survives an app crash or relaunch. The daemon idle-exits
once it has had zero sessions for a short grace period, so it never lingers
forever. It does **not** exit merely because the app disconnected — that is what
preserves agents across a crash. (launchd-managed supervision is a hardening
follow-up, out of scope for v1.)

The daemon is the **crash-resilience** layer. Normal, visible persistence and
user control are handled by the menu bar app (next section), so the daemon never
runs as an invisible ghost in normal use.

## App lifecycle and the menu bar (system tray)

The Agency app behaves as a **menu bar app**, not a window that dies on close:

- **Close the main window** → the app does *not* quit. It retreats to a macOS
  menu bar status item and keeps running, with the daemon + agents untouched. The
  status item is the always-visible proof that work is still running — there is no
  invisible background state in normal operation.
- **The menu bar menu** shows running-agent count/status (sourced from the daemon
  via `List`/`Status`) and offers "Open Agency" (re-show the window) and "Quit
  Agency".
- **Quit Agency** (from the menu bar, or `Cmd+Q`) → a confirmation dialog warns it
  will **stop all running agents**. On confirm, the app `Kill`s every session,
  sends the daemon a `Shutdown`, and exits fully — no orphaned processes.
- **Reopen / relaunch** → the app reconnects to the (still-running) daemon,
  `List`s sessions, re-adopts them, and re-shows the status item.

The status item is owned by the **app** (via Tauri's tray API), keeping the
daemon headless and lightweight. This cleanly separates the two persistence
layers: the menu bar app is *visible persistence + control*; the daemon is
*crash resilience*.

**The one bounded ghost window.** After a true crash (not a menu-bar Quit), the
daemon + agents survive with no status item until the next relaunch re-adopts
them — this is the crash-survival requirement working as intended, not a leak. It
is bounded: relaunch reclaims it, and if the user never relaunches, the agents
finish and the daemon idle-exits on its own.

## Components

All in `agency-core`, daemon-side unless noted. The `state.rs` session layer is
refactored to the new client API — **no compatibility shim**, this is a clean
switch (single user, no migration burden).

| Component | Responsibility | Origin |
|---|---|---|
| `pty` | spawn PTY, reader + wait threads, exit code | **moved** from `supervisor.rs` |
| `emulator` | wrap `alacritty_terminal::Term`; feed bytes; extract plain text (capture); serialize snapshot | new |
| `session` | one PTY + one emulator + status + subscriber list; owns lock discipline | new |
| `registry` | `id → session`; create / list / kill; idle-exit timer | new |
| `protocol` | frame codec + message enums (shared daemon ↔ client) | new |
| `server` | Unix-socket accept loop, per-connection codec, dispatch, output fan-out | new |
| `client` (`TermClient`) | **app-side**: connect, spawn-daemon-if-absent, purpose-built API | new |
| `agency-termd` | thin `main()` → `server::run()` | new bin target |

Each component has one purpose and a narrow interface: `pty` knows nothing about
the protocol; `emulator` knows nothing about sockets; `server` knows nothing about
`alacritty_terminal` internals (it talks to `session`). `tmux.rs` is deleted; the
old `Tmux` call sites in `state.rs` move to `TermClient`.

## Emulator: `alacritty_terminal` (Apache-2.0)

Mature, robust scrollback ring; trivial plain-text extraction (walk cells → chars)
for `capture`. The one custom piece is a **snapshot serializer**: walk scrollback
+ visible grid and emit text with SGR-diffed ANSI escapes plus a final cursor
position. It is bounded (~150 lines) and unit-tested by round-trip: serialize →
feed a fresh emulator → assert the two grids are equal.

Fallback (documented, not chosen): the `vt100` crate ships `contents_formatted()`
for free but has shallower scrollback; we would only fall back if the serializer
proves fussy and we accept weaker history.

## IPC protocol

Unix `SOCK_STREAM`. Length-prefixed frames: `[u32 len][u8 type][payload]`.
Control payloads are JSON (serde, debuggable); the **output hot path is raw bytes**
(no base64 tax inside the daemon link). The protocol is **additive by design** —
tagged messages, unknown fields ignored — so it survives app updates without
breaking running agents (see Versioning).

**Client → daemon**

- `StartSession { id, cwd, command, args, env, cols, rows }` — spawn PTY + emulator.
- `Subscribe { id }` — begin receiving this session's output (reply: `Snapshot`, then `Output` stream).
- `Unsubscribe { id }` — stop receiving; session keeps running.
- `Input { id, bytes }` — write to the PTY (`send_text` becomes `Input` of text + `\r`).
- `Resize { id, cols, rows }` — resize PTY + emulator.
- `Capture { id, lines }` — reply `Captured { text }` (plain text from grid).
- `Status { id }` — reply `SessionStatus`.
- `Kill { id }` — terminate the session.
- `List` — reply with all session ids + statuses (for startup rehydration).
- `Shutdown` — kill all sessions and exit the daemon (sent by the menu bar "Quit Agency" path).

**Daemon → client**

- `Snapshot { id, data, cursor, cols, rows }` — `data` is the ANSI reconstruction of scrollback + screen.
- `Output { id, bytes }` — live raw delta.
- `Exited { id, code }` — pushed on child exit.
- replies: `Captured`, `SessionStatus`, `SessionList`, `Error { id?, message }`.

**Reattach repaint (the fix).** `Subscribe` replies with a `Snapshot`
reconstructed from the emulator grid, then streams live `Output`. The daemon holds
a session lock across *snapshot serialization + subscriber registration* so no
bytes slip between the two (the reader thread blocks on the same lock while
applying bytes). Result: a freshly attached xterm.js always receives full current
state — never blank-on-reattach.

**Multiple subscribers.** The daemon fans `Output` out to all subscribers of a
session, replacing tmux multi-client. In practice the app is one connection
multiplexing many sessions; previews use `Capture` (no subscription).

## Lifecycle parity

Maps 1:1 to today's behavior; only the data source changes.

| Today (tmux) | New (daemon) |
|---|---|
| `start_session` on run launch | `StartSession` |
| `attach` / `detach_clients` | `Subscribe` / `Unsubscribe` |
| `capture` (idle detect, previews) | `Capture` — **idle-detection hashing stays app-side**, only the source swaps |
| `send_text` (review comments) | `Input` (text + `\r`) |
| `session_status` | `Status` |
| `kill_session` (`close_project`, `delete_project`) | `Kill` |
| `remain-on-exit on` | session record retained with `Exited(code)` until explicit `Kill` |
| incidental session rediscovery on app restart | **explicit `List` → rehydrate run statuses on startup** |
| `COLORTERM`/`TERM`/truecolor overrides | carried into the PTY env at spawn (same Finder-bundle reasons) |

Exit codes come from the existing wait thread in `supervisor.rs`. `remain-on-exit`
parity is natural: the daemon simply does not drop the session record when the
child exits — it flips status to `Exited(code)` and keeps the final grid until
`Kill`.

## Scrollback + search (the two additions beyond parity)

The daemon includes scrollback in the `Snapshot`; `FocusTerminal.tsx` configures
xterm.js with a large `scrollback` and holds it in the client buffer. Search is
the xterm.js `SearchAddon` (client-side over that buffer) plus a small search box
in `FocusTerminal.tsx`. No extra daemon surface is required for search — it is a
pure consequence of serving scrollback in the snapshot.

## Error handling (no silent failures)

- **Daemon unreachable / crashed** — same failure domain as the tmux server dying:
  agents are gone (they are the daemon's children). The app detects the
  disconnect, attempts one respawn, runs `List` on reconnect, and **surfaces the
  state explicitly** (e.g. "terminal backend restarted; N sessions lost") rather
  than rendering a dead blank terminal.
- **Per-session spawn failure** — daemon returns `Error { id, message }`; the UI
  renders it on the affected terminal.
- **Frame/codec errors** — logged and the offending connection is dropped; the app
  reconnects. Never swallowed.

## Versioning (revised — does NOT mean killing agents each update)

The handshake stamps a `protocol_version`. Because the protocol is additive,
**ordinary app updates do not change the wire format** — a new app connects to the
still-running old daemon and agents are undisturbed. This is the normal case.

A version mismatch only occurs on a **deliberate breaking change** to the
protocol (renaming/removing a message, changing a field's meaning) — rare and
entirely in our control. When it happens, the app detects the mismatch on connect
and presents a **clear, manual choice**: keep the running agents (reconnect using
the compatible format) or drain and restart. Never a silent break, never
every-update.

This is strictly better than today: an external tmux upgrade under running
sessions already forces a "protocol version mismatch / kill the server" event,
which we have simply been lucky to avoid. Full hot-upgrade of the daemon is out
of scope for v1.

## Testing

- **Daemon unit tests** (pattern of existing `tests/tmux.rs`): registry
  create/list/kill; emulator feed; **snapshot serializer round-trip**; capture
  plain text; status transitions (Running → Exited(code)); idle-exit timer.
- **IPC integration test**: spin the daemon on a temp socket; client
  `StartSession` → `Subscribe` (assert snapshot) → `Input` → `Capture` (assert
  echoed text) → `Kill` (assert `Exited`).
- `tests/tmux.rs` is **replaced** by `tests/termd.rs`.

## Out of scope (v1)

- launchd-managed daemon supervision (self-daemonize is sufficient for v1).
- Daemon hot-upgrade across breaking protocol changes (manual drain is acceptable).
- Split panes / multiple windows per session (each session stays one pane, as today).
- The `vt100` fallback emulator (documented only; default is `alacritty_terminal`).
- Windows/Linux menu bar parity — the design targets the macOS menu bar status
  item. The Tauri tray API is cross-platform, so the same code is expected to work
  as a system-tray icon elsewhere, but only macOS is validated for v1.

## Sequencing

Big-bang cutover on `feat/termd`: build the daemon, swap all three session types
(agent, run-script, shell) to `TermClient`, refactor the `state.rs` session layer,
add the menu bar status item + window-close-to-tray + Quit-confirmation in the
Tauri app, then delete `tmux.rs` last once the daemon path is green. tmux is
removed only at the end of the branch. The menu bar lifecycle work is app-side
(`agency-app`, Tauri tray API) and independent of the daemon internals, so it can
land in parallel with the protocol/emulator work.

## Risks

- **Snapshot serializer fidelity** — colors/wrapping/alt-screen edge cases.
  Mitigated by round-trip tests; `vt100` fallback if needed.
- **Self-daemonization correctness on macOS bundles** — fd/cwd/session handling
  must be right or the daemon dies with the app (defeating crash survival).
  Validate explicitly across all three lifecycle paths: (1) **close window** →
  agent keeps running, status item present, reopening re-adopts it; (2) **menu bar
  Quit** → confirmation shown, all agents stopped, daemon exited, no orphaned
  processes; (3) **hard crash** (`kill -9` the app) → daemon + agent survive,
  relaunch re-adopts via `List` and re-shows the status item.
- **Performance of the live path** — fanning raw bytes + feeding the emulator on
  every chunk. Expected fine (one local socket, per-session threads), but verify
  with a high-output agent.
