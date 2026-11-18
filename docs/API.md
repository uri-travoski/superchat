# API (v1)

Types are defined once in `crates/proto` (serde + schemars). `cargo run -p atomic-proto --bin gen-schema` writes
`schema/api.schema.json`; the admin panel's `src/api-types.ts` is generated from it (`npm run gen-types`).
Base path `/api/v1`. `GET /meta` returns `api_version`, `min_client_version`, `server_version`, `setup_required`,
`max_upload_bytes`. Clients send `X-Client-Version` and are refused with `client_too_old` (HTTP 426) below the minimum.

Errors: `{"error": {"code": "<stable code>", "message": "<plain English>"}}`. Codes: unauthorized, invalid_credentials,
token_expired, session_revoked, account_disabled, password_reset, history_cleared (these last four end the session and wipe
the device), locked_out, rate_limited, forbidden, not_found, validation, conflict, edit_window_closed, file_too_large,
cursor_expired (410), client_too_old, setup_required, setup_done, backup_failed, storage_error, not_implemented, internal.

Auth: `POST /login`, `POST /login/link`, `POST /refresh` (stable refresh token), `POST /logout`; bearer access tokens.

Sync: `GET /sync/bootstrap` (snapshot + cursor), `GET /events?after=<cursor>`, WebSocket `/ws` (`hello{cursor}` then
`events`; also typing, presence, ack, ping). Each user has an ordered event log; the cursor is its id.

Chat: `/conversations` (+`/dm`, `/self`, `/{id}`, `/{id}/messages`, `/{id}/read`, `/{id}/unread`), `/messages/{id}` (PATCH edit,
DELETE tombstone), `/messages/{id}/reactions/{emoji}`, `/files` (upload), `/files/{id}` (Range, signed URLs via `/files/{id}/url`),
`/search`, `/users`, `/me`, `/me/password`, `/sessions`.
Sends are idempotent on `(sender, client_id)`: a retry returns the stored message (200 instead of 201).
Edit rule (`proto::rules::edit_allowed`): channels and notes-to-self always; DMs and group DMs only while nobody else has posted later.

Admin (`/admin/...`, owner or admin only): overview, users (+reset-password, login-link, revoke-sessions, clear-history, backup,
delete-range), backups, sessions, conversations (+members), files, storage (+test, migrate, verify), jobs.

## 1.1.2 additions

* Self-service sign-out is gated by the admin's Privacy options (`privacy_controls_enabled` in `GET /me`). For a member without it, `POST /logout`, `DELETE /sessions/{own id}`,
  `POST /sessions/revoke-others` and the new `POST /sessions/revoke-all` answer `403 forbidden` ("sign-out is not allowed for this account") and change nothing.
  Admins and owners are exempt, admins can still end any session (`DELETE /sessions/{id}`, `POST /admin/users/{id}/revoke-sessions`), and token refresh and the forced
  revocations (`account_disabled`, `password_reset`, `history_cleared`) are unaffected. `GET /sessions` stays readable.
* `POST /sessions/revoke-all` signs the caller out of every device including the calling one and returns `{"revoked": n}`.
* Older apps (1.1.1 and before) still show their sign-out buttons for such users; the server refuses them.

## 1.1.1 additions

* `PUT /admin/users/{id}/role` with `{"role":"member"|"admin"|"owner"}` returns the updated `AdminUser`. Owner only (`forbidden` for admins and members);
  `forbidden` when changing your own role; `validation` for deleted accounts, for a bot given any role but member, and when it would leave no owner.
  Permissions are read on every request, so the change is effective for open sessions immediately. The user's apps receive `me_update` (their own `Me`
  with the new role) and everyone receives `user_upsert`; no new event type, `min_client_version` stays 1.0.0. `PATCH /admin/users/{id}` with `role` follows the same rules.

## 1.1 additions

* **Login link landing page** (no auth, not under `/api/v1`): `GET /l/<token>` shows the open-in-app page without consuming the token (200 valid, 410 used or
  expired, 404 unknown). `Cache-Control: no-store`, `Referrer-Policy: no-referrer`, a CSP that allows no outside content. `GET /downloads` lists installers and
  `GET /downloads/<file>` serves one from `SUPERCHAT_DOWNLOADS_DIR`.
