# PRD: Private team chat platform (server + Android, Linux and Windows apps)

Version 1.0, 17 November 2026
Scope source: the owner's ticked feature list (98 features) and notes. Anything not ticked is out of scope unless listed under "Implied dependencies". Feature IDs (M-01, C-01 and so on) are stable and can be used in commits, tests and release notes.

This document describes the product and the system behind it. It does not assume any existing server or chat protocol. The server and the protocol between server and apps are to be **designed from scratch** (section 5).

---

## 1. Product summary

A self-hosted team chat in the style of Slack, with an Element X-like look. One organisation runs one server. Administrators create all accounts. Conversations are stored readable on the server, forever by default, and every device syncs from it. The apps are small and fast, written in Rust where sensible, for Android, Linux and Windows.

Goals
1. A small, reliable server a single person can run on a modest VPS with Docker.
2. One shared Rust client core with thin UI layers, low battery use, small install size.
3. Excellent legibility: the UI must look right at **Regular** and **one size up (Large)** font sizes (the owner wears glasses).
4. Cover the 98 ticked features with the owner's modifications (section 2).

Non-goals: iOS app (a push stub only), web client, public channels, federation with other servers, end-to-end encryption, screen-reader support, group calls and huddles, canvases, billing, a third-party marketplace, connecting to Slack or other chat networks.

---

## 2. Owner modifications to the ticked features (apply over the defaults)

| Feature | Owner note | Result |
| --- | --- | --- |
| Edit message | Private channels: editable forever. 1:1 DMs and group DMs: editable only until a reply is received. | Server enforces the rule (M-08). |
| Create channel | Private only. | There are no public channels anywhere. |
| Permissions | Only admins can create channels and groups. | Server enforces; UI hides the controls from members. |
| Mobile push | FCM and APNs plus ntfy. | Android: FCM and ntfy. APNs support in the server and an iOS-ready interface, no iOS app yet. |
| Profile photo | Two letters of the name/channel/group when no photo is set. | Initials avatar (4.5). |
| Last active time | A privacy section lets the user turn it off. Privacy settings are hidden by default and only active for users selected on the server. | Server-side flag per user (SV-12). |
| Platform API | Just an API, no OAuth. | Bearer-token REST API only. |
| Invite people | Admin creates accounts; keep registration off. | No sign-up or invite links. |
| Font size, zoom, reduce motion, screen reader | Font size, zoom and reduce motion only; no screen-reader support. | No screen-reader work. |
| Time and date formats | Default Australian format. | en-AU defaults (4.6). |

---

## 3. Users and roles

| Role | Can do |
| --- | --- |
| Owner | Everything an admin can, plus change roles and server-wide settings. |
| Admin | Create/disable users, reset passwords, create/rename/archive/delete channels and group DMs and manage their members, delete files, manage bots and webhooks, view sessions. |
| Member | Chat, create DMs, react, search, upload, use bots, manage own profile and settings. |

Accounts have a username and a display name. Users sign in with server address, username and password.

---

## 4. Client requirements: platforms, stack, UX

### 4.1 Targets
- Android 8.0+ (API 26), arm64 and x86_64 (the latter for the emulator). APK on GitHub Releases.
- Linux x86_64: AppImage and `.deb`; Flatpak manifest if easy.
- Windows 10/11 x64: portable `.exe` and an installer.
- Size targets: APK under 25 MB, desktop binary under 30 MB, idle RAM under 150 MB on desktop.

### 4.2 Recommended client stack (confirm with the spike, section 9 Phase 0)
- **Core crate (shared, no UI):** Rust, `tokio`, `reqwest` + `rustls`, a WebSocket client, `rusqlite` with FTS5 for the local cache, `serde` for the API types (generated from the shared schema in section 5.2).
- **UI:** **Slint** first choice (Rust-native, small, Android + Linux + Windows). Fallback: **Tauri 2 with Svelte or Solid**. Not egui (weak Android IME and text input).
- **Android shell:** small Kotlin/Gradle wrapper for what Rust cannot do well: foreground service, push receivers (FCM and ntfy/UnifiedPush), notification channels, share target, camera/microphone permissions, Keystore.
- **Desktop glue:** tray icon, OS notifications, launch on login, secret store (Credential Manager, libsecret).

