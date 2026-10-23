#!/usr/bin/env python3
"""Bounded stdin transport for opt-in Nu/Clink adapters, never a command runner.

Only an explicitly configured backend executable is launched. Suggested text is
never evaluated. Diagnostics are intentionally static and contain no payloads.
"""
from __future__ import annotations

import argparse
import ctypes
import json
import os
from pathlib import Path
import re
import signal
import select
import stat
import subprocess
import sys
import tempfile
import threading
import time
import uuid

MAX_REQUEST = 24 * 1024
MAX_RESPONSE = 64 * 1024
TIMEOUT_SECONDS = 15
PREFIX = "reedline-ai-clink-"
_CANCELLED = False
_LIFELINE = None


def cancel_signal(_signum, _frame) -> None:
    # Never raise between Popen creating a process and returning its handle.
    global _CANCELLED
    _CANCELLED = True


def supervise_posix() -> int | None:
    """Keep cleanup alive when a native shell SIGKILLs its direct child.

    The direct helper only waits. Its forked supervisor owns the backend and
    polls a private pipe whose sole writer is the direct helper. Even SIGKILL
    closes that writer. The backend cannot inherit either end (close_fds).
    Called before reading input or starting threads; no payload reaches disk.
    """
    global _LIFELINE
    read_fd, write_fd = os.pipe()
    try:
        child = os.fork()
    except BaseException:
        os.close(read_fd)
        os.close(write_fd)
        raise
    if child == 0:
        os.close(write_fd)
        _LIFELINE = read_fd
        return None
    os.close(read_fd)
    try:
        while True:
            if _CANCELLED and write_fd is not None:
                os.close(write_fd)
                write_fd = None
            finished, status = os.waitpid(child, os.WNOHANG)
            if finished:
                return os.waitstatus_to_exitcode(status) if os.WIFEXITED(status) else 1
            time.sleep(0.025)
    finally:
        if write_fd is not None:
            os.close(write_fd)


def safe_text(value: object, maximum: int) -> bool:
    if not isinstance(value, str):
        return False
    try:
        if len(value.encode("utf-8")) > maximum:
            return False
    except UnicodeEncodeError:
        return False
    return not any(
        ord(c) < 32 or 0x7F <= ord(c) <= 0x9F or ord(c) == 0x061C
        or 0x200B <= ord(c) <= 0x200F or 0x2028 <= ord(c) <= 0x202E
        or 0x2060 <= ord(c) <= 0x206F or ord(c) == 0xFEFF for c in value
    )


def check_request(request: object) -> dict:
    if not isinstance(request, dict) or set(request) != {"protocol", "request_id", "shell", "cwd", "buffer", "cursor"}:
        raise ValueError("invalid request")
    if type(request["protocol"]) is not int or request["protocol"] != 1:
        raise ValueError("invalid protocol")
    if not isinstance(request["request_id"], str) or not re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", request["request_id"]):
        raise ValueError("invalid identity")
    if not isinstance(request["shell"], dict) or request["shell"].get("kind") not in ("cmd", "nushell"):
        raise ValueError("invalid shell")
    if not safe_text(request["shell"].get("version"), 256):
        raise ValueError("invalid version")
    if not safe_text(request["cwd"], 4096) or not request["cwd"] or not safe_text(request["buffer"], 8192):
        raise ValueError("invalid input")
    cursor = request["cursor"]
    encoded = request["buffer"].encode("utf-8")
    if type(cursor) is not int or cursor < 0 or cursor > len(encoded):
        raise ValueError("invalid cursor")
    encoded[:cursor].decode("utf-8")
    return request


def check_response(request: dict, response: object) -> tuple[dict, bool]:
    if not isinstance(response, dict):
        raise ValueError("invalid response")
    for name in ("protocol", "request_id", "buffer", "cursor"):
        if type(response.get(name)) is not type(request[name]) or response[name] != request[name]:
            raise ValueError("mismatched response")
    if not safe_text(response.get("insertion"), 1024) or not safe_text(response.get("result"), 9216) or "```" in response["insertion"]:
        raise ValueError("invalid result")
    source = request["buffer"].encode("utf-8")
    insertion = response["insertion"].encode("utf-8")
    expected = source[:request["cursor"]] + insertion + source[request["cursor"]:]
    if response["result"].encode("utf-8") != expected:
        raise ValueError("modified source")
    if type(response.get("result_cursor")) is not int or response["result_cursor"] != request["cursor"] + len(insertion):
        raise ValueError("invalid result cursor")
    review = response.get("risk_review")
    if not isinstance(review, dict) or type(review.get("incomplete")) is not bool:
        raise ValueError("missing risk review")
    if not isinstance(review.get("warnings"), list) or not all(isinstance(w, str) for w in review["warnings"]):
        raise ValueError("missing risk warnings")
    if type(response.get("additional_acceptance_required")) is not bool:
        raise ValueError("missing acceptance flag")
    blocked = response["additional_acceptance_required"] or review["incomplete"] or bool(review["warnings"])
    return response, blocked


