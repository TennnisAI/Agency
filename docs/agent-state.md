# Design: agent state from the agent's own hooks

Status: **phases 1 and 2 built** (AGE-206, 2026-10-04) for Claude Code. Drafted
2026-09-08; the sections below record what was built, and where the build
departed from the draft, why. Phases 3 and 4 are still open.

Scope: take a run's state from what the agent reports about itself where it
can, keep the pane-diff heuristic in `activity.rs` as the fallback, and split
the old `waiting` into `blocked` and `done`.

## What was true before

`activity.rs` derived a run's state from exactly one bit per tick: did the
pane's content hash change since the last notifier poll. `notifier.rs` computes
that bit (`RunSnapshot::pane_hash`) while capturing panes for its own edge
detection, and `state.rs::read_activity` classifies from it on read.

Three states came out, and the quality dropped sharply across them:

- **working**: the pane changed inside `WORKING_TTL_MS` (10s). This one is
  sound. Agents redraw constantly while they work: spinners, streaming tokens,
  tool output.
- **waiting**: quiet, but a user-driven turn is in flight (`turn_driven`).
- **idle**: quiet with no turn in flight, or `waiting` that has sat unattended
  for 30 minutes and decayed.

`waiting` was the problem. It was one label over two situations that call for
opposite responses:

- the agent hit a permission prompt and *cannot proceed* until you answer it;
- the agent finished its turn and is *done*, waiting for you to read the diff.

Both render as a quiet pane. A hash diff cannot tell them apart, and neither
can any refinement of a hash diff. On the all-projects overview (18 projects,
12 agents in the README screenshot) that is the difference between the two
runs that are burning wall-clock waiting on a keystroke and the ten that are
not. `needsAttention` flagged both, so the badge meant "something happened
here, maybe", and a badge that vague trains you to ignore it.

`idle` was worse than a guess: it was a decay timer. A blocked agent that you
ignore for half an hour is still blocked, but Agency stopped saying so.

## The gap this closes

Coding agents already publish their own lifecycle. Claude Code has fired hooks
for permission prompts, turn completion and prompt submission for a long time;
Gemini CLI shipped roughly twelve lifecycle events with hooks on by default
from v0.26.0; OpenCode, Copilot CLI and Cursor have their own hook or plugin
event systems. Agency had never asked any of them, and had been reconstructing
from pixels a fact the agent would have told us.

Prior art in this space (a self-hosted terminal multiplexer built for agent
fleets, read 2026-09-08) settles this with a two-tier authority model: an
installed, actively-reporting integration is authoritative for state and
session identity, and screen reading is the fallback only for agents that have
no integration. It carries four states (idle, working, blocked, done) and
rolls the strongest one up from pane to tab to workspace. That model is right,
and the reasoning is not subtle: a reported fact beats an inferred one, and the
inference is still needed because the coverage is never complete.

## Goals / non-goals

**Goals**

- `blocked` is a first-class state, distinguishable from `done`, for every
  agent whose CLI will tell us.
- Reported state outranks inferred state while the report is live.
- The pane-diff heuristic survives as the fallback, so an agent with no hooks
  (Codex, today) and a plain shell run are no worse off than before.
- No new dependency, no new daemon, no new port, no file the user did not
  already have Agency writing into their worktree.
- Nothing here can slow down, block, or fail an agent's turn.

**Non-goals**

- Screen-based detection rules richer than a content hash. That is AGE-207 and
  is the *other* tier; this design only has to leave room for it.
- Reading the agent's transcript for state. `usage.rs` already reads
  transcripts for token accounting, and they lag: `transcript_path` is written
  asynchronously and may be behind. State needs to be live.
- Anything that reports state off this machine (AGE-211, AGE-212).

## What the binary actually does

The draft was written against the hooks reference. Before building, every
claim it relied on was checked against Claude Code **2.1.289**, in `-p` mode
and in an interactive session in tmux, with an `http` hook on each event
pointed at a listener that logged what arrived. Four of the draft's
assumptions did not survive.

