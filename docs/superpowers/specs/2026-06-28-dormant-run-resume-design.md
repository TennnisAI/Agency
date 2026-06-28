# Resume Stopped Runs on Reopen (Dormant-Run Reactivation) — Design

**Date:** 2026-06-28
**Branch:** `feat/termd` (continues the terminal-daemon work)

## Goal

After the app is quit and reopened, past agent/terminal runs currently render as
dead, unrecoverable panes (a blinking cursor and nothing else) because the UI
attaches to a daemon session that no longer exists. This makes Quit effectively
destroy work.

Fix it so a stopped run is **dormant, not broken**: the run list shows it as
stopped, and **clicking it immediately brings it back** — resuming the prior
agent conversation where the agent supports it, or starting fresh otherwise. No
ghost processes (nothing keeps running after Quit) and no auto-restart-everything
on launch.

## Background (what already exists)

- **Run records** persist in SQLite (`crates/agency-core/src/registry.rs:19-33`):
  `id`, `project_id`, `agent` (profile name), `prompt`, `base`, `branch`, `kind`
  (`"agent"` | `"terminal"`), `archived_at`, `title`, etc.
- **Worktree** path is deterministic: `<repo>/.agency/worktrees/<run_id>`, branch
  `agent/<run_id>` (`crates/agency-core/src/worktree.rs`). Worktrees survive an app
  quit (they are on-disk git worktrees).
- **Agent profiles** persist in the `profiles` table (`registry.rs:67-72`):
  `name`, `command`, `args` (JSON), `env` (JSON). `AgentProfile::render_args(prompt)`
  substitutes `{{prompt}}`.
- **Spawning**: `create_run` (`state.rs:473-516`) and `create_terminal`
  (`state.rs:527-555`) build command/args/env/cwd and call
  `term.start_session(...)`. An agent runs the profile command in its worktree; a
  terminal runs `$SHELL -l` in the project root.
- **`rerun(id)`** (`state.rs:769-788`) already re-spawns an agent by re-rendering
  the original prompt — but it is a "start over from the prompt" action, not a
  resume.
- **Status** is live-only: `run_status` (`state.rs:562`) → daemon `Status` →
  `SessionStatus::{Running, Exited{code}, Gone}`. A run whose daemon session is
  gone reports `Gone`. There is no persisted status column.
- **The bug**: on focus, `FocusTerminal.tsx:87` → `attach_run` → `subscribe`. If
  the daemon has no session, `subscribe` errors (`server.rs:123-126`) and the pane
  shows blank. After a Quit (which shuts the daemon down), *every* prior run is
  `Gone`, so they all render dead.

Everything needed to re-spawn a run is already persisted — the missing piece is a
reactivation path and a non-broken dormant state.

## Behavior

- On reopen, runs list as today, each annotated **running** vs **stopped**
  (`Gone` = stopped). Nothing auto-restarts on launch.
- **Click a stopped run → it resumes immediately.** A brief "resuming…" transient,
  then the live session:
  - **Resume-capable agents** (claude, codex, pi, opencode, copilot): re-launch the
    agent with its resume recipe **in the run's existing worktree**, restoring the
    prior conversation from the agent's own session store.
  - **Fresh-only agents** (cursor, hermes) and any agent with no resume recipe:
    start a new agent session using the stored profile + original prompt.
  - **Terminals**: a fresh login shell in the project root (a shell has nothing to
    resume).
- The dead-blank-pane bug disappears: the UI never attaches to a missing session —
  it ensures the run is active first.

## Resume recipes (per-profile)

`AgentProfile` gains one optional field, `resume_args: Option<Vec<String>>`, and
the `profiles` table gains a nullable `resume_args` column (JSON). A migration adds
the column; existing rows default to `NULL` (= fresh-restart fallback). When a run
is reactivated and its profile has `resume_args`, the agent is spawned as
`command + resume_args` (no prompt re-render), run in the worktree. Otherwise it
falls back to the normal `render_args(prompt)` fresh start.

Seeded defaults (only for profiles the app seeds; user profiles keep whatever they
set):

| Profile | `command` | `resume_args` | Resume kind |
|---|---|---|---|
| claude | `claude` | `["--continue"]` | cwd-keyed resume |
| codex | `codex` | `["resume", "--last"]` | cwd-scoped resume |
| pi | `pi` | `["--continue"]` | cwd-keyed resume (verified: per-path session store) |
| opencode | `opencode` | `["--continue"]` | cwd-keyed resume |
| copilot | `copilot` | `["--continue"]` | cwd-keyed resume |
| cursor | `cursor-agent` | `null` | **fresh** (session id-keyed, not cwd — would resume the wrong global session) |
| hermes | `hermes` | `null` | **fresh** (verified id/name-keyed: `~/.hermes/sessions` is a flat global store filtered by source, not cwd) |

