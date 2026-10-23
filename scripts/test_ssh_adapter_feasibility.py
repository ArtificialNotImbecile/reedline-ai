"""Offline SSH adapter contract tests; no SSH daemon, credentials, or sockets."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

MODULE = Path(__file__).resolve().parents[1] / "experiments/ssh_adapter/feasibility.py"
spec = importlib.util.spec_from_file_location("ssh_feasibility", MODULE)
f = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = f
spec.loader.exec_module(f)


class StaticTransport:
    def __init__(self, payload):
        self.payload = payload
        self.closed = False
        self.calls = 0

    def probe(self):
        self.calls += 1
        if isinstance(self.payload, Exception):
            raise self.payload
        return self.payload

    def close(self):
        self.closed = True


class SSHContractTests(unittest.TestCase):
    def test_actual_helper_facts_in_synthetic_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            cwd = Path(tmp) / "long path ' ; $(touch SHOULD_NOT_EXIST) ü"
            cwd.mkdir()
            transport, relay = f.MockTransport(cwd), f.SyntheticRelay()
            result = f.validate_session(transport, relay)
            self.assertEqual(result.mode, "enhanced-contract")
            self.assertEqual(result.facts["cwd"], str(cwd.resolve()))
            self.assertEqual(result.facts["adapter_shell"], "/bin/sh")
            self.assertIsNone(result.facts["login_shell"])
            self.assertTrue(Path(result.facts["interpreter"]).is_file())
            self.assertEqual(list(cwd.iterdir()), [])
            self.assertTrue(transport.closed)
            self.assertFalse(relay.active)
            self.assertEqual(relay.requests, 1)
            # Parse generated path as DATA, never execute the suggestion.
            import shlex
            self.assertEqual(shlex.split(result.candidate), ["cd", "--", str(cwd.resolve())])
            with self.assertRaises(ValueError):
                relay.suggest(result.facts)
            with self.assertRaises(ValueError):
                transport.probe()

    def test_plan_does_not_override_tools_or_forward_authentication(self):
        plan = f.ssh_plan("test-user@fixture.invalid")
        self.assertEqual(plan[0], "ssh")
        self.assertIn("-a", plan)
        self.assertIn("StrictHostKeyChecking=yes", plan)
        self.assertEqual(plan[-1], f.remote_command())
        self.assertNotIn("-R", plan)
        self.assertEqual(f.ssh_plan("fixture.invalid", False)[-1], "fixture.invalid")
        self.assertNotIn("-T", f.ssh_plan("fixture.invalid", False))

    def test_destination_injection_rejected(self):
        for value in ["-oProxyCommand=touch", "x;touch PWNED", "x\nfoo", "x $(id)", "", "x/../y"]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                f.ssh_plan(value)

    def test_policy_failure_falls_back_without_probe(self):
        for option in [{"forwarding_denied": True}, {"supported_shell": False}]:
            transport, relay = StaticTransport(b"unused"), f.SyntheticRelay()
            result = f.validate_session(transport, relay, **option)
            self.assertEqual(result.mode, "normal-shell")
            self.assertEqual(transport.calls, 0)
            self.assertEqual(relay.requests, 0)
            self.assertTrue(transport.closed)
            self.assertFalse(relay.active)

    def test_helper_errors_fallback_and_close(self):
        for payload in [b"bad JSON", b"{}", b"[]", b"x" * (f.MAX_FRAME + 1), b"[" * 2000 + b"]" * 2000,
                        subprocess.TimeoutExpired("synthetic", 3), OSError("synthetic missing interpreter")]:
            transport, relay = StaticTransport(payload), f.SyntheticRelay()
            result = f.validate_session(transport, relay)
            self.assertEqual(result.mode, "normal-shell")
            self.assertTrue(transport.closed)
            self.assertFalse(relay.active)
            self.assertEqual(relay.requests, 0)

    def test_bad_shell_and_path_facts_are_rejected(self):
        good = {"version": 1, "cwd": "/synthetic", "adapter_shell": "/bin/sh",
                "interpreter": "/usr/bin/python3", "login_shell": None}
        for name, value in [("version", True), ("cwd", "guessed/relative"), ("cwd", "/bad\npath"),
                            ("cwd", None), ("adapter_shell", "/bin/fish"), ("login_shell", "/bin/bash")]:
            with self.subTest(name=name, value=value), self.assertRaises(ValueError):
                f.parse_facts(json.dumps({**good, name: value}).encode())

    def test_optional_forward_plan_rejects_public_binds_and_bad_ports(self):
        self.assertEqual(f.loopback_forward_plan("127.0.0.1", 43111, 43112)[-1],
                         "127.0.0.1:43111:127.0.0.1:43112")
        for host in ["0.0.0.0", "*", "", "localhost", "::"]:
            with self.assertRaises(ValueError):
                f.loopback_forward_plan(host, 43111, 43112)
        for port in [0, 22, 65536, True, "43111"]:
            with self.assertRaises(ValueError):
                f.loopback_forward_plan("127.0.0.1", port, 43112)

    def test_relay_failure_closes_session(self):
        with tempfile.TemporaryDirectory() as tmp:
            relay = f.SyntheticRelay()
            relay.close()
            transport = f.MockTransport(tmp)
            self.assertEqual(f.validate_session(transport, relay).mode, "normal-shell")
            self.assertTrue(transport.closed)
        class BrokenClose(StaticTransport):
            def close(self):
                raise OSError("synthetic teardown failure")
        relay = f.SyntheticRelay()
        with self.assertRaises(OSError):
            f.validate_session(BrokenClose(b"{}"), relay)
        self.assertFalse(relay.active)


if __name__ == "__main__":
    unittest.main()
