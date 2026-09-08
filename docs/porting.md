# Porting notes: Linux and Windows

Agency ships macOS-only. This file catalogs every place that assumes a
macOS environment so a port knows exactly what to touch. Keep it updated when
adding new shell-outs or install flows.

Renamed from `windows-compat.md` on 2026-09-08. The old name encoded an
assumption worth correcting: that leaving macOS meant one port. It is two, and
they are wildly different sizes.

## Linux is nearly there, and nobody has noticed

`cargo test -p agency-core --locked` has been running on `ubuntu-latest` on
every PR (`.github/workflows/ci.yml`, the `core` job). So the whole of
`agency-core` already compiles and passes its tests on Linux, continuously:
the PTY daemon, the Unix-domain-socket protocol, git, worktrees, merge,
`issuefs`, `mcp`, the preview server, loops. The "agency-termd is the single
largest port item" line below is **true for Windows only**. On Linux that item
is already done and verified on every push.

The frontend is in the same position: the `ui` job runs typecheck, unit tests
and a production `vite build` on `ubuntu-latest`.

That leaves `agency-app`, and it was written portably without anyone setting
out to:

- Every macOS-specific module is already behind `cfg`, with the non-macOS arm
  already written: `foreground.rs`, `clipboard.rs` and `preview_shot.rs` each
  have a `cfg(not(target_os = "macos"))` fallback, and `notif_macos` is a
  macOS-only module (`lib.rs:13`) whose call sites sit inside a macOS-only
  block (`lib.rs:395`).
- The objc2 stack is already scoped under
  `[target.'cfg(target_os = "macos")'.dependencies]` in
  `crates/agency-app/Cargo.toml`.
- `tray.rs`'s only macOS-specific line is `set_title`, the count badge beside
  the menu-bar icon.

**So agency-app may already compile on Linux. Nobody knows, because no CI job
would say.** The cheapest high-information step in this whole document is a
`cargo check -p agency-app` on `ubuntu-latest`: it either goes green, which
puts Linux within reach of packaging alone, or it prints the exact remaining
list. Do that before estimating anything else.

What Linux would still need after that:

- **Packaging.** `externalBin` in `tauri.conf.json` points at
  `../../target/release/agency-termd` and the bundle block only configures
  `macOS`. Linux needs its sidecar triple and AppImage/deb targets. It needs no
  signing or notarization, so `scripts/release-macos.sh` has no equivalent to
  port, only a simpler sibling to write.
- **The install-command table** below, which is per-platform work Linux shares
  with Windows. The `npm install -g` rows already work as-is.
- **Two accepted degradations.** Tauri's Linux tray wants
  libayatana-appindicator and has no `set_title`, so the count badge becomes
  tooltip-only; and there is no delegate to patch for notification clicks, so
  only the return-from-background fallback described under "Notifications"
  survives. Both are worse than macOS, neither blocks a release.

Everything below this line is the Windows catalogue, and Windows is the large
one: ConPTY and named pipes in place of Unix PTYs and Unix domain sockets, no
`sh -lc`, no `-l` login semantics, no process groups, PATHEXT lookup.
`std::os::unix` appears in twelve files. Build the `platform` module in
"Suggested approach" for the Linux work anyway; it is where Windows plugs in
later, so that part is not wasted.

---

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
