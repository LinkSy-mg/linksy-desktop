#!/usr/bin/env bash
# The release notes for version V: how to install each package, then the version's section
# of CHANGELOG.md (dist/vV/notes.md, written by package-linux.sh).
# Usage: scripts/release-notes.sh V > release-notes.md
set -euo pipefail
cd "$(dirname "$0")/.."
V=${1:?version}
cat <<NOTES
**Install** (x86_64). With a package, download \`linksy-common\` and the app for your desktop:
\`linksy\` for GNOME and most desktops, \`linksy-qt\` for KDE Plasma 6.

| System | Download | Command |
|---|---|---|
| Any distribution | \`linksy-$V.flatpak\` | \`flatpak install --user linksy-$V.flatpak\` |
| Ubuntu 24.10+, Debian 13+ | \`linksy-common_${V}_amd64.deb\` and \`linksy_${V}_amd64.deb\` or \`linksy-qt_${V}_amd64.deb\` | \`sudo apt install ./linksy-common_${V}_amd64.deb ./linksy_${V}_amd64.deb\` |
| Fedora 41+ | \`linksy-common-$V-1.x86_64.rpm\` and \`linksy-$V-1.x86_64.rpm\` or \`linksy-qt-$V-1.x86_64.rpm\` | \`sudo dnf install ./linksy-common-$V-1.x86_64.rpm ./linksy-$V-1.x86_64.rpm\` |
| openSUSE Tumbleweed | the same, \`.suse.x86_64.rpm\` | \`sudo zypper install --allow-unsigned-rpm ./linksy-common-$V-1.suse.x86_64.rpm ./linksy-$V-1.suse.x86_64.rpm\` |
| Others (no root) | \`linksy-$V-linux-x86_64.tar.gz\` (GTK app) | extract, then \`./install.sh\` |

On KDE Plasma, use \`linksy-qt\` in place of \`linksy\` in the commands. The Qt app needs Qt 6.8 and KDE Frameworks 6.13 or newer (Debian 13, Ubuntu 25.04+, Fedora, openSUSE Tumbleweed).

Opening the files one at a time instead (Software, Discover, gdebi): \`linksy-common\` first, then the app, both of this version.

NOTES
cat "dist/v$V/notes.md"