**Spike gate (must pass first):** in a minimal app on an Android emulator confirm (a) typing with IME, emoji and autocorrect, (b) Regular and Large font scale render without clipping, (c) a foreground service holds a realtime connection with the screen off, (d) a notification opens the right conversation, (e) APK size within target. If any fails, switch to the fallback stack and record why.

### 4.3 Look and layout (Element X-like)
- Clean, rounded, high-contrast. Conversation list with circular avatars, bold unread names, unread badges, last message preview and time; filter chips (All, Unreads, People, Channels, Favourites).
- Timeline with bubbles (own on right), grouping by sender and day, day dividers, reaction chips under bubbles. Long-press (mobile) or hover/right-click (desktop) opens an action sheet with quick reactions on top.
- Composer with formatting toolbar, attach, voice-message and send buttons.
- Thread view as a side panel (desktop) or full screen (mobile).
- Adaptive: phone one pane; wide windows two panes (list + timeline), three with thread or details.
- Design tokens (colour, spacing, radius, type scale) in one place; light and dark themes.
- Main areas: Chats, Threads, Search, Settings. Room details tabs: Messages, Files, Pins (channel bookmarks shown on top). No Canvas tab.

### 4.4 Font size and legibility (hard requirement)
- Font scale with three steps: **Regular (100%)**, **Large (115%)**, **Extra large (130%)**. Regular and Large are QA-guaranteed; Extra large is best effort.
- On Android multiply by the system font scale; cap the combined scale at 200%.
- Desktop zoom with Ctrl +, Ctrl -, Ctrl 0 and a setting. Zoom scales the whole UI; font size scales text only.
- No fixed-height text containers; labels wrap instead of truncating (names may ellipsize in the list only, full name in the header). Touch targets at least 48 dp, growing with scale.
- Body text contrast at least 4.5:1 in both themes; minimum body size 16 sp/px at Regular.
- Acceptance: automated screenshot tests at Regular and Large for every main screen (conversation list, timeline, thread, composer, details, search, settings, login) on a small phone (360x640 dp) and a desktop window (1000x700). No clipped or overlapping text.
- **Reduce motion** setting turns off transitions and animated reactions.

### 4.5 Avatars
Show the photo if set, otherwise **two letters** on a coloured tile: two or more words use the first letter of the first two words ("Dan Smith" = "DS"); one word uses its first two letters ("general" = "GE"); strip leading `#` and `@`; uppercase. Colour comes from a stable hash of the user/conversation id in a fixed palette. Channels and groups follow the same rule.

### 4.6 Date and time (default Australian)
Locale `en-AU`. Full dates `DD/MM/YYYY`, "Today", "Yesterday", weekday names within a week. Time as `3:45 pm` (12-hour) with a 24-hour setting. Week starts Monday. Server times are UTC; display in the device time zone.

### 4.7 Spell check
Use the OS/IME checker on Android; the OS checker on desktop (Windows spell-check API, Hunspell on Linux). Language follows the system.

---xmpp

## 5. Server and protocol: design from scratch

### 5.1 Principles
- **Server is the source of truth.** The app's database is a cache. If the server reports the account disabled or the session revoked, the app wipes local data. Messages removed on the server disappear from the app after reconciliation.
- **Readable on the server**, no end-to-end encryption (needed for search, summaries, retention). TLS everywhere.
- **Simple to run:** one container image plus object storage, SQLite (WAL) by default, PostgreSQL optional later. Target: up to 500 users, 200 concurrent connections, on 1 vCPU and 1 GB RAM.
- Server language: Rust recommended (shares types with the client), final choice is the implementer's, record it.
- Registration closed; only admins create accounts.

