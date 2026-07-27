# Agency

A desktop app for running and managing coding agents across git worktrees. Built
with [Tauri 2](https://tauri.app) — a Rust backend (`crates/`) and a React + Vite
frontend (`ui/`).

Agency does no first-party data collection: no analytics, no telemetry, no
account, no vendor backend. There is no server of ours for it to talk to.
Outbound traffic is only what you initiate — the coding agent you launch talking
to its own provider, and git talking to your remotes.

The one exception is the update check: on launch Agency asks GitHub's public API
for the latest release tag and compares it to the running version. It sends no
identifiers — just a plain request GitHub sees like any other — and it downloads
and installs nothing. If a newer version exists, Agency dots the Settings button
and offers a link; you install the DMG yourself, whenever suits you. Turn it off
in Settings ▸ Diagnostics ("Check for updates on launch") and Agency makes no
network calls of its own at all.

## Installing (beta)

1. Download the latest `Agency_*.dmg` from
   [Releases](https://github.com/nic123/Agency/releases).
2. Open the DMG and drag **Agency** to Applications.
3. Launch it. The build is signed and notarized, so it opens without a Gatekeeper
   prompt — if macOS says the app is damaged, you have an unsigned local build
   rather than a release DMG.

**Requirements**

- **macOS 11+ on Apple Silicon.** Release builds are `aarch64` only right now;
  there is no Intel or universal build yet.
- **Git**, and Xcode Command Line Tools (`xcode-select --install`).
- **At least one coding-agent CLI** — Claude Code, Codex, Pi, OpenCode, Copilot
  CLI, Cursor, Hermes, Gemini CLI, Kimi Code, or Crush. Agency detects what is on
  your `PATH` and offers to install a missing one for you. Most install via
  `npm install -g`, so **Node** is needed for those.
- **[gh](https://cli.github.com)** (optional) — needed only for the GitHub import
  and PR flows.

On first launch, Agency asks you to pick a default agent, then you add a project
by pointing it at a local git repo.

**Reporting problems.** Settings ▸ Diagnostics has your exact version, an "Open
logs" button, and a link to file an issue. Attaching the log makes a bug report
far easier to act on.

## Repository layout

| Path                 | What it is                                              |
| -------------------- | ------------------------------------------------------- |
| `crates/agency-core` | Core library + the `agency-termd` terminal daemon       |
| `crates/agency-app`  | The Tauri app shell (Rust backend, `tauri.conf.json`)   |
| `ui/`                | React + Vite frontend (pnpm)                            |
| `scripts/`           | Release tooling                                         |
| `docs/`              | Design notes and plans                                  |

## Prerequisites

- **Rust** (stable; built with 1.96). Install via [rustup](https://rustup.rs).
- **cargo-tauri** — `cargo install tauri-cli` (provides `cargo tauri …`).
- **pnpm** 11+ and **Node** 22+. Use `pnpm`, not `npm` (the lockfile is pnpm's).
- **Xcode Command Line Tools** — `xcode-select --install`.

Install frontend dependencies once:

```sh
pnpm --dir ui install
```

## Development

```sh
cargo tauri dev
```

This runs the Vite dev server (`pnpm dev` in `ui/`) and launches the app with
hot-reloading via the `beforeDevCommand` hook in `tauri.conf.json`.

## Building for macOS

### Unsigned local build

For a quick local bundle (won't run on other machines without Gatekeeper
warnings):

```sh
# Build the agency-termd sidecar first — the app bundles it as an externalBin.
./crates/agency-app/build-termd.sh

cargo tauri build
```

Output lands in `target/release/bundle/` (`.app` under `macos/`, `.dmg` under
`dmg/`).

### Signed + notarized release build

Produces a DMG that opens cleanly on other Macs (no "Agency is damaged"). This is
the build to hand to testers.

```sh
./scripts/release-macos.sh
```

The script runs the sidecar build, `cargo tauri build` (which deep-signs the whole
bundle — including the `agency-termd` sidecar — using the identity in
`tauri.conf.json`), then notarizes and staples the DMG. The first run takes a few
minutes while Apple processes the upload.

Signing config lives in `crates/agency-app/tauri.conf.json` under `bundle.macOS`
(signing identity, hardened runtime, `Agency.entitlements`).

#### One-time signing setup

Required on a machine that hasn't cut a release before:

1. **Developer ID Application certificate** — create it in Xcode
   (Settings → Accounts → your team → Manage Certificates → **+** →
   *Developer ID Application*) or at developer.apple.com. Creating one needs the
   **Account Holder** role on the team. Verify it's installed:

   ```sh
   security find-identity -v -p codesigning | grep "Developer ID Application"
   ```

   If the identity string differs from the one in `tauri.conf.json`, update
   `bundle.macOS.signingIdentity` to match.

2. **Notary credentials** — create an app-specific password at
   [appleid.apple.com](https://appleid.apple.com) (Sign-In & Security →
   App-Specific Passwords), then store a notarytool profile:

   ```sh
   xcrun notarytool store-credentials agency-notary \
     --apple-id "<your-apple-id>" --team-id "<TEAM_ID>" \
     --password "<app-specific-password>"
   ```

   The release script reads the `agency-notary` profile (override with the
   `AGENCY_NOTARY_PROFILE` env var).

#### Verifying a build

Check what Gatekeeper will decide — ideally on a second Mac or a fresh user
account, since your own machine trusts your cert:

```sh
spctl -a -vvv -t install target/release/bundle/dmg/Agency_*.dmg
```

If notarization is rejected, inspect the log for the offending file or
entitlement:

```sh
xcrun notarytool log <submission-id> --keychain-profile agency-notary
```

### Automated releases (CI)

`.github/workflows/release.yml` builds, signs, notarizes, and attaches a DMG to a
**draft** GitHub Release on the Apple Silicon runner. Trigger it by pushing a
version tag (or run it manually from the Actions tab):

```sh
git tag v0.1.0
git push origin v0.1.0
```

It runs the same `cargo tauri build`; the only difference from the local script is
that Tauri does signing **and** notarization natively from environment variables
(cert from a secret, notarization via an app-specific password) instead of your
local keychain and notary profile.

#### One-time CI secrets

Add these under **Settings → Secrets and variables → Actions**:

| Secret                       | Value                                                          |
| ---------------------------- | -------------------------------------------------------------- |
| `APPLE_CERTIFICATE`          | base64 of the exported Developer ID `.p12` (see below)         |
| `APPLE_CERTIFICATE_PASSWORD` | the password you set when exporting the `.p12`                 |
| `APPLE_SIGNING_IDENTITY`     | `Developer ID Application: Nicholas Counts (9XD5R448N5)`        |
| `APPLE_ID`                   | Apple ID email used for notarization                           |
| `APPLE_PASSWORD`             | app-specific password for that Apple ID                        |
| `APPLE_TEAM_ID`              | `9XD5R448N5`                                                    |

Export the cert for `APPLE_CERTIFICATE`: in **Keychain Access → login → My
Certificates**, select *Developer ID Application: Nicholas Counts*, right-click →
**Export** to a `.p12` with a password, then:

```sh
base64 -i Certificates.p12 | pbcopy   # paste into the APPLE_CERTIFICATE secret
```

The release is created as a **draft** — review the attached DMG, then publish it
manually.

> **Note:** CI currently builds for Apple Silicon (`aarch64`) only. Intel Macs
> would need a `universal-apple-darwin` build (both Rust targets plus a universal
> sidecar) — a follow-up if testers need it.
