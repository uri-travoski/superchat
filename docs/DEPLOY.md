# Deploying SuperChat

The server is one container (Rust binary plus the admin panel). Everything that must survive lives in one volume, `/data`
(SQLite database, settings, local files if you use local storage, backups). Attachments normally live in your chosen
storage (S3, R2, B2, Google Drive), not on the server's disk.

## Quick start (install)

The server is **one container, one compose file**: `docker-compose.yml` has a single service, `superchat`, which pulls the published image
`ghcr.io/uri-travoski/superchat-server` (a **public** package: no login, nothing is built on your machine). There is no reverse proxy,
tunnel, ntfy or object store in it; those run separately (see below). You need Docker with compose and two files from the latest GitHub Release:

```
mkdir superchat && cd superchat
curl -fLO https://github.com/uri-travoski/superchat/releases/latest/download/docker-compose.yml
curl -fL -o .env https://github.com/uri-travoski/superchat/releases/latest/download/superchat.env.example
nano .env                     # set SUPERCHAT_PUBLIC_URL at least; every variable is explained in the file
docker compose up -d          # http://<this machine>:8080/admin  -> setup wizard
```

(From a git checkout the same two files are `deploy/docker-compose.yml` and `deploy/.env.example`. The compose file works with `docker compose` and `docker-compose`.)

Nothing in `.env` is required: with no `.env` at all the server starts on port 8080 with the defaults below. The first visit to `/admin` shows the setup
wizard: owner account plus attachment storage (with a Test button). Alternatively set `SUPERCHAT_BOOTSTRAP_ADMIN=user:password` in `.env` to create the owner on first start.

## Updating the server

Every release publishes a `docker-compose.yml` whose default image is **that release's version** (pinned on purpose, so nothing moves under you). To update:

```
curl -fLO https://github.com/uri-travoski/superchat/releases/latest/download/docker-compose.yml     # the new pinned version
docker compose pull && docker compose up -d
```

or, without touching the file, set the image in `.env` and run the same two compose commands:

```
SUPERCHAT_IMAGE=ghcr.io/uri-travoski/superchat-server:1.4.1     # a specific release (tags are the release numbers without the v)
SUPERCHAT_IMAGE=ghcr.io/uri-travoski/superchat-server:latest    # always the newest release: docker compose pull is then the whole update
```

Run them in the folder with `docker-compose.yml`. The data volume is kept; the container restarts on the new image in a few seconds and the
database is migrated on start. The admin panel's Overview has an **Updates** card that says whether a newer release exists, shows its release notes, the
commands and links to that release's app downloads. Take a backup first if you want a way back. To go back one version, pin the older tag (a newer database
is not downgraded, so restore a backup if a release changed the schema; the release notes say when).

## Reverse proxy, tunnel and ntfy run separately

SuperChat only serves plain HTTP on container port 8080 (published on the host as `SUPERCHAT_PORT`, default 8080). HTTPS, a Cloudflare Tunnel or any other front door is
yours to run, in its own container or on the host. What the server needs from it:

