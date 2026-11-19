# syntax=docker/dockerfile:1
# Build context is the repository root:  docker build -f docker/Dockerfile -t superchat-server .
# Developers and tests only: installs pull the published image (see deploy/docker-compose.yml).

# ---- admin panel (static files) ----
FROM node:22-alpine AS admin
WORKDIR /w/admin-web
COPY admin-web/package.json admin-web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY admin-web/ ./
COPY schema/ /w/schema/
RUN npm run build

# ---- server binary ----
FROM rust:1-slim-trixie AS build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /w
# The client and Android crates are workspace members but are not part of the server image.
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY apps/desktop/Cargo.toml apps/desktop/Cargo.toml
COPY apps/android/Cargo.toml apps/android/Cargo.toml
COPY tests/e2e/Cargo.toml tests/e2e/Cargo.toml
RUN mkdir -p apps/desktop/src apps/android/src tests/e2e/src \
 && touch apps/desktop/src/lib.rs apps/desktop/src/main.rs apps/android/src/lib.rs tests/e2e/src/lib.rs
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/w/target \
    cargo build --release -p atomic-server --bin superchat-server \
 && cp target/release/superchat-server /superchat-server

# ---- runtime ----
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tini && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /data superchat && mkdir -p /data /srv/admin && chown superchat /data
COPY --from=build /superchat-server /usr/local/bin/superchat-server
COPY --from=admin /w/admin-web/dist /srv/admin
ENV ATOMIC_BIND=0.0.0.0:8080 ATOMIC_DATA_DIR=/data ATOMIC_ADMIN_DIR=/srv/admin
USER superchat
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s CMD ["/usr/local/bin/superchat-server", "healthcheck"]
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/superchat-server"]
