#!/usr/bin/env bash
# Build the Linux release: dist/vX.Y.Z/ gets the tarball (linksy-X.Y.Z-linux-x86_64.tar.gz),
# linksy_X.Y.Z_amd64.deb, linksy-X.Y.Z-1.x86_64.rpm (Fedora) and linksy-X.Y.Z-1.suse.x86_64.rpm
# (openSUSE). The Flatpak is package-flatpak.sh.
# The app and screen helper need GTK and GStreamer headers; without them on the host,
# they're built in the linksy-dev toolbox. Packing needs rpmbuild and dpkg-deb (Fedora:
# rpm-build dpkg; openSUSE: rpm-build dpkg).
# Usage: scripts/package-linux.sh
set -euo pipefail
cd "$(dirname "$0")/.."

for tool in rpmbuild dpkg-deb; do
  command -v "$tool" >/dev/null || { echo "$tool is missing: install rpm-build and dpkg (Fedora: dnf; openSUSE: zypper)"; exit 1; }
done

V=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
NAME="linksy-$V-linux-x86_64"
DIST="dist/v$V"
STAGE="$DIST/$NAME"

# linksy-usb-setup carries a copy of the udev rule (for the tarball): they must match.
sed -n "/<<'RULES'/,/^RULES\$/p" data/linksy-usb-setup | sed '1d;$d' | diff -q - data/70-linksy-usb.rules >/dev/null \
  || { echo "data/linksy-usb-setup is out of date with data/70-linksy-usb.rules"; exit 1; }
cargo build --release -p linksyd -p linksy
if pkg-config --exists gtk4 libadwaita-1 gstreamer-1.0 2>/dev/null; then
  cargo build --release -p linksy-app -p linksy-screen
elif command -v toolbox >/dev/null; then
  toolbox run -c linksy-dev bash -lc 'export PATH=$HOME/.cargo/bin:$PATH; cargo build --release -p linksy-app -p linksy-screen'
else
  echo "GTK/GStreamer headers missing and no toolbox: install gtk4-devel libadwaita-devel gstreamer-devel gstreamer-plugins-base-devel (openSUSE) or gstreamer1-devel gstreamer1-plugins-base-devel (Fedora)"
  exit 1
fi

rm -rf "$STAGE"; mkdir -p "$STAGE/bin" "$STAGE/share/icons"
for b in linksyd linksy linksy-app linksy-screen; do install -m755 "target/release/$b" "$STAGE/bin/$b"; done
install -m755 data/linksy-camera-setup data/linksy-usb-setup data/linksy-system-setup "$STAGE/bin/"
cp data/mg.diary.linksy.Linksy.desktop data/linksyd.service "data/nautilus/Send with Linksy" "$STAGE/share/"
cp data/icons/*.svg "$STAGE/share/icons/"
install -m755 packaging/linux/install.sh packaging/linux/uninstall.sh "$STAGE/"
cp packaging/linux/README.txt LICENSE THIRD-PARTY-NOTICES "$STAGE/"

tar -C "$DIST" -czf "$DIST/$NAME.tar.gz" "$NAME"

# .deb and .rpm share one system-wide layout, with the service as a global user unit.
ROOT="$DIST/root"
rm -rf "$ROOT"
for b in linksyd linksy linksy-app linksy-screen; do install -Dm755 "target/release/$b" "$ROOT/usr/bin/$b"; done
sed "s|@BINDIR@|/usr/bin|" data/linksyd.service | install -Dm644 /dev/stdin "$ROOT/usr/lib/systemd/user/linksyd.service"
install -Dm644 data/mg.diary.linksy.Linksy.desktop "$ROOT/usr/share/applications/mg.diary.linksy.Linksy.desktop"
install -Dm644 data/mg.diary.linksy.Linksy.metainfo.xml "$ROOT/usr/share/metainfo/mg.diary.linksy.Linksy.metainfo.xml"
install -Dm644 data/icons/mg.diary.linksy.Linksy.svg "$ROOT/usr/share/icons/hicolor/scalable/apps/mg.diary.linksy.Linksy.svg"
install -Dm644 data/icons/mg.diary.linksy.Linksy-symbolic.svg "$ROOT/usr/share/icons/hicolor/symbolic/apps/mg.diary.linksy.Linksy-symbolic.svg"
install -Dm644 -t "$ROOT/usr/share/licenses/linksy" LICENSE THIRD-PARTY-NOTICES
for s in linksy-camera-setup linksy-usb-setup linksy-system-setup; do install -Dm755 "data/$s" "$ROOT/usr/libexec/linksy/$s"; done
install -Dm644 data/linksy.firewalld.xml "$ROOT/usr/lib/firewalld/services/linksy.xml"
install -Dm644 data/70-linksy-usb.rules "$ROOT/usr/lib/udev/rules.d/70-linksy-usb.rules"

DEB="$DIST/deb"
rm -rf "$DEB"; cp -a "$ROOT" "$DEB"; mkdir -p "$DEB/DEBIAN"
sed -e "s|@VERSION@|$V|" -e "s|@SIZE@|$(du -sk "$ROOT" | cut -f1)|" packaging/linux/deb/control > "$DEB/DEBIAN/control"
install -m755 packaging/linux/deb/postinst packaging/linux/deb/prerm "$DEB/DEBIAN/"
dpkg-deb --root-owner-group -Zxz --build "$DEB" "$DIST/linksy_${V}_amd64.deb" >/dev/null
rm -rf "$DEB"

# Two RPMs from one spec: Fedora's names for the dependencies, then openSUSE's.
RPMTOP="$(pwd)/$DIST/rpmbuild"
rpmbuild -bb --quiet --define "_topdir $RPMTOP" --define "version $V" --define "root $(pwd)/$ROOT" packaging/linux/rpm/linksy.spec
mv "$RPMTOP"/RPMS/x86_64/linksy-"$V"-1.x86_64.rpm "$DIST/"
rpmbuild -bb --quiet --define "_topdir $RPMTOP" --define "version $V" --define "root $(pwd)/$ROOT" --define "linksy_suse 1" packaging/linux/rpm/linksy.spec
mv "$RPMTOP"/RPMS/x86_64/linksy-"$V"-1.suse.x86_64.rpm "$DIST/"
rm -rf "$RPMTOP" "$ROOT"
# Release notes: this version's section of CHANGELOG.md.
awk -v v="$V" '/^## /{p = index($0, v) > 0; next} p' CHANGELOG.md > "$DIST/notes.md"
(cd "$DIST" && sha256sum "$NAME.tar.gz" "linksy_${V}_amd64.deb" "linksy-$V-1.x86_64.rpm" "linksy-$V-1.suse.x86_64.rpm" > SHA256SUMS)
ls -1 "$DIST"/*.tar.gz "$DIST"/*.deb "$DIST"/*.rpm
