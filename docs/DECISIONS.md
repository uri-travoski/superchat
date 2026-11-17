# Decisions

Dates are 2026-11-17 unless stated. Newest decisions at the bottom of each section.

## Phase 0 spike: Slint vs Tauri (PRD 4.2 gate)

**Decision: Slint stays.** Tauri 2 + Svelte was not needed.

Tested on AVD `timeline-phone` (emulator-5556 only, x86_64, API 34-class image, Gboard):

| Gate item | Result | Evidence |
|---|---|---|
| Builds for Android with `cargo-ndk` + `android-activity` and runs as an APK | pass | Gradle project in `apps/android` runs `cargo ndk` then packages the `.so` plus a small Kotlin shell |
| Soft-keyboard text input (IME) | pass | typed "Hello Atomic 123" into a Slint `TextInput`; `mInputShown=true` |
| Emoji input from the IME | pass | three colour emoji inserted from Gboard's emoji panel and rendered in colour (`docs/screenshots/android-spike-emoji-input.png`) |
| Font scale Regular and Large | pass | in-app Regular/Large buttons; list rows grow and text wraps, nothing clips (`android-spike-large.png`) |
| Android system font scale | pass | `settings put system font_scale 1.3` is picked up (`onConfigurationChanged` -> JNI) and combined with the in-app scale, capped at 200% (`android-spike-system-1.3x.png`) |
| Foreground service keeps the process alive | pass | `dumpsys`: `isForeground=true`, type dataSync |
| Notification tap opens the right conversation | pass | `TestNotifyReceiver` posts a message notification; tapping it reaches Rust (`open conversation conv-abc123`) |
| client-core runs on Android and syncs | pass | spike signed in to a dev server over `10.0.2.2` and reported `connection: online` |
| APK size | see below | |

APK size: the native library is ~19 MB per ABI after `strip` (Slint femtovg renderer + client-core + SQLite + rustls). A debuggable x86_64-only
build is ~39 MB because Gradle stores the `.so` uncompressed (`useLegacyPackaging=false`, which is better for install size on device).
The two-ABI release APK (arm64-v8a + x86_64) is 44.5 MB. It is larger than a Kotlin app but acceptable for a self-hosted app distributed
as an APK.

Problems found and fixed during the spike:
* `android-activity`'s bundled C++ references `std::__ndk1::__hash_memory` (NDK 27 libc++), which older system libc++ does not export, so the
  library failed to load with "cannot locate symbol". Fixed by defining that symbol in Rust (FNV-1a) in `apps/android/src/android.rs`.
  Static libc++ linking via rustflags did not work because cargo-ndk overrides rustflags.
* Android 15 targets are edge-to-edge: content drew under the status bar. The real UI wraps everything in the window's
  `safe-area-insets` (see `apps/desktop/ui/main.slint`).
* `i-slint-backend-android-activity`'s build script needs `ANDROID_JAR`/`ANDROID_HOME`; set by the Gradle `cargoNdk` task.
* The emulator shows "System UI isn't responding" under heavy host load (builds in parallel). Not an app issue.

Not verified: real-device performance, IME composition for CJK, accessibility (TalkBack) with Slint's accesskit on Android, arm64 runtime
(built but only x86_64 was run; there is no arm64 emulator here and physical devices are off limits).

## Architecture and stack

* Rust workspace, edition 2021. `proto` is the single source of API types (serde + schemars); `schema/api.schema.json` is generated and the admin
  panel's TypeScript types are generated from it, so the client and server cannot drift.
* Server: axum 0.8 + tokio, rusqlite (bundled SQLite, WAL, FTS5). No ORM. One writer connection behind a mutex (`Db::call`), which is plenty for
  a team server and keeps ordering simple.
* **Per-user event log with a cursor** is the sync primitive. Every state change is written as event rows (one per recipient) inside the same
  transaction as the change, then pushed to the WebSocket hub after commit while the DB mutex is still held, so live order always equals cursor order.
  Clients store the last cursor, reconnect with it, and dedupe with `should_apply`. Pruning (default 90 days) raises an `events_floor`;
  a client behind the floor gets `cursor_expired` (410) and re-bootstraps.
