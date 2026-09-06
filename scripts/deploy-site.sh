#!/usr/bin/env bash
# Deploy site/ to Cloudflare Pages (project: agency-site, domain getagency.dev).
#
#   ./scripts/deploy-site.sh
#
# Deploys a staging copy rather than site/ itself. Pages uploads the directory
# verbatim, so every file in it becomes a public URL, and `.assetsignore` does
# not apply to `pages deploy` — it is a Workers Static Assets feature. Checked
# on wrangler 4.129.0 against a real deployment: with an .assetsignore listing
# README.md in place, both /README.md and /.assetsignore came back 200. The
# exclusions have to happen before upload, so they happen here.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

rsync -a --exclude 'README.md' "$ROOT/site/" "$STAGE/"

echo "Staged $(find "$STAGE" -type f | wc -l | tr -d ' ') files from site/"
npx --yes wrangler@latest pages deploy "$STAGE" \
  --project-name agency-site \
  --branch main \
  --commit-dirty=true
