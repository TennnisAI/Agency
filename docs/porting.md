# Porting notes: Linux and Windows

Agency ships for macOS and Linux. This file catalogs every place that assumes
a macOS environment so a port knows exactly what to touch, and records how the
Linux port went. Keep it updated when
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

**agency-app compiles on Linux.** Found 2026-09-08 in a Debian container and
confirmed the same day by the `linux-check` job on an x86_64 CI runner (PR #1),
which now blocks like any other job. It needed **no source changes at all**,
and exactly one dependency change.

That change is worth reading, because it is why this went untested for so long.
`agency-app` depended on `rfd` directly with default features, which turn on
`xdg-portal`, while `tauri-plugin-dialog`'s defaults turn on `gtk3`. Cargo
unifies the two and rfd's build script aborts: *"You can't enable both `gtk3`
and `xdg-portal` features at once."* It fires partway through the dependency
tree, long before any Agency code is reached, so a first attempt reads as
"Linux is broken" rather than "one feature flag collides". The only rfd use
here is a single synchronous `MessageDialog` in `lib.rs` (the setup-failure
fallback), so `default-features = false` fixes it and costs nothing. macOS
still compiles, and the resolve graph loses eleven crates it never needed
(`ashpd`, `pollster`, the wayland stack).

So Linux is within reach of packaging, and `linux-check` in
`.github/workflows/ci.yml` keeps it that way.

What Linux would still need after that:

- **Packaging: done.** `scripts/release-linux.sh` builds `.deb`, `.rpm` and
  AppImage. The existing `externalBin` and icon entries were already
  platform-neutral, and both binaries land in the package (`/usr/bin/Agency`
  and `/usr/bin/agency-termd`). The script has no signing, notarization or
  stapling steps because Linux has no equivalent, which is why it is half the
  length of the macOS one. On macOS it builds in a container, since Tauri
  links against the host's webkit2gtk and cannot cross-compile.

  For both architectures, build on native runners:
  `.github/workflows/linux-packages.yml` runs the same script on its
  native-Linux path across an x86_64 and an aarch64 leg. Locally, `--arch both`
  works but x86_64 on an Apple Silicon Mac is qemu, which is slow enough to be
  impractical for a release.

  **The first package looked untrustworthy, and that was mostly us.** Opened
  in Ubuntu 24.04's App Center (2026-09-08) it showed "agency", "Unknown
  publisher", a placeholder icon, "License unknown", the one-line description
  "Tauri app shell for Agency" and a long description of "(none)". The control
  file said `Maintainer: agency` (the bundler's last-resort fallback, the first
  word of the identifier), had no `Homepage` or `Section`, and the desktop file
  had an empty `Categories=`. lintian counted five errors: malformed-contact,
  missing-dependency-on-libc, no-copyright-file, and unstripped binaries
  twice. Fixed at the source: `authors` and `homepage` in
  `crates/agency-app/Cargo.toml`, and `homepage`, `license`, `copyright`,
  `category`, `shortDescription`, `longDescription`, `linux.deb.section` and
  the `files` maps in `tauri.conf.json`. The `files` maps install an AppStream
  metainfo (`crates/agency-app/linux/`), the Debian copyright file, and
  `LICENSE`, `NOTICE` and `THIRD-PARTY.md` under `/usr/share/doc/agency/`, so
  the notices travel with the Linux binaries the way the DMG carries them.
  `tests/linux_packaging.rs` pins the metainfo to the config, and the
  script's verification step runs desktop-file-validate, appstreamcli and
  lintian on the built .deb.

  What that does *not* fix is App Center's page for a local .deb, and it is
  worth knowing where the line is before spending more time on it. That page
  hard-codes the title to the package name, passes no publisher (so "Unknown
  publisher") and no icon (so the placeholder), shows "License unknown"
  because PackageKit's apt backend has no licence field to read from a .deb,
  and shows the "Potentially unsafe / third party" warnings for every local
  package regardless of contents. The only fields it takes from the package
  are the summary, the long description, the homepage link and the size. The
  metainfo and the icon take effect after install, on the installed-apps page
  and in the launcher.

  **App Center's install itself hung at "Installing" indefinitely** on the
  same machine, and that is not the package either: the identical file
  installs with `sudo apt install ./Agency_0.1.1_amd64.deb` on Ubuntu 24.04
  (verified in a container, both architectures, 2026-09-10), and the installed
  app starts, launches its daemon and creates its data directory under a
  headless X server. Tell Linux users to install with apt, or to run the
  AppImage, and do not route them through App Center.

  **The build image sets a glibc floor, and it is the whole ballgame for
  distribution.** A Linux binary runs on any glibc at least as new as the one it
  linked against and on none older. Observed: a build on `ubuntu-latest`
  (24.04, glibc 2.39) installed on Debian 12 and refused to start with
  ``version `GLIBC_2.39' not found``. Pinned to 22.04 the binary needs only
  GLIBC_2.34, and it installs and starts on Debian 12. The .deb now declares
  the floor (`libc6 (>= 2.34)` in `linux.deb.depends`), so a too-old system
  refuses the package instead of installing it and dying at launch, and the
  release script fails the build if the declared floor is lower than what the
  binaries actually import. The rpm cannot carry a version there: the bundler
  hands each `depends` entry to the package as a bare name. AppImages do not
  help either, since they bundle libraries but never libc. Raise the runner
  only when dropping those distros is a decision someone has made.

  **Arch has no Tauri target.** The bundler offers `deb`, `rpm` and `appimage`
  only, so Arch is served by the AppImage unless someone maintains a PKGBUILD
  on the AUR. `packaging/aur/agency-bin/` is that PKGBUILD, and it repackages
  the release .deb rather than building from source. Tested 2026-09-11 on Arch
  Linux ARM in a container: `makepkg -si` validated the checksum, pulled
  WebKitGTK and the rest from the Arch repositories and installed, and the app
  started its daemon. The `.SRCINFO` that `packaging/aur/update.sh` generates on
  a Mac matched `makepkg --printsrcinfo` line for line. Not published yet: the
  AUR needs the maintainer's own account (AGE-227).
- **Release: done.** `release.yml` calls `linux-packages.yml` on a version tag
  and attaches all six packages to the same draft as the DMG. The README
  carries the install steps for Debian and Ubuntu, Fedora and Arch; the
  download page does not link the Linux packages yet (that is the site's own
  change, kept separate from this one).
  Installed and launched in containers on 2026-09-11 from the arm64 CI build:
  the .deb on Debian 12 and Ubuntu 22.04, the .rpm on Fedora 44. Each resolved
  its libraries from the distribution, started `agency-termd` and created its
  data directory. The arm64 AppImage ran on Arch Linux ARM, started with
  `APPIMAGE_EXTRACT_AND_RUN=1` since a container has no FUSE. The x86_64 .rpm and AppImage have not been run
  anywhere: the official Arch image is x86_64 only, and the colima VM these
  tests ran in cannot emulate x86_64.
- **The tools Agency itself needs: done.** Observed 2026-09-10 on a fresh
  Ubuntu desktop: every agent's install line began with `npm` on a machine
  with no npm, and "Initialize repository" failed with `No such file or
  directory (os error 2)` because there was no git. `crates/agency-app/src/tools.rs`
  is the catalog (git, Node.js and npm, gh) with a per-platform install
  recipe: Homebrew or `xcode-select --install` on macOS; apt, dnf, pacman or
  zypper under `pkexec` on Linux, so the desktop's own polkit prompt asks
  for the password and no terminal is needed; winget on Windows, written
  but not yet exercised. Onboarding grows a first step when any are
  missing, and an error that says "git is not installed" opens the same
  install row instead of a toast (`ui/src/lib/missingTool.ts`). Without
  polkit or Homebrew the row shows a line to copy and the download page.
- **Installs run in the background, not in a terminal.**
  `crates/agency-app/src/installer.rs` runs an install line under the login
  shell with its output captured, and the onboarding tile ticks itself when
  the command lands on PATH. An npm global on a machine whose node came from
  a distribution package (or the nodejs.org .pkg) would die with EACCES on
  the root-owned prefix; `tools::install_script` falls back to
  `--prefix ~/.local` only in that case, and `pathenv::adopt_new_dirs` picks
  the new `~/.local/bin` up without restarting the app.
- **Window chrome: done.** `titleBarStyle: Overlay` is macOS-only, so Linux
  showed the window manager's title bar, then the GTK menu strip, then
  Agency's own 40px title row: three bars before any content. The Linux
  window is undecorated (`tauri.linux.conf.json`), the native menu is not
  built there (`lib.rs`), and the frontend's title bar carries the menus
  (`ui/src/lib/appMenu.ts`, mirrored against `menu.rs` by a test) and the
  minimize, maximize and close buttons. tao already edge-resizes an
  undecorated GTK window (5px hit-test border) and Tauri's drag region
  maximizes on double-click, so nothing else was needed. Accepted: the
  window has square corners and no compositor shadow, as any undecorated
  X11 window does. Shortcut labels go through `ui/src/lib/platform.ts`,
  which spells ⌘ as Ctrl off macOS.
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

The tools Agency itself needs (git, npm, gh) are handled per platform in
`tools.rs`, see above. The agent CLIs' own install lines are still the one
table in `ui/src/agents.ts`, run either in an in-app terminal (from a
project) or in the background (from onboarding). Each still needs a
per-platform variant for Windows:

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