* Idempotent sends: unique `(sender, client_id)`. A retry returns the stored message (HTTP 200 instead of 201).
* Edit rule lives in `proto::rules::edit_allowed` and is used by both sides: channels and notes-to-self always; DMs and group DMs only while no one
  else has posted later. The server is the authority; the client hides the Edit action using the same function.
* Per-user visibility: `message_hidden(user_id, message_id)`. "Each side of a chat stored separately" is implemented as a per-user hide table rather
  than duplicated rows, so deleting or clearing one person's copy never touches anyone else's and attachments are reference-counted by visibility.

## Auth and sessions

* Opaque tokens, not JWT: access `at_...` (default 15 min) and a refresh token `rt_...`. Only SHA-256 hashes are stored in `sessions`.
  Revocation (disable, password reset, clear history, sign-out) is therefore immediate: the next request, and the open WebSocket, fail.
* **Refresh tokens are not rotated.** Rotation needs a grace window to survive a lost response on flaky mobile networks; it is a
  possible hardening later. Mitigation: refresh tokens are per device and revocable, and stored only in the OS-backed secret store when available.
* Passwords: argon2id. Login lockout per username and per IP, token-bucket rate limiting, and a dummy hash so unknown users cost the same time.
* Disabled account: sessions revoked with reason `account_disabled`, sockets closed with 4403, the client receives `SignedOut{code}` and
  wipes the cache, drafts, outbox and secrets (X-05). Re-enabling restores the history (cloud chats).
* One-time login links: `atomic-chat://login?server=<url>&token=<token>`; token hashed, single use, 7-day expiry. The app also accepts a pasted https
  link or a bare token.
* No public registration. First-run `setup` creates the owner and is refused afterwards (`setup_done`).

## Transport security

* Clients require `https://`. `http://` is refused unless `allow_insecure_http` is set (tests, trusted LAN). Android needs a marker file for it.
* Bare-IP servers use Caddy `tls internal`; the client pins the certificate on first use (TOFU, `probe_certificate` + `PinOrRoots` verifier) and
  refuses to connect if the pin changes.
* **No end-to-end encryption.** The server can read messages; this is a self-hosted team tool where the admin is the account owner.
  Encryption at rest of the attachment bucket is the storage provider's job. Stated plainly in FEATURES.md.

## Storage

* One `Storage` trait (`put_file`, `put_bytes`, `get` with range, `head`, `delete`, `rename`). `ObjStore` (the `object_store` crate) covers local disk,
  S3, R2, B2 and MinIO; `GDrive` is a small hand-written Drive v3 REST client (OAuth refresh token, `drive.file` scope, one app folder).
* "Move all files": background job that copies each object, verifies size (and sha256 when deep), switches the active backend, then deletes sources.
  Uploads and backups pause while it runs (`migrating` flag; uploads get `conflict`, backups get `backup_failed`).
* Google Drive is tested only against an in-process fake (`tests/e2e/tests/fake_gdrive.rs`), not the real service (no credentials, and the plan
  forbids bulk-hitting external services). Real-world quotas, rate limits and Google's app verification for the `drive.file` scope are untested.
* Local storage is allowed (dev, small installs) but the admin panel says to use a cloud backend for production.

## Clear history and backups

* Clear history, delete user and date-range delete write a **per-user backup first** (JSON document, written to the storage, read back and checked);
  if that fails the action is refused with `backup_failed`. Attachments that no visible message references any more are *moved* (not deleted) to
  `backups/users/<uid>/<backup>/att/`.
* Uploader exception for downloads: the uploader can fetch a file only while it is not attached to any message yet.
* **Restore is not implemented** (planned P2). Backups are complete JSON so nothing is lost, but there is no UI/endpoint to restore from them.
* Encrypted full-server backups: not implemented (P2).

## Client

* `client-core` is the only place with logic: login, cursor sync over WebSocket with backoff + jitter, SQLite cache, FTS5 local search, outbox with
  idempotent client ids, drafts, wipe-on-revoke. The CLI, desktop and Android UIs are thin shells over it.
