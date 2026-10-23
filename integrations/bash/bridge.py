#!/usr/bin/env python3
"""Private Bash transport: bounded subprocess, strict JSON, NUL-delimited data.

No shell=True, eval, terminal escapes, history reads, files, or model execution.
Input is five NUL-terminated UTF-8 fields on stdin, never command-line arguments.
"""
from __future__ import annotations

import json
import math
import os
import select
import selectors
import signal
import subprocess
import sys
import time
import unicodedata

MAX_INPUT = 32 * 1024
MAX_BUFFER = 8192
MAX_INSERTION = 1024
MAX_OUTPUT = 128 * 1024
MAX_PREVIEW = 16 * 1024
_CANCELLED = False


def cancel_signal(_signum: int, _frame: object) -> None:
    # Do not throw from Popen setup: a signal between fork and assignment must
    # not bypass process-group cleanup. The bounded loop checks this flag.
    global _CANCELLED
    _CANCELLED = True


class Failure(Exception):
    pass


def emit(status: str, result: str = "", cursor: int = 0, risk: bool = False, preview: str = "") -> None:
    frame = "\0".join([status, result, str(cursor), str(int(risk)), preview]) + "\0"
    try:
        sys.stdout.buffer.write(frame.encode("utf-8"))
        sys.stdout.buffer.flush()
    except (BrokenPipeError, OSError):
        # Cancelled/closed shell; no result has any effect.
        pass


def byte_cursor(text: str, cursor: object) -> bool:
    if type(cursor) is not int or not 0 <= cursor <= len(text.encode("utf-8")):
        return False
    try:
        text.encode("utf-8")[:cursor].decode("utf-8")
        return True
    except UnicodeError:
        return False


def single_line(text: str) -> bool:
    # Block control, surrogate and invisible formatting codepoints even in data
    # inserted into Readline. Preview also uses ASCII JSON escapes.
    return all(unicodedata.category(char) not in {"Cc", "Cs", "Cf", "Zl", "Zp"} for char in text)


def read_request() -> dict:
    raw = sys.stdin.buffer.read(MAX_INPUT + 1)
    if len(raw) > MAX_INPUT:
        raise Failure("invalid-input")
    try:
        fields = raw.decode("utf-8").split("\0")
        if len(fields) != 6 or fields[-1] != "":
            raise ValueError("frame")
        request_id, version, cwd, buffer, point, _ = fields
        cursor = int(point)
        if not buffer.strip():
            raise Failure("empty")
        if len(buffer.encode("utf-8")) > MAX_BUFFER or not single_line(buffer) or not byte_cursor(buffer, cursor):
            raise ValueError("line")
        return {"protocol": 1, "request_id": request_id, "shell": {"kind": "bash", "version": version},
                "cwd": cwd, "buffer": buffer, "cursor": cursor}
    except (ValueError, UnicodeError) as error:
        raise Failure("invalid-input") from error


def no_duplicates(pairs: list) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate key")
        result[key] = value
    return result


def validate(raw: bytes, request: dict) -> tuple[str, int, bool, str]:
    try:
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=no_duplicates)
        if not isinstance(value, dict) or type(value.get("protocol")) is not int or value["protocol"] != 1:
            raise ValueError("protocol")
        for key in ("request_id", "buffer", "cursor"):
            if type(value.get(key)) is not type(request[key]) or value[key] != request[key]:
                raise ValueError("identity")
        insertion, result, cursor = value["insertion"], value["result"], value["result_cursor"]
        if not isinstance(insertion, str) or not isinstance(result, str) or not single_line(result):
            raise ValueError("result")
        if len(insertion.encode("utf-8")) > MAX_INSERTION or "```" in insertion:
            raise ValueError("insertion limit or code fence")
        original = request["buffer"].encode("utf-8")
        point = request["cursor"]
        expected = original[:point] + insertion.encode("utf-8") + original[point:]
        if result.encode("utf-8") != expected or not byte_cursor(result, cursor):
            raise ValueError("insertion")
        if cursor != point + len(insertion.encode("utf-8")):
            raise ValueError("cursor")
        if not insertion:
            raise Failure("empty")
        review = value["risk_review"]
        required = value["additional_acceptance_required"]
        if not isinstance(review, dict) or type(required) is not bool:
            raise ValueError("review")
        warnings, incomplete = review["warnings"], review["incomplete"]
        if not isinstance(warnings, list) or any(not isinstance(w, str) for w in warnings) or type(incomplete) is not bool:
            raise ValueError("review")
        preview = json.dumps(result, ensure_ascii=True)
        if len(preview) > MAX_PREVIEW:
            raise Failure("output-limit")
        # Do not trust --risk-policy off/warn to disable the adapter's own gate.
        risk = required or incomplete or bool(warnings)
        return result, cursor, risk, preview
    except (KeyError, ValueError, TypeError, UnicodeError) as error:
        raise Failure("invalid-response") from error


def run(command: list[str], request: dict, timeout: float) -> bytes:
    payload = json.dumps(request, ensure_ascii=True).encode("utf-8")
    deadline = time.monotonic() + timeout
    # The backend is isolated from the terminal and gets only the request pipe.
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, start_new_session=True, close_fds=True)
    assert process.stdin is not None and process.stdout is not None
    selector = selectors.DefaultSelector()
    os.set_blocking(process.stdin.fileno(), False)
    os.set_blocking(process.stdout.fileno(), False)
    selector.register(process.stdin, selectors.EVENT_WRITE, "input")
    selector.register(process.stdout, selectors.EVENT_READ, "output")
    cancelled = select.poll()
    cancelled.register(sys.stdout.fileno(), select.POLLERR | select.POLLHUP)
    output = bytearray()
    sent = 0
    try:
        while selector.get_map():
            if _CANCELLED or cancelled.poll(0):
                raise Failure("unavailable")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise Failure("timeout")
            for key, _ in selector.select(min(remaining, 0.05)):
                if key.data == "input":
                    try:
                        sent += os.write(key.fd, payload[sent:sent + 4096])
                    except BrokenPipeError:
                        sent = len(payload)
                    if sent == len(payload):
                        selector.unregister(key.fileobj)
                        process.stdin.close()
                else:
                    chunk = os.read(key.fd, 16384)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        process.stdout.close()
                    else:
                        output.extend(chunk)
                        if len(output) > MAX_OUTPUT:
                            raise Failure("output-limit")
        while process.poll() is None:
            if _CANCELLED or cancelled.poll(0):
                raise Failure("unavailable")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise Failure("timeout")
            time.sleep(min(remaining, 0.05))
        returncode = process.returncode
        if returncode:
            raise Failure("backend-error")
        return bytes(output)
    finally:
        selector.close()
        # Kill the isolated process group, including descendants holding pipes.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        process.stdin.close()
        process.stdout.close()


def main() -> None:
    for signum in (signal.SIGHUP, signal.SIGTERM, signal.SIGINT):
        signal.signal(signum, cancel_signal)
    try:
        timeout = float(sys.argv[1])
        if not math.isfinite(timeout) or not 0.05 <= timeout <= 60 or sys.argv[2] != "--" or len(sys.argv) < 4:
            raise Failure("invalid-input")
        request = read_request()
        result, cursor, risk, preview = validate(run(sys.argv[3:], request, timeout), request)
        emit("ok", result, cursor, risk, preview)
    except Failure as error:
        emit(str(error))
    except (OSError, ValueError, IndexError, UnicodeError):
        emit("unavailable")


if __name__ == "__main__":
    main()
