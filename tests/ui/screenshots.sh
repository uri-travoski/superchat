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
OUT="${OUT:-$ROOT/docs/screenshots}"
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
export ATOMIC_AUDIO_NULL=1 ATOMIC_NO_KEYRING=1 ATOMIC_ALLOW_HTTP=1 XDG_CONFIG_HOME="$WORK/config" HOME="$WORK/home"; mkdir -p "$HOME"
sleep 1

SEED_URL_FILE="$WORK/url" SEED_STOP_FILE="$WORK/stop" "$SEED_BIN" --ignored --nocapture >"$WORK/seed.log" 2>&1 &
for _ in $(seq 1 120); do [ -s "$WORK/url" ] && break; sleep 0.5; done
[ -s "$WORK/url" ] || { echo "seed server did not start"; cat "$WORK/seed.log"; exit 1; }
URL="$(cat "$WORK/url")"

# name | size | theme | font step (100/115/130) | open | optional user (default alice; carol has Privacy options Allow)
VARIANTS=(
  # ---- desktop 1280x800: the Slack-style shell ----
  "desk-1280-light-regular-home|1280x800|light|100|chat:general"
  "desk-1280-dark-regular-home|1280x800|dark|100|chat:general"
  "desk-1280-light-large-home|1280x800|light|115|chat:general"
  "desk-1280-dark-large-home|1280x800|dark|115|chat:general"
  "desk-1280-light-regular-hover-toolbar|1280x800|light|100|chat:general||mousemove 800 330"
  "desk-1280-dark-regular-hover-toolbar|1280x800|dark|100|chat:general||mousemove 800 330"
  "desk-1280-light-regular-thread-panel|1280x800|light|100|thread:general"
  "desk-1280-dark-regular-thread-panel|1280x800|dark|100|thread:general"
  "desk-1280-light-regular-details-about|1280x800|light|100|details:about:general"
  "desk-1280-dark-regular-details-members|1280x800|dark|100|details:members:general"
  "desk-1280-light-regular-add-people|1280x800|light|100|addpeople:general"
  "desk-1280-light-regular-activity|1280x800|light|100|tab:activity"
  "desk-1280-dark-regular-activity|1280x800|dark|100|tab:activity"
  "desk-1280-light-regular-later|1280x800|light|100|savedemo:general"
  "desk-1280-dark-regular-later|1280x800|dark|100|savedemo:general"
  "desk-1280-light-regular-labels|1280x800|light|100|labels:general"
  "desk-1280-light-regular-dms|1280x800|light|100|tab:dms"
  "desk-1280-light-regular-search-results|1280x800|light|100|searchq:general:release"
  "desk-1280-dark-regular-search-results|1280x800|dark|100|searchq:general:release"
  "desk-1280-light-regular-search-filter|1280x800|light|100|searchmenu:f-has"
  "desk-1280-light-regular-preferences|1280x800|light|100|settings"
  "desk-1280-dark-regular-preferences-appearance|1280x800|dark|100|settings-page:appearance"
  "desk-1280-light-regular-app-lock|1280x800|light|100|settings-page:lock:general"
  "desk-1280-light-regular-context-menu|1280x800|light|100|convmenu:general"
  "desk-1280-light-regular-message-menu|1280x800|light|100|menu:general"
  "desk-1280-light-regular-help|1280x800|light|100|help:general"
  "desk-1280-light-regular-files-tab|1280x800|light|100|files:design"
  "desk-1280-light-regular-menu-server|1280x800|light|100|chat:general||mousemove 150 72 click 1"
  "desk-1280-light-regular-menu-more|1280x800|light|100|chat:general||mousemove 34 392 click 1"
  "desk-1280-light-regular-menu-avatar|1280x800|light|100|chat:general||mousemove 34 760 click 1"
  "desk-1280-light-regular-later-empty|1280x800|light|100|tab:later"
  "desk-1280-light-regular-menu-sound|1280x800|light|100|soundmenu:general"
  "desk-1280-light-regular-drag-reorder|1280x800|light|100|chat:general||mousemove 150 324 mousedown 1 mousemove 150 316 mousemove 150 300 mousemove 150 280 mousemove 150 262"
  "desk-1280-light-regular-drag-reordered|1280x800|light|100|chat:general||mousemove 150 324 mousedown 1 mousemove 150 316 mousemove 150 300 mousemove 150 280 mousemove 150 262 mouseup 1 mousemove 600 400"
  "desk-1280-light-regular-menu-section|1280x800|light|100|chat:general||mousemove 150 234 click 3"
  "desk-1280-light-regular-media-cards|1280x800|light|100|chat:design"
  "desk-1280-dark-regular-media-cards|1280x800|dark|100|chat:design"
  # ---- desktop 1000x700 ----
  "desk-1000-light-regular-home|1000x700|light|100|chat:general"
  "desk-1000-dark-regular-home|1000x700|dark|100|chat:general"
  "desk-1000-light-large-home|1000x700|light|115|chat:general"
  "desk-1000-dark-large-home|1000x700|dark|115|chat:general"
  "desk-1000-light-regular-thread-panel|1000x700|light|100|thread:general"
  "desk-1000-light-regular-details-members|1000x700|light|100|details:members:general"
  "desk-1000-dark-regular-later|1000x700|dark|100|savedemo:general"
  "desk-1000-light-regular-search-results|1000x700|light|100|searchq:general:release"
  "desk-1000-light-regular-form|1000x700|light|100|form:general"
  "desk-1000-light-regular-app-home|1000x700|light|100|home:Expense Bot"
  "desk-1000-light-regular-schedule|1000x700|light|100|schedule:general"
  "desk-1000-light-regular-drafts|1000x700|light|100|drafts"
  "desk-1000-dark-regular-bookmark|1000x700|dark|100|bookmark:general"
  "desk-1000-light-regular-threads|1000x700|light|100|threads"
  "desk-1000-dark-regular-switcher|1000x700|dark|100|switcher"
  # ---- phone layout 360x640 (the same UI at phone width; the emulator shots are android-1.2-*) ----
  "phone-light-regular-home|360x640|light|100|none"
  "phone-dark-regular-home|360x640|dark|100|none"
  "phone-light-large-home|360x640|light|115|none"
  "phone-dark-large-home|360x640|dark|115|none"
  "phone-light-regular-chat|360x640|light|100|chat:design"
  "phone-dark-large-chat|360x640|dark|115|chat:design"
  "phone-light-regular-dms|360x640|light|100|tab:dms"
  "phone-light-regular-activity|360x640|light|100|tab:activity"
  "phone-dark-regular-you|360x640|dark|100|tab:you"
  "phone-light-regular-later|360x640|light|100|savedemo:general"
  "phone-light-regular-msg-sheet|360x640|light|100|msgsheet:general"
  "phone-dark-large-msg-sheet|360x640|dark|115|msgsheet:general"
  "phone-light-regular-details|360x640|light|100|details::general"
  "phone-dark-large-details|360x640|dark|115|details::general"
  "phone-light-regular-details-members|360x640|light|100|details:members:general"
  "phone-light-regular-details-files|360x640|light|100|details:files:design"
  "phone-light-regular-add-people|360x640|light|100|addpeople:general"
  "phone-light-regular-thread|360x640|light|100|thread:general"
  "phone-dark-large-thread|360x640|dark|115|thread:general"
  "phone-light-regular-search|360x640|light|100|searchq:general:release"
  "phone-light-regular-settings|360x640|light|100|settings"
  "phone-dark-large-settings|360x640|dark|115|settings"
  "phone-light-regular-settings-privacy|360x640|light|100|settings-page:privacy|carol"
  "phone-light-regular-app-lock|360x640|light|100|settings-page:lock:general"
  "phone-dark-large-app-lock|360x640|dark|115|settings-page:lock:general"
  "phone-light-regular-lock-screen|360x640|light|100|lockpin:design"
  "phone-dark-regular-lock-list-unlocked|360x640|dark|100|lockopen:design"
  "phone-light-regular-conv-menu|360x640|light|100|convmenu:general"
  "phone-light-regular-form|360x640|light|100|form:general"
  "phone-light-regular-threads|360x640|light|100|threads"
  "phone-light-regular-drafts|360x640|light|100|drafts"
  "phone-light-large-label-picker|360x640|light|130|labelpick:general"
  "phone-light-large-you|360x640|light|130|tab:you"
  "phone-light-large-settings|360x640|light|130|settings"
)
n=0
for v in "${VARIANTS[@]}"; do
  IFS='|' read -r name size theme font open user acts <<<"$v"
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
  WID="$(xdotool search --onlyvisible --name "SuperChat" | tail -1)"; [ -n "$WID" ] || WID="$(xdotool search --name "SuperChat" | tail -1)"
  xdotool windowsize "$WID" "$W" "$H"; xdotool windowmove "$WID" 0 0
  sleep 3
  xdotool windowsize "$WID" "$W" "$H"; xdotool windowmove "$WID" 0 0
  # The window manager-less Xvfb sometimes ignores the first resize: retry until the geometry matches.
  for _ in $(seq 1 10); do
    GEO="$(xdotool getwindowgeometry "$WID" | awk '/Geometry/{print $2}')"
    [ -n "${KEEP_LOG:-}" ] && echo "geo=$GEO want=${W}x${H} wins=$(xdotool search --name SuperChat | tr '\n' ' ')"
    [ "$GEO" = "${W}x${H}" ] && break
    xdotool windowsize "$WID" "$W" "$H"; xdotool windowmove "$WID" 0 0; sleep 1
  done
  sleep 3
  # Optional pointer / keyboard steps (xdotool commands separated by ';') before the shot, e.g. hover a message.
  if [ -n "${acts:-}" ]; then
    IFS=';' read -ra STEPS <<<"$acts"
    for st in "${STEPS[@]}"; do eval "xdotool $st"; sleep 0.8; done
  fi
  [ -n "${KEEP_LOG:-}" ] && ls -la "$DATA/client/files" 2>&1 | tail -3 || true
  import -window root -crop "${W}x${H}+0+0" "$OUT/$name.png"
  echo "wrote $OUT/$name.png"
  [ -n "${KEEP_LOG:-}" ] && grep IMGDBG "$WORK/app-$n.log" | tail -3 || true
  kill "$APP_PID" 2>/dev/null || true; wait "$APP_PID" 2>/dev/null || true; APP_PID=
  unset ATOMIC_UI_OPEN
done
echo "done: $n screenshots in $OUT"
