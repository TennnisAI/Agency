#!/usr/bin/env bash
# Build the agency-termd daemon (release) and produce the triple-suffixed
# sidecar binary that Tauri's externalBin bundler expects.
#
# Run this once before `pnpm tauri build` (or `cargo tauri build`):
#
#   ./crates/agency-app/build-termd.sh
#
# The script works from any directory inside the repo.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# Determine the host target triple at runtime so the script works on both
# aarch64-apple-darwin and x86_64-apple-darwin without modification.
TRIPLE="$(rustc -vV | sed -n 's/host: //p')"
echo "Host triple: $TRIPLE"

echo "Building agency-termd (release)..."
cargo build -p agency-core --bin agency-termd --release \
  --manifest-path "$REPO_ROOT/Cargo.toml"

SRC="$REPO_ROOT/target/release/agency-termd"
DST="$REPO_ROOT/target/release/agency-termd-$TRIPLE"

cp "$SRC" "$DST"
echo "Sidecar ready: $DST"
