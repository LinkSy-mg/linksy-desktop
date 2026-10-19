#!/usr/bin/env bash
# Build the Linux release tarball: dist/vX.Y.Z/linksy-X.Y.Z-linux-x86_64.tar.gz
# The app and screen helper need GTK and GStreamer headers; without them on the host,
# they're built in the linksy-dev toolbox.
# Usage: scripts/package-linux.sh
set -euo pipefail
cd "$(dirname "$0")/.."

V=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
NAME="linksy-$V-linux-x86_64"
DIST="dist/v$V"
STAGE="$DIST/$NAME"

cargo build --release -p linksyd -p linksy
if pkg-config --exists gtk4 libadwaita-1 gstreamer-1.0 2>/dev/null; then
  cargo build --release -p linksy-app -p linksy-screen
else
  toolbox run -c linksy-dev bash -lc 'export PATH=$HOME/.cargo/bin:$PATH; cargo build --release -p linksy-app -p linksy-screen'
fi

rm -rf "$STAGE"; mkdir -p "$STAGE/bin" "$STAGE/share/icons"
for b in linksyd linksy linksy-app linksy-screen; do install -m755 "target/release/$b" "$STAGE/bin/$b"; done
cp data/app.linksy.Linksy.desktop data/linksyd.service "data/nautilus/Send with Linksy" "$STAGE/share/"
cp data/icons/*.svg "$STAGE/share/icons/"
install -m755 packaging/linux/install.sh packaging/linux/uninstall.sh "$STAGE/"
cp packaging/linux/README.txt LICENSE* "$STAGE/" 2>/dev/null || cp packaging/linux/README.txt "$STAGE/"

tar -C "$DIST" -czf "$DIST/$NAME.tar.gz" "$NAME"
# Release notes: this version's section of CHANGELOG.md.
awk -v v="$V" '/^## /{p = index($0, v) > 0; next} p' CHANGELOG.md > "$DIST/notes.md"
(cd "$DIST" && sha256sum "$NAME.tar.gz" > SHA256SUMS)
echo "$DIST/$NAME.tar.gz"
