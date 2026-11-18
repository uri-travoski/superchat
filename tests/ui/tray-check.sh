#!/usr/bin/env bash
# E-08: checks the tray icon end to end without a desktop: a private Xvfb + dbus session, a fake StatusNotifierWatcher
# (tests/ui/fake_tray_watcher.py) and the real app started with --minimised. Asserts that the app registers a tray item,
# shows no window (start minimised goes to the tray) and exposes the menu Open / Status / Pause notifications / Quit.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${BIN:-$ROOT/target/debug/superchat}"
[ -x "$BIN" ] || (cd "$ROOT" && cargo build -p atomic-desktop -q)
W="$(mktemp -d)"; mkdir -p "$W/home"
cleanup() { kill ${APP:-} ${WP:-} ${DP:-} ${XP:-} 2>/dev/null || true; rm -rf "$W"; }
trap cleanup EXIT
Xvfb :97 -screen 0 1280x900x24 -nolisten tcp >/dev/null 2>&1 & XP=$!
dbus-daemon --session --nofork --print-address=3 3>"$W/addr" >/dev/null 2>&1 & DP=$!
sleep 1
export DBUS_SESSION_BUS_ADDRESS="$(head -1 "$W/addr")" DISPLAY=:97
python3 "$(dirname "$0")/fake_tray_watcher.py" "$W/reg" & WP=$!
sleep 1
HOME="$W/home" XDG_CONFIG_HOME="$W/cfg" ATOMIC_NO_KEYRING=1 ATOMIC_AUDIO_NULL=1 "$BIN" --minimised >"$W/app.log" 2>&1 & APP=$!
for _ in $(seq 1 30); do [ -s "$W/reg" ] && break; sleep 0.3; done
[ -s "$W/reg" ] || { echo "FAIL: the app did not register a tray item"; exit 1; }
read -r SENDER _ < "$W/reg"
LABELS="$(gdbus call --session --dest "$SENDER" --object-path /MenuBar --method com.canonical.dbusmenu.GetLayout -- 0 -1 '@as []' | tr -d '\n' | grep -o "'label': <'[^']*'>")"
for want in "Open SuperChat" "Status: Active" "Pause notifications" "Quit SuperChat"; do
  echo "$LABELS" | grep -q "<'$want'>" || { echo "FAIL: tray menu lacks '$want'"; echo "$LABELS"; exit 1; }
done
if [ -n "$(xdotool search --onlyvisible --name SuperChat 2>/dev/null)" ]; then echo "FAIL: a window is visible after start minimised"; exit 1; fi
echo "tray ok: registered, menu complete, no window shown"
