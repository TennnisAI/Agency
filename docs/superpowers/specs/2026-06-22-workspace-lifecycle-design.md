# Workspace Lifecycle — Design

**Date:** 2026-06-22
**Status:** Approved (design); ready for implementation planning
**Topic:** Setup/run scripts, port allocation + preview, archive, review-comment-to-agent, notifications

## Motivation

A competitive review of an adjacent tool surfaced five gaps in agency's workspace
lifecycle. Today a new agent run gets a bare `git worktree` with no dependencies,
no way to run the app it's editing, only destructive run cleanup (stop/discard),
no path from a diff comment back to the agent, and no notifications. This design
closes those gaps while preserving agency's local-first / zero-data-collection
posture (no cloud, no required GitHub integration).

The five features share one new foundation: **committed per-project config**.

## Non-goals

- Cloud sync, GitHub PR/CI integration, or any network dependency for these features.
- Multi-agent orchestration / DAGs (out of scope; separate effort).
- Preserving agent terminal scrollback across archive (branch + diff + metadata
  are preserved; live scrollback is not — see Feature 3).
- A full layered settings hierarchy (user/managed tiers). We adopt
  repo + machine-local layering only; user/managed tiers are a future extension.

## Foundation — committed project config

**New module:** `agency-core/src/config.rs`.

Loads and merges, in precedence order (later overrides earlier):

1. `.agency/agency.toml` — git-tracked, shared with teammates.
2. `.agency/agency.local.toml` — machine-local override, git-ignored.

Secrets (Anthropic API key, provider URLs) stay in the SQLite `settings` table and
are **never** written to TOML.

**Exclude change:** `WorktreeManager::ensure_excluded` (worktree.rs:51) currently
writes `.agency/` to `.git/info/exclude`. Narrow it to `.agency/worktrees/` and
`.agency/agency.local.toml` so `agency.toml` is trackable while worktrees and
machine overrides stay invisible to git. Migration: on load, if the old `.agency/`
line is present, replace it with the two narrower lines.

**Schema:**

```toml
[scripts]
setup    = "pnpm install"                  # optional
run      = "pnpm dev --port $AGENCY_PORT"   # optional
archive  = "./scripts/cleanup.sh"           # optional
run_mode = "concurrent"                     # "concurrent" (default) | "nonconcurrent"

[ports]
base       = 5200   # default 5200
block_size = 10     # default 10
```

**Config type (Rust):**

```rust
pub struct AgencyConfig {
    pub scripts: ScriptsConfig,   // setup/run/archive: Option<String>, run_mode: RunMode
    pub ports: PortsConfig,       // base: u16 (5200), block_size: u16 (10)
}
```

A missing file, missing tables, or missing keys all resolve to defaults — agency
must behave exactly as it does today when no `.agency/agency.toml` exists.

**Injected environment** (available to every lifecycle script):

| Variable | Value |
| --- | --- |
| `AGENCY_WORKSPACE_PATH` | absolute path of the run's worktree |
| `AGENCY_ROOT_PATH` | absolute path of the project repo |
| `AGENCY_PORT` | base port allocated to this run |
| `AGENCY_WORKSPACE_NAME` | the run/task id (e.g. `fix-login-a3k2`) |

## Feature 1 — Setup scripts

**New module:** `agency-core/src/scripts.rs` — resolves a lifecycle script + builds
its env. Reused by run and archive scripts.

**Wiring:** in `AppState::create_run` (state.rs:308), after `WorktreeManager::create`
and before `tmux.start_session`, if a `setup` script is configured the session
command becomes:

```
sh -lc '<setup> && exec <agent-command> <args...>'
```

Rationale for the chained single-session model (approved):
- One tmux session — setup output is visible inline in the agent terminal.
- The agent launches **only if** setup exits 0 (`&&` + `exec`).
- `exec` replaces the shell so the agent is PID 1 of the session (clean signals,
  status, resize — same as today).

