# Scene for scripts/qt-shot.sh: the other test device ("Studio Desktop") extends its desktop
# onto this one, with its test pattern as the second display; this side says yes. The app
# opens the display by itself. D is the scene folder, B the binaries.
(yes y | LINKSY_SOCKET="$D/a.sock" "$B/linksy" watch >/dev/null 2>&1) &
sleep 0.5
(LINKSY_SOCKET="$D/b.sock" "$B/linksy" extend "This Laptop" >/dev/null 2>&1) &
sleep 2
