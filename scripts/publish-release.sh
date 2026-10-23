#!/usr/bin/env bash
# Release the version in Cargo.toml: tag vX.Y.Z and push it. GitHub does the rest (the
# Packages workflow): it builds every package and the Flatpak, installs them on every
# supported distribution, and only when all of that passes publishes the release here and,
# as linux-vX.Y.Z (not "latest", which belongs to the Android APKs), on
# LinkSy-mg/linksy-releases. CHANGELOG.md and the metainfo's <releases> must already have
# this version, committed.
# Usage: scripts/publish-release.sh
set -euo pipefail
cd "$(dirname "$0")/.."

V=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
grep -q "^## $V\$" CHANGELOG.md || { echo "CHANGELOG.md has no ## $V section"; exit 1; }
grep -q "release version=\"$V\"" data/mg.diary.linksy.Linksy.metainfo.xml || { echo "metainfo has no $V release"; exit 1; }
[ -z "$(git status --porcelain --untracked-files=no)" ] || { echo "commit your changes first"; exit 1; }

# Flathub builds from the tag the manifest names
sed -i "s/^        tag: v.*/        tag: v$V/" packaging/flatpak/mg.diary.linksy.Linksy.yml
if ! git diff --quiet; then git commit -qam "build(flatpak): build v$V"; fi

# Others push here too: catch up first.
git pull -q --rebase origin main
git push -q origin main
git tag -a "v$V" -m "Linksy $V"
git push -q origin "v$V"
echo "Tagged v$V. GitHub builds, tests and publishes it:"
echo "  https://github.com/LinkSy-mg/linksy-desktop/actions/workflows/packages.yml"
