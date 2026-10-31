#!/usr/bin/env python3
"""Opt-in real OpenSSH/Docker test; never invoked by aggregate local CI.

Creates disposable authentication and a container only after explicit --run.
No paid API, real history, generated-command execution, or remote production host.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("docker_fixture_feasibility", HERE.parent / "feasibility.py")
f = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = f
spec.loader.exec_module(f)


def run(argv, *, timeout=30, check=True):
    return subprocess.run(argv, capture_output=True, text=True, timeout=timeout, check=check)


def published_port(raw):
    # Reject any wildcard or additional binding rather than quietly accepting it.
    match = re.fullmatch(r"127\.0\.0\.1:([0-9]+)\s*", raw)
    if not match or not 1024 <= int(match[1]) <= 65535:
        raise ValueError("Docker did not publish one loopback-only high port")
    return int(match[1])


def require_local_docker():
    # DOCKER_CONTEXT overrides DOCKER_HOST in the Docker CLI.
    context = os.environ.get("DOCKER_CONTEXT")
    endpoint = os.environ.get("DOCKER_HOST") if not context else None
    if not endpoint:
        if not context:
            context = run(["docker", "context", "show"]).stdout.strip()
        endpoint = run(["docker", "context", "inspect", context,
                        "--format", "{{.Endpoints.docker.Host}}"]).stdout.strip()
    if not endpoint.startswith("unix:///"):
        raise ValueError("fixture requires local Unix-socket Docker; remote contexts are not allowed")


def ssh_args(port, key, hosts):
    if type(port) is not int or not 1024 <= port <= 65535:
        raise ValueError("invalid fixture port")
    return ["ssh", "-F", "none", "-a", "-x", "-T", "-p", str(port),
            "-i", str(key), "-o", "IdentitiesOnly=yes", "-o", "IdentityAgent=none",
            "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
            "-o", "UserKnownHostsFile=" + str(hosts), "-o", "GlobalKnownHostsFile=/dev/null",
            "-o", "ConnectTimeout=3", "-o", "ConnectionAttempts=1",
            "-o", "ControlMaster=no", "-o", "ControlPath=none",
            "-o", "PermitLocalCommand=no"]


def cleanup(name, tag, container_attempted, image_attempted, checks, errors):
    # Each cleanup is independent: a daemon failure must not skip the next attempt.
    if container_attempted:
        try:
            removed = run(["docker", "rm", "-f", name], check=False)
            remaining = run(["docker", "ps", "-aq", "--filter", "name=^/" + name + "$"], check=False)
            if remaining.returncode or remaining.stdout.strip():
                errors.append("container cleanup unconfirmed: " + name)
            elif removed.returncode == 0:
                checks.append("fixture container removed")
        except (OSError, subprocess.SubprocessError):
            errors.append("container cleanup failed: " + name)
    if image_attempted:
        try:
            if run(["docker", "image", "rm", tag], check=False).returncode:
                errors.append("test image cleanup unconfirmed: " + tag)
        except (OSError, subprocess.SubprocessError):
            errors.append("test image cleanup failed: " + tag)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="create ephemeral keys, build/run container, test and clean up")
    args = parser.parse_args(argv)
    if not args.run:
        parser.print_help()
        return 0
    for binary in ("docker", "ssh", "ssh-keygen"):
        if not shutil.which(binary):
            parser.error("missing prerequisite: " + binary)
    require_local_docker()
    run(["docker", "info"], timeout=15)
    suffix = uuid.uuid4().hex[:12]
    name, tag = "reedline-ssh-" + suffix, "reedline-ssh-fixture:" + suffix
    checks, errors = [], []
    image_attempted = container_attempted = False
    try:
        with tempfile.TemporaryDirectory(prefix="reedline-docker-ssh-") as tmp:
            root = Path(tmp)
            key, hosts = root / "client", root / "known_hosts"
            public = root / "public"
            public.mkdir()
            run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(key)])
            shutil.copyfile(str(key) + ".pub", public / "client.pub")
            # Mount only the public key; never the directory containing private auth.
            image_attempted = True
            run(["docker", "build", "-t", tag, str(HERE)], timeout=600)
            container_attempted = True
            run(["docker", "run", "-d", "--name", name,
                 "--publish", "127.0.0.1::22", "--mount",
                 "type=bind,src=" + str(public) + ",dst=/fixture,readonly", tag])
            port = published_port(run(["docker", "port", name, "22/tcp"]).stdout)
            checks.append("published IPv4 loopback only")
            for _ in range(30):
                hostkey = run(["docker", "exec", name, "cat", "/run/sshd/fixture_host_ed25519.pub"], check=False)
                if hostkey.returncode == 0:
                    break
                time.sleep(0.2)
            else:
                raise RuntimeError("fixture host key not ready")
            # Trust derives from the local Docker control plane, never ssh-keyscan or accept-new.
            fields = hostkey.stdout.split()
            if len(fields) < 2 or fields[0] != "ssh-ed25519":
                raise ValueError("unexpected fixture host identity")
            hosts.write_text(f"[127.0.0.1]:{port} {fields[0]} {fields[1]}\n")
            base = ssh_args(port, key, hosts)
            destination = "fixture@127.0.0.1"
            for _ in range(30):
                ready = run(base + [destination, "true"], check=False)
                if ready.returncode == 0:
                    break
                time.sleep(0.2)
            else:
                raise RuntimeError("SSH authentication/readiness failed")
            checks.append("real SSH authentication with pinned host key")
            raw = run(base + [destination, f.remote_command()]).stdout.encode()
            facts = f.parse_facts(raw)
            if facts["cwd"] != "/home/fixture":
                raise AssertionError("unexpected remote home")
            relay = f.SyntheticRelay()
            candidate = relay.suggest(facts)
            relay.close()
            if shlex.split(candidate) != ["cd", "--", facts["cwd"]]:
                raise AssertionError("unexpected synthetic candidate")
            checks.append("remote helper facts and company-side synthetic relay; candidate not executed")
            missing = run(base + [destination, "/definitely-missing-fixture-python"], check=False)
            if missing.returncode != 127:
                raise AssertionError("missing helper was not reported")
            run(base + [destination, "true"])
            checks.append("missing helper exits 127; separate ordinary command remains usable")
            # No forwarding is required by the stdio relay. Explicitly test server refusal.
            denied = run(base + ["-o", "ExitOnForwardFailure=yes", "-R",
                                "127.0.0.1:43111:127.0.0.1:43112", destination, "true"], check=False)
            if denied.returncode == 0 or "remote port forwarding failed" not in denied.stderr:
                raise AssertionError("expected forwarding refusal not observed")
            checks.append("forwarding denied without retry or alternate forwarding route")
            # Empty private known_hosts file must fail, not prompt or trust automatically.
            hosts.write_text("")
            unknown = run(base + [destination, "true"], check=False)
            if unknown.returncode != 255 or "Host key verification failed" not in unknown.stderr:
                raise AssertionError("unknown host was not rejected")
            checks.append("unknown host identity rejected")
    except (OSError, ValueError, AssertionError, RuntimeError, subprocess.SubprocessError) as exc:
        errors.append(type(exc).__name__ + ": " + str(exc))
    finally:
        cleanup(name, tag, container_attempted, image_attempted, checks, errors)
    print(json.dumps({"scope": "Docker real SSH transport, not interactive shell/mobile/production integration",
                      "passed": checks, "errors": errors}, indent=2))
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
