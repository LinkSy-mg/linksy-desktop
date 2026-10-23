# Scene for scripts/qt-shot.sh: files and the clipboard already shared, and one more file
# waiting to be accepted. D is the scene folder, B the binaries.
L() { LINKSY_SOCKET="$D/$1.sock" "$B/linksy" "${@:2}" >/dev/null 2>&1; }
seq 1 200000 > "$D/Quarterly report.csv"
echo "Pick up bread" > "$D/notes.txt"
L a settings auto-accept on
L b send "This Laptop" "$D/Quarterly report.csv" "$D/notes.txt"
L b clip "This Laptop" "Meeting moved to 3 pm, room 2B"
L a settings auto-accept off
# Offered once the app is up, so its prompt shows
(sleep 2; L b send "This Laptop" "$D/notes.txt") &
