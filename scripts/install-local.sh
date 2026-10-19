#!/usr/bin/env bash
# Build Linksy and install it for the current user (~/.local), with the service enabled.
# The GTK app needs gtk4-devel and libadwaita-devel (or run inside the linksy-dev toolbox).
set -euo pipefail
cd "$(dirname "$0")/.."

BIN="$HOME/.local/bin"
SHARE="${XDG_DATA_HOME:-$HOME/.local/share}"
UNITS="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

cargo build --release -p linksyd -p linksy
if pkg-config --exists gtk4 libadwaita-1 2>/dev/null; then
  cargo build --release -p linksy-app
  WITH_APP=1
else
  echo "gtk4/libadwaita headers not found: skipping the app (service and CLI still install)."
  WITH_APP=0
fi

install -Dm755 target/release/linksyd "$BIN/linksyd"
install -Dm755 target/release/linksy "$BIN/linksy"
if [ "$WITH_APP" = 1 ]; then
  install -Dm755 target/release/linksy-app "$BIN/linksy-app"
  install -Dm644 data/app.linksy.Linksy.desktop "$SHARE/applications/app.linksy.Linksy.desktop"
fi
install -Dm644 data/icons/app.linksy.Linksy.svg "$SHARE/icons/hicolor/scalable/apps/app.linksy.Linksy.svg"
install -Dm644 data/icons/app.linksy.Linksy-symbolic.svg "$SHARE/icons/hicolor/symbolic/apps/app.linksy.Linksy-symbolic.svg"
sed "s|@BINDIR@|$BIN|" data/linksyd.service > /tmp/linksyd.service.$$
install -Dm644 /tmp/linksyd.service.$$ "$UNITS/linksyd.service"
rm -f /tmp/linksyd.service.$$

systemctl --user daemon-reload
systemctl --user enable --now linksyd.service
echo "Installed. Service status: $(systemctl --user is-active linksyd.service)"
