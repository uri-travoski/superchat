# Status: v1.1.1 (v1.0 plus the 1.1 changes below; calls are Phase 4, not built)

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

Partial / not done: video recording (M-31: upload and system player only), no waveform or speed control, FCM compiled only with a Firebase config
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
| X-02 credentials in platform secret store | tested | partial | Desktop: Secret Service keyring through `secret-tool` when it works, else 0600 file. Windows Credential Manager and Android Keystore not wired (Android uses the 0600 file in app-private storage) |
| X-03 reconnect, backoff, cursor catch-up, background pause | tested | done | Android keeps connection via foreground service rather than pausing |
| X-04 SQLite cache, optional encryption | partial | partial | Plain SQLite; SQLCipher not done |
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
| P-03 profile photo, initials fallback | tested | partial | Settings takes the path of a photo; other people's photos are not drawn (initials only) |
| P-04 display name | tested | done | Settings |
| E-01 dark mode / themes | n/a | done | System / Light / Dark and four accent colours, saved in `prefs.json` |
| E-02 compact layout | n/a | done | Settings switch |
| E-03 font size | n/a | done | Regular/Large buttons plus Android system font scale; verified by screenshots |
| E-04 en-AU dates, 12/24 h | tested | partial | 12/24 h switch added. Android time zone still falls back to UTC (no JNI zone hook) |
| F-01 upload with progress, limit | tested | partial | Native file chooser (zenity or kdialog) or a typed path; no progress bar or cancel |
| F-02 drag-drop and paste | not started | not started | Slint 1.19 has no OS file drop or image paste hook that fits; file chooser used instead |
| F-03 previews | not started | partial | Inline image previews (downloaded in the background), file cards; video and audio open in the system player |
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