### 5.2 API design (the implementer designs the exact schema; these are constraints)
- **Transport:** HTTPS JSON API for commands and queries plus one authenticated **WebSocket** per device for realtime events and typing/presence. One versioned schema file (OpenAPI or JSON Schema) is the single source for server and client types; breaking changes bump the API version and the server reports `min_client_version`.
- **Auth:** `POST /login` (username, password, device name) returns an access token (short-lived) and a refresh token tied to a device session. Passwords hashed with Argon2id. Rate limiting and temporary lockout on repeated failures. Sessions can be listed and revoked (SV-11).
- **Sync model:** every change a user can see is an event in a per-user ordered log with a monotonically increasing cursor. The client stores its cursor and asks "give me everything after cursor N" on connect; realtime events arrive on the WebSocket with the same cursor. Messages have a server id, a conversation-local sequence number and a UTC timestamp; the client sends a client-generated id for idempotent sends (retries never duplicate). Deletes are tombstones; edits keep a version number.
- **Pagination:** history by conversation with `before`/`after` cursors, by id or by date (for "jump to date").
- **Idempotency and ordering:** per-conversation ordering is guaranteed; clients tolerate out-of-order realtime delivery by sequence number.
- **Errors:** a small stable list of machine-readable codes (for example `account_disabled`, `session_revoked`, `edit_window_closed`, `forbidden`, `file_too_large`).

### 5.3 Core data model
User, Device session, Conversation (kind: `dm`, `self`, `group_dm`, `channel`; private only), Membership (role in conversation), Message, Reaction, Attachment, Pin, Bookmark, User group, Draft, Scheduled message, Read state, Favourite, Sidebar section, Notification setting, Push registration, Bot, Webhook, User privacy settings, Presence. Message body is plain text in a small formatting markup (section 6.2 M-12..M-15); the server stores and returns it verbatim and never executes or renders it.

### 5.4 Server requirements (SV-xx)

