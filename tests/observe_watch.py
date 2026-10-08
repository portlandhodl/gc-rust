#!/usr/bin/env python3
"""Listen on Dolphin's MemoryWatcher socket for tag/value messages.

Usage: observe_watch.py <socket-path> <timeout_s>
Prints every "tag:value" change observed, one per line.
"""
import os
import socket
import sys
import time

if len(sys.argv) < 3:
    print("usage: observe_watch.py <sock> <secs>")
    sys.exit(1)

path, secs = sys.argv[1], int(sys.argv[2])
try:
    os.unlink(path)
except FileNotFoundError:
    pass

srv = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
srv.bind(path)  # we are the listener; Dolphin's sendto targets this path
srv.settimeout(0.25)

deadline = time.time() + secs
seen = {}
while time.time() < deadline:
    try:
        data, _ = srv.recvfrom(65536)
    except socket.timeout:
        continue
    if not data:
        continue
    text = data.decode("ascii", "replace")
    toks = [t for t in text.split("\n") if t]
    i = 0
    while i + 1 < len(toks):
        seen[toks[i]] = toks[i + 1]
        i += 2

for k, v in seen.items():
    print(f"{k}:{v}")
