#!/usr/bin/env bash
# Build the Linux release: dist/vX.Y.Z/ gets the tarball (linksy-X.Y.Z-linux-x86_64.tar.gz,
# the GTK app), and three .debs and three .rpms for Fedora and openSUSE each: linksy-common,
# linksy (GTK) and linksy-qt (Qt, for KDE Plasma). The Flatpak is package-flatpak.sh.
# The app and screen helper need GTK and GStreamer headers; without them on the host,
# they're built in the linksy-dev toolbox. Packing needs rpmbuild and dpkg-deb (Fedora:
# rpm-build dpkg; openSUSE: rpm-build dpkg).
# Usage: scripts/package-linux.sh
#   LINKSY_PREBUILT=1: pack the programs already in target/release (linksy-qt included), as
#   CI does after building them in its own container; nothing is built here then.
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
PREBUILT=${LINKSY_PREBUILT:-}
if [ -n "$PREBUILT" ]; then
  for b in linksyd linksy linksy-app linksy-screen linksy-qt; do
    [ -x "target/release/$b" ] || { echo "LINKSY_PREBUILT: target/release/$b is missing"; exit 1; }
  done
else
  cargo build --release -p linksyd -p linksy
  if pkg-config --exists gtk4 libadwaita-1 gstreamer-1.0 2>/dev/null; then
    cargo build --release -p linksy-app -p linksy-screen
  elif command -v toolbox >/dev/null; then
    toolbox run -c linksy-dev bash -lc 'export PATH=$HOME/.cargo/bin:$PATH; cargo build --release -p linksy-app -p linksy-screen'
  else
    echo "GTK/GStreamer headers missing and no toolbox: install gtk4-devel libadwaita-devel gstreamer-devel gstreamer-plugins-base-devel (openSUSE) or gstreamer1-devel gstreamer1-plugins-base-devel (Fedora)"
    exit 1
  fi
fi