| ID | Requirement |
| --- | --- |
| SV-01 | Account management by admins only: create, disable/enable (closes sessions at once and blocks sign-in), reset password (signs the user out), delete with all their data. Registration endpoints do not exist. |
| SV-02 | Authentication and sessions as in 5.2; per-device sessions with name, client version, last seen, IP. |
| SV-03 | Conversations: create/rename/topic/archive/unarchive/delete channels and group DMs **by admins only**; members-only and hidden from non-members; DMs and the notes-to-self conversation created on demand by any member. Membership managed by admins (admin panel and API). |
| SV-04 | Message store: append, edit (with the rule below), delete (tombstone), reactions, thread parent/reply references, mentions, attachments. All kept forever by default; configurable retention is a later option. |
| SV-05 | **Edit rule (authoritative on the server):** author only. In `channel` conversations always allowed. In `dm` and `group_dm`: allowed only while no other participant has posted a message after the original (error `edit_window_closed` otherwise). In `self`: always. The client mirrors the rule for UI but the server decides. |
| SV-06 | Realtime delivery over WebSocket to all of the user's devices, typing indicators, presence, read-state sync, delivery states (accepted by server, delivered to at least one recipient device). |
| SV-07 | Attachments: upload API with progress-friendly streaming, configurable maximum (default 100 MB), content-type recorded, pluggable storage backend (local disk and S3-compatible object storage required; others optional), download via authenticated or short-lived signed URLs, previews/thumbnails for images optional. Only conversation members can fetch a file. Admin can delete a file (object removed, message shows "File removed"). |
| SV-08 | Search: full-text index over messages and file names with filters (from, in, before/after, has link/file/reaction); results limited strictly to conversations the caller belongs to. People search and own-channel search included. |
| SV-09 | Pins and channel bookmarks per conversation (limit 100 pins), written by owner/admin, readable by members, with realtime events. User groups (`@design-team`) managed by admins. |
| SV-10 | Scheduled messages: store, list, cancel, send as the user at the due time, survive restarts. |
| SV-11 | Sessions: list and revoke own sessions; admins can list and revoke anyone's. |
| SV-12 | **Privacy flag:** per-user `privacy_controls_enabled` set only by an admin (default off) and `hide_last_active` set by the user only if the flag is on. When `hide_last_active` is true the server never returns that user's last-active time. `GET /me` exposes the flag so the app shows or hides the Privacy screen. |
| SV-13 | Push: register FCM tokens and ntfy topics (APNs token field accepted, delivery implemented later) per device; send **content-free wake-up pushes** when a user has an unread-worthy event and no live connection; honour mute and do-not-disturb; ntfy server URL configurable (self-hosted or public). |
| SV-14 | Roles (owner/admin/member) stored per user and enforced on every endpoint. |
| SV-15 | Bots: admin creates bots and bearer tokens. Bots post into rooms they are members of, send formatted text and interactive forms, receive messages by polling or webhook. **Bearer-token REST API only, no OAuth.** Documented in the repository. |
| SV-16 | Webhooks: incoming (a secret URL posts into a conversation), outgoing (conversation events POSTed to configured URLs with a signature header and retries). Managed by admins. |
| SV-17 | Interactive messages: a JSON "form" schema (text, multi-line, boolean, select, multi-select, buttons) that bots can send; the app renders it natively and submits the response back to the bot. Bots can also publish a "Home" page (blocks: header, text, divider, button, form) shown in the app. |
| SV-18 | Calls: signalling for 1:1 audio/video calls over the WebSocket (offer, answer, ICE candidates, ring, hang up) and a TURN relay (coturn) with short-lived credentials issued by the API. |
| SV-19 | Admin web panel (new): overview, users, roles, conversations and members, bots, webhooks, storage settings with a "test" button, sessions, privacy flag per user, backups. |
| SV-20 | Backups: encrypted full backup of database and settings, scheduled daily with retention count, manual "back up now", restore onto a fresh server that reconnects to the same attachment storage. |
| SV-21 | Optional AI features, off by default, configured in admin settings with a provider and key: natural-language search, channel recap/summary. The admin panel states that message text is sent to the provider. |
| SV-22 | Deployment: Docker image, `docker compose` file, TLS via an automatic certificate option or a reverse proxy, health endpoint, structured logs, metrics endpoint optional. Server address is configured by the admin; nothing about the domain is hard-coded. |
| SV-23 | Security: input validation, request size limits, per-user and per-IP rate limits, CORS closed, no secrets in logs, dependency audit in CI. |
| SV-24 | Test suite: unit tests plus an end-to-end test that boots the server in Docker with a throwaway database and a fake S3, driven by the headless client core (login, DM, channel, thread, reaction, edit rule, retraction, upload, sync after offline, disabled-account behaviour, search isolation between users). |

---

## 6. Client functional requirements

Legend for "Part": C = client only, S = needs server work (SV-xx noted), B = both. **Bold** marks an owner modification.

### 6.1 Account, connection and sync (implied dependencies)
| ID | Requirement |
| --- | --- |
| X-01 | Login screen: server address, username, password. No sign-up. TLS required; for test servers with a self-signed certificate show the fingerprint and let the user pin it (trust on first use). Never accept silently. |
| X-02 | Tokens and credentials stored in the platform secret store. |
| X-03 | Realtime connection with automatic reconnect and backoff, cursor-based catch-up (5.2), pausing when the app is in the background on mobile. |
| X-04 | Local SQLite cache of conversations, messages, reactions, drafts, settings, with optional encryption (key in the secret store). |
| X-05 | **Server is source of truth:** on `account_disabled` or `session_revoked` wipe all local data and return to login; reconcile cached history with the server so messages deleted server-side disappear. |
| X-06 | Android foreground service with a low-priority persistent notification, user-toggleable; with push configured the service may stop and push wakes the app. |
| X-07 | Handle forced sign-out (admin password reset) with a clear message. |
| X-08 | Minimal emoji picker (recent, common set, OS emoji keyboard) for reactions and the composer. Custom emoji out of scope. |