def _private_windows_directory(path: Path) -> None:
    """Create atomically with a protected current-user + SYSTEM inherited DACL."""
    from ctypes import wintypes as w
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    advapi = ctypes.WinDLL("advapi32", use_last_error=True)
    kernel.GetCurrentProcess.restype = w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]
    kernel.LocalFree.argtypes = [w.HLOCAL]
    kernel.LocalFree.restype = w.HLOCAL
    advapi.OpenProcessToken.argtypes = [w.HANDLE, w.DWORD, ctypes.POINTER(w.HANDLE)]
    advapi.GetTokenInformation.argtypes = [w.HANDLE, ctypes.c_int, w.LPVOID, w.DWORD, ctypes.POINTER(w.DWORD)]
    advapi.ConvertSidToStringSidW.argtypes = [w.LPVOID, ctypes.POINTER(w.LPWSTR)]
    advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [w.LPCWSTR, w.DWORD, ctypes.POINTER(w.LPVOID), ctypes.POINTER(w.DWORD)]
    kernel.GetVolumePathNameW.argtypes = [w.LPCWSTR, w.LPWSTR, w.DWORD]
    kernel.GetVolumeInformationW.argtypes = [w.LPCWSTR, w.LPWSTR, w.DWORD, ctypes.POINTER(w.DWORD), ctypes.POINTER(w.DWORD), ctypes.POINTER(w.DWORD), w.LPWSTR, w.DWORD]
    root = ctypes.create_unicode_buffer(32768)
    flags = w.DWORD()
    if not kernel.GetVolumePathNameW(str(path.parent), root, len(root)) or not kernel.GetVolumeInformationW(root.value, None, 0, None, None, ctypes.byref(flags), None, 0) or not flags.value & 0x00000008:
        raise OSError("temporary volume lacks persistent ACL support")
    token = w.HANDLE()
    if not advapi.OpenProcessToken(kernel.GetCurrentProcess(), 0x0008, ctypes.byref(token)):
        raise ctypes.WinError(ctypes.get_last_error())
    sid_text = w.LPWSTR()
    descriptor = w.LPVOID()
    try:
        size = w.DWORD()
        advapi.GetTokenInformation(token, 1, None, 0, ctypes.byref(size))
        info = ctypes.create_string_buffer(size.value)
        if not advapi.GetTokenInformation(token, 1, info, size, ctypes.byref(size)):
            raise ctypes.WinError(ctypes.get_last_error())
        sid = ctypes.cast(info, ctypes.POINTER(w.LPVOID))[0]
        if not advapi.ConvertSidToStringSidW(sid, ctypes.byref(sid_text)):
            raise ctypes.WinError(ctypes.get_last_error())
        sddl = f"D:P(A;OICI;FA;;;{sid_text.value})(A;OICI;FA;;;SY)"
        if not advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, 1, ctypes.byref(descriptor), None):
            raise ctypes.WinError(ctypes.get_last_error())
        class SecurityAttributes(ctypes.Structure):
            _fields_ = [("length", w.DWORD), ("descriptor", w.LPVOID), ("inherit", w.BOOL)]
        security = SecurityAttributes(ctypes.sizeof(SecurityAttributes), descriptor, False)
        kernel.CreateDirectoryW.argtypes = [w.LPCWSTR, ctypes.POINTER(SecurityAttributes)]
        if not kernel.CreateDirectoryW(str(path), ctypes.byref(security)):
            raise ctypes.WinError(ctypes.get_last_error())
    finally:
        if descriptor:
            kernel.LocalFree(descriptor)
        if sid_text:
            kernel.LocalFree(ctypes.cast(sid_text, w.HLOCAL))
        kernel.CloseHandle(token)


def prepare_directory() -> Path:
    parent = Path(tempfile.gettempdir()).resolve()
    directory = parent / (PREFIX + uuid.uuid4().hex)
    if os.name == "nt":
        _private_windows_directory(directory)
    else:
        directory.mkdir(mode=0o700)
    return directory


