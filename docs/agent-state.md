# Design: agent state from the agent's own hooks

Status: **draft for discussion** (2026-09-08, AGE-206). Nothing here is built.
Scope: replace the pane-diff guess in `activity.rs` with state the agent
reports about itself, and split today's `waiting` into `blocked` and `done`.

## What is true today

`activity.rs` derives a run's state from exactly one bit per tick: did the
pane's content hash change since the last notifier poll. `notifier.rs` computes
that bit (`RunSnapshot::pane_hash`) while capturing panes for its own edge
detection, and `state.rs::read_activity` classifies from it on read.

Three states come out, and the quality drops sharply across them:

- **working** — the pane changed inside `WORKING_TTL_MS` (10s). This one is
  sound. Agents redraw constantly while they work: spinners, streaming tokens,
  tool output.
- **waiting** — quiet, but a user-driven turn is in flight (`turn_driven`).
- **idle** — quiet with no turn in flight, or `waiting` that has sat unattended
  for `WAITING_MAX_MS` (30 minutes) and decayed.

`waiting` is the problem. It is one label over two situations that call for
opposite responses:

- the agent hit a permission prompt and *cannot proceed* until you answer it;
- the agent finished its turn and is *done*, waiting for you to read the diff.

Both render as a quiet pane. A hash diff cannot tell them apart, and neither
can any refinement of a hash diff. On the all-projects overview — 18 projects,
12 agents in the README screenshot — that is the difference between the two
runs that are burning wall-clock waiting on a keystroke and the ten that are
not. `ui/src/lib/runstate.ts::needsAttention` currently flags both as needing
attention, so the badge means "something happened here, maybe", and a badge
that vague trains you to ignore it.

`idle` is worse than a guess: it is a decay timer. A blocked agent that you
ignore for half an hour is still blocked, but Agency stops saying so.

## The gap this closes

Coding agents already publish their own lifecycle. Claude Code has fired hooks
for permission prompts, turn completion and prompt submission for a long time;
Gemini CLI shipped roughly twelve lifecycle events with hooks on by default
from v0.26.0; OpenCode, Copilot CLI and Cursor have their own hook or plugin
event systems. Agency has never asked any of them, and has been reconstructing
from pixels a fact the agent would have told us.

Prior art in this space (a self-hosted terminal multiplexer built for agent
fleets, read 2026-09-08) settles this with a two-tier authority model: an
installed, actively-reporting integration is authoritative for state and
session identity, and screen reading is the fallback only for agents that have
no integration. It carries four states — idle, working, blocked, done — and
rolls the strongest one up from pane to tab to workspace. That model is right,
and the reasoning is not subtle: a reported fact beats an inferred one, and the
inference is still needed because the coverage is never complete.

## Goals / non-goals

**Goals**

- `blocked` is a first-class state, distinguishable from `done`, for every
  agent whose CLI will tell us.
- Reported state outranks inferred state while the report is live.
- The pane-diff heuristic survives untouched as the fallback, so an agent with
  no hooks (Codex, today) and a plain shell run are no worse off than now.
- No new dependency, no new daemon, no new port, no file the user did not
  already have Agency writing into their worktree.
- Nothing here can slow down, block, or fail an agent's turn.

**Non-goals**

- Screen-based detection rules richer than a content hash. That is AGE-207 and
  is the *other* tier; this design only has to leave room for it.
- Reading the agent's transcript for state. `usage.rs` already reads
  transcripts for token accounting, and they lag: the docs say
  `transcript_path` is written asynchronously and may be behind. State needs to
  be live.
- Anything that reports state off this machine (AGE-211, AGE-212).

## The state model

Four states replace three:

| state | meaning | who can produce it |
| --- | --- | --- |
| `working` | producing output, or inside a tool call | reports and heuristic |
| `blocked` | stopped on a permission prompt or a question, cannot proceed | reports only |
| `done` | finished a turn, waiting for the user to look | reports; heuristic approximates it |
| `idle` | nothing in flight; a fresh run nobody has prompted | reports and heuristic |