### 6.2 Conversations and messaging
| ID | Feature | Part | Requirement |
| --- | --- | --- | --- |
| M-01 | Private channels | B | Members-only, hidden; appear in the list for members automatically. |
| M-02 | Direct messages (1:1) | B | Started from people search; stored on the server. |
| M-03 | Group DMs | B | Small private group created by an admin (see Q1). |
| M-04 | Message yourself | B | Pinned self-conversation; no delivery or typing indicators. |
| M-05 | Threads | B | Reply with a thread root reference; panel shows root and replies. |
| M-06 | Threads view | B | List of threads I started or follow, newest activity first, unread counts. |
| M-07 | Follow/unfollow thread | B | Follow set synced across devices; replying or being mentioned auto-follows. |
| M-08 | Edit message | B | **Private channels: always. 1:1 and group DMs: only until another participant has posted a later message. Self: always.** SV-05 enforces; client hides Edit when closed and handles `edit_window_closed`. |
| M-09 | "(edited)" label | B | Shown when a version exists. |
| M-10 | Delete own message | B | Tombstone, shown as "Message deleted". |
| M-11 | Emoji reactions | B | Add/remove, chips with counts, tap toggles. |
| M-12 | Text formatting | C | Bold, italic, strike, inline code in the message markup. |
| M-13 | Code blocks | C | Monospace, horizontal scroll, copy button. |
| M-14 | Quotes and lists | C | Block quote, bullet and numbered lists. |
| M-15 | Rich text toolbar | C | Buttons inserting the markup, live preview. |
| M-16 | @mentions | B | Autocomplete from members; highlighted; counts as a notification trigger; mentions stored as structured references. |
| M-17 | @channel / @here / @everyone | B | `@channel` and `@everyone` = all members; `@here` = members currently online; rendered as pills. |
| M-18 | User groups | B | Autocomplete from groups; expands to member mentions on send (SV-09). |
| M-19 | Message permalinks | B | "Copy link" gives an app deep link with conversation and message id; opening scrolls to it. |
| M-20 | Share/forward | B | Forward a message (with attribution) to another conversation. |
| M-21 | Quote reply | B | Reply with a quoted preview. |
| M-22 | Drafts | B | Per-conversation drafts saved locally and synced. |
| M-23 | Drafts & sent view | C | One list of drafts and scheduled messages. |
| M-24 | Schedule send | B | Date/time picker, SV-10; list and cancel in M-23. |
| M-25 | Typing indicators | B | Throttled; shown in header and list. |
| M-26 | Unread and mark as unread | B | Per-conversation read state synced; "Mark as unread"; badge counts on list and app icon. |
| M-27 | Multi-device sync | B | History, read state, drafts, favourites and settings follow the user. |
| M-28 | Offline delivery | B | Persisted outbox with idempotent ids; pending/failed states with retry. |
| M-29 | Jump to date / history | B | Date picker queries history by date; infinite scroll for older messages. |
| M-30 | Voice messages | B | Record in app (Opus or AAC), waveform, upload, inline player with speed control. |
| M-31 | Video clips / screen recordings | B | Video recording on mobile, screen recording on desktop, upload; show size limit first. |
| M-32 | Delivery / sent / failed status | B | Sending, sent, delivered, failed (with retry). |

### 6.3 Channels
| ID | Feature | Part | Requirement |
| --- | --- | --- | --- |
| C-01 | Rename | B | Owner/admin. |
| C-02 | Topic and description | B | Shown under the title; owner/admin edit. |
| C-03 | Create channel | B | **Admin only. Private only.** Name, topic, members. |
| C-04 | Archive | B | Read-only, moves to an "Archived" section. |
| C-05 | Unarchive | B | Reverse of C-04. |
| C-06 | Delete channel | B | Owner/admin with typed-name confirmation; removes messages and files on the server. |
| C-07 | Channel bookmarks | B | Link/file shortcuts at the top; owner/admin edit (SV-09). |
| C-08 | Channel tabs | C | **Messages, Files, Pins** (no Canvas). |
| C-09 | Pinned messages | B | Pin/unpin from the message menu; Pins tab jumps to the message (SV-09). |

