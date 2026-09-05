# Windows compatibility notes

Agency is currently macOS-first. This file catalogs every place that assumes a
Unix/macOS environment so a future Windows port knows exactly what to touch.
Keep it updated when adding new shell-outs or install flows.

## Install one-liners (highest impact)

All "this CLI isn't installed" flows open an in-app terminal that runs a
Unix-shaped command. Each needs a per-platform variant (a
`INSTALL_COMMANDS[platform][agent]` table, or detection at runtime):

| Site | Today (macOS) | Windows equivalent |
| --- | --- | --- |
| `ui/src/components/GhSetupHint.tsx` | `brew install gh` | `winget install --id GitHub.cli` |
| `ui/src/agents.ts` `INSTALL_COMMANDS.cursor` | `curl https://cursor.com/install -fsS \| bash` | Cursor ships a Windows installer; no curl-pipe |
| `ui/src/agents.ts` `INSTALL_COMMANDS.kimi` | `curl -fsSL https://code.kimi.com/kimi-code/install.sh \| bash` | PowerShell: `irm https://code.kimi.com/kimi-code/install.ps1 \| iex` |
| `ui/src/agents.ts` npm-based entries (claude/codex/pi/opencode/copilot/gemini/crush) | `npm install -g …` | Same command works, but runs under a different shell (see below) |
| graphify (docs + MCP default) | `uv tool install --force "graphifyy[mcp,openai,anthropic]"` / `uv tool run …` | Same via uv's Windows build; the extras need quoting in PowerShell too, and verify `python -m graphify.serve` path resolution |

`gh auth login` itself is cross-platform once gh is installed.

## Shell assumptions

Everything Agency spawns goes through a Unix login shell today:

- `crates/agency-core/src/scripts.rs` — setup/run/archive scripts run as
  `sh -lc <script>`. Windows needs `cmd /C` or (better) PowerShell, and the
  `-l` login-shell semantics (PATH from user profile) have no direct analog.
- `crates/agency-app/src/state.rs` — multiple `$SHELL` fallbacks to
  `/bin/zsh` / `/bin/bash` (terminal creation, install terminals, shell
  profile seeding at `state.rs:342`) and `sh -lc` for the run script and the
  knowledge-graph rebuild. On Windows: `%COMSPEC%` / PowerShell, no `-l`.
- The default knowledge-graph build command now carries a `VAR=value` prefix
  (`GRAPHIFY_CLAUDE_CLI_MODEL=haiku graphify . --backend claude-cli`), which is
  a POSIX shell assignment and not a PowerShell one. `config.rs` composes it,
  `command_binary` already reads past it for the PATH check, and a Windows port
  needs `$env:VAR = …` or an env entry on the spawned command instead.
- `state.rs start_knowledge_build` puts the graph build in its own process
  group (`process_group(0)`) and stops it with `/bin/kill -TERM -<pgid>`, so
  Stop reaches the agent CLI graphify spawns per document and not just the
  process Agency started. Windows has no process groups in that sense: a job
  object (or `taskkill /T /PID`) is the equivalent, and `Child::kill` is the
  fallback the code already takes when the group signal fails.
- `create_install_terminal` composes `"<command>\nexec \"$SHELL\" -l"` —
  the "run installer, then drop into an interactive shell" trick needs a
  PowerShell equivalent.

## Other platform-specific pieces

- **agency-termd (PTY daemon)** — built on Unix PTYs and a Unix domain
  socket (`crates/agency-core/src/term/`). Windows needs ConPTY and named
  pipes; this is the single largest port item.
- **`pathenv.rs`** — repairs the stripped Finder-launch environment
  (Homebrew paths, TERM, LANG). Windows GUI launches have a different (and
  smaller) version of this problem.
- **Path handling** — worktree paths, `.git/info/exclude` writing, and
  `[files] copy` all use std paths and should mostly work, but anything
  formatting paths into shell strings (e.g. the graphify MCP serve command in
  `state.rs merged_mcp_servers`) must quote for spaces/backslashes.
- **Notifications / tray / titlebar overlay** — Tauri abstracts most of it,
  but `notif_macos.rs` is macOS-only twice over: it gets a banner shown while
  Agency itself is frontmost (Windows toasts already do that), and it patches
  the notification delegate to hear clicks, which is how `state.rs` knows to
  open the run a notification was about. A port needs its own answer to "the
  user clicked this notification"; without one, only the fallback survives —
  a return from the background opens the run notified while the app was away.
  `foreground.rs` is the other half of that: it answers whether Agency is
  frontmost, which the webview's focus bit only approximates.
- **`command_on_path`** (`state.rs`) — checks executability Unix-style;
  Windows needs PATHEXT-aware lookup (`gh` → `gh.exe`).

## Suggested approach when the port happens

1. Introduce a small `platform` module (core) that answers: default shell +
   "run this script line" argv, install command per known CLI, and
   executable-on-PATH lookup. Route all sites above through it.
2. Port agency-termd to ConPTY behind the same daemon protocol.
3. Keep install commands data-driven per platform rather than branching at
   call sites.
