#!/usr/bin/env bash
# Local validation only: no GitHub Actions runs or remote model API calls.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

for tool in cargo python3; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf 'Required tool not found: %s\n' "$tool" >&2
    exit 1
  fi
done

python3 -m unittest discover -s scripts -p 'test_*.py'
python3 -m unittest discover -s integrations/shared -p 'test_*.py'

case "$(uname -s)" in
  Linux|Darwin) ;;
  *)
    printf 'This script requires Linux or macOS for POSIX PTY tests. See README for Windows checks.\n' >&2
    exit 1
    ;;
esac

cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --release --all-targets
cargo build --locked
cargo build --locked --release
cargo run --locked -- --complete 'git st'
cargo run --locked -- --complete 'git  --short' --cursor 4
cargo run --locked -- --complete 'echo 世' --cursor 8
python3 scripts/pty_smoke.py
python3 scripts/pty_smoke.py target/release/reedline-ai
python3 scripts/pty_native.py
python3 scripts/pty_native.py target/release/reedline-ai
python3 scripts/pty_risk.py
python3 scripts/pty_risk.py target/release/reedline-ai

python3 scripts/pty_mobile.py
python3 scripts/pty_mobile.py target/release/reedline-ai

# Real native Bash adapters and self-contained clean-HOME installation.
python3 scripts/pty_bash_adapter.py
python3 scripts/pty_bash_adapter.py target/release/reedline-ai
python3 scripts/pty_installed_bash.py

printf '\nLocal CI passed on %s. No hosted workflow or remote model API was invoked.\n' "$(uname -s)"
