#!/usr/bin/env bash
# Build, sign, notarize, and staple a distributable Agency DMG.
#
# Prerequisites (one-time, already done on Nick's machine):
#   * "Developer ID Application" cert installed in the login keychain
#   * notarytool keychain profile created:
#       xcrun notarytool store-credentials agency-notary \
#         --apple-id <id> --team-id 9XD5R448N5 --password <app-specific-pw>
#
# The signing identity + hardened runtime + entitlements live in
# crates/agency-app/tauri.conf.json, so `cargo tauri build` deep-signs the
# whole bundle (including the agency-termd sidecar) automatically. This script
# only adds the notarize + staple steps that Tauri doesn't do on its own.
#
# Usage:  ./scripts/release-macos.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

NOTARY_PROFILE="${AGENCY_NOTARY_PROFILE:-agency-notary}"

# Package the DMG headless-safe: CI=true makes Tauri's bundle_dmg.sh skip the
# Finder AppleScript that styles the DMG window (icon layout / background). That
# osascript step requires GUI automation permission and fails when the build runs
# from a non-interactive shell, leaving a temp image mounted. The drag-to-
# Applications shortcut is still created — only the cosmetic layout is skipped.
export CI=true

echo "==> [1/5] Building agency-termd sidecar (release)"
./crates/agency-app/build-termd.sh

echo "==> [2/5] Building + signing the app bundle (cargo tauri build)"
cargo tauri build

APP="target/release/bundle/macos/Agency.app"
DMG="$(find target/release/bundle/dmg -maxdepth 1 -name '*.dmg' | head -n1)"

if [[ ! -d "$APP" ]]; then
  echo "error: expected app bundle not found at $APP" >&2
  exit 1
fi
if [[ -z "${DMG:-}" || ! -f "$DMG" ]]; then
  echo "error: no .dmg produced under target/release/bundle/dmg" >&2
  exit 1
fi

echo "==> [3/5] Verifying the signature (deep, strict)"
codesign --verify --deep --strict --verbose=2 "$APP"

echo "==> [4/5] Notarizing $DMG (this uploads to Apple and waits)"
xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait

echo "==> [5/5] Stapling the notarization ticket"
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"

echo
echo "Done. Distributable DMG:"
echo "  $DMG"
echo
echo "Sanity check on a clean machine (or after re-download):"
echo "  spctl -a -vvv -t install \"$DMG\""