Rationale for cursor/hermes being fresh: their `--continue` resumes the *globally
most-recent* session rather than the current worktree's, so with multiple
concurrent runs it could resume the wrong conversation. A fresh start is correct
and predictable; precise per-run resume for them needs the deferred per-run
session-id assignment (see Out of scope). Resume recipes were sourced from each
CLI's local `--help` (claude, pi, opencode, cursor, hermes — all installed) or
official docs (codex, copilot); all resume interactively in a PTY, matching our
spawn model.

## Backend: `ensure_run_active(id)`

One new entry point the frontend calls before subscribing. It guarantees a live
daemon session exists for the run, then returns; the frontend subscribes as it does
today.

Logic:
1. Query the run's session status.
   - `Running` or `Exited` (session still present in the daemon) → **no-op** (the
     existing session is attachable as-is).
   - `Gone` (no session) → respawn (below).
2. Respawn a gone run:
   - Load the run record. If its worktree is missing, restore it (reuse the
     existing worktree-restore path that `rerun` uses — checkout the run's branch).
   - **Terminal** (`kind == "terminal"`): `start_session` with `$SHELL -l` in the
     project root (same as `create_terminal`).
   - **Agent** (`kind == "agent"`): load the profile.
     - Profile has `resume_args` → `start_session` with `command + resume_args`,
       cwd = worktree, env = the same env assembly used at create time
       (`provider_env` + profile env + script env).
     - No `resume_args` → fresh start: `start_session` with
       `command + render_args(prompt)`, cwd = worktree (same as a normal create).

`ensure_run_active` is a focused sibling of `rerun`, not a replacement: `rerun`
stays the explicit "start over from the prompt" action (it kills any existing
session and re-sends the prompt); `ensure_run_active` is the transparent "bring it
back, preferring resume" path that only acts when the session is gone.

This is exposed as a Tauri command `ensure_run_active(id)` in `commands.rs`,
mirroring the existing command style.

## Frontend: status in list + resume-on-focus

- **Run list** renders each run's status (running vs stopped) from the existing
  status polling (`run_status` → `Gone` = stopped). A small indicator/label; follow
  the list's current styling conventions (no new design system).
- **Resume-on-focus**: when a run pane mounts/focuses, the attach flow becomes
  `ensure_run_active(id)` → then `attach_run`/`subscribe` (today it subscribes
  directly). The "resuming…" transient is the gap until the first snapshot arrives.
  On `ensure_run_active` error, surface it in the pane (not a silent blank).

## Error handling

- `ensure_run_active` returns a clear error if the run record is missing, the
  worktree can't be restored, or the spawn fails; the pane shows the error rather
  than a blank cursor.
- Reactivation is best-effort per run and never blocks other runs.

## Scope boundaries

**In scope**
- `resume_args` profile field + migration + seeded defaults.
- `ensure_run_active` backend command (resume-or-fresh-or-shell).
- Run-list stopped/running status indicator.
- Resume-on-focus wiring; removal of the dead-blank-pane behavior.

**Out of scope (separate follow-ups)**
- Per-run session-id **assignment** for precise resume of id-keyed agents (cursor,
  hermes): assign the run id as the agent's session id/name at launch where the CLI
  supports it (hermes `-c <name>` / cursor `create-chat` → `--resume <id>`, and the
  `--session-id` flags several others expose), then resume by that exact id. This is
  cleaner than scraping ids from output and would let the currently-fresh agents
  resume their exact run.
- The daemon-crash live-terminal-freeze surfacing flagged in the termd final review
  (related but a different code path).
- Persisting/replaying Agency's own transcript for a frozen pre-resume preview.
- Bulk "resume all on startup."

## Testing

- **Backend unit tests**:
  - `resume_args` round-trips through the `profiles` table and the migration
    (NULL → `None`; JSON array → `Some(vec)`).
  - `ensure_run_active` on a `Gone` agent run with `resume_args` spawns
    `command + resume_args` (assert the session exists and the recorded argv).
  - `ensure_run_active` on a `Gone` agent run *without* `resume_args` falls back to
    the rendered-prompt fresh start.
  - `ensure_run_active` on a `Gone` terminal run spawns a shell in the project root.
  - `ensure_run_active` on a `Running`/`Exited` run is a no-op (does not respawn).
  - Worktree restore is invoked when the worktree is missing.
- **Frontend**: manual — reopen after Quit, confirm stopped runs show as stopped
  and clicking resumes (agent context returns for resume-capable agents; cursor/
  hermes/terminals start fresh).
