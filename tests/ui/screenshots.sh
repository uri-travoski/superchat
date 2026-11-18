#!/usr/bin/env bash
# Regenerates docs/screenshots/*.png: the real desktop app against a real (in-process) server that is
# seeded with a demo team. Runs only on a private Xvfb display with its own dbus session, never on
# a real desktop. Variants: Regular and Large text, light and dark, phone 360x640 and desktop 1000x700.
#
#   tests/ui/screenshots.sh            # all variants
#   SHOTS="desk-light-regular-chat" tests/ui/screenshots.sh   # one
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH" CC=/usr/bin/cc CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=/usr/bin/cc
OUT="$ROOT/docs/screenshots"
WORK="$(mktemp -d)"
DISPLAY_NUM="${DISPLAY_NUM:-99}"
mkdir -p "$OUT"
cd "$ROOT"
cargo build -p atomic-desktop -q
cargo test -p atomic-client-core --test seed_demo --no-run -q
SEED_BIN="$(ls -t target/debug/deps/seed_demo-* | grep -v '\.d$' | head -1)"

cleanup() {
  touch "$WORK/stop" 2>/dev/null || true
  [ -n "${APP_PID:-}" ] && kill "$APP_PID" 2>/dev/null || true
  [ -n "${XVFB_PID:-}" ] && kill "$XVFB_PID" 2>/dev/null || true
  [ -n "${DBUS_PID:-}" ] && kill "$DBUS_PID" 2>/dev/null || true
  sleep 1; rm -rf "$WORK"
}
trap cleanup EXIT

Xvfb ":$DISPLAY_NUM" -screen 0 1280x900x24 -nolisten tcp >/dev/null 2>&1 &
XVFB_PID=$!
export DISPLAY=":$DISPLAY_NUM"
dbus-daemon --session --nofork --print-address=3 3>"$WORK/dbus-addr" >/dev/null 2>&1 &
DBUS_PID=$!
for _ in $(seq 1 20); do [ -s "$WORK/dbus-addr" ] && break; sleep 0.2; done
export DBUS_SESSION_BUS_ADDRESS="$(head -1 "$WORK/dbus-addr")"
# Never touch the user's keyring or desktop session.
export ATOMIC_NO_KEYRING=1 ATOMIC_ALLOW_HTTP=1 XDG_CONFIG_HOME="$WORK/config" HOME="$WORK/home"; mkdir -p "$HOME"
sleep 1

SEED_URL_FILE="$WORK/url" SEED_STOP_FILE="$WORK/stop" "$SEED_BIN" --ignored --nocapture >"$WORK/seed.log" 2>&1 &
for _ in $(seq 1 120); do [ -s "$WORK/url" ] && break; sleep 0.5; done
[ -s "$WORK/url" ] || { echo "seed server did not start"; cat "$WORK/seed.log"; exit 1; }
URL="$(cat "$WORK/url")"

