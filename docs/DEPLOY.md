# Deploying SuperChat

The server is one container (Rust binary plus the admin panel). Everything that must survive lives in one volume, `/data`
(SQLite database, settings, local files if you use local storage, backups). Attachments normally live in your chosen
storage (S3, R2, B2, Google Drive), not on the server's disk.

## Quick start (install)

The server image is published on GitHub's container registry for every release: `ghcr.io/uri-travoski/superchat-server`. The package is
**public**, so no login or token is needed to pull it, and nothing is built on your machine. All you need is Docker with the compose plugin
and two files:

```
mkdir superchat && cd superchat
curl -fLO https://raw.githubusercontent.com/uri-travoski/superchat/main/deploy/docker-compose.yml
curl -fL  -o .env https://raw.githubusercontent.com/uri-travoski/superchat/main/deploy/.env.example
# for the caddy profile (HTTPS) also: curl -fLO https://raw.githubusercontent.com/uri-travoski/superchat/main/deploy/Caddyfile
nano .env                   # set ATOMIC_PUBLIC_URL at least
docker compose pull && docker compose up -d     # http://localhost:8080/admin  -> setup wizard
```

(From a git checkout the same files are in `deploy/`.)

## Updating the server

```
docker compose pull && docker compose up -d
```

Run it in the folder with `docker-compose.yml`. The data volume is kept; the container restarts on the new image in a few seconds and the
database is migrated on start. The admin panel's Overview has an **Updates** card that says whether a newer release exists, shows its release
notes, these two commands and links to that release's app downloads. Take a backup first if you want a way back.

**Pin a version** (so `pull` never moves you past a release you have not read about): set `SUPERCHAT_IMAGE` in `.env`,

```
SUPERCHAT_IMAGE=ghcr.io/uri-travoski/superchat-server:1.3.1
```

(`ATOMIC_IMAGE` is accepted as the older spelling; `SUPERCHAT_IMAGE` wins when both are set). Tags are the release numbers without the `v` (`1.3.1`)
and `latest`, the newest release. Empty or unset means `latest`. To go back one version, pin the older tag and run the same two commands (a newer
database is not downgraded, so restore a backup if a release changed the schema; the release notes say when).

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
| `ATOMIC_PUBLIC_URL` | none | Address clients use; goes into one-time login links (`<url>/l/<token>`). When unset the owner can save one in the admin panel, else it is taken from the request's Host header. |
| `ATOMIC_DOWNLOADS_DIR` | `<data>/downloads` | Folder with the installers (`.apk`, `.AppImage`, `.deb`, `.exe`, ...) offered by the login link page and `/downloads`. |
| `ATOMIC_RELEASE_URL` | the project's GitHub releases | Where the login link page sends people when the downloads folder is empty. |
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

ntfy on the same host: `docker compose --profile ntfy up -d` starts ntfy bound to the host's loopback on `NTFY_PORT` (default 2586; set `NTFY_BIND_ADDR=0.0.0.0` only on a trusted network, or put it behind your HTTPS proxy). Set `ATOMIC_NTFY_URL` to the address phones use (for example `https://ntfy.example.org`), then in the app choose You > Preferences > Notifications > Push method > ntfy. The server only sends a wake-up to people who have no open connection, and the wake never carries message text. `tests/e2e/ntfy-emulator.sh` runs this against a real ntfy container and the Android emulator. FCM needs a Firebase `google-services.json` at build time plus a service account in the admin panel and has not been run on a real device.

## Building the image (developers only)

Normal installs never build: they pull the published image (see above). To run your own build of a checkout, uncomment the `build:` block under the
`superchat` service in `deploy/docker-compose.yml`, then `docker compose build && docker compose up -d`, or just

```
docker build -f deploy/Dockerfile -t superchat:1.3.1 .
docker tag superchat:1.3.1 ghcr.io/uri-travoski/superchat-server:1.3.1     # what release.yml publishes, if you want to test the compose file offline
```
The image is multi-stage (node for the admin panel, rust for the server, debian-slim runtime with ca-certificates and tini,
non-root user, HEALTHCHECK).

