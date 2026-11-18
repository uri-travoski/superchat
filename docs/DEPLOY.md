# Deploying SuperChat

The server is one container (Rust binary plus the admin panel). Everything that must survive lives in one volume, `/data`
(SQLite database, settings, local files if you use local storage, backups). Attachments normally live in your chosen
storage (S3, R2, B2, Google Drive), not on the server's disk.

## Quick start

```
cd deploy
cp .env.example .env        # edit ATOMIC_PUBLIC_URL at least
docker compose up -d        # http://localhost:8080/admin  -> setup wizard
```

The first visit to `/admin` shows the setup wizard: owner account plus attachment storage (with a Test button).
Alternatively set `ATOMIC_BOOTSTRAP_ADMIN=user:password` in `.env` to create the owner on first start.

## Reaching it from the internet

| Situation | Command | `.env` |
|---|---|---|
| You have a domain | `docker compose --profile caddy up -d` | `ATOMIC_HOST=chat.example.com`, `ATOMIC_PUBLIC_URL=https://chat.example.com`. Ports 80 and 443 must be open. Caddy obtains a public certificate. |
| Bare public IP, no domain | `docker compose --profile caddy up -d` | `ATOMIC_HOST=203.0.113.7`, `CADDY_TLS=tls internal`, `ATOMIC_PUBLIC_URL=https://203.0.113.7`. Caddy uses its own CA; clients pin the certificate the first time they connect (trust on first use) and warn loudly if it ever changes. |
| No open ports | `docker compose --profile tunnel up -d` | `CLOUDFLARE_TUNNEL_TOKEN=...`; in Cloudflare point the hostname at `http://atomic:8080`. |
| LAN testing only | plain `docker compose up -d` | `http://<lan-ip>:8080`. Clients refuse plain http unless "allow insecure http" is enabled; use only on a trusted network. |

## Profiles

* `caddy`: HTTPS reverse proxy (WebSockets pass through).
* `tunnel`: Cloudflare Tunnel (`cloudflared`).
* `ntfy`: self-hosted ntfy for push (used from Phase 3).
* `minio`: a local S3 for trying the storage settings. Development only. In the admin panel use endpoint `http://minio:9000`,
  bucket `atomic`, tick path-style and allow-http. (MinIO no longer publishes `minio/minio` on Docker Hub; the compose file
  uses `bitnamilegacy/minio` by default, override with `MINIO_IMAGE`.)

## Configuration (environment)

| Variable | Default | Meaning |
|---|---|---|
| `ATOMIC_PUBLIC_URL` | none | Address clients use; goes into one-time login links. |
| `ATOMIC_NAME` | SuperChat | Server name shown in apps. |
| `ATOMIC_BOOTSTRAP_ADMIN` | none | `user:password` of the owner to create when no users exist. |
| `ATOMIC_MAX_UPLOAD_MB` | 100 | Largest single attachment. |
| `ATOMIC_ACCESS_TTL_SECS` | 900 | Access token lifetime. |
| `ATOMIC_LOCKOUT_THRESHOLD` / `_SECS` | 5 / 300 | Failed logins before a temporary lockout (per username and per IP). |
| `ATOMIC_EVENT_RETENTION_DAYS` | 90 | Event log pruning. Clients that were offline longer re-bootstrap. |
| `ATOMIC_NTFY_URL` | none | ntfy base URL. |
| `ATOMIC_BACKUP_PASSPHRASE` | file `<data>/backup.key` | Passphrase for encrypted backups (age, scrypt). If unset one is generated into `backup.key`. Keep a copy elsewhere; it is never inside a backup. |
| `ATOMIC_BACKUP_WORK_FACTOR` | age default | Lower only in tests. |
| `ATOMIC_BIND`, `ATOMIC_DATA_DIR`, `ATOMIC_ADMIN_DIR` | `0.0.0.0:8080`, `/data`, `/srv/admin` | Inside the container; normally untouched. |

## Backups of the server itself

Back up the `atomic-data` volume (stop the container or copy the SQLite file with `sqlite3 .backup`). Per-user backups made by the
admin panel are stored in your attachment storage under `backups/`, not on the server disk, and survive a server rebuild.

### Restoring onto a new server

1. Start a fresh container with an empty data volume and open `/admin/`: the setup page offers "Restore from backup".
2. Enter the same attachment storage settings (the backups and attachments live there) and the backup passphrase, or upload the `.age` file.
3. Users, chats, settings and sessions come back; attachments are reconnected in place. Every app re-syncs automatically (event ids jump and clients re-bootstrap).

Push (optional): ntfy needs only a URL. FCM needs a Google service account JSON and APNs a `.p8` key, both entered in the admin panel; wakes never contain message text.

## Building the image

```
docker build -f deploy/Dockerfile -t superchat-server .
```
The image is multi-stage (node for the admin panel, rust for the server, debian-slim runtime with ca-certificates and tini,
non-root user, HEALTHCHECK). Updating: pull or rebuild, then `docker compose up -d`.

## Names and upgrading from Atomic Chat 1.0

The project was renamed from Atomic Chat to SuperChat in v1.1. What that changes for a server:

* Image `ghcr.io/uri-travoski/superchat-server`, binary `superchat-server`, compose project `superchat`, service `superchat`
  (Caddy and the tunnel point at `http://superchat:8080`).
* **Environment variables**: every setting can be given as `SUPERCHAT_<NAME>` or as the legacy `ATOMIC_<NAME>`; when both are set
  the `SUPERCHAT_` one wins inside the server and the apps. The compose file and `.env.example` keep using the `ATOMIC_*` spellings
  (docker-compose substitutes only those), so an existing `.env` keeps working unchanged.
* **Data volume**: the 1.0 compose project created the volume `atomic-chat_atomic-data`. To keep using it, add
  `SUPERCHAT_DATA_VOLUME=atomic-chat_atomic-data` to `.env` before `docker-compose up -d`. New installs get `superchat-data`.
  The database file inside is still called `atomic.db` (internal, nothing to do).
* Login links now start with `superchat://`; links made by 1.0 (`atomic-chat://`) are still accepted by the apps.
* The existing server name (stored in the database) is not changed; edit it in the admin panel if you want it to read SuperChat.
* Desktop apps move `~/.local/share/atomic-chat` (Windows: `%APPDATA%\AtomicChat`) to the new SuperChat folder on first start and
  still read sign-in secrets saved under the old name. The Android app has a new application id (`app.superchat`), so it installs
  next to the old app and you sign in again with a new login link.
