# Agency — Local Agent Orchestrator (Design)

**Date:** 2026-06-18
**Status:** Approved design, pre-implementation
**Name:** Agency

## 1. Summary

Agency is a **local-only, native desktop app** for running multiple terminal-based
coding agents in parallel, each isolated in its own git worktree, with a GUI to watch
them live, review their changes, and integrate the good ones back to `main`.

It is a self-hosted alternative to the hosted agent runners. The defining stance: **no analytics, no
telemetry, no data collection, no account, no vendor backend** — Agency itself never phones
home. It still talks to the external services the user explicitly configures and initiates:
model providers (the Anthropic API; or nothing remote at all with local models via LM
Studio) and git remotes such as GitHub (push, PRs). The distinction is **zero first-party
data collection**, not zero network.

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
- Remote access / mobile control (a local web server + tunnel, as some adjacent
  tools offer). Appealing for later — explicitly out of v1.
- Structured event sidecar (e.g. Claude Code `stream-json`) for richer status/token UI —
  v1.1 enhancement on top of the PTY substrate.

## 3. Architecture

A **Tauri** application:
- **Rust core** owns all state, process management, and git operations.
- **Web UI** (React + xterm.js) is the renderer.

Agency's own code performs no telemetry, analytics, or background network calls. Network
activity is limited to operations the user explicitly initiates against services they
configured — agent/provider calls and git-remote operations (e.g. push to GitHub). A test
asserts the core engine makes no network calls of its own.

### Rust core modules
Each is a focused, independently testable unit.

1. **Project registry** — repos under management; each = repo path + defaults (preferred
   agent, model provider). Persisted in local SQLite in the app data dir.
2. **Worktree manager** — wraps `git worktree add/list/remove`. Each task →
   `…/.agency/worktrees/<task-id>` on branch `agent/<task-id>` off a chosen base
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
- **No-telemetry assertion**: test that the core engine performs no network calls of its own
  (no analytics/telemetry/background egress). Legitimate provider and git-remote calls are
  user-initiated and out of the core's scope.

## 7. Key decisions (resolved during brainstorming)

| Decision | Choice |
|---|---|
| Name | Agency |
| Delivery shape | Native desktop app |
| Framework | Tauri + Rust core, web UI |
| Multi-project dashboard | Yes (v1) |
| Agent backends | Pluggable; v1 = Claude Code, Pi, Hermes (terminal agents) |
| Model providers | Anthropic (Claude) + local via LM Studio |
| Exec model | PTY terminal panes (universal, interactive) |
| Git surface | Full per-worktree git panel incl. push |
| Merge-resolver | Authored skill + configurable backend (default Claude Code) |

## 8. Positioning

The category is not new, and this document originally carried a detailed reading
of the closest existing product: what it shared with Agency, where the two
diverged, and a dated assessment of its privacy posture. That analysis is kept
privately and deliberately out of the public record — it was a point-in-time
reading recorded to sharpen our own position, projects change, and a stale
critique of somebody else's product is not something to publish under our name.

What survives is the part that stands on its own, because it is stated in
Agency's own terms rather than by comparison:

**Zero first-party data collection** — no analytics, no telemetry, no account,
no vendor backend. Agency makes exactly one call the user did not initiate: an
update check against GitHub's public releases API, which sends nothing beyond
the request, downloads and installs nothing, and can be switched off in
Settings. Everything else is a connection to a service the user configured and
started themselves: model providers, git remotes. This is *not* a claim that no
bytes ever leave the machine — the agents are third party and talk to their own
providers. Combined with the Rust-native core and local-models-as-peer, that is
the position.

The roadmap implication also survives the anonymization: power-user features
(group chat and moderator routing, message queueing, output filtering, a
headless CLI, remote and mobile access via a tunnel) are the bar to measure
against, and belong on the deferred list rather than in v1. A future
remote-tunnel feature is compatible with the stance above, being user-initiated
access to the user's own machine rather than first-party collection.
