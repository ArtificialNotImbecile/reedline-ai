"""Static/unit tests only. Never starts Docker/SSH or creates authentication."""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
HERE = ROOT / "experiments/ssh_adapter/docker_fixture"
spec = importlib.util.spec_from_file_location("docker_ssh_fixture_runner", HERE / "run.py")
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class DockerSSHFixtureTests(unittest.TestCase):
    def test_remote_docker_override_rejected_without_contact(self):
        with patch.dict(os.environ, {"DOCKER_HOST": "ssh://remote"}, clear=True), patch.object(m, "run") as run:
            with self.assertRaises(ValueError):
                m.require_local_docker()
            run.assert_not_called()

    def test_context_takes_precedence_over_host_override(self):
        with patch.dict(os.environ, {"DOCKER_HOST": "unix:///local", "DOCKER_CONTEXT": "remote"}, clear=True):
            with patch.object(m, "run", return_value=subprocess.CompletedProcess([], 0, "tcp://remote:2376\n", "")) as run:
                with self.assertRaises(ValueError):
                    m.require_local_docker()
                self.assertIn("remote", run.call_args.args[0])

    def test_local_docker_override_accepted(self):
        with patch.dict(os.environ, {"DOCKER_HOST": "unix:///run/user/1000/docker.sock"}, clear=True):
            with patch.object(m, "run") as run:
                m.require_local_docker()
                run.assert_not_called()

    def test_default_is_inert_help(self):
        with patch.object(m, "run") as run, contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(m.main([]), 0)
            run.assert_not_called()

    def test_port_parser_requires_exact_loopback_binding(self):
        self.assertEqual(m.published_port("127.0.0.1:43123\n"), 43123)
        for value in ["0.0.0.0:43123", "[::]:43123", "localhost:43123", "127.0.0.1:22",
                      "127.0.0.1:99999", "127.0.0.1:43123\n0.0.0.0:43123", ""]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                m.published_port(value)

    def test_client_pins_identity_and_ignores_ambient_auth(self):
        args = m.ssh_args(43123, Path("/synthetic/client"), Path("/synthetic/hosts"))
        for value in ["StrictHostKeyChecking=yes", "IdentityAgent=none", "IdentitiesOnly=yes",
                      "BatchMode=yes", "PermitLocalCommand=no", "ControlMaster=no",
                      "GlobalKnownHostsFile=/dev/null", "UserKnownHostsFile=/synthetic/hosts"]:
            self.assertIn(value, args)
        self.assertEqual(args[:3], ["ssh", "-F", "none"])
        for port in [True, 22, 65536, "43123"]:
            with self.assertRaises(ValueError):
                m.ssh_args(port, Path("a"), Path("b"))

    def test_server_disables_password_and_forwarding(self):
        config = (HERE / "sshd_config").read_text()
        for directive in ["PasswordAuthentication no", "PermitEmptyPasswords no",
                          "PermitRootLogin no", "AllowTcpForwarding no",
                          "AllowAgentForwarding no", "PermitUserRC no"]:
            self.assertIn(directive, config.splitlines())

    def test_cleanup_attempts_image_after_container_error(self):
        calls, errors, checks = [], [], []
        def fake(argv, **kw):
            calls.append(argv)
            if argv[1] == "rm":
                raise subprocess.TimeoutExpired(argv, 30)
            return subprocess.CompletedProcess(argv, 0, "", "")
        with patch.object(m, "run", side_effect=fake):
            m.cleanup("test", "tag", True, True, checks, errors)
        self.assertEqual(calls[-1], ["docker", "image", "rm", "tag"])
        self.assertIn("container cleanup failed: test", errors)

    def test_cleanup_requires_absence(self):
        checks, errors = [], []
        with patch.object(m, "run", side_effect=[
            subprocess.CompletedProcess([], 0, "", ""),
            subprocess.CompletedProcess([], 0, "still-present\n", ""),
        ]):
            m.cleanup("test", "tag", True, False, checks, errors)
        self.assertFalse(checks)
        self.assertIn("container cleanup unconfirmed: test", errors)

    def test_no_cleanup_before_resources_attempted(self):
        with patch.object(m, "run") as run:
            m.cleanup("test", "tag", False, False, [], [])
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
