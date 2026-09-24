# Design: scheduled runs

Status: **draft for discussion** (2026-09-24, AGE-237). Nothing here is built.
Scope: let a user say "run this agent, with this prompt and model, on this
schedule", on a machine that is sometimes asleep and sometimes has Agency
quit.

## The question, and the short answer

AGE-237 asks whether Agency should have recurring jobs the way the hosted
assistants do (a task, a model, a cadence) given that a desktop app is not
always on.

Yes, but only if it is honest about the machine it runs on. A hosted runner
fires on a server that is always up, so "every day at 9" means every day at 9.
Agency fires on a laptop. The lid closes, the app gets quit, the network is
down for the first minute after wake. A schedule here is a **best-effort
intent**, and the design is mostly about three things that follow from that:

1. What happens to a firing that was due while the Mac was asleep or Agency was
   quit, and making that visible instead of silent.
2. What a firing *produces*, since every Agency run is a worktree and a branch,
   and a nightly schedule left alone for a month is thirty of them.
3. That an unattended agent gets exactly the permissions the user chose for it,
   visibly, and not one more.

Almost none of the machinery is new. A firing is a loop run, which already has
a headless one-shot recipe, caps, a pure transition function, persistence
across restarts and a finish flow. The new part is a small, pure "is anything
due" function on the existing 2 s poller, and a table.

## What is true today

- **Closing the window does not quit.** `lib.rs` turns `CloseRequested` into
  `window.hide()`; the app lives on in the menu bar with the notifier thread
  still ticking every 2 s. Only the tray's Quit (or Cmd-Q, confirmed) exits.
- **Quitting stops everything.** `lifecycle::confirm_quit` kills every daemon
  session and shuts `agency-termd` down. Nothing of Agency runs while it is
  quit, and nothing is registered with launchd.
- **There is no open-at-login.** Agency runs from the moment the user opens it
  until they quit it or log out.
- **Sleep is invisible to the poller.** Nothing listens for sleep or wake. The
  notifier thread simply stops while the machine sleeps and resumes after.
