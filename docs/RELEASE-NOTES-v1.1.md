# SuperChat v1.1

Atomic Chat is now **SuperChat**. A 1.0 server and 1.0 apps keep working together (`min_client_version` is still 1.0.0).

## New

* **Tap-to-sign-in links.** The one-time login link is now a normal web address, `https://<your-server>/l/<token>` (plain `http://` on a private LAN works too).
  Opening it shows a page with a big **Open in SuperChat** button that also tries to open the app by itself (Android via an intent link, Linux and Windows via the
  `superchat://` handler). Without the app it offers the downloads, a **Copy login link** button and manual steps. The page only checks the link; the link is used up when the app signs in.
  The page loads nothing from other sites, is never cached and sends no referrer. Expired, used and unknown links give a clear message.
* **QR code** below the link in the admin panel (the QR holds the same web address, so a phone camera opens the page and the app). **Copy** and **Share** buttons.
  Android has **Scan QR code** on the sign-in screen.
* **Public address**: `SUPERCHAT_PUBLIC_URL`, else an owner setting in the admin panel, else the address you opened the panel with.
* **Downloads**: files in `SUPERCHAT_DOWNLOADS_DIR` (default `<data>/downloads`) are offered on the link page and at `/downloads`; otherwise `SUPERCHAT_RELEASE_URL`.
* **Sign-in screen** on phones: two big buttons (Username & password, Login link), a **Paste** button, a link pasted into the server field (even with other text around it) is recognised and
  switches to link mode, and the error says exactly what is wrong (cannot reach the server, plain http blocked, link invalid, expired or already used).
* **Several AI providers** (Anthropic, OpenAI, OpenAI-compatible): add, edit, delete, test, per-feature default (search, recap), masked keys, automatic failover. A 1.0 provider migrates to "Default".
* **App Lock**: Settings > App Lock sets a 6-digit PIN (typed twice). Conversation options > **Lock this conversation** hides it whenever the app is locked: from the list, search, quick switcher, threads,
  unread counts and drafts; notifications about it say only "SuperChat - New message". A padlock at the top of the list shows the state; tap it to enter the PIN (wrong PIN: just "Incorrect PIN"). The app starts locked and locks again after 5 minutes without
  input, after 5 minutes in the background, or when you tap the padlock. Five wrong PINs in a row block entry for 30 s, then the wait doubles. Changing or removing the PIN needs the current one.
* **Admin panel on phones**: menu button, cards instead of tables, touch-sized controls, single-pane chat viewer. Tested at 390x844.
* **Fixed**: deleting a user who had ever signed in returned `500 internal`. Deleting now cascades (sessions, devices, memberships, visibility, drafts, preferences, push registrations, bot tokens, login links), keeps other people's copies of chats, and server errors are logged (route, status, cause; never secrets).

## Upgrading from 1.0 (rename migration)

* Environment: `ATOMIC_*` still works; `SUPERCHAT_*` is accepted too and wins. An existing `.env` needs no change.
* Docker: the image is `superchat-server`; to keep the 1.0 data volume set `SUPERCHAT_DATA_VOLUME=atomic-chat_atomic-data`. See [DEPLOY.md](DEPLOY.md).
* Desktop apps move their data folder and still read the old saved sign-in. Old `atomic-chat://` links and the 1.0 session are still accepted.
* Android has a new application id (`app.superchat`): install it, sign in with a new link, then remove the old app.
* Proxies must pass `/l/` and `/downloads` to the server (the shipped Caddy and tunnel configs do).

## Honest limitations

* **App Lock hides chats in the app; it does not encrypt the local cache.** Someone with the unlocked device and file access can read the cache. A 6-digit PIN has only a million values; the delays protect the screen, not the disk. If you forget the PIN you sign out (that erases the lock with the local data).
  Locks are per device and are not synced. Android task-switcher thumbnails are not blanked.
* Android may show an "Open with" chooser the first time you tap a web link (app links are not verified). The landing page opens the app either way. Opening a link that points at a plain `http://` public address is refused by the apps by design (private LAN only).
* Verified on the Android emulator: the landing page in Chrome opened SuperChat and signed in, and a `http://<server>/l/<token>` VIEW intent addressed to the app signed in directly. A plain `am start -d <link>` goes to the default browser (the filters are not verified app links), which then opens the app.
* QR scanning was exercised on the emulator with injected links, not with a real camera. The QR encoder supports links up to about 270 characters (version 10, level M); longer links show "copy instead".
* The AppImage does not register the `superchat://` handler by itself (use a desktop integration tool such as appimaged; the .deb does it). Windows registry handling and the installer are untested on Windows, as in 1.0.
* Unchanged from 1.0: Google Drive, FCM, APNs, ntfy and AI providers were only tested against local fakes; Windows and arm64 were never executed; calls and end-to-end encryption are not built.

## Tests at release

`cargo test --workspace`: all green (about 148 tests, including App Lock, landing page, user deletion, AI providers, link parsing, desktop link hand-off); `tests/e2e/run.sh` green; Playwright admin-smoke, admin-viewer, admin-automation and admin-mobile (390x844) green; clippy 0 errors.