When no setup script is configured, the command is the agent command directly
(today's behavior, unchanged).

**UX:** while setup runs, the run's status reads "Preparing workspace…". On setup
failure the agent never starts; the terminal shows the error. The user can
**Re-run** (re-attempts setup) or **Discard**.

**Note on `rerun`:** `AppState::rerun` (state.rs:402) must apply the same setup
chaining so re-runs prepare the workspace too.

## Feature 2 — Run scripts + ports + preview

### Port allocation

`AGENCY_PORT = ports.base + slot * ports.block_size`, where `slot` is the lowest
free non-negative integer.

**When:** the port is allocated in `create_run`, before scripts run, so the same
`AGENCY_PORT` is in the injected env for the **setup**, **agent**, and **run**
processes alike. It is persisted on the run record (stable across reattach/rerun)
and released when the run is archived or discarded.

- **Schema:** add `port_base INTEGER` (nullable) to `runs`.
- The allocator computes the lowest free slot from the set of non-archived runs
  that currently hold a `port_base`, in `state.rs`.

### Execution

The run script executes in a **separate** tmux session `agency-run-<id>` — fully
independent of the agent session `agency-<id>`. This lets the dev server and the
agent run simultaneously and be stopped independently.

`run_mode`:
- `concurrent` (default): multiple workspaces can run their scripts at once.
- `nonconcurrent`: starting a run script stops any other workspace's run script
  first (for projects with fixed ports/DB).

**New commands** (mirror the agent attach path in commands.rs / state.rs):
- `start_run_script(task_id)` — start `agency-run-<id>` with the run command + env.
- `stop_run_script(task_id)` — kill the run session.
- `run_script_status(task_id)` → `SessionStatus`.
- `attach_run_script(task_id, onChunk)` / `detach_run_script` / `run_script_input`
  / `resize_run_script` — stream + interact, identical pattern to the agent PTY.

### UI

A **"Run" tab** within the focused workspace, alongside Agent and Diff:
- Streams `agency-run-<id>` logs via xterm (reuse `FocusTerminal`).
- Start/Stop controls; disabled (with a hint) when no `run` script is configured.
- **Embedded preview:** an `<iframe>` pointed at `http://localhost:$AGENCY_PORT`
  (localhost http loads fine inside the app webview). Manual + auto refresh.
- **Open in browser ↗** button (Tauri opener plugin) — opens the same URL in the
  user's real browser.

## Feature 3 — Archive

**Schema:** add `archived_at INTEGER` (nullable) to `runs` (idempotent
`ALTER TABLE ... ADD COLUMN`, guarded so re-running is safe).

**Archive action** (`archive_run(task_id)`):
1. Kill agent session and run-script session.
2. Run the `archive` lifecycle script if configured (cleanup of external resources).
3. `git worktree remove --force` — **but keep the branch `agent/<id>`** (do *not*
   `git branch -D`). This is the key difference from `discard_run`, which deletes
   the branch.
4. Set `archived_at = now` on the run record (record is kept, not deleted).

**Restore action** (`restore_run(task_id)`):
1. `git worktree add <worktrees>/<id> agent/<id>` (re-attach the kept branch; note
   **no** `-b` — the branch already exists).
2. Clear `archived_at`.
3. The user can then re-run the agent (fresh session on the existing branch).

**Listing:**
- `list_runs` filters out archived runs (`archived_at IS NULL`).
- New `list_archived_runs(project_id)`.

**UI:** an **Archive** button beside Stop/Discard in the focus controls and tile.
Archived runs appear in a collapsible **"Archived"** section in the project tree /
agents view, each offering **Restore** and **Discard**.

**Limitation (accepted):** terminal scrollback / agent chat is not preserved across
archive. Restore yields a fresh agent session on the same branch; the branch, its
commits, the diff, and run metadata are all intact.

`WorktreeManager` needs a `remove_keep_branch(task_id)` variant (or a `keep_branch:
bool` parameter) so archive removes only the worktree.

## Feature 4 — Review comment → agent

Builds on the diff panel from the in-flight `git-source-control-redesign`; sequenced
to land with/after it.

**Schema:** new table
`review_comments(id, run_id, path, line_start, line_end, body, sent INTEGER, created_at)`.

**Flow:**
1. In the diff viewer, select one or more lines → "Add comment" → body persisted to
   `review_comments` (unsent).
2. Comments render as inline threads in the diff and as a list in the review panel.
3. **"Send to agent"** composes a single markdown message:

   ```
   Please address these review comments:

   ### path/to/file.ts:42-45
   <comment body>

   ```diff
   <the relevant hunk>
   ```
   ```

   …and writes it to the agent PTY via the existing `run_input`. Sent comments are
   marked `sent = 1`.
4. If the agent session has exited, the action becomes **"Re-run with this
   feedback"** — re-runs the agent with the composed message as the prompt.

**New commands:** `add_review_comment`, `list_review_comments(run_id)`,
`delete_review_comment`, `send_review_comments(run_id)`.

## Feature 5 — Notifications

**Plugin:** add `tauri-plugin-notification`.

**Status watcher:** a background thread in `agency-app` (started at app init) polls
every ~2s across all known runs and fires OS notifications on state transitions.
It keeps a last-known-state map per run to detect edges (not levels).

**Events (all enabled by default):**

| Event | Detection |
| --- | --- |
| Agent finished | `agency-<id>` session transitions running → exited |
| Needs input / idle | tmux `monitor-silence` flag set on `agency-<id>` (no output for `idle_secs`, default 30s) while still running |
| Run script crashed | `agency-run-<id>` exits non-zero |
| Merge needs attention | merge returns conflicts, or resolver session exits |

Idle detection uses tmux's native silence monitoring: enable
`set monitor-silence <idle_secs>` on the session window at start, then read the
window's silence flag in the poll loop. Heuristic but cheap and agent-agnostic.

**Settings** (SQLite `settings` keys):
- Per-event toggles (4 booleans).
- `notify_only_when_unfocused` (default true).
- Suppress notifications for the run the user is **actively viewing** (focused run).

The frontend reports window focus state and the active/focused run id to the
backend so suppression is correct. A notification body includes the run title and,
where relevant, diffstat or conflict count (mirrors the previews shown in design).

## Data model summary (schema changes)

`runs` table gains:
- `port_base INTEGER` (nullable) — Feature 2.
- `archived_at INTEGER` (nullable) — Feature 3.

New table `review_comments` — Feature 4.

New `settings` keys — Feature 5 (notification toggles).

All migrations are additive and idempotent (`ADD COLUMN` guarded; `CREATE TABLE IF
NOT EXISTS`).

## Sequencing

1. **Foundation** — `config.rs` (load/merge `.agency/agency.toml`), exclude
   narrowing, injected-env helper, `scripts.rs`. Prerequisite for 1 and 2.
2. **Feature 1 — setup scripts** (chained into create_run / rerun).
3. **Feature 2 — run scripts + ports + Run tab + preview.**
4. **Feature 3 — archive** (independent; can parallel 2/3 after foundation).
5. **Feature 5 — notifications** (independent; needs the status watcher).
6. **Feature 4 — review-comment-to-agent** — rides on `git-source-control-redesign`;
   lands with/after that branch.

## Testing

- **config.rs:** unit tests for merge precedence, defaults on missing file/keys,
  env-var construction. Exclude-migration test (old `.agency/` line → narrowed).
- **scripts.rs:** setup-chaining command construction; failure short-circuits the
  agent (`&&` semantics) — assert on the composed command string.
- **ports:** allocator returns lowest free slot; releases on archive/discard; stable
  across rerun. Unit-testable in isolation.
- **archive:** worktree removed but branch survives (integration test in a temp
  repo, mirroring existing worktree tests); restore re-creates the worktree on the
  same branch; `list_runs` excludes archived.
- **review_comments:** CRUD + composed-message format (snapshot test on the markdown).
- **notifications:** edge-detection logic (running→exited fires once; silence flag
  transition) tested against a synthetic state sequence; the watcher's pure
  transition function is extracted so it's unit-testable without tmux.

## Open risks

- **Embedded preview:** some dev servers set `X-Frame-Options`/CSP that block
  iframing. Mitigation: the "Open in browser ↗" path always works; if the iframe is
  refused we show a hint + the open-in-browser fallback.
- **Idle heuristic:** `monitor-silence` flags genuine long-running work as "idle"
  (e.g. a 60s build with no output). 30s default is configurable; acceptable for v1.
- **Port reuse:** an externally-occupied port in a workspace's block will fail the
  run script. v1 surfaces the error in the Run terminal; a future iteration can
  probe for free ports.