- **Loop runs are the unattended primitive.** `looper.rs` drives headless
  attempts from each profile's `loop_args` (`claude -p … --permission-mode
  acceptEdits`, `codex exec --full-auto …`), with an attempt cap, optional
  wall-clock and token caps, a crash-loop guard, and a `stall_reason`. An empty
  check command with `max_attempts = 1` is a single headless attempt.
- **Model is already per-run.** `create_run`/`create_loop` take `model`, and
  `agent_catalog.rs` knows how each CLI takes one (`ModelDelivery`). **Effort
  is not a concept anywhere in Agency** yet.

## The three states the machine can be in

| State | Poller ticks? | Agents run? | What a due schedule does |
|---|---|---|---|
| Agency running, awake (window open or hidden) | yes | yes | fires on time |
| Mac asleep (lid closed, idle sleep) | no | frozen | fires on the first tick after wake, per the catch-up rule |
| Agency quit, or logged out | no | none | fires on the first tick after next launch, per the catch-up rule |

The first row is the product. The other two are where a desktop scheduler earns
or loses trust, and both reduce to the same case: *the scheduler wakes up and
finds it has missed one or more due times.* So there is one catch-up rule, not
one for sleep and another for quit.

### The catch-up rule

On every tick the scheduler compares each schedule's `last_due_handled` with
now. If one or more due times have passed:

- **Coalesce.** However many were missed, fire at most once. Seven missed
  nightly runs after a week's holiday is one run, not seven agents racing over
  the same base. This is how launchd treats missed calendar intervals too, and
  for the same reason.
- **Within grace, fire late.** If the most recent missed due time is within the
  schedule's grace window, fire now. Default grace is the smaller of the
  schedule's own period and 12 hours: a daily 9:00 job whose Mac woke at 11:30
  runs at 11:30; the same job found at 9:05 the next morning has a fresher due
  time of its own and the stale one is dropped.
- **Outside grace, skip and say so.** The schedule records `missed: n` with
  the time range, and the schedule row shows "missed 3 while Agency was closed"
  until the next successful firing. One notification per catch-up, never one
  per missed firing.
- **Per-schedule override.** "Skip missed firings" (fire only on time) and
  "always catch up once" (grace = unbounded) are the two other choices. Some
  jobs are only worth doing at the time they name: a stand-up summary at 17:00
  is noise at 08:00.

Everything is recorded against the *due time*, not the firing time, so a late
firing does not shift the schedule. A daily 9:00 job that ran at 11:30 is next
due at 9:00 tomorrow.

### Detecting a wake without listening for it

The poller already ticks every 2 s. After a sleep, the first tick sees wall
clock jump far past the 2 s it slept. That is enough: the catch-up rule works
from wall-clock due times, so it needs no sleep/wake notification at all.

One trap for whoever builds it: `std::time::Instant` on macOS reads
`CLOCK_UPTIME_RAW`, which **does not advance during sleep**. Everything in
the scheduler has to be computed from `SystemTime` (wall clock), or a Mac that
slept through 9:00 believes it is still 8:59.

After a detected wake (a wall-clock jump of more than, say, 60 s) the scheduler
waits a settle delay before firing anything, default 60 s. The first minute
after wake is when Wi-Fi, VPN and the agent's own auth refresh are still coming
up, and a firing that fails its first API call there burns a crash-loop strike
for nothing. This is a design choice, not an observed failure; drop it if it
turns out not to matter.

### What we deliberately do not do

- **Wake the Mac to run a schedule.** `pmset schedule wake` needs admin rights
  and changes a system setting. Rejected outright; it breaks "never mutate a
  user's global config".
- **A launchd agent that fires while Agency is quit.** It would have to start
  the whole app (the daemon, the worktree code and the agent environment all
  live there) and it writes a plist into `~/Library/LaunchAgents`. The user
  quit the app; starting it behind their back to run an agent is exactly the
  kind of autonomy that has to stay visible and chosen. Rejected for v1.
- **Prevent sleep while waiting for a schedule.** Keeping a laptop awake all
  night for a job due at 3:00 is a battery cost the user did not ask for.

What we do instead:

- **Open at login, as a Settings toggle.** Registered through the OS login-item
  API (`SMAppService.mainApp` on macOS 13+), which writes no file of ours and
  shows up in System Settings where the user can see and revoke it. Off by
  default. The schedule list offers it ("Agency is not set to open at login, so
  schedules pause when you log out") only while at least one schedule exists.
- **Stay awake while a firing is in flight.** Hold a
  `PreventUserIdleSystemSleep` assertion (what `caffeinate -i` holds) from a
  firing's start until its loop reaches a terminal state, so idle sleep does not
  freeze an agent halfway through a turn. Closing the lid still sleeps; that is
  the user's call and we do not override it. A Settings toggle, on by default,
  and it goes into `porting.md` as a macOS assumption (Linux has
  `systemd-inhibit`).
- **Say it at quit.** The quit confirmation already counts running agents. With
  schedules enabled it adds one line: "2 schedules will not run while Agency is
  closed."

## What a firing is

A firing creates **a loop run from the schedule's template**. Not an
interactive run.

An interactive session never exits: it idles at its prompt, and if it hits a
permission prompt at 3:00 it sits blocked until the user wakes up. That is the
exact problem `agentic-loops.md` solved with the headless recipe, and a
schedule needs the same clean boundary: the process exits, the check runs, the
loop is terminal, the schedule knows the firing is over. So:

- A schedule with no check command fires a loop with `max_attempts = 1` and an
  empty check: one headless attempt, then Complete. This is the common case
  ("summarise yesterday's commits", "bump the patch dependencies and run the
  tests").
- A schedule with a check command fires a real loop ("every night, fix one
  failing test until `cargo test` is green, at most 5 attempts").
- Agents without `loop_args` cannot be scheduled, for the same reason they
  cannot loop. The agent picker filters them out, exactly as Loop mode does.

Each firing is a normal run: its own worktree off the base, its own
`agent/<id>` branch, its own terminal, the same review, merge and archive. It
carries a nullable `schedule_id` (the `race_id` precedent: a grouping field,
not a new run kind) and a title of the schedule's name plus the due date, so
thirty of them are distinguishable.

### Where the output goes

A scheduled job produces one of two things, and they need different endings:

- **A change** (commits on the branch). It waits for review like any run. The
  notification says so: "Nightly deps: ready for review".
- **No change** (the agent reported, summarised, or found nothing to do). The
  branch is at the base, so by `run-teardown.md`'s own model it is *merged*:
  archiving it loses nothing, and the record keeps the transcript. Such a
  firing is archived automatically, and the schedule row links to its record
  so "what did last night's triage say" is one click. Notification is off by
  default for these.

Posting a report somewhere more useful than a transcript (a comment on a
tracked issue, say) is a v2 candidate below, not v1.

### Backpressure: the thirty-branches problem

Left alone, a daily schedule that produces changes creates a reviewable run
every day. Two guards:

- **Overlap: skip while the previous firing is still running.** A firing due
  while the last one is still in its loop is recorded as skipped ("previous
  firing still running"), not queued. Queueing just moves the pile-up later.
- **Pile-up: pause at N unreviewed.** When a schedule has N firings awaiting
  review (live, not archived, with commits), the schedule pauses itself and
  notifies once: "Nightly deps paused: 3 runs waiting for review". Default N
  is 3; the user can raise it. Reviewing, merging or archiving any of them
  un-pauses it on the next tick. This is the one cap that is on by default,
  because it caps something the user has not seen yet, not something they are
  spending.

## Permissions and cost

Scheduled firings are the most unattended thing Agency will do, so the two
commitments from `CLAUDE.md` apply at full strength.

**Never silently escalate.** A firing uses the profile's `loop_args` verbatim.
The schedule editor shows the exact permission flags the firing will run with
(`--permission-mode acceptEdits`, `--full-auto`, …) on the form itself, not
behind a disclosure, because at schedule time the user is choosing what an
agent may do while they are not there. Scheduling never adds a flag, and there
is no "unattended mode" that widens anything.

**Caps are chosen, never hidden.** `agentic-loops.md` rule 1 stands: a cap the
user did not ask for never trips. The editor pre-fills a wall-clock cap (60
minutes) as a visible, editable value; a pre-filled number on the form is a cap
the user chose by leaving it. Clearing it means off. The attempt cap is
required, as it already is for loops.

## Model and effort

The issue asks for "which model/effort". Model is solved: the schedule stores
it and the firing passes it, through the same `checked_model` and
`ModelDelivery` path as every other run.

Effort does not exist in Agency for any run. It is a per-agent flag with a
different name and value set per CLI, which is the same shape as
`ModelDelivery` (an `EffortDelivery` in the catalog, a picker that hides for
agents without one). It should be built for all runs, not just scheduled ones,
and a schedule then gets it for free. Filed separately as AGE-242 rather
than smuggled into this.

## The schedule itself

### Cadence

Not a cron string in v1. Cron is compact for people who already read cron and
opaque to everyone else, and five fields do not say what happens across a DST
change. The editor offers:

- every N hours (N in 1, 2, 3, 4, 6, 8, 12), aligned to the hour;
- daily at HH:MM;
- on chosen weekdays at HH:MM (covers "weekdays at 9");
- weekly on a day at HH:MM.

Those cover the cadences people actually ask for (hourly-ish, daily,
weekdays, weekly). A cron field can come later as an "advanced" option over
the same `next_due`.

Times are **local wall-clock time**, because that is what a person means by
"9:00". Across DST: a time that does not exist (02:30 on spring-forward day)
fires at the first valid minute after it; a time that happens twice (01:30 on
fall-back day) fires once, at the first. Both are table tests. Local time needs
`chrono` with its `clock` feature as a direct dependency; it is already in the
lockfile transitively, so this should not change `THIRD-PARTY.md`, but run
`scripts/third-party.py` to confirm.

### Where schedules live

In the per-machine SQLite registry, **never in `.agency/agency.toml`**. A
schedule in tracked config would travel with every clone, and a teammate who
pulls would find agents firing on their machine that they never set up. That
is the "default-deny for anything shared" rule in its plainest form. If
sharing schedules is ever wanted, it arrives as an import the receiving user
confirms, one schedule at a time.

```
schedules
  id              TEXT PRIMARY KEY
  project_id      TEXT NOT NULL
  name            TEXT NOT NULL
  enabled         INTEGER NOT NULL
  cadence         TEXT NOT NULL   -- JSON: {"kind":"daily","at":"09:00"} …
  catch_up        TEXT NOT NULL   -- "grace" | "skip" | "always"
  template        TEXT NOT NULL   -- JSON: prompt, agent, model, base,
                                  --   merge_target, LoopConfig
  max_unreviewed  INTEGER NOT NULL  -- default 3
  state           TEXT NOT NULL   -- JSON: last_due_handled, last_run_id,
                                  --   missed, paused_reason
