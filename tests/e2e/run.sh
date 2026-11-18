#!/usr/bin/env bash
# SV-24 end-to-end: builds the server image, then runs tests/e2e/tests/sv24.rs against it
# (server in docker, MinIO in docker, fake Google Drive in-process, driven by client-core and the CLI).
# Host ports used: 18300 (server) and 18310 (MinIO).
set -euo pipefail
cd "$(dirname "$0")/../.."
# Use the rustup toolchain if present (a distro rustc may be too old for the dependencies).
export PATH="$HOME/.cargo/bin:$PATH"
docker build -f deploy/Dockerfile -t superchat-server:e2e .
cargo build -p atomic-cli
ATOMIC_E2E=1 cargo test -p atomic-e2e --test sv24 --test sv24_bots -- --nocapture --test-threads=1
