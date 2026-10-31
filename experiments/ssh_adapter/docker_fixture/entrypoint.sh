#!/bin/sh
set -eu
# Test-only host identity, held inside this disposable container.
ssh-keygen -q -t ed25519 -N '' -f /run/sshd/fixture_host_ed25519
exec /usr/sbin/sshd -D -e -f /etc/ssh/sshd_config
