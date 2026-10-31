#!/usr/bin/env python3
"""User journey through the actual launcher, saved history, insertion and undo.

Uses a controlling PTY and synthetic HOME. No network or suggested execution.
"""
import fcntl
import os
from pathlib import Path
import pty
import select
import shlex
import signal
import struct
import sys
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[1]
LAUNCHER = Path(os.environ.get("RAI_BASH_LAUNCHER", ROOT / "scripts/try-bash.sh")).resolve()
BINARY = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "target/debug/reedline-ai").resolve()

with tempfile.TemporaryDirectory(prefix="rai-startup-") as temporary:
    home = Path(temporary).resolve()
    history = home / ".bash_history"
    original = "echo cold_start_from_saved_history\necho another_saved_command\n"
    history.write_text(original)
    snapshot = home / "snapshot"
    env = dict(os.environ, HOME=str(home), REEDLINE_AI_BIN=str(BINARY),
               REEDLINE_AI_HISTORY="1", REEDLINE_AI_HISTORY_FILE=str(history),
               REEDLINE_AI_BASH=os.environ.get("RAI_BASH", "bash"),
               INPUTRC="/dev/null", TERM="xterm-256color",
               LC_ALL="en_US.UTF-8" if sys.platform == "darwin" else "C.UTF-8")
    for name in ("BASH_ENV", "ENV", "DEEPSEEK_API_KEY", "REEDLINE_AI_API_KEY"):
        env.pop(name, None)
    pid, master = pty.fork()
    if pid == 0:
        os.chdir(ROOT)
        os.execve("/bin/bash", ["bash", str(LAUNCHER)], env)
    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    transcript = bytearray()

    def pump(seconds=.05):
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

    def send(data):
        os.write(master, data.encode() if isinstance(data, str) else data)
        pump()

    def state():
        snapshot.unlink(missing_ok=True)
        send(b"\x18\x14")
        deadline = time.monotonic() + 2
        while not snapshot.exists() and time.monotonic() < deadline:
            pump()
        assert snapshot.exists(), "editor did not answer while work was pending"
        return snapshot.read_bytes().split(b"\0")[:2]

    exited = False
    try:
        deadline = time.monotonic() + 5
        while b"Ctrl-x Ctrl-g: request" not in transcript and time.monotonic() < deadline:
            pump()
        assert b"Ctrl-x Ctrl-g: request" in transcript, "startup help missing"
        send("__startup_snapshot() { printf '%s\\0%s\\0' \"$READLINE_LINE\" \"$__rai_previewed\" > "
             + shlex.quote(str(snapshot)) + "; }; bind -x "
             + shlex.quote('"\\C-x\\C-t":__startup_snapshot') + "\r")
        send("echo cold")
        start = time.monotonic()
        send(b"\x18\x07")
        deadline = start + 5
        while True:
            send(b"\x18\x07")
            line, preview = state()
            assert line == b"echo cold", "review inserted without acceptance"
            if preview == b"1":
                break
            assert time.monotonic() < deadline, "saved history did not produce a preview"
        latency = time.monotonic() - start
        send(b"\x18\x07")
        assert state()[0] == b"echo cold_start_from_saved_history"
        send(b"\x1f")  # Native Readline undo, one edit.
        assert state()[0] == b"echo cold"
        send(b"\x18\x0b")
        assert state() == [b"echo cold", b"0"]
        send(b"\x01\x0bexit\r")  # Clear generated text before fixed exit.
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            done, status = os.waitpid(pid, os.WNOHANG)
            if done:
                exited = True
                assert status == 0
                break
            pump()
        assert exited, "launcher did not return after exit"
        assert history.read_text() == original, "trial rewrote saved history"
        assert not any((home / name).exists() for name in (".bashrc", ".bash_profile", ".inputrc"))
        print(f"Startup E2E PASS: help, saved-history preview ({latency:.2f}s including key polling), "
              "accept, one-step undo, cancel, exit; history/profiles unchanged; zero model calls.")
    finally:
        if not exited:
            os.close(master)
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            os.waitpid(pid, 0)
        else:
            os.close(master)
