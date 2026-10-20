#!/usr/bin/env bash
# Build the Flatpak from this checkout and bundle it: dist/vX.Y.Z/linksy-X.Y.Z.flatpak
# Needs (user install, no root): flatpak install --user flathub org.flatpak.Builder
# After Cargo.lock changes, regenerate packaging/flatpak/cargo-sources.json with
# flatpak-cargo-generator (github.com/flatpak/flatpak-builder-tools, cargo/).
# Usage: scripts/package-flatpak.sh [--install]
set -euo pipefail
cd "$(dirname "$0")/.."

V=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
WORK=target/flatpak
mkdir -p "$WORK" "dist/v$V"
cp packaging/flatpak/cargo-sources.json "$WORK/"
# The manifest's sources are the release tag (that's what Flathub builds); here, this checkout.
awk -v root="$(pwd)" '
  /# @SOURCE@/ { print "      - type: dir"; print "        path: " root; print "        skip: [target, dist, .git]"; skip = 3; next }
  skip > 0 { skip--; next }
  { print }
' packaging/flatpak/mg.diary.linksy.Linksy.yml > "$WORK/mg.diary.linksy.Linksy.yml"

flatpak run --filesystem="$(pwd)" org.flatpak.Builder --user --install-deps-from=flathub --disable-rofiles-fuse \
  --state-dir="$WORK/state" --repo="$WORK/repo" --force-clean ${1:+--install} \
  "$WORK/build" "$WORK/mg.diary.linksy.Linksy.yml"
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
  "$WORK/repo" "dist/v$V/linksy-$V.flatpak" mg.diary.linksy.Linksy
echo "dist/v$V/linksy-$V.flatpak"