| observation | consequence |
| --- | --- |
| `PermissionRequest` fires the instant a permission dialog appears, and also for an `AskUserQuestion` question. `Notification` with `permission_prompt` fires about **six seconds later**. | `PermissionRequest` is the `blocked` signal. `permission_prompt` is not subscribed: an answer given inside those six seconds would race it and put a working agent back to `blocked`. |
| Answering **"No"** to a permission dialog, or pressing **Esc** on it, fires **nothing**. No `Stop`, no `PostToolUseFailure`, no `Notification`. | The draft said a refusal fires `Stop`. It does not, so a reported `blocked` cannot rely on the next event to end it. See *Lapse*. |
| `SessionStart` never fires an `http` hook, interactive or not. | The draft's plan to capture `session_id` from it for AGE-210 does not work. Every other event carries `session_id`, so the id is still there to take; nothing stores it yet. |
| `async: true` is accepted on an `http` hook but **not honoured**: a listener that slept 1.5s held the tool call for 1.5s. | The draft hoped `async` would remove the timeout concern. It does not. The server answers before it does anything else, and the 2s timeout bounds a wedged app. |
| With nothing listening, the post costs nothing (loopback refuses at once), but the agent prints `PreToolUse:Bash hook error ... ECONNREFUSED` into its transcript on every tool call. A non-2xx answer prints the same kind of error. | The draft's reason to emit no hooks without a server (two seconds per tool call) was wrong; the real reason is the noise. The route answers 204 to everything, including a report it drops. |
| `PostToolUseFailure` replaces `PostToolUse` when a tool fails. | Subscribed, as `working`. |
| `Stop` is followed by `SubagentStop` from an internal subagent, carrying `agent_id`. | Not subscribed, and any body with an `agent_id` is dropped at the parse. |
| `Notification` with `idle_prompt` fires about 60s after `Stop`. | Subscribed, as `done`. It is also what lands after an Esc that left no other report. |
| `SessionEnd` fires on `/clear` as well as on exit. | Mapped to `idle`: after `/clear` the session is fresh. |
| Header values interpolate `$VAR` for names in `allowedEnvVars`; an unset variable becomes an empty string. | The session id rides in a header (see *Transport*). |
| No hook fires before the user accepts the folder-trust dialog for a new directory. | Nothing to do; a worktree under a trusted repo inherits the trust. |

