#!/bin/sh
# Encodes the masters for the site, and writes the posters and the social card.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
OUT="$HERE/../out"
SITE="$HERE/../../../site/assets"
F="ffmpeg -v error -y"
mkdir -p "$SITE/video"

# Hero loop: 1920x1200 covers a window that tops out near 1112 CSS px.
$F -i "$OUT/hero-master.mp4" -vf scale=1920:1200:flags=lanczos -c:v libx264 -preset slow -crf 26 -profile:v high -pix_fmt yuv420p -movflags +faststart -an "$SITE/video/hero.mp4"
$F -i "$OUT/hero-master.mp4" -vf scale=1920:1200:flags=lanczos -c:v libvpx-vp9 -b:v 0 -crf 38 -row-mt 1 -deadline good -cpu-used 2 -pix_fmt yuv420p -an "$SITE/video/hero.webm"
$F -i "$OUT/hero-master.mp4" -frames:v 1 -vf scale=1920:1200:flags=lanczos "$OUT/hero-poster.png"
cwebp -quiet -q 80 "$OUT/hero-poster.png" -o "$SITE/video/hero-poster.webp"

# Tour: 1920x1080 on the site. The 2560x1440 master stays in out/ for uploads.
$F -i "$OUT/tour-master.mp4" -vf scale=1920:1080:flags=lanczos -c:v libx264 -preset slow -crf 23 -profile:v high -pix_fmt yuv420p -movflags +faststart -an "$SITE/video/tour.mp4"
$F -i "$OUT/tour-master.mp4" -vf scale=1920:1080:flags=lanczos -c:v libvpx-vp9 -b:v 0 -crf 36 -row-mt 1 -deadline good -cpu-used 2 -pix_fmt yuv420p -an "$SITE/video/tour.webm"
# 3.4s is the intro card with all three headline lines in.
$F -ss 3.4 -i "$OUT/tour-master.mp4" -frames:v 1 -vf scale=1920:1080:flags=lanczos "$OUT/tour-poster.png"
cwebp -quiet -q 80 "$OUT/tour-poster.png" -o "$SITE/video/tour-poster.webp"

# Social card as JPEG, because not every link unfurler reads WebP.
$F -ss 3.4 -i "$OUT/tour-master.mp4" -frames:v 1 -vf "scale=1200:675:flags=lanczos,crop=1200:630:0:22" -q:v 3 "$SITE/img/og.jpg"
ls -l "$SITE/video" "$SITE/img/og.jpg"