* **Secrets**: a 0600 file store (`FileSecretStore`) behind a `SecretStore` trait. OS keyring (libsecret, Windows Credential Manager) and the
  Android Keystore are not wired up yet. This is a known gap (X-04 partially met).
* **SQLCipher-encrypted cache is not done** (X-04): the cache is plain SQLite in the app's private directory.
* Preferences live in a separate file so they survive a wipe-on-disabled; `auto_away` is a presence preference.
* Dates are en-AU (`dd/mm/yyyy`, 12-hour `h:mm pm`, Monday first); `logic::datefmt` has unit tests including the day-label rules.
* The Android UI shares the desktop Slint UI (`apps/desktop` is a lib + bin; `apps/android` is a cdylib that calls `atomic_ui::run_with`).
  Local time zone uses `chrono::Local`, which falls back to UTC on Android; a JNI hook for the zone offset is a known gap.

## Admin panel

* Vanilla TypeScript + Vite (no framework): ~22 kB JS, builds with a plain `npm run build`, served by the server from `ATOMIC_ADMIN_DIR` at `/admin`.
  The Docker image builds it in a node stage and copies `dist`.
* All text is set through `textContent`, never `innerHTML`.
* Smoke-tested with Playwright (`tests/e2e/ui/admin-smoke.mjs`): setup wizard, user creation with one-time link, storage, sessions, backups, conversations.

## Deployment

* Image: multi-stage (node, rust, debian trixie-slim with ca-certificates + tini), non-root, HEALTHCHECK via `atomic-server healthcheck`
  (no curl needed), ~141 MB.
* MinIO stopped publishing `minio/minio` on Docker Hub, so the dev profile and the e2e test use `bitnamilegacy/minio` (a MinIO build);
  override with `MINIO_IMAGE`. Legacy image: development/testing only.

## Testing

* `crates/client-core/tests/flow.rs`: in-process server + real clients (30 tests).
* SV-24 (`tests/e2e`): the server runs as the real Docker image, MinIO runs in Docker, a fake Google Drive runs in the test process; clients are
  client-core and the `atomic` CLI. Run `tests/e2e/run.sh`. Skipped (passes) without `ATOMIC_E2E=1` so `cargo test --workspace` needs no docker.

## Phase 2 decisions

* **Backups use age (scrypt passphrase)**, one file per backup. The passphrase is kept in `<data>/backup.key` or `ATOMIC_BACKUP_PASSPHRASE`, never inside the backup or in the database. A full backup is a `VACUUM INTO` snapshot, so it is consistent while the server runs. `backups/full/latest.json` plus `index.json` in the storage target let a fresh server find the newest backup.
* **Restore bumps event ids by 1,000,000** and emits `resync`, so every client re-bootstraps instead of replaying a stale log.
* **Restore semantics**: "merge" (default) adds what is missing and keeps newer data; "replace" makes the person's chats equal to the backup. Deleted accounts are recreated. Per-user deletes need `user_ids` on restore; deletes "for everyone" write a `messages` backup that restores without it.
* **Mandatory backup before any bulk delete**; failure refuses the delete (`backup_failed`).
* **Push is content-free.** The server only sends a wake (ntfy body `wake`, FCM/APNs data `type: wake`); the app syncs and builds its own notification. Wakes go only to offline, non-DND, non-muted members whose notify level allows it. Tested against local fakes only.
* **Search is always members-only**, also when filters name a conversation the caller is not in.
* **Rich text uses Slint's `StyledText`** (CommonMark subset). The Atomic markup is converted to Markdown with escaping (`client_core::logic::markup::ui_blocks`); code blocks and quotes are drawn by the UI because StyledText has no block types. Mentions are bold in the accent colour.
* **No toolkit swap, one UI for desktop and Android**; layout switches on window width (720 px sidebar + chat, 1000 px adds the thread panel).
* **Desktop integrations call standard tools** (`secret-tool`, `notify-send`, `xdg-open`, `zenity`/`kdialog`, `wl-copy`/`xclip`) instead of linking native libraries; the keyring falls back to a 0600 file when no Secret Service answers a round-trip test.
* **Formatting toolbar acts on the last word** (or adds a line prefix) because Slint's text field exposes no selection.
* **Preferences** (`prefs.json`) hold no chat data so they survive a forced wipe; zoom, theme, accent, compact, 12/24 h, sort and push choice live there. Organisation settings (favourites, sections, mute) are on the server and sync.
* **Screenshots** come from the real app against a seeded in-process server (`tests/ui/screenshots.sh`); the app reads `ATOMIC_AUTOLOGIN_*` and `ATOMIC_UI_OPEN` for that only.

