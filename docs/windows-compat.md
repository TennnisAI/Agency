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
| `ui/src/agents.ts` npm-based entries (claude/codex/pi/opencode/copilot) | `npm install -g …` | Same command works, but runs under a different shell (see below) |
| graphify (docs + MCP default) | `uv tool install graphifyy` / `uv tool run …` | Same via uv's Windows build; verify `python -m graphify.serve` path resolution |

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
  but the notification click-deeplink workaround in `state.rs` is tuned to
  macOS notification-center behavior.
- **`command_on_path`** (`state.rs`) — checks executability Unix-style;
  Windows needs PATHEXT-aware lookup (`gh` → `gh.exe`).

## Suggested approach when the port happens

1. Introduce a small `platform` module (core) that answers: default shell +
   "run this script line" argv, install command per known CLI, and
   executable-on-PATH lookup. Route all sites above through it.
2. Port agency-termd to ConPTY behind the same daemon protocol.
3. Keep install commands data-driven per platform rather than branching at
   call sites.
