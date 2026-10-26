#!/usr/bin/env python3
"""Offline, stdlib-only integration checks against a real Bash/Readline PTY.

Usage: python3 scripts/pty_bash_adapter.py [target/debug/reedline-ai]
Does not read shell profiles, execute suggested text, or call any network API.
"""
from __future__ import annotations

import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shlex
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "target/debug/reedline-ai").resolve()
INVOKE = b"\x18\x07"
ACCEPT = b"\x18\x01"
CONFIRM = b"\x18\x12"
SNAPSHOT = b"\x18\x14"

FIXTURE = r'''
import json, os, pathlib, sys, time
request = json.load(sys.stdin)
config = json.loads(pathlib.Path(os.environ["RAI_FIXTURE_CONFIG"]).read_text())
with open(os.environ["RAI_FIXTURE_CALLS"], "a") as log:
    log.write(json.dumps(dict(request, _test_pid=os.getpid())) + "\n")
while config.get("block_until") and not pathlib.Path(config["block_until"]).exists():
    time.sleep(0.01)
time.sleep(config.get("delay", 0))
mode = config.get("mode", "ok")
if mode == "exit":
    print("\x1b]52;c;evil\x07private backend failure", file=sys.stderr)
    raise SystemExit(9)
if mode == "huge":
    print("x" * 200000)
    raise SystemExit(0)
insert = config.get("insert", "fixture")
source = request["buffer"].encode()
cursor = request["cursor"]
result = (source[:cursor] + insert.encode() + source[cursor:]).decode()
response = dict(protocol=request["protocol"], request_id=request["request_id"], buffer=request["buffer"], cursor=cursor,
    insertion=insert, result=result, result_cursor=cursor + len(insert.encode()),
    risk_review=dict(warnings=config.get("warnings", []), incomplete=config.get("incomplete", False)),
    additional_acceptance_required=config.get("risk", False))
if "replacement" in config:
    response.update(replacement=config["replacement"], insertion="", result=config["replacement"], result_cursor=len(config["replacement"].encode()))
if mode == "wrong-id": response["request_id"] = "another-request"
if mode == "wrong-cursor": response["result_cursor"] = cursor + 1
if mode == "wrong-result": response["result"] = "not an insertion"
if mode == "missing-review": response.pop("risk_review")
if mode == "bool-cursor": response["cursor"] = True
print(json.dumps(response))
'''


