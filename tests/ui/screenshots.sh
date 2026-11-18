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
  # ---- phone 360x640 ----
  "phone-light-regular-list|360x640|light|100|none"
  "phone-dark-regular-list|360x640|dark|100|none"
  "phone-light-large-list|360x640|light|115|none"
  "phone-dark-large-list|360x640|dark|115|none"
  "phone-dark-regular-list-menu|360x640|dark|100|sheet:overflow"
  "phone-light-regular-list-menu|360x640|light|100|sheet:overflow"
  "phone-dark-regular-chat|360x640|dark|100|chat:design"
  "phone-light-regular-chat|360x640|light|100|chat:design"
  "phone-light-large-chat|360x640|light|115|chat:design"
  "phone-dark-large-chat|360x640|dark|115|chat:design"
  "phone-light-regular-chat-menu|360x640|light|100|sheet:chat-menu:design"
  "phone-dark-regular-msg-sheet|360x640|dark|100|msgsheet:design"
  "phone-light-large-msg-sheet|360x640|light|115|msgsheet:design"
  "phone-dark-regular-conv-options|360x640|dark|100|sheet:conv:design"
  "phone-light-regular-chat-info-members|360x640|light|100|info:design:0"
  "phone-dark-regular-chat-info-files|360x640|dark|100|info:design:1"
  "phone-light-large-chat-info-files|360x640|light|115|info:design:1"
  "phone-light-regular-chat-info-pins|360x640|light|100|info:general:2"
  "phone-dark-regular-settings|360x640|dark|100|settings"
  "phone-light-regular-settings|360x640|light|100|settings"
  "phone-light-large-settings|360x640|light|115|settings"
  "phone-dark-large-settings|360x640|dark|115|settings"
  "phone-light-regular-settings-signout|360x640|light|100|settings|carol"
  "phone-light-regular-settings-appearance|360x640|light|100|settings-page:appearance"
  "phone-dark-large-settings-appearance|360x640|dark|115|settings-page:appearance"
  "phone-light-regular-settings-notifications|360x640|light|100|settings-page:notif"
  "phone-light-regular-settings-privacy|360x640|light|100|settings-page:privacy|carol"
  "phone-light-regular-settings-lock|360x640|light|100|settings-page:lock"
  "phone-dark-regular-settings-devices|360x640|dark|100|settings-page:devices|carol"
  "phone-light-regular-settings-about|360x640|light|100|settings-page:about"
  "phone-light-regular-sheet-theme|360x640|light|100|settings-page:appearance"
  "phone-light-regular-search|360x640|light|100|search:general:release"
  "phone-dark-large-search|360x640|dark|115|search:general:release"
  "phone-light-regular-new-chat|360x640|light|100|people"
  "phone-light-regular-thread|360x640|light|100|thread:general"
  "phone-dark-large-thread|360x640|dark|115|thread:general"
  "phone-light-regular-threads|360x640|light|100|threads"
  "phone-light-regular-drafts|360x640|light|100|drafts"
  "phone-light-regular-form|360x640|light|100|form:general"
  "phone-dark-large-form|360x640|dark|115|form:general"
  "phone-light-regular-schedule|360x640|light|100|schedule:general"
  "phone-light-regular-bookmark|360x640|light|100|bookmark:general"
  "phone-light-regular-lock-screen|360x640|light|100|lockpin:design"
  "phone-dark-large-lock-screen|360x640|dark|115|lockpin:design"
  "phone-light-regular-lock-list-locked|360x640|light|100|locklist:design"
  "phone-dark-regular-lock-list-unlocked|360x640|dark|100|lockopen:design"
  # ---- desktop 1000x700 ----
  "desk-light-regular-chat|1000x700|light|100|chat:general"
  "desk-dark-regular-chat|1000x700|dark|100|chat:general"
  "desk-light-large-chat|1000x700|light|115|chat:general"
  "desk-dark-large-chat|1000x700|dark|115|chat:general"
  "desk-light-regular-settings|1000x700|light|100|settings"
  "desk-dark-regular-settings-appearance|1000x700|dark|100|settings-page:appearance"
  "desk-light-regular-chat-menu|1000x700|light|100|sheet:chat-menu:general"
  "desk-dark-regular-msg-actions|1000x700|dark|100|msgsheet:general"
  "desk-light-regular-chat-info|1000x700|light|100|info:design:1"
  "desk-dark-regular-search|1000x700|dark|100|search:general:release"
  "desk-light-regular-threads|1000x700|light|100|threads"
  "desk-dark-regular-switcher|1000x700|dark|100|switcher"
  "desk-light-regular-thread|1000x700|light|100|thread:general"
  "desk-light-regular-form|1000x700|light|100|form:general"
  "desk-light-regular-home|1000x700|light|100|home:Expense Bot"
  "desk-light-regular-schedule|1000x700|light|100|schedule:general"
  "desk-light-regular-drafts|1000x700|light|100|drafts"
  "desk-dark-regular-bookmark|1000x700|dark|100|bookmark:general"
  "desk-light-regular-ai-search|1000x700|light|100|search:general:"
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