### 6.4 Navigation and organisation
| ID | Feature | Part | Requirement |
| --- | --- | --- | --- |
| NV-01 | Pin a channel | B | Favourite star; favourites in a top section. |
| NV-02 | Pin a DM | B | Same list. |
| NV-03 | Pin a group | B | Same list; favourites sync across devices. |
| NV-04 | Custom sidebar sections | B | Named sections, drag to add/reorder, collapse; synced. |
| NV-05 | Sorting and unreads filter | C | Sort by activity or A-Z; "Unreads only". |
| NV-06 | Hide/show muted | C | Toggle; muted dimmed. |
| NV-07 | Mute channel or DM | B | 1 h, 8 h, until tomorrow, forever; synced; server skips push for muted conversations. |
| NV-08 | Quick switcher | C | Ctrl/Cmd+K on desktop; fuzzy search field on mobile. |
| NV-09 | Back/forward, jump to unread | C | Alt+Left/Right; "Jump to first unread" button. |

### 6.5 Search
| ID | Feature | Part | Requirement |
| --- | --- | --- | --- |
| SR-01 | Message search | B | SV-08 plus local cache for instant results offline; grouped by conversation with highlights. |
| SR-02 | Filters | B | Chips: from, in, before/after, has:link/file/reaction. |
| SR-03 | Search files | B | File name and message text; type and size shown. |
| SR-04 | Search people | B | By name or username; starts a DM. |
| SR-05 | Search channels | B | Only channels I belong to. |
| SR-06 | Recent and saved searches | C | Stored locally. |
| SR-07 | AI natural-language search | B | Shown only if the server reports AI search enabled (SV-21); labelled that text goes to the provider. |

### 6.6 Files
| ID | Feature | Part | Requirement |
| --- | --- | --- | --- |
| F-01 | Upload files | B | Progress, cancel, retry, size limit from the server. |
| F-02 | Drag-drop and paste | C | Desktop drop and paste screenshots; mobile picker and camera. |
| F-03 | Previews | C | Inline image, video, audio; PDF preview or system viewer; lazy download and capped cache. |
| F-04 | Share file to several channels | C | "Share to…" re-posts the same file reference without re-uploading. |
| F-05 | Files browser | B | Files tab per conversation and a global Files view; filter by type (SV-07, SV-08). |
| F-06 | Comments on files | B | Replying to a file message starts a thread. |
| F-07 | Admin deletes files | B | Admin-only; confirmation; "File removed". |

### 6.7 Forms, calls, notifications
| ID | Feature | Part | Requirement |
| --- | --- | --- | --- |
| WF-01 | Workflow forms | B | Render form messages (SV-17) as a card or modal; submit response to the sender. |
| CL-01 | 1:1 voice and video calls | B | WebRTC (Rust `webrtc` library or native), signalling and TURN from SV-18. Ring screen, mute, camera flip, speaker. Riskiest item; schedule last. |
| N-01 | Mobile push | B | Android: **FCM and ntfy**, user chooses in Settings, ntfy URL configurable; APNs interface present for later. Pushes carry no content; the app wakes, syncs, then posts the local notification. |
| N-02 | Desktop notifications | C | Native toasts with sender and optional preview; click opens the conversation; inline reply where supported. |
| N-03 | Per-channel notification settings | B | All / Mentions only / Nothing per conversation, synced. |
| N-04 | Sounds and badges | C | Per-channel sound, global on/off, badge on launcher/taskbar icon. |