`waiting` disappears from the wire. Under the heuristic alone, what is `waiting`
today becomes `done` — it is the better of the two readings, because
`turn_driven` already means the user started a turn, and it makes the fallback
strictly a coarser version of the reported model rather than a different one.

The 30-minute decay to `idle` goes away for reported states. A decay timer was
only ever a hedge against not knowing; once the agent tells us it is blocked,
it stays blocked until it tells us otherwise. The timer stays for heuristic
`done`, where the hedge is still warranted.

Ranking, strongest first: `blocked` > `done` > `working` > `idle`. That order
is what AGE-213 rolls up to a project row, and what AGE-215 sorts by.

## Where the reports come from

Claude Code first, because it is the default agent, has the richest hook set,
and — critically — supports an **HTTP hook type**, which removes the entire
question of writing a shell script into the user's worktree and hoping `jq` is
on the PATH.

Verified against the hooks reference on 2026-09-08:

| hook event | matcher | what it means for us |
| --- | --- | --- |
| `UserPromptSubmit` | none | turn started → `working` |
| `PreToolUse` | `*` | still working; `tool_name` is the status line (AGE-208) |
| `PostToolUse` | `*` | still working |
| `Notification` | `permission_prompt` | **`blocked`** |
| `Notification` | `idle_prompt` | `done` |
| `Stop` | none | turn finished → `done` |
| `SessionStart` | `startup\|resume` | `idle`, and carries `session_id` |
| `SessionEnd` | none | session over |

`Notification` taking a matcher on `notification_type` is the whole design.
`permission_prompt` is not a heuristic for "blocked"; it *is* blocked, named by
the agent at the moment it happens.

Two payload details matter:

- Every event carries `session_id`, `cwd`, `transcript_path` and
  `permission_mode`. `session_id` is the session identity AGE-210 needs for
  resume, arriving free with the state work — worth capturing from
  `SessionStart` even before anything consumes it.
- Subagent events carry `agent_id` and `agent_type`. **Discard any report that
  has `agent_id`.** A run whose agent spawns four subagents would otherwise
  flicker through their turn boundaries; the run's state is the main agent's
  state.

Per-agent coverage is a table to be built the way `skills.rs` built its
directory table: against live binaries with the version pinned in the comment,
never against documentation alone. `skills.rs` documents a case where the docs
claimed a directory the binary did not read. Assume the same here. Codex is the
known gap — no user-facing hooks as of this writing — and is exactly what
AGE-207 exists to cover.

## Transport

The receiving end already exists. `preview/` runs a per-run server bound to
`127.0.0.1` on the last port of the run's port block, loopback only, serving
`/__agency__/…` control routes alongside the proxied dev server. Adding one
route is a small change to a server that already has the shape.

```
POST /__agency__/state
Authorization: Bearer $AGENCY_RUN_TOKEN
{ "hook_event_name": "Notification", "notification_type": "permission_prompt",
  "session_id": "...", "cwd": "/…/worktrees/agent-aso3", ... }
```

The emitted `.claude/settings.local.json` block is then, per hook:

```json
{ "type": "http",
  "url": "http://127.0.0.1:5231/__agency__/state",
  "timeout": 2,
  "headers": { "Authorization": "Bearer $AGENCY_RUN_TOKEN" },
  "allowedEnvVars": ["AGENCY_RUN_TOKEN"] }
```

Three consequences to design against:

- **The hook must never stall the agent.** A hook that times out is cancelled
  and its output discarded, and on these events no decision is applied, so the
  agent proceeds — but it proceeds *after* the timeout. On `PreToolUse` that
  cost lands on every tool call. Hence `timeout: 2`, and the server answers
  `204` immediately having done nothing but enqueue. Whether the `http` hook
  type accepts `async: true` (documented under command hooks) needs checking
  against a live binary; if it does, use it and drop the timeout concern
  entirely. If a run's server is not listening, two seconds per tool call is
  unacceptable, so **emit no hooks unless the server for that run is up**.
