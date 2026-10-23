"""Real Nushell runtime/PTY regressions; no provider calls or user profiles.

NU_BINARY=/absolute/path/to/nu python3 -m unittest discover -s integrations/nushell
A missing Nushell or POSIX PTY is reported as skipped, never as a native pass.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

NU = os.environ.get('NU_BINARY') or shutil.which('nu')
ADAPTER = Path(os.environ.get('RAI_NUSHELL_ADAPTER_PATH', Path(__file__).with_name('reedline-ai.nu'))).resolve()


@unittest.skipUnless(NU, 'Nushell runtime not installed; set NU_BINARY')
class PureNuTests(unittest.TestCase):
    def run_nu(self, code):
        return subprocess.run([NU, '-n', '-c', f'source {json.dumps(str(ADAPTER), ensure_ascii=False)}; {code}'], capture_output=True, text=True, timeout=5)

    def test_grapheme_byte_roundtrip(self):
        result = self.run_nu('for item in [[text cursor bytes]; ["echo 世" 6 8] ["echo é" 6 8] ["echo 🦀" 6 9] ["" 0 0]] { if (reedline-ai-grapheme-to-byte $item.text $item.cursor) != $item.bytes { error make {msg: "wrong bytes"} }; if (reedline-ai-byte-to-grapheme $item.text $item.bytes) != $item.cursor { error make {msg: "wrong graphemes"} } }; print PASS')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('PASS', result.stdout)
        result = self.run_nu('reedline-ai-byte-to-grapheme "echo é" 6')
        self.assertNotEqual(result.returncode, 0)

    def test_response_guard_and_native_binding_preservation(self):
        code = '''let r = {protocol: 1, request_id: x, shell: {kind: nushell}, cwd: /tmp, buffer: 'git  --short', cursor: 4}
        let s = {protocol: 1, request_id: x, buffer: 'git  --short', cursor: 4, insertion: status, result: 'git status --short', result_cursor: 10, additional_acceptance_required: false, risk_review: {warnings: [], incomplete: false}}
        if not (reedline-ai-check-response $r $s) { error make {msg: 'good response rejected'} }
        for bad in [($s | upsert request_id other) ($s | upsert result overwritten) ($s | upsert result_cursor 9) ($s | reject risk_review)] {
            if (reedline-ai-check-response $r $bad) { error make {msg: 'bad response accepted'} }
        }
        let before = $env.config.keybindings
        $env.REEDLINE_AI_EXECUTABLE = (which python3 | first | get path)
        reedline-ai-enable
        reedline-ai-disable
        if $before != $env.config.keybindings { error make {msg: 'native bindings changed'} }
        reedline-ai-enable
        $env.config.keybindings = ($env.config.keybindings | each {|binding| if $binding.name == reedline_ai_request { $binding | upsert event {send: left} } else { $binding } })
        reedline-ai-disable
        if not ($env.config.keybindings | any {|binding| $binding.name == reedline_ai_request and $binding.event.send == left }) { error make {msg: 'later user rebind was removed'} }
        print PASS'''
        result = self.run_nu(code)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('PASS', result.stdout)


@unittest.skipUnless(NU and os.name == 'posix', 'Real native test needs Nushell and POSIX PTY')
class NativeNuTests(unittest.TestCase):
    def setUp(self):
        import fcntl
        import pty
        import struct
        import termios
        self.temp = tempfile.TemporaryDirectory()
        folder = Path(self.temp.name)
        self.count = folder / 'count'
        self.pidfile = folder / 'pids'
        fixture = folder / 'fixture'
        fixture.write_text(f'''#!{sys.executable}
import json,sys,time,pathlib,os
assert sys.argv[1:] == ['--adapter-request']
r=json.load(sys.stdin)
with pathlib.Path({str(self.count)!r}).open('a') as f: f.write('1\\n')
pathlib.Path({str(self.pidfile)!r}).write_text(f'{{os.getppid()}} {{os.getpid()}}')
if r['buffer'].startswith('slow '): time.sleep(.8)
if r['buffer'].startswith('timeout '): time.sleep(30)
i='界' if '世' in r['buffer'] else 'status'
b=r['buffer'].encode(); c=r['cursor']
s=dict(protocol=1,request_id=r['request_id'],buffer=r['buffer'],cursor=c,insertion=i,result=(b[:c]+i.encode()+b[c:]).decode(),result_cursor=c+len(i.encode()),additional_acceptance_required=False,risk_review=dict(incomplete=False,warnings=[]))
if r['buffer'].startswith('risk '): s['risk_review']['incomplete']=True
if r['buffer'].startswith('malformed '): s['request_id']='wrong'
if r['buffer'].startswith('flood '): print('x' * 1000000); raise SystemExit(0)
print(json.dumps(s))
''')
        fixture.chmod(0o700)
        setup = folder / 'setup.nu'
        setup.write_text(f'''source {json.dumps(str(ADAPTER), ensure_ascii=False)}
$env.REEDLINE_AI_EXECUTABLE = '{fixture}'
$env.config.show_banner = false
$env.PROMPT_COMMAND = {{|| 'NU> '}}
$env.PROMPT_COMMAND_RIGHT = {{|| ''}}
reedline-ai-enable
$env.config.keybindings ++= [{{name: inspect_test, modifier: control_alt, keycode: char_q, mode: [emacs], event: {{send: executehostcommand, cmd: 'print $"SNAP:(commandline | to json --raw):(commandline get-cursor)"; $env.REEDLINE_AI_HOST_EVENT = true'}}}}]
''')
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 120, 0, 0))
        def terminal():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        self.process = subprocess.Popen([NU, '-n', '-e', f'source {json.dumps(str(setup), ensure_ascii=False)}'], stdin=slave, stdout=slave, stderr=slave,
                                       env=dict(os.environ, TERM='xterm-256color', XDG_CONFIG_HOME=str(folder), HOME=str(folder)), preexec_fn=terminal)
        os.close(slave)
        self.transcript = bytearray()
        self.pending_bytes = bytearray()
        self.pump(.4)
        self.assertNotIn(b'Error:', self.transcript)

    def tearDown(self):
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=3)
        os.close(self.master)
        self.temp.cleanup()

    def pump(self, seconds=.10):
        import select
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([self.master], [], [], max(0, deadline - time.monotonic()))[0]:
                try: chunk = os.read(self.master, 65536)
                except OSError: return
                if not chunk: return
                self.transcript.extend(chunk)
                self.pending_bytes.extend(chunk)
                while b'\x1b[6n' in self.pending_bytes:
                    index = self.pending_bytes.index(b'\x1b[6n')
                    del self.pending_bytes[:index + 4]
                    os.write(self.master, b'\x1b[1;1R')
                if len(self.pending_bytes) > 1000: del self.pending_bytes[:-10]

    def send(self, data, wait=.25):
        os.write(self.master, data.encode() if isinstance(data, str) else data)
        self.pump(wait)

    def key(self, name, wait=.25):
        sequence = {'request': b'\x1b\x07', 'preview': b'\x1b\x10', 'accept': b'\x1b\x01', 'cancel': b'\x1b\x18'}[name]
        start = len(self.transcript)
        self.send(sequence, wait)
        # Readiness is explicit in the real UI. Do not assume Python/Rust starts
        # within 400ms when other test/build jobs share this machine.
        deadline = time.monotonic() + 4
        while name == 'preview' and b'still pending' in self.transcript[start:] and time.monotonic() < deadline:
            self.pump(.1)
            start = len(self.transcript)
            self.send(sequence, .25)

    def snapshot(self):
        start = len(self.transcript)
        self.send(b'\x1b\x11')
        deadline = time.monotonic() + 2
        while True:
            text = self.transcript[start:].decode('utf-8', errors='replace')
            match = re.search(r'SNAP:("[^\r\n]*"):(\d+)\r?\n', text)
            if match or time.monotonic() >= deadline: break
            self.pump(.1)
        self.assertIsNotNone(match, text)
        return json.loads(match[1]), int(match[2])

    def count_requests(self):
        return len(self.count.read_text().splitlines()) if self.count.exists() else 0

    def test_blank_and_typing_do_not_start_requests(self):
        self.key('request'); self.send('   '); self.key('request'); self.send('echo data'); self.pump(.3)
        self.assertEqual(self.count_requests(), 0)

    def test_middle_preview_then_accept_without_execution(self):
        self.send('git  --short'); self.send(b'\x01' + b'\x1b[C' * 4)
        self.key('request', .4); self.key('accept')
        self.assertEqual(self.snapshot(), ('git  --short', 4))
        self.key('preview'); self.key('accept')
        self.assertEqual(self.snapshot(), ('git status --short', 10))
        self.key('accept')
        self.assertEqual(self.snapshot(), ('git status --short', 10))
        self.assertEqual(self.count_requests(), 1)

    def test_unicode_middle_byte_conversion(self):
        self.send('echo 世 suffix'); self.send(b'\x01' + b'\x1b[C' * 6)
        self.key('request', .4); self.key('preview'); self.key('accept')
        self.assertEqual(self.snapshot(), ('echo 世界 suffix', 7), bytes(self.transcript).decode(errors='replace'))

    @unittest.skipUnless(os.environ.get('REEDLINE_AI_TEST_BINARY'), 'Set REEDLINE_AI_TEST_BINARY for the real Rust mock backend')
    def test_real_rust_mock_backend_and_risk_gate(self):
        binary = json.dumps(str(Path(os.environ['REEDLINE_AI_TEST_BINARY']).resolve()), ensure_ascii=False)
        self.send(f'reedline-ai-disable; $env.REEDLINE_AI_EXECUTABLE = {binary}; reedline-ai-enable\r', .3)
        self.send('echo 世'); self.key('request', .5); self.key('preview'); self.key('accept')
        self.assertEqual(self.snapshot(), ('echo 世界', 7))
        self.send(b'\x03'); self.send('git st'); self.key('request', .5); self.key('preview'); self.key('accept')
        self.assertEqual(self.snapshot(), ('git st', 6))
        self.assertIn(b'acceptance blocked by risk review', self.transcript)
        self.assertEqual(self.count_requests(), 0)

    def test_typing_during_pending_invalidates_result(self):
        self.send('slow '); self.key('request', .05); self.send('edited', .05)
        self.assertEqual(self.snapshot(), ('slow edited', 11))
        self.pump(1); self.key('preview'); self.key('accept')
        self.assertEqual(self.snapshot(), ('slow edited', 11))

    def test_cancel_keeps_single_draining_worker(self):
        self.send('slow '); self.key('request', .15); self.key('cancel', .15)
        self.key('request', .15)
        self.assertEqual(self.count_requests(), 1)
        self.assertIn(b'one bounded request is still draining', self.transcript)
        self.pump(1)
        self.key('request', 1.3); self.key('preview'); self.key('accept')
        self.assertEqual(self.count_requests(), 2)
        self.assertEqual(self.snapshot(), ('slow status', 11))

    @unittest.skipUnless(sys.platform.startswith('linux'), 'Linux /proc verifies process cleanup')
    def test_native_exit_kills_helper_supervisor_and_backend(self):
        self.send('timeout '); self.key('request', .3)
        deadline = time.monotonic() + 3
        while not self.pidfile.exists() and time.monotonic() < deadline:
            self.pump(.1)
        self.assertTrue(self.pidfile.exists())
        supervisor, backend = map(int, self.pidfile.read_text().split())
        helper = int(Path(f'/proc/{supervisor}/stat').read_text().split()[3])
        self.send(b'\x03'); self.send('exit\r', .2)
        if self.process.poll() is None: self.send('exit\r', .3)
        self.process.wait(timeout=3)
        def alive(pid):
            path = Path(f'/proc/{pid}/stat')
            return path.exists() and path.read_text().split()[2] != 'Z'
        deadline = time.monotonic() + 2
        while any(alive(pid) for pid in (helper, supervisor, backend)) and time.monotonic() < deadline:
            time.sleep(.02)
        self.assertFalse(any(alive(pid) for pid in (helper, supervisor, backend)))

    def test_cursor_change_invalidates_preview(self):
        self.send('git '); self.key('request', .4); self.key('preview')
        self.send(b'\x1b[D'); self.key('accept')
        self.assertEqual(self.snapshot(), ('git ', 3))

    def test_cancel_and_ctrl_c_new_prompt_do_not_revive_identical_text(self):
        for method in ('cancel', 'ctrl-c'):
            self.send('git '); self.key('request', .4); self.key('preview')
            if method == 'cancel': self.key('cancel')
            else: self.send(b'\x03'); self.send('git ')
            self.key('accept')
            self.assertEqual(self.snapshot(), ('git ', 4))
            self.send(b'\x03')

    def test_risk_malformed_and_flood_fail_closed(self):
        for prefix in ('risk ', 'malformed ', 'flood '):
            self.send(prefix); self.key('request', .4); self.key('preview'); self.key('accept')
            self.assertEqual(self.snapshot(), (prefix, len(prefix)))
            self.send(b'\x03')

    def test_timeout_leaves_editor_usable(self):
        self.send('timeout '); self.key('request', .3); self.pump(16.7)
        self.key('preview'); self.key('accept')
        self.assertEqual(self.snapshot(), ('timeout ', 8))
        self.assertIn(b'failed, timed out, or returned an invalid response', self.transcript)

    def test_native_tab_history_and_editing_remain_available(self):
        self.send('echo native-history-marker\r', .2)
        self.send(b'\x1b[A')
        self.assertEqual(self.snapshot()[0], 'echo native-history-marker')
        self.send(b'\x03'); self.send('str len'); self.send(b'\t', .2)
        # Tab is still the native completion menu; escape returns to editing.
        self.send(b'\x1b'); self.send(b'\x01'); self.send('x')
        self.assertTrue(self.snapshot()[0].startswith('xstr len'))
        self.assertEqual(self.count_requests(), 0)


if __name__ == '__main__': unittest.main()