* **Point it at the server**: `http://<host>:8080` (the host's address, or `http://127.0.0.1:8080` if cloudflared runs on the host), or `http://superchat:8080` when the
  proxy shares a Docker network with the server (see the `networks:` example at the bottom of `docker-compose.yml`: create the network once, `docker network create proxy`,
  uncomment the lines, and the published port can be dropped). With a proxy on the same machine set `SUPERCHAT_BIND_ADDR=127.0.0.1` so the plain-HTTP port is not exposed.
* **Pass everything through**, in particular the **WebSocket** at `/api/v1/ws` (live messages). Cloudflare Tunnel does this by default; for nginx, Caddy or Traefik make sure the
  `Upgrade` header is forwarded (Caddy's `reverse_proxy` and Traefik do it automatically; nginx needs `proxy_set_header Upgrade $http_upgrade; proxy_set_header Connection "upgrade";`).
  `/l/` (login links) and `/downloads` must reach the server too. Allow request bodies of at least `SUPERCHAT_MAX_UPLOAD_MB`.
* **Set `SUPERCHAT_PUBLIC_URL`** to the public https address (`https://chat.example.com`); it goes into one-time login links.
* **ntfy** (optional push): run your own ntfy server separately and set `SUPERCHAT_NTFY_URL` to its base URL, for example `https://ntfy.example.com`,
  `http://host.docker.internal:2586` (ntfy published on the same host: the compose file maps that name to the host) or `http://ntfy:80` (shared Docker network). In the app choose
  You > Preferences > Notifications > Push method > ntfy. The server only sends a wake-up to people who have no open connection, and the wake never carries message text.
* A bare IP with a self-signed certificate works too: apps pin the certificate the first time they connect (trust on first use) and warn if it ever changes.

| Situation | What to do |
|---|---|
| A domain, ports 80/443 open | your own reverse proxy (Caddy, nginx, Traefik) in front of `:8080`; `SUPERCHAT_PUBLIC_URL=https://chat.example.com` |
| No open ports | your own cloudflared; hostname -> `http://<host>:8080` or `http://superchat:8080`; `SUPERCHAT_PUBLIC_URL=https://chat.example.com` |
| LAN testing only | nothing extra: `http://<lan-ip>:8080`. Clients refuse plain http unless "allow insecure http" is enabled; use only on a trusted network. |

## Attachment storage (S3, R2, B2)

No object store is shipped: the server talks to yours. Choose it in the admin panel on first setup (or later under **Storage**), fill the form and press **Test**:

* **Cloudflare R2**: create a bucket and an R2 API token with object read and write. Endpoint `https://<account-id>.r2.cloudflarestorage.com`, region `auto`, the token's access key ID and secret.
* **Backblaze B2**: create a bucket and an application key with access to it. Endpoint `https://s3.<region>.backblazeb2.com` (the region is in the bucket's endpoint on the B2 page, for example `us-west-004`), region the same string.
* **Amazon S3**: leave the endpoint empty, set the region, bucket and an IAM user's keys. Any other S3-compatible service works with its endpoint (tick path-style if it needs it).
* **Google Drive** (OAuth client, refresh token with the `drive.file` scope) and **local disk** (inside the `/data` volume; for testing and small installs) are the other choices.

## Configuration (environment)

The compose file passes the variables below from `.env`. The server reads each as `SUPERCHAT_<NAME>`; the legacy `ATOMIC_<NAME>` spelling is accepted by the server too
(`SUPERCHAT_` wins). Compose itself only substitutes the `SUPERCHAT_*` names, so an old `.env` needs `sed -i 's/^ATOMIC_/SUPERCHAT_/' .env`.

| Variable | Default | Meaning |
|---|---|---|
| `SUPERCHAT_PUBLIC_URL` | none | Address clients use; goes into one-time login links (`<url>/l/<token>`). When unset the owner can save one in the admin panel, else it is taken from the request's Host header. |
| `SUPERCHAT_NAME` | SuperChat | Server name shown in apps. |
| `SUPERCHAT_BOOTSTRAP_ADMIN` | none | `user:password` of the owner to create when no users exist. |
| `SUPERCHAT_MAX_UPLOAD_MB` | 100 | Largest single attachment. |
| `SUPERCHAT_NTFY_URL` | none | Base URL of your own ntfy server. |
| `SUPERCHAT_BACKUP_PASSPHRASE` | file `<data>/backup.key` | Passphrase for encrypted backups (age, scrypt). If unset one is generated into `backup.key`. Keep a copy elsewhere; it is never inside a backup. |
| `SUPERCHAT_LOG_LEVEL` | `info,tower_http=warn` | Compose passes it as `RUST_LOG`. |
| `SUPERCHAT_IMAGE` | `ghcr.io/uri-travoski/superchat-server:<release>` | Image to run (compose only). |
| `SUPERCHAT_PORT`, `SUPERCHAT_BIND_ADDR` | `8080`, `0.0.0.0` | Host port and address that publish the server (compose only). |
| `SUPERCHAT_DATA_VOLUME` | `superchat-data` | Name of the Docker volume mounted at `/data` (compose only). |

Further settings the server understands (not in `.env.example`; add them under `environment:` in the compose file if you need them):

| Variable | Default | Meaning |
|---|---|---|
| `SUPERCHAT_DOWNLOADS_DIR` | `<data>/downloads` | Folder with the installers (`.apk`, `.AppImage`, `.deb`, `.exe`, ...) offered by the login link page and `/downloads`. |
| `SUPERCHAT_RELEASE_URL` | the project's GitHub releases | Where the login link page sends people when the downloads folder is empty. |
| `SUPERCHAT_ACCESS_TTL_SECS` | 900 | Access token lifetime. |
| `SUPERCHAT_LOCKOUT_THRESHOLD` / `_SECS` | 5 / 300 | Failed logins before a temporary lockout (per username and per IP). |
| `SUPERCHAT_EVENT_RETENTION_DAYS` | 90 | Event log pruning. Clients that were offline longer re-bootstrap. |
| `SUPERCHAT_BACKUP_WORK_FACTOR` | age default | Lower only in tests. |
| `SUPERCHAT_BIND`, `SUPERCHAT_DATA_DIR`, `SUPERCHAT_ADMIN_DIR` | `0.0.0.0:8080`, `/data`, `/srv/admin` | Set by the image; normally untouched. |

## Backups of the server itself

Back up the data volume (`superchat-data`, or whatever `SUPERCHAT_DATA_VOLUME` names) (stop the container or copy the SQLite file with `sqlite3 .backup`). Per-user backups made by the
admin panel are stored in your attachment storage under `backups/`, not on the server disk, and survive a server rebuild.

### Restoring onto a new server

1. Start a fresh container with an empty data volume and open `/admin/`: the setup page offers "Restore from backup".
2. Enter the same attachment storage settings (the backups and attachments live there) and the backup passphrase, or upload the `.age` file.
3. Users, chats, settings and sessions come back; attachments are reconnected in place. Every app re-syncs automatically (event ids jump and clients re-bootstrap).

Push (optional): ntfy needs only a URL (see "Reverse proxy, tunnel and ntfy run separately"). FCM needs a Google service account JSON and APNs a `.p8` key, both entered in the admin panel; wakes never contain message text. `tests/e2e/ntfy-emulator.sh` runs the ntfy path against a real ntfy container and the Android emulator. FCM needs a Firebase `google-services.json` at build time and has not been run on a real device.

## Building the image (developers only)

Normal installs never build: they pull the published image. The Dockerfile lives in `docker/` (not in `deploy/`, which holds only the two files of a release):

```
docker build -f docker/Dockerfile -t superchat:1.4.1 .
docker tag superchat:1.4.1 ghcr.io/uri-travoski/superchat-server:1.4.1     # what release.yml publishes; lets deploy/docker-compose.yml run it offline
docker compose -f docker/docker-compose.build.yml up -d --build            # or: build and run a throw-away dev instance (port 8080, volume superchat-dev-data)
```
The image is multi-stage (node for the admin panel, rust for the server, debian-slim runtime with ca-certificates and tini,
non-root user, HEALTHCHECK). `tests/e2e/run.sh` builds it as `superchat-server:e2e` and starts its own MinIO, ntfy and fake Google Drive for the end-to-end suite;
none of that is part of the install.

## Update checks (apps and server)

Every app checks GitHub for a newer release: the desktop apps and the Android app under Settings > About (button **Check for updates**, plus an
automatic check when the app starts and then every 24 hours, which can be switched off there), and the server in the admin panel's Updates card.
Only the release list is read (`https://api.github.com/repos/uri-travoski/superchat/releases/latest`), and the only thing sent is `User-Agent: SuperChat/<version>`
(the server sends `SuperChat-Server/<version>`): no account, device or server information. Drafts and pre-releases are ignored.

* **Which file an app downloads** (names are those `release.yml` publishes, each with a `.sha256` file next to it): Android `superchat-<v>-android-arm64.apk` or `superchat-<v>-android-x86_64.apk` (the one that matches the device's `Build.SUPPORTED_ABIS`, first match wins; releases before 1.4 have one `superchat-<v>-android.apk`, still used as the fallback);
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
  (a proxy or tunnel on a shared network points at `http://superchat:8080`).
* **Environment variables**: every setting can be given as `SUPERCHAT_<NAME>` or as the legacy `ATOMIC_<NAME>`; when both are set
  the `SUPERCHAT_` one wins inside the server and the apps. Since 1.3.2 the compose file and `.env.example` use the `SUPERCHAT_*` spellings only (rename the lines of an old `.env`, see above).
* **Data volume**: the 1.0 compose project created the volume `atomic-chat_atomic-data`. To keep using it, add
  `SUPERCHAT_DATA_VOLUME=atomic-chat_atomic-data` to `.env` before `docker-compose up -d`. New installs get `superchat-data`.
  The database file inside is still called `atomic.db` (internal, nothing to do).
* Login links are now web addresses, `https://<your-server>/l/<token>`; links made by 1.0 (`atomic-chat://`) and `superchat://` links are still accepted by the apps.
  Behind a reverse proxy or tunnel make sure `/l/` and `/downloads` reach the server (a proxy or tunnel that passes everything through does this). Android may ask once whether
  to open such a link with SuperChat or the browser (app links are not verified); the landing page opens the app by itself either way.
* The existing server name (stored in the database) is not changed; edit it in the admin panel if you want it to read SuperChat.
* Desktop apps move `~/.local/share/atomic-chat` (Windows: `%APPDATA%\AtomicChat`) to the new SuperChat folder on first start and
  still read sign-in secrets saved under the old name. The Android app has a new application id (`app.superchat`), so it installs
  next to the old app and you sign in again with a new login link.