### 6.8 Presence and profile
| ID | Feature | Part | Requirement |
| --- | --- | --- | --- |
| P-01 | Online/away/DND | B | Presence dot; user can set Online, Away, Do Not Disturb (DND silences notifications). |
| P-02 | Auto-away | C | Desktop: away after 10 min idle. Mobile: away when backgrounded. |
| P-03 | Profile photo | B | Upload and crop; **initials fallback** (4.5). |
| P-04 | Display name | B | Editable in profile. |
| P-05 | Last active time | B | "Last active 3:45 pm" in DM headers and profiles. **Privacy screen in Settings hidden unless the server says `privacy_controls_enabled`; it holds "Hide my last active time" (SV-12).** |

### 6.9 Apps and automation
| ID | Feature | Part | Requirement |
| --- | --- | --- | --- |
| A-01 | Incoming webhooks | S | SV-16; bot/webhook messages show an "APP" tag. |
| A-02 | Outgoing webhooks / events | S | SV-16. |
| A-03 | Modals and forms | B | Same renderer as WF-01; buttons can open a modal form. |
| A-04 | App Home tab | B | A bot's profile has a Home tab rendered from blocks (SV-17). |
| A-05 | Platform API | S | **Bearer-token REST only, no OAuth** (SV-15). |

### 6.10 Admin and security
| ID | Feature | Part | Requirement |
| --- | --- | --- | --- |
| AD-01 | Invite people | S | **None in the app; admin creates accounts; registration off** (SV-01). |
| AD-02 | Roles | B | Owner/admin/member; admin-only controls hidden for members (SV-14). |
| AD-03 | Deactivate accounts | B | SV-01; the app wipes local data (X-05). |
| AD-04 | Reset passwords / force sign-out | B | SV-01; app handles it (X-07). |
| AD-05 | Device and session management | B | Settings > Devices lists sessions, signs out others (SV-11). |
| AD-06 | Data export and backups | S | SV-20; no client UI. |
| AD-07 | Permissions | S | **Only admins create channels/groups** (SV-03). |
| AD-08 | Custom domain | B | Server address entered at login; nothing hard-coded. |
| AD-09 | Data residency | S | Self-hosted; no telemetry or third-party analytics; crash reports only with opt-in. |
| AD-10 | Transport encryption | B | TLS 1.2+ everywhere, certificate validation on, no plaintext fallback. |

### 6.11 Experience and settings
| ID | Feature | Part | Requirement |
| --- | --- | --- | --- |
| E-01 | Dark mode, themes, sidebar colours | C | Light, Dark, System; 4 to 6 accent/sidebar presets. |
| E-02 | Compact layout | C | Comfortable and Compact density; Compact must still pass the Regular/Large tests. |
| E-03 | Font size, zoom, reduce motion | C | As 4.4. **No screen-reader work.** |
| E-04 | Time and date formats | C | **Default Australian** (4.6), 12/24-hour setting. |
| E-05 | Spell check / autocorrect | C | 4.7. |
| E-06 | Mobile share extension | C | Android share target for text, images, files; pick conversation, caption, send. |
| E-07 | Offline mode and draft sync | B | Opens instantly from cache, offline banner, queued sends (M-28), synced drafts (M-22). |
| E-08 | Launch on login and tray icon | C | Desktop only: optional start minimised, tray menu (Open, Status, Quit), unread indicator. |

---

## 7. Non-functional requirements
- **Performance:** cold start to a usable list under 1.5 s on a mid-range phone with a warm cache; 60 fps scrolling with 10,000 cached messages in a conversation; server p95 under 200 ms for send and history calls at the target load.
- **Battery:** with push configured and screen off, wake only for messages; no wake locks beyond the foreground service when push is off.
- **Reliability:** no message loss on network drops (idempotent client ids); no duplicates after retries.
- **Security and privacy:** no secrets in logs, deep links validated, downloads sandboxed, message text never rendered as HTML, push payloads content-free, no analytics.
- **Localisation:** English (Australia) only; strings in a resource file for future languages.
- **Accessibility scope:** font size, zoom, contrast, reduce motion only.

