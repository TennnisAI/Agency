# Terminal + Resume Bug Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix three independent bugs found in manual testing — claude agents dying on resume, opencode run titles polluted by terminal escape sequences, and hundreds of blank lines above every terminal.

**Architecture:** (1) A proactive per-worktree session probe decides resume-vs-fresh for claude/pi before launching (the daemon's exit-based fallback can't help claude, which doesn't exit on resume-failure). (2) The first-prompt capture skips terminal escape sequences instead of only the ESC byte, plus the backend title sanitizer strips ANSI. (3) The snapshot serializer skips leading blank scrollback rows.

**Tech Stack:** Rust (`agency-app`, `agency-core`), TypeScript/React + vitest (`ui`).

## Global Constraints

- Branch: `feat/termd`. Never touch `main`.
- Fix 1 keeps the daemon early-exit fallback: only claude/pi (probeable) are gated by the probe; opencode/codex/copilot (`Unknown`) keep the existing resume-with-fallback; cursor/hermes (no `resume_args`) stay fresh.
- `resume_probe(home: &Path, command: &str, worktree: &Path) -> ResumeProbe` with `enum ResumeProbe { Has, None, Unknown }`. claude encoding = worktree path with every `/` and `.` replaced by `-` (leading `/` kept → leading `-`); pi encoding = strip leading `/`, replace `/`→`-` (dots kept), wrap `--`…`--`. `home` is injected for testability (real caller uses `$HOME`).
- No new crate dependencies (use `std::env`, `std::fs`). `tempfile` is already a dev-dependency of `agency-app` and `agency-core`.
- Run from `<home>/agency`: `cargo test -p agency-app`, `cargo test -p agency-core`. UI tests via the direct binary: `./node_modules/.bin/vitest run <file>` from `ui/` (per project memory, not `pnpm test`).

---

### Task 1: Claude/pi resume probe + `ensure_run_active` gating

**Files:**
- Create: `crates/agency-app/src/resume_probe.rs`
- Modify: `crates/agency-app/src/lib.rs` (add `mod resume_probe;`)
- Modify: `crates/agency-app/src/state.rs` (`ensure_run_active` agent branch)
- Test: inline `#[cfg(test)]` in `resume_probe.rs`

**Interfaces:**
- Produces: `enum ResumeProbe { Has, None, Unknown }`; `fn resume_probe(home: &Path, command: &str, worktree: &Path) -> ResumeProbe`.

- [ ] **Step 1: Write the failing probe tests**

Create `crates/agency-app/src/resume_probe.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn claude_probe_has_vs_none() {
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/Users/x/agency/.agency/worktrees/agent-abcd");
        // claude encodes '/' and '.' as '-' (leading '/' -> leading '-').
        let enc = "-Users-x-agency--agency-worktrees-agent-abcd";
        let dir = home.path().join(".claude").join("projects").join(enc);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(resume_probe(home.path(), "claude", wt), ResumeProbe::None); // empty dir
        fs::write(dir.join("s.jsonl"), "x").unwrap();
        assert_eq!(resume_probe(home.path(), "claude", wt), ResumeProbe::Has);
    }

    #[test]
    fn claude_probe_none_when_dir_absent() {
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/Users/x/agency/.agency/worktrees/agent-zzzz");
        assert_eq!(resume_probe(home.path(), "claude", wt), ResumeProbe::None);
    }

    #[test]
    fn pi_probe_encoding_and_detection() {
        let home = tempfile::tempdir().unwrap();
        let wt = Path::new("/Users/x/agency");
        // pi strips the leading '/', replaces '/' with '-' (dots kept), wraps in '--'..'--'.
        let enc = "--Users-x-agency--";
        let dir = home.path().join(".pi").join("agent").join("sessions").join(enc);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("session.jsonl"), "x").unwrap();
        assert_eq!(resume_probe(home.path(), "pi", wt), ResumeProbe::Has);
    }

    #[test]
    fn unknown_agents_are_unknown() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(resume_probe(home.path(), "opencode", Path::new("/x")), ResumeProbe::Unknown);
        assert_eq!(resume_probe(home.path(), "cursor-agent", Path::new("/x")), ResumeProbe::Unknown);
        assert_eq!(resume_probe(home.path(), "hermes", Path::new("/x")), ResumeProbe::Unknown);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-app --lib resume_probe -- --nocapture`
Expected: FAIL to compile (`resume_probe`/`ResumeProbe` missing). (After Step 3 + the `mod` line it will compile.)

- [ ] **Step 3: Implement the probe**

Prepend to `crates/agency-app/src/resume_probe.rs`:

```rust
//! Proactive per-agent check: does a resumable session exist for this worktree?
//! Used so we never launch a resume command (e.g. `claude --continue`) when there
//! is nothing to resume — claude does not exit on resume-failure in a PTY, so the
//! daemon's early-exit fallback cannot recover it.
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeProbe {
    /// A per-worktree session store exists and is non-empty.
    Has,
    /// We can check this agent and there is no session.
    None,
    /// No probe for this agent — caller should attempt resume (the daemon
    /// early-exit fallback covers agents that exit fast on resume-failure).
    Unknown,
}

/// `home` is the user's home dir (injected for testability). `command` is the
/// agent launch command (e.g. "claude", "pi"); only the basename is matched.
pub fn resume_probe(home: &Path, command: &str, worktree: &Path) -> ResumeProbe {
    let base = Path::new(command)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(command);
    match base {
        "claude" => dir_probe(&home.join(".claude").join("projects").join(claude_enc(worktree))),
        "pi" => dir_probe(&home.join(".pi").join("agent").join("sessions").join(pi_enc(worktree))),
        _ => ResumeProbe::Unknown,
    }
}

fn dir_probe(dir: &Path) -> ResumeProbe {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => {
            if entries.next().is_some() { ResumeProbe::Has } else { ResumeProbe::None }
        }
        Err(_) => ResumeProbe::None,
    }
}

/// Claude encodes a cwd by replacing every '/' and '.' with '-'.
fn claude_enc(worktree: &Path) -> String {
    worktree
        .to_string_lossy()
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
}

/// Pi strips a leading '/', replaces '/' with '-' (dots kept), wraps in '--'..'--'.
fn pi_enc(worktree: &Path) -> String {
    let s = worktree.to_string_lossy();
    let s = s.strip_prefix('/').unwrap_or(&s);
    let inner: String = s.chars().map(|c| if c == '/' { '-' } else { c }).collect();
    format!("--{inner}--")
}
```

- [ ] **Step 4: Wire the module**

In `crates/agency-app/src/lib.rs`, add (next to the other `mod` declarations):

```rust
mod resume_probe;
```

- [ ] **Step 5: Run the probe tests**

Run: `cargo test -p agency-app --lib resume_probe -- --nocapture`
Expected: PASS (4 tests).

- [ ] **Step 6: Gate `ensure_run_active` on the probe**

In `crates/agency-app/src/state.rs`, in `ensure_run_active`'s agent branch, replace the block that begins `let setup = config.scripts.setup.as_deref();` down through the `start_session_with_fallback(...)` call with:

```rust
        let setup = config.scripts.setup.as_deref();
        // Decide resume-vs-fresh up front. For claude/pi we can prove whether a
        // session exists (they don't exit on resume-failure, so the daemon
        // fallback can't save them); other resume-capable agents fall through to
        // the resume-with-fallback path (the fallback catches their fast exits).
        let probe = std::env::var_os("HOME")
            .map(|h| crate::resume_probe::resume_probe(std::path::Path::new(&h), &profile.command, &worktree))
            .unwrap_or(crate::resume_probe::ResumeProbe::Unknown);
        let use_resume =
            profile.resume_args.is_some() && probe != crate::resume_probe::ResumeProbe::None;
        let (command, args) = agent_argv(&profile, &run.prompt, use_resume, setup);
        let fallback = if use_resume {
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

- [ ] **Step 7: Build + run app tests**

Run: `cargo build -p agency-app` then `cargo test -p agency-app`.
Expected: clean build; all tests pass (existing `ensure_run_active` tests still green — for the `flaky` fake profile whose command is `/bin/sh`, the probe returns `Unknown`, so the resume-with-fallback path is unchanged).

- [ ] **Step 8: Commit**

```bash
git add crates/agency-app/src/resume_probe.rs crates/agency-app/src/lib.rs crates/agency-app/src/state.rs
git commit -m "fix(resume): proactive claude/pi session probe so resume never dead-ends"
```

---

### Task 2: Run title polluted by terminal escape sequences

**Files:**
- Modify: `ui/src/lib/firstPrompt.ts`
- Modify: `ui/src/lib/firstPrompt.test.ts`
- Modify: `crates/agency-core/src/title.rs`

**Interfaces:**
- `feed`/`initialCapture`/`CaptureState` keep their names; `CaptureState` gains an `esc` field.

- [ ] **Step 1: Write the failing frontend tests**

Add to `ui/src/lib/firstPrompt.test.ts` (it already imports `feed`, `initialCapture`):

```ts
test("skips an OSC color-report reply, captures real typing", () => {
  let s = initialCapture();
  // xterm.js reply to an OSC 11 query, terminated by ST (ESC \)
  s = feed(s, "\x1b]11;rgb:b3b3/bcbc/b2b2\x1b\\").state;
  const r = feed(s, "hi\r");
  expect(r.line).toBe("hi");
});

test("skips a CSI DECRQM reply", () => {
  let s = initialCapture();
  s = feed(s, "\x1b[?1016;2$y").state;
  const r = feed(s, "ok\r");
  expect(r.line).toBe("ok");
});

test("skips an escape sequence split across chunks", () => {
  let s = initialCapture();
  s = feed(s, "\x1b]11;rgb:b3b3").state; // first half of OSC
  s = feed(s, "/bcbc/b2b2\x07").state;   // rest + BEL terminator
  const r = feed(s, "go\r");
  expect(r.line).toBe("go");
});
```

- [ ] **Step 2: Run to verify failure**

Run (from `ui/`): `./node_modules/.bin/vitest run src/lib/firstPrompt.test.ts`
Expected: the new tests FAIL (the escape-sequence body currently leaks into the captured line).

- [ ] **Step 3: Implement the escape-skipping `feed`**

Replace the contents of `ui/src/lib/firstPrompt.ts` with:

```ts
type EscState = "none" | "esc" | "csi" | "osc" | "oscEsc";

export interface CaptureState {
  buf: string;
  done: boolean;
  esc: EscState;
}

export const initialCapture = (): CaptureState => ({ buf: "", done: false, esc: "none" });

// Feed a raw terminal-input chunk (xterm.js onData — typed keys AND automatic
// replies to terminal queries). Returns the (possibly updated) state and, when
// the first non-empty line is submitted, that trimmed line. Whole escape
// sequences (CSI / OSC / other) are skipped, not captured, so a terminal's
// query-replies never pollute the captured prompt.
export function feed(
  state: CaptureState,
  chunk: string,
): { state: CaptureState; line: string | null } {
  if (state.done) return { state, line: null };
  let buf = state.buf;
  let esc = state.esc;

  for (const ch of chunk) {
    const code = ch.codePointAt(0)!;

    // Inside an escape sequence: consume until its terminator.
    if (esc !== "none") {
      if (esc === "esc") {
        esc = ch === "[" ? "csi" : ch === "]" ? "osc" : "none"; // other ESC x = 2-byte
      } else if (esc === "csi") {
        if (code >= 0x40 && code <= 0x7e) esc = "none"; // CSI final byte
      } else if (esc === "osc") {
        if (code === 0x07) esc = "none"; // BEL terminator
        else if (ch === "\x1b") esc = "oscEsc"; // maybe ST (ESC \)
      } else if (esc === "oscEsc") {
        esc = ch === "\\" ? "none" : "esc"; // ESC \ ends OSC; else new ESC
      }
      continue;
    }

    if (ch === "\x1b") {
      esc = "esc";
    } else if (ch === "\r" || ch === "\n") {
      const line = buf.trim();
      if (line.length > 0) {
        return { state: { buf: "", done: true, esc: "none" }, line };
      }
      buf = ""; // empty submission — keep waiting
    } else if (ch === "\x7f" || ch === "\b") {
      buf = buf.slice(0, -1);
    } else if (code >= 0x20 && code !== 0x7f) {
      buf += ch;
    }
    // other control bytes are dropped
  }

  return { state: { buf, done: false, esc }, line: null };
}
```

- [ ] **Step 4: Run frontend tests**

Run (from `ui/`): `./node_modules/.bin/vitest run src/lib/firstPrompt.test.ts`
Expected: PASS (new tests + the file's existing tests). Then `./node_modules/.bin/tsc --noEmit` clean.

- [ ] **Step 5: Write the failing backend test (defense in depth)**

Add to the `#[cfg(test)] mod tests` in `crates/agency-core/src/title.rs`:

```rust
#[test]
fn sanitize_strips_escape_sequences_and_controls() {
    // An OSC reply + CSI reply with the ESC bytes intact must reduce to clean text.
    let raw = "\x1b]11;rgb:b3b3/bcbc/b2b2\x1b\\\x1b[?1016;2$yreal title";
    assert_eq!(sanitize_title(raw), "real title");
    // Pure escape noise (with ESC) reduces to empty.
    assert_eq!(sanitize_title("\x1b[?2027;0$y\x1b[?1004;h"), "");
}
```

- [ ] **Step 6: Run to verify failure**

Run: `cargo test -p agency-core title -- --nocapture`
Expected: the new test FAILS (current `sanitize_title` keeps the escape bodies).

- [ ] **Step 7: Harden `sanitize_title`**

In `crates/agency-core/src/title.rs`, add an ANSI/control stripper and call it first in `sanitize_title`:

```rust
pub fn sanitize_title(raw: &str) -> String {
    let cleaned = strip_ansi_and_controls(raw);
    let line = cleaned.lines().next().unwrap_or("").trim();
    let line = line.trim_matches(|c| c == '"' || c == '\'').trim();
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(60).collect()
}

/// Remove ESC-introduced escape sequences (CSI/OSC/other) and other C0 control
/// characters, keeping printable text and newlines.
fn strip_ansi_and_controls(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    while let Some(&n) = chars.peek() {
                        chars.next();
                        if ('\u{40}'..='\u{7e}').contains(&n) { break; }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(&n) = chars.peek() {
                        chars.next();
                        if n == '\u{7}' { break; }
                        if n == '\x1b' { chars.next(); break; } // ST (ESC \)
                    }
                }
                _ => { chars.next(); }
            }
            continue;
        }
        if c == '\n' { out.push('\n'); continue; }
        if c.is_control() { continue; }
        out.push(c);
    }
    out
}
```

- [ ] **Step 8: Run backend tests**

Run: `cargo test -p agency-core title -- --nocapture`
Expected: PASS (new test + existing title tests).

- [ ] **Step 9: Commit**

```bash
git add ui/src/lib/firstPrompt.ts ui/src/lib/firstPrompt.test.ts crates/agency-core/src/title.rs
git commit -m "fix(title): skip terminal escape sequences in first-prompt capture + sanitizer"
```

---

### Task 3: Hundreds of blank lines above every terminal

**Files:**
- Modify: `crates/agency-core/src/term/emulator.rs` (`snapshot`)
- Test: inline `#[cfg(test)]` in `emulator.rs`

**Interfaces:**
- `Emulator::snapshot(&self) -> Snapshot` unchanged signature.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `crates/agency-core/src/term/emulator.rs`:

```rust
#[test]
fn snapshot_does_not_emit_blank_scrollback_capacity() {
    let mut e = Emulator::new(40, 6);
    e.feed(b"hello");
    let snap = e.snapshot();
    // Before the fix this emitted ~SCROLLBACK blank rows; a fresh 6-row screen
    // must produce at most ~rows lines.
    let newlines = snap.data.iter().filter(|&&b| b == b'\n').count();
    assert!(newlines <= 7, "snapshot emitted {newlines} lines (blank-padding bug)");
    // And it must still reproduce the content.
    let mut b = Emulator::new(snap.cols, snap.rows);
    b.feed(&snap.data);
    assert!(b.capture(10).contains("hello"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agency-core snapshot_does_not_emit -- --nocapture`
Expected: FAIL — `newlines` is ~10000 (the full scrollback capacity is emitted).

- [ ] **Step 3: Skip leading blank scrollback in `snapshot`**

In `crates/agency-core/src/term/emulator.rs`, in `snapshot()`, replace the line computing `history` and the emit loop header. The current code is:

```rust
        let history = (total - self.rows as usize) as i32;
        let mut last_flags = Flags::empty();
        let mut last_fg = Color::Named(NamedColor::Foreground);
        let mut last_bg = Color::Named(NamedColor::Background);
        for li in (-history)..self.rows as i32 {
```

Replace with:

```rust
        let history = total.saturating_sub(self.rows as usize) as i32;
        // Start at the first non-blank SCROLLBACK line (the grid reserves the
        // full scrollback capacity, most of which is blank); always emit the
        // whole screen (li >= 0) so its layout is preserved.
        let mut start = 0i32;
        for li in (-history)..0 {
            let blank = (0..self.cols as usize)
                .all(|col| grid[Line(li)][Column(col)].c == ' ');
            if !blank {
                start = li;
                break;
            }
        }
        let mut last_flags = Flags::empty();
        let mut last_fg = Color::Named(NamedColor::Foreground);
        let mut last_bg = Color::Named(NamedColor::Background);
        for li in start..self.rows as i32 {
```

(The body of the loop, the cursor trailer, and the returned `Snapshot` are unchanged.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p agency-core emulator -- --nocapture`
Expected: PASS — the new test plus the existing emulator tests (`snapshot_round_trips_through_a_fresh_emulator`, the capture tests).

- [ ] **Step 5: Commit**

```bash
git add crates/agency-core/src/term/emulator.rs
git commit -m "fix(terminal): snapshot skips blank scrollback capacity (no leading blank lines)"
```

---

## Self-Review notes

- **Spec coverage:** Fix 1 = Task 1 (probe + gating, fallback kept for Unknown agents); Fix 2 = Task 2 (frontend `feed` escape-skip + backend `sanitize_title` hardening); Fix 3 = Task 3 (snapshot skips blank scrollback). Each has tests.
- **Out of scope (unchanged):** opencode/codex/copilot auto-resume via probe (they keep the fallback); cursor/hermes (already fresh); menu bar / crash-survival.
- **Type consistency:** `ResumeProbe { Has, None, Unknown }` and `resume_probe(home, command, worktree)` are used identically in the probe module and `ensure_run_active`. `CaptureState` gains `esc: EscState` consistently across `initialCapture`/`feed`. `sanitize_title` keeps its signature; `strip_ansi_and_controls` is private.
