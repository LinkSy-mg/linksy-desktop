#!/usr/bin/env bash
# Build every Linux package for the version in Cargo.toml and publish it: tag vX.Y.Z here,
# and release it on this repo and, as linux-vX.Y.Z (not "latest", which belongs to the
# Android APKs), on LinkSy-mg/linksy-releases. CHANGELOG.md and the metainfo's <releases>
# must already have this version, committed.
# Usage: scripts/publish-release.sh
set -euo pipefail
cd "$(dirname "$0")/.."

V=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
grep -q "^## $V\$" CHANGELOG.md || { echo "CHANGELOG.md has no ## $V section"; exit 1; }
grep -q "release version=\"$V\"" data/mg.diary.linksy.Linksy.metainfo.xml || { echo "metainfo has no $V release"; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "commit your changes first"; exit 1; }

sed -i "s/^        tag: v.*/        tag: v$V/" packaging/flatpak/mg.diary.linksy.Linksy.yml
if ! git diff --quiet; then git commit -qam "flatpak: build v$V"; fi

scripts/package-linux.sh
scripts/package-flatpak.sh >/dev/null

D="dist/v$V"
FILES=("linksy-$V.flatpak" "linksy_${V}_amd64.deb" "linksy-$V-1.x86_64.rpm" "linksy-$V-1.suse.x86_64.rpm" "linksy-$V-linux-x86_64.tar.gz")
(cd "$D" && sha256sum "${FILES[@]}" > SHA256SUMS)
{
  cat <<NOTES
**Install** (x86_64):

| System | Download | Command |
|---|---|---|
| Any distribution | \`linksy-$V.flatpak\` | \`flatpak install --user linksy-$V.flatpak\` |
| Ubuntu 24.10+, Debian 13+ | \`linksy_${V}_amd64.deb\` | \`sudo apt install ./linksy_${V}_amd64.deb\` |
| Fedora 41+ | \`linksy-$V-1.x86_64.rpm\` | \`sudo dnf install ./linksy-$V-1.x86_64.rpm\` |
| openSUSE Tumbleweed | \`linksy-$V-1.suse.x86_64.rpm\` | \`sudo zypper install --allow-unsigned-rpm ./linksy-$V-1.suse.x86_64.rpm\` |
| Others (no root) | \`linksy-$V-linux-x86_64.tar.gz\` | extract, then \`./install.sh\` |

NOTES
  cat "$D/notes.md"
} > "$D/release-notes.md"

# Others push here too: catch up first.
git pull -q --rebase origin main
git push -q origin main
git tag -a "v$V" -m "Linksy $V"
git push -q origin "v$V"
ASSETS=("${FILES[@]/#/$D/}" "$D/SHA256SUMS")
gh release create "v$V" -R LinkSy-mg/linksy-desktop --title "Desktop release · Linksy $V for Linux" --notes-file "$D/release-notes.md" "${ASSETS[@]}"
gh release create "linux-v$V" -R LinkSy-mg/linksy-releases --latest=false --title "Desktop release · Linksy $V for Linux" --notes-file "$D/release-notes.md" "${ASSETS[@]}"

# The public repo's front page: its Linux download links point at this version.
PAGE=$(mktemp)
gh api repos/LinkSy-mg/linksy-releases/contents/README.md --jq .content | base64 -d > "$PAGE"
SHA=$(gh api repos/LinkSy-mg/linksy-releases/contents/README.md --jq .sha)
sed -i "/<!-- linux:start -->/,/<!-- linux:end -->/s/[0-9]\+\.[0-9]\+\.[0-9]\+/$V/g" "$PAGE"
gh api -X PUT repos/LinkSy-mg/linksy-releases/contents/README.md -f message="Linux downloads: $V" -f sha="$SHA" -f content="$(base64 -w0 "$PAGE")" >/dev/null
rm -f "$PAGE"
