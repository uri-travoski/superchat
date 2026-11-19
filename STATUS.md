# Status: v1.3.1 (update checks, ghcr image, signed-release guard; calls are Phase 4, not built)

## v1.3.1 summary

Done: update check in the desktop, Android and admin apps (client-core `update` module, About page, rail and Home banners, Android PackageInstaller flow, admin Updates card through a server proxy), SHA-256 verification (a download without a checksum is refused), a 24 h schedule with a one-hour back-off after a failed check, the compose file pulls the public ghcr image, the release workflow fails when the Android signing secrets are missing, version 1.3.1 everywhere. Details: `docs/RELEASE-NOTES-v1.3.1.md`, `CHANGELOG.md`, `docs/DECISIONS.md` (1.3.1 section). Feature audit: `docs/FEATURE-AUDIT.md` (130 original rows unchanged, plus U-01 to U-07).

Found and fixed while testing on the emulator: after a failed check the automatic check re-ran on every 300 ms timer tick (85 requests in under two minutes against the fake server; GitHub allows 60 an hour per address). It now waits an hour after a failure (`auto_due`, unit-tested); a rate-limit run afterwards made exactly one request.

Tests (final run for 1.3.1): `cargo test --workspace --no-fail-fast` 224 passed, 0 failed (2 ignored: the seed server and the ntfy emulator harness; `flow::login_dm_and_realtime_delivery` timed out once while the machine was busy and passed on the rerun and in the full run); new: 11 `update` unit tests, 6 fake-server tests in `crates/client-core/tests/update.rs`, 2 nav tests; `tests/e2e/run.sh` passes (both tests); Playwright admin-roles, admin-viewer, admin-dialogs, admin-automation, admin-mobile and the new admin-updates (mocked release) OK; `cargo clippy --workspace --all-targets`: 0 errors. Docker image `superchat:1.3.1` (also tagged `ghcr.io/uri-travoski/superchat-server:1.3.1`) built, not pushed. `./gradlew assembleRelease` builds the full-ABI APK (debug-signed locally).
Emulator (emulator-5556, debuggable debug-signed APKs x86_64, a fake GitHub on 10.0.2.2 serving a v9.9.9 release whose APK is a debug build signed with the same key): About shows "Version 1.3.1 (code 8)" and the server version; up to date; update available with notes and a Home banner (found by the start-up check); download with progress; SHA-256 verified; the "install unknown apps" page opened and was allowed; the system "Do you want to update this app?" prompt, Cancel ("Installation cancelled", Install again), then Update: the app became 9.9.9 (code 99) and ran; GitHub rate limit, offline, tampered download ("does not match its checksum") and missing checksum errors. Screenshots: `docs/screenshots/android-update/`. Desktop screenshots (`tests/ui/screenshots.sh`): `docs/screenshots/*-upd-*.png` (About up to date, update available light and dark, rate limit, downloading, rail button, phone Home banner and About).
Not run: a real signature mismatch (needs a second signing key and none may be created here; the message and the mapping in `Installer.kt` exist), Windows, the .deb installer path, the real GitHub API from an app (latest there is still v1.3.0, so there is nothing newer to find), a real phone.
Seen once: after the first in-place update on the emulator the app was signed out (the Keystore key was not readable, "no secret store for the cache key" in logcat). A second PackageInstaller update and an `adb install -r` kept the session. Not reproduced; the person would only have to sign in again.


## v1.3.0 summary (previous release)

Done: owner fixes to menus (desktop popovers, phone sheets, every menu checked in a screenshot), uploads with progress and cancel, voice waveform and speed, video clips (Android in-app recorder, desktop screen recording), PDF first-page preview, profile photos, delete channel, notification sounds, tray icon, drag-to-reorder sidebar, spell check, encrypted local cache, Windows Credential Manager, ntfy push verified on the emulator, Android file, photo and video pickers. Details: `docs/RELEASE-NOTES-v1.3.0.md`, `CHANGELOG.md`, `docs/DECISIONS.md` (1.3.0 section). Feature audit: `docs/FEATURE-AUDIT.md`.

