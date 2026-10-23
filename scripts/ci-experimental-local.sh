#!/usr/bin/env bash
# Explicit native-runtime gate. Does not download runtimes or call model APIs.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
pwsh=${REEDLINE_AI_PWSH:-$(command -v pwsh || :)}
nu=${NU_BINARY:-$(command -v nu || :)}
if [[ ! -x "$pwsh" || ! -x "$nu" ]]; then
  printf '%s\n' 'Set REEDLINE_AI_PWSH and NU_BINARY to installed executable paths. Missing runtimes are not a pass.' >&2
  exit 1
fi
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
export XDG_CACHE_HOME="$scratch/cache" XDG_CONFIG_HOME="$scratch/config" XDG_DATA_HOME="$scratch/data"
export PYTHONDONTWRITEBYTECODE=1
unset REEDLINE_AI_API_KEY
cargo build --locked
python3 -m unittest discover -s integrations/shared -p 'test_*.py'
"$pwsh" -NoLogo -NoProfile -NonInteractive -File integrations/powershell/tests/Test-ReedlineAI.ps1 -Binary "$PWD/target/debug/reedline-ai"
python3 integrations/powershell/tests/pty_smoke.py "$pwsh" target/debug/reedline-ai
NU_BINARY="$nu" REEDLINE_AI_TEST_BINARY="$PWD/target/debug/reedline-ai" python3 -m unittest discover -s integrations/nushell -p 'test_*.py'
python3 scripts/pty_installed_powershell.py "$pwsh"
python3 scripts/pty_installed_nushell.py "$nu"
printf '\nExperimental Linux native/installed gates passed. Windows CMD/Clink and macOS remain unverified.\n'
