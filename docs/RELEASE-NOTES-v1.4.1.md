# SuperChat v1.4.1

Android feedback release: top bars with real titles, a photo viewer and video player, rebuilt radio buttons, a bigger new-message button, channel and private pictures, a calmer App Lock, and sharing into locked chats. The server API only grows (new endpoints); older apps keep working.

## What changed

* **Top bar titles**: Direct Messages, Channels, Search, Menu. The 3-dots menu has a "Lock" / "Unlock" row below "Later" when App Lock is set (there is no padlock in the bar).
* **Photo viewer and video player** on Android: pinch and double-tap zoom, pan, swipe between pictures, swipe down or Back to close, Share, Save to the gallery; videos play full screen with play/pause, seek, time and mute.
* **Radio buttons** are perfectly round with a centred dot at every density; the **new-message button** is 7 % bigger.
* **Pictures**: channel and group pictures set by owners and admins; private pictures you set for anyone or any conversation (only you see them, on all your devices, in your backup).
* **App Lock**: full-screen PIN screen with its own keypad, one-tap "Lock chat" with Undo, a lock glyph on locked chats while unlocked, an App Lock settings page with an auto-lock timer and the list of locked chats. Locking and unlocking take effect immediately everywhere, without restarting or losing your place.
* **Share** into any chat, locked ones included, without opening it or changing its lock.

## Upgrading

Install the APK for your device over the old app (same signing key). Pull `ghcr.io/uri-travoski/superchat-server:1.4.1` for the server (new picture endpoints; the database migrates by itself). Old apps keep working against a 1.4.1 server; the new picture features need the new app and server.

## Root causes fixed

* **Images did nothing on Android**: the app opened pictures with `xdg-open`, which does not exist on Android, and ignored the error. Pictures, avatars, file thumbnails, the Files tab and search results now open a native full-screen viewer (Kotlin, `ViewerActivity`); videos open `VideoActivity`.
* **Lock/unlock needed a restart**: hiding depended on an "engaged" flag and nothing told the UI when it changed. The lock state now has a generation counter and a `LockChanged` event, every list re-filters at once, and the PIN check no longer holds the lock state while Argon2 runs.
* **Sharing into a locked chat**: the share picker used the lock-filtered list and finished by opening the chat. It now lists every chat, sends with `share_to` (no open, no change to the lock) and returns to the sharing app.
* **Found during emulator testing**: the PIN screen and channel/group pictures had a root with no size (PIN screen offset, pictures never drawn); the share sheet in the viewer lacked clip data, so the preview was blank.

## Tests (final run)

* `cargo test --workspace`: 239 passed, 0 failed, 2 ignored (seed server, ntfy emulator harness). New: `avatars141`, `lock141`, applock unit tests (12).
* `cargo clippy --workspace --all-targets`: 0 errors (style warnings only, as before).
* `tests/e2e/run.sh`: SV-24 full scenario and bots/webhooks/forms/schedule/AI both pass.
* Playwright (Node 24): admin-roles, admin-viewer, admin-dialogs, admin-automation, admin-mobile, admin-updates OK. `admin-smoke.mjs` needs a module that is not installed here and was not run.
* Docker image `superchat:1.4.1` (also tagged `ghcr.io/uri-travoski/superchat-server:1.4.1`), 157 MB.

## Android emulator checks (API 36 emulator, R8-shrunk x86_64 release build)

Run and screenshotted in `docs/screenshots/android-1.4.1-*.png`: tab titles (Regular and font scale 1.3); photo viewer (open, double-tap zoom, pinch, pan, swipe to previous/next, Save to Pictures/SuperChat checked in MediaStore, Share sheet with preview, swipe down closes); video player (play, pause, seek, time, mute, Back); radio buttons at 1x, 2x, 2.625x and font scale 1.3 (zoomed crops in `android-1.4.1-radio-zoom.png`; measured dot-to-ring centre offset 0.0 px except 0.5 px at 2.625x where 12 dp is 31.5 px); FAB; channel photo set by an admin and a group photo shown to everyone; a private photo for another account, shown in details, chat header and messages, then removed; "Lock chat" (row vanishes at once, unread badge updates, Undo toast); 3-dots "Unlock" with a wrong PIN ("Incorrect PIN") then the right PIN (chat returns in place, no restart); 3-dots "Lock"; relock on background with "Immediately" while inside a locked chat (returns to the list, chat hidden); share from another app into a locked chat (chat stays locked and hidden, message delivered, back in the sharing app); App Lock settings page and auto-lock sheet.

Not run on a device: the 5-minute idle and background-over-5-minutes timers (covered by `lock141` with a fake clock), Android's own "Sent" toast text for the sharing app, and two-device privacy (covered by `avatars141`: another user never sees a private photo).

## Artifacts

`dist/superchat-1.4.1-android-arm64.apk` (48.6 MB) and `dist/superchat-1.4.1-android-x86_64.apk` (52.3 MB), versionCode 11 / versionName 1.4.1 (confirmed with aapt2), each with a `.sha256` file.