Two things were not verified and stay open: what fires under
`bypassPermissions` (starting a session in that mode shows a warning the user
has to accept, and accepting it is written to their global settings, which is
not Agency's to do on their behalf), and whether an MCP elicitation dialog
fires `PermissionRequest` the way `AskUserQuestion` does.

## The state model

Four states replace three:

| state | meaning | who can produce it |
| --- | --- | --- |
| `working` | producing output, or inside a tool call | reports and heuristic |
| `blocked` | stopped on a permission dialog or a question, cannot proceed | reports only |
| `done` | finished a turn, waiting for the user to look | reports; heuristic approximates it |
| `idle` | nothing in flight; a fresh run nobody has prompted | reports and heuristic |

`waiting` is gone from the wire. Under the heuristic alone, what was `waiting`
is now `done`, because `turn_driven` already means the user started a turn,
and it makes the fallback strictly a coarser version of the reported model
rather than a different one.

`ActivityInfo` carries `reported: bool` beside the state. The draft did not
have it, and it turned out to matter for honesty: a heuristic `done` is "quiet
after your turn", which may really be a permission prompt from an agent that
cannot say so. The board labels a reported `done` as "done" and a guessed one
as "waiting", with a tooltip that says the agent does not report which.

The 30-minute decay to `idle` applies to the guessed `done` only. A decay timer
was only ever a hedge against not knowing; once the agent says it is done, it
stays done until it says otherwise.

Ranking, strongest first: `blocked` > `done` > `working` > `idle`. That order
is what AGE-213 rolls up to a project row, and what AGE-215 sorts by.

## Where the reports come from

Claude Code only, in this change, because it is the default agent and it has
an `http` hook type: the agent posts straight at the server, with no shell
script written into the user's worktree and no `jq` assumed on the PATH.

The subscribed events, in `state_hooks.rs::CLAUDE_EVENTS`, with the mapping in
`preview/report.rs`:

| hook event | matcher | state |
| --- | --- | --- |
| `UserPromptSubmit` | none | `working` |
| `PreToolUse` | `*` | `working` |
| `PostToolUse` | `*` | `working` |
| `PostToolUseFailure` | `*` | `working` |
| `PermissionRequest` | `*` | **`blocked`** |
| `Notification` | `idle_prompt` | `done` |
| `Stop` | none | `done` |
| `SessionEnd` | none | `idle` |

The parse is default-deny: the event and notification type are matched against
exact values, and anything else is ignored rather than guessed at. Only
`hook_event_name`, `notification_type` and `agent_id` are read. The payload
also carries `tool_input` (the agent's actual commands and file contents) and
`transcript_path`; none of that is kept.

Per-agent coverage beyond Claude Code is a table to be built the way
`skills.rs` built its directory table: against live binaries with the version
pinned in the comment, never against documentation alone. This change is the
precedent for why: four of the draft's documented assumptions were wrong.

## Transport

The receiving end is the run's existing server in `preview/`, bound to
`127.0.0.1` on the last port of the run's port block. One route was added:

```
POST /__agency__/state
X-Agency-Session: <run id, or <run>--<n> for an extra tab>
{ "hook_event_name": "PermissionRequest", "tool_name": "Bash", ... }
```

The emitted block in `.claude/settings.local.json` is, per event:

```json
{ "type": "http",
  "url": "http://127.0.0.1:5249/__agency__/state",
  "timeout": 2,
  "headers": { "X-Agency-Session": "$AGENCY_SESSION_ID" },
  "allowedEnvVars": ["AGENCY_SESSION_ID"] }
```

`agent_env`, the one builder all five launch paths go through, sets
`AGENCY_SESSION_ID` to the session being launched.

**A session header, not a per-run token.** The draft proposed
`AGENCY_RUN_TOKEN`, a random per-run secret, as the authentication. What
actually needed solving was identity: every tab of a run shares the worktree,
and so the settings file and the URL, and only the session id says which tab
is reporting. The port already identifies the run, and a session of another
run posting there is dropped. A `claude` the user starts by hand in the
worktree has the variable unset, sends an empty header, and is ignored. A
token would also have had to survive an app restart, since the agent's
environment is fixed at spawn and outlives the server, which means persisting
a secret per run. Against what threat? Browsers are already kept out by the
server's origin check, and any local process that can reach loopback can
already call `set_status` on the same server. That was not worth a stored
secret.

**The route always answers 204, before doing anything.** Claude Code waits on
every post, and prints a hook error for any non-2xx answer.

**`Caps` has a third member, `state`.** A server now runs while preview tools,
open-file sharing, *or* state hooks want it. `state` is on for a run whose
worktree carries our hooks, read off the file by `state_hooks::emitted` on each
sweep rather than remembered. That is what keeps the listener up across an app
restart: an agent that survived in the daemon read its hooks at startup and
posts to its port whether anything listens or not.

**The state cap starts a server; it does not add an MCP entry.** The draft
assumed the cap would also give every run the `agency-preview` server and its
`set_status` tool. It does not, deliberately. Claude Code asks the user to
approve a server it finds in a workspace's `.mcp.json`, so emitting one to
every Claude run would put an approval prompt in front of runs that have none
today. The hooks need the server listening, not the agent connected to it.
`state.rs` keeps the two questions apart: `preview_mcp_port_for` decides the
MCP entry, `server_port_for` decides the listener.

## Emission

`state_hooks.rs` follows `mcp.rs::emit_for_agent`'s three rules:

1. **Merge, never replace.** Our groups are recognised by their URL (any
   loopback port, path `/__agency__/state`), stripped, and re-added at the
   current port. The user's own hooks, matcher groups and every other key are
   left exactly as they were. A group the user shares with one of ours keeps
   the user's entry.
2. **A tracked file is the repo's own; leave it alone.** Same `tracked` test as
   `mcp.rs`.
3. **Exclude what we write.** `/.claude/settings.local.json`, anchored, for
   the reason AGE-115 recorded.

One rule more than `mcp.rs`'s `upsert_json`, which starts over from `{}` when a
file does not parse: **a file that does not parse is left alone.** Claude Code
writes to `settings.local.json` itself (a "don't ask again" answer is saved
there as a permission rule), so starting it over would throw away the user's
permissions. The write goes through `issuefs::atomic_write`.

Hooks are written only once the run's server is up, and taken out when it
cannot be, so a stale block never produces the transcript noise above. The
launch path (`prepare_state_hooks`, called from `agent_env`) starts the server
itself, writes the hooks at whatever port actually bound, and clears the
session's last report: a new process is not what the old one reported, and
`SessionStart` will not fire to say so. The project's own checkout is never
written into, the same rule as MCP config.

## The transition function

`activity.rs` stays pure. A report is another input to the same fold:

```rust
pub struct Report { pub state: ActivityState, pub since_ms: i64, pub at_ms: i64 }

pub fn update(prev: Option<ActivityEntry>, pane_changed: bool, now_ms: i64) -> ActivityEntry;
pub fn reported(prev: Option<ActivityEntry>, state: ActivityState, now_ms: i64) -> ActivityEntry;
pub fn answered(entry: ActivityEntry) -> ActivityEntry;
pub fn classify(entry: &ActivityEntry, turn_driven: bool, now_ms: i64) -> ActivityInfo;
```

`ActivityEntry` carries the last report beside the pane bookkeeping, in the
same in-memory map, keyed by session. The server's report hook writes into it;
the notifier tick writes the pane half. The draft's `proto` field is not
there: the emitted block is recognised by URL, so a stale one is replaced on
the next launch rather than needing a version to be told apart.

**Authority and lapse.**

- `blocked`, `done` and `idle` are *resting* states. They persist with no
  further events, and they outrank the pane: an agent redrawing as its turn
  ends, or the user typing at the prompt afterwards, does not make a finished
  turn `working`.
- `working` is *heartbeat-backed*. The tool hooks fire all through a turn, and
  an Esc mid-turn ends it with no `Stop`, so a `working` with no report inside
  `REPORT_WORKING_TTL_MS` (10s) falls back to the pane, exactly as if nothing
  had been reported. The pane is right about working, so this costs nothing: a
  long tool call still animates a spinner. (Not "counting the turn as driven":
  a headless loop attempt draws nothing while its model thinks, and that would
  read every pause between its tool calls as a finished turn.)
- `blocked` ends on the user's keystroke. Because a refusal fires nothing, any
  key that can answer a dialog (`sendq::answers_a_prompt`: Enter, a printable
  key, a lone Esc, Ctrl-C) clears a reported `blocked` and hands the session
  back to the pane. An approval then reports `working` within a moment; a
  refusal leaves the pane to read a quiet prompt as a guessed `done`, until
  `idle_prompt` reports `done` for real. `classify_input` could not be reused
  for this, since it files a lone Esc under "nothing typed" on purpose.
- A dead process is neither blocked nor done. The draft had `classify` take
  `agent_running`; it turned out every consumer (the UI, the notifier, the send
  queue) already composes the session's status first, so the fact did not need
  to enter the fold.

## What changes around it

**`sendq.rs`.** The send queue types review comments, failing checks and merge
conflicts into a session once it is quiet, and appends them anyway after
`MAX_HOLD_MS`. A blocked session is quiet. An Enter that reached an
`AskUserQuestion` question picked its first option while this was being
verified, and the first option of a permission dialog is "Yes". So a blocked
session now holds the queue **with no timeout**: a queued message must never
approve a command the user did not see.

**`notifier.rs`.** `RunSnapshot` and `RunWatch` carry the reported state of the
tab speaking for the run, and `step()` fires on its edges: `Blocked` ("Agent
needs you", "`{label}`: waiting on your answer") on entering `blocked`, and the
existing turn-finished notification on entering `done`. While a report is
live, the quiet-pane timer does not fire, so `idle_secs` only governs agents
without hooks. `NotifSettings` gains `agent_blocked`, default on, and old saved
settings load with it on. `agent_idle` keeps its name: renaming a key for
tidiness is churn in every saved settings file, and the label already says
"Agent finished a turn". The other notification bodies lost their em dashes on
the way past.

The loop question from the draft is answered yes: a loop attempt that blocks
notifies. Loops suppress per-attempt events because the loop recovers from them
on its own, and a dialog is the one thing it cannot recover from.

`suppressed()` is unchanged. Its reasoning (stay quiet about the one run the
user is watching) is independent of how the state was derived.

**The UI.** `RunActivity` is `"working" | "blocked" | "done" | "idle"` plus
`reported`. `runStatus` gives `blocked` its own red, glowing dot and
"blocked · 2m"; `done` keeps the amber dot as "done" when reported and
"waiting" when guessed. The overview header counts `blocked` and `waiting`
separately, each with its own filter, and a project row says "N blocked". An
issue row's activity dot turns red when a linked agent is blocked.
`needsAttention` is `blocked || done`.

## Privacy and trust

- **The payloads stay on the machine.** The hook posts to `127.0.0.1` on a port
  the run already owns. Nothing here is a network call in the sense the README
  makes a claim about, and nothing here changes that claim.
- **Do not store what we do not need.** Three fields are read; the rest of the
  body, including the tool input, is never kept.
- **Default-deny on what we accept.** Exact event and notification-type values.
  A denylist here would quietly mis-state a run the first time an agent adds an
  event.

## Phasing

1. **Claude Code end to end.** Built: the `state` cap and route, the session
   header, emission with merge/tracked/exclude, the report in `activity.rs`,
   four states on the wire, the UI.
2. **`notifier.rs`.** Built: `Blocked` and turn-finished as edges on reported
   state, the setting, the copy.
3. **The rest of the catalog.** Open. Per-agent hook tables verified against
   live binaries, one agent per change, each landing with its own test. For
   an agent whose hooks can only run a command, that means a shell hook, and a
   decision about writing a script into the worktree that this change avoided.
4. **The roll-up and the sort** (AGE-213, AGE-215). Open, and now unblocked.

AGE-207 (declarative screen rules) is the tier for agents phase 3 cannot
reach. AGE-208 (agent-authored status) landed first, on the same server.
AGE-209's `wait(run_id)` was gated on phase 1 and no longer is.

## Open questions

- What fires under `bypassPermissions` and `--dangerously-skip-permissions`?
  `PermissionRequest` presumably never does, which is correct, but it could not
  be confirmed without accepting a warning on the user's behalf.
- Does an MCP elicitation dialog fire `PermissionRequest`? If it fires only a
  `Notification` with `elicitation_dialog`, it needs adding to the allowlist.
- After an Esc on a permission dialog, the pane guesses `done` for up to 60s
  before `idle_prompt` reports it. The turn-finished notification can then
  fire for a turn the user ended themselves, a minute after they did, unless
  they are still watching that run.
- `session_id` arrives on every event but `SessionStart`, and AGE-210 wants it.
  Nothing stores it yet.
- Does a `blocked` run still count toward a project's "working" roll-up for the
  purposes of the tray icon, or is blocked strictly stronger there too?
