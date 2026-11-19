# Feature audit against the original requirements (1.4.0)

One row per feature in `Client Apps.md` (sections X, M, C, NV, SR, F, WF, N, P, A, AD, E, SV). Status words:
**works** (was there and checked again), **fixed now** (missing or hidden in 1.1.3, changed in 1.2.0), **partial** (works with the gap in Notes), **deferred** (not built, scheduled later).
"Phone" and "Desktop" give the shortest path from a visible control. Discoverability rule: every user feature is within 2 to 3 taps or clicks of a visible control; no gesture-only or shortcut-only features; admin features are visibly reachable.
"Tested" legend: **core** = client-core integration tests (`crates/client-core/tests`); **SV24** = `tests/e2e/run.sh`; **UI-unit** = `cargo test -p atomic-desktop`; **PW** = Playwright `tests/e2e/ui`; **emu** = driven by hand on Android emulator-5556 this release; **shot** = desktop or phone screenshot reviewed.

## Client foundation (X)

| ID | Feature | Status | Phone | Desktop | Tested | Notes |
|---|---|---|---|---|---|---|
| X-01 | Login, TLS, fingerprint pinning | works | login screen | login screen | core, emu (login) | fingerprint dialog only for https servers with a non-public certificate; not exercised against a real self-signed host |
| X-02 | Secret store | partial | n/a | n/a | core, emu, CI (Windows job) | Android Keystore-encrypted file; Linux `secret-tool` else a 0600 file; Windows Credential Manager (`keyring`, windows-native) is wired, type-checked for x86_64-pc-windows-msvc and has a CI round-trip test, but was never run on Windows here |
| X-03 | Reconnect, backoff, catch-up | works | automatic | automatic | core | Android keeps the connection with the foreground service instead of pausing |
| X-04 | SQLite cache with optional encryption | works | n/a | n/a | core (`cache_encryption.rs`), emu | SQLCipher, key in the secret store, an older plain cache is converted on first start; checked on the Android emulator (the cache file has no SQLite header). Attachment files in the cache are not encrypted |
| X-05 | Server is source of truth, wipe on disable | works | login shows the message | same | core, SV24 | |
| X-06 | Android foreground service | works | You > Preferences > Notifications | n/a | emu (1.1.3) | |
| X-07 | Forced sign-out message | works | login screen | login screen | core | |
| X-08 | Emoji picker | works | composer smiley, reaction button | composer smiley, hover toolbar | shot, emu | 1.2 adds `:name` shortcode autocomplete |

## Messaging (M)