def read_envelope(path: Path) -> dict:
    if path.name != "request.json" or not re.fullmatch(PREFIX + r"[a-f0-9]{32}", path.parent.name):
        raise ValueError("unexpected request path")
    for target in (path.parent, path):
        info = target.lstat()
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
            raise ValueError("reparse points are not allowed")
        if os.name != "nt" and (info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) & 0o077):
            raise ValueError("request permissions are not private")
    if not path.is_file() or not path.parent.is_dir():
        raise ValueError("unexpected request type")
    with path.open("rb") as stream:
        raw = stream.read(MAX_REQUEST + 1)
    if len(raw) > MAX_REQUEST:
        raise ValueError("oversized request")
    return check_envelope(json.loads(raw))


def check_envelope(envelope: object) -> dict:
    if not isinstance(envelope, dict) or set(envelope) != {"backend", "arguments", "request"}:
        raise ValueError("invalid envelope")
    backend = envelope["backend"]
    if not isinstance(backend, str) or not Path(backend).is_absolute() or not Path(backend).is_file() or (os.name == 'nt' and Path(backend).suffix.lower() != '.exe'):
        raise ValueError("backend must be an absolute executable path")
    arguments = envelope["arguments"]
    if not isinstance(arguments, list) or len(arguments) > 32 or not all(safe_text(a, 4096) for a in arguments):
        raise ValueError("invalid backend arguments")
    check_request(envelope["request"])
    return envelope


def enter_windows_job():
    """Contain this helper and all future descendants, or fail before spawning.

    The non-inherited job handle stays alive until helper process exit. Windows
    closes it at exit and kills remaining descendants, even after a timeout.
    Only the standalone CLI calls this; never attach an importing test process.
    """
    from ctypes import wintypes as w
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateJobObjectW.argtypes = [w.LPVOID, w.LPCWSTR]
    kernel.CreateJobObjectW.restype = w.HANDLE
    kernel.GetCurrentProcess.restype = w.HANDLE
    kernel.SetInformationJobObject.argtypes = [w.HANDLE, ctypes.c_int, w.LPVOID, w.DWORD]
    kernel.AssignProcessToJobObject.argtypes = [w.HANDLE, w.HANDLE]
    kernel.CloseHandle.argtypes = [w.HANDLE]
    class BasicLimits(ctypes.Structure):
        _fields_ = [("process_time", ctypes.c_int64), ("job_time", ctypes.c_int64),
                    ("flags", w.DWORD), ("minimum_working_set", ctypes.c_size_t),
                    ("maximum_working_set", ctypes.c_size_t), ("active_processes", w.DWORD),
                    ("affinity", ctypes.c_size_t), ("priority_class", w.DWORD), ("scheduling_class", w.DWORD)]
    class IoCounters(ctypes.Structure):
        _fields_ = [(name, ctypes.c_uint64) for name in ("read_ops", "write_ops", "other_ops", "read_bytes", "write_bytes", "other_bytes")]
    class ExtendedLimits(ctypes.Structure):
        _fields_ = [("basic", BasicLimits), ("io", IoCounters),
                    ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                    ("peak_process_memory", ctypes.c_size_t), ("peak_job_memory", ctypes.c_size_t)]
    handle = kernel.CreateJobObjectW(None, None)
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())
    limits = ExtendedLimits()
    limits.basic.flags = 0x00002000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    if not kernel.SetInformationJobObject(handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)) or not kernel.AssignProcessToJobObject(handle, kernel.GetCurrentProcess()):
        error = ctypes.get_last_error()
        kernel.CloseHandle(handle)
        raise ctypes.WinError(error)
    return handle


