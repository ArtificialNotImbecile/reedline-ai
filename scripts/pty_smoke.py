#!/usr/bin/env python3
"""POSIX PTY smoke test. No third-party modules or network services required.

Usage: python3 scripts/pty_smoke.py [path/to/reedline-ai]
Exercises the actual interactive binary, not a shell that executes its output.
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

binary = sys.argv[1] if len(sys.argv) > 1 else "target/debug/reedline-ai"
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
env = dict(os.environ, TERM="xterm-256color")
process = subprocess.Popen([binary, "--debounce-ms", "30"], stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
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
    pump(0.5)
    send(b"git st")
    send(b"\x06")  # Ctrl-F accepts suffix hint.
    send(b"\r")
    assert b"Accepted (not executed): git status" in transcript, bytes(transcript)
    send(b"git  --short")
    send(b"\x01")  # Ctrl-A, then 4 right-arrow motions to byte cursor 4.
    send(b"\x1b[C" * 4)
    send(b"\t")
    send(b"\r")  # Accept menu, not submit.
    send(b"\r")
    assert b"Accepted (not executed): git status --short" in transcript, bytes(transcript)
    send("echo 界".encode())
    send(b"\x1b[D")  # Move before the Unicode suffix.
    send(b"\t")
    send(b"\r")
    send(b"\r")
    assert "Accepted (not executed): echo 世界".encode() in transcript, bytes(transcript)
    # Regression: edit + accept within one Reedline input batch must not use
    # the previous render's hint. Repeated hint keys must be one-shot.
    for hint_key in [b"\x06", b"\x1b[C", b"\x05", b"\x1b[1;5C"]:
        send(b"git st")
        start = len(transcript)
        send(b"a" + hint_key + b"\r")
        assert b"Accepted (not executed): git sta" in transcript[start:], bytes(transcript[start:])
        assert b"git staatus" not in transcript[start:], bytes(transcript[start:])
        send(b"git st")
        start = len(transcript)
        send(hint_key + hint_key + b"\r")
        assert b"Accepted (not executed): git status" in transcript[start:], bytes(transcript[start:])
        assert b"git statusatus" not in transcript[start:], bytes(transcript[start:])
    # Earlier acceptance must not be revoked by later typing in the same batch.
    send(b"git st")
    start = len(transcript)
    send(b"\x06 --short\r")
    assert b"Accepted (not executed): git status --short" in transcript[start:], bytes(transcript[start:])
    # Regression: reject terminal controls before the default ANSI renderer
    # sees a bracketed paste, not merely when submitting/asking the model.
    start = len(transcript)
    attack = b"echo \x1b]0;INJECTED_TITLE\x07"
    send(b"\x1b[200~" + attack + b"\x1b[201~")
    assert b"INJECTED_TITLE" not in transcript[start:], bytes(transcript[start:])
    send(b"echo safe\r")
    assert b"Accepted (not executed): echo safe" in transcript[start:], bytes(transcript[start:])
    send(b"discard this")
    send(b"\x03")  # Ctrl-C clears rather than exits.
    send(b":quit\r")
    process.wait(timeout=3)
    assert process.returncode == 0, bytes(transcript)
    print("PASS: suffix/FIM/Unicode, batched stale-hint guards, unsafe paste rejection, cancel, exit")
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    os.close(master)