## 8. Testing and acceptance
- Unit tests: edit-rule logic, avatar initials, date formatting, search filters, sync cursor handling, markup parsing.
- Server e2e (SV-24) and client integration tests with a headless client core against a Docker server.
- UI screenshot tests at Regular and Large, light and dark, phone and desktop sizes, for all main screens.
- Manual checks on the Android emulator and real desktop builds. Do not install on a physical device without the owner's explicit permission.
- Definition of done per feature: implemented, tested, works at Regular and Large, listed in the README feature table.

## 9. Delivery plan
- **Phase 0, foundation and spike (gate):** repository layout, schema file, server skeleton with login and a WebSocket, client spike per 4.2, one DM end to end on Android and desktop.
- **Phase 1, core chat:** SV-01..SV-07, SV-14, SV-19 (basic), SV-22, SV-24; X-01..X-08, M-01..M-04, M-08..M-13, M-16, M-25..M-29, M-32, C-01..C-03, P-01..P-04, E-01..E-04, F-01..F-03, N-02.
- **Phase 2, organisation and search:** SV-08..SV-13, SV-20; M-05..M-07, M-14, M-15, M-17..M-23, M-30, M-31, C-04..C-09, NV-01..NV-09, SR-01..SR-06, F-04..F-07, P-05, AD-02, AD-05, AD-07, E-05..E-08, N-01, N-03, N-04.
- **Phase 3, automation and extras:** SV-10, SV-15..SV-17, SV-21; M-24, WF-01, A-01..A-05, SR-07.
- **Phase 4, calls:** SV-18, CL-01.
Each phase ships as a tagged build (APK, AppImage, deb, Windows installer, server image) with release notes listing feature IDs.

## 10. Out of scope (not ticked)
iOS app (push interface only), web client, public channels and channel directory, joining/leaving, in-app member management (admin panel handles it), moderator deletion of others' messages, custom emoji, GIF search, link previews, polls, custom status, canvases, lists, workflow automation builder, huddles and group calls, calendar integrations, multiple workspaces, enterprise search, SSO and 2FA, SCIM, audit logs, data-loss rules, retention-policy UI, connecting to other chat networks.

## 11. Assumptions and open questions (defaults applied unless the owner says otherwise)
- **Q1** Group DMs: ticked, and only admins create channels/groups, so admins create group DMs too.
- **Q2** "Until a reply is received" means until any other participant posts a later message in that conversation.
- **Q3** "Turn off privacy settings" means "hide my last active time"; the whole Privacy screen is visible only to users an admin selects.
- **Q4** Pins and bookmarks are editable by owner/admin only.
- **Q5** Member management is done by admins (panel and API), not inside the app.
- **Q6** Australian time default is 12-hour with a 24-hour switch.
- **Q7** Carried-over design decisions to confirm: accounts created by admins only, messages readable on the server and kept forever, no federation, 100 MB upload limit, attachments in S3-compatible or local storage (cloud-drive storage is optional), encrypted backups.
- **Q8** The Slack feature list was written from knowledge of Slack up to mid-2026; behaviour follows this PRD, not live Slack.

## 12. Appendix: traceability
Ticked features by group: Messaging M-01..M-32 (32), Channels C-01..C-09 (9), Navigation NV-01..NV-09 (9), Search SR-01..SR-07 (7), Files F-01..F-07 (7), Forms WF-01 (1), Calls CL-01 (1), Notifications N-01..N-04 (4), Presence P-01..P-05 (5), Apps A-01..A-05 (5), Admin AD-01..AD-10 (10), Experience E-01..E-08 (8). Total 98. X-01..X-08 and SV-01..SV-24 are implied dependencies added to make those features work.



a disabled user being logged out with their old chats removed from the phone. once logged in all recent chats come back + load old chats as user scrolls up.

privacy section where user can turn off privacy settings in the client app settings - privacy settings should only become active for selected users (selection from server)

Apps need to be fast (rust based may be?)