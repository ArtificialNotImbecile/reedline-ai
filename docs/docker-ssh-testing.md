# Docker SSH transport fixture

**Status, 2026-10-26: recipe and unit/static checks only. The Docker end-to-end
test has NOT run.** The assistant's cloud environment has no Docker CLI, daemon,
or socket. This file does not remove that runtime blocker and does not claim
Windows, mobile, interactive-shell, or production SSH support.

This opt-in fixture complements, not replaces, the existing
[offline contract experiment](ssh-adapter-feasibility.md). It exercises stock
OpenSSH against a disposable Debian container, using the existing fixed helper
and `SyntheticRelay`. It is not an automatic `ssh` wrapper or an API proxy.

## Prerequisites and explicit execution

Use a Linux host with an already configured, authorized Docker Engine,
OpenSSH client/keygen, and Python 3.10+. Do not run against a remote Docker
context: bind mounts and loopback endpoints must refer to the same local host.
The runner rejects non-Unix-socket Docker endpoints before contacting a daemon.
Building fetches the official `debian:bookworm-slim` image and Debian packages;
versions are not digest-pinned, so retain image/build information with results.
Docker storage, package downloads and build cache use local disk/network.

After approval to create the isolated test container and temporary test keys:

```sh
python3 experiments/ssh_adapter/docker_fixture/run.py --run
```

Without `--run`, the script prints help and does nothing. Aggregate local CI
only runs unit/static tests, never this Docker command. It does not install
Docker, change host security settings, enable a daemon or modify shell profiles.

## Boundary and checks

- Unique disposable image/container names; the host publishes only
  `127.0.0.1:<ephemeral high port>`. The runner verifies this exact binding.
- Test client authentication is generated in a temporary directory and deleted
  at exit. Only its public key directory is mounted, read-only. The private key
  is never mounted or sent to the container. Test host identity is generated
  inside the container; its public key is obtained from the trusted local
  Docker control plane and pinned. Unknown host keys must fail.
- Password/root login, agent/X11/TCP forwarding, user RC and environment
  injection are disabled. SSH client configuration, agent use and connection
  multiplexing are disabled for this fixture. No host SSH configuration changes.
- A fixed helper runs over a real authenticated SSH command channel. Its actual
  cwd, interpreter and adapter-shell facts feed an in-process company-side
  synthetic relay. The generated candidate is checked as data, never executed.
- A missing helper must return 127, followed by a separate successful ordinary
  command. This is not a validated interactive fallback terminal.
- A reverse-forward request must be denied by server policy. The fixture does
  not retry forwarding or switch routes to circumvent the refusal.
- Container removal is followed by an absence check. Unique test image removal
  is attempted independently. Failed cleanup is explicitly reported; the
  printed resource names identify anything requiring manual cleanup. Base
  image layers/build cache may remain; no global Docker prune is performed.

The server listens on all interfaces **inside** its isolated container; the
Docker-published host endpoint is loopback-only. No privileged container,
Docker socket mount, host networking or real home-directory mount is used.
The default Docker bridge is not an offline network: this fixture proves no
remote API dependency in its own code, not enforced egress isolation.

## Not yet covered

Docker execution itself remains unverified. Once runnable, this recipe still
does not establish: remote interactive line-editor integration, automatic helper
deployment for arbitrary existing SSH sessions, live cwd tracking, mobile PTY
UX, robust disconnect/timeout recovery, real model calls, or a network API relay.
The relay is a local deterministic function and sends no requests or credentials.
Do not describe these as passed based on static/unit tests.

Safe checks that require no Docker, keys, services or network:

```sh
python3 -m unittest discover -s scripts -p 'test_docker_ssh_fixture.py' -v
python3 -m py_compile experiments/ssh_adapter/docker_fixture/run.py
sh -n experiments/ssh_adapter/docker_fixture/entrypoint.sh
```
