#!/usr/bin/env bash
# Render the greeter headlessly (demo mode) and screenshot it.
set -euo pipefail
BIN=${1:-target/debug/edex-greeter}
OUT=${2:-target/smoke}
mkdir -p "$OUT"
export XDG_RUNTIME_DIR=$(mktemp -d /tmp/edex-greeter.XXXXXX)
chmod 700 "$XDG_RUNTIME_DIR"
export EDEX_SHARE_DIR="${EDEX_SHARE_DIR:-$(cd "$(dirname "$0")/.." && pwd)/share}"
cleanup() { [ -n "${SWAY_PID:-}" ] && kill "$SWAY_PID" 2>/dev/null || true; }
trap cleanup EXIT
echo "output HEADLESS-1 resolution 1280x720" > "$XDG_RUNTIME_DIR/sway.conf"
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway -c "$XDG_RUNTIME_DIR/sway.conf" > "$OUT/sway-greeter.log" 2>&1 &
SWAY_PID=$!
for _ in $(seq 1 50); do
  WD=$(ls "$XDG_RUNTIME_DIR" 2>/dev/null | grep -m1 -E '^wayland-[0-9]+$' || true)
  [ -n "$WD" ] && break
  sleep 0.2
done
export WAYLAND_DISPLAY=$WD
( sleep 3; grim "$OUT/10-greeter.png" ) &
GRIM_PID=$!
"$BIN" --demo --smoke-test 5 --config /nonexistent/greeter.toml > "$OUT/greeter-report.json" 2> "$OUT/greeter.log"
wait "$GRIM_PID" || true
cat "$OUT/greeter-report.json"
python3 -c "import json; r=json.load(open('$OUT/greeter-report.json')); assert r['frames']>=3, r"
echo "GREETER SMOKE OK"
