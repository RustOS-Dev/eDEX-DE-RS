#!/usr/bin/env bash
# Headless smoke test: start sway (headless, pixman), run edex-de for a few seconds, exercise
# notifications, IPC and overlays, take screenshots, and assert the report.
# Usage: scripts/smoke-sway.sh [path/to/edex-de] [out-dir]
set -euo pipefail

BIN=${1:-target/debug/edex-de}
OUT=${2:-target/smoke}
SECS=${SMOKE_SECS:-18}
mkdir -p "$OUT"
export XDG_RUNTIME_DIR=$(mktemp -d /tmp/edex-smoke.XXXXXX)
chmod 700 "$XDG_RUNTIME_DIR"
export XDG_CONFIG_HOME="$XDG_RUNTIME_DIR/config"
export XDG_STATE_HOME="$XDG_RUNTIME_DIR/state"
export EDEX_SHARE_DIR="${EDEX_SHARE_DIR:-$(cd "$(dirname "$0")/.." && pwd)/share}"
export RUST_LOG=${RUST_LOG:-info}

cleanup() {
  pkill -P $$ >/dev/null 2>&1 || true
  [ -n "${DBUS_PID:-}" ] && kill "$DBUS_PID" 2>/dev/null || true
  [ -n "${SWAY_PID:-}" ] && kill "$SWAY_PID" 2>/dev/null || true
}
trap cleanup EXIT

# Private session bus so the notification server can own its name.
DBUS_ADDR_FILE="$XDG_RUNTIME_DIR/dbus-addr"
dbus-daemon --session --fork --print-address 3 --print-pid 4 3> "$DBUS_ADDR_FILE" 4> "$XDG_RUNTIME_DIR/dbus-pid"
DBUS_PID=$(cat "$XDG_RUNTIME_DIR/dbus-pid")
export DBUS_SESSION_BUS_ADDRESS=$(cat "$DBUS_ADDR_FILE")

cat > "$XDG_RUNTIME_DIR/sway.conf" <<CONF
output HEADLESS-1 resolution 1280x720
CONF
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
  sway -c "$XDG_RUNTIME_DIR/sway.conf" > "$OUT/sway.log" 2>&1 &
SWAY_PID=$!
for _ in $(seq 1 50); do
  WD=$(ls "$XDG_RUNTIME_DIR" 2>/dev/null | grep -m1 -E '^wayland-[0-9]+$' || true)
  [ -n "$WD" ] && break
  sleep 0.2
done
[ -n "${WD:-}" ] || { echo "sway did not start"; cat "$OUT/sway.log"; exit 1; }
export WAYLAND_DISPLAY=$WD
export SWAYSOCK=$(ls "$XDG_RUNTIME_DIR"/sway-ipc.* | head -1)

"$BIN" run --no-hypr --smoke-test "$SECS" > "$OUT/report.json" 2> "$OUT/edex-de.log" &
EDEX_PID=$!

IPC="$XDG_RUNTIME_DIR/edex-de/ipc.sock"
for _ in $(seq 1 100); do
  [ -S "$IPC" ] && break
  sleep 0.2
done
[ -S "$IPC" ] || { echo "ipc socket never appeared"; cat "$OUT/edex-de.log"; exit 1; }
sleep 2

fail=0
step() { echo "==> $*"; }

step "ping"
"$BIN" ipc ping | tee "$OUT/ping.json" | grep -q '"ok":true' || fail=1

step "screenshot: canvas"
grim "$OUT/01-canvas.png" || fail=1

# Real input devices: wtype adds a virtual keyboard to the seat after the shell has started.
# The shell must pick it up (it once ignored every seat that existed before it connected, so
# neither keyboard nor pointer ever worked). Typing itself is checked on Hyprland (docs/testing.md):
# sway only grants exclusive keyboard focus to top/overlay layers and has no pointer here.
step "keyboard device is picked up"
wtype -s 4000 "x" &
WTYPE_PID=$!
sleep 2
"$BIN" ipc state > "$OUT/state-input.json"
grep -q '"keyboard":true' "$OUT/state-input.json" || { echo "shell did not take the seat keyboard"; cat "$OUT/state-input.json"; fail=1; }
wait $WTYPE_PID || true

step "notify-send → toast"
notify-send -a smoke "Smoke test" "hello from notify-send" || fail=1
sleep 1
"$BIN" ipc state > "$OUT/state-toast.json"
grep -q '"toasts":1' "$OUT/state-toast.json" || { echo "expected one toast"; cat "$OUT/state-toast.json"; fail=1; }
grim "$OUT/02-toast.png" || true

step "tiled app window lands in the terminal slot"
foot >/dev/null 2>&1 &
sleep 2
swaymsg -t get_tree > "$OUT/tree.json"
python3 - "$OUT/tree.json" <<'PY' || fail=1
import json, sys
t = json.load(open(sys.argv[1]))
def walk(n):
    if n.get("app_id") == "foot":
        yield n
    for c in n.get("nodes", []) + n.get("floating_nodes", []):
        yield from walk(c)
wins = list(walk(t))
assert wins, "foot window not found"
r = wins[0]["rect"]
print("foot rect:", r)
# The reservers must leave the app strictly inside the screen with margins on every side.
assert r["x"] > 0 and r["y"] > 0 and r["x"] + r["width"] < 1280 and r["y"] + r["height"] < 720, "app window is not tiled into the reserved terminal slot"
PY
grim "$OUT/03-app-tiled.png" || true

for ov in launcher settings privacy notifications power; do
  step "overlay: $ov"
  "$BIN" ipc show "$ov" | grep -q '"ok":true' || fail=1
  sleep 0.7
  "$BIN" ipc state | grep -q "\"overlay\":\"$ov\"" || { echo "overlay $ov not open"; fail=1; }
  grim "$OUT/04-$ov.png" || true
  "$BIN" ipc hide "$ov" >/dev/null || fail=1
done

step "theme switch"
"$BIN" ipc theme tron | grep -q '"ok":true' || fail=1

step "scene dump"
"$BIN" ipc screenshot-scene > "$OUT/scene.json" || fail=1
python3 -c "import json,sys; d=json.load(open('$OUT/scene.json'))['data']; assert d['rects']>50 and any('TERMINAL' in t.upper() or 'EDEX' in t.upper() for t in d['texts']), d['texts'][:20]; print('scene ok:', d['rects'], 'rects', len(d['texts']), 'texts')" || fail=1

wait $EDEX_PID && rc=0 || rc=$?
echo "edex-de exit code: $rc"
cat "$OUT/report.json"
[ "$rc" -eq 0 ] || fail=1
grep -q '"canvas_configured": true' "$OUT/report.json" || fail=1
python3 -c "import json; r=json.load(open('$OUT/report.json')); assert r['frames']>=3, r['frames']; assert r['notifications_received']>=1, 'no notification received'; assert r['notification_server'], 'not the notification server'" || fail=1
if grep -E "ERROR|panicked" "$OUT/edex-de.log" | grep -v "eglInitialize" ; then
  echo "errors in the log"; fail=1
fi
[ "$fail" -eq 0 ] && echo "SMOKE OK" || { echo "SMOKE FAILED"; tail -40 "$OUT/edex-de.log"; exit 1; }