## Phase 3 and 4 decisions

* **Forms are a card plus a modal.** The card shows title, description and buttons; a form with fields shows "Fill in", which opens a modal with the
  fields and the submit button. Plain buttons submit immediately without the fields (the server validates this way); a button that carries its own
  form opens that form as the modal. Field values are held in Rust (`forms.rs`) and the Slint model is re-set only on toggles, so typing is never interrupted.
* **App Home is a tab of a direct message with a bot** (the "Home" pill), not a separate page; blocks are the server's `header, text, divider, button, form`.
* **Schedule send**: presets (1 h, 3 h, tomorrow 9:00, next Monday 9:00, local time) plus a typed `dd/mm/yyyy hh:mm`. The server sends the message (so it goes out
  with the app closed). "Drafts & sent" lists scheduled (cancellable), drafts and recent sent/failed items.
* **AI search toggle** appears only when the server's `/meta` says AI is on; the label says the question and names go to the provider. AI search runs on Enter, not on every keystroke (cost).
* **Colour emoji are bundled pictures.** Slint's text renderers draw colour-bitmap fonts (Noto Color Emoji) as outlines, so the picker and reaction chips show 64x64
  PNGs cut from Noto Color Emoji (SIL OFL 1.1) by `tools/gen_emoji.py` (`apps/desktop/emoji`, embedded). Emoji typed inside message text still use the font and may look monochrome; emoji we do not bundle fall back to the font.
* **Tray icon** uses Slint's own `SystemTrayIcon` (StatusNotifier on Linux, Shell_NotifyIcon on Windows). Closing the window hides it to the tray (a setting); if no tray could be created closing quits. Unread count is in the tooltip.
* **Drag-and-drop** uses the winit `DroppedFile` event (`unstable-winit-030`); it works where winit supports it (X11, Windows), not Wayland. Image paste is a "Paste image" button (wl-paste / xclip / PowerShell) because the text field owns Ctrl+V.
* **Secrets**: Linux Secret Service through `secret-tool` (file fallback), Windows Credential Manager through the `keyring` crate, Android Keystore (AES-GCM key in `AndroidKeyStore`, ciphertext in a private file; old plain-file secrets are migrated and deleted).
* **Android push**: the app registers a random per-device ntfy topic with the server (`PUT /me/push`) and writes `ntfy.json`; the foreground service holds a streaming GET to `<ntfy>/<topic>/json`. A pushed event wakes the Rust client (`start` + `reconnect_now`),
  and the service's client posts the local notification (honouring mute, notify level, DND and preview). No message text ever travels in a push. FCM: the receiver (`src/fcm`) is compiled only when `app/src/fcm/assets/fcm-config.json` exists, so the default APK has no Google libraries.
* **Android time zone** is pushed from Kotlin (`TimeZone.getDefault().getOffset`) on start and resume, because chrono falls back to UTC on Android.
* **Share target** copies shared streams into the app cache and shows a "Share to" conversation list (reusing the forward list).
* **Voice messages** record to AAC (`.m4a`, MediaRecorder) on Android and WAV with `arecord` on Linux; the inline player uses MediaPlayer (Android) or ffplay/mpv/paplay. Video clips upload as files and open in the system player (no in-app video decoding in Slint). There is no waveform or speed control.
* **Release pipeline**: tag `v*` runs `release.yml` (Linux deb + AppImage, Windows portable exe + NSIS installer, Android APK, server image on ghcr.io); `ci.yml` runs tests, clippy and the admin build. Windows code and the NSIS script are built only in CI (not testable on this machine).
* **Android signing**: `ATOMIC_KEYSTORE_*` environment variables select a keystore; CI creates a throw-away one when the secrets are absent; local builds fall back to the debug key.
