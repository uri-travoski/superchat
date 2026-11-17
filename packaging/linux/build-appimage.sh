#!/usr/bin/env bash
# Builds dist/Atomic_Chat-<version>-x86_64.AppImage from a release build. Needs appimagetool
# (downloaded into packaging/.tools when missing). Usage: packaging/linux/build-appimage.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
cargo build --release -p atomic-desktop
APPDIR=$(mktemp -d)/AtomicChat.AppDir
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps"
cp target/release/atomic-chat "$APPDIR/usr/bin/"
cp packaging/linux/atomic-chat.desktop "$APPDIR/"
cp packaging/linux/atomic-chat.desktop "$APPDIR/usr/share/applications/"
cp assets/icon/atomic-chat-256.png "$APPDIR/atomic-chat.png"
cp assets/icon/atomic-chat-256.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/atomic-chat.png"
ln -s usr/bin/atomic-chat "$APPDIR/AppRun"
TOOLS=packaging/.tools
mkdir -p "$TOOLS" dist
if ! command -v appimagetool >/dev/null && [ ! -x "$TOOLS/appimagetool" ]; then
  curl -fsSL -o "$TOOLS/appimagetool" https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x "$TOOLS/appimagetool"
fi
AT=$(command -v appimagetool || echo "$TOOLS/appimagetool")
ARCH=x86_64 "$AT" --appimage-extract-and-run "$APPDIR" "dist/Atomic_Chat-$VERSION-x86_64.AppImage"
ls -l dist/*.AppImage
