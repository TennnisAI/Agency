<img src="app-icon/agency-icon-128.png" alt="" width="96" align="left" hspace="4" vspace="4">

# Agency

**A desktop app for running coding agents in parallel, each isolated in its own
git worktree.** Watch them live, review what they changed, merge the good ones.

<br clear="left">

[![CI](https://github.com/TennnisAI/Agency/actions/workflows/ci.yml/badge.svg)](https://github.com/TennnisAI/Agency/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-macOS%20(Apple%20Silicon)-lightgrey.svg)](#requirements)

<!-- SCREENSHOT
     Add the hero capture here once one exists from a demo repo. The images in
     site/assets/img/ are from the real working machine: they show fifteen real
     project names, a home-directory path in agent output, and the machine
     hostname in a terminal prompt. Recapture before using any of them here.

     ![Agency](docs/img/hero.webp)
-->

Built with [Tauri 2](https://tauri.app): a Rust backend (`crates/`) and a React
+ Vite frontend (`ui/`).

## What it does

- **Runs many agents at once**, each in its own `git worktree`, so they never
  collide over a working tree or a branch.
- **Terminal-first.** Every agent is a real PTY you can watch, interrupt, and
  type into. Nothing is hidden behind a chat box.
- **Bring your own agent.** Claude Code, Codex, Cursor, Copilot CLI, OpenCode,
  Gemini CLI, Kimi Code, Crush, Hermes, Pi, DeepSeek Harness. Agency detects
  what is on your `PATH`.
- **Review and merge in the app.** Per-worktree diff review, conflict
  resolution, and merge back to your base branch.
- **A local issue tracker**, plain markdown under `.agency/issues/`, that agents
  can be dispatched against and can file follow-ups into.
- **Loops.** Run one agent to convergence with hard attempt, wall-clock and
  token caps, rather than racing many.

## Privacy

Agency does no first-party data collection: no analytics, no telemetry, no
account, no vendor backend. There is no server of ours for it to talk to.
Outbound traffic is only what you initiate, and the coding agents are third
party and talk to their own providers.

The one exception is the update check: on launch Agency asks GitHub's public API
for the latest release tag and compares it to the running version. It sends no
identifiers, and it downloads and installs nothing. If a newer version exists,
Agency dots the Settings button and offers a link; you install the DMG yourself,
whenever suits you. Turn it off in Settings ▸ Diagnostics ("Check for updates on
launch") and Agency makes no network calls of its own at all.

## Installing (beta)

1. Download the latest `Agency_*.dmg` from
   [Releases](https://github.com/TennnisAI/Agency/releases).
2. Open the DMG and drag **Agency** to Applications.
3. Launch it. The build is signed and notarized, so it opens without a Gatekeeper
   prompt. If macOS says the app is damaged, you have an unsigned local build
   rather than a release DMG.

On first launch, Agency asks you to pick a default agent, then you add a project
by pointing it at a local git repo.

### Requirements

- **macOS 11 (Big Sur) or later on Apple Silicon.** Release builds are `aarch64`
  only; there is no Intel or universal build yet.
- **Git**, and Xcode Command Line Tools (`xcode-select --install`).
- **At least one coding-agent CLI** from the list above. Agency offers to install
  a missing one for you; most install via `npm install -g`, so those need
  **Node**.
- **[gh](https://cli.github.com)** (optional) - needed only for the GitHub import
  and PR flows.

### Reporting problems

Settings ▸ Diagnostics has your exact version, an "Open logs" button, and a link
to file an issue. Attaching the log makes a bug report far easier to act on.

## Building from source

**Prerequisites**

- **Rust** (stable; built with 1.96). Install via [rustup](https://rustup.rs).
- **cargo-tauri** - `cargo install tauri-cli`.
- **pnpm** 11+ and **Node** 22+. Use `pnpm`, not `npm`; the lockfile is pnpm's.
- **Xcode Command Line Tools** - `xcode-select --install`.

```sh
pnpm --dir ui install   # once
./dev.sh                # builds the agency-termd sidecar, then launches the app
```

**Use `./dev.sh`, not `cargo tauri dev`.** Tauri's `beforeDevCommand` starts Vite
only. It does not build the `agency-termd` daemon, and the app fails at startup
without one. A dev build keeps its own data directory and daemon socket, so it
never disturbs an installed Agency.app.

Cutting a signed release is in [`docs/releasing.md`](docs/releasing.md).

## Repository layout

| Path | What it is |
| --- | --- |
| `crates/agency-core` | Core library and the `agency-termd` terminal daemon |
| `crates/agency-app` | The Tauri app shell (Rust backend, `tauri.conf.json`) |
| `ui/` | React + Vite frontend (pnpm) |
| `site/` | The getagency.dev marketing site; static, no build step |
| `scripts/` | Release tooling |
| `docs/` | Design notes and plans |

## Project status

Agency is in beta, built and maintained by one person as a working tool. Issues
are welcome and read; responses are best-effort. PRs are considered
case-by-case: see [CONTRIBUTING.md](CONTRIBUTING.md) before starting anything
substantial, and [SECURITY.md](SECURITY.md) for anything sensitive.

Nearly every feature here started as a written design and a task-by-task plan,
and those documents stayed in the repo. [`docs/`](docs/) is that record.

## License

Apache-2.0, see [LICENSE](LICENSE). The name "Agency" is not licensed; see
[NOTICE](NOTICE).