rm -rf "$STAGE"; mkdir -p "$STAGE/bin" "$STAGE/share/icons"
for b in linksyd linksy linksy-app linksy-screen; do install -m755 "target/release/$b" "$STAGE/bin/$b"; done
install -m755 data/linksy-camera-setup data/linksy-usb-setup data/linksy-system-setup "$STAGE/bin/"
cp data/mg.diary.linksy.Linksy.desktop data/linksyd.service data/linksyd-autostart.desktop "data/nautilus/Send with Linksy" "$STAGE/share/"
cp data/icons/*.svg "$STAGE/share/icons/"
install -m755 packaging/linux/install.sh packaging/linux/uninstall.sh "$STAGE/"
cp packaging/linux/README.txt LICENSE THIRD-PARTY-NOTICES "$STAGE/"

tar -C "$DIST" -czf "$DIST/$NAME.tar.gz" "$NAME"

# The Qt app, built against Debian 13's Qt (the oldest it supports) so it runs wherever
# the packages install; in a container, so no Qt is needed here.
if [ -n "$PREBUILT" ]; then
  QT_BIN=target/release/linksy-qt
else
  if ! command -v podman >/dev/null; then
    echo "podman is missing: the Qt app (linksy-qt) is built in a container"; exit 1
  fi
  podman image exists linksy-qt-release || podman build -q -t linksy-qt-release -f packaging/qt/Containerfile.release packaging/qt >/dev/null
  podman volume create linksy-cargo-deb >/dev/null 2>&1 || true
  podman run --rm -v "$(pwd)":/src:z -v linksy-cargo-deb:/root/.cargo/registry -w /src -e CARGO_TARGET_DIR=/src/target/qt-release \
    linksy-qt-release cargo build --release -p linksy-qt
  QT_BIN=target/qt-release/release/linksy-qt
fi

# .deb and .rpm share one system-wide layout, with the service as a global user unit. Three
# packages, as Transmission splits its apps: linksy-common (service, helpers, tools, the
# menu entry), linksy (GTK app) and linksy-qt (Qt app); linksy-open, the menu entry's
# command, opens the Qt window on Plasma when it's installed.
ROOT="$DIST/root"
rm -rf "$ROOT"
for b in linksyd linksy linksy-app linksy-screen; do install -Dm755 "target/release/$b" "$ROOT/usr/bin/$b"; done
install -Dm755 "$QT_BIN" "$ROOT/usr/bin/linksy-qt"
install -Dm755 data/linksy-open "$ROOT/usr/bin/linksy-open"
sed "s|@BINDIR@|/usr/bin|" data/linksyd.service | install -Dm644 /dev/stdin "$ROOT/usr/lib/systemd/user/linksyd.service"
install -Dm644 data/linksyd-autostart.desktop "$ROOT/etc/xdg/autostart/linksyd.desktop"
sed "s|^Exec=linksy-app|Exec=linksy-open|" data/mg.diary.linksy.Linksy.desktop | install -Dm644 /dev/stdin "$ROOT/usr/share/applications/mg.diary.linksy.Linksy.desktop"
install -Dm644 data/mg.diary.linksy.Linksy.metainfo.xml "$ROOT/usr/share/metainfo/mg.diary.linksy.Linksy.metainfo.xml"
install -Dm644 data/icons/mg.diary.linksy.Linksy.svg "$ROOT/usr/share/icons/hicolor/scalable/apps/mg.diary.linksy.Linksy.svg"
install -Dm644 data/icons/mg.diary.linksy.Linksy-symbolic.svg "$ROOT/usr/share/icons/hicolor/symbolic/apps/mg.diary.linksy.Linksy-symbolic.svg"
install -Dm644 -t "$ROOT/usr/share/licenses/linksy" LICENSE THIRD-PARTY-NOTICES
for s in linksy-camera-setup linksy-usb-setup linksy-system-setup; do install -Dm755 "data/$s" "$ROOT/usr/libexec/linksy/$s"; done
install -Dm644 data/linksy.firewalld.xml "$ROOT/usr/lib/firewalld/services/linksy.xml"
install -Dm644 data/70-linksy-usb.rules "$ROOT/usr/lib/udev/rules.d/70-linksy-usb.rules"

# A .deb holds one package: each gets its own tree, cut from the shared one.
deb() {
  local pkg=$1 control=$2; shift 2
  local tree="$DIST/deb-$pkg"
  rm -rf "$tree"; mkdir -p "$tree/DEBIAN"
  for f in "$@"; do install -D -m "$(stat -c %a "$ROOT/$f")" "$ROOT/$f" "$tree/$f"; done
  sed -e "s|@VERSION@|$V|g" -e "s|@SIZE@|$(du -sk --exclude=DEBIAN "$tree" | cut -f1)|" "packaging/linux/deb/$control" > "$tree/DEBIAN/control"
  if [ "$pkg" = linksy-common ]; then
    install -m755 packaging/linux/deb/postinst.common "$tree/DEBIAN/postinst"
    install -m755 packaging/linux/deb/prerm.common "$tree/DEBIAN/prerm"
  fi
  dpkg-deb --root-owner-group -Zxz --build "$tree" "$DIST/${pkg}_${V}_amd64.deb" >/dev/null
  rm -rf "$tree"
}
COMMON=$(cd "$ROOT" && find . -type f ! -path ./usr/bin/linksy-app ! -path ./usr/bin/linksy-qt | sed 's|^\./||')
# shellcheck disable=SC2086
deb linksy-common control.common $COMMON
deb linksy control.gtk usr/bin/linksy-app
deb linksy-qt control.qt usr/bin/linksy-qt

# Three RPMs from one spec, twice: Fedora's names for the dependencies, then openSUSE's.
RPMTOP="$(pwd)/$DIST/rpmbuild"
for suse in 0 1; do
  rpmbuild -bb --quiet --define "_topdir $RPMTOP" --define "version $V" --define "root $(pwd)/$ROOT" --define "linksy_suse $suse" packaging/linux/rpm/linksy.spec
done
mv "$RPMTOP"/RPMS/x86_64/*.rpm "$DIST/"
rm -rf "$RPMTOP" "$ROOT"
# Release notes: this version's section of CHANGELOG.md.
awk -v v="$V" '/^## /{p = index($0, v) > 0; next} p' CHANGELOG.md > "$DIST/notes.md"
(cd "$DIST" && sha256sum "$NAME.tar.gz" ./*.deb ./*.rpm | sed 's|  \./|  |' > SHA256SUMS)
ls -1 "$DIST"/*.tar.gz "$DIST"/*.deb "$DIST"/*.rpm