- **`Caps` needs a third member.** Today `Caps::none()` is documented as "the
  state in which no server should be listening", and the server only runs when
  the preview or editor capability is on. State reporting wants the server up
  for any run with a hook-capable agent, so `Caps { preview, editor, state }`
  and the lifetime rule becomes "any cap on". The state cap is on when we
  emitted hooks for that run, which keeps the two facts from drifting apart.
- **The token is per run.** `scripts.rs::script_env` already builds the
  `AGENCY_*` set that reaches the agent through `StartSession { env }`; add
  `AGENCY_RUN_TOKEN`, a random per-run value, alongside `AGENCY_PORT`. Do not
  key on `cwd` alone: it identifies the worktree, and several runs can share a
  worktree (see `run-teardown.md`, AGE-184). The token identifies the run; the
  `cwd` is a cross-check, and a mismatch is dropped and logged.

## Emission

`mcp.rs::emit_for_agent` is the pattern to copy, not to extend — it emits MCP
servers, and hooks are a different payload with the same three rules:

1. **Merge, never replace.** `upsert_json` semantics: our block goes in by a
   reserved key, the user's own hooks are left exactly as they are. Claude Code
   merges hooks across settings levels, so ours coexisting with theirs is the
   supported case, not a workaround.
2. **A tracked file is the repo's own; leave it alone.** `mcp.rs::tracked` and
   `skills.rs` both skip on this test. Same test here. The natural target is
   `.claude/settings.local.json`, which is the local-override slot and is
   conventionally untracked, but "conventionally" is not "verified" and the
   check is two lines.
3. **Exclude what we write.** `ensure_exclude_pattern`, anchored
   (`/.claude/settings.local.json`), for the reason AGE-115 recorded: the one
   worktree file drop without an exclude was the one that ended up staged by an
   agent's `git add -A` and merged into the project.

The exclusion in `mcp.rs` for user-scope OAuth servers has no analogue here.
Hooks are workspace-scoped by nature.

## The transition function

`activity.rs` stays pure, per the house style it is already one of the models
for. The report is another input to the same fold, not a side channel:

```rust
pub struct Report {
    pub state: ActivityState,
    pub at_ms: i64,
    /// Highest hook-protocol version the reporter used, so a stale emitted
    /// block can be recognised rather than misread.
    pub proto: u8,
}

pub fn update(
    prev: Option<ActivityEntry>,
    pane_changed: bool,
    report: Option<Report>,
    now_ms: i64,
) -> ActivityEntry;

pub fn classify(
    entry: &ActivityEntry,
    turn_driven: bool,
    agent_running: bool,
    now_ms: i64,
) -> ActivityInfo;
```

The app owns the impure half: the HTTP handler parses, authenticates and drops
subagent events, then hands `update` a `Report`. The map in `state.rs:2287`
gains the last report per run and stays in-memory, which is right — a forgotten
run showing `idle` is the correct answer after a restart.

**Authority and lapse.** Reports win while they are live, with the lapse rule
turning on which state it is:

- `blocked`, `done` and `idle` are *resting* states. They are supposed to
  persist with no further events, and the pane is quiet in all three, so there
  is nothing for the heuristic to disagree with. They persist indefinitely.
- `working` is *heartbeat-backed*. `PreToolUse` and `PostToolUse` fire
  repeatedly through a turn, so a reported `working` with no event inside
  `REPORT_WORKING_TTL` is suspect — the agent was killed mid-turn, or crashed,
  or the emitted block is stale. It lapses back to the heuristic, which will
  correctly read a quiet pane.
- Any reported state lapses when the session is no longer
  `SessionStatus::Running`. A dead process is not blocked; it is gone. This is
  why `classify` takes `agent_running` — the fact lives in the snapshot the
  notifier already has.

Blocked never gets stuck in practice: the user answers the prompt in the pane,
the tool runs, `PostToolUse` fires, and the run is `working` again. If they
refuse instead, `Stop` fires and it is `done`. Both edges are already on the
list above, which is the check that the list is complete.

## What changes around it

