# Server-Side Terminal Daemon (`agency-termd`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace tmux with a purpose-built terminal session daemon that owns PTYs + an `alacritty_terminal` emulator and serves the Agency app structured snapshots/deltas over a Unix socket, fixing the attach-seam bug class and adding scrollback + search and a menu bar lifecycle.

**Architecture:** A new `agency-termd` binary (built from `agency-core`) self-daemonizes and owns every PTY + emulator, persisting across app crash/relaunch. The Tauri app is a thin client (`TermClient`) over a Unix socket; on window-close it retreats to a menu bar status item; "Quit" kills all sessions + the daemon. tmux is removed at the end of the branch.

**Tech Stack:** Rust (`agency-core`, `agency-app`/Tauri 2), `alacritty_terminal 0.26`, `portable-pty 0.8`, `daemonize 0.5`, std `UnixListener`/`UnixStream`, xterm.js 5.5 + `@xterm/addon-search`.

## Global Constraints

- Branch: `feat/termd`. tmux (`crates/agency-core/src/tmux.rs`) is deleted only in the final cutover task, after the daemon path is green.
- Crate versions (exact): `alacritty_terminal = "0.26"`, `daemonize = "0.5"`. Keep `portable-pty = "0.8"`.
- Emulator default is `alacritty_terminal`; the `vt100` fallback is NOT implemented (out of scope).
- IPC frame format: `[u32 len little-endian][payload]`, where `payload[0]` is a type byte. Type 0 = JSON control message; type 1 = Output (binary); type 2 = Input (binary); type 3 = Snapshot (binary). Control messages are JSON via serde_json; byte-carrying frames are binary (no base64).
- `SessionStatus` serde shape MUST stay identical to today's (`#[serde(tag = "state", rename_all = "camelCase")]`, variants `Running` / `Exited { code }` / `Gone`) — the UI/notifier depend on it.
- No silent failures: daemon-unreachable, per-session spawn failure, and codec errors are surfaced, never swallowed.
- COLORTERM/TERM/truecolor defaults from today's `tmux.rs` (`COLORTERM=truecolor`, `TERM=xterm-256color`) must be applied to the spawned PTY env for the same Finder-bundle reasons.
- Socket path is supplied BY THE APP (it knows its Tauri data dir) as `argv[1]` to the daemon and as a parameter to `TermClient` — `agency-core` does not compute paths.
- Run `cargo test -p agency-core` and `cargo build` from the workspace root (the repo root).

---

## File Structure

**`crates/agency-core/` (new `term` module + bin):**
- `src/term/mod.rs` — module root; re-exports; `pub use protocol::SessionStatus`.
- `src/term/protocol.rs` — message enums + frame codec (shared daemon ↔ client).
- `src/term/emulator.rs` — `alacritty_terminal` wrapper: feed, resize, capture text, snapshot.
- `src/term/pty.rs` — PTY spawn + reader/wait threads (generalized from `supervisor.rs`).
- `src/term/session.rs` — one PTY + emulator + status + subscribers; lock discipline.
- `src/term/registry.rs` — `id → Session` map; create/get/list/kill; idle-exit policy.
- `src/term/server.rs` — `UnixListener` accept loop; per-connection dispatch + fan-out.
- `src/term/client.rs` — `TermClient` (app-side): connect/spawn, demux reader, request/reply.
- `src/bin/agency-termd.rs` — thin `main`: daemonize + `server::run`.
- `src/lib.rs` — add `pub mod term;`.
- `tests/termd.rs` — integration tests (replaces `tests/tmux.rs`).

**Deleted at cutover:** `src/tmux.rs`, `src/supervisor.rs` (logic moved to `term/pty.rs`), `tests/tmux.rs`.

**`crates/agency-app/`:**
- `src/state.rs` — replace `Tmux` field with `TermClient`; refactor session methods; startup rehydrate.
- `src/lib.rs` — daemon spawn on startup; menu bar tray; window-close-to-tray; Quit confirmation.
- `Cargo.toml` — add Tauri `tray-icon` feature + `tauri-plugin-dialog`.

**`ui/`:**
- `src/components/FocusTerminal.tsx` — scrollback config + `SearchAddon` + search box.
- `package.json` — add `@xterm/addon-search`.

---

## Phase 1 — Daemon core (`agency-core`)

### Task 1: Add dependencies, module skeleton, and alacritty smoke test

**Files:**
- Modify: `crates/agency-core/Cargo.toml`
- Create: `crates/agency-core/src/term/mod.rs`
- Modify: `crates/agency-core/src/lib.rs`
- Test: `crates/agency-core/src/term/mod.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `agency_core::term` module; confirms the exact `alacritty_terminal 0.26` feed→read-grid API that Task 2 builds on.

- [ ] **Step 1: Add deps**

In `crates/agency-core/Cargo.toml` under `[dependencies]` add:

```toml
alacritty_terminal = "0.26"
daemonize = "0.5"
```

- [ ] **Step 2: Create the module root**

Create `crates/agency-core/src/term/mod.rs`:

```rust
//! Terminal session daemon: PTY + emulator ownership, IPC protocol, client.
pub mod protocol;
pub mod emulator;
pub mod pty;
pub mod session;
pub mod registry;
pub mod server;
pub mod client;

pub use protocol::SessionStatus;
```

Comment out every `pub mod` line except none yet — to compile incrementally, start with only what exists. For Task 1, replace the body with just the doc comment and a smoke test (add the `pub mod` lines back as each file lands in later tasks).

Set `src/term/mod.rs` for Task 1 to:

```rust
//! Terminal session daemon: PTY + emulator ownership, IPC protocol, client.

#[cfg(test)]
mod smoke {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::Processor;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line};

    /// Minimal `Dimensions` so we can construct a `Term` headlessly.
    struct Dims { cols: usize, screen: usize, total: usize }
    impl Dimensions for Dims {
        fn total_lines(&self) -> usize { self.total }
        fn screen_lines(&self) -> usize { self.screen }
        fn columns(&self) -> usize { self.cols }
    }

    #[test]
    fn feed_bytes_and_read_a_char() {
        let dims = Dims { cols: 80, screen: 24, total: 24 + 1000 };
        let mut term: Term<VoidListener> = Term::new(Config::default(), &dims, VoidListener);
        let mut parser = Processor::new();
        parser.advance(&mut term, b"hi");
        let grid = term.grid();
        let c0 = grid[Line(0)][Column(0)].c;
        let c1 = grid[Line(0)][Column(1)].c;
        assert_eq!((c0, c1), ('h', 'i'));
    }
}
```

- [ ] **Step 3: Wire the module**

In `crates/agency-core/src/lib.rs` add:

```rust
pub mod term;
```

- [ ] **Step 4: Run the smoke test**

Run: `cargo test -p agency-core term::smoke -- --nocapture`
Expected: PASS. If the `alacritty_terminal 0.26` API differs (constructor signature, `Processor::advance` arity, grid indexing), THIS is where you fix the calls — every later emulator task uses exactly these primitives. Record any signature corrections in `src/term/emulator.rs` doc comments.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/Cargo.toml crates/agency-core/src/term/mod.rs crates/agency-core/src/lib.rs
git commit -m "feat(termd): add deps and confirm alacritty_terminal feed/read API"
```

---

### Task 2: Emulator wrapper (feed, resize, capture, snapshot)

**Files:**
- Create: `crates/agency-core/src/term/emulator.rs`
- Modify: `crates/agency-core/src/term/mod.rs` (uncomment `pub mod emulator;`)
- Test: inline `#[cfg(test)]` in `emulator.rs`

**Interfaces:**
- Produces:
  - `Emulator::new(cols: u16, rows: u16) -> Emulator`
  - `Emulator::feed(&mut self, bytes: &[u8])`
  - `Emulator::resize(&mut self, cols: u16, rows: u16)`
  - `Emulator::capture(&self, lines: usize) -> String`
  - `Emulator::snapshot(&self) -> Snapshot`
  - `pub struct Snapshot { pub cols: u16, pub rows: u16, pub cx: u16, pub cy: u16, pub data: Vec<u8> }`
- Consumes: alacritty primitives confirmed in Task 1.

- [ ] **Step 1: Write the failing tests**

Create `crates/agency-core/src/term/emulator.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_returns_plain_text() {
        let mut e = Emulator::new(80, 24);
        e.feed(b"hello world");
        assert!(e.capture(5).contains("hello world"));
    }

    #[test]
    fn capture_includes_scrollback_lines() {
        let mut e = Emulator::new(80, 3);
        for i in 0..10 {
            e.feed(format!("line{i}\r\n").as_bytes());
        }
        let cap = e.capture(20);
        assert!(cap.contains("line0"), "scrollback line missing: {cap:?}");
        assert!(cap.contains("line9"));
    }

    #[test]
    fn snapshot_round_trips_through_a_fresh_emulator() {
        let mut a = Emulator::new(40, 6);
        a.feed(b"alpha\r\nbeta\r\ngamma");
        let snap = a.snapshot();

        let mut b = Emulator::new(snap.cols, snap.rows);
        b.feed(&snap.data);
        assert_eq!(b.capture(10).trim_end(), a.capture(10).trim_end());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core emulator -- --nocapture`
Expected: FAIL to compile (`Emulator` not defined).

- [ ] **Step 3: Implement the emulator**

Prepend to `crates/agency-core/src/term/emulator.rs` (above the test module):

