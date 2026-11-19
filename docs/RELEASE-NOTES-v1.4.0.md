# SuperChat v1.4.0

A simpler phone app, file-type icons everywhere, and smaller Android downloads. The desktop layout is unchanged apart from the file icons. The server API is unchanged.

## What changed

* **Phone top bar**: the app name (or the configured server name) in the accent colour, the App Lock lock icon only when a PIN is set, and a 3-dots menu at the far right. The server avatar, the pencil, the filter icon and the "Jump to or search" bar are gone.
* **3-dots menu**: Threads, Drafts & sent, Later, Sort (Recent, Name A-Z), Filter (Unread only, Hide muted). The "Shortcuts" section is gone from the lists.
* **Four tabs: DMs, Channels, Search, Menu.** DMs is the default and the last tab is remembered. Each tab has its own Back stack; Back goes back inside the tab, then to DMs, then leaves the app. Menu: your avatar, name and status (tap for the You page), Activity (with the unread badge) and Settings.
* **New message** is a white + on an accent circle on DMs and Channels.
* **Pin to top** for DMs and Channels (long-press menu, conversation details). Pinned rows are at the top, no header, in your order (long-press and drag, or Move up / Move down), saved on the server and synced to your other devices. Sort and Filter only apply to unpinned rows; pinned rows are always shown.
* **File-type icons** (Word, PDF, Excel, PowerPoint, archives, text, audio, video, unknown) in messages, the Files tab, the global Files page, attach previews and search results.
* **Removed on request**: in-app video recording on Android (videos can still be attached and played; voice messages are unchanged) and screen-reader (accessibility) support in the apps.
* **Android APKs per ABI**: `superchat-1.4.0-android-arm64.apk` (phones and TV boxes) and `superchat-1.4.0-android-x86_64.apk` (emulators), each with a `.sha256`. The app's update check downloads the one that matches the device; an older release's single `-android.apk` still works as the fallback.

## Upgrading

Install the APK for your device over the old app (same signing key). The server needs no change; pulling the 1.4.0 image is optional (`SUPERCHAT_IMAGE=ghcr.io/uri-travoski/superchat-server:1.4.0`). Old apps keep working against a 1.4.0 server. Pinned conversations are your existing favourites, in your existing order.

## Sizes (APK) and build

| | 1.3.2 (one universal APK) | 1.4.0 arm64-v8a | 1.4.0 x86_64 |
|---|---|---|---|
| APK | 104,112,132 B (99.3 MB) | 41,124,747 B (39.2 MB) | 46,875,618 B (44.7 MB) |
| Rust library (`libatomic_android.so`) | 48,660,920 B (arm64) + 51,827,832 B (x86_64) | 40,219,616 B | 45,948,104 B |
| Kotlin code (`classes.dex`, R8 off / on) | 7,707,300 B | 988,916 B | 988,916 B |
| `resources.arsc` + `res/` (resource shrinking off / on) | about 8.2 MB with the dex | about 1.4 MB with the dex | same |

* **What gave what** (arm64): splitting by ABI removes the other architecture's library (about 49-52 MB); R8 and resource shrinking cut the Kotlin and resource part from about 8.2 MB to 1.4 MB (the CameraX video capture dependency went too); the `release-android` Cargo profile (opt-level z, fat LTO, one codegen unit, strip) took the arm64 library from 48.7 MB to 40.2 MB. versionCode is 10 for both APKs (checked with `aapt2 dump badging`).
* The libraries are stored uncompressed in the APK (so they are mapped straight from the file and nothing is extracted on install); compressing them would shrink the download to roughly a third but double the space used on the phone. Not done.
* **Accessibility removal**: dropping Slint's `accessibility` feature changed the Android library by 16 bytes (the Android backend does not use AccessKit), so the size win on Android is nil; it removes the AccessKit and AT-SPI crates from the desktop build (see `Cargo.lock`) and the `accessible-*` properties from the UI as requested. Touch targets are unchanged.
* Both APKs are debug-signed when built locally; `release.yml` signs them with the permanent key.

## Testing

* `cargo test --workspace --no-fail-fast`: 231 passed, 0 failed (2 ignored: the seed server and the ntfy emulator harness). New: `client-core/tests/pins14.rs` (pin order, sync to a second device, sort and filter leave pinned alone, move and drag, DMs and channels share one order), pin unit tests in `client/pins.rs`, ABI asset selection in `update.rs`, per-tab back stacks in `nav.rs`.
* `tests/e2e/run.sh` (SV-24 full scenario and bots) against the image `superchat:1.4.0`: pass. Playwright `admin-roles`, `admin-viewer`, `admin-dialogs`, `admin-automation`, `admin-mobile`, `admin-updates`: pass. `cargo clippy --workspace --all-targets`: 0 errors.
* Docker image `superchat:1.4.0`, also tagged `ghcr.io/uri-travoski/superchat-server:1.4.0` (built, not pushed).
* **Emulator** (emulator-5556, x86_64 R8-shrunk release APK, debug-signed, seeded server on :18300): login by link and by password; DMs and Channels lists; top bar (accent name, 3-dots; lock icon appears once a PIN is set); dots menu with Sort (Recent, Name A-Z) and Filter (Unread only); bottom bar with four tabs and Back (page, tab, DMs, exit); Menu > avatar > You page, Settings, Activity; Search tab (focused field, Files results with DOCX/WAV/MP4 icons); the + button; Pin to top, Unpin, Move up / Move down, long-press-and-drag reorder (handle and drop line shown), a pinned row staying visible under "Unread only" and Name sort, pins restored after a fresh sign-in (synced from the server); file icons for docx, xlsx, pptx, zip, txt, csv, md, wav, mp4 in messages, search and the Files list; voice message recorded and sent; no "Record video" entry in the attach sheet; QR scan screen opens (the emulator has no camera, so CameraX reports "no camera"), "Choose image" decoded a QR picture with ZXing under R8; a message notification arrived while the app was in the background and opened the conversation; last tab remembered across a restart; font scale 1.3 for DMs, Channels, dots menu, Menu, You, Activity, Settings and Search; update check against the fake release server picked `superchat-9.9.9-android-x86_64.apk` and its `.sha256` (server log), and "Up to date" for the same version. Screenshots: `docs/screenshots/android-1.4-*.png`; the desktop-rendered phone layout is in `docs/screenshots/phone-*.png` (regenerated) and the desktop file icons in `desk-1280-*-media-cards.png`, `files-tab`, `search-results`.

## Known limits

* The arm64-v8a APK was built and inspected (`aapt2`, contents) but not run: the only emulator is x86_64. The Kotlin code and R8 rules are the same for both.
* The drag reorder was driven with synthetic touch events on the emulator; the Move up / Move down menu items are the fallback if a real finger drag feels wrong on some phone.
* **Phones on 1.3.x do not see 1.4.0 as an update.** Their update check looks for `superchat-<v>-android.apk`; a 1.4.0 release with only the two ABI names has no such file for them. They must install the new APK by hand (or the release can also carry a universal `-android.apk`, which this release does not). From 1.4.0 on the check follows the ABI.
* Pinned rows ignore Filter and Sort by decision (docs/DECISIONS.md). Unread filtering on the desktop sidebar is unchanged.
* `tests/e2e/ui/admin-smoke.mjs` cannot run here (missing module in this machine's Playwright folder); it did not run before this release either.
* The `-android-arm64.apk` / `-android-x86_64.apk` build, signing and upload only happen in `release.yml` after a tag is pushed; nothing was pushed or tagged.
