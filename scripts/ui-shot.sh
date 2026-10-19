#!/usr/bin/env bash
# Development aid: render linksy-app on a headless Wayland compositor and save a PNG.
# Needs the linksy-dev toolbox with weston installed. Nothing appears on screen.
# Usage: scripts/ui-shot.sh <socket> <out.png> [delay-ms] [light|dark] [files to stage…]
set -euo pipefail
cd "$(dirname "$0")/.."
SOCK=$1 OUT=$2 DELAY=${3:-2500} SCHEME=${4:-dark}
shift $(( $# < 4 ? $# : 4 ))
FILES=""
for f in "$@"; do FILES+=" '$f'"; done
toolbox run -c linksy-dev bash -lc "
  export XDG_RUNTIME_DIR=\$(mktemp -d)
  weston --backend=headless --socket=linksy-shot --width=1200 --height=900 --idle-time=0 >/dev/null 2>&1 & WP=\$!
  for _ in \$(seq 50); do [ -S \$XDG_RUNTIME_DIR/linksy-shot ] && break; sleep 0.1; done
  WAYLAND_DISPLAY=linksy-shot GDK_BACKEND=wayland ADW_DEBUG_COLOR_SCHEME=prefer-$SCHEME \
    LINKSY_SOCKET='$SOCK' LINKSY_SCREENSHOT='$OUT' LINKSY_SCREENSHOT_DELAY=$DELAY \
    timeout 30 target/debug/linksy-app $FILES 2>&1 | grep -v -e '^$' -e 'Gdk-' || true
  kill \$WP 2>/dev/null; rm -rf \$XDG_RUNTIME_DIR"
