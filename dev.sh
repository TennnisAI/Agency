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
#   2. Launches tauri dev via pnpm from the ui/ directory
#
# For release builds, run crates/agency-app/build-termd.sh first, then
# `pnpm tauri build` from crates/agency-app/.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"

echo "Building agency-termd (debug)..."
cargo build -p agency-core --bin agency-termd \
  --manifest-path "$REPO_ROOT/Cargo.toml"
echo "Daemon binary: $REPO_ROOT/target/debug/agency-termd"

echo "Launching tauri dev..."
cd "$REPO_ROOT/ui"
exec pnpm tauri dev