## Update checks (apps and server)

Every app checks GitHub for a newer release: the desktop apps and the Android app under Settings > About (button **Check for updates**, plus an
automatic check when the app starts and then every 24 hours, which can be switched off there), and the server in the admin panel's Updates card.
Only the release list is read (`https://api.github.com/repos/uri-travoski/superchat/releases/latest`), and the only thing sent is `User-Agent: SuperChat/<version>`
(the server sends `SuperChat-Server/<version>`): no account, device or server information. Drafts and pre-releases are ignored.

* **Which file an app downloads** (names are those `release.yml` publishes, each with a `.sha256` file next to it): Android `superchat-<v>-android.apk`;
  Linux AppImage (when started from `$APPIMAGE`) `SuperChat-<v>-x86_64.AppImage`, otherwise `superchat_<v>-1_amd64.deb`; Windows `superchat-<v>-x64-setup.exe`
  when installed with the installer, `superchat-<v>-x64-portable.exe` for the portable file. A Flatpak install is told to run `flatpak update`.
  The download is checked against its `.sha256` file and discarded on a mismatch; a release without that file is refused.
* **Pointing the check somewhere else** (tests, a mirror): environment `SUPERCHAT_UPDATE_URL` (or `ATOMIC_UPDATE_URL`) for the apps and the server, or the hidden
  `update_url` entry in the app's `prefs.json`. `tests/update/fake-release-server.py` is a fake GitHub for this.
* **Admin panel**: the *server* asks GitHub (`GET /api/v1/admin/updates`, admins only, cached for an hour, "Check again" allowed once a minute), not the browser, so the
  admin pages keep their strict Content-Security-Policy and need no `connect-src` for api.github.com. If the server has no outbound internet the card says so and still
  shows the commands.

## Android signing

An installed Android app can only be updated by an APK signed with the **same key**. Every release must therefore be signed with one permanent key, which the
owner keeps (and backs up: a lost key means everybody has to uninstall and reinstall). `release.yml` **fails** the Android job, with an error that points here, when
any signing secret is missing; it never falls back to a throw-away key. Add four repository secrets (Settings > Secrets and variables > Actions > New repository secret):

| Secret | Value |
|---|---|
| `ANDROID_KEYSTORE_BASE64` | the keystore file, base64 encoded on one line: `base64 -w0 release.jks` |
| `ANDROID_KEYSTORE_PASSWORD` | the keystore password |
| `ANDROID_KEY_ALIAS` | the key alias inside it |
| `ANDROID_KEY_PASSWORD` | the key password (same as the keystore password for a PKCS12 keystore) |

Create a key once with `keytool -genkeypair -v -keystore release.jks -alias superchat -keyalg RSA -keysize 4096 -validity 10000` (do not commit it). The workflow decodes the
secrets into `ATOMIC_KEYSTORE_FILE`, `ATOMIC_KEYSTORE_PASSWORD`, `ATOMIC_KEY_ALIAS` and `ATOMIC_KEY_PASSWORD`, which `apps/android/app/build.gradle.kts` reads. A local
`./gradlew assembleRelease` without them is signed with the debug key: fine for testing, but it cannot update an install signed with the release key (Android says the
signatures do not match; the app explains that and asks the user to uninstall first).

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
* Login links are now web addresses, `https://<your-server>/l/<token>`; links made by 1.0 (`atomic-chat://`) and `superchat://` links are still accepted by the apps.
  Behind a reverse proxy or tunnel make sure `/l/` and `/downloads` reach the server (the shipped Caddyfile and tunnel send everything). Android may ask once whether
  to open such a link with SuperChat or the browser (app links are not verified); the landing page opens the app by itself either way.
* The existing server name (stored in the database) is not changed; edit it in the admin panel if you want it to read SuperChat.
* Desktop apps move `~/.local/share/atomic-chat` (Windows: `%APPDATA%\AtomicChat`) to the new SuperChat folder on first start and
  still read sign-in secrets saved under the old name. The Android app has a new application id (`app.superchat`), so it installs
  next to the old app and you sign in again with a new login link.
