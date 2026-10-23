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
import tempfile
import json

binary = sys.argv[1] if len(sys.argv) > 1 else "target/debug/reedline-ai"
fixture = tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False)
json.dump({"version":1,"confirmed":{"Danger":{"path":"/synthetic","workflows":{"run":{"steps":[{"action":"command","program":"rm","args":["-rf","/synthetic/disposable"]}]}}}}}, fixture)
fixture.close()
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
env = dict(os.environ, TERM="xterm-256color")
process = subprocess.Popen([binary, "--debounce-ms", "0", "--shell", "bash", "--risk-policy", "confirm", "--config", fixture.name], stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
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
    # A risky replacement is visible, but ordinary Enter must not insert it.
    send(b"run Danger\t", 0.3)
    assert b"F2" in transcript
    start = len(transcript)
    send(b"\r\r")
    assert b"Accepted (not executed): run Danger" in transcript[start:], bytes(transcript[start:])
    assert b"Accepted (not executed): 'rm'" not in transcript[start:]
    # Acknowledge the displayed candidate, then native Enter inserts; second submits.
    send(b"run Danger\t", 0.3)
    send(b"\x1bOQ", 0.2)  # xterm F2
    start = len(transcript)
    send(b"\r\r", 0.3)
    assert b"Accepted (not executed): 'rm' '-rf' '/synthetic/disposable'" in transcript[start:], bytes(transcript[start:])
    # Cancellation/reopening must not retain acknowledgment.
    send(b"run Danger\t", 0.3)
    send(b"\x1bOQ")
    send(b"\x1b")
    send(b"\t")
    start = len(transcript)
    send(b"\r\r")
    assert b"Accepted (not executed): run Danger" in transcript[start:], bytes(transcript[start:])
    # Input ordered batch: acknowledgment, edit, Enter cannot apply stale preview.
    send(b"run Danger\t", 0.3)
    start = len(transcript)
    send(b"\x1bOQx\r\r", 0.3)
    assert b"Accepted (not executed): 'rm'" not in transcript[start:], bytes(transcript[start:])
    send(b"\x03")
    # An acknowledgment arriving after Enter in the same batch is too late.
    send(b"run Danger\t", 0.3)
    start = len(transcript)
    send(b"\r\x1bOQ\r", 0.3)
    assert b"Accepted (not executed): run Danger" in transcript[start:], bytes(transcript[start:])
    # Generated FIM with incomplete git review uses the same acknowledgment gate.
    send(b"git  --short")
    send(b"\x01" + b"\x1b[C" * 4)
    send(b"\t", 0.3)
    send(b"\x1bOQ")
    start = len(transcript)
    send(b"\r\r", 0.3)
    assert b"Accepted (not executed): git status --short" in transcript[start:], bytes(transcript[start:])
    send(b":quit\r")
    process.wait(timeout=3)
    print("PTY risk PASS: additional acceptance, cancellation, batched edit, whole-result FIM review")
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    os.close(master)
    os.unlink(fixture.name)
