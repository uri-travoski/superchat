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

@@SIZES@@

## Testing

@@TESTS@@

## Known limits

@@LIMITS@@
