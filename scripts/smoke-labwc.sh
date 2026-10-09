#!/usr/bin/env bash
# Headless smoke test on labwc: the session as RustOS runs it (labwc -C <eDEX's generated
# config> -s 'edex-de run --wm labwc'). Checks the foreign-toplevel backend: a foot window is
# tiled (maximized into the terminal slot), shows up as a tab, minimizes and comes back.
# Usage: scripts/smoke-labwc.sh [path/to/edex-de] [out-dir]
set -euo pipefail

BIN=$(realpath "${1:-target/debug/edex-de}")
OUT=$(realpath -m "${2:-target/smoke-labwc}")
SECS=${SMOKE_SECS:-25}
mkdir -p "$OUT"
export XDG_RUNTIME_DIR=$(mktemp -d /tmp/edex-labwc.XXXXXX)
chmod 700 "$XDG_RUNTIME_DIR"
export XDG_CONFIG_HOME="$XDG_RUNTIME_DIR/config"
export XDG_STATE_HOME="$XDG_RUNTIME_DIR/state"
export EDEX_SHARE_DIR="${EDEX_SHARE_DIR:-$(cd "$(dirname "$0")/.." && pwd)/share}"
export RUST_LOG=${RUST_LOG:-info}
export PATH="$(dirname "$BIN"):$PATH"

cleanup() {
  pkill -P $$ >/dev/null 2>&1 || true
  [ -n "${DBUS_PID:-}" ] && kill "$DBUS_PID" 2>/dev/null || true
  [ -n "${LABWC_PID_:-}" ] && kill "$LABWC_PID_" 2>/dev/null || true
}
trap cleanup EXIT

DBUS_ADDR_FILE="$XDG_RUNTIME_DIR/dbus-addr"
dbus-daemon --session --fork --print-address 3 --print-pid 4 3> "$DBUS_ADDR_FILE" 4> "$XDG_RUNTIME_DIR/dbus-pid"
DBUS_PID=$(cat "$XDG_RUNTIME_DIR/dbus-pid")
export DBUS_SESSION_BUS_ADDRESS=$(cat "$DBUS_ADDR_FILE")

# The configuration the settings export writes (key bindings, the tiling window rule).
"$BIN" labwc-config "$XDG_RUNTIME_DIR/labwc" >/dev/null
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 WLR_HEADLESS_OUTPUTS=1 \
  labwc -C "$XDG_RUNTIME_DIR/labwc" -s "sh -c 'edex-de run --wm labwc --smoke-test $SECS > $OUT/report.json 2> $OUT/edex-de.log'" \
  > "$OUT/labwc.log" 2>&1 &
LABWC_PID_=$!

IPC="$XDG_RUNTIME_DIR/edex-de/ipc.sock"
for _ in $(seq 1 100); do
  [ -S "$IPC" ] && break
  sleep 0.2
done
[ -S "$IPC" ] || { echo "ipc socket never appeared"; cat "$OUT/labwc.log" "$OUT/edex-de.log" 2>/dev/null; exit 1; }
WD=$(ls "$XDG_RUNTIME_DIR" | grep -m1 -E '^wayland-[0-9]+$')
export WAYLAND_DISPLAY=$WD
sleep 2

fail=0
step() { echo "==> $*"; }
state() { "$BIN" ipc state > "$OUT/$1.json"; }
check() { python3 - "$OUT/$1.json" "$2" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))["data"]
ok = eval(sys.argv[2], {}, {"d": d, "w": d["wm"], "wins": d["wm"]["windows"]})
print("   ", sys.argv[2], "->", bool(ok))
sys.exit(0 if ok else 1)
PY
}

step "backend"
state wm
check wm 'w["kind"] == "labwc" and w["connected"] and w["name"] == "labwc"' || fail=1

step "foot tiles into the terminal slot"
foot >/dev/null 2>&1 &
sleep 3
state tiled
check tiled 'len(wins) == 1 and wins[0]["class"] == "foot" and not wins[0]["floating"] and w["apps_cover_terminal"]' || fail=1
grim "$OUT/01-tiled.png" 2>/dev/null || true
ID=$(python3 -c "import json;print(json.load(open('$OUT/tiled.json'))['data']['wm']['windows'][0]['id'])")

step "maximize (full width, side panels step aside)"
"$BIN" ipc action maximize >/dev/null || fail=1
sleep 1
state wide
check wide 'wins[0]["maximized"]' || fail=1
"$BIN" ipc action maximize >/dev/null || fail=1

step "minimize into a tab, and back"
"$BIN" ipc action minimize >/dev/null || fail=1
sleep 1
state min
check min 'wins[0]["minimized"] and not w["apps_cover_terminal"]' || fail=1
"$BIN" ipc focus terminal >/dev/null || fail=1

step "close"
labwc_ok=1
pkill -x foot || true
sleep 1
state closed
check closed 'len(wins) == 0' || fail=1

step "overlays"
for ov in launcher settings power; do
  "$BIN" ipc show "$ov" | grep -q '"ok":true' || fail=1
  sleep 0.5
  "$BIN" ipc state | grep -q "\"overlay\":\"$ov\"" || { echo "overlay $ov not open"; fail=1; }
  "$BIN" ipc hide "$ov" >/dev/null || fail=1
done

wait_for_exit() { for _ in $(seq 1 $((SECS * 5))); do kill -0 "$LABWC_PID_" 2>/dev/null || return 0; [ -s "$OUT/report.json" ] && return 0; sleep 0.2; done; }
wait_for_exit
python3 -c "import json; r=json.load(open('$OUT/report.json')); assert r['frames'] >= 3, r['frames']; print('frames:', r['frames'])" || fail=1
[ $fail -eq 0 ] && echo "SMOKE OK" || { echo "SMOKE FAILED"; tail -30 "$OUT/edex-de.log"; exit 1; }
