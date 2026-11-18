# Changelog

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
