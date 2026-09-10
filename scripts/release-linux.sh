#!/usr/bin/env bash
# Build distributable Agency packages for Linux: .deb, .rpm and AppImage.
#
# The counterpart to release-macos.sh, and deliberately shorter: Linux has no
# signing identity, no notarization service and no stapling, so steps 3-5 of the
# macOS script have no analogue here. What you get out is what you ship.
#
# Usage:
#   ./scripts/release-linux.sh                  # deb + rpm + AppImage, this arch
#   ./scripts/release-linux.sh --arch amd64      # x86_64, via emulation
#   ./scripts/release-linux.sh --arch both       # what a release ships
#   ./scripts/release-linux.sh --bundles deb     # just one format
#   ./scripts/release-linux.sh --native          # build here, no container
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
ARCH="host"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --native)  NATIVE=1; shift ;;
    --bundles) BUNDLES="${2:?--bundles needs a value, e.g. deb or deb,rpm}"; shift 2 ;;
    --arch)    ARCH="${2:?--arch needs one of: host, amd64, arm64, both}"; shift 2 ;;
    -h|--help) sed -n '2,37p' "$0"; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ "$(uname -s)" != "Linux" && "$NATIVE" -eq 0 ]]; then
  # ---- Docker path -------------------------------------------------------
  IMAGE="rust:1-bookworm"
  OUT="$REPO_ROOT/target/linux"

  case "$ARCH" in
    host)  PLATFORMS=("") ;;
    amd64) PLATFORMS=("linux/amd64") ;;
    arm64) PLATFORMS=("linux/arm64") ;;
    both)  PLATFORMS=("linux/arm64" "linux/amd64") ;;
    *) echo "error: --arch must be one of: host, amd64, arm64, both" >&2; exit 2 ;;
  esac

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
  # COPYFILE_DISABLE: without it macOS tar writes an AppleDouble `._name`
  # sidecar for every file carrying an extended attribute, and tauri-build
  # then reads `capabilities/._default.json` as a capability file and fails
  # with "stream did not contain valid UTF-8" (observed 2026-09-10; files
  # saved by some editors and agents carry com.apple.provenance).
  git ls-files --cached --others --exclude-standard -z \
    | COPYFILE_DISABLE=1 tar --null -T - -cf "$STAGE/src.tar"
  mkdir -p "$OUT"

  for PLATFORM in "${PLATFORMS[@]}"; do
  echo "==> Building in $IMAGE${PLATFORM:+ ($PLATFORM)}; artifacts land in target/linux/"
  if [[ "$PLATFORM" == "linux/amd64" && "$(uname -m)" == "arm64" ]]; then
    echo "    (x86_64 under emulation on Apple Silicon; expect this to take a while)"
  fi

  # Named volumes so the crate registry and the build cache survive between
  # runs — without them every invocation recompiles the whole tree.
  #
  # The target volume is per-architecture and the registry is not. Registry
  # entries are unpacked crate *sources*, identical everywhere; build artifacts
  # are not, and nothing here passes `--target`, so every arch would otherwise
  # write its objects to the same target/release and evict the other one on
  # each switch.
  VOL_SUFFIX="$(printf '%s' "${PLATFORM:-host}" | tr '/' '-')"
  docker run --rm ${PLATFORM:+--platform "$PLATFORM"} \
    -v "$STAGE:/stage:ro" \
    -v "$OUT:/out" \
    -v agency-linux-cargo:/usr/local/cargo/registry \
    -v "agency-linux-target-$VOL_SUFFIX:/work/target" \
    -e "BUNDLES=$BUNDLES" \
    "$IMAGE" bash -euo pipefail -c '
      export DEBIAN_FRONTEND=noninteractive
      # xdg-utils is not a build dependency of anything: the AppImage bundler
      # copies /usr/bin/xdg-open *into* the image, because tauri-plugin-opener
      # shells out to it at runtime. Without it the deb and rpm bundle fine and
      # only the AppImage dies, with "xdg-open binary not found".
      echo "==> Installing build dependencies in the container"
      # Log rather than discard. Sending apt to /dev/null meant a failed
      # install surfaced as nothing but "E: Sub-process /usr/bin/dpkg returned
      # an error code (1)", with the line naming the actual package thrown
      # away — which is a bad trade for quiet output in a build script.
      apt() { apt-get "$@" >>/tmp/apt.log 2>&1 || { echo "apt failed:"; tail -40 /tmp/apt.log; exit 1; }; }
      apt update -qq
      # appstream and lintian are for the verification step, not the build;
      # see verify_deb below for what each catches.
      apt install -y --no-install-recommends \
        libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
        librsvg2-dev libsoup-3.0-dev pkg-config file desktop-file-utils \
        curl ca-certificates xdg-utils appstream lintian
      # Node from nodesource, not the nodejs package in bookworm: that one is
      # Node 18, and pnpm 11 refuses to run on anything below 22.13 ("This
      # version of pnpm requires at least Node.js v22.13"). 22 is also what the
      # ui job in .github/workflows/ci.yml pins.
      curl -fsSL https://deb.nodesource.com/setup_22.x | bash - >>/tmp/apt.log 2>&1
      apt install -y --no-install-recommends nodejs
      npm install -g pnpm@11 >>/tmp/apt.log 2>&1
      mkdir -p /work && cd /work && tar xf /stage/src.tar
      # Passed as a flag, not left to the exported variable: the script sets
      # its own default before parsing arguments, so `--bundles deb` on the
      # outside used to build all three formats on the inside.
      /work/scripts/release-linux.sh --native --bundles "$BUNDLES"
      echo "==> Copying packages out"
      find /work/target/release/bundle -maxdepth 2 -type f \
        \( -name "*.deb" -o -name "*.AppImage" -o -name "*.rpm" \) \
        -exec cp -v {} /out/ \;
    '
  done

  echo
  echo "Done. Packages in target/linux/:"
  ls -1sh "$OUT" | sed "s|^|  |"
  echo
  echo "Install one on a Linux box with:"
  # Newest by name, not first found: target/linux/ keeps earlier versions, and
  # the hint used to name the 0.1.0 package after building 0.1.1.
  echo "  sudo apt install ./$(basename "$(find "$OUT" -name '*.deb' | sort -V | tail -n1)" 2>/dev/null || echo 'Agency_*.deb')"
  exit 0
