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
# exist at target/release/agency-termd-<triple> even in dev, and tauri-build
# copies it next to the app executable — which is the binary the app actually
# spawns. crates/agency-app/build.rs stages one when it is missing (so plain
# cargo commands work on a fresh worktree), but only when it is missing.
#
# So copy every run, not just when the file is absent: a stale copy means
# daemon-side changes silently don't run and you debug the old code. (A later
# `pnpm tauri build` still uses build-termd.sh for the real release sidecar.)
TRIPLE="$(rustc -vV | sed -n 's/host: //p')"
SIDECAR="$REPO_ROOT/target/release/agency-termd-$TRIPLE"
mkdir -p "$REPO_ROOT/target/release"
cp "$REPO_ROOT/target/debug/agency-termd" "$SIDECAR"
echo "Staged dev sidecar: $SIDECAR"

# A dev build keeps its own data dir (crates/agency-app/src/datadir.rs), so it
# runs the daemon binary just built, on its own socket, and cannot disturb the
# installed app's sessions. Different DB too: the dev one is seeded from the
# installed app's projects on first launch, and its runs are its own.
DEV_DIR="$HOME/Library/Application Support/build.agency.app.dev"
echo
echo "Dev data dir: $DEV_DIR"
echo "  own agency.db and termd.sock; the installed Agency.app is untouched."
echo "  To end this build's daemon (and its sessions):"
echo "    pkill -f 'agency-termd .*build.agency.app.dev'"
echo "  Delete the dir to start over; the next launch re-seeds it."
echo

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
