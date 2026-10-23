#!/usr/bin/env bash
# Development aid: render the Qt app (linksy-qt) against two local services paired with each
# other (ui-scene.sh), off screen, and save a PNG. Runs in the linksy-qt-dev container
# (packaging/qt/Containerfile), so nothing touches this computer's own Linksy.
# Usage: scripts/qt-shot.sh <out.png> [delay-ms] [WxH]
# LINKSY_SCREENSHOT_OPEN=invite|prefs opens that screen first. LINKSY_QT_SCENE=<script> runs a
# script once the scene is up, with D set to the scene folder (a.sock is the app's service).
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=$1 DELAY=${2:-3000} SIZE=${3:-1280x820}
podman run --rm -v "$PWD":/src:z -v linksy-cargo:/cargo -w /src linksy-qt-dev bash -c "
  export XDG_RUNTIME_DIR=\$(mktemp -d) HOME=\$(mktemp -d)
  D=\$(mktemp -d)
  LINKSY_BIN=target/qt/debug scripts/ui-scene.sh \$D >/dev/null 2>&1 || true
  [ -n '${LINKSY_QT_SCENE:-}' ] && D=\$D B=target/qt/debug bash '${LINKSY_QT_SCENE:-}'
  QT_FORCE_STDERR_LOGGING=1 LC_ALL=C.UTF-8 LINKSY_SOCKET=\$D/a.sock QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software \
    QT_QUICK_CONTROLS_STYLE=org.kde.desktop LINKSY_SCREENSHOT=/src/$OUT LINKSY_SCREENSHOT_DELAY=$DELAY \
    LINKSY_SCREENSHOT_SIZE=$SIZE LINKSY_SCREENSHOT_OPEN=${LINKSY_SCREENSHOT_OPEN:-} LANGUAGE=${LANGUAGE:-} \
    timeout 40 target/qt/debug/linksy-qt 2>&1 | grep -v -e '^\$' -e 'UTF-8' -e 'locale' | head -40"
# The container goes away at the end, and the test services with it.
