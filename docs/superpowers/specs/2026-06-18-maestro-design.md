# Maestro — Local Agent Orchestrator (Design)

**Date:** 2026-06-18
**Status:** Approved design, pre-implementation
**Working name:** Maestro (rename anytime)

## 1. Summary

Maestro is a **local-only, native desktop app** for running multiple terminal-based
coding agents in parallel, each isolated in its own git worktree, with a GUI to watch
them live, review their changes, and integrate the good ones back to `main`.

It is a self-hosted alternative to Conductor. The defining constraint: **nothing leaves
the machine** except traffic the user explicitly configures an agent to make (e.g. the
Anthropic API, or nothing at all when using local models via LM Studio). No account, no
vendor backend, no telemetry/analytics.

## 2. Goals / Non-goals

### Goals (v1)
- Native desktop app (Tauri + Rust core + web UI).
- Multi-project dashboard: manage several repos at once.
- Pluggable **terminal-agent** backends; ship profiles for **Claude Code, Pi, Hermes**.
- Model providers: **Anthropic (Claude)** and **local models via LM Studio**
  (OpenAI-compatible endpoint).
- Live PTY terminal pane per agent — watch and interject.
- Full per-worktree **git panel**: history, file viewing, stage/unstage (file + hunk),
  commit, **push**, and approve→merge to `main`.
- **Merge/conflict-resolver** authored as a portable skill, run by a **configurable
  backend** (default Claude Code, optional local model).

### Non-goals (v1, deferred)
- Codex / Cursor / other agent providers (the adapter trait keeps this a config change).
- Cloud or remote agent execution.
- Live PTY reattach across app restarts (restart marks surviving tasks "Exited, re-run").
- Structured event sidecar (e.g. Claude Code `stream-json`) for richer status/token UI —
  v1.1 enhancement on top of the PTY substrate.

## 3. Architecture

A **Tauri** application:
- **Rust core** owns all state, process management, and git operations.
- **Web UI** (React + xterm.js) is the renderer.

Outbound network = only what a configured agent calls. A test asserts no other network
activity occurs.

### Rust core modules
Each is a focused, independently testable unit.

1. **Project registry** — repos under management; each = repo path + defaults (preferred
   agent, model provider). Persisted in local SQLite in the app data dir.
2. **Worktree manager** — wraps `git worktree add/list/remove`. Each task →
   `…/.maestro/worktrees/<task-id>` on branch `agent/<task-id>` off a chosen base
   (default `main`). Handles cleanup on task delete.
3. **Agent supervisor** — spawns each agent in a real PTY (`portable-pty`) with
   `cwd` = its worktree and env = chosen model provider (e.g. `ANTHROPIC_API_KEY`, or an
   OpenAI-compatible base URL like `http://localhost:1234/v1` for LM Studio). Streams PTY
   bytes to the UI via a Tauri `Channel`, feeds user keystrokes back in, and tracks status
   heuristically.
4. **Agent profiles (adapter trait)** — declarative launch config per agent type:
   command, arg template, env. v1 ships Claude Code, Pi, Hermes. Trait makes adding more
   a config change.
5. **Git panel / diff service** — per-worktree source control: branch commit log and
   divergence from `main`; file tree + per-file diff (working tree and committed);
   stage/unstage by file and hunk; commit; push to the repo's remote; plus the
   approve→merge action.
6. **Merge-resolver** — runs the authored `merge-resolver` skill via a configurable
   backend (default Claude Code, optional local model). Checks out `main`, merges/rebases
   the agent branch, resolves conflicts, commits, writes a plain-language summary. Reuses
   the supervisor + PTY plumbing (it is a special agent run).
7. **Persistence** — SQLite for projects/tasks/run history; app settings in a TOML file.

### Web UI surfaces
- **Project sidebar** — the multi-project dashboard.
- **Task board** per project — cards = agent runs with live status dots.
- **Live terminal pane** (xterm.js) bound to the selected task's PTY.
- **Git panel** — file tree + diff viewer + stage/commit/push + approve/discard/feedback.
- **Settings** — agent profiles, model providers (Anthropic key, LM Studio base URL).

## 4. Task lifecycle (data flow)

1. **Create** — pick project, write task prompt, choose agent profile + model provider.
2. **Isolate** — worktree manager branches `agent/<task-id>` off the base into a fresh
   worktree.
3. **Run** — supervisor spawns the agent in a PTY there, injecting prompt + provider env.
   Output streams to the task's terminal pane; status → *Running*.
4. **Work & watch** — agent edits and auto-commits. User can interject in the live
   terminal or use "send feedback" (typed back into the PTY). Git panel reflects
   commits/working-tree changes/diffs in real time.
5. **Review** — read diffs; optionally stage/commit a manual tweak; optionally push the
   branch.
6. **Integrate** — *Approve* → merge-resolver runs the skill via configured backend:
   checks out `main`, merges/rebases, resolves conflicts, commits, writes a summary. On
   success, offers to remove the worktree.
7. **Discard** — deletes worktree + branch, always behind a confirm (never auto-delete
   unreviewed work).

## 5. Status & error handling

- **Status heuristics** (PTY): combine process exit code + shell-prompt-return regex +
  idle timeout into states: *Running, Awaiting input, Idle/Done, Exited(code), Crashed*.
  Ambiguous → "Idle — review needed"; never a false "done."
- **Provider/auth errors**: surface agent stderr in the pane; detect common cases (missing
  `ANTHROPIC_API_KEY`, LM Studio unreachable at base URL) with actionable hints.
- **Git/merge errors**: resolver returns unresolved conflicts rather than forcing a bad
  merge; dirty tree / detached HEAD surfaced clearly. Destructive ops (discard, worktree
  remove, force-push) require explicit confirm.
- **Crash recovery**: projects/tasks/worktrees persisted in SQLite; restart re-attaches to
  state. v1 caveat: PTY processes don't survive app exit — surviving tasks show as
  *Exited, re-run to resume*.

## 6. Testing strategy

- **Rust unit tests** against temp git repos: worktree manager, git-panel/diff service
  (crafted commits + conflicts), agent-profile command rendering.
- **Supervisor tests** use a **fake agent** — a tiny scripted CLI that prints/exits — to
  validate PTY streaming, status detection, and input injection without a real model.
- **Merge-resolver tests**: stub backend running scripted merges + integration tests on
  hand-built conflict repos.
- **UI**: component tests for terminal pane + diff view; a few end-to-end happy-path runs
  (create → fake agent → diff → merge) via Tauri's test harness.
- **Local-only assertion**: test that asserts no network calls occur except to configured
  provider endpoints.

## 7. Key decisions (resolved during brainstorming)

| Decision | Choice |
|---|---|
| Delivery shape | Native desktop app |
| Framework | Tauri + Rust core, web UI |
| Multi-project dashboard | Yes (v1) |
| Agent backends | Pluggable; v1 = Claude Code, Pi, Hermes (terminal agents) |
| Model providers | Anthropic (Claude) + local via LM Studio |
| Exec model | PTY terminal panes (universal, interactive) |
| Git surface | Full per-worktree git panel incl. push |
| Merge-resolver | Authored skill + configurable backend (default Claude Code) |
