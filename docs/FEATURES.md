# What SuperChat does today (version 1.4.1)

* Self-hosted team chat. One small server (Docker), apps for Linux and Android (Windows later), an admin web panel, and a command-line client.
* **Accounts are created by admins only.** There is no sign-up. New people get a one-time login link (works once, expires in 7 days): a web address that opens the app and signs them in, also as a QR code.
* **Private by design:** channels and group chats are members-only and invisible to everyone else; only admins create them. Direct messages and
  "notes to self" are created on demand.
* **Messages:** edit (channels and notes always; direct chats until the other person replies), delete (leaves "Message deleted"), reactions,
  typing indicators, unread counts that follow you between devices, search across only the chats you can see, history by date.
* **Works offline:** messages you send without a connection wait in a queue and go out when you are back; the app opens instantly from its cache.
* **Files:** up to 100 MB each (configurable). Stored in your own S3, Cloudflare R2, Backblaze B2, MinIO or Google Drive, not on the server's disk.
  Admins can move everything to another storage with a check afterwards.
* **Admin control:** disabling someone signs them out of every device immediately and wipes the app's local data. "Clear history" backs the
  person up first, then removes only their own copy of chats; other people keep theirs. If the backup cannot be written, nothing is deleted.
* **Reach it anywhere:** works with a domain, a bare IP address, or a Cloudflare Tunnel.
* **Text size:** Regular and Large, and it follows the Android system font size.

Not included: end-to-end encryption (the server can read messages), voice/video calls, in-app video recording on Android (removed in 1.4), public sign-up, custom emoji, screen-reader support (removed in 1.4).
See `STATUS.md` for what is partly built.

## Phase 2

* Threads (panel and a Threads list with unread), pins, bookmarks bar, files tab, forward, copy a link to a message, drafts that follow you between devices.
* Search with filter chips (from, in, has link/file/reaction, before, after), people, files and channels in the results, recent searches.
* Favourites, sections, mute for a time, notify level per conversation, sort, unread only, hide muted, mark as unread, quick switcher (Ctrl+K), back and forward (Alt+Left/Right).
* Rich messages: bold, italic, strike, code, code blocks, lists, quotes, links, @mentions with autocomplete, emoji picker, reactions with counts, inline pictures.
* Navigation (1.1.3): one Back stack (Android Back, Esc, Alt+Left), bottom sheets for menus and pickers, alert dialogs for confirmations, full-screen pages, floating New chat button, Chat info page, pinned/bookmark banner, long-press message actions, grouped Settings with sub-pages, Android QR scan with image fallback.
* Settings: theme and accent, text size (Regular/Large/Extra large), zoom (Ctrl + / - / 0), compact, reduce motion, 12/24 hour, status, profile, devices, sign out and privacy (devices, sign out and sign out of all devices only when your admin sets Privacy options to Allow).
* Admin: chat viewer, bulk delete with backup, encrypted backups, selective restore, restore onto a new server.
* Not built yet: voice/video clips, drag and drop, Android share target, tray icon, Android Keystore, push registration on the device.

## 1.1 additions

* **Clickable login links and QR codes** (`<server>/l/<token>`): the page opens the app (Android, Linux, Windows) or offers the download; the apps also take the link pasted, and Android scans the QR code.
* **Multiple AI providers** with per-feature defaults, test button, masked keys and failover.
* **App Lock**: a 6-digit PIN kept on the device only; chosen conversations are hidden from the list, search, switcher, threads, counts, drafts and notifications until you unlock. Padlock at the top of the list; locks again after 5 idle minutes, 5 minutes in the background, or on tap.
* **Phone-friendly admin panel** (cards, menu button, single-pane chat viewer).
* **Friendlier login screen**: Username & password or Login link buttons, Paste button, a link pasted into the server field is recognised, exact error messages.

## 1.2 additions

* **Slack-style desktop and phone UI** (rail or bottom tabs, sidebar sections, hover toolbar, thread panel, details with tabs, Activity, Later, search tabs). SuperChat's own theme and icons.
* **Add people to channels and groups** (owners and admins), remove people, rename, topic, archive from the app.
* **Save for later with labels**, a Later view, and a server-backed Activity feed.
* **App Lock** is discoverable: Settings explains it and lists locked conversations; row menus offer Lock conversation.
* **A way back from every screen**; see `docs/UX-NAVIGATION.md`. The full feature-by-feature check is `docs/FEATURE-AUDIT.md`.

## 1.3 additions

* **Uploads** show progress and can be cancelled or retried; **voice messages** have a waveform, duration, seek and 1x/1.5x/2x speed; **video clips** can be attached from the picker on Android (desktop can also record the screen), with a poster frame and duration; **PDFs** show their first page.
* **Profile photos** everywhere a person appears (Android picks from the system photo picker). **Delete a channel** from Details with the name typed to confirm.
* **Notification sounds** per conversation and a default, plus an unread number on the badge; a **tray icon** on desktop with Status and Pause notifications.
* **Sidebar order** by drag and drop or Move up/down, synced between devices.
* **Spell check** with red squiggles, suggestions, Add to dictionary and a language choice (desktop; Android uses the keyboard).
* **Encrypted local cache** (SQLCipher) and the Windows Credential Manager for secrets; **ntfy push** tested end to end.
* **Paste** files and images with Ctrl+V; Android can pick any file, photo or video from the attach sheet.