```rust
//! Headless terminal emulator over a PTY byte stream.
//!
//! NOTE: API confirmed against `alacritty_terminal 0.26` in `term::smoke`
//! (Task 1). If `Term::new` / `Processor::advance` / grid indexing change with
//! a version bump, fix them here and in the smoke test together.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

const SCROLLBACK: usize = 10_000;

pub struct Snapshot {
    pub cols: u16,
    pub rows: u16,
    pub cx: u16,
    pub cy: u16,
    pub data: Vec<u8>,
}

struct Dims { cols: usize, screen: usize }
impl Dimensions for Dims {
    fn total_lines(&self) -> usize { self.screen + SCROLLBACK }
    fn screen_lines(&self) -> usize { self.screen }
    fn columns(&self) -> usize { self.cols }
}

pub struct Emulator {
    term: Term<VoidListener>,
    parser: Processor,
    cols: u16,
    rows: u16,
}

impl Emulator {
    pub fn new(cols: u16, rows: u16) -> Emulator {
        let dims = Dims { cols: cols as usize, screen: rows as usize };
        let term = Term::new(Config::default(), &dims, VoidListener);
        Emulator { term, parser: Processor::new(), cols, rows }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let dims = Dims { cols: cols as usize, screen: rows as usize };
        self.term.resize(dims);
        self.cols = cols;
        self.rows = rows;
    }

    /// Plain text of the last `lines` rows (scrollback + screen), no escapes.
    pub fn capture(&self, lines: usize) -> String {
        let grid = self.term.grid();
        let total = grid.total_lines();
        let want = lines.min(total);
        // Lines are indexed with 0 = top of screen, negatives = scrollback.
        let start = self.rows as i32 - want as i32;
        let mut out = String::new();
        for li in start..self.rows as i32 {
            let mut row = String::new();
            for col in 0..self.cols as usize {
                row.push(grid[Line(li)][Column(col)].c);
            }
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out
    }

    /// Reconstruct scrollback + screen as an ANSI repaint stream that, fed into a
    /// fresh emulator (or xterm.js), reproduces the current display.
    pub fn snapshot(&self) -> Snapshot {
        let grid = self.term.grid();
        let total = grid.total_lines();
        let mut data: Vec<u8> = Vec::new();
        // Clear + home, then emit scrollback above the screen as plain lines.
        data.extend_from_slice(b"\x1b[2J\x1b[3J\x1b[H");

        let history = (total - self.rows as usize) as i32;
        let mut last_flags = Flags::empty();
        let mut last_fg = Color::Named(NamedColor::Foreground);
        let mut last_bg = Color::Named(NamedColor::Background);
        for li in (-history)..self.rows as i32 {
            for col in 0..self.cols as usize {
                let cell = &grid[Line(li)][Column(col)];
                if cell.flags != last_flags || cell.fg != last_fg || cell.bg != last_bg {
                    data.extend_from_slice(sgr(cell.flags, cell.fg, cell.bg).as_bytes());
                    last_flags = cell.flags;
                    last_fg = cell.fg;
                    last_bg = cell.bg;
                }
                let mut buf = [0u8; 4];
                data.extend_from_slice(cell.c.encode_utf8(&mut buf).as_bytes());
            }
            data.extend_from_slice(b"\x1b[0m\r\n");
            last_flags = Flags::empty();
            last_fg = Color::Named(NamedColor::Foreground);
            last_bg = Color::Named(NamedColor::Background);
        }

        let cur = self.term.grid().cursor.point;
        let cx = cur.column.0 as u16;
        let cy = cur.line.0.max(0) as u16;
        // Position the cursor (1-based) after the repaint.
        data.extend_from_slice(format!("\x1b[{};{}H", cy + 1, cx + 1).as_bytes());

        Snapshot { cols: self.cols, rows: self.rows, cx, cy, data }
    }
}

/// Build a minimal SGR sequence for the common attributes we reproduce.
fn sgr(flags: Flags, fg: Color, bg: Color) -> String {
    let mut codes: Vec<String> = vec!["0".into()];
    if flags.contains(Flags::BOLD) { codes.push("1".into()); }
    if flags.contains(Flags::DIM) { codes.push("2".into()); }
    if flags.contains(Flags::ITALIC) { codes.push("3".into()); }
    if flags.contains(Flags::UNDERLINE) { codes.push("4".into()); }
    if flags.contains(Flags::INVERSE) { codes.push("7".into()); }
    push_color(&mut codes, fg, true);
    push_color(&mut codes, bg, false);
    format!("\x1b[{}m", codes.join(";"))
}

fn push_color(codes: &mut Vec<String>, c: Color, fg: bool) {
    match c {
        Color::Spec(rgb) => {
            codes.push(if fg { "38".into() } else { "48".into() });
            codes.push("2".into());
            codes.push(rgb.r.to_string());
            codes.push(rgb.g.to_string());
            codes.push(rgb.b.to_string());
        }
        Color::Indexed(i) => {
            codes.push(if fg { "38".into() } else { "48".into() });
            codes.push("5".into());
            codes.push(i.to_string());
        }
        Color::Named(_) => { /* default fg/bg already reset by leading 0 */ }
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agency-core emulator -- --nocapture`
Expected: PASS. If `Term::resize`, `grid.cursor.point`, or `Flags`/`Color` paths differ in 0.26, adjust using the compiler errors (the field/method *names* are the only likely drift; the shape holds).

- [ ] **Step 5: Enable the module + commit**

In `src/term/mod.rs` ensure `pub mod emulator;` is present (and `pub mod protocol;` etc. stay commented until their tasks). Then:

```bash
git add crates/agency-core/src/term/emulator.rs crates/agency-core/src/term/mod.rs
git commit -m "feat(termd): headless emulator with capture + snapshot round-trip"
```

---

### Task 3: Protocol — message enums + frame codec

