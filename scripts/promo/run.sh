#!/bin/sh
# Records promo clips and stills of the real Agency UI against the mock backend.
# Needs the Vite dev server on :1420 (./dev.sh, or `pnpm --dir ui exec vite`).
# Usage: scripts/promo/run.sh [overview grid issues race review docs loop]
cd "$(dirname "$0")" && exec node scenes.mjs "$@"
