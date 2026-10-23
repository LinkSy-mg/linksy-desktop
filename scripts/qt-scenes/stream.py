#!/usr/bin/env python3
"""Development aid for the Qt app's screen viewer: serve a moving test picture as an H.264
stream on a Unix socket, the way the service hands one to the app.

Usage: stream.py <socket> [WxH]
"""
import os
import socket
import subprocess
import sys

path = sys.argv[1]
size = sys.argv[2] if len(sys.argv) > 2 else "1280x720"
w, h = size.split("x")
if os.path.exists(path):
    os.unlink(path)
server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(path)
server.listen(1)
conn, _ = server.accept()
pipeline = (
    f"videotestsrc is-live=true pattern=ball motion=sweep background-color=0xff2563ff "
    f"! video/x-raw,width={w},height={h},framerate=30/1 ! videoconvert "
    f"! openh264enc ! h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream "
    f"! fdsink fd={conn.fileno()} sync=false"
)
subprocess.run(["gst-launch-1.0", "-q", *pipeline.split()], pass_fds=[conn.fileno()])
