#!/usr/bin/env bash
# Build the Linux release: dist/vX.Y.Z/ gets the tarball (linksy-X.Y.Z-linux-x86_64.tar.gz),
# linksy_X.Y.Z_amd64.deb and linksy-X.Y.Z-1.x86_64.rpm. The Flatpak is package-flatpak.sh.
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

# .deb and .rpm share one system-wide layout, with the service as a global user unit.
ROOT="$DIST/root"
rm -rf "$ROOT"
for b in linksyd linksy linksy-app linksy-screen; do install -Dm755 "target/release/$b" "$ROOT/usr/bin/$b"; done
sed "s|@BINDIR@|/usr/bin|" data/linksyd.service | install -Dm644 /dev/stdin "$ROOT/usr/lib/systemd/user/linksyd.service"
install -Dm644 data/app.linksy.Linksy.desktop "$ROOT/usr/share/applications/app.linksy.Linksy.desktop"
install -Dm644 data/app.linksy.Linksy.metainfo.xml "$ROOT/usr/share/metainfo/app.linksy.Linksy.metainfo.xml"
install -Dm644 data/icons/app.linksy.Linksy.svg "$ROOT/usr/share/icons/hicolor/scalable/apps/app.linksy.Linksy.svg"
install -Dm644 data/icons/app.linksy.Linksy-symbolic.svg "$ROOT/usr/share/icons/hicolor/symbolic/apps/app.linksy.Linksy-symbolic.svg"
install -Dm644 LICENSE "$ROOT/usr/share/licenses/linksy/LICENSE"

DEB="$DIST/deb"
rm -rf "$DEB"; cp -a "$ROOT" "$DEB"; mkdir -p "$DEB/DEBIAN"
sed -e "s|@VERSION@|$V|" -e "s|@SIZE@|$(du -sk "$ROOT" | cut -f1)|" packaging/linux/deb/control > "$DEB/DEBIAN/control"
install -m755 packaging/linux/deb/postinst packaging/linux/deb/prerm "$DEB/DEBIAN/"
dpkg-deb --root-owner-group -Zxz --build "$DEB" "$DIST/linksy_${V}_amd64.deb" >/dev/null
rm -rf "$DEB"

RPMTOP="$(pwd)/$DIST/rpmbuild"
rpmbuild -bb --quiet --define "_topdir $RPMTOP" --define "version $V" --define "root $(pwd)/$ROOT" packaging/linux/rpm/linksy.spec
mv "$RPMTOP"/RPMS/x86_64/linksy-"$V"-1.x86_64.rpm "$DIST/"
rm -rf "$RPMTOP" "$ROOT"
# Release notes: this version's section of CHANGELOG.md.
awk -v v="$V" '/^## /{p = index($0, v) > 0; next} p' CHANGELOG.md > "$DIST/notes.md"
(cd "$DIST" && sha256sum "$NAME.tar.gz" "linksy_${V}_amd64.deb" "linksy-$V-1.x86_64.rpm" > SHA256SUMS)
ls -1 "$DIST"/*.tar.gz "$DIST"/*.deb "$DIST"/*.rpm