Feature audit counts (130 rows): 111 works, 12 fixed now, 5 partial, 2 deferred.
Partial: X-02 (Windows Credential Manager not run on Windows), F-02 (Wayland drop untested), N-01 and SV-13 (FCM and APNs never run on a device; ntfy verified), E-08 (tray tested with a fake tray host only).
Deferred: CL-01 and SV-18 (calls).

Tests (final run for 1.3.0): `cargo test --workspace` 205 passed, 0 failed (2 ignored: the seed server and the ntfy emulator harness); `tests/e2e/run.sh` SV-24 passes (full scenario and bots, webhooks, forms, schedule, AI); Playwright admin-roles, admin-viewer, admin-dialogs, admin-automation, admin-mobile all OK (Node 24, `PLAYWRIGHT_DIR` pointing at a node_modules with playwright-core; the browser is found under `~/.cache/ms-playwright`); `cargo clippy --workspace --all-targets`: 0 errors (style warnings only). New tests: upload progress and cancel, failed upload retry, profile photos, delete channel, voice metadata, sidebar order sync, PDF preview, video metadata (`crates/client-core/tests/features13.rs`), SQLCipher cache (`cache_encryption.rs`), spell check and sound preferences (unit), tray (`tests/ui/tray-check.sh`), ntfy on the emulator (`tests/e2e/ntfy-emulator.sh`).
Emulator (emulator-5556, debug APK x86_64): Part 1 items at Regular and font scale 1.3 (Home, shortcuts collapsed, Later), design channel (image, PDF preview, voice waveform and speed, video posters), attach sheet, in-app video recorder (virtual camera) recording and sending, file picker upload with progress and Cancel, notification sound sheet, Move up in the list, profile photo picker and update, Details > Manage > Delete channel dialog, ntfy notification, encrypted cache header check. Screenshots: `docs/screenshots/android-1.3-*.png`. Desktop screenshots: 112 variants from `tests/ui/screenshots.sh` (1280x800 and 1000x700 and phone 360x640, light and dark, Regular and Large) including every menu.
Not verified: real phones, Windows, Wayland drop, a real tray panel, the Android keyboard's spell check, FCM.

## v1.2.0 summary (previous release)

Done: Slack-style desktop shell and phone shell (one Slint UI), membership management in the apps, Save for later and labels, server-backed Activity feed, App Lock discoverability, one Back stack with a visible way back everywhere (`docs/UX-NAVIGATION.md`), rename/topic/archive in Details, quote reply, `@channel`/group mentions, search tabs and highlighting, global Files view, auto-away. Spec: `docs/UI-SPEC-1.2.md`. Feature audit: `docs/FEATURE-AUDIT.md`.

