#!/bin/sh
# Assets for the GitHub org profile (github.com/TennnisAI/.github, profile/):
# the banner in both themes, and the hero loop as an animated WebP, because a
# README can show an image but cannot play a self-hosted video.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
OUT="$HERE/../out"
mkdir -p "$OUT/profile"
node "$HERE/render-banner.mjs"
rm -rf "$OUT/awebp" && mkdir "$OUT/awebp"
ffmpeg -v error -i "$OUT/hero-master.mp4" -vf "fps=15,scale=1200:750:flags=lanczos" "$OUT/awebp/%04d.png"
# -mixed and a keyframe every 1-2s: all-lossy deltas left grey blocks on the
# dark ground wherever a crossfade had passed.
img2webp -loop 0 -mixed -q 80 -m 4 -kmin 15 -kmax 30 -d 67 "$OUT"/awebp/*.png -o "$OUT/profile/hero.webp"
rm -rf "$OUT/awebp"
ls -l "$OUT/profile"
