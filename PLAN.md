# Atomic Chat — build plan for v1.0

Source specs (read them fully): `Server` (owner's server notes) and `Client Apps.md` (the PRD).
Where they conflict, the decisions below win.

## Owner decisions (2026-11-17)
1. **No end-to-end encryption.** TLS in transit; messages readable on the server by admins (needed for admin chat viewer, search). The database file is encrypted at rest only via the backup encryption (backups are encrypted with a passphrase; age/AES-256-GCM).
2. **v1.0 scope = PRD phases 0–3** plus everything in the `Server` doc. **Calls (SV-18, CL-01) are deferred to v1.1**; keep a stub signalling message type in the protocol only.
3. **Attachment storage backends:** local disk (for testing/dev), S3, Cloudflare R2, Backblaze B2 (all three via the S3 API), Google Drive (OAuth refresh token, drive.file scope). Chosen at first-run setup in the admin panel; changeable later with "move all files to storage X" + a verification pass that every attachment is still resolvable. Only config, accounts and the text DB live on the server disk when a cloud backend is selected.
4. **GitHub repo:** `uri-travoski/atomic-chat`, **public**. NEVER commit `credentials.md` (gitignore it). Release tag `v1.0`, release title "Atomic Chat v1.0", crate versions 1.0.0. Pushing/release is done by the orchestrator (Opus), not the builder agents — builders commit locally only.

## Reconciled server requirements (from the `Server` doc, on top of PRD SV-01..SV-24)
- Works behind Cloudflare Tunnel **or** directly; domain or bare IP; nothing hard-coded. Compose ships an optional `cloudflared` service (profile `tunnel`) so the VPS needs **no open inbound port** when using the tunnel. Without a tunnel: Caddy profile for automatic TLS on a domain, or self-signed cert for IP (client does TOFU fingerprint pinning, X-01).
- Push: ntfy (service in same compose network, profile `ntfy`), FCM (HTTP v1, service-account JSON), APNs (token-based .p8; delivery code present). Pushes are content-free wake-ups.
- Admin creates user (username, display name, password) **and a one-time login link/code** the user pastes on the app login page (link encodes server URL + one-time token; app exchanges it for a session and the user then sets/keeps password). Normal username+password login also works.
- Disable user ⇒ all sessions revoked immediately, websocket closed with `account_disabled`, app wipes local data and returns to login (X-05).
- "Clear history" for a user: automatic per-user backup first (refuse if backup fails), wipe that user's copy of chats server-side, move their attachments into the backup folder in storage (not erased), force re-login.
- **Each side of a chat stored separately**: implement as per-user message visibility (a `message_hidden_for(user_id, message_id)` / per-user deletion table) so clearing/deleting one user leaves the other person's copy. Attachment objects are reference-counted; only moved to backup when no remaining visible reference.
- Date-range delete (admin): messages between two dates for a user/conversation; option "also delete the other person's copy" (off by default).
- Per-user backup button (account, contacts/DM list, conversations, messages, attachment refs). Selective restore: one / several / all users, merge by default, "replace" option. Attachments moved back from backup folder on restore.
- Full encrypted backup: scheduled daily (retention count) + one-click; target local / GDrive / S3 / R2 / B2. Restore onto a fresh server reconnects to existing attachment storage.
- Bots: Telegram-style endpoints `/bot<token>/sendMessage`, `/sendDocument`, `/setWebhook`, `/deleteWebhook`, `/getUpdates`, `/getMe` (Telegram-compatible JSON shape `{ok, result}`), plus the PRD bot API (forms SV-17, App Home).
- **Admin web chat viewer** (Element-X-web look, full window): dropdown of users/channels/groups, shows conversations and history like WhatsApp Web, inline photos/videos/files, search, date filters, delete single/bulk, delete before date; admin can do everything a normal user can (send as a selected user is NOT required — admin acts as themself; they can post in conversations they're added to).
- Hosting: `docker compose` (note: on this dev box use `docker-compose` v1 binary; `docker compose` plugin is missing).

## Architecture
Cargo workspace (Rust, edition 2021, rustc 1.99 via `~/.cargo/bin`):
```
atomic-chat/
  crates/proto/        # shared API types (serde + schemars); `cargo run -p proto --bin gen-schema` writes schema/api.schema.json; API_VERSION, error codes
  crates/server/       # axum + tokio + rusqlite (bundled, WAL, FTS5) + argon2 + object_store-like Storage trait
  crates/client-core/  # headless client: reqwest(rustls) + tokio-tungstenite + rusqlite cache + sync engine + outbox
  crates/cli/          # `atomic` headless CLI on client-core (used by e2e tests and for scripting)
  apps/desktop/        # Slint UI (Linux + Windows) on client-core
  apps/android/        # Gradle Kotlin shell + Slint android-activity (cargo-ndk) on client-core; FCM + ntfy (UnifiedPush) receivers, foreground service
  admin-web/           # admin panel + chat viewer: plain TypeScript + Vite (or Svelte) built to static files served by the server at /admin
  deploy/              # docker-compose.yml, Dockerfile, Caddyfile, cloudflared example, .env.example
  tests/e2e/           # scripts booting compose with throwaway DB + MinIO (fake S3) + fake GDrive + ntfy, driven by client-core
  docs/                # API.md, BOTS.md, ADMIN.md, DEPLOY.md, FEATURES.md (feature-ID table)
```
- Server: one binary/image, SQLite WAL default. REST JSON under `/api/v1`, WebSocket `/api/v1/ws`. Per-user ordered event log with cursor (5.2). Idempotent sends by client id. Error codes list from 5.2.
- Client UI stack: **Slint** first (PRD 4.2). Run the Phase-0 spike on the Android emulator; if Slint fails the gate (IME/emoji input, font scale, etc.) switch to **Tauri 2 + Svelte** and record why in docs/DECISIONS.md.
- Ports for local dev/testing: server on **18300** (avoid 8080/8085/5222/3000/5432/9000/8090/80 which are in use by other projects). Do not stop or touch other running containers.

## Phases (each phase ends green: `cargo test --workspace`, e2e, clippy clean)
- **P0** workspace, proto crate, server skeleton (login, refresh, ws), client-core login+DM, CLI, Slint spike on desktop + Android emulator. Record spike outcome.
- **P1 core chat** — PRD phase 1 list + Server-doc admin features (users, one-time link, disable/kick, clear history w/ backup, storage backends + move/verify).
- **P2 organisation & search** — PRD phase 2 list, backups/restore (full + per-user + selective), date-range delete, admin web chat viewer.
- **P3 automation** — bots (Telegram-style + forms + App Home), webhooks in/out, scheduled messages, AI search/summary (off by default; provider-agnostic, Anthropic default model `claude-sonnet-5-5`), push (ntfy/FCM/APNs).
- **P4 packaging** — Docker image; Linux AppImage + .deb; Windows portable .exe + installer (build in GitHub Actions on `windows-latest`); Android APK (arm64 + x86_64). GitHub Actions `release.yml` on `v*` tags builds all and attaches to the release; Linux artifacts and APK may also be built locally.

## Testing rules
- Unit tests per PRD section 8; server e2e (SV-24) via client-core against docker (MinIO for S3, a fake GDrive HTTP server, ntfy container).
- Android: only the local emulator. Use AVD `timeline-phone` (start with `ANDROID_AVD_HOME=$HOME/.android/avd /opt/android-sdk/emulator/emulator -avd timeline-phone -port 5556 -no-window -no-audio -gpu swiftshader_indirect`), always `adb -s emulator-5556`, never bare `adb`/`adb -e`. **Never install on any physical device.** Emulator reaches host at 10.0.2.2.
- Desktop GUI tests: private Xvfb (`:99`) + own dbus-daemon; never the user's `:0` display. Screenshot tests at Regular and Large.
- Admin web: Playwright (playwright-core + `~/.cache/ms-playwright/chromium-*`).
- Do not bulk-hit any external service. Fakes for FCM/APNs/GDrive in tests.
- Linking quirk: `~/.local/bin/cc` is a broken distrobox shim → build with `CC=/usr/bin/cc CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=/usr/bin/cc` and `PATH=$HOME/.cargo/bin:$PATH`.
- Docker: run compose/bind mounts from `/home/owner/distrobox-homes/devbox/Projects/Atomic Chat/...` paths, never `/run/host/...`. Build images with `docker build`, then `docker-compose up --no-build`.
- Test admin login for local instances: username `admin`, password `test-admin-pw`.

## Status
Tracked in `STATUS.md` (per phase: done / tested / known gaps).