# name | size | theme | font step (100/115/130) | open | optional user (default alice; carol has Privacy options Allow)
VARIANTS=(
  "desk-light-regular-chat|1000x700|light|100|chat:general"
  "desk-dark-regular-chat|1000x700|dark|100|chat:general"
  "desk-light-large-chat|1000x700|light|115|chat:general"
  "desk-dark-large-chat|1000x700|dark|115|chat:general"
  "phone-light-regular-list|360x640|light|100|none"
  "phone-dark-large-list|360x640|dark|115|none"
  "phone-dark-regular-chat|360x640|dark|100|chat:design"
  "phone-light-large-chat|360x640|light|115|chat:design"
  "desk-light-regular-settings|1000x700|light|100|settings"
  "desk-light-regular-settings-no-signout|1000x700|light|100|settings-bottom"
  "desk-light-large-settings-no-signout|1000x700|light|115|settings-bottom"
  "desk-light-regular-settings-signout|1000x700|light|100|settings-bottom|carol"
  "desk-light-large-settings-signout|1000x700|light|115|settings-bottom|carol"
  "desk-dark-regular-search|1000x700|dark|100|search:general:release"
  "desk-light-regular-threads|1000x700|light|100|threads"
  "desk-dark-regular-switcher|1000x700|dark|100|switcher"
  "desk-light-regular-thread|1000x700|light|100|thread:general"
  "desk-light-regular-pins|1000x700|light|100|pins:general"
  "desk-light-regular-files|1000x700|light|100|files:design"
  "phone-light-regular-settings|360x640|light|100|settings"
  "desk-light-regular-form|1000x700|light|100|form:general"
  "desk-dark-regular-form|1000x700|dark|100|form:general"
  "phone-light-regular-form|360x640|light|100|form:general"
  "desk-light-regular-home|1000x700|light|100|home:Expense Bot"
  "desk-light-regular-schedule|1000x700|light|100|schedule:general"
  "desk-light-regular-drafts|1000x700|light|100|drafts"
  "desk-dark-regular-bookmark|1000x700|dark|100|bookmark:general"
  "desk-light-regular-ai-search|1000x700|light|100|search:general:"
  "phone-light-regular-lock-screen|360x640|light|100|lockpin:design"
  "phone-dark-regular-lock-screen|360x640|dark|100|lockpin:design"
  "phone-light-large-lock-screen|360x640|light|115|lockpin:design"
  "phone-dark-large-lock-screen|360x640|dark|115|lockpin:design"
  "phone-light-regular-lock-list-locked|360x640|light|100|locklist:design"
  "phone-dark-regular-lock-list-locked|360x640|dark|100|locklist:design"
  "phone-light-large-lock-list-locked|360x640|light|115|locklist:design"
  "phone-dark-large-lock-list-locked|360x640|dark|115|locklist:design"
  "phone-light-regular-lock-list-unlocked|360x640|light|100|lockopen:design"
  "phone-dark-large-lock-list-unlocked|360x640|dark|115|lockopen:design"
)
n=0
for v in "${VARIANTS[@]}"; do
  IFS='|' read -r name size theme font open user <<<"$v"
  if [ -n "${SHOTS:-}" ] && ! grep -qw -- "$name" <<<"$SHOTS"; then continue; fi
  n=$((n+1))
  DATA="$WORK/data-$n"; mkdir -p "$DATA"
  printf '{"theme":"%s","font_scale":%s}' "$theme" "$font" >"$DATA/prefs.json"
  W="${size%x*}"; H="${size#*x}"
  export ATOMIC_DATA_DIR="$DATA" ATOMIC_AUTOLOGIN_SERVER="$URL" ATOMIC_AUTOLOGIN_USER="${user:-alice}" ATOMIC_AUTOLOGIN_PASS=password-123
  unset ATOMIC_UI_OPEN
  [ "$open" = none ] || export ATOMIC_UI_OPEN="$open"
  "$ROOT/target/debug/superchat" >"$WORK/app-$n.log" 2>&1 &
  APP_PID=$!
  for _ in $(seq 1 40); do xdotool search --name "SuperChat" >/dev/null 2>&1 && break; sleep 0.25; done
  WID="$(xdotool search --name "SuperChat" | head -1)"
  xdotool windowsize "$WID" "$W" "$H"; xdotool windowmove "$WID" 0 0
  sleep 3
  xdotool windowsize "$WID" "$W" "$H"; xdotool windowmove "$WID" 0 0
  sleep 4
  [ -n "${KEEP_LOG:-}" ] && ls -la "$DATA/client/files" 2>&1 | tail -3 || true
  import -window root -crop "${W}x${H}+0+0" "$OUT/$name.png"
  echo "wrote $OUT/$name.png"
  [ -n "${KEEP_LOG:-}" ] && grep IMGDBG "$WORK/app-$n.log" | tail -3 || true
  kill "$APP_PID" 2>/dev/null || true; wait "$APP_PID" 2>/dev/null || true; APP_PID=
  unset ATOMIC_UI_OPEN
done
echo "done: $n screenshots in $OUT"
