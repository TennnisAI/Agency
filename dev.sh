#!/usr/bin/env bash
# Developer convenience script: build the agency-termd daemon (debug) then
# launch the Tauri dev server.
#
# Use this instead of running `pnpm tauri dev` directly.  The Tauri
# `beforeDevCommand` in tauri.conf.json only starts the Vite UI server; it
# does NOT build the agency-core daemon binary.  Running `tauri dev` without
# the daemon present causes AppState::new() to fail to spawn agency-termd.
#
# Usage (from any directory in the repo):
#   ./dev.sh
#
# What this does:
#   1. Builds target/debug/agency-termd  (cargo build -p agency-core --bin agency-termd)
#   2. Installs ui/ dependencies if missing (fresh worktrees have no node_modules)
#   3. Launches tauri dev via pnpm from the ui/ directory
#
# For release builds, run crates/agency-app/build-termd.sh first, then
# `pnpm tauri build` from crates/agency-app/.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"

echo "Building agency-termd (debug)..."
cargo build -p agency-core --bin agency-termd \
  --manifest-path "$REPO_ROOT/Cargo.toml"
echo "Daemon binary: $REPO_ROOT/target/debug/agency-termd"

# Tauri's externalBin (tauri.conf.json) requires the triple-suffixed sidecar to
# exist at target/release/agency-termd-<triple> even in dev — its build script
# validates the resource and aborts if missing. build-termd.sh normally produces
# it via a full release build, but a fresh git worktree has an empty target/ and
# has never run it, so `tauri dev` fails with "resource path ... doesn't exist".
# Stage the just-built debug binary under that name so dev works with no separate
# release build. (A later `pnpm tauri build` still uses build-termd.sh for the
# real release sidecar.)
#
# Copy every run, not just when the file is missing: this is the binary the app
# actually spawns, so a stale copy means daemon-side changes silently don't run
# and you debug the old code.
TRIPLE="$(rustc -vV | sed -n 's/host: //p')"
SIDECAR="$REPO_ROOT/target/release/agency-termd-$TRIPLE"
mkdir -p "$REPO_ROOT/target/release"
cp "$REPO_ROOT/target/debug/agency-termd" "$SIDECAR"
echo "Staged dev sidecar: $SIDECAR"

# The daemon is shared, not per-build: one socket in Application Support, and
# whoever spawns it first owns the binary that serves everyone. If the installed
# app (or its daemon) is already up, the dev build attaches to *that* daemon and
# any daemon-side change you just built is not in play. Say so instead of
# letting it look like the change did nothing.
SOCK="$HOME/Library/Application Support/build.agency.app/termd.sock"
if pgrep -f "Agency.app/Contents/MacOS/agency-termd" >/dev/null 2>&1; then
  echo
  echo "NOTE: a daemon from the installed Agency.app is running and owns $SOCK."
  echo "      This dev build will attach to it, so daemon changes won't apply."
  echo "      To test them: quit Agency.app, then"
  echo "        pkill -f 'Agency.app/Contents/MacOS/agency-termd'"
  echo "      (this ends the sessions that daemon hosts) and re-run ./dev.sh."
  echo
fi

# Each git worktree is a separate checkout with its own empty ui/node_modules,
# so the tauri CLI (a devDependency) is absent until deps are installed. Install
# on demand so a fresh worktree runs without a manual `pnpm install` first.
if [ ! -x "$REPO_ROOT/ui/node_modules/.bin/tauri" ]; then
  echo "Installing ui/ dependencies (pnpm install)..."
  pnpm --dir "$REPO_ROOT/ui" install
fi

echo "Launching tauri dev..."
cd "$REPO_ROOT/crates/agency-app"
exec "$REPO_ROOT/ui/node_modules/.bin/tauri" dev
