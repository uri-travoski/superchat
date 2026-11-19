# SuperChat v1.3.1

Every app now looks for a newer release, says which version it is, and (on Android and the desktop) can download, verify and install the update. The server image is pulled from ghcr instead of built, and the release workflow refuses to publish an unsigned Android APK. Server API: one new admin-only endpoint (`GET /api/v1/admin/updates`); older apps keep working.

## What changed

* **Update check** in the desktop apps, the Android app and the admin panel. Source: `https://api.github.com/repos/uri-travoski/superchat/releases/latest` (one constant; override with `SUPERCHAT_UPDATE_URL` or the hidden `update_url` preference). It runs at start-up when the last check is over an hour old, then every 24 hours, can be switched off under Settings > About, and sends nothing but the app version. After a failed check (offline, GitHub rate limit) the next automatic try waits an hour.
* **Settings > About** shows the version (and build or version code), the server's name and version, the release notes, and Check for updates, Remind me later and Skip this version. Results are plain: up to date, update available, downloading with progress, ready to install, or an error you can act on (can't reach GitHub, GitHub is limiting checks, unexpected answer, checksum mismatch, no checksum published). A banner on Home (phones), an Update button in the desktop rail, the Help menu's "About SuperChat" and the version on the login screen lead there.
* **Android**: downloads the APK, compares its SHA-256 with the release's `.sha256` file, then installs through a PackageInstaller session. If Android needs permission, the "Install unknown apps" page for SuperChat opens and the install continues when you return. Cancelling shows "Installation cancelled" with Install again; a signature mismatch (for example a debug build over a release build) says so and suggests uninstalling first.
* **Desktop**: an AppImage replaces itself in place and restarts, a `.deb` goes to the system's graphical installer, Windows runs the setup (or the portable file opens in its folder). When nothing can be installed automatically the page offers the release link and, for servers, the commands.
* **Admin panel**: the version in the header and footer, and an Updates card with the server's version, the latest release, the upgrade commands (`docker compose pull && docker compose up -d`, pinning with `SUPERCHAT_IMAGE`) and download links. The server fetches the release and caches it for an hour; the browser needs no extra permission and the strict Content Security Policy is unchanged.
* **Server image**: `deploy/docker-compose.yml` uses `ghcr.io/uri-travoski/superchat-server` (public package). Pin a version with `SUPERCHAT_IMAGE=ghcr.io/uri-travoski/superchat-server:1.3.1` in `deploy/.env`. The `build:` block is kept as a comment for building from source. `docs/DEPLOY.md` has the pull and up steps.
* **Release workflow**: the Android job stops with a clear error when any of `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` or `ANDROID_KEY_PASSWORD` is missing, instead of signing with a throw-away key (which would make every later update fail on a signature mismatch). The four secrets are described in `README.md` and `docs/DEPLOY.md`. No keystore was created or read while making this release.
* Version 1.3.1 everywhere; Android versionName 1.3.1 / versionCode 8; Docker image `superchat:1.3.1`.

## Tested

* Unit and fake-server tests for version comparison, asset choice per platform, SHA-256 (good, tampered, missing), cancel, skip and remind, schedule and back-off, rate limit and offline; a Playwright test of the admin Updates card with a mocked release.
* Android emulator against a local fake GitHub serving v9.9.9 with a real, same-key APK: up to date, update available and banner, progress, checksum verified, the unknown-sources page, the system installer prompt (cancelled once, then accepted: the app became 9.9.9 and ran), rate limit, offline, tampered download, missing checksum. Screenshots in `docs/screenshots/android-update/`.
* Desktop screenshots of About (up to date, update available light and dark, rate limit, downloading), the rail button and the phone banner.

## Honest limitations

* The real GitHub API was not exercised by an app: the latest release there is v1.3.0, so there is nothing newer to find. The code is tested against a fake with the same JSON shape.
* A real **signature mismatch** was not run (it needs an APK signed with a different key, and creating a key was out of bounds). The error text and its mapping exist in `Installer.kt`.
* **Windows** and the **.deb installer** path were not run here. The AppImage replacement and the checksum steps are unit-tested.
* After the first in-place update on the emulator the app was signed out once (Keystore key unreadable); two later updates kept the session. Not reproduced.
* Release builds must be signed with the same key as the installed app for Android to accept an update; local builds are debug-signed, so a locally built APK will not update over a CI-built one.
* Calls (CL-01, SV-18) are still not built; FCM and APNs caveats from 1.3.0 are unchanged.