| ID | Feature | Status | Phone | Desktop | Tested | Notes |
|---|---|---|---|---|---|---|
| M-01 | Private channels | works | Home > Channels | sidebar Channels | core, SV24, emu, shot | |
| M-02 | Direct messages | works | Home > DMs section, pencil button | sidebar Direct messages "+" | core, emu, shot | |
| M-03 | Group DMs | works | Home > DMs section | sidebar Direct messages | core, emu | created by admins only (SV-03) |
| M-04 | Message yourself | works | Home > DMs > Notes to self | sidebar | core, emu | |
| M-05 | Threads | works | "N replies" row or long-press > Reply in thread | hover toolbar, "N replies" | core, emu, shot | full-screen thread on phone, side panel on desktop |
| M-06 | Threads view | works | Home > Threads | sidebar Threads | core, emu | |
| M-07 | Follow / unfollow thread | works | thread header menu | thread panel header | core | |
| M-08 | Edit with the edit rule | works | long-press > Edit | hover toolbar > More > Edit, Up in empty box | core, SV24 | Edit hidden once closed |
| M-09 | "(edited)" | works | in the row | in the row | shot | |
| M-10 | Delete own message | works | long-press > Delete | hover toolbar > More | core, SV24 | |
| M-11 | Reactions | works | long-press quick row, pill "+" | hover toolbar, pills | core, emu, shot | |
| M-12 | Formatting | works | "Aa" toggles the toolbar | formatting row | UI-unit, shot | |
| M-13 | Code blocks | works | formatting row | formatting row | shot | copy button on blocks is partial: use select and copy |
| M-14 | Quotes and lists | works | formatting row | formatting row | shot | |
| M-15 | Rich text toolbar | works | "Aa" | always visible | shot | buttons wrap the selection or insert markers |
| M-16 | @mentions with autocomplete | works | type @ | type @ | UI-unit, shot | |
| M-17 | @channel / @here / @everyone | fixed now | type @ | type @ | UI-unit, shot | autocomplete offers them and expands on send; `@here` limited to online members by the server |
| M-18 | User groups | fixed now | type @ | type @ | core | autocomplete includes groups; groups are managed in the admin panel |
| M-19 | Message permalinks | works | long-press > Copy link | hover > More > Copy link | UI-unit, core | opening a pasted link jumps to the message; there is no conversation-level link, only per message (DECISIONS.md) |
| M-20 | Forward | works | long-press > Forward | hover toolbar > Forward | core | |
| M-21 | Quote reply | fixed now | long-press > Quote reply | hover > More > Quote reply | core, shot | quote bar above the composer, sent as `reply_to` |
| M-22 | Drafts | works | Home > Drafts & sent | sidebar Drafts & sent | core | |
| M-23 | Drafts & sent view | works | Home > Drafts & sent | sidebar | core, emu (count) | |
| M-24 | Schedule send | works | composer > Send chevron | composer chevron | core, SV24 | |
| M-25 | Typing indicators | works | automatic | automatic | core | |
| M-26 | Unread, mark as unread | works | long-press row (list) > Mark unread; bold row and badge | right-click row > Mark as unread | core, emu, shot | |
| M-27 | Multi-device sync | works | automatic | automatic | core, SV24 | saved items and labels sync too (new) |
| M-28 | Offline delivery | works | automatic | automatic | core | |
| M-29 | Jump to date, older history | works | Home > search > Date; scroll up | search > Date; scroll up | core | |
| M-30 | Voice messages | works | + > Voice message; tap the card to play, 1x button for speed | + > Voice message; click the waveform to seek | core (`features13.rs`), shot, emu | waveform (64 bars) and duration travel with the message; play, pause, seek, 1x/1.5x/2x. Recording itself was tried only with a synthetic audio source |
| M-31 | Video clips, screen recordings | works (in-app recording removed on Android by owner request in 1.4) | + > Photo or video, File (videos still attach and play; there is no in-app camera recorder any more) | + > Upload video, Record screen | core (`features13.rs`), emu, shot | poster frame, duration and size are computed by the sender; the emulator's virtual camera recorded, sent and showed a clip. Real phone cameras and pickers untested; desktop screen recording needs ffmpeg on X11 or Windows |
| M-32 | Delivery status | works | tick and "not sent / Retry" | same | core, emu (tick), shot | |

## Channels (C)

| ID | Feature | Status | Phone | Desktop | Tested | Notes |
|---|---|---|---|---|---|---|
| C-01 | Rename | fixed now | Details > Settings > Admin tools | header name > Settings > Admin tools | UI build, shot | owner/admin only; was admin panel only |
| C-02 | Topic and description | fixed now | Details > About (admins edit) | details modal > About | core, shot | |
| C-03 | Create channel (admin, private) | fixed now | Home > Channels "+" > Create channel (admins) | sidebar Channels "+", server menu | UI build, SV24 | admin-only by design |
| C-04 | Archive | fixed now | Details > Settings > Admin tools | same | core, shot | moves to an Archived section (collapsed by default) |
| C-05 | Unarchive | fixed now | Home > Archived > details | same | core | same toggle |
| C-06 | Delete channel with typed name | works | Details > Manage > Delete channel... | Details > Settings tab > Delete channel... | core (`features13.rs`), shot | type the channel name to confirm; the server also removes pins, bookmarks, saved items and files |
| C-07 | Channel bookmarks | works | Details > Bookmarks | Bookmarks tab, "Add a bookmark" (admins) | core, shot | |
| C-08 | Channel tabs | works | Details > Files, Pins, Bookmarks | tabs Messages, Files, Pins, Bookmarks | emu, shot | back from every page (UX-NAVIGATION.md) |
| C-09 | Pinned messages | works | long-press > Pin; Details > Pins | hover > More > Pin; Pins tab | core | |

## Navigation (NV)

