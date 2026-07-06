# Design: Agentic loops

Status: draft for discussion
Scope: run an agent repeatedly in its worktree — fresh session each attempt —
until a verifiable "done" condition passes or a cap is hit.

## Background

The pattern (popularized as the "Ralph loop", now native in Claude Code as
`/goal`) is: instead of a human reviewing each turn and typing the next prompt,
a harness re-invokes the agent with the *same* prompt in a *fresh* context, and
the agent picks up where it left off by reading state from disk (the worktree,
git log, a progress file). Fresh context per attempt avoids context rot on long
tasks; the filesystem is the memory. The two things that make or break it:

1. **The verifier.** A loop without a real "done" check ships bugs with
   confidence or spins forever. The only trustworthy verifiers are
   deterministic: a command that exits 0 (tests pass, lint clean, build green).
2. **Caps and visibility.** Cost compounds; confident non-progress looks like
   progress. Hard attempt caps and a live "attempt N of M" display are
   non-negotiable.

Agency's pitch: **racing is breadth, loops are depth.** We already have the
per-run worktree isolation, the respawn primitive, the polling daemon, and the
finish flow (diff review → merge) that a loop needs on both ends. Conductor-
class tools parallelize agents; none of them iterate one to convergence with a
proper cockpit. This is a feature overlay, not an architecture change.

## Goals / non-goals

Goals (v1):

- A run can be created in **Loop mode**: prompt + check command + max attempts.
- Each attempt is a fresh agent process in the run's existing worktree.
- Between attempts Agency runs the check command in the worktree; exit 0 ends
  the loop as complete.
- Hard caps: max attempts (required, default 10), consecutive-crash stall.
- Live status in the run panel; notifications for loop complete / stalled.
- Stop button that halts the loop without killing the worktree.

Non-goals (v1):

- No LLM-judged "is the goal met" verifier (deterministic check command only).
- No scheduling/cron ("run this loop nightly") — separate feature.
- No cross-run orchestration (race then loop the winner is v2, see below).
- No output-marker parsing (`<promise>DONE</promise>`) — v2 candidate.

## The critical mechanic: what is an "attempt boundary"?

Agency runs agents *interactively* in a PTY. An interactive `claude` session
does not exit when the task is done — it idles at its prompt. That is why
`notifier.rs` has the pane-quiet `Idle` heuristic at all. A loop keyed on
"process exited" would never fire for interactive sessions.

So loop attempts run the agent **headless (one-shot)**: `claude -p "<prompt>"`,
`codex exec "<prompt>"`, etc. The process exits when the turn is done — a
clean, deterministic attempt boundary, and exactly how every Ralph harness
works (`while :; do cat PROMPT.md | claude -p; done`). Output still streams
through the PTY into the existing terminal view, so the user watches attempts
live in the normal run terminal.

This needs one addition to profiles, following the `resume_args` precedent
exactly (nullable column, `ensure_*` seeding helper, `column_exists` migration
in `registry.rs`):

```rust
pub struct AgentProfile {
    ...
    pub resume_args: Option<Vec<String>>,
    /// One-shot invocation recipe for loop attempts. `{{prompt}}` token is
    /// replaced with the run prompt. None = agent can't loop (UI hides Loop
    /// mode for it).
    pub loop_args: Option<Vec<String>>,
}
```

Seeded defaults: claude `["-p", "{{prompt}}"]`, codex `["exec", "{{prompt}}"]`,
cursor-agent / opencode per their headless flags. Profiles without `loop_args`
simply can't loop.

Alternative considered — keep the session interactive and treat the notifier's
Idle latch as the attempt boundary, re-driving via the resume path. Rejected
for v1: the idle heuristic is a UX nudge, not a semantic signal (a long tool
call looks idle); and re-driving an interactive session accumulates exactly the
context rot the pattern exists to avoid. The headless spawn reuses
`rerun()`-shaped code (`state.rs:1349`) nearly verbatim.

## Data model

Additive columns on `runs` (house-style `column_exists` migrations,
`registry.rs:113`):

