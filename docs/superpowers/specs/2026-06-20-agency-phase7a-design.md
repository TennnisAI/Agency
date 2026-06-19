# Agency Phase 7a — Multi-Agent Shell, Grid/Focus & tmux Sessions (Design)

**Date:** 2026-06-20
**Status:** Approved design, pre-planning
**Supersedes UI of:** Phases 2–6 frontend shell (the engine + git/merge/settings backends are reused)

## 1. Summary

Phase 7a rebuilds Agency's UI to match the designers' handoff (`docs/design-handoff/`)
and reworks agent sessions to be **durable via tmux**. The product becomes a true
multi-agent dashboard: inside a project you spin up multiple agents (picking the type —
Claude / Pi / Hermes / shell), each running in its own git worktree **and its own tmux
session**, shown as a grid of live tiles, openable into a focused terminal, persisted
across app restarts with live reattach.

This is the first of three Phase-7 slices. **7b** = Source Control redesign + per-hunk
staging + Git Review side-panel. **7c** = full modals (New task, Merge resolver), Settings
reskin, ⌘K search + keyboard shortcuts, collapsible-panel polish.

## 2. Goals / Non-goals

### Goals (7a)
- App shell: 40px title bar (sidebar toggle · logo · "Agency" · visual-only ⌘K search) +
  27px status bar.
- Collapsible sidebar with a **project tree**: projects expand to their agent runs (with
  status dots); Settings nav at the bottom.
- Content header: **Agents | Source Control** segmented nav; in Agents a **Grid / Focus**
  toggle and a **+ New task** action.
- **Agents · Grid**: a tile per run — status dot · prompt title · **agent-type badge**
  (Claude=peach, Pi=teal, Hermes=mauve, shell=default) · branch · diff stat (+adds −dels,
  files) · capture-pane terminal preview · footer status + contextual action.
- **Agents · Focus**: collapsible agent-list rail + one live terminal (tmux attach) +
  header (badge/branch/status + Approve→) + input line.
- **New task flow**: pick agent profile + prompt (+ base) → creates a run → tile appears →
  its tmux session + terminal spin up.
- **tmux-backed sessions**: agents survive app quit; relaunch reattaches live.
- **Persisted runs**: survive restart; show live (reattached) or Exited.
- Source Control nav routes to the existing git panel (unchanged in 7a). Settings + Merge
  modal reused unchanged.

### Non-goals (deferred)
- 7b: Source-Control redesign, per-hunk staging, history project-pills, Git Review panel.
- 7c: New-task modal & Merge-resolver modal full visual treatment, Settings reskin, ⌘K
  search wiring, ⌘N/⌘G/⌘↵ shortcuts, sidebar/rail collapse animations.
- Windows/Linux tmux equivalents (7a targets macOS; tmux assumed present — see §7).
- Bundling Geist/JetBrains Mono font files (system fallbacks).

## 3. Architecture

### 3.1 tmux session module (`agency-core`)
A new `agency_core::tmux` module replaces the direct portable-pty path for agent runs. The
**tmux session `agency-<task-id>` is the durable unit**; portable-pty is used only as the
transport for a live *attach*.

