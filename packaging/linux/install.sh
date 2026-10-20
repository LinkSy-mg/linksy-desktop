#!/usr/bin/env bash
# Install Linksy from this release folder for the current user (~/.local), and start the
# background service. Nothing goes outside your home folder; no root needed.
# Usage: ./install.sh [--no-service]      Remove with: ./uninstall.sh
set -euo pipefail
cd "$(dirname "$0")"

BIN="$HOME/.local/bin"
SHARE="${XDG_DATA_HOME:-$HOME/.local/share}"
UNITS="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

for b in linksyd linksy linksy-app linksy-screen; do
  install -Dm755 "bin/$b" "$BIN/$b"
done
# Before 0.1.2 the app ID was app.linksy.Linksy.
rm -f "$SHARE/applications/app.linksy.Linksy.desktop" "$SHARE/icons/hicolor/scalable/apps/app.linksy.Linksy.svg" "$SHARE/icons/hicolor/symbolic/apps/app.linksy.Linksy-symbolic.svg"
mkdir -p "$SHARE/applications"
# Absolute path: the desktop session's PATH may not include ~/.local/bin.
sed "s|^Exec=linksy-app|Exec=$BIN/linksy-app|" share/mg.diary.linksy.Linksy.desktop > "$SHARE/applications/mg.diary.linksy.Linksy.desktop"
install -Dm644 share/icons/mg.diary.linksy.Linksy.svg "$SHARE/icons/hicolor/scalable/apps/mg.diary.linksy.Linksy.svg"
install -Dm644 share/icons/mg.diary.linksy.Linksy-symbolic.svg "$SHARE/icons/hicolor/symbolic/apps/mg.diary.linksy.Linksy-symbolic.svg"
mkdir -p "$SHARE/nautilus/scripts"
sed "s|@BINDIR@|$BIN|" "share/Send with Linksy" > "$SHARE/nautilus/scripts/Send with Linksy"
chmod 755 "$SHARE/nautilus/scripts/Send with Linksy"
command -v update-desktop-database >/dev/null && update-desktop-database -q "$SHARE/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t -f "$SHARE/icons/hicolor" || true
mkdir -p "$UNITS"
sed "s|@BINDIR@|$BIN|" share/linksyd.service > "$UNITS/linksyd.service"

if [ "${1:-}" != "--no-service" ]; then
  systemctl --user daemon-reload
  systemctl --user enable --now linksyd.service
  echo "Linksy is installed and running. Open it from your apps, or run: linksy-app"
else
  echo "Linksy is installed to $BIN (service not started)."
fi
