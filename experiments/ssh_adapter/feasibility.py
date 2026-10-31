#!/usr/bin/env python3
"""Offline contract experiment. Never invokes ssh or connects to a host.

The mock executes a fixed helper in a synthetic local cwd, through a POSIX
shell, to exercise the remote-command string boundary used by OpenSSH.
This is NOT an SSH, terminal integration, or TCP forwarding implementation.
"""
from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile

MAX_FRAME = 8192
# In-memory helper: no installation, startup-file edits, history/file reads,
# model endpoint, credentials, or execution of generated suggestions.
HELPER = r'''
import json, os, subprocess, sys
shell = "/bin/sh"
probe = subprocess.run([shell, "-c", 'printf "%s" "$0"'],
                       capture_output=True, timeout=2, check=True)
print(json.dumps({"version": 1, "cwd": os.getcwd(),
                  "adapter_shell": probe.stdout.decode("utf-8"),
                  "interpreter": sys.executable,
                  "login_shell": None}))
'''


def remote_command():
    # One fixed remote command string. OpenSSH joins command arguments with
    # spaces; a local argv list alone would NOT protect the remote shell.
    return "python3 -c " + shlex.quote(HELPER)


def destination(value):
    # Deliberately narrow experiment grammar. No flags, URI, ProxyCommand,
    # IPv6, whitespace, shell metacharacters, or arbitrary SSH options.
    if not re.fullmatch(r"(?:[A-Za-z0-9_][A-Za-z0-9_.-]*@)?[A-Za-z0-9][A-Za-z0-9.-]*", value):
        raise ValueError("unsupported destination")
    return value


def ssh_plan(host, enhanced=True):
    """Return a plan only; this module never executes this argv."""
    argv = ["ssh", "-F", "none", "-a", "-x", "-o", "ClearAllForwardings=yes",
            "-o", "StrictHostKeyChecking=yes", "-o", "ControlMaster=no",
            "-o", "ControlPath=none", "-o", "PermitLocalCommand=no"]
    if enhanced:
        argv += ["-T"]
    argv += [destination(host)]
    if enhanced:
        argv += [remote_command()]
    return argv


@dataclass(frozen=True)
class Result:
    mode: str
    reason: str
    facts: dict | None = None
    candidate: str | None = None


def parse_facts(raw):
    if len(raw) > MAX_FRAME:
        raise ValueError("oversized frame")
    facts = json.loads(raw)
    if not isinstance(facts, dict) or set(facts) != {
        "version", "cwd", "adapter_shell", "interpreter", "login_shell"
    }:
        raise ValueError("invalid facts")
    if type(facts["version"]) is not int or facts["version"] != 1:
        raise ValueError("unsupported protocol")
    for name in ("cwd", "adapter_shell", "interpreter"):
        value = facts[name]
        if not isinstance(value, str) or not value.startswith("/") or any(ord(c) < 32 for c in value):
            raise ValueError("invalid path fact")
    if facts["adapter_shell"] != "/bin/sh" or facts["login_shell"] is not None:
        raise ValueError("unsupported shell facts")
    return facts


class SyntheticRelay:
    """Company-side in-process fixture, not a network service or real API.

    No credential is needed or copied. Only a path reported by the helper
    may be used; no guessed long paths, filesystem search, or local history.
    """
    def __init__(self):
        self.active = True
        self.requests = 0

    def suggest(self, facts):
        if not self.active:
            raise ValueError("session closed")
        self.requests += 1
        return "cd -- " + shlex.quote(facts["cwd"])

    def close(self):
        self.active = False


class MockTransport:
    """Not SSH: executes the fixed helper locally with a clean environment.

    No supplied host, command, script, or ambient environment is executed.
    The caller owns the synthetic cwd, which must not contain user data.
    """
    def __init__(self, cwd):
        self.cwd = cwd
        self.closed = False
        self.calls = 0

    def probe(self):
        if self.closed:
            raise ValueError("transport closed")
        self.calls += 1
        return subprocess.run(
            ["/bin/sh", "-c", remote_command()], cwd=self.cwd,
            env={"PATH": str(Path(sys.executable).parent) + ":/usr/bin:/bin", "HOME": str(self.cwd), "LC_ALL": "C.UTF-8"},
            capture_output=True, timeout=3, check=True,
        ).stdout

    def close(self):
        self.closed = True


def validate_session(transport, relay, *, supported_shell=True, forwarding_denied=False):
    """Exercise state contract and teardown, NOT a login/fallback connection.

    A forwarding denial is a synthetic policy event for a future optional
    forwarding route. Do not bypass it by switching routes automatically.
    The current executable fixture uses stdio, not port forwarding.
    """
    try:
        if forwarding_denied:
            return Result("normal-shell", "forwarding denied; no retry or alternate route")
        if not supported_shell:
            return Result("normal-shell", "unsupported shell; helper not started")
        facts = parse_facts(transport.probe())
        return Result("enhanced-contract", "mock transport only", facts, relay.suggest(facts))
    except (ValueError, OSError, subprocess.SubprocessError, UnicodeError, RecursionError):
        return Result("normal-shell", "helper or relay unavailable; details not logged")
    finally:
        try:
            transport.close()
        finally:
            relay.close()


def loopback_forward_plan(bind_host, remote_port, company_port):
    """Optional future OpenSSH -R spec, never executed by this harness.

    Server GatewayPorts policy can override binding behavior: string validation
    alone DOES NOT prove a real server listener is loopback-only.
    """
    if bind_host != "127.0.0.1":
        raise ValueError("only explicit IPv4 loopback allowed")
    if any(type(port) is not int or not 1024 <= port <= 65535 for port in (remote_port, company_port)):
        raise ValueError("invalid unprivileged port")
    return ["-o", "ExitOnForwardFailure=yes", "-R",
            f"127.0.0.1:{remote_port}:127.0.0.1:{company_port}"]


def main():
    with tempfile.TemporaryDirectory(prefix="reedline-ssh-contract-") as tmp:
        cwd = Path(tmp) / "synthetic project ' spaced ; $(not-a-command)"
        cwd.mkdir()
        result = validate_session(MockTransport(cwd), SyntheticRelay())
        print(json.dumps({"validation": "LOCAL MOCK ONLY; no SSH connection", **result.__dict__}, indent=2))


if __name__ == "__main__":
    main()