runs.schedule_id  TEXT NULL       -- column_exists migration, like race_id
```

JSON columns for the same reason `agentic-loops.md` gave: written and read as a
unit, never queried by their parts.

### The pure part

A new `crates/agency-app/src/scheduler.rs`, the `looper.rs` shape exactly:

```rust
pub fn next_due(cadence: &Cadence, after: NaiveDateTime, tz: &impl TimeZone)
    -> DateTime<Utc>;

pub fn step(
    sched: &ScheduleState,
    cfg: &ScheduleConfig,
    snap: &ScheduleSnapshot,   // now, last_tick, previous firing's status,
                               // unreviewed count
) -> (ScheduleState, Vec<ScheduleAction>);

pub enum ScheduleAction {
    Fire { due: DateTime<Utc> },
    RecordMissed { from: DateTime<Utc>, to: DateTime<Utc>, count: u32 },
    RecordSkipped { due: DateTime<Utc>, why: SkipReason }, // Overlap | Paused
    Pause(PauseReason),                                     // Unreviewed(n)
    Unpause,
    Notify(ScheduleEvent),
}
```

No repo, no daemon, no clock of its own: `now` comes in on the snapshot.
Everything in this document (catch-up, coalescing, grace, settle delay, DST,
overlap, pause and unpause) is a table test against `step` and `next_due`. The
driver in the poller does the side effects: `Fire` calls the same inner
function `create_loop` does and stamps `schedule_id`.

Firing is off-thread, like everything slow on the poller: creating a worktree
can take seconds and the 2 s tick must not stall.

## UI

- **A Schedules section per project**, under the run list: one row per
  schedule with its cadence in words ("weekdays at 09:00"), next due time, last
  result, and any missed or paused state. Glyph `◷` (no emoji presentation;
  pin with U+FE0E anyway if the font disagrees).
- **The editor is the Loop dialog plus a When section**: name, cadence, catch-up
  choice, the unreviewed limit. The permission flags show inline. A "Run now"
  button fires once immediately without touching the schedule's due times,
  which is also how a user tests a schedule before trusting it overnight.
- **Firings are ordinary runs** in the run list, marked with the glyph and
  grouped under their schedule's name.
- **Notifications**: firing finished with changes (on), finished with no
  changes (off), missed on catch-up (on, one per catch-up), paused for review
  backlog (on). No "firing started" toast; the loop already reports its own
  completion and stalls, and per-attempt noise is suppressed for loops today.

Copy to keep straight, since this is where overclaiming would creep in: "Runs
while Agency is open. Missed times run once when Agency is next open, if they
are recent." Never "runs every day at 9".

## Phasing

**v1** (AGE-241): the `schedules` table and `runs.schedule_id`; `scheduler.rs` with its
tests; the poller driver; the four cadences; catch-up with grace, skip and
always; overlap skip; unreviewed pause; auto-archive of no-change firings; the
editor, section and notifications; the idle-sleep assertion while a firing is
in flight; the quit-dialog line.

**Separate issues, not v1**:

- Open at login, AGE-243 (useful to everyone, and a prerequisite for schedules to feel
  reliable, but not coupled to them).
- Effort, for all runs, AGE-242.

**v2 candidates**, in rough priority order:

1. **Issue-linked schedules.** "Every Monday, work the top backlog issue", or a
   firing whose report is appended as a comment on a named tracked issue. The
   tracker already takes comments by file; this makes a report land where the
   user looks.
2. A cron field as an advanced cadence.
3. Fetch before firing, so the worktree is cut from the remote tip rather than
   the local base. Needs a policy for what a firing does when the fetch fails
   offline.
4. Trigger on an event rather than a clock (a PR opened, CI failing on main).
   Different mechanism, same template and firing path.

## Open questions

1. **Headless permission denial per agent.** The design assumes a headless
   agent *denies* an unpermitted tool and carries on or exits, rather than
   blocking on a prompt with no one to answer it. That is the expected
   behaviour of `claude -p` and has to be confirmed there too. The others have to be checked one by one before their scheduling is
   enabled; any that block cannot be scheduled.
2. **Grace default.** min(period, 12 h) is a guess. The alternative is always
   catching up once, simpler to explain and wrong for time-of-day jobs.
3. **Settle delay after wake.** 60 s, unobserved. Keep it until a firing is
   seen failing without it, or drop it and see.
4. **Pile-up pause counts which runs?** "Live with commits" is the proposal. A
   run the user opened, looked at and left is arguably reviewed; we have no
   signal for "looked at" today, so it counts until merged or archived.
5. **Multiple machines.** Two Macs with the same project both have their own
   registry, so a schedule set up on both fires twice. Machine-local is the
   right default; worth one line in the editor ("This schedule runs on this
   Mac only").