class Shell:
    def __init__(self, folder: Path, prompt_array: bool = False, prompt_export: bool = False):
        self.folder = folder
        self.snapshot_file = folder / "snapshot"
        self.config = folder / "fixture.json"
        self.calls_file = folder / "calls.jsonl"
        self.fixture = folder / "backend.py"
        self.fixture.write_text(FIXTURE)
        self.configure()
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
        env = {key: value for key, value in os.environ.items()
               if not key.startswith("BASH_FUNC_") and key not in {"BASH_ENV", "ENV", "PROMPT_COMMAND", "SHELLOPTS", "BASHOPTS"}}
        env.update(HOME=str(folder), INPUTRC="/dev/null", HISTFILE="/dev/null", TERM="xterm-256color",
                   LC_ALL="C.UTF-8", RAI_FIXTURE_CONFIG=str(self.config), RAI_FIXTURE_CALLS=str(self.calls_file),
                   REEDLINE_AI_BIN=str(BINARY), PS1="RAI> ")
        self.process = subprocess.Popen(["bash", "--noprofile", "--norc", "-i"], cwd=folder,
                                        stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
        os.close(slave)
        self.transcript = bytearray()
        self.pump(0.1)
        source = shlex.quote(os.environ.get("RAI_BASH_ADAPTER_PATH", str(ROOT / "integrations/bash/reedline-ai.bash")))
        target = shlex.quote(str(self.snapshot_file))
        prompt = "PROMPT_COMMAND=('__rai_seen=$?' '__rai_second=yes'); " if prompt_array else "PROMPT_COMMAND='__rai_seen=$?'; "
        if prompt_export:
            prompt += "export PROMPT_COMMAND; "
        original_bindings = (
            "__rai_test_original() { READLINE_LINE=restored; READLINE_POINT=8; }; "
            "bind '\"\\C-x\\C-g\": \"OLD_MACRO\"'; "
            "bind '\"\\C-x\\C-a\": backward-char'; "
            "bind -x '\"\\C-x\\C-r\":__rai_test_original'; "
        )
        command = (
            prompt + original_bindings + f"unset HISTFILE; set +H; PS1='RAI> '; source {source}; "
            f"__rai_test_snapshot() {{ builtin printf '%s\\0%s\\0%s\\0%s\\0%s\\0' "
            f'"$READLINE_LINE" "$READLINE_POINT" "$__rai_pending" "$__rai_previewed" "$__rai_confirmed" > {target}; }}; '
            "bind -x '\"\\C-x\\C-t\":__rai_test_snapshot'; "
            f"__rai_test_cd() {{ builtin cd -- {shlex.quote(str(folder / 'elsewhere'))}; }}; "
            "bind -x '\"\\C-x\\C-d\":__rai_test_cd'"
        )
        self.send(command.encode() + b"\r", 0.1)
        assert b"invalid keymap" not in self.transcript, bytes(self.transcript)

    def pump(self, seconds: float = 0.05):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([self.master], [], [], max(0, end - time.monotonic()))[0]:
                try:
                    data = os.read(self.master, 65536)
                except OSError:
                    return
                if not data:
                    return
                self.transcript.extend(data)

    def send(self, value: bytes | str, seconds: float = 0.05):
        os.write(self.master, value.encode() if isinstance(value, str) else value)
        self.pump(seconds)

    def snapshot(self) -> tuple[str, int, str, str, str]:
        self.snapshot_file.unlink(missing_ok=True)
        self.send(SNAPSHOT, 0.02)
        end = time.monotonic() + 1
        while not self.snapshot_file.exists() and time.monotonic() < end:
            self.pump(0.01)
        fields = self.snapshot_file.read_bytes().split(b"\0")
        assert len(fields) == 6 and fields[-1] == b"", fields
        return fields[0].decode(), int(fields[1]), *(x.decode() for x in fields[2:5])

    def line(self, text: str):
        self.send(b"\x01\x0b") # Native Home + kill-line, no command submission.
        self.send(text)

    def command(self, text: str):
        self.line(text)
        self.send(b"\r", 0.05)

    def configure(self, **settings):
        self.config.write_text(json.dumps(settings))

    def fixture_backend(self):
        self.command(f"REEDLINE_AI_BIN=python3; REEDLINE_AI_ARGS=({shlex.quote(str(self.fixture))})")

    def calls(self) -> list:
        if not self.calls_file.exists():
            return []
        return [json.loads(line) for line in self.calls_file.read_text().splitlines()]

    def wait_calls(self, expected: int):
        deadline = time.monotonic() + 2
        while len(self.calls()) < expected and time.monotonic() < deadline:
            self.pump(0.02)
        assert len(self.calls()) == expected, (expected, self.calls())

    def ready_preview(self, line: str, delay: float = 0.2):
        self.line(line)
        self.send(INVOKE, delay)
        state = self.review_ready()
        assert state[0] == line and state[3] == "1", (state, bytes(self.transcript[-3000:]))

    def review_ready(self):
        deadline = time.monotonic() + 2
        while True:
            self.send(ACCEPT)
            state = self.snapshot()
            if state[2] != "1" or time.monotonic() >= deadline:
                return state
            self.pump(0.05)

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        os.close(self.master)


def bridge_cases(shell: Shell):
    bridge = Path(os.environ.get("RAI_BASH_ADAPTER_PATH", str(ROOT / "integrations/bash/reedline-ai.bash"))).parent / "bridge.py"
    request = b"test-id\0" + b"5.2\0/tmp\0echo \x00" + b"5\0"
    env = dict(os.environ, RAI_FIXTURE_CONFIG=str(shell.config), RAI_FIXTURE_CALLS=str(shell.calls_file))
    cases = [
        ({"mode": "wrong-id"}, "invalid-response"),
        ({"mode": "wrong-result"}, "invalid-response"),
        ({"mode": "missing-review"}, "invalid-response"),
        ({"mode": "bool-cursor"}, "invalid-response"),
        ({"mode": "wrong-cursor", "insert": "世界"}, "invalid-response"),
        ({"insert": "x\ny"}, "invalid-response"),
        ({"insert": "\x1b]52;c;injected\x07"}, "invalid-response"),
        ({"insert": "\u202eevil"}, "invalid-response"),
        ({"insert": "```echo bad```"}, "invalid-response"),
        ({"insert": "x" * 1025}, "invalid-response"),
        ({"mode": "huge"}, "output-limit"),
        ({"mode": "exit"}, "backend-error"),
        ({"delay": 1}, "timeout"),
    ]
    for config, expected in cases:
        shell.configure(**config)
        # Only the dedicated deadline case uses a short limit. Other cases test
        # parsing, not Python startup speed on a concurrently loaded machine.
        limit = "0.15" if expected == "timeout" else "2"
        proc = subprocess.run([sys.executable, str(bridge), limit, "--", sys.executable, str(shell.fixture)],
                              input=request, capture_output=True, env=env, timeout=4)
        assert proc.stdout.split(b"\0")[0].decode() == expected, (config, proc.stdout, proc.stderr)
        assert b"private backend failure" not in proc.stdout + proc.stderr
    before = len(shell.calls())
    for request in (b"id\0" + b"5.2\0/tmp\0 \0" + b"1\0",
                    b"id\0" + b"5.2\0/tmp\0echo \xe4\xb8\x96\0" + b"6\0"):
        proc = subprocess.run([sys.executable, str(bridge), "1", "--", sys.executable, str(shell.fixture)],
                              input=request, capture_output=True, env=env, timeout=2)
        assert proc.stdout.split(b"\0")[0] in {b"empty", b"invalid-input"}, proc.stdout
    assert len(shell.calls()) == before, "blank/invalid cursor reached backend"
    # Explicit SIGHUP covers shutdown even on PTYs lacking a controlling tty.
    shell.configure(delay=10)
    before = len(shell.calls())
    process = subprocess.Popen([sys.executable, str(bridge), "8", "--", sys.executable, str(shell.fixture)],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               env=env, start_new_session=True)
    assert process.stdin is not None
    process.stdin.write(b"hup-id\0" + b"5.2\0/tmp\0echo \0" + b"5\0")
    process.stdin.close()
    process.stdin = None
    try:
        deadline = time.monotonic() + 2
        while len(shell.calls()) == before and time.monotonic() < deadline:
            time.sleep(0.02)
        assert len(shell.calls()) == before + 1
        backend_pid = shell.calls()[-1]["_test_pid"]
        process.send_signal(signal.SIGHUP)
        process.communicate(timeout=2)
        try:
            os.kill(backend_pid, 0)
        except ProcessLookupError:
            pass
        else:
            raise AssertionError("SIGHUP orphaned the backend")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


def main():
    if not shutil.which("bash"):
        raise SystemExit("SKIP: Bash unavailable")
    if not BINARY.exists():
        raise SystemExit(f"Build the binary first: {BINARY}")
    with tempfile.TemporaryDirectory(prefix="reedline-bash-") as temp:
        folder = Path(temp)
        (folder / "elsewhere").mkdir()
        (folder / "completion_unique_fixture").write_text("native completion")
        shell = Shell(folder)
        try:
            # Existing scalar prompt code still sees the prior command status.
            shell.command("false")
            shell.command(f"printf '%s' \"$__rai_seen\" > {shlex.quote(str(folder / 'status'))}")
            assert (folder / "status").read_text() == "1"
            # Actual binary, no paid model: invoke and preview do not mutate.
            shell.ready_preview("echo ")
            shell.send(ACCEPT)
            assert shell.snapshot()[:2] == ('echo "hello world"', 18)
            assert b"inserted only, not executed" in shell.transcript
            # Actual conservative git review requires a distinct acknowledgment.
            shell.ready_preview("git ")
            shell.send(ACCEPT)
            assert shell.snapshot()[0] == "git "
            shell.send(CONFIRM)
            shell.send(ACCEPT)
            assert shell.snapshot()[:2] == ("git status", 10)
            # Explicit local config reaches the native adapter without any provider.
            memory = shlex.quote(str(ROOT / 'examples/project-memory.synthetic.json'))
            shell.command(f"REEDLINE_AI_ARGS=(--backend mock --config {memory})")
            shell.ready_preview("启动监控")
            shell.send(CONFIRM)
            shell.send(ACCEPT)
            assert shell.snapshot()[0] == "cd -- '/synthetic/projects/cctv' && 'python' 'app.py'"
            shell.command("REEDLINE_AI_ARGS=(--backend mock)")
            shell.fixture_backend()
            before = len(shell.calls())
            shell.line("cat completion_uni")
            shell.send(b"\t")
            assert shell.snapshot()[0] == "cat completion_unique_fixture "
            shell.command("echo native_history_fixture")
            shell.send(b"\x1b[A")
            assert shell.snapshot()[0] == "echo native_history_fixture"
            shell.line("abc")
            shell.send(b"\x1b[D\x7f") # native cursor movement + backspace
            assert shell.snapshot()[:2] == ("ac", 1)
            shell.line("   ")
            shell.send(INVOKE)
            assert shell.snapshot()[0] == "   "
            assert len(shell.calls()) == before, "native operations or blank input called backend"
            # Bash bind -x uses locale characters; bridge converts to UTF-8 bytes.
            shell.configure(insert="界")
            shell.line("echo 世 后")
            shell.send(b"\x1b[D" * 2)
            assert shell.snapshot()[:2] == ("echo 世 后", 6), shell.snapshot()
            shell.send(INVOKE, 0.2)
            shell.review_ready()
            shell.send(ACCEPT)
            assert shell.snapshot()[:2] == ("echo 世界 后", 7)
            assert shell.calls()[-1]["cursor"] == 8
            assert shell.calls()[-1]["shell"]["kind"] == "bash"
            assert shell.calls()[-1]["cwd"] == str(folder)
            # V2 whole-line edits use the same preview, stale and confirmation gates.
            shell.configure(replacement="echo 世界😀")
            shell.ready_preview("启动监控")
            shell.send(ACCEPT)
            assert shell.snapshot()[:2] == ("echo 世界😀", 8)
            shell.configure(replacement="echo replacement", warnings=["Synthetic risk"])
            shell.ready_preview("启动监控")
            shell.send(ACCEPT)
            assert shell.snapshot()[0] == "启动监控"
            shell.send(CONFIRM)
            shell.send("x")
            shell.send(ACCEPT)
            assert shell.snapshot()[0] == "启动监控x"
            shell.ready_preview("启动监控")
            shell.send(CONFIRM)
            shell.send(ACCEPT)
            assert shell.snapshot()[0] == "echo replacement"
            # Expiry and rollback are checked again at confirmation/acceptance.
            for mutation in ["__rai_created=$((SECONDS-30))", "__rai_preview_time=$((SECONDS-15))", "__rai_preview_time=$((SECONDS+5))"]:
                shell.command("__rai_test_expire() { " + mutation + "; }; bind -x '\"\\C-x\\C-e\":__rai_test_expire'")
                shell.configure(replacement="echo replacement", warnings=["Synthetic risk"])
                shell.ready_preview("启动监控")
                shell.send(CONFIRM)
                shell.send(b"\x18\x05")
                shell.send(CONFIRM)
                shell.send(ACCEPT)
                assert shell.snapshot()[0] == "启动监控", mutation
            # Model punctuation is data, including substitutions and quote breaks.
            marker = folder / "MUST_NOT_EXIST"
            insertion = f'"; $(touch {marker}); `touch {marker}`; echo "'
            shell.configure(insert=insertion)
            shell.ready_preview("echo ")
            shell.send(ACCEPT)
            assert shell.snapshot()[0] == "echo " + insertion
            assert not marker.exists(), "suggested text was executed"
            # Inference runs asynchronously: typing and native snapshot stay live.
            release = folder / "release-backend"
            shell.configure(insert="fixture", block_until=str(release))
            shell.line("echo ")
            count = len(shell.calls())
            shell.send(INVOKE, 0.02)
            shell.wait_calls(count + 1)
            # Backend cannot complete until the test releases it. A live snapshot
            # here proves editing responsiveness without fixed inference sleeps.
            shell.send("x", 0.02)
            assert shell.snapshot()[0] == "echo x"
            shell.send(INVOKE, 0.1)
            assert len(shell.calls()) == count + 1, "duplicate in-flight worker"
            release.touch()
            shell.review_ready()
            assert shell.snapshot()[0] == "echo x"
            assert b"stale result discarded" in shell.transcript
            # Cursor changes after preview make acceptance stale.
            shell.configure(insert="fixture")
            shell.ready_preview("echo ")
            shell.send(b"\x1b[D")
            shell.send(ACCEPT)
            assert shell.snapshot()[:2] == ("echo ", 4)
            # Confirmation is bound to the precise preview and never carried over.
            shell.configure(insert="fixture", warnings=["Synthetic risk"])
            shell.ready_preview("echo ")
            shell.send(CONFIRM)
            shell.send("x")
            shell.send(ACCEPT)
            assert shell.snapshot()[0] == "echo x"
            shell.ready_preview("echo ")
            shell.send(ACCEPT)
            assert shell.snapshot()[0] == "echo "
            shell.send(CONFIRM)
            shell.send(ACCEPT)
            assert shell.snapshot()[0] == "echo fixture"
            # A new prompt must erase confirmation even when retyping the same line.
            shell.ready_preview("echo ")
            shell.send(CONFIRM)
            shell.send(b"\x03")
            shell.line("echo ")
            shell.send(ACCEPT)
            assert shell.snapshot()[0] == "echo " and shell.snapshot()[3:] == ("0", "0")
            # Ctrl-C also cancels a live worker, not just an already-ready preview.
            shell.configure(delay=5)
            shell.line("echo ")
            count = len(shell.calls())
            shell.send(INVOKE, 0.02)
            shell.wait_calls(count + 1)
            pending_pid = shell.calls()[-1]["_test_pid"]
            shell.send(b"\x03", 0.2)
            shell.line("echo ")
            shell.send(ACCEPT)
            assert shell.snapshot()[3:] == ("0", "0")
            try:
                os.kill(pending_pid, 0)
            except ProcessLookupError:
                pass
            else:
                raise AssertionError("cancelled backend process is still alive")
            # Bounded failure, backend error text cannot escape onto the terminal.
            shell.command("REEDLINE_AI_TIMEOUT_SECONDS=0.1")
            shell.configure(insert="fixture", delay=1)
            shell.line("echo ")
            shell.send(INVOKE, 0.02)
            shell.review_ready()
            assert shell.snapshot()[0] == "echo " and b"timeout; command line unchanged" in shell.transcript
            shell.command("REEDLINE_AI_TIMEOUT_SECONDS=8")
            shell.configure(insert="fixture")
            shell.ready_preview("echo ")
            shell.send(b"\x18\x04") # test-only cwd change callback
            shell.send(ACCEPT)
            assert shell.snapshot()[0] == "echo "
            bridge_cases(shell)
            before = len(shell.calls())
            shell.command("reedline_ai_disable")
            shell.line("")
            shell.send(INVOKE)
            assert shell.snapshot()[0] == "OLD_MACRO", "prior macro binding was not restored"
            shell.line("abc")
            shell.send(ACCEPT)
            assert shell.snapshot()[:2] == ("abc", 2), "prior function binding was not restored"
            shell.send(CONFIRM)
            assert shell.snapshot()[:2] == ("restored", 8), "prior bind -x binding was not restored"
            assert len(shell.calls()) == before
            shell.command(f"declare -p PROMPT_COMMAND > {shlex.quote(str(folder / 'prompt-decl'))}")
            assert (folder / "prompt-decl").read_text().startswith("declare -- PROMPT_COMMAND="), "scalar prompt type was not restored"
            # Later rebindings belong to the user/plugin, not the adapter.
            adapter_path = os.environ.get("RAI_BASH_ADAPTER_PATH", str(ROOT / "integrations/bash/reedline-ai.bash"))
            shell.command(f"source {shlex.quote(adapter_path)}")
            shell.command("__rai_test_later() { READLINE_LINE=later; READLINE_POINT=5; }; "
                          "bind '\"\\C-x\\C-g\": \"LATER_MACRO\"'; "
                          "bind '\"\\C-x\\C-a\": forward-char'; "
                          "bind -x '\"\\C-x\\C-r\":__rai_test_later'; reedline_ai_disable")
            for _ in range(2):
                shell.line("")
                shell.send(INVOKE)
                assert shell.snapshot()[0] == "LATER_MACRO", "disable destroyed a later macro binding"
                shell.line("abc")
                shell.send(b"\x01" + ACCEPT)
                assert shell.snapshot()[:2] == ("abc", 1), "disable destroyed a later Readline binding"
                shell.send(CONFIRM)
                assert shell.snapshot()[:2] == ("later", 5), "disable destroyed a later bind -x binding"
                shell.command("reedline_ai_disable")
            # Native prompt arrays preserve every command and status too.
            array_folder = folder / "array-prompt"
            array_folder.mkdir()
            (array_folder / "elsewhere").mkdir()
            array_shell = Shell(array_folder, prompt_array=True, prompt_export=True)
            try:
                array_shell.command("false")
                array_shell.command(f"printf '%s:%s' \"$__rai_seen\" \"$__rai_second\" > {shlex.quote(str(array_folder / 'status'))}")
                assert (array_folder / "status").read_text() == "1:yes"
                array_shell.ready_preview("echo ")
                array_shell.send(b"\x03")
                array_shell.line("echo ")
                array_shell.send(ACCEPT)
                assert array_shell.snapshot()[3:] == ("0", "0")
                array_shell.command("reedline_ai_disable")
                array_shell.command(f"declare -p PROMPT_COMMAND > {shlex.quote(str(array_folder / 'prompt-decl'))}")
                declaration = (array_folder / "prompt-decl").read_text()
                assert declaration.startswith("declare -ax PROMPT_COMMAND="), declaration
                assert "__rai_seen" in declaration and "__rai_second" in declaration, declaration
                # Exported indexed argument arrays are accepted when enabling.
                array_shell.command(f"export REEDLINE_AI_ARGS; source {shlex.quote(adapter_path)}")
                array_shell.ready_preview("echo ")
                # Terminal exit while the isolated backend is pending must not orphan it.
                array_shell.fixture_backend()
                array_shell.configure(delay=10)
                array_shell.line("echo ")
                count = len(array_shell.calls())
                array_shell.send(INVOKE, 0.02)
                array_shell.wait_calls(count + 1)
                backend_pid = array_shell.calls()[-1]["_test_pid"]
                array_shell.send(b"\x01\x0b\x04", 0.3)
                array_shell.process.wait(timeout=2)
                end = time.monotonic() + 1
                while True:
                    try:
                        os.kill(backend_pid, 0)
                    except ProcessLookupError:
                        break
                    if time.monotonic() >= end:
                        raise AssertionError("terminal-close orphaned the backend")
                    time.sleep(0.02)
            finally:
                array_shell.close()
            print("PTY Bash adapter PASS: real mock backend, preview/accept, risk gate, UTF-8 byte cursor and suffix, native Tab/history/editing, inert metacharacters, zero-call blank, async typing, stale edit/cursor/cwd, timeout, malformed/control/oversize response rejection, prompt preservation, cancellation/terminal-exit cleanup, disable restoration")
        except BaseException:
            print(shell.transcript[-10000:].decode("utf-8", errors="backslashreplace"), file=sys.stderr)
            raise
        finally:
            shell.close()


if __name__ == "__main__":
    main()