| ID | Feature | Status | Phone | Desktop | Tested | Notes |
|---|---|---|---|---|---|---|
| NV-01 | Pin a channel (favourite) | works | long-press row > Pin to top / Unpin, Details > Pin to top; pinned rows sit at the top of DMs and Channels, reorder by long-press and drag or Move up / Move down | right-click > Star | core | |
| NV-02 | Pin a DM | works | same | same | core | |
| NV-03 | Pin a group | works | same | same | core | |
| NV-04 | Custom sections | works | row menu > Move to section, Move up/down; Home section header | drag and drop in the sidebar, Move up/down, sidebar "+" > New section | core (`features13.rs`), emu, shot | the order of conversations and sections is synced between devices |
| NV-05 | Sort, unreads only | works | Home filter icon | server menu / sidebar filter | core, emu | |
| NV-06 | Hide muted | works | Home filter icon | server menu | core | |
| NV-07 | Mute | works | row menu > Mute; Details > Mute | right-click > Mute | core | |
| NV-08 | Quick switcher | works | Home search pill | Ctrl+K or the top-bar search | shot | |
| NV-09 | Back/forward, jump to unread | works | system Back; "Jump to first unread" | arrows in the top bar, Alt+Left/Right | UI-unit, emu | | compact "N new messages" pill (phone) / slim bar (desktop), hides once first unread visible |

## Search (SR)

| ID | Feature | Status | Phone | Desktop | Tested | Notes |
|---|---|---|---|---|---|---|
| SR-01 | Message search with highlights | fixed now | Home search pill | top-bar search, Ctrl+F | core, shot | matches are bold-highlighted |
| SR-02 | Filters | works | From, In, Date, Has pills | same | core, shot | |
| SR-03 | Search files | works | search > Files tab | search > Files tab | core, shot | |
| SR-04 | Search people | works | search > People | search > People | core | starts a DM |
| SR-05 | Search channels | works | search > Channels | search > Channels | core | |
| SR-06 | Recent, saved searches | works | search screen | search overlay | core | |
| SR-07 | AI search | works | search > "Ask AI" (only if enabled) | same | core (fake provider) | real providers untested |

## Files (F)

| ID | Feature | Status | Phone | Desktop | Tested | Notes |
|---|---|---|---|---|---|---|
| F-01 | Upload | works | + > file (progress bar and Cancel in the message list) | same | core (`features13.rs`), emu, SV24 | progress with bytes sent, Cancel, Retry that keeps files that already finished |
| F-02 | Drag-drop and paste | partial | + > Paste image from clipboard | Ctrl+V of files and images; drop on X11 and Windows | core, shot | Ctrl+V works; dropping files was driven on X11 only. Wayland drop could not be tested here |
| F-03 | Previews | works | inline image, PDF first page, video poster; Open for the rest | same | core (`features13.rs`), emu, shot | the server draws the first PDF page (pure Rust, cached); a broken PDF falls back to a file card |
| F-04 | Share file to channels | works | long-press > Forward | hover > Forward | core | re-posts the reference |
| F-05 | Files browser | fixed now | You > Files, Details > Files | More > Files, server menu > Files, Files tab | emu (Details > Files), shot | global Files view with type chips is the Files tab of Search |
| F-06 | Comments on files | works | "Reply in thread" on the file message | same | core | |
| F-07 | Admin deletes files | works | Open > Remove (admins) | file card menu (admins) | core, PW | "File removed" shown |

## Workflows, calls (WF, CL)

| ID | Feature | Status | Phone | Desktop | Tested | Notes |
|---|---|---|---|---|---|---|
| WF-01 | Workflow forms | works | form card in the chat | form card or modal | core, SV24 | |
| CL-01 | 1:1 calls | deferred | none | none | none | with SV-18, Phase 4 |

## Notifications (N)

