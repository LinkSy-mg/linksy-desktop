#!/usr/bin/env bash
# Install the built packages in clean containers and check they work: each app installs with
# what it needs, no library is missing, the Qt window loads its QML, linksy-open picks the
# right window, upgrading from the previous release moves files over cleanly, and installing
# one file at a time (linksy-common first, as gdebi or a software store does) works.
# Usage: scripts/test-packages.sh [distro…]     (default: all; needs podman)
# Exits 1 when any check fails (CI runs one distribution per job).
#   Distros: fedora-44 fedora-43 opensuse debian-13 ubuntu-26.04 ubuntu-25.10 ubuntu-24.04
# Reads dist/vX.Y.Z (this version) and, for the upgrade, dist/v<previous> when it's there.
set -uo pipefail
cd "$(dirname "$0")/.."
LOG=$(mktemp)
trap 'rm -f "$LOG"' EXIT
V=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
PREV=$(ls -1d dist/v* 2>/dev/null | sed 's|dist/v||' | sort -V | grep -v "^$V$" | tail -1)
ALL=(fedora-44 fedora-43 opensuse debian-13 ubuntu-26.04 ubuntu-25.10 ubuntu-24.04)
# The GTK app only, since 0.9: Qt 6.4 is too old for the Qt app (refused), and no earlier
# release installed there to upgrade from
GTK_ONLY=(ubuntu-24.04)
[ $# -gt 0 ] && ALL=("$@")

image() {
  case $1 in
    fedora-*) echo "registry.fedoraproject.org/fedora:${1#fedora-}" ;;
    opensuse) echo "registry.opensuse.org/opensuse/tumbleweed:latest" ;;
    debian-*) echo "docker.io/library/debian:${1#debian-}" ;;
    ubuntu-*) echo "docker.io/library/ubuntu:${1#ubuntu-}" ;;
  esac
}

# Commands to install packages (paths under /pkgs), per family
install_cmd() {
  case $1 in
    fedora-*) echo "dnf install -y -q" ;;
    opensuse) echo "zypper -q --non-interactive install --allow-unsigned-rpm" ;;
    *) echo "apt-get install -y -qq" ;;
  esac
}

files() {   # this version's packages for a distro and app (gtk or qt)
  local d=$1 app=$2 pkg=linksy
  [ "$app" = qt ] && pkg=linksy-qt
  case $d in
    fedora-*) echo "/pkgs/v$V/linksy-common-$V-1.x86_64.rpm /pkgs/v$V/$pkg-$V-1.x86_64.rpm" ;;
    opensuse) echo "/pkgs/v$V/linksy-common-$V-1.suse.x86_64.rpm /pkgs/v$V/$pkg-$V-1.suse.x86_64.rpm" ;;
    *) echo "/pkgs/v$V/linksy-common_${V}_amd64.deb /pkgs/v$V/${pkg}_${V}_amd64.deb" ;;
  esac
}

old_files() {   # the previous release's GTK app: one linksy package with everything before 0.8.4, with linksy-common since
  local common="" app
  case $1 in
    fedora-*) app="linksy-$PREV-1.x86_64.rpm"; common="linksy-common-$PREV-1.x86_64.rpm" ;;
    opensuse) app="linksy-$PREV-1.suse.x86_64.rpm"; common="linksy-common-$PREV-1.suse.x86_64.rpm" ;;
    *) app="linksy_${PREV}_amd64.deb"; common="linksy-common_${PREV}_amd64.deb" ;;
  esac
  [ -e "dist/v$PREV/$common" ] && echo -n "/pkgs/v$PREV/$common "
  echo "/pkgs/v$PREV/$app"
}

# What each run checks, inside the container
read -r -d '' CHECKS <<'EOF'
missing=$(for b in linksyd linksy linksy-screen linksy-app linksy-qt; do p=$(command -v $b) && ldd $p | grep "not found" | sed "s|^|$b: |"; done)
[ -z "$missing" ] && echo "  libraries: ok" || echo "  libraries MISSING: $missing"
echo "  desktop entry: $(grep -h ^Exec= /usr/share/applications/mg.diary.linksy.Linksy.desktop)"
if command -v linksy-qt >/dev/null; then
  out=$(XDG_RUNTIME_DIR=/tmp LINKSY_SOCKET=/none QT_QPA_PLATFORM=offscreen QT_FORCE_STDERR_LOGGING=1 LC_ALL=C.UTF-8 timeout 8 linksy-qt 2>&1)
  if [ $? -eq 124 ] && ! echo "$out" | grep -q "didn't load"; then echo "  Qt window: loads"; else echo "  Qt window FAILED:"; echo "$out" | grep -v -e UTF-8 -e locale | head -8 | sed "s/^/    /"; fi
fi
[ -x /usr/bin/linksy-open ] && echo "  launcher: installed" || echo "  launcher MISSING"
EOF

run() {   # distro, label, shell commands
  echo "== $1: $2"
  podman run --rm -v "$PWD/dist":/pkgs:ro,z "$(image "$1")" bash -c "
    case $1 in debian-*|ubuntu-*) apt-get update -qq >/dev/null 2>&1;; opensuse) zypper -q refresh >/dev/null 2>&1;; esac
    export DEBIAN_FRONTEND=noninteractive
    ( $3 ) >/tmp/install.log 2>&1 || { echo '  install FAILED:'; tail -8 /tmp/install.log | sed 's/^/    /'; exit 0; }
    $CHECKS" 2>&1 | tee -a "$LOG"
}

# What must not install: the Qt app where Qt is too old. distro, command, label
refused() {
  echo "== $1: $3"
  podman run --rm -v "$PWD/dist":/pkgs:ro,z "$(image "$1")" bash -c "
    case $1 in debian-*|ubuntu-*) apt-get update -qq >/dev/null 2>&1;; opensuse) zypper -q refresh >/dev/null 2>&1;; esac
    export DEBIAN_FRONTEND=noninteractive
    if $2 >/tmp/install.log 2>&1; then echo '  installed anyway: FAILED'; else echo '  refused: ok'; fi" 2>&1 | tee -a "$LOG"
}

for d in "${ALL[@]}"; do
  in=$(install_cmd "$d")
  gtk_only=0
  [[ " ${GTK_ONLY[*]} " == *" $d "* ]] && gtk_only=1
  run "$d" "linksy (GTK)" "$in $(files "$d" gtk)"
  if [ $gtk_only = 1 ]; then
    refused "$d" "$in $(files "$d" qt)" "linksy-qt where Qt is too old"
  else
    run "$d" "linksy-qt (Qt)" "$in $(files "$d" qt)"
  fi
  run "$d" "one file at a time" "$in $(files "$d" gtk | cut -d' ' -f1) && $in $(files "$d" gtk | cut -d' ' -f2)"
  if [ $gtk_only = 0 ] && [ -n "$PREV" ] && [ -e "dist/$(old_files "$d" | awk '{print $NF}' | sed 's|/pkgs/||')" ]; then
    run "$d" "upgrade from $PREV" "$in $(old_files "$d") && $in $(files "$d" gtk)"
  fi
done

if grep -q -e FAILED -e MISSING "$LOG"; then
  echo "Some checks failed."
  exit 1
fi