```
loop_config   TEXT     -- JSON, immutable after creation; NULL = not a loop
loop_state    TEXT     -- JSON, mutated by the loop driver
```

```rust
#[derive(Serialize, Deserialize)]
pub struct LoopConfig {
    pub check_command: String,   // run in worktree via $SHELL; exit 0 = done
    pub max_attempts: u32,       // hard cap, default 10
    pub check_timeout_secs: u64, // default 600
}

#[derive(Serialize, Deserialize)]
pub struct LoopState {
    pub status: LoopStatus,      // Running | Checking | Complete | Stalled | Stopped
    pub attempt: u32,            // 1-based, current or last
    pub consecutive_failures: u32, // agent nonzero exits; 3 => Stalled
    pub last_check_exit: Option<i32>,
    pub updated_at: i64,
}
```

JSON columns rather than one-column-per-field: the config is written once and
read as a unit, nothing queries on its parts, and it avoids six migrations. If
we later need to query loops by status, promote `status` to a real column.

The run's `kind` stays `"agent"`; `loop_config IS NOT NULL` is what marks a
loop. Racing's `race_id` pattern shows this shape works (a nullable grouping
field, not a new kind).

## Execution model

A new pure module `crates/agency-app/src/looper.rs`, mirroring `notifier.rs`:
a `step(prev_state, snapshot) -> (new_state, Vec<LoopAction>)` edge detector
with unit tests, driven by the existing 2 s polling daemon (`lib.rs:60-128`).
The daemon thread stays non-blocking; anything slow happens off-thread.

State machine per looping run:

```
AwaitingAgent   agent session running (headless attempt in flight)
   └─ agent Exited(0)        → spawn check (thread), status = Checking
   └─ agent Exited(nonzero)  → consecutive_failures += 1
                                ≥ 3 → Stalled (notify)
                                else → respawn attempt (no check; a crashed
                                       attempt can't have finished the work)
Checking        check command running in worktree
   └─ exit 0                 → Complete (notify; finish flow takes over)
   └─ exit nonzero / timeout → attempt >= max_attempts → Stalled (notify)
                                else attempt += 1, respawn agent
Stopped         user pressed Stop; terminal state
```

Details:

- **Respawn** = the `rerun()` recipe (`state.rs:1349`) with `loop_args`
  substituted for the interactive argv: kill session if any, `start_session`
  in the same worktree with the run's stored prompt. Same env plumbing
  (`provider_env`, `script_env`, `wrap_setup`).
- **Check execution**: `std::process::Command` via the user's `$SHELL -c`,
  cwd = worktree, spawned on its own thread; result posted back through a
  channel the poller drains. Never blocks the 2 s tick. Timeout kills the
  check and counts it as a failed check.
- **Dirty-state guard**: before each respawn, if the worktree has uncommitted
  changes, auto-commit them (`wip: loop attempt N`) — same rationale as
  `merge_task`'s auto-commit: an agent that forgot to commit must not have its
  work clobbered or invisibly carried across attempts. Every attempt then has
  an auditable boundary in `git log`.
- **Crash-loop guard**: 3 consecutive nonzero agent exits → Stalled. Catches
  bad CLI flags, auth failures, and rate-limit storms without burning the
  attempt budget on them.
- **Persistence**: `loop_state` is written to SQLite on every transition, so
  an app restart resumes cleanly: on launch, a run with status
  AwaitingAgent/Checking and a Gone session is treated as "attempt
  interrupted" and respawned (attempt counter unchanged).

## Verifier (v1: check command only)

The check command is the loop's contract and the UI should say so plainly:
"Loop ends when this command exits 0." Examples surfaced as placeholder text:
`pnpm test`, `cargo test`, `./scripts/check.sh`.

Deliberately not in v1:

- *Output markers* (`<promise>DONE</promise>`): requires scraping emulator
  content; deferred until demand is proven.
- *LLM-as-judge*: nondeterministic, costs tokens, and is the documented way
  loops go wrong. If an agent supports a native goal mode (Claude Code
  `/goal`), users can put that in the prompt themselves.
