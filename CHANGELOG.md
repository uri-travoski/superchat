# Changelog

## 1.3.0

Owner fixes to the 1.2 phone and desktop UI plus the features that were still "partial" in the 1.2.0 audit. Additive server API (`PUT /files/{id}/meta`, `DELETE` of channels removes everything they own); 1.0 to 1.2 apps keep working with a 1.3 server and the other way round.

* **Menus and popups**: on desktop every menu opens as a popover next to what was clicked (attach, emoji, row menus, sound menu) instead of a bottom sheet; on phones they stay bottom sheets. Audited for clipping at 1280x800 and 1000x700, light and dark, Regular and Large text.
* **Uploads (F-01)**: progress bar with bytes sent, Cancel, and Retry that keeps the files that already finished.
* **Profile photos (P-03)**: shown everywhere a person appears; Android picks the photo with the system photo picker, desktop with the file dialog.
* **Delete channel (C-06)**: from Details > Settings with the channel name typed to confirm; the server also removes pins, saved items and files.
* **Voice messages (M-30)**: waveform, duration, play and pause, seek and 1x/1.5x/2x speed; video and file cards show duration and size.
* **Sounds and badge (N-04)**: bundled notification sounds, a default and a per-conversation choice, a global sound switch, unread count on the badge (Android channel per sound).
* **Tray (E-08)**: tray icon with an unread dot and count, Status and Pause notifications menu, start minimised to the tray.
* **Sidebar order (NV-04)**: drag and drop or Move up/down for conversations and sections, synced between devices.
* **PDF preview (F-03)**: the server draws the first page (pure Rust, no poppler); broken PDFs fall back to a file card.
* **Spell check (E-05)**: red squiggles in the message box on desktop, right-click suggestions, Add to dictionary, Ignore, language choice (bundled en_AU plus system Hunspell dictionaries). On Android it is the keyboard's own spell check.
* **Video (M-31)**: Android records inside the app (CameraX, 480p) with the upload limit shown first and the size shown while recording, picks photos and videos with the system picker, and sends any file with the file picker (new: before, only Share-to-SuperChat worked). Desktop: Upload video, Record screen (needs ffmpeg on X11 or Windows), poster frame and duration.
* **Encrypted cache (X-04)**: the local cache is SQLCipher-encrypted by default, the key lives in the secret store, and an older plain cache is converted on first start.
* **Windows secret store (X-02)**: Credential Manager is type-checked and a CI job round-trips a secret on a Windows runner.
* **Push (N-01, SV-13)**: ntfy wake-ups verified end to end on the emulator against a real ntfy container (`tests/e2e/ntfy-emulator.sh`); `docker compose --profile ntfy` and its ports are configurable. FCM remains build-time optional and unrun on a device.
* **Paste (F-02)**: Ctrl+V of files and images into the message box.
* **Licences**: Inter, Noto emoji and the en_AU dictionary licences ship with the deb, AppImage, Flatpak and Windows installer (`THIRD-PARTY-NOTICES.txt`).
* Version 1.3.0 everywhere (workspace, admin panel, Android versionName 1.3.0 / versionCode 7, installer, release workflow, Docker image `superchat:1.3.0`).

## 1.2.0