Feature audit counts (130 rows): 101 works, 12 fixed now, 15 partial, 2 deferred.
Partial: X-02 (no Windows credential manager), X-04 (no SQLCipher), M-30 (no waveform or speed), M-31 (no in-app video recording), C-06 (delete channel in the admin panel), NV-04 (no drag reorder), F-01 (no progress or cancel), F-02 (Wayland drop untested), F-03 (no PDF preview), N-01 (push only against fakes), N-04 (no per-channel sound), P-03 (other people's photos), E-05 (spell check is a switch), E-08 (no tray icon), SV-13 (push on device).
Deferred: CL-01 and SV-18 (calls).

Tests (final run for 1.2.0): `cargo test --workspace` 182 passed, 0 failed (1 ignored); `tests/e2e/run.sh` SV-24 passes (2 tests: full scenario including the new membership add/remove and saved-items/labels isolation block, and bots/webhooks/forms/schedule/AI); Playwright admin-roles, admin-viewer, admin-dialogs, admin-automation, admin-mobile all OK (run with a symlink from `~/.cache/ms-playwright/chromium-1/chrome-linux64/chrome` to the installed Chrome); `cargo clippy --workspace --all-targets`: 0 errors (style warnings only).
Emulator (emulator-5556, debug APK x86_64, Regular and font scale 1.3 for Home): Home with sections, channel, long-press message sheet, Add label then Later with label filter, channel details > Files > back > details > back > chat, thread open, reply with "Also send to #channel", Back, Threads list, Activity tab, You tab (Saved items, Files, Preferences, App Lock), Settings > Privacy > App Lock set PIN, lock "random" (disappears), padlock > PIN (reappears), admin Add people to a channel, global Files from You > Files and Back to You. Screenshots: `docs/screenshots/android-1.2-*.png`.
Not run on the emulator: Back-key run Activity tab > Home > leave app was covered by the nav unit tests only; the 1.3 font scale was exercised on Home only; layout polish items are listed in `docs/RELEASE-NOTES-v1.2.0.md`.

## v1.1.3 summary

Done: central Back stack (unit tested; verified on the emulator: Settings to list, sheet closes alone, chat to list, list to launcher), bottom sheets and alert dialogs, chat list / chat / Chat info / Settings redesign, icon set and design tokens, Android QR scan fix with runtime permission, image fallback and visible errors, admin dialog close X with Playwright test, version 1.1.3.
Partial: live camera scan not exercised with a real QR in front of a camera (emulator scene has none); swipe-down only from the sheet handle; no group creation, in-app sound picker, storage page or update check (see docs/RELEASE-NOTES-v1.1.3.md).

## v1.1 summary

Done: rename to SuperChat with compatibility migrations; new icon; multiple AI providers; clickable login links with landing page, QR code, Android and desktop link handling,
public-address setting; phone-friendly admin panel; login screen rework; user-deletion bug fix with regression test; App Lock (client-core logic and tests, UI, notifications, screenshots).
See docs/RELEASE-NOTES-v1.1.md for the honest limitations (Windows and arm64 still unexecuted, FCM and real AI services untested, App Lock is a screen lock and not encryption).
Tests: see the release notes for the counts of the final run.

## v1.0 summary (final autonomous pass)

## v1.0 summary (final autonomous pass)

Done: Phase 3 server (bots, webhooks, forms, App Home, scheduled messages, AI; SV-24 phase 3 passes), admin pages (bots, webhooks, AI, groups,
bookmarks, scheduled messages and APP tag in the viewer; Playwright `admin-automation.mjs`), desktop/Android UI for WF-01/A-03 (form card + modal), A-04 (Home tab),
APP tag, M-23/M-24, SR-06, SR-07, bookmarks, colour emoji (picker and reactions), tray (E-08), drag-drop and paste (F-02), voice messages (M-30),
Android: Keystore secrets (X-02), ntfy listener + local notifications + wake (N-01), time-zone fix, share target (E-06), RECORD_AUDIO recording.
Release: README, docs/RELEASE-NOTES-v1.0.md, ci.yml, release.yml, packaging. Local artifacts in `dist/` (git-ignored): deb, AppImage, APK; image `superchat:1.0`.

Partial / not done (1.2.0 list, see the 1.3.0 summary above for what changed): FCM compiled only with a Firebase config
(never run against Google), Windows paths never run, arm64 APK never executed, emoji inside message text monochrome, Wayland drag-drop,
voice recording not exercised with real audio on the emulator, calls (SV-18, CL-01), auto-away (P-02), flatpak not built in CI.
Verified on the emulator (x86_64): install, sign-in, chat view, Keystore-encrypted `secrets.keystore.json`. The ntfy listener, share intent and
recording were not driven end to end on the emulator.


Legend: **tested** = automated test passes; **done** = implemented and checked by hand or by the smoke test; **partial** = works with the gaps listed;
**not started**. "core" means server + client-core + CLI; "UI" means the Slint app (desktop and Android share it).

Test totals (2026-11-17): proto 10, client-core unit 35, client-core integration (in-process server, real clients): `flow.rs` 30,
`phase2.rs` 8, `push.rs` 3, `backups.rs` 5, `adminview.rs` 6; server crypto/unit tests; SV-24 end-to-end 1 (docker server + MinIO + fake
Google Drive + fake ntfy + a second empty server for restore; includes the Phase 2 scenario); admin panel Playwright
(`tests/e2e/ui/admin-viewer.mjs`, run by hand). `cargo clippy --workspace --all-targets`: no errors, a few style warnings.
Screenshots: `docs/screenshots/` (16 images), regenerate with `tests/ui/screenshots.sh` (private Xvfb + own dbus, never the real desktop).

## Phase 0

| Item | Status | Notes |
|---|---|---|
| Workspace layout, version 1.0.0 | done | `crates/{proto,server,client-core,cli}`, `apps/{desktop,android}`, `admin-web`, `deploy`, `tests/e2e`, `docs` |
| proto crate, schema generator, API_VERSION, min_client_version, error codes | tested | `gen-schema` writes `schema/api.schema.json`; admin TS types generated from it |
| Server: login, refresh, device sessions, per-user event log + cursor, WebSocket, idempotent sends | tested | flow.rs |
| client-core: login, cursor sync + reconnect/backoff, SQLite cache, outbox, wipe on disabled/revoked | tested | flow.rs |
| CLI `atomic` | done | login/link/logout/whoami/sessions/chats/users/dm/history/send/upload/download/edit/delete/react/read/search/listen + admin commands. Exercised by SV-24 (login, whoami, dm, send, history, logout) |
| Slint spike, desktop | done | |
| Slint spike, Android emulator, gate items | done, passed | see `docs/DECISIONS.md` (IME, emoji, Regular/Large + system font scale, service, notification tap, client-core online). Slint kept |

## Phase 1: server

| ID | Status | Notes |
|---|---|---|
| SV-01 accounts (create, disable/enable, reset, delete, no registration) | tested | disable = immediate kick + client wipe; one-time login link; delete backs up first |
| SV-02 auth and sessions | tested | opaque tokens, per-device sessions with name/version/last seen/IP, lockout, rate limit. Refresh tokens not rotated (decision) |
| SV-03 conversations (admin-only channels/groups, members-only, DMs and notes on demand) | tested | |
| SV-04 message store (edit, tombstone, reactions, mentions, attachments, thread/reply refs) | tested | thread/reply fields stored; threads UI is not Phase 1 |
| SV-05 edit rule | tested | unit tests in `proto::rules` and integration tests (DM closes after reply, channel always) |
| SV-06 realtime, typing, presence, read sync, delivery states | tested | |
| SV-07 attachments, streaming upload, size limit, storage backends | tested | local, S3-compatible (MinIO tested), R2/B2 are the same code path with different endpoints (not tested against the real services), Google Drive tested only against a fake |
| SV-14 roles | tested | owner/admin/member enforced per endpoint |
| SV-19 admin panel (basic) | done | Vite + TS, served at `/admin`: setup wizard, login, users (create, disable, reset, login link, backup, clear history, delete, privacy flag), conversations + members, storage (test, save, move, verify), sessions, backups, overview. Playwright smoke script passes; not part of `cargo test` |
| SV-22 deployment | done | Dockerfile (multi-stage, 141 MB), compose with profiles `tunnel`, `caddy`, `ntfy`, `minio`, `.env.example`, Caddyfile incl. `tls internal` for bare IPs. Built and run in the e2e. Caddy, cloudflared and ntfy profiles were not started here (no domain / tunnel token / not needed yet) |
| SV-24 test suite | tested | `tests/e2e/run.sh`: login, DM, channel, edit rule, tombstone, upload/download via MinIO, sync after offline, disabled-account wipe, search isolation, storage move+verify S3 -> Drive -> S3, clear-history backup |
| Server-doc: per-user visibility, clear history with mandatory backup, files moved to backup | tested | refused with `backup_failed` when the backup cannot be written, including while a storage move runs |
| Server-doc: backup restore | tested | see Phase 2 |
| Server-doc: encrypted full backups, admin web chat viewer | tested | see Phase 2 |

## Phase 1: client features (core = tested in client-core/CLI; UI = in the Slint app)

| ID | core | UI | Gaps |
|---|---|---|---|
| X-01 login (server, user, password; TLS; fingerprint for self-signed) | tested | done | Fingerprint dialog (trust on first use) shows for https servers without a public certificate; accepted fingerprint is pinned. Verified only against http test servers and by code path (no self-signed https server was run through the UI) |
| X-02 credentials in platform secret store | tested | partial | 1.3: Secret Service or 0600 file (Linux), Android Keystore file, Windows Credential Manager wired and type-checked with a CI round-trip test; never run on Windows here |
| X-03 reconnect, backoff, cursor catch-up, background pause | tested | done | Android keeps connection via foreground service rather than pausing |
| X-04 SQLite cache, optional encryption | tested | done | 1.3: SQLCipher by default, key in the secret store, plain caches converted on first start |
| X-05 wipe on disabled/revoked | tested | done | UI returns to login with the server's message |
| X-06 Android foreground service | n/a | done | low-priority persistent notification, verified on emulator; user toggle not built |
| X-07 forced sign-out message | tested | done | |
| X-08 emoji picker | n/a | done | In-app picker for reactions and the composer (about 80 emoji in four groups) plus the OS emoji keyboard |
| M-01..M-04 channels, DMs, group DMs, notes to self | tested | done | UI lists and opens all four kinds; "New" opens a people list to start a DM (desktop verified; the Android APK built before this button was added) |
| M-08 edit (rule) / M-09 "(edited)" | tested | done | |
| M-10 delete (tombstone) | tested | done | |
| M-11 reactions | tested | done | Chips with counts, own reaction highlighted, tap a chip to toggle, picker to add |
| M-12 formatting / M-13 code blocks | tested (parser) | done | Bold, italic, strike, inline code, links, lists, quotes via Slint `StyledText` (CommonMark subset); code blocks in a monospace box. Toolbar buttons act on the last word because the text field exposes no selection |
| M-16 mentions | tested | done | Highlighted in messages; autocomplete list of conversation members when typing `@` (assumes the cursor is at the end of the text) |
| M-25 typing | tested | done | |
| M-26 unread / mark as unread | tested | done | Badges, "New messages" divider that stays while the chat is open, First unread jump, Mark as unread in the conversation menu |
| M-27 multi-device sync | tested | done | Drafts are saved per conversation and restored when it is opened |
| M-28 offline outbox, retry | tested | done | pending/failed states and Retry link |
| M-29 jump to date / history | tested | done | Older messages load when scrolling to the top; Go to date field (dd/mm/yyyy). Scroll position after loading older is approximate |
| M-32 delivery states | tested | done | sending / not sent / delivered tick |
| C-01..C-03 rename, topic, create channel | tested | n/a | admin panel and CLI do these; members cannot, by design |
| P-01 presence / DND | tested | done | Online / Away / Do not disturb in Settings; dot on own avatar. Auto-away idle detection still missing (P-02) |
| P-02 auto-away | partial | not started | core `set_idle`; no idle detection yet |
| P-03 profile photo, initials fallback | tested | done | 1.3: photos shown everywhere; Android system photo picker, desktop file dialog |
| P-04 display name | tested | done | Settings |
| E-01 dark mode / themes | n/a | done | System / Light / Dark and four accent colours, saved in `prefs.json` |
| E-02 compact layout | n/a | done | Settings switch |
| E-03 font size | n/a | done | Regular/Large buttons plus Android system font scale; verified by screenshots |
| E-04 en-AU dates, 12/24 h | tested | partial | 12/24 h switch added. Android time zone still falls back to UTC (no JNI zone hook) |
| F-01 upload with progress, limit | tested | done | 1.3: progress bar, Cancel, Retry; native file chooser on desktop, system pickers on Android |
| F-02 drag-drop and paste | tested | partial | 1.3: Ctrl+V of files and images; drop on X11 and Windows; Wayland drop untested |
| F-03 previews | tested | done | 1.3: inline images, server-drawn PDF first page, video poster; the rest open in the system |
| N-02 desktop notifications | tested | partial | Desktop: `notify-send` on incoming messages, honouring mute, notify level, DND and the preview switch. Android: channel + tap-to-open only; real message notifications from the service not built |

## Phase 2: organisation and search

Server and core are covered by the tests listed at the top; "admin web" is the TypeScript panel.

| ID | server + core | UI | Notes |
|---|---|---|---|
| SV-08 search filters (from, in, has:link/file/reaction, before/after), members-only | tested | done | Search screen with chips; typed filters work too |
| SV-09 threads (reply lists, follow, unread, thread list) | tested | done | Thread panel (side panel at 1000 px and wider, full screen on a phone) and Threads screen |
| SV-10 pins, bookmarks bar | tested | partial | Pins tab and pin/unpin (owners and admins) done; bookmarks bar shows link bookmarks, adding bookmarks has no UI (admin or API) |
| SV-11 user groups (`@oncall` style mentions) | tested | n/a | Mentions resolve on the server; no group management UI |
| SV-12 favourites, sections, mute, notify level, sort, unread filter, hide muted | tested | done | Conversation menu (mute, notify level, mark unread, favourite, move to section): right-click a row on desktop, or "Options" in the conversation header (works on a phone); sections are created in Settings. Sort, unread-only and hide-muted pills sit above the list |
| SV-13 push (ntfy, FCM, APNs wakes, content-free) | tested with fakes | partial | Android Settings stores the choice and ntfy address; registering with the server and ntfy/FCM delivery on the device are not built. Not tested against real ntfy/FCM/APNs |
| SV-20 forward, permalinks, drafts, files list | tested | partial | Forward and copy-link done; permalinks open when pasted (`superchat://m?...`); Files tab lists cached files |
| Encrypted backups (age, passphrase outside the backup), scheduled/full/per-user | tested | n/a | admin web Backups page |
| Restore: fresh server, selective (merge/replace), attachments moved back | tested | n/a | e2e restores onto an empty second server |
| Date-range delete, also other person's copy, mandatory backup first | tested | n/a | |
| Admin chat viewer, bulk delete, admin can chat | n/a | admin web done | Playwright `admin-viewer.mjs` |
| Quick switcher (Ctrl+K), back/forward (Alt+Left/Right), jump to unread | n/a | done | |
| Settings (theme, accent, compact, font, zoom, reduce motion, 12/24 h, privacy when enabled, devices) | n/a | done | Zoom with Ctrl +, -, 0 |
| Voice and video clips | not started | not started | Not built; no audio/video capture in the Slint shell |
| Android share target, tray icon, Android Keystore, FCM on device | not started | not started | Tray: Slint tray feature compiled in but no tray icon is created; "Start minimised" minimises the window. Launch on login writes an XDG autostart file (Linux only) |

## Packaging

* APK: build with `./gradlew assembleRelease` (or `assembleDebug -PatomicAbis=x86_64` for an emulator build; the debug build allows `run-as` to create the `files/allow_http` marker for http test servers). The Phase 2 UI was built for x86_64 and checked on emulator-5556: the conversation list and Settings render with the new layout; sign-in, chat and the organisation menus were verified on desktop only. Earlier size figure for two ABIs: 44.5 MB. Gradle project `apps/android` (`./gradlew assembleRelease`, ABIs `arm64-v8a` + `x86_64`, or `-PatomicAbis=x86_64`). Debug-signed. Run on the emulator
  only. arm64 compiles but was never executed.
* Linux binary: `cargo build --release -p atomic-desktop` (binary `superchat`). No AppImage/deb yet (Phase 4).
* No Windows build yet (Phase 4).

## Known risks

* Google Drive backend is only proven against a fake.
* The Phase 2 phone layout was checked at 360x640 under Xvfb and one pass on the Android emulator; the Android long-press menu, keyboard handling of the new fields and push on a real device are untested.
* Backup passphrase loss means encrypted backups cannot be read; there is no recovery.
* Desktop GUI was tested under Xvfb with a manual window-focus workaround (no window manager); IME and clipboard on real desktops untested.
* Slint accessibility on Android not verified; no screen-reader work is in scope (E-03).
