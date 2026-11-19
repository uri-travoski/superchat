# SuperChat

_SuperChat was called Atomic Chat up to v1.0 (see "Upgrading from Atomic Chat 1.0" in [docs/DEPLOY.md](docs/DEPLOY.md))._

A self-hosted, Slack-style team chat. One small Rust server (Docker), an admin web panel, and native apps for Linux, Windows and
Android (one Slint UI shared by all three), plus a command-line client. Your data stays on your server and in the storage you choose
(S3, Cloudflare R2, Backblaze B2 or Google Drive).

* Accounts are created by an admin only (with a one-time login link you can tap, scan or paste). No public sign-up.
* Channels and group chats are private to their members. Direct messages and notes to yourself are created on demand.
* No end-to-end encryption: TLS in transit, and admins can read messages (needed for the admin chat viewer and search). Backups are
  encrypted with a passphrase.
* Bots and automation: Telegram-style bot API, webhooks, interactive forms, scheduled messages, optional AI search and recaps.
* New in 1.3.2: the server is one standalone `docker-compose.yml` (published image, no Caddy, cloudflared, ntfy or MinIO inside) attached to every release together with `superchat.env.example`; see [docs/RELEASE-NOTES-v1.3.2.md](docs/RELEASE-NOTES-v1.3.2.md).
* In 1.3.1: every app checks GitHub for a newer release (Settings > About > Check for updates, plus an automatic check you can switch off), shows its version clearly, and downloads and installs updates with a checksum check; the admin panel has an Updates card with the upgrade commands; the server image is pulled from ghcr.io; release APKs are always signed with one permanent key; see [docs/RELEASE-NOTES-v1.3.1.md](docs/RELEASE-NOTES-v1.3.1.md).
* In 1.3.0: upload progress and cancel, voice waveform and speed, video recording, PDF previews, profile photos, delete channel, notification sounds, tray icon, drag-to-reorder sidebar, spell check, an encrypted local cache and verified ntfy push; see [docs/RELEASE-NOTES-v1.3.0.md](docs/RELEASE-NOTES-v1.3.0.md).
* In 1.2.0: a Slack-style rebuild of the desktop and phone UI (rail or bottom tabs, sidebar sections, thread panel, Activity, Later with labels), add people to channels from the app, Save for later, a discoverable App Lock and a way back from every screen; see [docs/RELEASE-NOTES-v1.2.0.md](docs/RELEASE-NOTES-v1.2.0.md) and [docs/FEATURE-AUDIT.md](docs/FEATURE-AUDIT.md).
* In 1.1.3: a mobile-native UX pass (one Back stack, bottom sheets, decluttered list and chat, redesigned Settings, working Android QR scan, closable admin dialogs); see [docs/RELEASE-NOTES-v1.1.3.md](docs/RELEASE-NOTES-v1.1.3.md).
* New in 1.1.2: self-service Sign out / Sign out of all devices exist only for users whose admin set Privacy options to Allow (see [docs/RELEASE-NOTES-v1.1.2.md](docs/RELEASE-NOTES-v1.1.2.md)).
* New in 1.1: tap-to-sign-in links with a QR code, several AI providers, App Lock (PIN-hidden conversations), a phone-friendly admin panel. See [docs/RELEASE-NOTES-v1.1.md](docs/RELEASE-NOTES-v1.1.md). Calls (voice/video) are still planned.

![Desktop app](docs/screenshots/desk-1280-light-regular-home.png)

| | | |
|---|---|---|
| ![Dark](docs/screenshots/desk-1280-dark-regular-thread-panel.png) | ![Form](docs/screenshots/desk-1280-light-regular-later.png) | ![Phone](docs/screenshots/android-1.2-home-regular.png) |
| ![App Lock](docs/screenshots/android-1.2-app-lock-hidden.png) | ![Login link page](docs/screenshots/mobile-4b-landing-page.png) | ![Admin on a phone](docs/screenshots/mobile-3-users.png) |

## Install the server