A Slack-style rebuild of the desktop and phone UI (SuperChat's own theme, name, icon and icon set) plus the functions owners asked for. Additive server API; 1.0 to 1.1.x apps keep working with a 1.2 server (`min_client_version` is still 1.0.0).

* **Desktop shell**: left rail (Home, DMs, Activity, Later, More, avatar and presence menu), top bar with back/forward, a centred "Search <server>" field and help; sidebar with server menu, Threads, Drafts & sent, Later, collapsible Channels, Direct messages and custom sections (bold unread, mention badges, dimmed muted, Archived), right-click row menu; flat message rows with a hover toolbar; resizable thread panel with "Also send to #channel"; composer with a formatting toolbar and `@` and `:emoji` autocomplete; channel details modal (About, Members with Add people and remove, Settings, Files, Pins) and conversation tabs Messages, Files, Pins, Bookmarks.
* **Phone shell**: bottom navigation Home, DMs, Activity, You, each with its own Back stack; Home with search pill, sections and a New message button; conversation top bar; long-press sheet; Details screen (About, Members, Add people, Files, Pins, Bookmarks, Notifications, Mute, Lock) where every page has a back arrow to Details and then the chat; full-screen threads; You tab (status, Pause notifications, Saved items, Files, Preferences, App Lock, Devices, Sign out only when allowed, Admin panel for admins).
* **Membership management**: owners and admins add and remove people in channels and groups from the app (`POST/DELETE /conversations/{id}/members` surfaced in client-core and both UIs).
* **Save for later and labels**: per-user, private, synced saved messages with coloured labels; Later view with label filter; message sheet and hover toolbar actions.
* **Activity**: server-backed feed (All, Mentions, Threads, Reactions) with unread state and Mark all read.
* **App Lock discoverability**: Settings > Privacy & Security > App Lock explains that locked conversations are hidden until you unlock with your PIN, lists them with add and remove; row menus and Details offer Lock conversation or "Set up App Lock". The unlock screen stays generic.
* **Way back everywhere**: one Back stack with 20 layers (people picker, label picker, details sub-pages, conversation tabs, phone tabs); audit in `docs/UX-NAVIGATION.md`, tests in `apps/desktop/src/nav.rs`.
* **Admin tools in the app**: rename, topic, archive and unarchive channels (Details > Settings), Create channel, quote reply, `@channel` `@here` `@everyone` and group mentions, search highlighting and result tabs, global Files view, auto-away (P-02).
* **Feature audit** against every original requirement: `docs/FEATURE-AUDIT.md`.
* Version 1.2.0 everywhere (workspace, admin panel, Android versionName 1.2.0 / versionCode 6, installer, release workflow).

## 1.1.3

A mobile-native UX pass (owner feedback). Server API is unchanged; 1.0 to 1.1.2 apps and servers stay compatible.

* **One Back stack** (`apps/desktop/src/nav.rs`, unit tested). Android system Back, Esc and Alt+Left close the topmost popup, sheet, dialog or menu first, then pop a page (Settings sub-page, Settings, chat info, thread, search...), then the chat, then the list; only then does Android move the app to the background. Fixes "Back on Settings leaves the app" and "Back does not close Options".
* **Native popups.** Action menus are bottom sheets (drag handle, dim scrim, tap outside or swipe down to close, rounded top corners): list menu, chat menu, chat options, mute, notification level, section, message actions with a quick-reaction row, attach, emoji, sort, forward/share, the Settings pickers. Confirmations are centred alert dialogs with Cancel and OK. Forms (bot form, schedule, bookmark, new channel, certificate trust) are sheets with a visible close X and a sticky footer. Wide windows keep popovers and side panels.
* **Chat list**: one slim top bar (title, search, a light outline lock/unlock only when a PIN is set, a 3-dot menu with Settings, Threads, Sort, Unread only, Hide muted, Drafts & sent). Floating New chat button bottom-right opens a full-screen New chat page (people search; admins also get New channel). Search is its own full-screen page with the filter chips and recent searches. No permanent search box or filter pills.
* **Chat**: top bar with back arrow, avatar, name and a one-line subtitle (members, last active, typing), search and a 3-dot menu. Tapping the title opens Chat info with Members, Files, Pins, Bookmarks (and Home for apps). Pins and bookmarks are one slim banner under the bar (tap to jump, it cycles). Composer: attach, text field, Aa (formatting toolbar, hidden by default), emoji, send or mic. Long-press (right-click on desktop) a message for its action sheet.
* **Settings redesign**: profile header card with status chip, grouped rounded cards with coloured icon rows (Profile, Notifications, Appearance, Chats, Privacy & Security, About, Desktop) and sub-pages with pickers as sheets. Sign-out and Devices stay gated by Privacy options. A new vector icon set (`ui/icons.slint`, 49 icons), design tokens (spacing 4/8/12/16/24, radii, type scale) and a softer dark theme with 4.5:1 text contrast.
* **QR scan fixed** (Android). Root cause: the login screen's `scan-qr` callback was never connected to the platform hook, so the button did nothing. Now it opens the scanner; the CAMERA permission is requested at runtime with a rationale; every failure (no camera, permission refused, bad QR, scanner not starting) shows as text on the login screen; new "Choose image with QR" (photo picker, decoded with ZXing); a scanned or tapped link signs in automatically.
* **Admin panel dialogs**: every dialog has a visible close X, closes with Esc and a click on the backdrop, fits and scrolls on phone heights (390x844, 360x640) and keeps its buttons in a sticky footer. New Playwright test `admin-dialogs.mjs`.
* Client-core: `Client::create_channel`.

## 1.1.2

* Sign out and Sign out of all devices are available in the apps only for users whose admin-set Privacy options is Allow (admins and owners always have them). For Disallow (the default) the controls and the Devices list are hidden everywhere in the desktop and Android apps; users cannot end their own session by accident. Changing the setting takes effect in open apps at once (`me_update`).
* Server enforces it: `POST /logout`, own-session revoke and `POST /sessions/revoke-others` return `forbidden` ("sign-out is not allowed for this account") for members without Allow. New `POST /sessions/revoke-all` (sign out of all devices, Allow only) with a client-core `logout_all()` and CLI `logout-all`.
* Unchanged: admin panel session revocation, disabling, password reset and history clearing still sign people out and wipe their devices.
* Admin panel: the Privacy options menu explains what Allow now controls.

## 1.1.1

* Role management: the owner can change a user's role (member, admin, owner) from the admin panel's Users page, with a confirmation. `PUT /api/v1/admin/users/{id}/role`. The change applies at once to the person's signed-in devices (no new login) and their apps show or hide admin-only controls without a restart.
* Rules: only an owner changes roles; nobody changes their own role; at least one owner always remains; bots stay members; disabled users can be changed.
* Fixed: the desktop app did not treat an owner as an admin for pinning and curating.
* Admin panel: the "Privacy screen" tick box is now a "Privacy options" menu (Allow / Disallow) that saves as soon as it changes.
* Compatible with 1.0 apps and servers (`min_client_version` is still 1.0.0; the existing `me_update` event carries the new role).

## 1.1.0

* Renamed Atomic Chat to SuperChat (compatible: `ATOMIC_*` variables, old data folders, old login links, docker volume keep working). New icon.
* Login links are web addresses (`<server>/l/<token>`) that open the app and sign the person in; QR code in the admin panel; "Scan QR code" and link intent filters on Android; `superchat://` handler on Linux and Windows.
* Multiple AI providers with defaults per feature, test, masked keys, failover.
* App Lock: local 6-digit PIN, locked conversations hidden until unlocked, generic notifications.
* Admin panel usable on phones; owner setting for the public address used in links.
* Login screen: Username & password / Login link buttons, Paste, link detection in any field, exact errors.
* Fixed: deleting a user who had signed in returned 500. Server errors are now logged.

## 1.0.0

First release. See docs/RELEASE-NOTES-v1.0.md.
