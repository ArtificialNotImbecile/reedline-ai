#!/usr/bin/env python3
"""Offline POSIX PTY smoke for actual PowerShell/PSReadLine, not Windows coverage.

Usage: python3 integrations/powershell/tests/pty_smoke.py /path/to/pwsh [binary]
No profiles or provider APIs; suggestions are inspected in the edit buffer,
never submitted. The only submitted command is a fixed local history marker.
"""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[3]
PWSH = str(Path(sys.argv[1]).resolve())
MODULE = Path(os.environ.get('RAI_POWERSHELL_ADAPTER_PATH', ROOT / 'integrations/powershell/ReedlineAI.psm1')).resolve()
BINARY = str(Path(sys.argv[2] if len(sys.argv) > 2 else ROOT / 'target/debug/reedline-ai').resolve())
REQUEST = b'\x1b[17~'
ACCEPT = b'\x1b[18~'
CANCEL = b'\x1b[19~'
SNAPSHOT = b'\x1b[24~'


def quote(text):
    return "'" + str(text).replace("'", "''") + "'"


with tempfile.TemporaryDirectory(prefix='reedline-ai-pwsh-pty-') as temporary:
    directory = Path(temporary)
    snapshot = directory / 'snapshot.json'
    ready = directory / 'ready'
    disabled = directory / 'disabled'
    memory = directory / 'memory.json'
    memory.write_text(json.dumps({'version': 1, 'confirmed': {'CCTV': {
        'path': str(directory / '世界🦀'), 'workflows': {'run': {
            'steps': [{'action': 'change_directory'}]}}}}, 'suggestions': []}))
    memory.chmod(0o600)
    fixture = directory / 'safe-replacement-fixture'
    fixture.write_text(f'''#!{sys.executable}
import json, subprocess, sys
raw = sys.stdin.read()
r = json.loads(raw)
if r['buffer'] == 'fixture replacement':
    text = 'echo 世界🦀'
    print(json.dumps(dict(protocol=r['protocol'], request_id=r['request_id'], buffer=r['buffer'], cursor=r['cursor'], insertion='', replacement=text, result=text, result_cursor=len(text.encode()), risk_review=dict(warnings=[], incomplete=False), additional_acceptance_required=False)))
else:
    p = subprocess.run([{BINARY!r}] + sys.argv[1:], input=raw, text=True, capture_output=True)
    sys.stdout.write(p.stdout)
    sys.exit(p.returncode)
''')
    fixture.chmod(0o700)
    setup = directory / 'setup.ps1'
    setup.write_text(f"""
$ErrorActionPreference = 'Stop'
Import-Module PSReadLine
Set-PSReadLineOption -EditMode Emacs -PredictionSource None -HistorySaveStyle SaveNothing
function global:prompt {{ 'PS_TEST> ' }}
Import-Module {quote(MODULE)}
Enable-ReedlineAI -Binary {quote(fixture)} -BackendArguments @('--backend', 'mock', '--config', {quote(memory)}) -RequestChord F6 -AcceptChord F7 -CancelChord F8
Set-PSReadLineKeyHandler -Chord 'F12' -ScriptBlock {{
    $line = ''; $cursor = 0
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
    [IO.File]::WriteAllText({quote(snapshot)}, (@{{ buffer = $line; cursor = $cursor }} | ConvertTo-Json -Compress))
}}
[IO.File]::WriteAllText({quote(ready)}, 'ready')
""", encoding='utf-8')
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 120, 0, 0))
    env = dict(os.environ, TERM='xterm-256color', HOME=temporary,
               XDG_CACHE_HOME=str(directory / 'cache'),
               XDG_CONFIG_HOME=str(directory / 'config'),
               XDG_DATA_HOME=str(directory / 'data'))
    process = subprocess.Popen([PWSH, '-NoLogo', '-NoProfile', '-NoExit', '-File', str(setup)],
                               cwd=temporary, stdin=slave, stdout=slave, stderr=slave,
                               env=env, start_new_session=True)
    os.close(slave)
    transcript = bytearray()
    pending = bytearray()

    def pump(seconds=0.2):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if not select.select([master], [], [], max(0, end - time.monotonic()))[0]:
                continue
            try:
                data = os.read(master, 65536)
            except OSError:
                return
            if not data:
                return
            transcript.extend(data)
            pending.extend(data)
            while b'\x1b[6n' in pending:
                index = pending.index(b'\x1b[6n')
                del pending[:index + 4]
                os.write(master, b'\x1b[1;1R')
            if len(pending) > 1000:
                del pending[:-10]

    def send(data, seconds=0.2):
        os.write(master, data.encode() if isinstance(data, str) else data)
        pump(seconds)

    def state():
        snapshot.unlink(missing_ok=True)
        send(SNAPSHOT, 0.1)
        end = time.monotonic() + 3
        while not snapshot.exists() and time.monotonic() < end:
            pump(0.05)
        assert snapshot.exists(), bytes(transcript[-6000:])
        return json.loads(snapshot.read_text())

    def line(text):
        send(CANCEL)
        send(b'\x01\x0b')
        send(text)

    def preview(text):
        line(text)
        send(REQUEST, 0.6)
        send(REQUEST)
        assert state()['buffer'] == text, bytes(transcript[-6000:])

    try:
        end = time.monotonic() + 12
        while not ready.exists() and time.monotonic() < end:
            pump(0.1)
        assert ready.exists(), bytes(transcript)
        pump(0.3)
        line('')
        send(REQUEST)
        assert state()['buffer'] == ''
        preview('run CCTV')
        send(ACCEPT)
        assert state()['buffer'] == 'run CCTV'  # Unknown Set-Location review remains blocked.
        preview('fixture replacement')
        send(ACCEPT)
        actual = state()
        assert actual == {'buffer': 'echo 世界🦀', 'cursor': 9}, (actual, bytes(transcript[-6000:]))
        send(ACCEPT)
        assert state() == actual
        send(b'\x18\x15')
        assert state()['buffer'] == 'fixture replacement'  # Whole-line edit is undoable, never submitted.
        preview('echo 世')
        assert 'Preview: echo 世界'.encode() in transcript
        send(ACCEPT)
        assert state() == {'buffer': 'echo 世界', 'cursor': 7}, bytes(transcript[-6000:])
        send(ACCEPT)
        assert state()['buffer'] == 'echo 世界'  # one-shot, not submitted
        send(b'\x18\x15')  # Native Ctrl+x, Ctrl+u Undo in Emacs mode.
        assert state()['buffer'] == 'echo 世', (state(), bytes(transcript[-6000:]))
        preview('echo 世')
        send('x')
        send(ACCEPT)
        assert state()['buffer'] == 'echo 世x'
        preview('echo 世')
        send(CANCEL)
        send(ACCEPT)
        assert state()['buffer'] == 'echo 世', (state(), bytes(transcript[-6000:]))
        line('echo 界')
        send(b'\x1b[D')
        send(REQUEST, 0.6)
        send(REQUEST)
        send(ACCEPT)
        assert state() == {'buffer': 'echo 世界', 'cursor': 6}, bytes(transcript[-6000:])
        preview('git st')
        send(ACCEPT)
        assert state()['buffer'] == 'git st'  # Incomplete review cannot insert.
        line('echo 世')
        send(REQUEST, 0.01)
        send('x', 0.6)
        send(REQUEST)
        send(ACCEPT)
        assert state()['buffer'] == 'echo 世x'  # Editing remains live while pending.
        line('Get-Chil')
        send(b'\t')
        assert state()['buffer'].startswith('Get-ChildItem')
        line("Write-Output 'NATIVE_HISTORY_MARKER'")
        send(b'\r', 0.3)  # Fixed test command only, never suggested text.
        send(b'\x1b[A')
        assert state()['buffer'] == "Write-Output 'NATIVE_HISTORY_MARKER'"
        line("Disable-ReedlineAI; [IO.File]::WriteAllText(" + quote(disabled) + ", [string](@(Get-PSReadLineKeyHandler -Chord F6,F7,F8).Count))")
        send(b'\r', 0.2)
        deadline = time.monotonic() + 5
        while not disabled.exists() and time.monotonic() < deadline:
            pump(0.1)
        assert disabled.exists() and disabled.read_text() == '0', bytes(transcript[-6000:])
        print('PASS: actual Linux PowerShell/PSReadLine PTY request, preview, accept, Unicode FIM, '
              'undo, blank, stale input, cancel, risk blocking, live editing, native Tab/history, session disable.')
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        os.close(master)