One container, one compose file, nothing built locally (the image `ghcr.io/uri-travoski/superchat-server` is public). Cloudflare Tunnel, a reverse proxy and ntfy are not
included: run those separately ([how](docs/DEPLOY.md#reverse-proxy-tunnel-and-ntfy-run-separately)). Attachments go to S3, Cloudflare R2, Backblaze B2 or Google Drive, chosen in the admin panel.

1. Download the compose file and the settings file from the latest release:
   ```sh
   mkdir superchat && cd superchat
   curl -fLO https://github.com/uri-travoski/superchat/releases/latest/download/docker-compose.yml
   curl -fL -o .env https://github.com/uri-travoski/superchat/releases/latest/download/superchat.env.example
   ```
2. Edit `.env` (at least `SUPERCHAT_PUBLIC_URL`, the address your users will type; every setting is explained in the file).
3. Start it: `docker compose up -d`

Then open `http://<host>:8080/admin`, create the owner account and pick the attachment storage (S3, R2 or B2: [how](docs/DEPLOY.md#attachment-storage-s3-r2-b2)).

**Update**: download the new `docker-compose.yml` (it pins that release's image) and run `docker compose pull && docker compose up -d`; or set
`SUPERCHAT_IMAGE=ghcr.io/uri-travoski/superchat-server:latest` in `.env` once, and then `docker compose pull && docker compose up -d` is the whole update. The admin panel's Overview says
when a new release exists. Upgrading from an older compose file (volume name, renamed variables): [release notes 1.3.2](docs/RELEASE-NOTES-v1.3.2.md).
Developers can build the image from source instead: "Building the image" in [docs/DEPLOY.md](docs/DEPLOY.md).

More in [docs/DEPLOY.md](docs/DEPLOY.md), admin guide in [docs/ADMIN.md](docs/ADMIN.md), bots in [docs/BOTS.md](docs/BOTS.md), the
HTTP API in [docs/API.md](docs/API.md).

## Download the apps

From the [Releases](https://github.com/uri-travoski/superchat/releases) page: Linux AppImage and `.deb`, Windows portable `.exe` and
installer, Android APK (each with a `.sha256` file). Sign in with the server address and the username/password (or one-time link) your admin gave you.
Installed apps check for new releases themselves (Settings > About, version shown there and on the sign-in screen) and can download and install them.

## Features

Status: **tested** = automated test; **done** = built and checked by hand; **partial** = see notes; **not built**.
Full per-item notes are in [STATUS.md](STATUS.md).

| IDs | Feature | Status |
|---|---|---|
| SV-01..SV-07, SV-14 | Accounts, auth and sessions, conversations, message store, edit rule, realtime, attachments, roles | tested |
| SV-08, SV-09 | Search with filters, threads | tested |
| SV-10 | Scheduled messages (survive restarts) | tested |
| SV-11, SV-12 | User groups (`@handle`), favourites/sections/mute/notify level | tested |
| SV-13 | Push: content-free wake-ups via ntfy, FCM, APNs | tested with fakes only |
| SV-15, A-05 | Bots, Telegram-style and native bearer-token API | tested |
| SV-16, A-01, A-02 | Incoming and outgoing (signed, retried) webhooks | tested |
| SV-17, WF-01, A-03, A-04 | Forms (card and modal), bot App Home | tested (server, core); UI done |
| SV-19 | Admin panel: users, conversations, storage, sessions, backups, chat viewer, bots, webhooks, AI, groups, bookmarks | done, Playwright |
| SV-20, AD-06 | Encrypted backups, restore onto a new server, selective restore | tested |
| SV-21, SR-07 | AI search and recap (off by default; text goes to the provider) | tested with a fake provider |
| SV-22 | Docker image, standalone compose file (one service; proxy, tunnel and ntfy run separately) | done |
| SV-24 | End-to-end suite (docker server, MinIO, fake Google Drive, fake ntfy, bots, webhooks, restart) | tested |
| SV-18, CL-01 | Voice/video calls | not built (v1.1) |
| X-01..X-08 | Login, TLS fingerprint pinning, secret storage, sync, offline cache, forced sign-out, emoji picker | done (see limits) |
| M-01..M-13, M-16, M-25..M-29, M-32 | Messaging: DMs, channels, edit/delete, reactions, formatting, mentions, unread, offline outbox | tested + done |
| M-20..M-24 | Forward, quote, drafts, drafts and sent view, schedule send | done |
| M-30, M-31 | Voice messages (record on Android and Linux with `arecord`, inline audio player), video clips (upload, opens system player) | partial |
| C-07, C-09 | Bookmarks (add in the apps and the admin panel), pins | done |
| NV-01..NV-09, SR-01..SR-06 | Switcher, history, sections, search filters, recent and saved searches | done |
| F-01..F-03 | File upload, drag-and-drop (X11/Windows), paste image, previews | partial |
| N-01, N-02 | Android ntfy and FCM wake-ups with local notifications; desktop notifications | partial (ntfy done; FCM compiled in only with a Firebase config) |
| E-01..E-08 | Themes, compact, font size, 12/24 h, share target (Android), launch on login, tray icon | done / partial |
| P-01..P-05 | Presence, DND, profile | done (auto-away missing) |

## Build from source

Needs Rust (stable), Node 22, and for Android the Android SDK + NDK 27, `cargo-ndk`.

```sh
cargo test --workspace                          # unit and integration tests
(cd admin-web && npm ci && npm run build)       # admin panel
cargo build --release -p atomic-server          # server (serves admin-web/dist via ATOMIC_ADMIN_DIR)
cargo build --release -p atomic-desktop         # desktop app: target/release/superchat
tests/e2e/run.sh                                # end-to-end suite (needs docker)
tests/ui/screenshots.sh                         # screenshots (private Xvfb, never your desktop)
```

Packages (output in `dist/`):

```sh
cargo install cargo-deb && cargo deb -p atomic-desktop --output dist/     # .deb
packaging/linux/build-appimage.sh                                         # AppImage
(cd apps/android && ./gradlew assembleRelease)                            # APK (arm64-v8a + x86_64)
docker build -f docker/Dockerfile -t superchat:1.3.2 .                    # server image (developers; installs pull the published one)
```

Windows (`.exe` and NSIS installer) and the published images are built by `.github/workflows/release.yml` when a `v*` tag is pushed.
**Android signing:** installed apps can only update to an APK signed with the same key, so releases use one permanent key. Set the four repository secrets
`ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD` (how: [docs/DEPLOY.md](docs/DEPLOY.md#android-signing));
`release.yml` fails the Android job with a clear error when one is missing instead of using a throw-away key. A local `assembleRelease` is debug-signed, which is fine for testing.

## Layout

`crates/proto` (API types, schema), `crates/server`, `crates/client-core` (all client logic), `crates/cli`, `apps/desktop` (Slint UI, also used
by Android), `apps/android` (Kotlin shell + Rust cdylib), `admin-web` (TypeScript), `deploy` (the release compose file and .env example), `docker` (Dockerfile, developer build), `packaging`, `tests`, `docs`.

## Licence

MIT, see [LICENSE](LICENSE). Colour emoji pictures come from Noto Color Emoji (SIL Open Font License 1.1). The UI uses
[Slint](https://slint.dev) under its royalty-free licence terms for desktop, mobile and web apps.
