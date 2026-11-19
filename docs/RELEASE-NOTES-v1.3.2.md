# SuperChat v1.3.2

The server is now a single standalone `docker-compose.yml`: one service, the published image, nothing else. The apps and the server behave exactly as in 1.3.1; this release is about how you deploy.

## What changed

* **One compose file, one service.** `docker-compose.yml` (attached to this release, also `deploy/docker-compose.yml` in the repository) runs only `superchat`, from `ghcr.io/uri-travoski/superchat-server:1.3.2`. There is no `build:` section and no Caddy, cloudflared, ntfy or MinIO service. It works with `docker compose` and `docker-compose`.
* **Settings file**: `superchat.env.example` (attached) is the commented `.env`; every variable is optional. All names are `SUPERCHAT_*`: `SUPERCHAT_PUBLIC_URL`, `SUPERCHAT_NAME`, `SUPERCHAT_BOOTSTRAP_ADMIN`, `SUPERCHAT_MAX_UPLOAD_MB`, `SUPERCHAT_NTFY_URL`, `SUPERCHAT_BACKUP_PASSPHRASE`, `SUPERCHAT_LOG_LEVEL`, `SUPERCHAT_IMAGE`, `SUPERCHAT_PORT`, `SUPERCHAT_BIND_ADDR`, `SUPERCHAT_DATA_VOLUME`.
* **Pinned image**: the default image tag is this release (`1.3.2`). To follow new releases automatically use `SUPERCHAT_IMAGE=ghcr.io/uri-travoski/superchat-server:latest`; otherwise update by downloading the new compose file (or setting `SUPERCHAT_IMAGE` to the new tag) and running `docker compose pull && docker compose up -d`.
* **Bring your own front door and push server**: run cloudflared (or any reverse proxy) and ntfy as your own containers. Point the tunnel at `http://<host>:8080`, or at `http://superchat:8080` on a shared Docker network (an example `networks:` block is in the compose file, commented). Pass the WebSocket path `/api/v1/ws`. Set `SUPERCHAT_PUBLIC_URL` to the public https address and `SUPERCHAT_NTFY_URL` to your ntfy base URL (`http://host.docker.internal:<port>` works for an ntfy on the same host). Details in `docs/DEPLOY.md`.
* **Attachment storage**: no MinIO or RustFS is shipped. Choose S3, Cloudflare R2, Backblaze B2, Google Drive or local disk in the admin panel on first setup (Storage page). R2: endpoint `https://<account-id>.r2.cloudflarestorage.com`, region `auto`. B2: endpoint `https://s3.<region>.backblazeb2.com`.
* **Release page**: named "SuperChat <version>", with these notes as its text, and the two install files attached.
* **Repository**: the Dockerfile moved to `docker/Dockerfile` (`docker/docker-compose.build.yml` builds and runs a local development image); `deploy/Caddyfile` and the MinIO profile were removed.

## Migrating from the 1.3.1 (or older) compose file

1. In your existing folder, keep `.env` and replace `docker-compose.yml` with the new one (`curl -fLO https://github.com/uri-travoski/superchat/releases/latest/download/docker-compose.yml`).
2. **Rename the variables**: `sed -i 's/^ATOMIC_/SUPERCHAT_/' .env`. (The compose file only substitutes `SUPERCHAT_*` now; the server inside still understands both.)
3. **Keep your data**: the volume must be the one your install already uses. Look with `docker volume ls`. If it is `superchat-data` (installs from 1.1 on) there is nothing to do. A 1.0-era install uses `atomic-chat_atomic-data`: add `SUPERCHAT_DATA_VOLUME=atomic-chat_atomic-data` to `.env`.
4. **Profiles are gone**: `docker compose --profile caddy|tunnel|ntfy|minio up -d` no longer exists. If you used Caddy, the tunnel or ntfy from the old file, start them as your own containers first (their old containers and volumes `caddy-data`, `ntfy-data` keep existing until you remove them; the tunnel token and ntfy address are yours to move). The values `ATOMIC_HOST`, `CADDY_TLS`, `CLOUDFLARE_TUNNEL_TOKEN`, `MINIO_*` and `NTFY_*` in `.env` are no longer read and can be deleted. If MinIO was your attachment store, keep it running yourself; the storage settings in the admin panel are unchanged.
5. `docker compose down --remove-orphans` (stops leftover profile containers; never use `-v`), then `docker compose pull && docker compose up -d`.
6. The port is still `SUPERCHAT_PORT` (was `ATOMIC_PORT`), default 8080. If your tunnel or proxy ran inside the old compose project, it reached the server as `http://superchat:8080`; give your own container the same network (see the `networks:` example) or point it at `http://<host>:8080`.

## Tested

* `docker-compose config` with only the `.env.example` values; the compose file started in a throw-away project (own port and volume, the image tagged as ghcr.io/uri-travoski/superchat-server:1.3.2): `/api/v1/meta`, `/admin`, healthy status.
* The full suite as for 1.3.1: `cargo test --workspace`, the end-to-end suite (`tests/e2e/run.sh`, with its own MinIO), the Playwright admin tests, clippy.

## Honest limitations

* The release workflow's new `release-files` job and the downloadable-asset URLs (`.../releases/latest/download/docker-compose.yml`) can only be exercised by publishing a release, which was not done here. The compose file was tested from a checkout, with the image built locally.
* The images are not verified on a real second machine; the compose file was run on the dev box with a locally built image.
