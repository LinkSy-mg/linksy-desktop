#!/usr/bin/env bash
# Runs scripts/media-live.py on a PipeWire of its own: a private D-Bus session, PipeWire,
# WirePlumber and the PulseAudio layer, started here and gone at the end. For containers
# (scripts/test-packages.sh) and for trying without touching the desktop's own sound.
# Usage: scripts/media-live.sh [path to linksy-screen]
set -uo pipefail
HERE=$(dirname "$(readlink -f "$0")")
export XDG_RUNTIME_DIR=$(mktemp -d)
exec dbus-run-session -- bash -c '
  pipewire >/dev/null 2>&1 &
  sleep 1
  wireplumber >/dev/null 2>&1 &
  pipewire-pulse >/dev/null 2>&1 &
  for _ in $(seq 50); do pactl info >/dev/null 2>&1 && break; sleep 0.2; done
  echo "PipeWire $(pipewire --version 2>/dev/null | sed -n "s/.*libpipewire //p" | head -1)"
  python3 "$0/media-live.py" "$1"
  code=$?
  kill $(jobs -p) 2>/dev/null
  exit $code
' "$HERE" "${1:-linksy-screen}"
