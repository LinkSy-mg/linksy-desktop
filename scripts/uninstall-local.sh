#!/usr/bin/env bash
# Remove what install-local.sh installed. Keeps your identity, paired devices and settings
# (~/.local/share/linksy) unless --purge is given.
set -euo pipefail
BIN="$HOME/.local/bin"
SHARE="${XDG_DATA_HOME:-$HOME/.local/share}"
UNITS="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

if [ -f "$UNITS/linksyd.service" ]; then
  systemctl --user disable --now linksyd.service 2>/dev/null || true
  rm -f "$UNITS/linksyd.service"
  systemctl --user daemon-reload 2>/dev/null || true
fi
rm -f "$BIN/linksyd" "$BIN/linksy" "$BIN/linksy-app" "$BIN/linksy-screen"
rm -f "$SHARE/applications/mg.diary.linksy.Linksy.desktop" "$SHARE/nautilus/scripts/Send with Linksy"
rm -f "$SHARE/icons/hicolor/scalable/apps/mg.diary.linksy.Linksy.svg" "$SHARE/icons/hicolor/symbolic/apps/mg.diary.linksy.Linksy-symbolic.svg"
if [ "${1:-}" = "--purge" ]; then
  rm -rf "$SHARE/linksy"
  echo "Removed Linksy and its data."
else
  echo "Removed Linksy. Your device identity and paired devices are kept in $SHARE/linksy."
fi
