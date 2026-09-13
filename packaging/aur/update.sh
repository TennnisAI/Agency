#!/usr/bin/env bash
# Point the AUR package at a published release: set pkgver, reset pkgrel, fill
# both checksums from the release's own .deb files, and regenerate .SRCINFO.
#
# Usage: packaging/aur/update.sh <version>      e.g. 0.2.0
#
# Run it after the release is published, since it reads the public download
# URLs, then push agency-bin/ to the AUR (docs/releasing.md has the steps).
# AGENCY_RELEASE_BASE overrides where the .deb files are read from, including a
# file:// URL, which is how it is tested before a release exists.
#
# .SRCINFO is generated here rather than by `makepkg --printsrcinfo` because
# makepkg only exists on Arch and releases are cut from a Mac. It is generated
# by sourcing the PKGBUILD, not typed out, so the two cannot drift apart: the
# AUR shows .SRCINFO and builds from the PKGBUILD, and a mismatch between them
# is a package page describing something other than what installs.
# Written for the bash 3.2 macOS ships: no namerefs, no mapfile.
set -euo pipefail

VERSION="${1:?usage: update.sh <version>, e.g. 0.2.0}"
DIR="$(cd "$(dirname "$0")" && pwd)/agency-bin"
BASE="${AGENCY_RELEASE_BASE:-https://github.com/TennnisAI/Agency/releases/download/v$VERSION}"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum | cut -d' ' -f1
  else shasum -a 256 | cut -d' ' -f1; fi
}
sum_of() {
  local url="$BASE/$1" sum
  sum="$(curl -fsSL "$url" | sha256)" || { echo "error: could not download $url" >&2; exit 1; }
  echo "$sum"
}

AMD64="$(sum_of "Agency_${VERSION}_amd64.deb")"
ARM64="$(sum_of "Agency_${VERSION}_arm64.deb")"

sed -e "s/^pkgver=.*/pkgver=$VERSION/" \
    -e "s/^pkgrel=.*/pkgrel=1/" \
    -e "s/^sha256sums_x86_64=.*/sha256sums_x86_64=('$AMD64')/" \
    -e "s/^sha256sums_aarch64=.*/sha256sums_aarch64=('$ARM64')/" \
    "$DIR/PKGBUILD" > "$DIR/PKGBUILD.new"
mv "$DIR/PKGBUILD.new" "$DIR/PKGBUILD"

# The fields this PKGBUILD uses, in the layout makepkg writes. A field added to
# the PKGBUILD has to be added here too; the check in docs/releasing.md
# (`makepkg --printsrcinfo` on an Arch box) is what catches a forgotten one.
(
  # shellcheck disable=SC1091
  source "$DIR/PKGBUILD"
  field() { local key="$1"; shift; local v; for v in "$@"; do printf '\t%s = %s\n' "$key" "$v"; done; }
  printf 'pkgbase = %s\n' "$pkgname"
  field pkgdesc "$pkgdesc"
  field pkgver "$pkgver"
  field pkgrel "$pkgrel"
  field url "$url"
  field arch "${arch[@]}"
  field license "${license[@]}"
  field depends "${depends[@]}"
  field optdepends "${optdepends[@]}"
  field provides "${provides[@]}"
  field conflicts "${conflicts[@]}"
  for a in "${arch[@]}"; do
    eval "field source_$a \"\${source_$a[@]}\""
    eval "field sha256sums_$a \"\${sha256sums_$a[@]}\""
  done
  printf '\npkgname = %s\n' "$pkgname"
) > "$DIR/.SRCINFO"

echo "agency-bin now points at $VERSION:"
echo "  x86_64   $AMD64"
echo "  aarch64  $ARM64"
