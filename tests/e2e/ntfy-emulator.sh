#!/usr/bin/env bash
# N-01 / SV-13 end to end: a real ntfy server (docker), a real server, and the Android emulator.
#
#   1. a test server starts (port 18340) and a real ntfy container (port 18341, `binwiederhier/ntfy`)
#   2. the app on the emulator signs in with a one-time login link and is set to push method ntfy
#      (the app's own URL setting points at the ntfy server through `adb reverse`)
#   3. the app is sent to the background and Android is told to stop it (as a phone does when it sleeps)
#   4. another user sends a message: the server posts a content-free wake-up to ntfy, the app's listener receives it,
#      syncs and posts its own notification. The script checks the notification through `dumpsys notification`.
#
# Needs: the emulator running as emulator-5556 with the debug or release APK installed, docker, curl.
# Ports used: 18340 (server), 18341 (ntfy). Nothing else is touched. Usage: tests/e2e/ntfy-emulator.sh [path-to-apk]
set -euo pipefail
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH"
ADB="/opt/android-sdk/platform-tools/adb -s emulator-5556"
SERVER_PORT=18340
NTFY_PORT=18341
APK="${1:-apps/android/app/build/outputs/apk/debug/app-debug.apk}"
WORK="$(mktemp -d)"
OUT="${SHOT_DIR:-$WORK}"
SERVER_PID=""
cleanup() {
  touch "$WORK/stop"
  [ -n "$SERVER_PID" ] && { sleep 1; kill "$SERVER_PID" 2>/dev/null || true; }
  docker rm -f superchat-ntfy-e2e >/dev/null 2>&1 || true
  $ADB reverse --remove "tcp:$NTFY_PORT" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

echo "== ntfy"
docker rm -f superchat-ntfy-e2e >/dev/null 2>&1 || true
docker run -d --name superchat-ntfy-e2e -p "127.0.0.1:$NTFY_PORT:80" binwiederhier/ntfy:latest serve >/dev/null
for _ in $(seq 1 30); do curl -fs "http://127.0.0.1:$NTFY_PORT/v1/health" >/dev/null && break; sleep 1; done

echo "== server"
ATOMIC_TEST_BIND="127.0.0.1:$SERVER_PORT" URL_FILE="$WORK/url" LINK_FILE="$WORK/link" SEND_FILE="$WORK/send" STOP_FILE="$WORK/stop" \
  cargo test -p atomic-client-core --test ntfy_emulator -- --ignored --nocapture >"$WORK/server.log" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 600); do [ -s "$WORK/link" ] && break; sleep 1; done
[ -s "$WORK/link" ] || { echo "server did not start"; tail -20 "$WORK/server.log"; exit 1; }
LINK="$(sed "s#http://127.0.0.1:$SERVER_PORT#http://10.0.2.2:$SERVER_PORT#" "$WORK/link")"

echo "== app"
$ADB reverse "tcp:$NTFY_PORT" "tcp:$NTFY_PORT"
$ADB install -r "$APK" >/dev/null
$ADB shell pm clear app.superchat >/dev/null
$ADB shell pm grant app.superchat android.permission.POST_NOTIFICATIONS || true
# Test builds refuse plain http unless this marker exists (emulator runs against a dev server only).
$ADB shell am start -n app.superchat/.MainActivity >/dev/null; sleep 4
$ADB shell am force-stop app.superchat
$ADB shell "run-as app.superchat sh -c 'mkdir -p files && touch files/allow_http'"
printf '{"push_choice":"ntfy","ntfy_url":"http://127.0.0.1:%s"}' "$NTFY_PORT" | $ADB shell "run-as app.superchat sh -c 'cat > files/prefs.json'"
$ADB shell am start -a android.intent.action.VIEW -d "$LINK" app.superchat >/dev/null
sleep 12
$ADB exec-out screencap -p > "$OUT/android-1.3-ntfy-signed-in.png"

echo "== background and stop the process"
$ADB shell input keyevent KEYCODE_HOME
sleep 8   # the app registers the device with the server and the listener subscribes
$ADB shell dumpsys notification --noredact | grep -c "pkg=app.superchat" || true
$ADB shell am kill app.superchat || true
sleep 2

echo "== send a message while the server believes the phone is offline"
# The app chose its own random topic and saved it in its preferences.
TOPIC="$($ADB shell "run-as app.superchat cat files/prefs.json" | grep -o '"ntfy_topic": *"[A-Za-z0-9_-]*"' | sed 's/.*: *"//; s/"$//')"
[ -n "$TOPIC" ] || { echo "FAIL: the app did not register an ntfy topic"; exit 1; }
echo "topic: ${TOPIC:0:6}..."
ntfy_events() { curl -fs "http://127.0.0.1:$NTFY_PORT/$TOPIC/json?poll=1&since=all" | grep -c '"event":"message"' || true; }
BEFORE="$(ntfy_events)"
echo offline > "$WORK/send"
for _ in $(seq 1 40); do
  if $ADB shell dumpsys notification --noredact | grep -q "Wake-up test message"; then FOUND=1; break; fi
  sleep 1
done
$ADB exec-out screencap -p > "$OUT/android-1.3-ntfy-notification.png"
AFTER="$(ntfy_events)"
echo "ntfy wake-ups on the topic: $BEFORE -> $AFTER"
# A wake-up never carries message text.
if curl -fs "http://127.0.0.1:$NTFY_PORT/$TOPIC/json?poll=1&since=all" | grep -q "Wake-up test"; then echo "FAIL: message text leaked into ntfy"; exit 1; fi
if [ "${FOUND:-0}" = 1 ] && [ "$AFTER" -gt "$BEFORE" ]; then
  echo "PASS: the server published a wake-up to ntfy, the phone received it, synced and showed the notification"
else
  echo "FAIL: no notification after the wake-up"; $ADB shell dumpsys notification --noredact | grep -i "app.superchat" | head; exit 1
fi
