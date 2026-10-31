#!/usr/bin/env python3
"""Real setup and one-line Bash init checks, synthetic key only, no API requests."""
import json
import os
from pathlib import Path
import pty
import select
import shlex
import shutil
import signal
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / 'target/debug/reedline-ai').resolve()

class Terminal:
    def __init__(self, folder, args, executable=BINARY):
        self.data = bytearray()
        self.pid, self.fd = pty.fork()
        if not self.pid:
            env = {k:v for k,v in os.environ.items() if not k.startswith('REEDLINE_AI_') and not k.startswith('BASH_FUNC_') and k not in ('BASH_ENV', 'ENV', 'PROMPT_COMMAND')}
            env.update(HOME=str(folder), XDG_CONFIG_HOME=str(folder/'config'), TERM='xterm-256color', LC_ALL='en_US.UTF-8' if sys.platform == 'darwin' else 'C.UTF-8', INPUTRC='/dev/null', HISTFILE='/dev/null', PS1='SETUPTEST> ')
            os.chdir(folder)
            os.execve(str(executable), [str(executable), *args], env)
    def send(self, value):
        os.write(self.fd, value if isinstance(value, bytes) else value.encode())
    def expect(self, text, timeout=5):
        deadline = time.monotonic()+timeout
        while text.encode() not in self.data:
            if time.monotonic() > deadline:
                raise AssertionError(f'missing {text!r}; transcript={self.data!r}')
            if select.select([self.fd], [], [], .05)[0]:
                try: chunk=os.read(self.fd, 65536)
                except OSError: chunk=b''
                if not chunk: raise AssertionError(f'closed before {text!r}; transcript={self.data!r}')
                self.data.extend(chunk)
    def pump(self, seconds=.1):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([self.fd], [], [], .02)[0]:
                try: chunk = os.read(self.fd, 65536)
                except OSError: chunk = b''
                if not chunk: return
                self.data.extend(chunk)
    def wait_file(self, path):
        deadline = time.monotonic() + 5
        while not path.exists():
            assert time.monotonic() < deadline, (str(path), bytes(self.data[-6000:]))
            if select.select([self.fd], [], [], .05)[0]:
                try: chunk = os.read(self.fd, 65536)
                except OSError: chunk = b''
                assert chunk, bytes(self.data[-6000:])
                self.data.extend(chunk)
        return path.read_text()
    def finish(self):
        deadline = time.monotonic()+5
        while time.monotonic()<deadline:
            pid,status=os.waitpid(self.pid,os.WNOHANG)
            if pid:
                os.close(self.fd); self.pid=None
                return os.waitstatus_to_exitcode(status)
            self.pump(.02)
        raise AssertionError(('process did not exit', bytes(self.data[-3000:])))
    def close(self):
        if self.pid:
            # Close the PTY before waiting: macOS can wait for terminal teardown on exit.
            os.close(self.fd)
            try: os.kill(self.pid, signal.SIGKILL)
            except ProcessLookupError: pass
            os.waitpid(self.pid,0); self.pid=None

def wizard(folder, answer='y', key='SYNTHETIC-HIDDEN-KEY'):
    t=Terminal(folder,['setup'])
    try:
        t.expect('Base URL'); t.send('http://127.0.0.1:1/v1\n')
        t.expect('Model name'); t.send('synthetic-model\n')
        t.expect('API key (hidden'); t.send(key+'\r')
        t.expect('Save these settings locally?'); t.send(answer+'\n')
        t.expect('Saved.' if answer=='y' else 'Cancelled;')
        assert not key or key.encode() not in t.data, 'key echoed in terminal'
        assert t.finish()==0
    finally: t.close()

with tempfile.TemporaryDirectory(prefix='reedline-setup-') as name:
    home=Path(name).resolve(); path=home/'config/reedline-ai/settings.json'
    wizard(home, 'n'); assert not path.exists()
    wizard(home); data=json.loads(path.read_text()); assert data['endpoint']=='http://127.0.0.1:1/v1/chat/completions'; assert data['api_key']=='SYNTHETIC-HIDDEN-KEY'
    assert path.stat().st_mode&0o777==0o600
    before=path.read_bytes(); wizard(home, 'n', 'NEW-SYNTHETIC-KEY'); assert path.read_bytes()==before
    t=Terminal(home,['setup'])
    try:
        t.expect('Base URL'); t.send('\n'); t.expect('Model name'); t.send('synthetic-model\n'); t.expect('API key (hidden'); t.send(b'PARTIAL-KEY\x03'); t.expect('setup cancelled'); assert b'PARTIAL-KEY' not in t.data; assert t.finish()!=0; assert path.read_bytes()==before
    finally: t.close()
    wizard(home, 'y', ''); assert 'api_key' not in json.loads(path.read_text())
    # Real interactive Bash initialization from the shipped binary, without API.
    t=Terminal(home,['--noprofile','--norc','-i'],Path(os.environ.get('RAI_BASH') or shutil.which('bash') or '/bin/bash'))
    try:
        t.expect('SETUPTEST> ')
        t.send(f'eval "$({shlex.quote(str(BINARY))} init --shell bash --mock)"; printf "INIT_OK\\n"\n')
        t.expect('INIT_OK\r\n')
        t.send(f'eval "$({shlex.quote(str(BINARY))} init --shell bash)"\n')
        t.expect('already enabled; run reedline_ai_disable before init again.')
        t.send('printf "%s" "${REEDLINE_AI_ARGS[*]}" > args-snapshot\n')
        assert t.wait_file(home/'args-snapshot')=='--backend mock'
        t.send("bind -x '\"\\C-x\\C-t\":printf \"%s\" \"$READLINE_LINE\" > line-snapshot'\n")
        t.pump()
        t.send('echo 世'); t.send(b'\x18\x07')
        deadline = time.monotonic() + 5
        while b'reedline-ai preview (not executed)' not in t.data:
            assert time.monotonic() < deadline, bytes(t.data[-6000:])
            t.send(b'\x18\x07'); t.pump()
        t.send(b'\x18\x07'); t.expect('inserted only, not executed')
        t.send(b'\x18\x14')
        actual = t.wait_file(home/'line-snapshot')
        assert actual == 'echo 世界', (actual, bytes(t.data[-6000:]))
        t.send(b'\x03'); time.sleep(.1); t.send('reedline_ai_disable; printf "DISABLED_OK\\n"\n'); t.expect('DISABLED_OK\r\n'); t.send('exit\n'); assert t.finish()==0
    finally: t.close()
    assert not any((home/p).exists() for p in ('.bashrc','.profile','.bash_history'))
print('Setup PTY PASS: hidden key, consent/cancel/Ctrl-C, replacement, no-key mode, real Bash init/request/accept/disable; no API calls')