- *"No check command" mode* (pure N iterations): allowed — check command may
  be empty, in which case every attempt "fails" the check and the loop runs
  exactly `max_attempts` times. Useful for entropy-reduction prompts ("find
  and fix one lint warning"). The UI labels this "fixed iterations" so nobody
  mistakes it for verified completion.

## Prompt guidance

The loop reuses `run.prompt` verbatim every attempt (Ralph semantics: same
prompt, fresh context, state on disk). Loop prompts have a known good shape —
the New Run dialog's Loop mode shows a one-line hint and offers a template:

> Read PROGRESS.md if it exists. Pick the single most important unfinished
> piece of: <task>. Implement it, run the tests, commit with a clear message,
> and append one line to PROGRESS.md describing what you did.

Agency does not parse PROGRESS.md; it's purely the agent's memory between
attempts. No new machinery.

## UI

- **New Run dialog**: third mode alongside Single / Race — `Loop`. Fields:
  prompt (required, unlike Single), check command, max attempts (default 10),
  agent picker filtered to profiles with `loop_args`. Monochrome glyph per
  house style (e.g. `⟳`), no emoji.
- **Run panel** (`ui/src/components/RunPanel.tsx`): status strip when
  `loop_config` present — `Attempt 3/10 · running` / `checking` /
  `complete (attempt 3)` / `stalled`, plus a Stop control. Terminal view is
  unchanged — attempts stream into the same session name (`agency-<id>`), so
  attach/scrollback just works.
- **Sidebar/tray**: looping runs show the attempt counter in their label via
  the existing `tray_runs` refresh.
- **Notifications** (`notifier.rs` additions): `LoopComplete` ("Loop complete —
  attempt 3 passed checks") and `LoopStalled` ("Loop stalled — 10 attempts,
  checks still failing"). Per-attempt Finished/Idle notifications are
  suppressed for looping runs — ten "agent exited" toasts is noise; the loop
  events are the signal. One new toggle in notification settings covers both.

## IPC surface (`commands.rs`)

```
create_loop(project_id, prompt, agent, base, merge_target, loop_config) -> RunInfo
stop_loop(run_id)          // -> Stopped; kills live session, worktree intact
loop_status(run_id) -> LoopState   // or fold into existing run_status payload
```

Everything downstream of completion is the existing finish flow: merge preview,
diff review, merge, archive. Nothing merges automatically — the loop's output
is a branch the user reviews, same as any run.

## Phasing

**v1** — the above: profiles `loop_args`, two run columns, `looper.rs` +
poller integration, dialog mode, run-panel strip, two notifications, three
IPC commands. Rough size: ~500 lines Rust (half of it `looper.rs` tests),
~200 lines UI.

**v2 candidates**, in rough priority order:

1. *Race → loop the winner*: after picking a race winner, "loop this until
   green" seeded with the same prompt. Composes two existing features; likely
   the killer flow.
2. Output-marker completion (agent self-reports done) as an OR with the check
   command.
3. Wall-clock and/or spend caps alongside attempt caps.
4. Per-attempt diff timeline in the run panel (git already has the data via
   the auto-commit boundaries).

## Open questions

1. **Attempt visibility in scrollback** — headless attempts print once and
   exit; is plain scrollback enough to tell attempts apart, or should the
   supervisor inject a separator line ("── attempt 4 ──") into the pane feed?
   (Leaning: inject; trivial in the daemon, big readability win.)
2. **Check command trust** — it's an arbitrary shell command the user typed
   into their own app, same trust level as `scripts.setup` in project config.
   Assume no extra confirmation needed. Confirm.
3. **Default max attempts** — 10 feels right for "overnightable but bounded";
   Ralph harnesses commonly use 5–25. Any reason to force an explicit choice
   instead of defaulting?
4. **Permission mode inside attempts** — headless agents still hit permission
   prompts unless the profile's args include the agent's auto-accept flag
   (e.g. claude's `--permission-mode acceptEdits`). Bake into seeded
   `loop_args` defaults, or leave to the user's profile? (Leaning: bake in —
   a loop that stalls on a permission prompt defeats the purpose; worktree
   isolation bounds the blast radius.)
