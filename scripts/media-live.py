#!/usr/bin/env python3
"""Does the phone's microphone really deliver on this system's PipeWire? Starts
`linksy-screen mic` as the service does, feeds it a tone for 6 seconds as a phone would, and
records the "linksy-mic" source apps see. PipeWire 1.6 delivered 0.7 s and then nothing
(desktop#89), which only a run like this shows. Needs a running PipeWire with its PulseAudio
layer (pactl, parec). Prints one line; exits 1 when the microphone is silent or absent.

  scripts/media-live.py [path to linksy-screen]
"""
import math
import os
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time

HELPER = sys.argv[1] if len(sys.argv) > 1 else "linksy-screen"
AMPLITUDE = 2000
SECONDS = 6


def main():
    path = os.path.join(tempfile.mkdtemp(prefix="lk"), "mic.sock")
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(path)
    listener.listen(1)
    listener.settimeout(10)
    helper = subprocess.Popen([HELPER, "mic", "--socket", path, "--name", "Linksy Microphone (test)"], stderr=subprocess.PIPE, text=True)
    try:
        conn, _ = listener.accept()
    except socket.timeout:
        helper.kill()
        return f"the helper never connected: {helper.stderr.read().strip()}"
    conn.settimeout(2)
    time.sleep(1.5)
    sources = subprocess.run(["pactl", "list", "short", "sources"], capture_output=True, text=True).stdout
    if "\tlinksy-mic\t" not in sources:
        helper.kill()
        return f"no linksy-mic source ({helper.stderr.read().strip() or 'the helper said nothing'})"
    recorder = subprocess.Popen(["parec", "-d", "linksy-mic", "--format=s16le", "--rate=48000", "--channels=1", "--raw"], stdout=subprocess.PIPE)
    start, sent = time.time(), 0
    while time.time() - start < SECONDS:
        chunk = b"".join(struct.pack("<h", int(AMPLITUDE * math.sin(2 * math.pi * 440 * (sent + i) / 48000))) for i in range(480))
        try:
            conn.sendall(chunk)
        except (BrokenPipeError, socket.timeout):
            break
        sent += 480
        wait = start + sent / 48000 - time.time()
        if wait > 0:
            time.sleep(wait)
    recorder.terminate()
    raw = recorder.stdout.read()
    conn.close()
    helper.send_signal(signal.SIGTERM)
    try:
        helper.wait(timeout=5)
    except subprocess.TimeoutExpired:
        helper.kill()
    samples = struct.unpack(f"<{len(raw) // 2}h", raw[: len(raw) // 2 * 2])
    # The last two seconds: a stream that stalled after a moment is silent there
    late = samples[-96000:] if len(samples) > 3 * 48000 else ()
    peak = max((abs(s) for s in late), default=0)
    left = subprocess.run(["pactl", "list", "short", "modules"], capture_output=True, text=True).stdout.count("linksy-mic")
    if len(samples) < 3 * 48000:
        return f"only {len(samples) / 48000:.1f} s of audio came in {SECONDS} s"
    if peak < AMPLITUDE:
        return f"silent after a moment (peak {peak} in the last two seconds)"
    if left:
        return f"{left} module(s) left behind"
    print(f"microphone: delivers for {SECONDS} s (peak {peak}, fed {AMPLITUDE})")
    return None


problem = main()
if problem:
    print(f"microphone: {problem}")
    sys.exit(1)
