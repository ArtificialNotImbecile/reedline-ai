# SSH adapter feasibility: bounded offline contract experiment

Status (2026-10-23): **local mock-transport validation only**, not a shipped
connection wrapper and not a successful SSH end-to-end test. Existing desktop
and mobile code paths are unchanged. No SSH client/protocol fork, shell alias,
startup-file edit, or replacement of `ssh`, `scp`, `sftp`, or Git is introduced.

## Intended topology

The company computer has Internet access and can initiate SSH to the offline
field computer. An explicit future `connect` action could invoke stock OpenSSH,
bring a small session-only helper over its command channel, and keep inference
and API credentials on the company computer. The field computer does not need
Internet or an API key. This is technically plausible using existing SSH remote
commands/channels; it is **not automatic for arbitrary existing SSH sessions**.

A production implementation would need a distinct, bounded request protocol
alongside the interactive terminal: e.g. a separate session channel/process, or
an explicitly approved reverse-forwarded service. Never mix unrestricted model
requests into terminal stdout, and never expose a general HTTP proxy or company
API bearer key to the remote host. A relay must allow only the intended operation,
validate size/rate/session limits, and stop at session end. A remote host can be
compromised; loopback alone is not authentication or isolation from other users.
Authorization to transfer selected remote context to a model must also be clear.
No real model request or sensitive context transmission occurs in this experiment.

## What is implemented and tested

Run from the repository root:

```sh
python3 experiments/ssh_adapter/feasibility.py
python3 -m unittest discover -s scripts -p 'test_ssh_adapter_feasibility.py' -v
```

The aggregate `bash scripts/ci-local.sh` discovers these tests automatically.
The fixture uses only Python's standard library and already-installed `/bin/sh`
and `python3`. It makes a fresh synthetic directory and starts a fixed Python
helper through the local POSIX shell. The helper source is carried in a quoted
command argument and exists only in process memory: no helper install, dotfile,
history read, recursive path scan, API secret, or generated-command execution.
Child environment is deliberately restricted, not copied from the parent.

The eight test cases cover:

- Actual helper-reported cwd, interpreter, and selected adapter shell from the
  synthetic process, including spaces, Unicode, quotes and shell metacharacters.
- Quoting at the remote-command string boundary, destination option/injection
  rejection, and separate enhanced vs ordinary-shell **argv plans**.
- A synthetic in-process company relay producing a suggestion using only the
  reported cwd. The suggestion is checked as data, never executed.
- Closed transport/relay after success, invalid facts, timeout, missing-helper
  simulation, and relay failure; repeated use of closed sessions is rejected.
- Unsupported shell and synthetic forwarding-denial events return a
  `normal-shell` decision without starting a helper or changing security policy.
- An optional, non-executed reverse-forward plan rejects wildcard/public bind
  addresses and invalid/privileged ports. No listener is opened by these tests.

The only current helper shell is `/bin/sh`. Its identity is measured by executing
that explicitly selected adapter shell, **not inferred from the company OS or
`$SHELL`**. `login_shell` is deliberately unknown (`null`): this experiment does
not discover the user's interactive shell or integrate its line editor. The
cwd is a one-time snapshot, not a live cwd after interactive `cd`. Python is a
prerequisite here, discovered via the fixed fixture PATH; it is not installed.
A target without the required interpreter/shell should retain its normal shell.
No unknown long path is fabricated; the sole path suggestion comes from the
helper's actual cwd. Field-host memory and bounded interpreter discovery remain
future explicit inputs, not scans of user files.

## Evidence boundary and fallback

Installed binaries were inspected read-only: this Linux environment has
OpenSSH_10.0p2 and an sshd binary. **No sshd was started, SSH connection made,
key generated, host contacted, socket opened, or security configuration changed
for this validation.** The mock runs the command string using local `/bin/sh`;
it does not emulate authentication, encryption, channel scheduling, login-shell
startup behavior, SSH quoting on non-POSIX hosts, PTYs, forwarding, disconnects,
or server policy. It cannot establish successful automatic remote deployment.

`normal-shell` is a tested decision/result and separately generated argv plan,
not a tested interactive fallback login. In a real wrapper the fallback must
tear down helper and relay first, explain the enhancement failure, and leave
ordinary host-key verification/authentication intact. Do not repeatedly log in,
auto-accept host keys, change SSH server policy, or switch routes to bypass a
forwarding denial. The sample plans disable agent/X11 forwarding, connection
multiplexing, configured local commands and implicit forwarding; `-F none`
intentionally ignores user SSH config and therefore does not support real-world
host aliases, jump hosts, or custom auth setups in this proof. Plans are not run.

The minimum blocker for real end-to-end validation is an explicitly approved,
disposable SSH server/authentication fixture. Creating test keys and starting an
isolated loopback-only sshd needs approval; merely finding installed binaries
is not that approval. An approved follow-on should verify, with synthetic data:

1. Host identity verification and successful fixed-helper launch over real SSH.
2. Missing interpreter/non-POSIX shell/forced-command rejection and plain-shell
   fallback without configuration changes.
3. Disconnect, timeout, EOF and cleanup, including no surviving helper or relay.
4. If forwarding is used: actual bind addresses, denied forwarding, session
   ownership/authentication, and company-side closure. `GatewayPorts yes` may
   force wildcard server binds; a loopback string alone does not guarantee safety.
5. Separate PTY and target-shell integration before claiming interactive support.

## Primary references

- [OpenSSH ssh(1)](https://man.openbsd.org/ssh.1): remote command arguments are
  joined with spaces, so local argv safety is not sufficient for remote shell
  quoting; stock SSH supports remote commands and forwarding.
- [OpenSSH sshd_config(5)](https://man.openbsd.org/sshd_config.5): server controls
  forwarding permission and bind behavior, including `AllowTcpForwarding` and
  `GatewayPorts`.
- [OpenSSH ssh_config(5)](https://man.openbsd.org/ssh_config.5):
  `ExitOnForwardFailure` concerns forward setup, not reachability of its ultimate
  destination; a relay handshake and lifetime validation would still be needed.
