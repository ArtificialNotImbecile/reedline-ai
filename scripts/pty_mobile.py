#!/usr/bin/env python3
"""Narrow canonical-terminal mobile checks; synthetic fixtures, no network."""
import fcntl
import json
import os
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time

binary = sys.argv[1] if len(sys.argv) > 1 else "target/debug/reedline-ai"


def run(policy):
    with tempfile.TemporaryDirectory() as directory:
        config = os.path.join(directory, "memory.json")
        marker = os.path.join(directory, "must-not-exist")
        with open(config, "w", encoding="utf-8") as stream:
            json.dump({"version": 1, "confirmed": {"Danger": {"path": "/synthetic/世界", "workflows": {"run": {"steps": [{"action": "change_directory"}, {"action": "command", "program": "rm", "args": ["-rf", "/synthetic/disposable"]}]} }}, "Probe": {"path": "/synthetic", "workflows": {"run": {"steps": [{"action": "command", "program": "touch", "args": [marker]}]}}}}}, stream)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 20, 0, 0))
        # Disable echo to inspect application rendering separately from typed input.
        attributes = termios.tcgetattr(slave)
        attributes[3] &= ~termios.ECHO
        termios.tcsetattr(slave, termios.TCSANOW, attributes)
        process = subprocess.Popen([binary, "--mode", "mobile", "--shell", "bash", "--risk-policy", policy, "--config", config, "--debounce-ms", "0"], stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
        os.close(slave)

        def read(seconds=0.2):
            output = bytearray()
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if select.select([master], [], [], max(0, deadline - time.monotonic()))[0]:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError:
                        break
                    if not chunk:
                        break
                    output.extend(chunk)
            text = output.decode("utf-8")
            assert "\x1b" not in text, repr(text)
            # Width includes multibyte text in later unit tests, ASCII here.
            for line in text.splitlines():
                assert len(line) <= 19, repr(line)
            return text.replace("\r", "").replace("\n", "")

        def send(text):
            os.write(master, text.encode("utf-8"))
            return read()

        try:
            assert "Mobile mode" in read()
            assert "No input" in send("\n")
            assert "No request sent" in send("suggest\n")
            assert "Candidates" in send("run Danger\n")
            preview = send("1\n")
            assert "Step 1: cd -- '/synthetic/世界'" in preview, preview
            assert "Step 2: 'rm' '-rf' '/synthetic/disposable'" in preview, preview
            assert "Accepted" not in send("confirm\n")
            accepted = send("accept\n")
            if policy == "confirm":
                assert "Type confirm" in accepted and "Accepted" not in accepted
                assert "Cancelled" in send("cancel\n")
                assert "Accepted" not in send("confirm\n")
                send("run Danger\n"); send("1\n"); send("accept\n")
                assert "Accepted" not in send("edit echo 世\nconfirm\n")
                send("suggest\n")
                assert "echo 世界" in send("1\n")
                accepted = send("accept\n")
                if "Type confirm" in accepted:
                    accepted = send("confirm\n")
                assert "Accepted (not executed):echo 世界" in accepted, accepted
                send("run Danger\n"); send("1\n"); send("accept\n")
                accepted = send("confirm\n")
            assert "Accepted (not executed):" in accepted, accepted
            send("run Probe\n"); send("1\n")
            accepted = send("accept\n")
            if "Type confirm" in accepted:
                accepted = send("confirm\n")
            assert "Accepted (not executed):" in accepted
            assert not os.path.exists(marker), "candidate was executed"
            # No key chord is required to quit; EOF also exits safely.
            if policy == "confirm":
                send("quit\n")
            else:
                os.write(master, b"\x04")
            process.wait(timeout=5)
            assert process.returncode == 0
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)


for risk_policy in ("confirm", "warn", "off"):
    run(risk_policy)
print("mobile narrow-PTY checks passed (confirm/warn/off; no execution)")
