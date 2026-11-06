#!/usr/bin/env bash
# Development aid: render linksy-app on a headless Wayland compositor and save a PNG.
# Needs the linksy-dev toolbox with weston installed. Nothing appears on screen.
# Usage: scripts/ui-shot.sh <socket> <out.png> [delay-ms] [light|dark] [files to stage…]
# LINKSY_SCREENSHOT_ACTION=win.preferences opens a dialog first; LINKSY_SCREENSHOT_SIZE=400x700 resizes.
# LINKSY_SCREENSHOT_TAB=deck opens the deck editor; LINKSY_SCREENSHOT_DECK_THEME=dots, _PICK=2 and
# _LANDSCAPE=1 show a theme, pick the second tile and turn the phone; _DRAG=move,80,60 or
# resize,70,70 shows the picked tile mid-drag. _DETAILS=1 opens the
# details panel of a narrow window. LINKSY_SCREENSHOT_SCALE=2 renders at 2x;
# LINKSY_SCREENSHOT_HOME=<dir> runs the app with that home (paths under it show as ~/…).
set -euo pipefail
cd "$(dirname "$0")/.."
SOCK=$1 OUT=$2 DELAY=${3:-2500} SCHEME=${4:-dark}
shift $(( $# < 4 ? $# : 4 ))
FILES=""
for f in "$@"; do FILES+=" '$f'"; done
# A screen big enough for LINKSY_SCREENSHOT_SIZE. Nobody will ever see it, but it has to fit.
SW=1200 SH=900
if [[ ${LINKSY_SCREENSHOT_SIZE:-} =~ ^([0-9]+)x([0-9]+)$ ]]; then
  SW=$(( BASH_REMATCH[1] > SW ? BASH_REMATCH[1] : SW )) SH=$(( BASH_REMATCH[2] > SH ? BASH_REMATCH[2] : SH ))
fi
SC=${LINKSY_SCREENSHOT_SCALE:-1}
SW=$(( SW * SC )) SH=$(( SH * SC ))
toolbox run -c linksy-dev bash -lc "
  export XDG_RUNTIME_DIR=\$(mktemp -d)
  weston --backend=headless --socket=linksy-shot --width=$SW --height=$SH --scale=$SC --idle-time=0 >/dev/null 2>&1 & WP=\$!
  for _ in \$(seq 50); do [ -S \$XDG_RUNTIME_DIR/linksy-shot ] && break; sleep 0.1; done
  HOME=${LINKSY_SCREENSHOT_HOME:-\$HOME} WAYLAND_DISPLAY=linksy-shot LANGUAGE=${LANGUAGE:-} GDK_BACKEND=wayland ADW_DEBUG_COLOR_SCHEME=prefer-$SCHEME \
    LINKSY_SOCKET='$SOCK' LINKSY_SCREENSHOT='$OUT' LINKSY_SCREENSHOT_DELAY=$DELAY \
    LINKSY_SCREENSHOT_ACTION='${LINKSY_SCREENSHOT_ACTION:-}' LINKSY_SCREENSHOT_SIZE='${LINKSY_SCREENSHOT_SIZE:-}' LINKSY_SCREENSHOT_NEWEST='${LINKSY_SCREENSHOT_NEWEST:-}' LINKSY_SCREENSHOT_THREAD='${LINKSY_SCREENSHOT_THREAD:-}' LINKSY_SCREENSHOT_TAB='${LINKSY_SCREENSHOT_TAB:-}' LINKSY_SCREENSHOT_DECK_THEME='${LINKSY_SCREENSHOT_DECK_THEME:-}' LINKSY_SCREENSHOT_DECK_PICK='${LINKSY_SCREENSHOT_DECK_PICK:-}' LINKSY_SCREENSHOT_DECK_LANDSCAPE='${LINKSY_SCREENSHOT_DECK_LANDSCAPE:-}' LINKSY_SCREENSHOT_DECK_DRAG='${LINKSY_SCREENSHOT_DECK_DRAG:-}' LINKSY_SCREENSHOT_DECK_ON='${LINKSY_SCREENSHOT_DECK_ON:-}' LINKSY_SCREENSHOT_DECK_SAVE='${LINKSY_SCREENSHOT_DECK_SAVE:-}' LINKSY_SCREENSHOT_DECK_BACK='${LINKSY_SCREENSHOT_DECK_BACK:-}' LINKSY_SCREENSHOT_DECK_DETAILS='${LINKSY_SCREENSHOT_DECK_DETAILS:-}' LINKSY_SCREENSHOT_SCROLL='${LINKSY_SCREENSHOT_SCROLL:-}' LINKSY_UPGRADE_FROM='${LINKSY_UPGRADE_FROM:-}' \
    timeout 30 ${LINKSY_APP:-target/debug/linksy-app} $FILES 2>&1 | grep -v -e '^$' -e 'Gdk-' || true
  kill \$WP 2>/dev/null; rm -rf \$XDG_RUNTIME_DIR"
