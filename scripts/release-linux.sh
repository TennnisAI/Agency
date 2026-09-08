#!/usr/bin/env bash
# Build distributable Agency packages for Linux: .deb, .rpm and AppImage.
#
# The counterpart to release-macos.sh, and deliberately shorter: Linux has no
# signing identity, no notarization service and no stapling, so steps 3-5 of the
# macOS script have no analogue here. What you get out is what you ship.
#
# Usage:
#   ./scripts/release-linux.sh                  # deb + rpm + AppImage
#   ./scripts/release-linux.sh --bundles deb     # just one of them
#   ./scripts/release-linux.sh --native          # build here, no container
#   AGENCY_LINUX_PLATFORM=linux/amd64 ./scripts/release-linux.sh
#
# Three formats cover the desktop Linux that matters, and Tauri can build all
# three: .deb is Debian *and* Ubuntu (one package, same format), .rpm is Fedora
# and openSUSE, and the AppImage runs anywhere including Arch. There is no
# pacman target in Tauri, so Arch is served by the AppImage; a native
# pacman package would mean maintaining a PKGBUILD on the AUR, which is a
# distribution chore rather than a build one.
#
# On macOS this re-runs itself inside a Debian container, because Tauri cannot
# cross-compile to Linux from macOS: the build links against the host's
# webkit2gtk, gtk and appindicator, and there is no SDK to point at. The
# container gets a *copy* of the tracked sources, never your working tree, so a
# Linux build cannot thrash the macOS artifacts in target/ or clobber
# ui/node_modules (whose esbuild binary is platform-specific). Built packages
# are copied back to target/linux/.
#
# Note on architecture: the container matches your machine unless you override
# it, so an Apple Silicon Mac produces an arm64 package. Most Linux desktops are
# x86_64; set AGENCY_LINUX_PLATFORM=linux/amd64 for one of those, which works
# through emulation and is considerably slower.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

