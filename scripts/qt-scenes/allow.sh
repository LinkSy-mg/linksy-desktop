# Scene for scripts/qt-shot.sh: the other test device says yes to everything it's asked
# (a screen to watch, control). D is the scene folder, B the binaries.
(yes y | LINKSY_SOCKET="$D/b.sock" "$B/linksy" watch >/dev/null 2>&1) &