fi

# ---- Native Linux path ---------------------------------------------------
if [[ "$(uname -s)" != "Linux" ]]; then
  echo "error: --native on $(uname -s). Tauri links against the host's webkit2gtk," >&2
  echo "       so a Linux package can only be built on Linux." >&2
  exit 1
fi

# Everything the verification step needs, checked before the twenty-minute
# build rather than after it. binutils is readelf; the rest are named after
# their apt packages.
for tool in dpkg-deb readelf desktop-file-validate appstreamcli lintian; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "error: $tool not found. The verification step needs:" >&2
    echo "       apt install binutils dpkg desktop-file-utils appstream lintian" >&2
    exit 1
  }
done

# Strip the release binaries. Nothing in the workspace sets a release profile,
# so the first package carried 32 MB of symbols in /usr/bin/Agency and lintian
# flagged both binaries as unstripped-binary-or-object. Set here rather than in
# Cargo.toml so it is a Linux packaging decision and not a change to what the
# macOS build produces; cargo reads it for build-termd.sh and `tauri build`
# alike.
export CARGO_PROFILE_RELEASE_STRIP=true

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

# What a .deb has to get right before anyone installs it. Each check is here
# because the first package to reach an Ubuntu desktop (2026-09-08) got it
# wrong and the build was green regardless; none of these fails at build time
# on its own.
verify_deb() {
  local pkg="$1" scratch
  scratch="$(mktemp -d)"
  # A .deb whose control file is unreadable fails on install rather than at
  # build time, which is the wrong end to find out.
  dpkg-deb --info "$pkg" > "$scratch/info" \
    || { echo "error: $pkg is not a readable .deb" >&2; exit 1; }
  # The whole tree, not a member list: the bundler writes members as
  # `usr/...`, and a `./usr/...` pattern is "Not found in archive".
  dpkg-deb -x "$pkg" "$scratch"

  # The glibc floor. A binary runs on any glibc at least as new as the one it
  # was linked against and on none older, and nothing declares that unless we
  # do: a package built on glibc 2.39 installed on Debian 12 (2.36) without a
  # word and died at launch with "version `GLIBC_2.39' not found". With a libc6
  # floor in Depends the same package refuses to install instead, which is the
  # right end. The floor is typed by hand into tauri.conf.json, so check it
  # against what the shipped binaries actually import.
  local need declared
  need="$(readelf -V "$scratch"/usr/bin/* | grep -o 'GLIBC_[0-9]\+\.[0-9]\+' \
    | sed 's/GLIBC_//' | sort -V | tail -n1)"
  declared="$(grep -o 'libc6 (>= [0-9.]*)' "$scratch/info" | grep -o '[0-9.]*)$' | tr -d ')' || true)"
  if [[ -z "$declared" ]]; then
    echo "error: $pkg does not depend on libc6; the binaries need GLIBC_$need." >&2
    echo "       Add \"libc6 (>= $need)\" to bundle.linux.deb.depends in tauri.conf.json." >&2
    exit 1
  fi
  if dpkg --compare-versions "$declared" lt "$need"; then
    echo "error: $pkg declares libc6 (>= $declared) but its binaries need GLIBC_$need." >&2
    echo "       Raise the floor in tauri.conf.json, or build on an older image." >&2
    exit 1
  fi
  echo "    glibc floor: declared $declared, binaries need $need"

  # The desktop file and the AppStream metainfo are what a software centre
  # reads; a malformed one is a launcher that does not appear, or an
  # "installed" page with no name and a placeholder icon.
  desktop-file-validate "$scratch"/usr/share/applications/*.desktop
  appstreamcli validate --no-net "$scratch"/usr/share/metainfo/*.metainfo.xml

  # Debian policy, at the error level only. The first package tripped five:
  # malformed-contact ("Maintainer: agency"), missing-dependency-on-libc,
  # no-copyright-file, and unstripped-binary-or-object twice. no-changelog is
  # suppressed on purpose: CHANGELOG.md is Markdown, and a Debian-format
  # changelog would be a second copy of it maintained by hand.
  lintian --fail-on error --suppress-tags no-changelog "$pkg"
  rm -rf "$scratch"
}

for pkg in "${PACKAGES[@]}"; do
  echo "  $pkg"
  if [[ "$pkg" == *.deb ]]; then
    verify_deb "$pkg"
  fi
done

echo
echo "Done."
