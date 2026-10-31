#!/usr/bin/env python3
"""POSIX streaming-preview PTY test, with a synthetic loopback SSE server.

Usage: python3 scripts/pty_streaming.py [path/to/reedline-ai]
No paid API. Exercises actual interactive rendering and acceptance boundaries.
"""
import fcntl
import os
import pty
import select
import struct
import subprocess
import sys
import termios
import time

import json
import socket
import threading

release = threading.Event()
listener = socket.socket()
listener.bind(("127.0.0.1", 0))
listener.listen()

def serve():
    connection, _ = listener.accept()
    with connection:
        data = b""
        while b"\r\n\r\n" not in data:
            data += connection.recv(4096)
        head, body = data.split(b"\r\n\r\n", 1)
        length = int(next(line.split(b":", 1)[1] for line in head.split(b"\r\n") if line.lower().startswith(b"content-length:")))
        while len(body) < length:
            body += connection.recv(4096)
        assert json.loads(body)["stream"] is True
        def event(text):
            return ("data: " + json.dumps({"choices": [{"index": 0, "delta": {"content": text}, "finish_reason": None}]}) + "\n\n").encode()
        first = event("str")
        last = event("eamed") + b'data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n'
        connection.sendall(f"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {len(first)+len(last)}\r\n\r\n".encode() + first)
        assert release.wait(6)
        connection.sendall(last)

thread = threading.Thread(target=serve, daemon=True)
thread.start()
binary = sys.argv[1] if len(sys.argv) > 1 else "target/debug/reedline-ai"
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
env = dict(os.environ, TERM="xterm-256color")
process = subprocess.Popen([binary, "--backend", "http", "--endpoint", f"http://127.0.0.1:{listener.getsockname()[1]}/v1/chat/completions", "--model", "synthetic", "--debounce-ms", "30"], stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
os.close(slave)
transcript = bytearray()
pending = bytearray()

def pump(seconds=0.2):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if select.select([master], [], [], max(0, deadline - time.monotonic()))[0]:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                return
            if not chunk:
                return
            transcript.extend(chunk)
            pending.extend(chunk)
            # Crossterm asks the terminal for its cursor position. A PTY alone
            # has no terminal emulator, so supply that minimal response.
            while b"\x1b[6n" in pending:
                index = pending.index(b"\x1b[6n")
                del pending[:index + 4]
                os.write(master, b"\x1b[1;1R")
            if len(pending) > 1000:
                del pending[:-10]


def send(data, wait=0.25):
    os.write(master, data)
    pump(wait)



try:
    pump(0.4)
    send(b"echo ")
    deadline = time.monotonic() + 4
    while b"str [receiving; not ready]" not in transcript and time.monotonic() < deadline:
        pump(0.05)
    assert b"str [receiving; not ready]" in transcript, bytes(transcript)
    send(b"\x06")  # Partial preview is not an acceptance token.
    send(b"\x1b[C\x1b[1;5C\x05")  # Right, word acceptance and End also cannot insert partial text.
    send(b"\t")  # Pending menu must not publish a partial candidate.
    send(b"\x1b")  # Close menu before final ordinary hint.
    release.set()
    deadline = time.monotonic() + 4
    while b"\x1b[90mstreamed\x1b[0m" not in transcript and time.monotonic() < deadline:
        pump(0.05)
    assert b"\x1b[90mstreamed\x1b[0m" in transcript, bytes(transcript)
    start = len(transcript)
    send(b"\x06\r")
    assert b"Accepted (not executed): echo streamed" in transcript[start:], bytes(transcript[start:])
    send(b":quit\r")
    process.wait(timeout=3)
    assert process.returncode == 0
    thread.join(timeout=2)
    print("PASS: incremental visible SSE preview, partial Ctrl-F blocked, complete Ctrl-F accepted without execution")
finally:
    release.set()
    if process.poll() is None:
        process.kill()
        process.wait()
    os.close(master)
    listener.close()