**Files:**
- Create: `crates/agency-core/src/term/protocol.rs`
- Modify: `crates/agency-core/src/term/mod.rs` (enable `pub mod protocol;`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `enum SessionStatus { Running, Exited { code: i32 }, Gone }` (serde tag `"state"`, camelCase)
  - `enum ClientMsg { StartSession{id,cwd,command,args,env,cols,rows}, Subscribe{id}, Unsubscribe{id}, Resize{id,cols,rows}, Capture{id,lines}, Status{id}, Kill{id}, List, Shutdown }`
  - `enum ServerMsg { Started{id}, Captured{id,text}, Status{id,status}, List{sessions: Vec<(String,SessionStatus)>}, Exited{id,code}, Error{id:Option<String>,message} }`
  - `enum ClientFrame { Msg(ClientMsg), Input{id:String,bytes:Vec<u8>} }`
  - `enum ServerFrame { Msg(ServerMsg), Output{id:String,bytes:Vec<u8>}, Snapshot{id:String,cols:u16,rows:u16,cx:u16,cy:u16,data:Vec<u8>} }`
  - `write_frame<W: Write>(w: &mut W, payload: &[u8]) -> io::Result<()>`
  - `read_frame<R: Read>(r: &mut R) -> io::Result<Vec<u8>>` (returns payload incl. type byte; `UnexpectedEof` on clean close)
  - `encode_json<M: Serialize>(m: &M) -> Vec<u8>` (type 0)
  - `encode_input(id,&[u8]) -> Vec<u8>` (type 2), `encode_output(id,&[u8]) -> Vec<u8>` (type 1)
  - `encode_snapshot(id,cols,rows,cx,cy,&[u8]) -> Vec<u8>` (type 3)
  - `decode_client(payload: &[u8]) -> Result<ClientFrame>`, `decode_server(payload: &[u8]) -> Result<ServerFrame>`

- [ ] **Step 1: Write the failing tests**

Create `crates/agency-core/src/term/protocol.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_client_msg_round_trips() {
        let payload = encode_json(&ClientMsg::Capture { id: "a".into(), lines: 50 });
        match decode_client(&payload).unwrap() {
            ClientFrame::Msg(ClientMsg::Capture { id, lines }) => {
                assert_eq!((id.as_str(), lines), ("a", 50));
            }
            other => panic!("wrong decode: {other:?}"),
        }
    }

    #[test]
    fn input_frame_carries_raw_bytes() {
        let payload = encode_input("sess", &[0x00, 0xff, b'x']);
        match decode_client(&payload).unwrap() {
            ClientFrame::Input { id, bytes } => {
                assert_eq!(id, "sess");
                assert_eq!(bytes, vec![0x00, 0xff, b'x']);
            }
            other => panic!("wrong decode: {other:?}"),
        }
    }

    #[test]
    fn snapshot_frame_round_trips() {
        let payload = encode_snapshot("s", 80, 24, 3, 4, b"\x1b[Hhi");
        match decode_server(&payload).unwrap() {
            ServerFrame::Snapshot { id, cols, rows, cx, cy, data } => {
                assert_eq!((id.as_str(), cols, rows, cx, cy), ("s", 80, 24, 3, 4));
                assert_eq!(data, b"\x1b[Hhi");
            }
            other => panic!("wrong decode: {other:?}"),
        }
    }

    #[test]
    fn frame_length_prefix_round_trips() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &encode_output("id", b"abc")).unwrap();
        let mut cur = std::io::Cursor::new(buf);
        let payload = read_frame(&mut cur).unwrap();
        match decode_server(&payload).unwrap() {
            ServerFrame::Output { id, bytes } => {
                assert_eq!((id.as_str(), bytes.as_slice()), ("id", b"abc".as_slice()));
            }
            other => panic!("wrong decode: {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core protocol -- --nocapture`
Expected: FAIL to compile.

- [ ] **Step 3: Implement protocol**

Prepend to `crates/agency-core/src/term/protocol.rs`:

```rust
//! IPC message types and the length-prefixed frame codec.
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SessionStatus {
    Running,
    Exited { code: i32 },
    Gone,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMsg {
    StartSession {
        id: String,
        cwd: String,
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cols: u16,
        rows: u16,
    },
    Subscribe { id: String },
    Unsubscribe { id: String },
    Resize { id: String, cols: u16, rows: u16 },
    Capture { id: String, lines: usize },
    Status { id: String },
    Kill { id: String },
    List,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMsg {
    Started { id: String },
    Captured { id: String, text: String },
    Status { id: String, status: SessionStatus },
    List { sessions: Vec<(String, SessionStatus)> },
    Exited { id: String, code: i32 },
    Error { id: Option<String>, message: String },
}

#[derive(Debug)]
pub enum ClientFrame {
    Msg(ClientMsg),
    Input { id: String, bytes: Vec<u8> },
}

#[derive(Debug)]
pub enum ServerFrame {
    Msg(ServerMsg),
    Output { id: String, bytes: Vec<u8> },
    Snapshot { id: String, cols: u16, rows: u16, cx: u16, cy: u16, data: Vec<u8> },
}

const T_JSON: u8 = 0;
const T_OUTPUT: u8 = 1;
const T_INPUT: u8 = 2;
const T_SNAPSHOT: u8 = 3;

pub fn write_frame<W: Write>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    let len = payload.len() as u32;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut lenb = [0u8; 4];
    r.read_exact(&mut lenb)?;
    let len = u32::from_le_bytes(lenb) as usize;
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok(payload)
}

pub fn encode_json<M: Serialize>(m: &M) -> Vec<u8> {
    let mut out = vec![T_JSON];
    out.extend_from_slice(&serde_json::to_vec(m).expect("serialize"));
    out
}

fn encode_id_bytes(t: u8, id: &str, bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![t];
    out.extend_from_slice(&(id.len() as u32).to_le_bytes());
    out.extend_from_slice(id.as_bytes());
    out.extend_from_slice(bytes);
    out
}

pub fn encode_output(id: &str, bytes: &[u8]) -> Vec<u8> { encode_id_bytes(T_OUTPUT, id, bytes) }
pub fn encode_input(id: &str, bytes: &[u8]) -> Vec<u8> { encode_id_bytes(T_INPUT, id, bytes) }

pub fn encode_snapshot(id: &str, cols: u16, rows: u16, cx: u16, cy: u16, data: &[u8]) -> Vec<u8> {
    let mut out = vec![T_SNAPSHOT];
    out.extend_from_slice(&(id.len() as u32).to_le_bytes());
    out.extend_from_slice(id.as_bytes());
    out.extend_from_slice(&cols.to_le_bytes());
    out.extend_from_slice(&rows.to_le_bytes());
    out.extend_from_slice(&cx.to_le_bytes());
    out.extend_from_slice(&cy.to_le_bytes());
    out.extend_from_slice(data);
    out
}

fn err(msg: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, msg) }

fn split_id(rest: &[u8]) -> io::Result<(String, &[u8])> {
    if rest.len() < 4 { return Err(err("short id frame")); }
    let idlen = u32::from_le_bytes(rest[0..4].try_into().unwrap()) as usize;
    if rest.len() < 4 + idlen { return Err(err("truncated id")); }
    let id = String::from_utf8_lossy(&rest[4..4 + idlen]).to_string();
    Ok((id, &rest[4 + idlen..]))
}

pub fn decode_client(payload: &[u8]) -> io::Result<ClientFrame> {
    match payload.first().copied() {
        Some(T_JSON) => Ok(ClientFrame::Msg(
            serde_json::from_slice(&payload[1..]).map_err(|e| err(&e.to_string()))?,
        )),
        Some(T_INPUT) => {
            let (id, bytes) = split_id(&payload[1..])?;
            Ok(ClientFrame::Input { id, bytes: bytes.to_vec() })
        }
        _ => Err(err("unknown client frame type")),
    }
}

pub fn decode_server(payload: &[u8]) -> io::Result<ServerFrame> {
    match payload.first().copied() {
        Some(T_JSON) => Ok(ServerFrame::Msg(
            serde_json::from_slice(&payload[1..]).map_err(|e| err(&e.to_string()))?,
        )),
        Some(T_OUTPUT) => {
            let (id, bytes) = split_id(&payload[1..])?;
            Ok(ServerFrame::Output { id, bytes: bytes.to_vec() })
        }
        Some(T_SNAPSHOT) => {
            let (id, rest) = split_id(&payload[1..])?;
            if rest.len() < 8 { return Err(err("short snapshot header")); }
            let cols = u16::from_le_bytes(rest[0..2].try_into().unwrap());
            let rows = u16::from_le_bytes(rest[2..4].try_into().unwrap());
            let cx = u16::from_le_bytes(rest[4..6].try_into().unwrap());
            let cy = u16::from_le_bytes(rest[6..8].try_into().unwrap());
            Ok(ServerFrame::Snapshot { id, cols, rows, cx, cy, data: rest[8..].to_vec() })
        }
        _ => Err(err("unknown server frame type")),
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agency-core protocol -- --nocapture`
Expected: PASS (4 tests).

- [ ] **Step 5: Enable module + commit**

Enable `pub mod protocol;` in `mod.rs`. Then:

```bash
git add crates/agency-core/src/term/protocol.rs crates/agency-core/src/term/mod.rs
git commit -m "feat(termd): IPC message enums and length-prefixed frame codec"
```

---

### Task 4: PTY process (generalized from `supervisor.rs`)

**Files:**
- Create: `crates/agency-core/src/term/pty.rs`
- Modify: `crates/agency-core/src/term/mod.rs` (enable `pub mod pty;`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `enum ProcStatus { Running, Exited(i32), Crashed }`
  - `struct PtyProcess` with `write_input(&self, &[u8]) -> Result<()>`, `resize(&self, rows: u16, cols: u16) -> Result<()>`, `status(&self) -> ProcStatus`
  - `spawn_pty<F: Fn(Vec<u8>) + Send + 'static, X: Fn(i32) + Send + 'static>(command: &str, args: &[String], cwd: &Path, env: &[(String,String)], cols: u16, rows: u16, on_output: F, on_exit: X) -> Result<PtyProcess>`
- Consumes: `portable-pty` (already a dep).

This is `supervisor.rs::spawn_agent` generalized to take command/args/env/size directly (the daemon spawns the agent command itself, not a `tmux attach`) and to fire an `on_exit(code)` callback so sessions can push `Exited` to subscribers.

- [ ] **Step 1: Write the failing test**

Create `crates/agency-core/src/term/pty.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn spawns_echo_and_reports_exit() {
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let p = spawn_pty(
            "/bin/echo",
            &["hi-there".to_string()],
            std::env::temp_dir().as_path(),
            &[],
            80,
            24,
            move |bytes| { let _ = tx.send(bytes); },
            move |code| { let _ = etx.send(code); },
        )
        .unwrap();

        let mut seen = String::new();
        while let Ok(chunk) = rx.recv_timeout(Duration::from_secs(2)) {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("hi-there") { break; }
        }
        assert!(seen.contains("hi-there"), "got: {seen:?}");
        assert_eq!(erx.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
        let _ = p;
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core pty -- --nocapture`
Expected: FAIL to compile.

- [ ] **Step 3: Implement PtyProcess**

Prepend to `crates/agency-core/src/term/pty.rs` (port of `supervisor.rs`, generalized):

```rust
//! One PTY-backed child process with reader + wait threads.
use anyhow::Result;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub enum ProcStatus {
    Running,
    Exited(i32),
    Crashed,
}

pub struct PtyProcess {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    status: Arc<Mutex<ProcStatus>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

impl PtyProcess {
    pub fn write_input(&self, data: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    }

    pub fn status(&self) -> ProcStatus {
        self.status.lock().unwrap().clone()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_pty<F, X>(
    command: &str,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    cols: u16,
    rows: u16,
    on_output: F,
    on_exit: X,
) -> Result<PtyProcess>
where
    F: Fn(Vec<u8>) + Send + 'static,
    X: Fn(i32) + Send + 'static,
{
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;

    let mut cmd = CommandBuilder::new(command);
    cmd.args(args);
    cmd.cwd(cwd);
    // Finder-launched bundles inherit no TERM/COLORTERM; default them unless the
    // caller overrides (matches the retired tmux.rs behavior).
    cmd.env("TERM", "xterm-256color");
    if !env.iter().any(|(k, _)| k == "COLORTERM") {
        cmd.env("COLORTERM", "truecolor");
    }
    for (k, v) in env {
        cmd.env(k, v);
    }

    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let killer = child.clone_killer();
    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let status = Arc::new(Mutex::new(ProcStatus::Running));

    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => on_output(buf[..n].to_vec()),
            }
        }
    });

    let status_for_wait = status.clone();
    std::thread::spawn(move || {
        let code = match child.wait() {
            Ok(es) => es.exit_code() as i32,
            Err(_) => {
                *status_for_wait.lock().unwrap() = ProcStatus::Crashed;
                on_exit(-1);
                return;
            }
        };
        *status_for_wait.lock().unwrap() = ProcStatus::Exited(code);
        on_exit(code);
    });

    Ok(PtyProcess {
        writer: Arc::new(Mutex::new(writer)),
        status,
        killer,
        master: pair.master,
    })
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agency-core pty -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Enable module + commit**

Enable `pub mod pty;`. Then:

```bash
git add crates/agency-core/src/term/pty.rs crates/agency-core/src/term/mod.rs
git commit -m "feat(termd): PTY process with reader/wait threads and exit callback"
```

---

### Task 5: Session (PTY + emulator + subscribers, lock discipline)

**Files:**
- Create: `crates/agency-core/src/term/session.rs`
- Modify: `crates/agency-core/src/term/mod.rs` (enable `pub mod session;`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `struct Session`
  - `Session::start(id: String, cwd: &Path, command: &str, args: &[String], env: &[(String,String)], cols: u16, rows: u16) -> Result<Arc<Session>>`
  - `Session::input(&self, &[u8])`, `Session::resize(&self, cols, rows)`, `Session::capture(&self, lines) -> String`, `Session::status(&self) -> SessionStatus`
  - `Session::subscribe(&self, client_id: u64, out: Sender<Vec<u8>>)` — sends a Snapshot frame into `out` and registers the subscriber, atomically under the emulator lock.
  - `Session::unsubscribe(&self, client_id: u64)`
  - `Session::kill(&self)`
- Consumes: `emulator::Emulator`, `pty::{spawn_pty, ProcStatus}`, `protocol::{SessionStatus, encode_output, encode_snapshot, encode_json, ServerMsg}`.

- [ ] **Step 1: Write the failing tests**

Create `crates/agency-core/src/term/session.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::protocol::{decode_server, ServerFrame};
    use std::sync::mpsc;
    use std::time::Duration;

    fn drain_until<P: Fn(&ServerFrame) -> bool>(
        rx: &mpsc::Receiver<Vec<u8>>,
        pred: P,
    ) -> ServerFrame {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Ok(frame) = rx.recv_timeout(Duration::from_millis(200)) {
                let decoded = decode_server(&frame).unwrap();
                if pred(&decoded) {
                    return decoded;
                }
            }
        }
        panic!("predicate never matched");
    }

    #[test]
    fn subscribe_delivers_a_snapshot_first() {
        let s = Session::start(
            "t1".into(),
            std::env::temp_dir().as_path(),
            "/bin/sh",
            &["-c".into(), "printf ready; sleep 2".into()],
            &[],
            80,
            24,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));

        let (tx, rx) = mpsc::channel();
        s.subscribe(1, tx);
        let snap = drain_until(&rx, |f| matches!(f, ServerFrame::Snapshot { .. }));
        match snap {
            ServerFrame::Snapshot { id, data, .. } => {
                assert_eq!(id, "t1");
                assert!(String::from_utf8_lossy(&data).contains("ready"));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn input_is_echoed_to_subscribers_and_capture() {
        let s = Session::start(
            "t2".into(),
            std::env::temp_dir().as_path(),
            "/bin/cat",
            &[],
            &[],
            80,
            24,
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        s.subscribe(1, tx);
        s.input(b"ping\n");
        drain_until(&rx, |f| {
            matches!(f, ServerFrame::Output { bytes, .. } if String::from_utf8_lossy(bytes).contains("ping"))
        });
        assert!(s.capture(5).contains("ping"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core session -- --nocapture`
Expected: FAIL to compile.

- [ ] **Step 3: Implement Session**

Prepend to `crates/agency-core/src/term/session.rs`:

```rust
//! A single terminal session: one PTY child, one emulator, N subscribers.
use crate::term::emulator::Emulator;
use crate::term::protocol::{encode_json, encode_output, encode_snapshot, ServerMsg, SessionStatus};
use crate::term::pty::{spawn_pty, ProcStatus, PtyProcess};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

type Subs = Arc<Mutex<HashMap<u64, Sender<Vec<u8>>>>>;

pub struct Session {
    id: String,
    pty: PtyProcess,
    emu: Arc<Mutex<Emulator>>,
    subs: Subs,
}

impl Session {
    pub fn start(
        id: String,
        cwd: &Path,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cols: u16,
        rows: u16,
    ) -> Result<Arc<Session>> {
        let emu = Arc::new(Mutex::new(Emulator::new(cols, rows)));
        let subs: Subs = Arc::new(Mutex::new(HashMap::new()));

        // Reader closure: feed the emulator, then fan raw bytes to subscribers.
        // Lock order is ALWAYS emu -> subs; `subscribe` uses the same order so a
        // newly-registered subscriber can never receive output before its snapshot.
        let emu_r = emu.clone();
        let subs_r = subs.clone();
        let id_out = id.clone();
        let on_output = move |bytes: Vec<u8>| {
            let mut e = emu_r.lock().unwrap();
            e.feed(&bytes);
            let frame = encode_output(&id_out, &bytes);
            for tx in subs_r.lock().unwrap().values() {
                let _ = tx.send(frame.clone());
            }
        };

        let subs_x = subs.clone();
        let id_exit = id.clone();
        let on_exit = move |code: i32| {
            let frame = encode_json(&ServerMsg::Exited { id: id_exit.clone(), code });
            for tx in subs_x.lock().unwrap().values() {
                let _ = tx.send(frame.clone());
            }
        };

        let pty = spawn_pty(command, args, cwd, env, cols, rows, on_output, on_exit)?;
        Ok(Arc::new(Session { id, pty, emu, subs }))
    }

    pub fn input(&self, bytes: &[u8]) {
        let _ = self.pty.write_input(bytes);
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.pty.resize(rows, cols);
        self.emu.lock().unwrap().resize(cols, rows);
    }

    pub fn capture(&self, lines: usize) -> String {
        self.emu.lock().unwrap().capture(lines)
    }

    pub fn status(&self) -> SessionStatus {
        match self.pty.status() {
            ProcStatus::Running => SessionStatus::Running,
            ProcStatus::Exited(code) => SessionStatus::Exited { code },
            ProcStatus::Crashed => SessionStatus::Exited { code: -1 },
        }
    }

    /// Register a subscriber and hand it the current screen as a snapshot,
    /// atomically: the emulator lock is held across snapshot + registration +
    /// snapshot-send so no output frame can jump ahead of the snapshot.
    pub fn subscribe(&self, client_id: u64, out: Sender<Vec<u8>>) {
        let emu = self.emu.lock().unwrap();
        let snap = emu.snapshot();
        let frame = encode_snapshot(&self.id, snap.cols, snap.rows, snap.cx, snap.cy, &snap.data);
        let mut subs = self.subs.lock().unwrap();
        let _ = out.send(frame);
        subs.insert(client_id, out);
    }

    pub fn unsubscribe(&self, client_id: u64) {
        self.subs.lock().unwrap().remove(&client_id);
    }

    pub fn kill(&self) {
        // Dropping the PtyProcess kills the child; sessions are removed from the
        // registry by the caller. Killing here is the explicit teardown path.
        let _ = self.pty.write_input(&[]); // no-op flush; real kill is on drop
    }
}
```

Note: `kill()` semantics live in the registry (it drops the `Arc<Session>`), so `Session::kill` stays minimal. The registry test in Task 6 covers teardown.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agency-core session -- --nocapture`
Expected: PASS (2 tests).

- [ ] **Step 5: Enable module + commit**

Enable `pub mod session;`. Then:

```bash
git add crates/agency-core/src/term/session.rs crates/agency-core/src/term/mod.rs
git commit -m "feat(termd): session ties PTY+emulator+subscribers with snapshot-on-subscribe"
```

---

### Task 6: Registry (session map, idle-exit policy)

**Files:**
- Create: `crates/agency-core/src/term/registry.rs`
- Modify: `crates/agency-core/src/term/mod.rs` (enable `pub mod registry;`)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `struct Registry` (use via `Arc<Registry>`)
  - `Registry::new() -> Arc<Registry>`
  - `Registry::start(&self, id, cwd, command, args, env, cols, rows) -> Result<()>`
  - `Registry::get(&self, id: &str) -> Option<Arc<Session>>`
  - `Registry::list(&self) -> Vec<(String, SessionStatus)>`
  - `Registry::kill(&self, id: &str)` (removes + drops the session, killing the child)
  - `Registry::kill_all(&self)`
  - `Registry::running_count(&self) -> usize`
- Consumes: `session::Session`, `protocol::SessionStatus`.

- [ ] **Step 1: Write the failing tests**

Create `crates/agency-core/src/term/registry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sleeper(reg: &Registry, id: &str) {
        reg.start(
            id.to_string(),
            std::env::temp_dir().as_path(),
            "/bin/sh",
            &["-c".to_string(), "sleep 5".to_string()],
            &[],
            80,
            24,
        )
        .unwrap();
    }

    #[test]
    fn start_list_kill() {
        let reg = Registry::new();
        sleeper(&reg, "a");
        sleeper(&reg, "b");
        assert_eq!(reg.list().len(), 2);
        assert_eq!(reg.running_count(), 2);
        reg.kill("a");
        assert_eq!(reg.list().len(), 1);
        assert!(reg.get("a").is_none());
        assert!(reg.get("b").is_some());
        reg.kill_all();
        assert_eq!(reg.list().len(), 0);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core registry -- --nocapture`
Expected: FAIL to compile.

- [ ] **Step 3: Implement Registry**

Prepend to `crates/agency-core/src/term/registry.rs`:

```rust
//! Owns all live sessions and the idle-exit accounting.
use crate::term::protocol::SessionStatus;
use crate::term::session::Session;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct Registry {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

impl Registry {
    pub fn new() -> Arc<Registry> {
        Arc::new(Registry { sessions: Mutex::new(HashMap::new()) })
    }

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
    ) -> Result<()> {
        let session = Session::start(id.clone(), cwd, command, args, env, cols, rows)?;
        self.sessions.lock().unwrap().insert(id, session);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(id).cloned()
    }

    pub fn list(&self) -> Vec<(String, SessionStatus)> {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .map(|(id, s)| (id.clone(), s.status()))
            .collect()
    }

    pub fn kill(&self, id: &str) {
        // Removing the Arc drops the Session (and its PtyProcess), killing the child.
        self.sessions.lock().unwrap().remove(id);
    }

    pub fn kill_all(&self) {
        self.sessions.lock().unwrap().clear();
    }

    pub fn running_count(&self) -> usize {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .filter(|s| matches!(s.status(), SessionStatus::Running))
            .count()
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agency-core registry -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Enable module + commit**

Enable `pub mod registry;`. Then:

```bash
git add crates/agency-core/src/term/registry.rs crates/agency-core/src/term/mod.rs
git commit -m "feat(termd): session registry with start/list/kill and running_count"
```

---

### Task 7: Server (Unix socket accept loop, dispatch, fan-out)

**Files:**
- Create: `crates/agency-core/src/term/server.rs`
- Modify: `crates/agency-core/src/term/mod.rs` (enable `pub mod server;`)
- Test: deferred to Task 9's end-to-end integration test (the server is exercised through `TermClient`). Add a focused unit test here for connection-accounting.

**Interfaces:**
- Produces:
  - `server::run(socket_path: &Path) -> Result<()>` — binds the socket (removing a stale file), accepts forever, idle-exits the process when no clients are connected and no sessions are running for `IDLE_GRACE`.
  - `server::run_with_registry(socket_path: &Path, registry: Arc<Registry>, clients: Arc<AtomicUsize>) -> Result<()>` — same, injectable for tests (does NOT call `process::exit`; returns when the listener is dropped).
- Consumes: `registry::Registry`, `protocol::*`, `session::Session`.

- [ ] **Step 1: Implement the server**

Create `crates/agency-core/src/term/server.rs`:

```rust
//! Unix-socket front end: one accept loop, one reader + one writer thread per
//! client connection, dispatch into the shared Registry, fan output back.
use crate::term::protocol::*;
use crate::term::registry::Registry;
use anyhow::Result;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::Duration;

const IDLE_GRACE: Duration = Duration::from_secs(45);

pub fn run(socket_path: &Path) -> Result<()> {
    let registry = Registry::new();
    let clients = Arc::new(AtomicUsize::new(0));
    spawn_idle_watch(registry.clone(), clients.clone());
    run_with_registry(socket_path, registry, clients)
}

/// Exits the process once no client has been connected and no session has been
/// running for IDLE_GRACE — bounds the post-crash "ghost" window.
fn spawn_idle_watch(registry: Arc<Registry>, clients: Arc<AtomicUsize>) {
    std::thread::spawn(move || {
        let mut idle_since: Option<std::time::Instant> = None;
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let idle = clients.load(Ordering::SeqCst) == 0 && registry.running_count() == 0;
            match (idle, idle_since) {
                (true, None) => idle_since = Some(std::time::Instant::now()),
                (true, Some(t)) if t.elapsed() >= IDLE_GRACE => std::process::exit(0),
                (true, Some(_)) => {}
                (false, _) => idle_since = None,
            }
        }
    });
}

pub fn run_with_registry(
    socket_path: &Path,
    registry: Arc<Registry>,
    clients: Arc<AtomicUsize>,
) -> Result<()> {
    let _ = std::fs::remove_file(socket_path); // clear stale socket
    if let Some(dir) = socket_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = UnixListener::bind(socket_path)?;
    let next_id = Arc::new(AtomicU64::new(1));

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let client_id = next_id.fetch_add(1, Ordering::SeqCst);
        let registry = registry.clone();
        let clients = clients.clone();
        clients.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            handle_client(stream, client_id, registry.clone());
            // On disconnect: drop this client's subscriptions everywhere.
            for (id, _) in registry.list() {
                if let Some(s) = registry.get(&id) {
                    s.unsubscribe(client_id);
                }
            }
            clients.fetch_sub(1, Ordering::SeqCst);
        });
    }
    Ok(())
}

fn handle_client(stream: UnixStream, client_id: u64, registry: Arc<Registry>) {
    let mut read_half = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut write_half = stream;

    // Writer thread: drain this client's outbound channel to the socket.
    let (out_tx, out_rx) = channel::<Vec<u8>>();
    std::thread::spawn(move || {
        while let Ok(frame) = out_rx.recv() {
            if write_frame(&mut write_half, &frame).is_err() {
                break;
            }
        }
    });

    loop {
        let payload = match read_frame(&mut read_half) {
            Ok(p) => p,
            Err(_) => break, // clean disconnect or codec error -> end connection
        };
        match decode_client(&payload) {
            Ok(frame) => dispatch(frame, client_id, &registry, &out_tx),
            Err(e) => {
                let _ = out_tx.send(encode_json(&ServerMsg::Error {
                    id: None,
                    message: format!("bad frame: {e}"),
                }));
            }
        }
    }
}

fn dispatch(frame: ClientFrame, client_id: u64, registry: &Arc<Registry>, out: &Sender<Vec<u8>>) {
    let reply = |m: ServerMsg| {
        let _ = out.send(encode_json(&m));
    };
    match frame {
        ClientFrame::Msg(ClientMsg::StartSession { id, cwd, command, args, env, cols, rows }) => {
            match registry.start(id.clone(), Path::new(&cwd), &command, &args, &env, cols, rows) {
                Ok(()) => reply(ServerMsg::Started { id }),
                Err(e) => reply(ServerMsg::Error { id: Some(id), message: e.to_string() }),
            }
        }
        ClientFrame::Msg(ClientMsg::Subscribe { id }) => match registry.get(&id) {
            Some(s) => s.subscribe(client_id, out.clone()),
            None => reply(ServerMsg::Error { id: Some(id), message: "no such session".into() }),
        },
        ClientFrame::Msg(ClientMsg::Unsubscribe { id }) => {
            if let Some(s) = registry.get(&id) {
                s.unsubscribe(client_id);
            }
        }
        ClientFrame::Msg(ClientMsg::Resize { id, cols, rows }) => {
            if let Some(s) = registry.get(&id) {
                s.resize(cols, rows);
            }
        }
        ClientFrame::Msg(ClientMsg::Capture { id, lines }) => match registry.get(&id) {
            Some(s) => reply(ServerMsg::Captured { id: id.clone(), text: s.capture(lines) }),
            None => reply(ServerMsg::Captured { id, text: String::new() }),
        },
        ClientFrame::Msg(ClientMsg::Status { id }) => {
            let status = registry.get(&id).map(|s| s.status()).unwrap_or(SessionStatus::Gone);
            reply(ServerMsg::Status { id, status });
        }
        ClientFrame::Msg(ClientMsg::Kill { id }) => registry.kill(&id),
        ClientFrame::Msg(ClientMsg::List) => reply(ServerMsg::List { sessions: registry.list() }),
        ClientFrame::Msg(ClientMsg::Shutdown) => {
            registry.kill_all();
            std::process::exit(0);
        }
        ClientFrame::Input { id, bytes } => {
            if let Some(s) = registry.get(&id) {
                s.input(&bytes);
            }
        }
    }
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p agency-core`
Expected: compiles. (End-to-end behavior is verified in Task 9.)

- [ ] **Step 3: Enable module + commit**

Enable `pub mod server;`. Then:

```bash
git add crates/agency-core/src/term/server.rs crates/agency-core/src/term/mod.rs
git commit -m "feat(termd): Unix-socket server with per-client dispatch and fan-out"
```

---

### Task 8: Daemon binary (self-daemonize + run)

**Files:**
- Create: `crates/agency-core/src/bin/agency-termd.rs`

**Interfaces:**
- Consumes: `server::run`, `daemonize::Daemonize`.
- Produces: an `agency-termd` binary taking the socket path as `argv[1]`.

- [ ] **Step 1: Implement the binary**

Create `crates/agency-core/src/bin/agency-termd.rs`:

```rust
//! The Agency terminal daemon. Usage: `agency-termd <socket-path>`.
//! Self-daemonizes (detaches from the launching app) then serves the socket.
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn main() {
    let sock = match std::env::args().nth(1) {
        Some(s) => PathBuf::from(s),
        None => {
            eprintln!("usage: agency-termd <socket-path>");
            std::process::exit(2);
        }
    };

    // If a daemon already owns this socket, do nothing.
    if UnixStream::connect(&sock).is_ok() {
        return;
    }
    let _ = std::fs::remove_file(&sock);

    // Detach BEFORE spawning any threads (fork + threads do not mix).
    daemonize::Daemonize::new()
        .working_directory(std::env::temp_dir())
        .start()
        .expect("daemonize");

    if let Err(e) = agency_core::term::server::run(&sock) {
        eprintln!("agency-termd exited: {e}");
        std::process::exit(1);
    }
}
```

- [ ] **Step 2: Build the binary**

Run: `cargo build -p agency-core --bin agency-termd`
Expected: produces `target/debug/agency-termd`.

- [ ] **Step 3: Manual smoke (optional but recommended)**

Run:
```bash
target/debug/agency-termd /tmp/agency-smoke.sock
sleep 1
test -S /tmp/agency-smoke.sock && echo "socket up"
```
Expected: prints `socket up` (daemon detached and bound). It will idle-exit after ~45s.

- [ ] **Step 4: Commit**

```bash
git add crates/agency-core/src/bin/agency-termd.rs
git commit -m "feat(termd): agency-termd binary with self-daemonization"
```

---

## Phase 2 — App client integration (`agency-core` client + `agency-app`)

### Task 9: TermClient (connect/spawn, demux, request/reply) + end-to-end test

**Files:**
- Create: `crates/agency-core/src/term/client.rs`
- Modify: `crates/agency-core/src/term/mod.rs` (enable `pub mod client;`)
- Test: `crates/agency-core/tests/termd.rs`

**Interfaces:**
- Produces:
  - `struct TermClient`
  - `TermClient::connect_or_spawn(socket_path: PathBuf, daemon_bin: PathBuf) -> Result<TermClient>`
  - `TermClient::start_session(&self, id, cwd: &Path, command: &str, args: &[String], env: &[(String,String)], cols: u16, rows: u16) -> Result<()>`
  - `TermClient::subscribe<F: Fn(Vec<u8>) + Send + 'static>(&self, id: &str, cols: u16, rows: u16, on_output: F) -> Result<Subscription>`
  - `TermClient::input(&self, id, bytes: &[u8]) -> Result<()>`, `send_text(&self, id, text: &str) -> Result<()>`
  - `TermClient::resize(&self, id, cols, rows) -> Result<()>`
  - `TermClient::capture(&self, id, lines) -> Result<String>`
  - `TermClient::status(&self, id) -> Result<SessionStatus>`
  - `TermClient::kill(&self, id) -> Result<()>`, `list(&self) -> Result<Vec<(String,SessionStatus)>>`, `shutdown(&self) -> Result<()>`
  - `struct Subscription` whose `Drop` sends `Unsubscribe` and deregisters the callback.
- Consumes: `protocol::*`.

**Design:** one `UnixStream`; a background reader thread demuxes server frames. `Output`/`Snapshot`/`Exited` route to the per-session callback map; request replies (`Captured`/`Status`/`List`/`Started`/`Error`) go to a single reply channel. Blocking calls hold a `req` mutex (one outstanding request at a time → trivial correlation) and `recv` the reply.

- [ ] **Step 1: Write the failing end-to-end test**

Create `crates/agency-core/tests/termd.rs`:

```rust
use agency_core::term::client::TermClient;
use agency_core::term::registry::Registry;
use agency_core::term::server;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

/// Spin the server in-process (no daemonize) on a temp socket and return a client.
fn server_and_client() -> (tempfile::TempDir, TermClient) {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("termd.sock");
    let registry = Registry::new();
    let clients = Arc::new(AtomicUsize::new(0));
    {
        let sock = sock.clone();
        std::thread::spawn(move || {
            let _ = server::run_with_registry(&sock, registry, clients);
        });
    }
    // Wait for the socket to appear.
    for _ in 0..50 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let client = TermClient::connect_or_spawn(sock, std::path::PathBuf::from("/nonexistent")).unwrap();
    (dir, client)
}

#[test]
fn start_subscribe_input_capture_kill() {
    let (_dir, client) = server_and_client();
    client
        .start_session("r1", std::env::temp_dir().as_path(), "/bin/cat", &[], &[], 80, 24)
        .unwrap();

    let (tx, rx) = mpsc::channel();
    let sub = client
        .subscribe("r1", 80, 24, move |bytes| {
            let _ = tx.send(bytes);
        })
        .unwrap();

    // First frame delivered to the callback is the snapshot.
    let first = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(first.windows(2).any(|w| w == b"\x1b["), "snapshot should contain escapes");

    client.input("r1", b"echo-me\n").unwrap();
    let mut seen = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if let Ok(b) = rx.recv_timeout(Duration::from_millis(200)) {
            seen.push_str(&String::from_utf8_lossy(&b));
            if seen.contains("echo-me") {
                break;
            }
        }
    }
    assert!(seen.contains("echo-me"), "got: {seen:?}");
    assert!(client.capture("r1", 5).unwrap().contains("echo-me"));

    assert_eq!(client.list().unwrap().len(), 1);
    drop(sub);
    client.kill("r1").unwrap();
    assert_eq!(client.list().unwrap().len(), 0);
}
```

Add `tempfile` to `[dev-dependencies]` if not present (it already is per `Cargo.toml`).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core --test termd -- --nocapture`
Expected: FAIL to compile (`TermClient` missing).

- [ ] **Step 3: Implement TermClient**

Create `crates/agency-core/src/term/client.rs`:

```rust
//! App-side client: one connection, demuxed output callbacks + request/reply.
use crate::term::protocol::*;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type OutputCb = Arc<dyn Fn(Vec<u8>) + Send + Sync>;

struct Shared {
    write: Mutex<UnixStream>,
    callbacks: Mutex<HashMap<String, OutputCb>>,
    reply_rx: Mutex<Receiver<ServerMsg>>,
    req: Mutex<()>, // serializes request/reply pairs
}

pub struct TermClient {
    shared: Arc<Shared>,
}

pub struct Subscription {
    id: String,
    shared: Arc<Shared>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.shared.callbacks.lock().unwrap().remove(&self.id);
        let _ = send_msg(&self.shared, &ClientMsg::Unsubscribe { id: self.id.clone() });
    }
}

fn send_msg(shared: &Shared, msg: &ClientMsg) -> Result<()> {
    let payload = encode_json(msg);
    let mut w = shared.write.lock().unwrap();
    write_frame(&mut *w, &payload).map_err(Into::into)
}

impl TermClient {
    pub fn connect_or_spawn(socket_path: PathBuf, daemon_bin: PathBuf) -> Result<TermClient> {
        let stream = match connect(&socket_path) {
            Some(s) => s,
            None => {
                // Spawn the daemon (it self-daemonizes) then retry with backoff.
                std::process::Command::new(&daemon_bin)
                    .arg(&socket_path)
                    .spawn()
                    .map_err(|e| anyhow!("spawn {daemon_bin:?}: {e}"))?;
                let mut found = None;
                for _ in 0..100 {
                    if let Some(s) = connect(&socket_path) {
                        found = Some(s);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                found.ok_or_else(|| anyhow!("daemon did not come up at {socket_path:?}"))?
            }
        };

        let read_half = stream.try_clone()?;
        let (reply_tx, reply_rx) = channel::<ServerMsg>();
        let shared = Arc::new(Shared {
            write: Mutex::new(stream),
            callbacks: Mutex::new(HashMap::new()),
            reply_rx: Mutex::new(reply_rx),
            req: Mutex::new(()),
        });

        spawn_reader(read_half, shared.clone(), reply_tx);
        Ok(TermClient { shared })
    }

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
        self.request(ClientMsg::StartSession {
            id: id.into(),
            cwd: cwd.to_string_lossy().into(),
            command: command.into(),
            args: args.to_vec(),
            env: env.to_vec(),
            cols,
            rows,
        })
        .map(|_| ())
    }

    pub fn subscribe<F>(&self, id: &str, cols: u16, rows: u16, on_output: F) -> Result<Subscription>
    where
        F: Fn(Vec<u8>) + Send + Sync + 'static,
    {
        self.shared
            .callbacks
            .lock()
            .unwrap()
            .insert(id.to_string(), Arc::new(on_output));
        send_msg(&self.shared, &ClientMsg::Resize { id: id.into(), cols, rows })?;
        send_msg(&self.shared, &ClientMsg::Subscribe { id: id.into() })?;
        Ok(Subscription { id: id.into(), shared: self.shared.clone() })
    }

    pub fn input(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let payload = encode_input(id, bytes);
        let mut w = self.shared.write.lock().unwrap();
        write_frame(&mut *w, &payload).map_err(Into::into)
    }

    pub fn send_text(&self, id: &str, text: &str) -> Result<()> {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(b'\r');
        self.input(id, &bytes)
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        send_msg(&self.shared, &ClientMsg::Resize { id: id.into(), cols, rows })
    }

    pub fn capture(&self, id: &str, lines: usize) -> Result<String> {
        match self.request(ClientMsg::Capture { id: id.into(), lines })? {
            ServerMsg::Captured { text, .. } => Ok(text),
            other => Err(anyhow!("unexpected reply: {other:?}")),
        }
    }

    pub fn status(&self, id: &str) -> Result<SessionStatus> {
        match self.request(ClientMsg::Status { id: id.into() })? {
            ServerMsg::Status { status, .. } => Ok(status),
            other => Err(anyhow!("unexpected reply: {other:?}")),
        }
    }

    pub fn kill(&self, id: &str) -> Result<()> {
        send_msg(&self.shared, &ClientMsg::Kill { id: id.into() })
    }

    pub fn list(&self) -> Result<Vec<(String, SessionStatus)>> {
        match self.request(ClientMsg::List)? {
            ServerMsg::List { sessions } => Ok(sessions),
            other => Err(anyhow!("unexpected reply: {other:?}")),
        }
    }

    pub fn shutdown(&self) -> Result<()> {
        send_msg(&self.shared, &ClientMsg::Shutdown)
    }

    /// Send a request and block for the next reply. `req` serializes callers so
    /// the single reply channel never mixes responses.
    fn request(&self, msg: ClientMsg) -> Result<ServerMsg> {
        let _guard = self.shared.req.lock().unwrap();
        send_msg(&self.shared, &msg)?;
        let rx = self.shared.reply_rx.lock().unwrap();
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(ServerMsg::Error { message, .. }) => Err(anyhow!(message)),
            Ok(m) => Ok(m),
            Err(_) => Err(anyhow!("daemon reply timeout")),
        }
    }
}

fn connect(path: &Path) -> Option<UnixStream> {
    UnixStream::connect(path).ok()
}

fn spawn_reader(mut read_half: UnixStream, shared: Arc<Shared>, reply_tx: Sender<ServerMsg>) {
    std::thread::spawn(move || loop {
        let payload = match read_frame(&mut read_half) {
            Ok(p) => p,
            Err(_) => break, // daemon gone; higher layer surfaces via failed requests
        };
        match decode_server(&payload) {
            Ok(ServerFrame::Output { id, bytes }) => fire(&shared, &id, bytes),
            Ok(ServerFrame::Snapshot { id, data, .. }) => fire(&shared, &id, data),
            Ok(ServerFrame::Msg(ServerMsg::Exited { id, code })) => {
                // Surface exit as an empty-output tick; status() reports the code.
                let _ = code;
                let _ = id;
            }
            Ok(ServerFrame::Msg(m)) => {
                let _ = reply_tx.send(m);
            }
            Err(_) => break,
        }
    });
}

fn fire(shared: &Arc<Shared>, id: &str, bytes: Vec<u8>) {
    let cb = shared.callbacks.lock().unwrap().get(id).cloned();
    if let Some(cb) = cb {
        cb(bytes);
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agency-core --test termd -- --nocapture`
Expected: PASS. Run the whole crate too: `cargo test -p agency-core`.

- [ ] **Step 5: Enable module + commit**

Enable `pub mod client;`. Then:

```bash
git add crates/agency-core/src/term/client.rs crates/agency-core/src/term/mod.rs crates/agency-core/tests/termd.rs
git commit -m "feat(termd): TermClient (connect/spawn, demux, request/reply) + e2e test"
```

---

### Task 10: Swap `state.rs` from `Tmux` to `TermClient`

**Files:**
- Modify: `crates/agency-app/src/state.rs`
- Modify: `crates/agency-app/Cargo.toml` (no new dep; `agency-core` already a path dep)

**Interfaces:**
- Consumes: `agency_core::term::client::{TermClient, Subscription}`, `agency_core::term::SessionStatus`.
- Note: this task changes only the backend wiring; the `commands.rs` ↔ UI transport (base64 over Tauri `Channel`) is unchanged because `subscribe`'s callback delivers the same `Vec<u8>` the old `attach` callback did.

- [ ] **Step 1: Locate the daemon binary + socket path helpers**

Add near the top of `state.rs` (adjust imports to match the file):

```rust
use agency_core::term::client::{Subscription, TermClient};
use agency_core::term::SessionStatus;

/// Socket + daemon-binary paths derived from the app data dir.
fn termd_socket(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("termd.sock")
}

fn termd_bin() -> std::path::PathBuf {
    // The daemon ships next to the app executable (Tauri externalBin sidecar in
    // release; cargo target dir in dev).
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("agency-termd")))
        .unwrap_or_else(|| std::path::PathBuf::from("agency-termd"))
}
```

- [ ] **Step 2: Replace the `Tmux` field and constructor**

In `AppState` (around `state.rs:225-262`), replace the `tmux: Tmux` field with:

```rust
    term: TermClient,
```

and replace the `tmux: Tmux::resolved(),` initializer (around `state.rs:262`) with (the data dir is available where the db path is built — pass it through `AppState::new`):

```rust
    term: TermClient::connect_or_spawn(termd_socket(data_dir), termd_bin())?,
```

If `AppState::new` does not currently receive a `data_dir`, thread it in (the caller in `lib.rs:20` already has `data_dir`). Update the signature to `pub fn new(db_path: &Path, data_dir: &Path) -> Result<AppState>` and the call site.

Replace the `attaches`/`run_attaches` map value types from `AgentHandle` to `Subscription`:

```rust
    attaches: Mutex<HashMap<String, Subscription>>,
    run_attaches: Mutex<HashMap<String, Subscription>>,
```

- [ ] **Step 3: Rewrite the session methods**

Replace each `self.tmux.*` call site (enumerated from `state.rs` grep) as follows. Show the new bodies:

`start_session` sites (`:458`, `:494`, `:682`, `:757`) — was `self.tmux.start_session(&name, &cwd, &command, &args, &env)?;` becomes:

```rust
self.term.start_session(&name, &cwd, &command, &args, &env, 220, 50)?;
```

(Use `220, 50` to match the previous fixed initial size; resize follows on attach.)

`attach_run` (`:523-530`) — was `let handle = self.tmux.attach(&session_name(id), on_output)?;` becomes:

```rust
pub fn attach_run<F>(&self, id: &str, cols: u16, rows: u16, on_output: F) -> Result<()>
where
    F: Fn(Vec<u8>) + Send + Sync + 'static,
{
    let sub = self.term.subscribe(&session_name(id), cols, rows, on_output)?;
    self.attaches.lock().unwrap().insert(id.to_string(), sub);
    Ok(())
}
```

`detach_run` (`:533-541`) — was `detach_clients` + handle drop, becomes just dropping the `Subscription` (its `Drop` sends `Unsubscribe`):

```rust
pub fn detach_run(&self, id: &str) {
    self.attaches.lock().unwrap().remove(id);
}
```

Apply the identical transformation to `attach_run_script`/`detach_run_script` (`:708-720`) using `run_attaches` and `run_session_name(id)`.

`capture` sites (`:578`, `:705`, `:979`) — was `self.tmux.capture(&name, lines)` becomes:

```rust
self.term.capture(&name, lines).unwrap_or_default()
```

`session_status` sites (`:401`, `:520`, `:700`, `:977-978`) — was `self.tmux.session_status(&name).unwrap_or(SessionStatus::Gone)` becomes:

```rust
self.term.status(&name).unwrap_or(SessionStatus::Gone)
```

`kill_session` sites (`:356`, `:358`, `:368`, `:370`, `:585`, `:587`, `:606`, `:608`, `:643`, `:645`, `:672`, `:681`, `:693`, `:756`) — was `self.tmux.kill_session(&name).ok();` becomes:

```rust
let _ = self.term.kill(&name);
```

`send_text` (`:952`) — was `self.tmux.send_text(&session_name(run_id), &message)?;` becomes:

```rust
self.term.send_text(&session_name(run_id), &message)?;
```

The `run_input`/`resize_run` handlers (which previously called `AgentHandle::write_input`/`resize` on the stored handle) now go straight to the client by id:

```rust
pub fn run_input(&self, id: &str, bytes: &[u8]) -> Result<()> {
    self.term.input(&session_name(id), bytes)
}

pub fn resize_run(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
    self.term.resize(&session_name(id), cols, rows)
}
```

(Apply the same to the `*_script` variants with `run_session_name`.) Update `commands.rs` callers if the `attach_run` signature gained `cols, rows` — pass the terminal's initial size from the frontend (the `FocusTerminal` already knows its `FitAddon` dims; thread them through the existing attach command). Where the frontend does not yet send size, default to `220, 50` and rely on the immediate post-attach resize.

- [ ] **Step 4: Add startup rehydration**

In `AppState::new`, after constructing `term`, reconcile DB runs with live daemon sessions:

```rust
// Rehydrate: any run the daemon still hosts is adopted as-is; the watch loop
// (watch_snapshot) then reports live status. Nothing to spawn here — surviving
// sessions are already running in the daemon.
if let Ok(sessions) = state.term.list() {
    log::info!("termd: adopted {} surviving session(s)", sessions.len());
}
```

(Place after `state` is built; `watch_snapshot` already polls `status`/`capture`, so no further wiring is needed for the UI to show adopted runs.)

- [ ] **Step 5: Build**

Run: `cargo build -p agency-app`
Expected: compiles. Fix any remaining `self.tmux` references the grep missed (search: `rg "self\.tmux" crates/agency-app`). Expected after fixes: zero matches.

- [ ] **Step 6: Commit**

```bash
git add crates/agency-app/src/state.rs
git commit -m "feat(termd): drive sessions through TermClient instead of tmux"
```

---

### Task 11: Delete tmux + supervisor; replace test file; green build

**Files:**
- Delete: `crates/agency-core/src/tmux.rs`, `crates/agency-core/src/supervisor.rs`, `crates/agency-core/tests/tmux.rs`
- Modify: `crates/agency-core/src/lib.rs` (remove `pub mod tmux;` / `pub mod supervisor;`)
- Modify: any remaining imports of `agency_core::tmux::*` or `supervisor::*`

- [ ] **Step 1: Remove the modules**

```bash
git rm crates/agency-core/src/tmux.rs crates/agency-core/src/supervisor.rs crates/agency-core/tests/tmux.rs
```

In `crates/agency-core/src/lib.rs` remove the `pub mod tmux;` and `pub mod supervisor;` lines.

- [ ] **Step 2: Fix dangling references**

Run: `rg "tmux|supervisor|AgentHandle|spawn_agent" crates/agency-core/src crates/agency-app/src`
Replace any remaining references:
- `agency_core::tmux::SessionStatus` → `agency_core::term::SessionStatus`
- `notifier`'s `SessionStatus` import (if it pulled from `tmux`) → `agency_core::term::SessionStatus`
- Remove the `AgentProfile`-based attach path if `profile.rs` referenced it (the daemon spawns commands directly; keep `AgentProfile` only if still used for run configuration).

- [ ] **Step 3: Full workspace build + test**

Run:
```bash
cargo build
cargo test -p agency-core
```
Expected: workspace compiles; `agency-core` tests pass (including `tests/termd.rs`). No `tmux` symbols remain.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "refactor(termd): remove tmux + supervisor modules, swap SessionStatus import"
```

---

## Phase 3 — App lifecycle / menu bar (`agency-app`, Tauri)

### Task 12: Menu bar status item + window-close-to-tray

**Files:**
- Modify: `crates/agency-app/src/lib.rs`
- Modify: `crates/agency-app/Cargo.toml`

**Interfaces:**
- Consumes: Tauri 2 `tray` + `menu` APIs.
- Produces: app stays alive on window close, showing a menu bar item with "Open Agency" / "Quit Agency".

- [ ] **Step 1: Enable the tray feature**

In `crates/agency-app/Cargo.toml` set:

```toml
tauri = { version = "2", features = ["tray-icon"] }
```

- [ ] **Step 2: Intercept window close → hide to tray**

In the Tauri builder setup in `lib.rs`, register a window-event handler that prevents close and hides the window instead:

```rust
.on_window_event(|window, event| {
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        // Don't quit — retreat to the menu bar. Quit happens only via the
        // tray "Quit Agency" item (Task 13).
        api.prevent_close();
        let _ = window.hide();
    }
})
```

- [ ] **Step 3: Build the tray + menu in `setup`**

Inside the Tauri `.setup(|app| { ... })` closure add:

```rust
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;

let open = MenuItemBuilder::with_id("open", "Open Agency").build(app)?;
let quit = MenuItemBuilder::with_id("quit", "Quit Agency").build(app)?;
let menu = MenuBuilder::new(app).items(&[&open, &quit]).build()?;

let _tray = TrayIconBuilder::new()
    .icon(app.default_window_icon().unwrap().clone())
    .menu(&menu)
    .on_menu_event(|app, event| match event.id().as_ref() {
        "open" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        "quit" => {
            // Wired to the confirmation flow in Task 13.
            crate::lifecycle::request_quit(app);
        }
        _ => {}
    })
    .build(app)?;
```

(Add `use tauri::Manager;` for `get_webview_window`. Create an empty `crate::lifecycle` module now with a stub `pub fn request_quit(app: &tauri::AppHandle) { app.exit(0); }` — replaced in Task 13.)

- [ ] **Step 4: Build + manual verify**

Run: `cargo build -p agency-app`, then launch the app (`pnpm tauri dev` from `ui/`, per the project's run skill). Close the main window.
Expected: window disappears, app keeps running, a menu bar icon is present; "Open Agency" re-shows the window.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app/src/lib.rs crates/agency-app/Cargo.toml
git commit -m "feat(termd): menu bar status item; window close retreats to tray"
```

---

### Task 13: Quit confirmation → kill all + shutdown daemon

**Files:**
- Create: `crates/agency-app/src/lifecycle.rs`
- Modify: `crates/agency-app/src/lib.rs` (add `mod lifecycle;`)
- Modify: `crates/agency-app/Cargo.toml` (add `tauri-plugin-dialog`)

**Interfaces:**
- Produces: `lifecycle::request_quit(app: &AppHandle)` — shows a confirm dialog warning N agents will stop; on confirm, kills all sessions, sends daemon `Shutdown`, and exits.
- Consumes: `AppState::term` (for `list`/`kill`/`shutdown`), `tauri-plugin-dialog`.

- [ ] **Step 1: Add the dialog plugin**

In `crates/agency-app/Cargo.toml`:

```toml
tauri-plugin-dialog = "2"
```

Register it in the builder: `.plugin(tauri_plugin_dialog::init())`.

- [ ] **Step 2: Implement `request_quit`**

Create `crates/agency-app/src/lifecycle.rs`:

```rust
//! Quit flow: confirm, stop all agents, shut the daemon down, exit.
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

pub fn request_quit(app: &tauri::AppHandle) {
    let state = app.state::<crate::state::AppState>();
    let running = state.term.list().map(|s| s.len()).unwrap_or(0);

    let app2 = app.clone();
    let message = if running > 0 {
        format!("Quitting will stop {running} running agent(s). Continue?")
    } else {
        "Quit Agency?".to_string()
    };

    app.dialog()
        .message(message)
        .title("Quit Agency")
        .buttons(MessageDialogButtons::OkCancelCustom("Quit".into(), "Cancel".into()))
        .show(move |confirmed| {
            if confirmed {
                let state = app2.state::<crate::state::AppState>();
                // Kill every session, then tell the daemon to exit.
                if let Ok(sessions) = state.term.list() {
                    for (id, _) in sessions {
                        let _ = state.term.kill(&id);
                    }
                }
                let _ = state.term.shutdown();
                app2.exit(0);
            }
        });
}
```

- [ ] **Step 3: Wire `mod lifecycle;` and the tray "quit" handler**

In `lib.rs` add `mod lifecycle;` and ensure the tray menu's `"quit"` arm calls `crate::lifecycle::request_quit(app)` (already stubbed in Task 12 — remove the stub `request_quit` and rely on this module).

Also intercept `Cmd+Q` / app-level exit so it routes through the same confirmation. In the builder, handle the `RunEvent::ExitRequested`:

```rust
.build(tauri::generate_context!())?
.run(|app, event| {
    if let tauri::RunEvent::ExitRequested { api, .. } = event {
        // Route OS-level quit through our confirmation instead of exiting.
        api.prevent_exit();
        crate::lifecycle::request_quit(app);
    }
});
```

(Adjust to the existing `lib.rs` run structure; the key is `prevent_exit()` + `request_quit`.)

- [ ] **Step 4: Build + manual verify**

Run: `cargo build -p agency-app`, launch, start an agent, then choose "Quit Agency" from the tray (and separately try `Cmd+Q`).
Expected: a dialog warns "will stop 1 running agent(s)"; Cancel keeps the app running with the agent alive; Quit stops the agent, the daemon exits (`pgrep agency-termd` returns nothing), and the app closes.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-app/src/lifecycle.rs crates/agency-app/src/lib.rs crates/agency-app/Cargo.toml
git commit -m "feat(termd): Quit confirmation stops agents and shuts down the daemon"
```

---

### Task 14: Daemon sidecar bundling + crash-survival validation

**Files:**
- Modify: `crates/agency-app/tauri.conf.json` (bundle the daemon binary as an external sidecar)
- Modify: build tooling so `agency-termd` is built before the app bundle (document in the task)

**Interfaces:**
- Produces: a release bundle where `agency-termd` sits next to the app binary so `termd_bin()` (Task 10) resolves it.

- [ ] **Step 1: Add the sidecar to the bundle config**

In `crates/agency-app/tauri.conf.json` under `bundle`, add the external binary:

```json
"externalBin": ["../agency-core/target/release/agency-termd"]
```

(Match the actual built path; for the workspace layout the daemon binary is at `target/release/agency-termd`. If Tauri's sidecar naming requires a target-triple suffix, follow the Tauri 2 sidecar convention — copy/rename `agency-termd` to `agency-termd-<triple>` in a prebuild step.)

- [ ] **Step 2: Ensure the daemon is built first**

Document/automate: `cargo build -p agency-core --bin agency-termd --release` runs before `tauri build`. Add it to the existing app build script (the project's bundling task referenced in recent commits).

- [ ] **Step 3: Validate the three lifecycle paths (manual, REQUIRED)**

This is the load-bearing validation for the whole spec. With a real agent running:

1. **Close window** → agent keeps running; menu bar item present; "Open Agency" re-shows it streaming live. ✅
2. **Menu bar Quit** → confirmation shown; on confirm, `pgrep -f agency-termd` empty, agent process gone, app exited. ✅
3. **Hard crash** → `kill -9` the app process (not the daemon). Confirm `pgrep -f agency-termd` still present and the agent PID alive. Relaunch the app → it reconnects, `list()` adopts the session, the terminal re-shows current state via snapshot. ✅

Record results in the commit message.

- [ ] **Step 4: Commit**

```bash
git add crates/agency-app/tauri.conf.json
git commit -m "feat(termd): bundle agency-termd sidecar; validate crash-survival lifecycle"
```

---

## Phase 4 — UI scrollback + search (`ui`)

### Task 15: xterm scrollback config + SearchAddon + search box

**Files:**
- Modify: `ui/src/components/FocusTerminal.tsx`
- Modify: `ui/package.json`

**Interfaces:**
- Consumes: the snapshot now carries scrollback (Task 2/5/9), so xterm.js holds history in its own buffer; `SearchAddon` searches it.

- [ ] **Step 1: Add the search addon dependency**

In `ui/`:

```bash
pnpm add @xterm/addon-search
```

(Per the project memory: use pnpm, not npm.)

- [ ] **Step 2: Configure scrollback + load SearchAddon**

In `FocusTerminal.tsx` where the `Terminal` is constructed, set a large scrollback and load the addon:

```tsx
import { SearchAddon } from "@xterm/addon-search";

const term = new Terminal({
  scrollback: 10000, // match daemon SCROLLBACK so reattach history is searchable
  // ...existing options
});
const searchAddon = new SearchAddon();
term.loadAddon(searchAddon);
// keep a ref so the search box can call it
searchAddonRef.current = searchAddon;
```

- [ ] **Step 3: Add a minimal search box**

Add a small search input (toggled with Cmd/Ctrl+F over the terminal) that calls:

```tsx
const onSearch = (q: string, prev = false) => {
  const addon = searchAddonRef.current;
  if (!addon) return;
  prev ? addon.findPrevious(q) : addon.findNext(q);
};
```

Wire Enter → `findNext`, Shift+Enter → `findPrevious`, Escape → close. Match the existing component's styling conventions (follow the file's current patterns rather than introducing a new UI system).

- [ ] **Step 4: Build + manual verify**

Run the app (`pnpm tauri dev`). Run an agent that produces >1 screen of output, scroll up with the mouse (history present), open search, find an earlier string.
Expected: scrollback visible beyond the current screen; search highlights and jumps to matches; reattaching (navigate away + back) preserves the scrollback via the snapshot.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/FocusTerminal.tsx ui/package.json ui/pnpm-lock.yaml
git commit -m "feat(termd): xterm scrollback + search over reattach history"
```

---

### Task 16: Protocol version handshake

**Files:**
- Modify: `crates/agency-core/src/term/protocol.rs`
- Modify: `crates/agency-core/src/term/server.rs` (dispatch `Hello`)
- Modify: `crates/agency-core/src/term/client.rs` (handshake on connect)
- Test: `crates/agency-core/tests/termd.rs` (version-mismatch case)

**Interfaces:**
- Produces: `pub const PROTOCOL_VERSION: u32 = 1;`, `ClientMsg::Hello { version: u32 }`, `ServerMsg::Hello { version: u32 }`.
- Consumes: existing request/reply path.

The spec requires a `protocol_version` stamped in the handshake so a version
mismatch (only possible on a deliberate breaking change) is surfaced clearly
rather than misbehaving. v1 keeps it minimal: exchange + check, no hot-upgrade.

- [ ] **Step 1: Write the failing test**

Append to `crates/agency-core/tests/termd.rs`:

```rust
#[test]
fn handshake_reports_protocol_version() {
    let (_dir, client) = server_and_client();
    assert_eq!(client.daemon_version().unwrap(), agency_core::term::protocol::PROTOCOL_VERSION);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core --test termd handshake -- --nocapture`
Expected: FAIL to compile (`daemon_version` missing).

- [ ] **Step 3: Add the messages + constant**

In `protocol.rs` add near the top:

```rust
pub const PROTOCOL_VERSION: u32 = 1;
```

Add `Hello { version: u32 }` as a variant to BOTH `ClientMsg` and `ServerMsg`.

- [ ] **Step 4: Dispatch `Hello` in the server**

In `server.rs` `dispatch`, add an arm:

```rust
ClientFrame::Msg(ClientMsg::Hello { .. }) => {
    reply(ServerMsg::Hello { version: PROTOCOL_VERSION });
}
```

(Add `PROTOCOL_VERSION` to the `use crate::term::protocol::*;` glob — already covered.)

- [ ] **Step 5: Handshake on connect + expose version**

In `client.rs`, after the reader is spawned in `connect_or_spawn`, perform the handshake and store the version:

```rust
let client = TermClient { shared };
let version = client.daemon_version()?;
if version != PROTOCOL_VERSION {
    return Err(anyhow!(
        "termd protocol mismatch: app speaks {PROTOCOL_VERSION}, daemon speaks {version}. \
         Quit running agents and restart, or relaunch the previous app version."
    ));
}
Ok(client)
```

Add the method:

```rust
pub fn daemon_version(&self) -> Result<u32> {
    match self.request(ClientMsg::Hello { version: PROTOCOL_VERSION })? {
        ServerMsg::Hello { version } => Ok(version),
        other => Err(anyhow!("unexpected reply: {other:?}")),
    }
}
```

(Import `PROTOCOL_VERSION` via the existing `use crate::term::protocol::*;`.)

- [ ] **Step 6: Run to verify pass**

Run: `cargo test -p agency-core --test termd -- --nocapture`
Expected: PASS (all termd tests, including handshake).

- [ ] **Step 7: Commit**

```bash
git add crates/agency-core/src/term/protocol.rs crates/agency-core/src/term/server.rs crates/agency-core/src/term/client.rs crates/agency-core/tests/termd.rs
git commit -m "feat(termd): protocol version handshake with clear mismatch error"
```

---

### Task 17: Surface daemon disconnect + one respawn attempt

**Files:**
- Modify: `crates/agency-core/src/term/client.rs`
- Modify: `crates/agency-app/src/state.rs` (surface the error to the UI watch loop)

**Interfaces:**
- Produces: `TermClient::is_alive(&self) -> bool` (set false when the reader thread sees EOF), and `TermClient::connect_or_spawn` reused for a single respawn attempt.
- Consumes: existing reader thread.

The spec requires: on daemon disconnect, attempt one respawn, `List` on
reconnect, and surface the state explicitly — never a silent dead terminal.

- [ ] **Step 1: Track liveness**

In `client.rs` add an `alive: Arc<AtomicBool>` to `Shared` (init `true`). In `spawn_reader`, set it to `false` on the `break` paths (EOF / decode error). Add:

```rust
pub fn is_alive(&self) -> bool {
    self.shared.alive.load(std::sync::atomic::Ordering::SeqCst)
}
```

- [ ] **Step 2: Surface in the watch loop**

In `state.rs` `watch_snapshot` (`:971`), before polling sessions, check liveness and attempt a single respawn:

```rust
if !self.term.is_alive() {
    // Daemon dropped. Try once to reconnect; surface either way.
    match TermClient::connect_or_spawn(termd_socket(&self.data_dir), termd_bin()) {
        Ok(client) => {
            log::warn!("termd reconnected; adopting {} session(s)",
                client.list().map(|s| s.len()).unwrap_or(0));
            *self.term_slot.lock().unwrap() = client; // see Step 3
        }
        Err(e) => {
            log::error!("termd unavailable: {e}");
            // Emit a snapshot entry the UI renders as "terminal backend lost".
            return Ok(self.degraded_snapshot(&format!("Terminal backend lost: {e}")));
        }
    }
}
```

- [ ] **Step 3: Make `term` replaceable**

Change the `AppState` field from `term: TermClient` to `term: TermClient` wrapped for replacement — store it as `term_slot: Mutex<TermClient>` and add a helper `fn term(&self) -> std::sync::MutexGuard<'_, TermClient>` (or, simpler, keep `term: TermClient` and have a `Mutex<TermClient>` only if reconnection is exercised). Add `data_dir: PathBuf` to `AppState` so respawn can recompute the socket path. Update all `self.term.X(...)` call sites from Task 10 to `self.term().X(...)`.

Add `degraded_snapshot(&self, msg: &str) -> Vec<notifier::RunSnapshot>` returning a single synthetic snapshot carrying the message so the UI shows it instead of a blank terminal.

- [ ] **Step 4: Build + manual verify**

Run: `cargo build -p agency-app`. Launch with an agent, then `pkill -9 -f agency-termd` to simulate a daemon crash.
Expected: the watch loop respawns the daemon (or, if respawn fails, the UI shows "Terminal backend lost: …"); it never shows a silently-dead blank terminal.

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/term/client.rs crates/agency-app/src/state.rs
git commit -m "feat(termd): detect daemon disconnect, respawn once, surface failure"
```

---

## Final cutover checklist

- [ ] `rg "tmux" crates/ ui/src` returns only historical comments/spec references, no live code.
- [ ] `cargo build` (workspace) is green.
- [ ] `cargo test -p agency-core` is green (including `tests/termd.rs`).
- [ ] The three lifecycle paths in Task 14 Step 3 all pass.
- [ ] Scrollback + search work in the running app.
- [ ] Merge `feat/termd`.