def run_backend(envelope: dict, timeout: float = TIMEOUT_SECONDS) -> dict:
    request = check_request(envelope["request"])
    if not request["buffer"].strip():
        raise ValueError("blank input does not start a backend")
    payload = json.dumps(request, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    if len(payload) > MAX_REQUEST:
        raise ValueError("oversized request")
    deadline = time.monotonic() + timeout
    if _CANCELLED:
        raise ValueError("cancelled")
    parent_pipe = None
    if os.name != "nt" and hasattr(select, "poll"):
        parent_pipe = select.poll()
        try:
            parent_pipe.register(sys.stdout.fileno(), select.POLLERR | select.POLLHUP)
            if _LIFELINE is not None:
                parent_pipe.register(_LIFELINE, select.POLLHUP | select.POLLERR)
        except (OSError, ValueError):
            parent_pipe = None
    if parent_pipe is not None and parent_pipe.poll(0):
        raise ValueError("cancelled")
    process = subprocess.Popen(
        [envelope["backend"], *envelope["arguments"], "--adapter-request"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        shell=False, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        start_new_session=os.name != "nt",
    )
    output = bytearray()
    failure: list[BaseException] = []
    def kill_tree() -> None:
        try:
            if os.name != "nt":
                os.killpg(process.pid, signal.SIGKILL)
            elif process.poll() is None:
                process.kill()
            # Windows CLI descendants are additionally killed by the enclosing
            # helper's kill-on-close Job Object when its process exits.
        except ProcessLookupError:
            pass
    def exchange() -> None:
        try:
            assert process.stdin is not None and process.stdout is not None
            process.stdin.write(payload)
            process.stdin.close()
            while True:
                chunk = process.stdout.read(min(8192, MAX_RESPONSE + 1 - len(output)))
                if not chunk:
                    break
                output.extend(chunk)
                if len(output) > MAX_RESPONSE:
                    raise ValueError("oversized response")
        except (OSError, ValueError) as error:
            failure.append(error)
            kill_tree()
    worker = threading.Thread(target=exchange, daemon=True)
    worker.start()
    try:
        while process.poll() is None or worker.is_alive():
            if _CANCELLED or (parent_pipe is not None and parent_pipe.poll(0)):
                raise ValueError("cancelled")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise subprocess.TimeoutExpired(process.args, timeout)
            worker.join(timeout=min(remaining, 0.05))
            if not worker.is_alive() and process.poll() is None:
                time.sleep(min(remaining, 0.01))
        if failure or process.returncode != 0 or len(output) > MAX_RESPONSE:
            raise ValueError("backend failed")
        response, _ = check_response(request, json.loads(output))
        return response
    finally:
        kill_tree()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            pass
        worker.join(timeout=0.1)
        # A descendant can retain an inherited pipe. Closing a BufferedReader
        # while its daemon reader holds the lock can deadlock; leave that pipe
        # to process-exit cleanup instead. Never perform an unbounded close.
        if not worker.is_alive():
            for stream in (process.stdin, process.stdout):
                if stream is not None:
                    stream.close()


def response_frame(request: dict, response: dict) -> str:
    response, blocked = check_response(request, response)
    fields = ["RLA1", request["request_id"].encode().hex(), request["buffer"].encode().hex(),
              str(request["cursor"]), response["insertion"].encode().hex(),
              response["result"].encode().hex(), str(response["result_cursor"]), "1" if blocked else "0"]
    return "\n".join(fields) + "\n"


def main() -> int:
    if os.name != "nt":
        for signum in (signal.SIGHUP, signal.SIGTERM, signal.SIGINT):
            signal.signal(signum, cancel_signal)
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--prepare", action="store_true")
    mode.add_argument("--run", type=Path)
    mode.add_argument("--stdin", action="store_true", help="bounded in-memory transport for Nushell")
    args = parser.parse_args()
    if args.prepare:
        try:
            print(str(prepare_directory()).encode("utf-8").hex())
            return 0
        except (OSError, ValueError):
            print("ERROR")
            return 1
    if os.name != "nt":
        supervisor_status = supervise_posix()
        if supervisor_status is not None:
            return supervisor_status
    path = args.run
    verified = False
    response_saved = False
    job_handle = None
    try:
        if os.name == "nt":
            job_handle = enter_windows_job()
        if args.stdin:
            raw = sys.stdin.buffer.read(MAX_REQUEST + 1)
            if len(raw) > MAX_REQUEST:
                raise ValueError("oversized envelope")
            envelope = check_envelope(json.loads(raw))
        else:
            envelope = read_envelope(path)
            verified = True
            # Remove the sole request artifact before starting inference.
            path.unlink()
        response = run_backend(envelope)
        if args.stdin:
            print(json.dumps(response, ensure_ascii=False, separators=(",", ":")))
        else:
            # Clink io.popenyield internally spools stdout to its own temp file.
            # Only a status marker may use that channel, never buffer/result data.
            result_path = path.parent / "response.frame"
            descriptor = os.open(result_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "w", encoding="ascii", newline="\n") as stream:
                stream.write(response_frame(envelope["request"], response))
            response_saved = True
            print("READY")
        return 0
    except (OSError, ValueError, UnicodeError, subprocess.SubprocessError):
        print("ERROR")
        return 1
    finally:
        # Keep job_handle until this helper exits; closing it here would kill
        # the helper itself before stdout has been flushed.
        _ = job_handle
        # Never remove a caller-supplied unverified path or recurse into files.
        if verified:
            path.unlink(missing_ok=True)
            if not response_saved:
                try:
                    path.parent.rmdir()
                except OSError:
                    pass


if __name__ == "__main__":
    raise SystemExit(main())