* `LoginLink.link` is now `<public-url>/l/<token>`. `POST /login/link` accepts that URL, the old `superchat://login?server=&token=` and `atomic-chat://` forms, or the bare token.
* `GET/PUT /admin/public-url` (`{"url": "https://chat.example.com" | null}`; PUT is owner-only; the response has `effective` and `source` = `env` | `setting` | `request`).
* AI providers (owner changes, admins read): `GET /admin/ai`, `POST /admin/ai/providers`, `PATCH`/`DELETE /admin/ai/providers/{id}`, `POST /admin/ai/providers/{id}/test`,
  `PUT /admin/ai/defaults`. Keys are never returned. See [BOTS.md](BOTS.md).
* `DELETE /admin/users/{id}` now cascades correctly for users who signed in (sessions, devices, memberships, visibility, drafts, preferences, push registrations,
  bot tokens, login links) and keeps other people's copies of chats. Server errors are logged as `METHOD route -> status` plus the cause, never with tokens.
* App Lock is client-only: no server endpoint, nothing synced.

## Phase 2 additions

All paths are under `/api/v1`; types are in `crates/proto` and `schema/api.schema.json`.

* Search: `GET /search?q=&from=&in=&has=link|file|reaction&before=<ms>&after=<ms>&type=image|video|audio|document` returns
  `{hits, people, files, channels}`; only conversations the caller belongs to are ever searched. `GET /files?conv=&kind=&q=&before=` lists files.
* Threads: `GET /conversations/{id}/threads/{root}` (root + replies), `GET /threads` (threads the user follows or posted in, with unread),
  `PUT|DELETE /threads/{root}/follow`, `POST /threads/{root}/read`. A reply is a message with `thread_root` set; it is not part of the top-level timeline.
* Pins and bookmarks: `GET /conversations/{id}/pins`, `PUT|DELETE /conversations/{id}/pins/{message_id}` (owners and admins only),
  `GET|POST /conversations/{id}/bookmarks`, `PATCH|DELETE /bookmarks/{id}`.
* Groups: `GET /groups`, admin `POST /admin/groups`, `PATCH|DELETE /admin/groups/{id}`. `@handle` in a message mentions every member.
* Preferences: `GET /me/prefs`, `PUT /conversations/{id}/prefs` (`favourite`, `section_id` (empty string clears), `mute_until_ms`, `notify` =
  `all|mentions|nothing`), `GET|POST /sections`, `PATCH|DELETE /sections/{id}`. All synced to the user's other devices through events.
* Drafts: `GET /drafts`, `PUT /conversations/{id}/draft` (optional `thread_root`).
* `GET /messages/{id}`, `POST /messages/{id}/forward {conv_ids, comment}` (forwarded copies carry `forwarded` attribution).
  Permalinks are `superchat://m?server=&c=&m=`.
* Push: `GET|PUT /me/push`, `DELETE /me/push/{id}` (device registration: `ntfy`, `fcm` or `apns`); admin `GET|PUT /admin/push` (secrets are masked on read).
  Pushes are content-free wakes: ntfy body `wake`, FCM/APNs data `type: wake`. They go only to offline, non-DND, non-muted members who want the notification.
* Sessions: `DELETE /sessions/{id}`, `POST /sessions/revoke-others`.
* Admin viewer: `GET /admin/viewer/conversations?user_id=&kind=&q=`, `GET /admin/viewer/conversations/{id}/messages?as_user=&before=&after=&around_id=&around_ts=&from_ms=&to_ms=&thread_root=&top_level=&limit=`,
  `GET /admin/viewer/search`, `POST /admin/messages/delete {message_ids | conv_id + from_ms/to_ms/before_ms, scope: "everyone"|"user", user_id, ...}` (see `AdminDeleteMessagesRequest`)
  (always writes a backup first and refuses with `backup_failed` otherwise).
* Backups and restore: `POST /admin/backups/full`, `GET|PUT /admin/backups/settings`, `POST /admin/backups/test-target`,
  `GET /admin/backups/{id}/download`, `POST /admin/restore {backup_id, user_ids | all, mode: "merge"|"replace"}` (backup kinds `user`, `full`, `messages`),
  and on a server with no users `POST /restore` (from the storage target) and `POST /restore/upload` (an uploaded `.age` file) with the passphrase.
  After a restore, event ids jump by 1,000,000 and clients receive `resync`, so every device re-bootstraps.
