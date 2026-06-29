# Terminal + Resume Bug Fixes (claude probe, title escapes, blank lines) — Design

**Date:** 2026-06-29
**Branch:** `feat/termd`

Three independent bugs surfaced in manual testing of the terminal-daemon + resume work. They share no code but are batched as one round of fixes.

---

## Fix 1: Claude/pi resume via a proactive session probe

### Problem
Reactivating a stopped **claude** agent runs `claude --continue`. When there is no
conversation to resume, claude prints "No conversation found to continue" and —
**in a real PTY — does not exit**; it stays running. The early-exit fallback (which
respawns fresh when the primary exits within a grace window) therefore never fires,
leaving a dead pane. The fallback *does* work for agents that exit fast on
resume-failure (pi, opencode, cursor, etc.), so it must be kept; claude is the lone
stay-running exception.

### Approach
Decide resume-vs-fresh **before** launching, for agents we can probe. A tri-state
probe inspects the agent's on-disk per-worktree session store:

```
enum ResumeProbe { Has, None, Unknown }
fn resume_probe(home: &Path, command: &str, worktree: &Path) -> ResumeProbe
```

- `claude` → `<home>/.claude/projects/<enc>/` where `enc` = the worktree path with
  every `/` and `.` replaced by `-`. Non-empty dir → `Has`, else `None`.
- `pi` → `<home>/.pi/agent/sessions/--<path with / → ->--/`. Non-empty → `Has`, else `None`.
- any other command → `Unknown`.

`resume_probe` takes `home` as a parameter so it is unit-testable against a faked
store under a temp dir; the real caller passes the user's home dir
(`dirs`/`std::env::home_dir`).

### `ensure_run_active` wiring (agent branch, profile has `resume_args`)
```
match resume_probe(home, &profile.command, &worktree) {
    ResumeProbe::None => /* proven nothing to resume -> fresh, no resume attempt */,
    ResumeProbe::Has | ResumeProbe::Unknown => /* current behavior:
        primary = resume argv, fallback = fresh argv (start_session_with_fallback) */,
}
```
- claude/pi with no session → fresh directly (fixes the dead pane; claude never
  launches `--continue` into nothing).
- claude/pi with a session → resume (with the fresh fallback still attached as a net).
- opencode/codex/copilot (`Unknown`) → unchanged resume-with-fallback (the fallback
  catches their fast-exit failures — no regression).
- cursor/hermes (no `resume_args`) → fresh, unchanged.

The "fresh, no resume attempt" branch starts the fresh session via
`start_session_with_fallback(..., None)` (no fallback needed) or `start_session`.

### Testing
- `resume_probe` unit tests (agency-app, with a temp home): claude store
  `<temphome>/.claude/projects/<enc>/x.jsonl` present → `Has`; absent dir → `None`;
  pi store path present → `Has`, absent → `None`; unknown command → `Unknown`;
  verify the exact `enc` encodings (claude: `/`+`.`→`-`; pi: `--`-wrapped, `/`→`-`).
  These are the load-bearing tests — the probe logic + encodings.
- The `ensure_run_active` gating is a thin tri-state `match` on the probe result
  (`None` → fresh, `Has`/`Unknown` → resume-with-fallback). It is exercised by the
  existing `ensure_run_active` tests (resume/fresh dispatch) plus manual GUI; a
  hermetic integration test is not added because it would require a real
  `claude`/`pi` binary and an injected home, which the probe unit tests already
  cover at the unit level.

---

## Fix 2: Run title polluted by terminal escape sequences

### Problem
For **opencode**, the run title becomes garbage like
`]11;rgb:b3b3/bcbc/b2b2\[?1016;2$y[?2027;0$y…`. Root cause: `FocusTerminal`'s
first-prompt capture is fed from `term.onData`, which fires for **xterm.js's
automatic replies** to the agent's terminal queries (OSC 11 background-color report,
DECRQM mode reports), not just typed text. `ui/src/lib/firstPrompt.ts::feed` drops
the ESC byte (`0x1b`) but keeps the rest of the sequence body (`]11;rgb:…` are all
printable ≥ 0x20), so the escape-sequence body accumulates and is stored as the
title via `set_run_title` → `fallback_title`. `sanitize_title` does not strip ANSI
either.

### Approach (defense in depth, two layers)
1. **Frontend `feed`** (`ui/src/lib/firstPrompt.ts`): properly skip whole escape
   sequences instead of only the ESC byte. Add an `inEsc` sub-state to `CaptureState`
   and a small skipper that, on `0x1b`, consumes the rest of the sequence:
   - CSI (`ESC [`): consume until a final byte in `0x40..=0x7E`.
   - OSC (`ESC ]`): consume until `BEL` (`0x07`) or ST (`ESC \`).
   - other `ESC x`: consume the single following byte.
   The state persists across chunks (a reply may arrive split). Only genuine
   printable typed text reaches `buf`.
2. **Backend `sanitize_title`** (`crates/agency-core/src/title.rs`): before the
   existing normalization, strip ANSI/control noise — remove ESC-introduced
   sequences and any remaining control characters (keep printable + normal
   whitespace). A title that reduces to empty is not stored (caller already guards
   on non-empty), so a polluted capture falls back to the branch label.

### Testing
- `firstPrompt.feed` unit tests (ui): feeding `\x1b]11;rgb:b3b3/bcbc/b2b2\x07` then
  `hi\r` captures `"hi"`, not the escape body; a CSI reply `\x1b[?1016;2$y` is fully
  skipped; a sequence split across two `feed` calls is still skipped.
- `sanitize_title` unit tests (agency-core): an input containing OSC/CSI sequences +
  control bytes reduces to the clean text (or empty); existing tests still pass.

---

## Fix 3: Hundreds of blank lines above every terminal

### Problem
Every agent/terminal shows hundreds of blank lines scrollable above the content.
`Emulator::snapshot()` emits `history = grid.total_lines() - rows` rows, and
`Dimensions::total_lines()` returns `screen + SCROLLBACK` (the full 10,000-line
scrollback **capacity**), so the snapshot replays ~10,000 blank rows before the real
content on every attach. `capture()` avoids this by clamping to a line budget;
`snapshot()` does not.

### Approach
In `snapshot()`, skip leading entirely-blank lines: scan from the topmost line
(`-history`) downward for the first line containing any non-blank cell, and begin
emitting from there. If all scrollback lines are blank (a fresh agent), start at the
screen top (`Line(0)`). Trailing/within-screen blank lines are preserved (normal
screen layout). This removes the blank prefix and also avoids walking ~10k empty
rows. The cursor-position trailer is computed as today.

### Testing
- `snapshot` unit test (agency-core, `emulator.rs`): a fresh emulator fed a single
  short line, then `snapshot()` → fed into a clean emulator → `capture()` shows the
  line with no large blank prefix (assert the reconstructed text does not begin with
  many blank lines, e.g. the first non-empty content appears within the first few
  rows). The existing `snapshot_round_trips_through_a_fresh_emulator` test still passes.

---

## Scope boundaries

**In:** the three fixes above.
**Out:** auto-resume for opencode/codex/copilot via a probe (they keep the
fallback; their probes are a later enhancement); any change to cursor/hermes
(already fresh); the menu bar / crash-survival paths (separate, already validated).

## Sequencing

Three independent fixes on `feat/termd`; any order. Each is small and
self-contained with its own tests.