**`notifier.rs`.** `NotifyKind::Idle` ("Agent finished a turn") is today fired
from quiet-pane timing gated on `user_input_pending`. It splits: `Blocked` when
a run enters that state, `Done` on turn completion. Both become edges on the
reported state rather than on a quiet-tick count, which removes `idle_secs`
from the reported path. `step()` stays a pure edge detector; `RunSnapshot`
gains the state. `NotifSettings` gains `agent_blocked`, defaulting on, and
`agent_idle` is renamed with a serde alias so saved settings keep loading —
`only_when_watching` already sets that precedent.

`suppressed()` needs no change and should not get one. Its reasoning ("the only
run we stay quiet about is the one the user is watching right now") is
independent of how the state was derived, and it is better than the
active-tab suppression the comparable tool does.

Notification copy, avoiding em dashes per the house rules:

- blocked: "Agent needs you", "`{label}`: waiting on a permission prompt"
- done: "Agent finished a turn", "`{label}`: ready for you"

**The UI.** `runstate.ts` is the whole of the change: `needsAttention` keys on
`"waiting"` at line 36 and becomes `blocked || done` with different weight;
`runStatus` and `HomeView`'s `waitingCount` follow; `IssueRow`'s activity dot
gains a fourth class. `ActivityState` serializes camelCase and `"waiting"` will
simply stop appearing, so the TS union must be updated in the same change or
the board silently renders nothing for the new states.

## Privacy and trust

Three things to hold onto:

- **The payloads stay on the machine.** The hook posts to `127.0.0.1` on a port
  the run already owns. Nothing here is a network call in the sense the README
  makes a claim about, and nothing here changes that claim.
- **Do not store what we do not need.** Hook payloads carry `tool_input`, which
  is the agent's actual commands and file contents, and `transcript_path`. We
  need `hook_event_name`, `notification_type`, `session_id`, `cwd` and (for
  AGE-208) `tool_name`. Parse those and drop the rest at the door, rather than
  keeping a payload we then have to be careful with. This is the same instinct
  the comparable tool showed in defaulting its scrollback replay to off because
  terminal output contains secrets.
- **Default-deny on what we accept.** The house rule for anything imported or
  shared. `hook_event_name` and `notification_type` are matched against an
  allowlist of exact values; an unrecognised event is ignored, not guessed at.
  A denylist here would quietly mis-state a run the first time an agent adds an
  event.

## Phasing

1. **Claude Code end to end.** The `state` cap and the `/__agency__/state`
   route, the per-run token, emission into `.claude/settings.local.json` with
   merge/tracked/exclude, `Report` threaded through `activity.rs`, the four
   states on the wire, `runstate.ts` updated. One agent, working `blocked`.
2. **`notifier.rs`.** Blocked and Done as edges on reported state; settings and
   copy.
3. **The rest of the catalog.** Per-agent hook tables verified against live
   binaries, one agent per change, each landing with its own test the way
   `skills.rs` rows do.
4. **The roll-up and the sort** (AGE-213, AGE-215), which are only worth
   building on top of a state that is real.

AGE-207 (declarative screen rules) and AGE-208 (agent-authored status) both
land after phase 1 and neither blocks it. AGE-209's `wait(run_id)` is gated on
phase 1: a blocking wait is only as trustworthy as the state it waits on.

## Open questions

- Does the `http` hook type accept `async: true`? If so most of the timeout
  reasoning above evaporates. Needs a live binary, not the docs.
- What happens to hooks under `--dangerously-skip-permissions` and the
  `bypassPermissions` permission mode? `permission_prompt` presumably never
  fires, which is correct (nothing is blocked) but should be confirmed rather
  than assumed. `permission_mode` is in every payload, so we can at least
  record which mode a run is in.
- The agent CLI may be upgraded under a long-lived run, changing hook payloads
  mid-flight. The `proto` field covers our end; theirs is unversioned.
- Loop runs (`looper.rs`) suppress per-attempt notifications by design. Does a
  loop attempt that blocks on a permission prompt deserve a notification? It is
  arguably the one thing a loop cannot recover from on its own, which suggests
  yes, and suggests it is the only per-attempt event that should escape the
  suppression.
- Does a `blocked` run still count toward a project's "working" roll-up for the
  purposes of the tray icon, or is blocked strictly stronger there too?
