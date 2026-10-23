"""Portable transport tests. Windows ACL/Clink terminal tests remain separate."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

SPEC = importlib.util.spec_from_file_location('clink_bridge', os.environ.get('RAI_SHARED_BRIDGE_PATH', Path(__file__).with_name('bridge.py')))
bridge = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bridge)


def request(text='echo 世 suffix', cursor=8):
    return dict(protocol=1, request_id='fixture-1', shell=dict(kind='cmd', version='Clink fixture'), cwd='/synthetic', buffer=text, cursor=cursor)


def response(r, insertion='界'):
    b = r['buffer'].encode(); c = r['cursor']; i = insertion.encode()
    return dict(protocol=1, request_id=r['request_id'], buffer=r['buffer'], cursor=c, insertion=insertion,
                result=(b[:c] + i + b[c:]).decode(), result_cursor=c + len(i),
                risk_review=dict(warnings=[], incomplete=False), additional_acceptance_required=False)


def process_running(pid):
    # Linux can retain a dead orphan as a zombie until PID 1 reaps it.
    status = Path(f'/proc/{pid}/stat')
    if status.exists() and status.read_text().split()[2] == 'Z':
        return False
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


class BridgeTests(unittest.TestCase):
    def test_unicode_fim_and_frame(self):
        r = request(); s = response(r)
        self.assertFalse(bridge.check_response(r, s)[1])
        self.assertEqual(s['result'], 'echo 世界 suffix')
        fields = bridge.response_frame(r, s).splitlines()
        self.assertEqual(bytes.fromhex(fields[5]).decode(), s['result'])
        self.assertEqual(fields[6], '11')

    def test_risk_fail_closed(self):
        r = request()
        for change in ('additional_acceptance_required', 'warnings', 'incomplete'):
            s = response(r)
            if change == 'warnings': s['risk_review'][change] = ['warning']
            elif change == 'incomplete': s['risk_review'][change] = True
            else: s[change] = True
            self.assertTrue(bridge.check_response(r, s)[1])
        s = response(r); del s['risk_review']
        with self.assertRaises(ValueError): bridge.check_response(r, s)

    def test_mismatch_control_and_cursor_rejected(self):
        r = request()
        for field, value in [('request_id', 'other'), ('buffer', 'other'), ('cursor', True),
                             ('result_cursor', 10), ('result', 'rewritten'), ('insertion', '\x1b[31m'),
                             ('result', 'echo\u202eevil'), ('additional_acceptance_required', 0)]:
            s = response(r); s[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError): bridge.check_response(r, s)
        with self.assertRaises(ValueError): bridge.check_request(request(cursor=6))
        with self.assertRaises(ValueError): bridge.check_request(request('line\nother', 4))

    @unittest.skipIf(os.name == 'nt', 'POSIX permission test, Windows requires native DACL validation')
    def test_private_directory_permissions_and_symlink_rejection(self):
        d = bridge.prepare_directory()
        try:
            self.assertEqual(d.stat().st_mode & 0o777, 0o700)
            p = d / 'request.json'; p.write_text('{}'); p.chmod(0o644)
            with self.assertRaises(ValueError): bridge.read_envelope(p)
            p.unlink(); p.symlink_to('/dev/null')
            with self.assertRaises(ValueError): bridge.read_envelope(p)
            p.unlink()
        finally: d.rmdir()

    def test_backend_stdin_not_shell_eval_and_timeout(self):
        with tempfile.TemporaryDirectory() as folder:
            d = Path(folder)
            fixture = d / 'backend.py'
            fixture.write_text('''import json, sys, time
r=json.load(sys.stdin)
assert sys.argv[1:] == ['--adapter-request']
i='suffix'; b=r['buffer'].encode(); c=r['cursor']
print(json.dumps(dict(protocol=1,request_id=r['request_id'],buffer=r['buffer'],cursor=c,insertion=i,result=(b[:c]+i.encode()+b[c:]).decode(),result_cursor=c+len(i),risk_review=dict(warnings=[],incomplete=False),additional_acceptance_required=False)))
''')
            r = request('echo %PATH% !x! & | < > ^ "quoted" $(noexec)', 5)
            envelope = dict(backend=sys.executable, arguments=[str(fixture)], request=r)
            s = bridge.run_backend(envelope)
            self.assertTrue(s['result'].startswith('echo suffix%PATH%'))
            fixture.write_text('import time; time.sleep(10)')
            start = time.monotonic()
            with self.assertRaises(subprocess.TimeoutExpired): bridge.run_backend(envelope, timeout=.15)
            self.assertLess(time.monotonic() - start, 2)
            envelope['request'] = request('  ', 2)
            with self.assertRaises(ValueError): bridge.run_backend(envelope)

    def test_output_budget(self):
        with tempfile.TemporaryDirectory() as folder:
            fixture = Path(folder) / 'oversized.py'
            fixture.write_text('import sys; sys.stdin.read(); print("x" * 1000000)')
            with self.assertRaises(ValueError):
                bridge.run_backend(dict(backend=sys.executable, arguments=[str(fixture)], request=request()), timeout=2)

    def test_cli_cleanup_and_private_errors(self):
        directory = bridge.prepare_directory()
        p = directory / 'request.json'
        r = request('echo synthetic-secret', 5)
        p.write_text(json.dumps(dict(backend=sys.executable, arguments=['-c', 'import sys; sys.stdin.read(); raise SystemExit(3)'], request=r)))
        if os.name != 'nt': p.chmod(0o600)
        result = subprocess.run([sys.executable, str(Path(bridge.__file__)), '--run', str(p)], capture_output=True, timeout=3)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b'ERROR\n')
        self.assertNotIn(b'synthetic-secret', result.stderr)
        self.assertFalse(directory.exists())

    @unittest.skipIf(os.name == 'nt', 'POSIX process-group containment regression')
    def test_descendant_held_stdout_has_one_deadline(self):
        with tempfile.TemporaryDirectory() as folder:
            pidfile = Path(folder) / 'pid'
            source = ('import subprocess,sys,pathlib; sys.stdin.read(); '
                      f'p=subprocess.Popen([sys.executable,"-c","import time;time.sleep(30)"]); pathlib.Path({str(pidfile)!r}).write_text(str(p.pid))')
            envelope = dict(backend=sys.executable, arguments=['-c', source], request=request())
            started = time.monotonic()
            with self.assertRaises(subprocess.TimeoutExpired):
                bridge.run_backend(envelope, timeout=.25)
            self.assertLess(time.monotonic() - started, 1.5)
            pid = int(pidfile.read_text())
            self.assertFalse(process_running(pid))

    @unittest.skipIf(os.name == 'nt', 'POSIX signal and direct-child SIGKILL regression')
    def test_helper_death_cleans_backend_and_descendants(self):
        for sig in (signal.SIGHUP, signal.SIGTERM, signal.SIGKILL):
            with self.subTest(signal=sig), tempfile.TemporaryDirectory() as folder:
                pidfile = Path(folder) / 'pids'
                source = ('import subprocess,sys,pathlib,os,time; sys.stdin.read(); '
                          'p=subprocess.Popen([sys.executable,"-c","import time;time.sleep(30)"]); '
                          f'pathlib.Path({str(pidfile)!r}).write_text(f"{{os.getppid()}} {{os.getpid()}} {{p.pid}}"); time.sleep(30)')
                envelope = dict(backend=sys.executable, arguments=['-c', source], request=request())
                process = subprocess.Popen([sys.executable, str(Path(bridge.__file__)), '--stdin'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                process.stdin.write(json.dumps(envelope).encode()); process.stdin.close(); process.stdin = None
                deadline = time.monotonic() + 3
                while not pidfile.exists() and time.monotonic() < deadline:
                    time.sleep(.01)
                self.assertTrue(pidfile.exists())
                pids = [int(pid) for pid in pidfile.read_text().split()]
                started = time.monotonic()
                process.send_signal(sig)
                stdout, stderr = process.communicate(timeout=2)
                self.assertLess(time.monotonic() - started, 1.5)
                deadline = time.monotonic() + 1
                while any(process_running(pid) for pid in pids) and time.monotonic() < deadline:
                    time.sleep(.01)
                self.assertFalse(any(process_running(pid) for pid in pids), pids)
                self.assertNotIn(b'echo', stdout + stderr)

    def test_clink_stdout_contains_only_status_not_private_result(self):
        directory = bridge.prepare_directory()
        path = directory / 'request.json'
        r = request('echo private-fixture', 5)
        source = 'import json,sys;json.load(sys.stdin);print(' + repr(json.dumps(response(r))) + ')'
        path.write_text(json.dumps(dict(backend=sys.executable, arguments=['-c', source], request=r)))
        if os.name != 'nt': path.chmod(0o600)
        try:
            result = subprocess.run([sys.executable, str(Path(bridge.__file__)), '--run', str(path)], capture_output=True, timeout=3)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, b'READY\n')
            self.assertFalse(path.exists())
            frame = directory / 'response.frame'
            self.assertEqual(frame.read_text(), bridge.response_frame(r, response(r)))
            if os.name != 'nt': self.assertEqual(frame.stat().st_mode & 0o777, 0o600)
        finally:
            (directory / 'response.frame').unlink(missing_ok=True)
            path.unlink(missing_ok=True)
            directory.rmdir()


if __name__ == '__main__':
    unittest.main()