**Self-contained / bundled tmux:** the module resolves the tmux binary via a configurable
path — in the packaged app it points at a tmux binary bundled inside the `.app` (Tauri
`externalBin` sidecar); in dev it falls back to `tmux` on `PATH` (the developer's 3.6b).
`TmuxBackend::new(tmux_bin: PathBuf)` takes the path; a resolver picks bundled-if-present
else `"tmux"`. **Actually bundling + signing the per-arch tmux binary is a packaging-phase
task** (alongside icons/notarization) — 7a builds and tests against system tmux. Net: the
shipped app requires no user install.

- `start_session(name, cwd, command, args, env) -> Result<()>` —
  `tmux new-session -d -s <name> -c <cwd> -e K=V… -- <command> <args…>`; then
  `tmux set-option -t <name> remain-on-exit on`.
- `session_exists(name) -> Result<bool>` — `tmux has-session`.
- `session_status(name) -> Result<SessionStatus>` where
  `SessionStatus { Running, Exited(i32), Gone }` — from `tmux list-panes -t <name> -F
  '#{pane_dead} #{pane_dead_status}'` (dead=1 → Exited(status); dead=0 → Running; no
  session → Gone).
- `capture(name, lines) -> Result<String>` — `tmux capture-pane -p -e -t <name> -S -<lines>`
  (escape sequences preserved for color).
- `send_text(name, data: &str) -> Result<()>` — `tmux send-keys -t <name> -l <data>` for
  text; a small helper sends Enter/control keys.
- `attach(name) -> Result<AttachHandle>` — spawn `tmux attach -t <name>` in a portable-pty;
  returns a reader stream + writer for a live Focus terminal. Dropping it detaches (the
  session keeps running).
- `kill_session(name) -> Result<()>` — `tmux kill-session`.

The existing `supervisor` (direct portable-pty + `AgentHandle`) is removed for agent runs;
`portable-pty` remains as the `attach` transport. The merge-resolver (`resolve_merge`) is
re-pointed at a tmux session too (the resolver becomes `agency-resolver-<id>` in the repo).

### 3.2 Persistence + run model (`agency-app` / `agency-core::registry`)
- New SQLite table `runs(id TEXT PK, project_id TEXT, agent TEXT, prompt TEXT, base TEXT,
  branch TEXT, created_at INTEGER)`. CRUD on `Registry`: `insert_run`, `list_runs(project_id)`,
  `get_run(id)`, `delete_run(id)`.
- `AppState` no longer keeps live `AgentHandle`s; the tmux server holds liveness. It keeps
  only short-lived **attach** handles for currently-focused runs (`attaches: Mutex<HashMap
  <String, AttachHandle>>`).
- `RunInfo { id, project_id, agent, prompt, branch, status: SessionStatus, added: u32,
  deleted: u32, files: u32 }` — assembled by `list_runs` from the DB row + `tmux::session_status`
  + `git::diff_stat`.

### 3.3 Git diff stat (`agency-core::git`)
- `diff_stat(worktree, base) -> Result<DiffStat { added, deleted, files }>` via
  `git diff --numstat <base>...HEAD` (sum added/deleted, count files). Tiles render it.

### 3.4 Commands (`agency-app`)
- `create_run(project_id, prompt, agent, base) -> RunInfo` — insert run, create worktree,
  `tmux::start_session` with provider env; return the RunInfo.
- `list_runs(project_id) -> Vec<RunInfo>`.
- `run_preview(id, lines) -> String` — `tmux::capture` (grid tiles poll this).
- `attach_run(id, on_chunk: Channel<TerminalChunk>) -> ()` — open a tmux attach, stream
  base64 chunks (Focus). `detach_run(id)` drops the attach.
- `run_input(id, data: String) -> ()` — write to the attach (or `send_text`).
- `run_status(id) -> SessionStatus`.
- `discard_run(id) -> ()` — kill-session + remove worktree + delete run.
- `rerun(id) -> RunInfo` — start a fresh tmux session in the existing worktree with the
  stored agent+prompt.
- Existing `merge_task`/`abort_merge_task`/`resolve_merge` adapt to the run model
  (branch/repo from the run record instead of the live session).

### 3.5 Frontend (replace shell; reuse engine-facing pieces)
- **Replace:** `App` (→ shell: TitleBar + Body + StatusBar), `ProjectSidebar` (→ `ProjectTree`),
  `TaskBoard` (→ `AgentsView` = Grid + Focus).
- **New:** `TitleBar`, `StatusBar`, `ProjectTree`, `AgentsGrid` (+ `AgentTile`), `AgentFocus`
  (+ `AgentRail`), `NewTaskForm` (functional; modal polish is 7c), a small run store
  (React context) holding `runs` + selection + view state.
- **Reused:** terminal/attach logic (refactored from `TerminalPane` into a `FocusTerminal`
  bound to `attach_run`/`run_input`), `GitPanel` (under the Source Control nav),
  `Settings`, `MergeModal`.
- **Grid tile preview**: poll `run_preview(id)` (e.g. every ~1.5s) and render the captured
  text (ANSI stripped or lightly parsed) faded at the bottom — not a live xterm per tile.
- **Focus terminal**: on open, `attach_run` streams live into an xterm seeded by an initial
  `capture` (scrollback); input → `run_input`; on close, `detach_run` + dispose xterm.

## 4. Data flow (create → watch → review → integrate)

1. **New task** → `create_run` → DB row + worktree + tmux session started → tile appears
   (status Running).
2. **Grid** polls `list_runs` (status + diff stat) and `run_preview` per tile.
3. **Focus** → `attach_run` live stream; type into the agent; `Approve→` opens the existing
   Merge modal.
4. **Quit & relaunch** → `list_runs` shows persisted runs; tmux `has-session` true → status
   Running (reattachable); false → Exited; worktree diffs still reviewable.
5. **Discard** → `discard_run` (kill session + worktree + record), behind a confirm.

## 5. Error handling
- tmux missing/not on PATH → `create_run` returns a clear error; the UI surfaces "tmux is
  required — install it (brew install tmux)". (Detected via `tmux -V`.)
- A session that died with non-zero exit → tile shows Exited(code) in red; Focus shows the
  captured final output (remain-on-exit).
- Attach failures / detach races → surfaced in the Focus pane, never panic.
- Destructive ops (discard, kill-session, worktree remove) require confirm.

## 6. Testing
- **tmux module** (`agency-core`): integration tests that start a session running a trivial
  command (`sh -c 'echo HI; sleep 0.2'`), assert `session_exists`, `capture` contains HI,
  `session_status` transitions Running→Exited(0) (with remain-on-exit), `send_text` reaches
  the pane, `kill_session` removes it. Gated on `tmux` being present (skip-with-note if not).
- **runs persistence** (`registry`): insert/list/get/delete across reopen.
- **diff_stat** (`git`): crafted worktree → known added/deleted/files.
- **commands/state**: `create_run` persists + starts a session + returns RunInfo;
  `list_runs` derives status + diff stat; `discard_run` cleans up.
- **frontend**: `b64`/pure helpers via vitest; component build under TS-strict; the live
  grid/focus interaction is a human visual check against `Agency v2.dc.html`.

## 7. Key decisions (resolved in brainstorming)
| Decision | Choice |
|---|---|
| Phase 7 sequencing | 7a (shell+grid+add-agent) first; 7b source-control; 7c modals/shortcuts |
| Run persistence | Persist runs in SQLite |
| Concurrency | Multiple agents per project, concurrent (grid) |
| Terminal renderer | xterm.js (unchanged — industry standard) |
| Session backing | **tmux** sessions per run (durable, reattachable, external-attach) |
| Grid preview | `tmux capture-pane` snapshots (not live xterm-per-tile) |
| Focus terminal | live `tmux attach` in a portable-pty → xterm |
| tmux distribution | **Bundled inside the app** (Tauri `externalBin` sidecar); module takes a configurable binary path; dev uses system tmux; binary bundling/signing is a packaging-phase task → self-contained shipped app |

## 8. Migration / risk notes
- Removing the direct-PTY `supervisor` touches `start_task`, `resolve_merge`, and their
  tests (Phases 1/4/5). These are re-pointed at the tmux module; the `fake_agent` PTY tests
  are replaced by tmux-session tests. This is the riskiest part and is sequenced first in
  the plan.
- The frontend shell is a near-total replacement of `App`/`ProjectSidebar`/`TaskBoard`;
  `GitPanel`/`Settings`/`MergeModal` are remounted under the new shell with minimal change.
- tmux output is ANSI; grid previews need light ANSI handling (strip or a tiny parser) so
  they read cleanly as text.
