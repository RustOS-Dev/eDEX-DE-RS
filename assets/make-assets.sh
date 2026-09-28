#!/usr/bin/env bash
# Render the PNG assets from the SVG sources (needs rsvg-convert from librsvg).
# Outputs land in assets/generated/ (git-ignored) and are installed by the packages.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=${1:-$HERE/generated}
mkdir -p "$OUT/icons" "$OUT/backgrounds"
command -v rsvg-convert >/dev/null || { echo "rsvg-convert (librsvg) is required" >&2; exit 1; }
for s in 32 48 64 128 256 512; do
  rsvg-convert -w $s -h $s "$HERE/logo.svg" -o "$OUT/icons/edex-de-$s.png"
done
rsvg-convert -w 1920 -h 1080 "$HERE/hexgrid.svg" -o "$OUT/backgrounds/hexgrid-1080p.png"
rsvg-convert -w 2560 -h 1440 "$HERE/hexgrid.svg" -o "$OUT/backgrounds/hexgrid-1440p.png"
rsvg-convert -w 3840 -h 2160 "$HERE/hexgrid.svg" -o "$OUT/backgrounds/hexgrid-4k.png"
cp "$OUT/backgrounds/hexgrid-1080p.png" "$OUT/backgrounds/lock.png"
echo "assets written to $OUT"
