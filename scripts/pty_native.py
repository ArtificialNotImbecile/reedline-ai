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
process = subprocess.Popen([binary, "--debounce-ms", "10000"], stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
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
    def accept(text):
        start = len(transcript)
        send(text.encode() + b"\r", 0.25)
        assert ("Accepted (not executed): " + text).encode() in transcript[start:], bytes(transcript[start:])
    accept("deploy service --dry-run")
    # A local hint arrives far before the 10-second AI debounce.
    send(b"deploy ", 0.05)
    start = len(transcript)
    send(b"\x1bf\r", 0.25)  # Native Alt-F: only next word.
    assert b"Accepted (not executed): deploy service\r" in transcript[start:], bytes(transcript[start:])
    accept("native history needle")
    start = len(transcript)
    send(b"\x1b[A\r", 0.25)
    assert b"Accepted (not executed): native history needle" in transcript[start:], bytes(transcript[start:])
    # Up then Down restores the unsubmitted draft.
    send(b"draft text")
    send(b"\x10\x0e")
    start = len(transcript)
    send(b"\r", 0.25)
    assert b"Accepted (not executed): draft text" in transcript[start:], bytes(transcript[start:])
    # Native reverse search remains active, and the prompt includes its term.
    send(b"\x12needle", 0.25)
    assert b"reverse-search" in transcript, bytes(transcript)
    start = len(transcript)
    send(b"\r", 0.25)
    if b"Accepted (not executed): native history needle" not in transcript[start:]:
        send(b"\r", 0.25)
    assert b"Accepted (not executed): native history needle" in transcript[start:], bytes(transcript[start:])
    # Native Ctrl-F exits history search without permanently disabling hints.
    accept("git status")
    send(b"\x12git", 0.25)
    send(b"\x06", 0.25)
    send(b"\x7f" * 4, 0.25)
    start = len(transcript)
    send(b"\x06\r", 0.25)
    assert b"Accepted (not executed): git status\r" in transcript[start:], bytes(transcript[start:])
    # Ctrl-D on a nonempty menu must not let Ctrl-F bypass native menu guards.
    accept("git status --short")
    send(b"git st", 0.25)
    send(b"\t", 0.25)
    send(b"\x04", 0.25)
    send(b"\x06", 0.25)
    send(b"\x1b", 0.25)
    start = len(transcript)
    send(b"\r", 0.25)
    assert b"Accepted (not executed): git st\r" in transcript[start:], bytes(transcript[start:])
    # Undo removes only the accepted native hint, not the typed prefix.
    send(b"deploy ", 0.25)
    send(b"\x06", 0.25)
    send(b"\x1a", 0.25)
    start = len(transcript)
    send(b"\r", 0.25)
    assert b"Accepted (not executed): deploy \r" in transcript[start:], bytes(transcript[start:])
    # Local middle insertion uses the native menu and keeps the suffix.
    send(b"deploy  --dry-run")
    send(b"\x01" + b"\x1b[C" * 7)
    send(b"\t", 0.05)
    start = len(transcript)
    send(b"\r\r", 0.25)
    assert b"Accepted (not executed): deploy service --dry-run" in transcript[start:], bytes(transcript[start:])
    # Selection must retain native guard: hint key collapses/moves, not inserts.
    accept("selection target")
    send(b"selection ", 0.25)
    send(b"\x01\x1b[1;2F", 0.25)
    start = len(transcript)
    send(b"\x06\r", 0.25)
    assert b"Accepted (not executed): selection \r" in transcript[start:], bytes(transcript[start:])
    # Native hints share the same one-shot, input-ordered stale guards.
    for key in [b"\x06", b"\x1b[C", b"\x05"]:
        accept("native batch target")
        send(b"native batch ", 0.25)
        start = len(transcript)
        send(key + key + b"\r", 0.25)
        assert b"Accepted (not executed): native batch target\r" in transcript[start:], bytes(transcript[start:])
        send(b"native batch ", 0.25)
        start = len(transcript)
        send(b"x" + key + b"\r", 0.25)
        assert b"Accepted (not executed): native batch x\r" in transcript[start:], bytes(transcript[start:])
    # Whole/word hint keys mixed in one batch consume exactly one shown token.
    accept("mixed words rest")
    send(b"mixed ", 0.25)
    start = len(transcript)
    send(b"\x1bf\x06\r", 0.25)
    assert b"Accepted (not executed): mixed words\r" in transcript[start:], bytes(transcript[start:])
    accept("mixed words rest")
    send(b"mixed ", 0.25)
    start = len(transcript)
    send(b"\x06\x1bf\r", 0.25)
    assert b"Accepted (not executed): mixed words rest\r" in transcript[start:], bytes(transcript[start:])
    # Preview and clear are local controls, never submitted/stored as commands.
    send(b":history-preview deploy\r", 0.25)
    assert b'"network_request":false' in transcript, bytes(transcript)
    send(b":history-clear\r", 0.25)
    start = len(transcript)
    send(b"\x1b[A\r", 0.25)
    assert b"Accepted (not executed)" not in transcript[start:], bytes(transcript[start:])
    send(b":quit\r")
    process.wait(timeout=3)
    assert process.returncode == 0, bytes(transcript)
    print("PASS: instant native history, word hints, Up/Down, Ctrl-R, undo, local FIM, preview/clear")
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
    os.close(master)
