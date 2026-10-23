# Scene for scripts/qt-shot.sh: a test picture streamed as a device's screen would be. Use
# with LINKSY_SCREENSHOT_OPEN=screen LINKSY_TEST_STREAM=/tmp/linksy-test.sock.
python3 scripts/qt-scenes/stream.py /tmp/linksy-test.sock 1280x720 >/dev/null 2>&1 &
sleep 1
