#!/usr/bin/env bash
# Development aid: start two local services ("This Laptop" and "Studio Desktop"), pair them,
# and leave them running so the app can be pointed at one with LINKSY_SOCKET.
# Usage: scripts/ui-scene.sh <dir>     (stop with: pkill -f "linksyd --name")
set -euo pipefail
cd "$(dirname "$0")/.."
D=${1:?scene dir}
rm -rf "$D"; mkdir -p "$D"
B=target/debug
$B/linksyd --name "This Laptop" --port 47230 --data-dir "$D/a" --download-dir "$D/a-dl" --socket "$D/a.sock" --no-notifications >"$D/a.log" 2>&1 &
$B/linksyd --name "Studio Desktop" --port 47231 --data-dir "$D/b" --download-dir "$D/b-dl" --socket "$D/b.sock" --no-notifications >"$D/b.log" 2>&1 &
sleep 2
(sleep 1; echo y) | LINKSY_SOCKET="$D/b.sock" timeout 6 $B/linksy watch >/dev/null 2>&1 &
sleep 0.3
echo y | LINKSY_SOCKET="$D/a.sock" timeout 10 $B/linksy pair "Studio Desktop"