NATIVE=0
BUNDLES="deb,rpm,appimage"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --native)  NATIVE=1; shift ;;
    --bundles) BUNDLES="${2:?--bundles needs a value, e.g. deb or deb,rpm}"; shift 2 ;;
    -h|--help) sed -n '2,36p' "$0"; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ "$(uname -s)" != "Linux" && "$NATIVE" -eq 0 ]]; then
  # ---- Docker path -------------------------------------------------------
  PLATFORM="${AGENCY_LINUX_PLATFORM:-}"
  IMAGE="rust:1-bookworm"
  OUT="$REPO_ROOT/target/linux"

  command -v docker >/dev/null 2>&1 || {
    echo "error: docker not found, and a Linux build needs it on $(uname -s)." >&2
    echo "       Install Docker (or colima), or run this on Linux with --native." >&2
    exit 1
  }
  docker info >/dev/null 2>&1 || {
    echo "error: the Docker daemon is not responding. Start it (colima start) and retry." >&2
    exit 1
  }

  # Tracked files only. Keeps target/ and ui/node_modules out of the copy, so
  # the container neither inherits macOS build products nor writes to yours.
  #
  # Staged under target/ and not in $TMPDIR: a Docker VM only shares some of the
  # host filesystem, and macOS `mktemp -d` returns a path under /var/folders,
  # which colima does not share. Mounting it silently yields an *empty
  # directory* inside the container rather than an error, so the build failed
  # with "tar: /stage/src.tar: Cannot open" and nothing to suggest the mount was
  # the problem. target/ is inside the repo, which is inside the shared home.
  STAGE="$REPO_ROOT/target/.linux-stage"
  rm -rf "$STAGE" && mkdir -p "$STAGE"
  trap 'rm -rf "$STAGE"' EXIT
  # `--cached --others --exclude-standard`: tracked files *plus* untracked ones
  # git would not ignore. Two reasons it is not plain `git ls-files`. It builds
  # the working tree you are looking at rather than the last commit, which is
  # what you want from a script you are iterating with; and plain `ls-files`
  # omits anything not yet added, which this script did to itself on its first
  # run — it was untracked, so it was not in its own tarball, and the container
  # failed with "/work/scripts/release-linux.sh: No such file or directory".
  # Ignored paths (target/, ui/node_modules) stay out either way.
  git ls-files --cached --others --exclude-standard -z \
    | tar --null -T - -cf "$STAGE/src.tar"
  mkdir -p "$OUT"

  echo "==> Building in $IMAGE${PLATFORM:+ ($PLATFORM)}; artifacts land in target/linux/"

  # Named volumes so the crate registry and the build cache survive between
  # runs — without them every invocation recompiles the whole tree.
  docker run --rm ${PLATFORM:+--platform "$PLATFORM"} \
    -v "$STAGE:/stage:ro" \
    -v "$OUT:/out" \
    -v agency-linux-cargo:/usr/local/cargo/registry \
    -v agency-linux-target:/work/target \
    -e "BUNDLES=$BUNDLES" \
    "$IMAGE" bash -euo pipefail -c '
      export DEBIAN_FRONTEND=noninteractive
      # xdg-utils is not a build dependency of anything: the AppImage bundler
      # copies /usr/bin/xdg-open *into* the image, because tauri-plugin-opener
      # shells out to it at runtime. Without it the deb and rpm bundle fine and
      # only the AppImage dies, with "xdg-open binary not found".
      echo "==> Installing build dependencies in the container"
      apt-get update -qq
      apt-get install -y -qq --no-install-recommends \
        libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
        librsvg2-dev libsoup-3.0-dev pkg-config file desktop-file-utils \
        curl ca-certificates xdg-utils >/dev/null
      # Node from nodesource, not the nodejs package in bookworm: that one is
      # Node 18, and pnpm 11 refuses to run on anything below 22.13 ("This
      # version of pnpm requires at least Node.js v22.13"). 22 is also what the
      # ui job in .github/workflows/ci.yml pins.
      curl -fsSL https://deb.nodesource.com/setup_22.x | bash - >/dev/null 2>&1
      apt-get install -y -qq --no-install-recommends nodejs >/dev/null
      npm install -g pnpm@11 >/dev/null 2>&1
      mkdir -p /work && cd /work && tar xf /stage/src.tar
      /work/scripts/release-linux.sh --native
      echo "==> Copying packages out"
      find /work/target/release/bundle -maxdepth 2 -type f \
        \( -name "*.deb" -o -name "*.AppImage" -o -name "*.rpm" \) \
        -exec cp -v {} /out/ \;
    '

  echo
  echo "Done. Packages in target/linux/:"
  ls -1sh "$OUT" | sed "s|^|  |"
  echo
  echo "Install one on a Linux box with:"
  echo "  sudo apt install ./$(basename "$(find "$OUT" -name '*.deb' | head -n1)" 2>/dev/null || echo 'Agency_*.deb')"
  exit 0
fi

# ---- Native Linux path ---------------------------------------------------
if [[ "$(uname -s)" != "Linux" ]]; then
  echo "error: --native on $(uname -s). Tauri links against the host's webkit2gtk," >&2
  echo "       so a Linux package can only be built on Linux." >&2
  exit 1
fi

echo "==> [1/4] Installing frontend dependencies"
pnpm --dir ui install --frozen-lockfile

echo "==> [2/4] Building agency-termd sidecar (release)"
./crates/agency-app/build-termd.sh

# `tauri build` runs beforeBuildCommand (pnpm build) itself, so the frontend is
# built here rather than as a separate step. Invoked through the project's own
# pinned CLI rather than a globally installed cargo-tauri, so the version is the
# one in ui/pnpm-lock.yaml.
echo "==> [3/4] Building the bundles ($BUNDLES)"
ui/node_modules/.bin/tauri build --bundles "$BUNDLES"

echo "==> [4/4] Verifying the artifacts"
BUNDLE_DIR="target/release/bundle"
mapfile -t PACKAGES < <(find "$BUNDLE_DIR" -maxdepth 2 -type f \
  \( -name '*.deb' -o -name '*.rpm' -o -name '*.AppImage' \) 2>/dev/null)

if [[ "${#PACKAGES[@]}" -eq 0 ]]; then
  echo "error: no packages produced under $BUNDLE_DIR" >&2
  exit 1
fi

for pkg in "${PACKAGES[@]}"; do
  echo "  $pkg"
  # A .deb whose control file is unreadable is a .deb that fails on install
  # rather than at build time, which is the wrong end to find out.
  if [[ "$pkg" == *.deb ]]; then
    dpkg-deb --info "$pkg" >/dev/null || { echo "error: $pkg is not a readable .deb" >&2; exit 1; }
  fi
done

echo
echo "Done."
