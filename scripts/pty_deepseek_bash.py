#!/usr/bin/env python3
"""Explicit opt-in: three synthetic requests in real Bash, never execute results.

Requires DEEPSEEK_API_KEY or REEDLINE_AI_API_KEY in the environment.
This billable check is deliberately excluded from offline CI.
"""
import json
import os
from pathlib import Path
import shlex
import sys
import tempfile
import time

if "--allow-billable-api" not in sys.argv:
    raise SystemExit("Pass --allow-billable-api to run three official DeepSeek requests.")
sys.argv.remove("--allow-billable-api")
allow_proxy = "--allow-env-proxy" in sys.argv
if allow_proxy:
    sys.argv.remove("--allow-env-proxy")
from pty_bash_adapter import Shell, INVOKE, CONFIRM  # noqa: E402

key_name = next((name for name in ("DEEPSEEK_API_KEY", "REEDLINE_AI_API_KEY") if os.environ.get(name)), None)
if key_name is None:
    raise SystemExit("Missing environment API key.")

with tempfile.TemporaryDirectory(prefix="rai-deepseek-pty-") as temporary:
    folder = Path(temporary)
    (folder / "elsewhere").mkdir()
    shell = Shell(folder)
    try:
        shell.command("REEDLINE_AI_HISTORY=0; REEDLINE_AI_TIMEOUT_SECONDS=16; "
                      "REEDLINE_AI_ARGS=(--backend http --provider deepseek "
                      "--endpoint https://api.deepseek.com/chat/completions --model deepseek-flash "
                      "--timeout-seconds 15 --api-key-env " + shlex.quote(key_name) + (" --allow-env-proxy" if allow_proxy else "") + ")")
        for name, buffer, cursor in (("suffix", "git st", 6), ("fim", "git  --short", 4),
                                     ("unicode", "echo 世", 6)):
            shell.line(buffer)
            shell.send(b"\x1b[D" * (len(buffer) - cursor))
            start = time.monotonic()
            shell.send(INVOKE)
            while True:
                shell.send(INVOKE)
                state = shell.snapshot()
                if state[2] != "1" or time.monotonic() - start > 17:
                    break
                shell.pump(0.1)
            if state[0] != buffer or state[3] != "1":
                raise SystemExit("DeepSeek PTY failed before preview; stopped without retry (details redacted).")
            shell.send(CONFIRM)
            shell.send(INVOKE)
            result, point, *_ = shell.snapshot()
            if not (len(result) > len(buffer) and result.startswith(buffer[:cursor])
                    and result.endswith(buffer[cursor:]) and point == cursor + len(result) - len(buffer)):
                raise SystemExit("DeepSeek PTY insertion validation failed; stopped without retry.")
            if os.environ[key_name] in result:
                raise SystemExit("Unexpected credential-like result; output withheld.")
            print(json.dumps({"case": name, "seconds": round(time.monotonic() - start, 2),
                              "result": result, "suffix_preserved": True, "executed": False}), flush=True)
    finally:
        shell.close()