| ID | Feature | Status | Phone | Desktop | Tested | Notes |
|---|---|---|---|---|---|---|
| N-01 | Mobile push | partial | You > Preferences > Notifications > Push method | n/a | core (fakes), emu (real ntfy container, `tests/e2e/ntfy-emulator.sh`) | ntfy wake-ups verified end to end on the emulator (content-free wake, sync, the app's own notification); FCM is compiled only with a Firebase config and never ran on a device |
| N-02 | Desktop notifications | works | n/a | Preferences > Notifications | shot | via `notify-send`; inline reply not built |
| N-03 | Per-conversation level | works | Details > Notifications | right-click > Notify; details > Settings | core | |
| N-04 | Sounds and badges | works | Preferences > Notifications > Default sound; chat row menu > Notification sound | same (Preferences, row menu) | unit (`prefs.rs`), emu, shot | bundled sounds, per-conversation sound, global switch, unread number on the badge. Sounds are per device, not synced |

## Presence and profile (P)

| ID | Feature | Status | Phone | Desktop | Tested | Notes |
|---|---|---|---|---|---|---|
| P-01 | Online / away / DND | works | You > Set status, Pause notifications | rail avatar menu | core, emu (You), shot | |
| P-02 | Auto-away | fixed now | backgrounded app reports idle | 10 minutes without input reports idle | core (set_idle) | desktop timer and Android background hook added in 1.2; not observed over a real 10 minute idle |
| P-03 | Profile photo | works | You > Preferences > Profile > Choose photo (system picker) | Preferences > Profile > Choose photo | core (`features13.rs`), emu | the photo shows everywhere the person appears; other people's photos are downloaded and cached |
| P-04 | Display name | works | You > profile | Preferences | core | |
| P-05 | Last active, privacy | works | DM header, Preferences > Privacy (only when the server flag is on) | same | core, PW | |

## Apps and platform (A)

| ID | Feature | Status | Phone | Desktop | Tested | Notes |
|---|---|---|---|---|---|---|
| A-01 | Incoming webhooks, APP tag | works | APP tag in rows | same | core, SV24 | managed in the admin panel |
| A-02 | Outgoing webhooks | works | n/a | admin panel | SV24, PW | |
| A-03 | Modals and forms | works | form card | form card | core | |
| A-04 | App Home tab | works | DM with a bot > Home | same | core | |
| A-05 | Platform API | works | n/a | docs/API.md | SV24 | bearer tokens only |

## Admin (AD)

| ID | Feature | Status | Phone | Desktop | Tested | Notes |
|---|---|---|---|---|---|---|
| AD-01 | Invite people | works | You > Admin panel (admins) | server menu > Invite people (explains the admin panel) | PW | no in-app sign-up by design |
| AD-02 | Roles | works | admin rows only for admins | same | core, PW | |
| AD-03 | Deactivate accounts | works | wipe and login | same | core, SV24 | |
| AD-04 | Reset password, force sign-out | works | login message | same | core | |
| AD-05 | Devices and sessions | works | You > Devices | Preferences > Devices | core, shot | hidden when the policy forbids sign-out |
| AD-06 | Export and backups | works | n/a | admin panel | SV24, PW | no client UI by design |
| AD-07 | Only admins create channels | works | "Create channel" only for admins | same | core, SV24 | |
| AD-08 | Custom domain | works | login field | login field | core | |
| AD-09 | Data residency, no telemetry | works | n/a | n/a | review | |
| AD-10 | Transport encryption | works | https required (http only with an explicit setting) | same | core | |

## Extras (E)

| ID | Feature | Status | Phone | Desktop | Tested | Notes |
|---|---|---|---|---|---|---|
| E-01 | Themes | works | Preferences > Appearance | same | shot (light, dark) | |
| E-02 | Compact | works | Preferences > Appearance | same | shot | |
| E-03 | Font size, zoom, reduce motion | works | Preferences; follows system | Ctrl +/-, Preferences | emu (Regular and 1.3), shot | 1.4: screen-reader support (Slint accessibility / AccessKit, accessible-* properties) removed by owner request; touch targets unchanged |
| E-04 | Date and time formats | works | Preferences | Preferences | UI-unit | |
| E-05 | Spell check | works | the keyboard's own spell check | red squiggles in the message box, right-click for suggestions, Add to dictionary, Ignore; Preferences > Chats > Check spelling, Spelling language | unit (`spell.rs`), shot | bundled en_AU plus system Hunspell dictionaries (pure Rust, no hunspell library). Android depends on the keyboard (not driven here) |
| E-06 | Android share target | works | share from another app | n/a | emu (1.0 build) | not re-run in 1.2 |
| E-07 | Offline mode, draft sync | works | offline banner | same | core | |
| E-08 | Launch on login, tray | partial | n/a | Preferences > General | `tests/ui/tray-check.sh` (fake tray host), shot | autostart file (Linux) and tray icon with an unread dot, Status and Pause notifications menu, start minimised; tested against a fake StatusNotifierWatcher, not a real desktop panel or Windows |

## Server (SV)

| ID | Feature | Status | Where in the app | Tested | Notes |
|---|---|---|---|---|---|
| SV-01 | Accounts by admins only | works | admin panel > Users | core, SV24, PW | |
| SV-02 | Auth and sessions | works | login, Devices | core, SV24 | |
| SV-03 | Conversations (admin-managed) | fixed now | Details > Settings > Admin tools; Members > Add people | core, SV24 (membership add/remove) | rename, topic, archive and membership are now in the apps |
| SV-04 | Message store | works | all chat | core, SV24 | |
| SV-05 | Edit rule | works | Edit hidden | core, SV24 | |
| SV-06 | Realtime, typing, presence, receipts | works | automatic | core | |
| SV-07 | Attachments, storage backends | works | + > file | core, SV24 | R2/B2 same code as S3 (not run against the real services) |
| SV-08 | Search | works | search | core, SV24 | |
| SV-09 | Pins, bookmarks, groups | works | Details | core | |
| SV-10 | Scheduled messages | works | Drafts & sent | core, SV24 | |
| SV-11 | Sessions | works | Devices | core | |
| SV-12 | Privacy flag | works | Preferences > Privacy | core, PW | |
| SV-13 | Push | partial | Preferences > Notifications | n/a | core (fakes), emu (ntfy) | ntfy end to end on the emulator; not against real FCM or APNs |
| SV-14 | Roles | works | admin rows | core, SV24 | |
| SV-15 | Bots | works | admin panel | SV24, PW | |
| SV-16 | Webhooks | works | admin panel | SV24, PW | |
| SV-17 | Forms | works | form card | core, SV24 | |
| SV-18 | Calls (signalling, TURN) | deferred | none | none | Phase 4 |
| SV-19 | Admin panel | works | You > Admin panel, sidebar server menu > Admin panel | PW | |
| SV-20 | Backups | works | admin panel | SV24, PW | |
| SV-21 | AI features | works | search, channel recap | core (fake) | |
| SV-22 | Deployment | works | n/a | SV24 (docker) | Caddy, cloudflared, ntfy profiles not started |
| SV-23 | Security limits | works | n/a | core | |
| SV-24 | Test suite | works | n/a | SV24 | extended in 1.2 with membership and saved-items isolation |

## New in 1.2 (not in the original PRD list)

Save for later and labels (per user, private, synced; Later view with label chips; isolated by user, tested in SV24 and core), server-backed Activity feed (All, Mentions, Threads, Reactions), App Lock discoverability (Settings > Privacy & Security > App Lock lists locked conversations; row menus offer Lock conversation or Set up App Lock; emulator-driven: set PIN, lock a channel, it disappears, unlock, it reappears), one Back stack with a visible way back on every screen (docs/UX-NAVIGATION.md).

## New in 1.3.1 (not in the original PRD list)

| ID | Feature | Status | Where | Tests | Notes |
|---|---|---|---|---|---|
| U-01 | Update check (GitHub latest release, 24 h, skip, remind, override URL) | works | Settings > About, rail, Home banner | unit, fake-server integration (`crates/client-core/tests/update.rs`) | real GitHub only reachable from CI or a release |
| U-02 | Version shown in every app | works | About, login footer, admin header and footer | nav unit, screenshots, PW | |
| U-03 | Android download, SHA-256 check, system installer | works | About | emulator run (fake release 9.9.9, installer prompt accepted, new version ran) | signature-mismatch text not run (no second key was created) |
| U-04 | Desktop install: AppImage replace, deb installer, Windows setup | partial | About | unit (asset choice, checksum, AppImage replace) | Windows and deb install not run here |
| U-05 | Admin panel version and Updates card | works | admin panel | PW `admin-updates.mjs` (mocked release) | |
| U-06 | Server image from ghcr | works | `deploy/docker-compose.yml` (standalone since 1.3.2) | compose config; ghcr manifest anonymous 200 | |
| U-07 | Release workflow fails without signing secrets | works | `.github/workflows/release.yml` | read only (no CI run) | |

## Counts

Rows: X 8, M 32, C 9, NV 9, SR 7, F 7, WF 1, CL 1, N 4, P 5, A 5, AD 10, E 8, SV 24 = 130.
See the counts line in `STATUS.md` (works, fixed now, partial, deferred) and the partial and deferred lists above.
