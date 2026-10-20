#!/usr/bin/env bash
# Build Linksy and install it for the current user (~/.local), with the service enabled.
# The GTK app needs gtk4-devel and libadwaita-devel; without them, build it first in a
# toolbox (`toolbox run -c linksy-dev cargo build --release -p linksy-app`) and this
# script installs that build.
# Usage: scripts/install-local.sh [--no-service]
set -euo pipefail
cd "$(dirname "$0")/.."

SERVICE=1
[ "${1:-}" = "--no-service" ] && SERVICE=0

BIN="$HOME/.local/bin"
SHARE="${XDG_DATA_HOME:-$HOME/.local/share}"
UNITS="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

cargo build --release -p linksyd -p linksy
if pkg-config --exists gtk4 libadwaita-1 gstreamer-1.0 2>/dev/null; then
  cargo build --release -p linksy-app -p linksy-screen
fi
WITH_APP=0
if [ -x target/release/linksy-app ]; then
  WITH_APP=1
else
  echo "gtk4/libadwaita headers not found and no app build: skipping the app (service and CLI still install)."
fi

install -Dm755 target/release/linksyd "$BIN/linksyd"
install -Dm755 target/release/linksy "$BIN/linksy"
# Screen sharing helper (started by linksyd only while sharing).
if [ -x target/release/linksy-screen ]; then
  install -Dm755 target/release/linksy-screen "$BIN/linksy-screen"
else
  echo "No screen helper build (needs GStreamer headers): screen sharing from this device is off."
fi
if [ "$WITH_APP" = 1 ]; then
  install -Dm755 target/release/linksy-app "$BIN/linksy-app"
  # Before 0.1.2 the app ID was app.linksy.Linksy.
  rm -f "$SHARE/applications/app.linksy.Linksy.desktop" "$SHARE/icons/hicolor/scalable/apps/app.linksy.Linksy.svg" "$SHARE/icons/hicolor/symbolic/apps/app.linksy.Linksy-symbolic.svg"
  # Absolute path: the desktop session's PATH may not include ~/.local/bin.
  mkdir -p "$SHARE/applications"
  sed "s|^Exec=linksy-app|Exec=$BIN/linksy-app|" data/mg.diary.linksy.Linksy.desktop > "$SHARE/applications/mg.diary.linksy.Linksy.desktop"
  command -v update-desktop-database >/dev/null && update-desktop-database -q "$SHARE/applications" || true
  # Files: right-click → Scripts → Send with Linksy.
  mkdir -p "$SHARE/nautilus/scripts"
  sed "s|@BINDIR@|$BIN|" "data/nautilus/Send with Linksy" > "$SHARE/nautilus/scripts/Send with Linksy"
  chmod 755 "$SHARE/nautilus/scripts/Send with Linksy"
fi
install -Dm644 data/icons/mg.diary.linksy.Linksy.svg "$SHARE/icons/hicolor/scalable/apps/mg.diary.linksy.Linksy.svg"
install -Dm644 data/icons/mg.diary.linksy.Linksy-symbolic.svg "$SHARE/icons/hicolor/symbolic/apps/mg.diary.linksy.Linksy-symbolic.svg"
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t -f "$SHARE/icons/hicolor" || true

mkdir -p "$UNITS"
sed "s|@BINDIR@|$BIN|" data/linksyd.service > "$UNITS/linksyd.service"

if [ "$SERVICE" = 1 ]; then
  systemctl --user daemon-reload
  systemctl --user enable --now linksyd.service
  echo "Installed. Service status: $(systemctl --user is-active linksyd.service)"
else
  echo "Installed to $BIN (service not enabled)."
fi
