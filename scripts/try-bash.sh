#!/usr/bin/env bash
# Start a disposable Bash session; never edit profiles or store credentials.
set -eu
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
mode=${1:---mock}
network_option=${2:-}
case "$network_option" in
  ""|--allow-env-proxy) ;;
  *) printf 'Unknown network option. Use --allow-env-proxy to opt into environment proxies.\n' >&2; exit 2 ;;
esac
case "$mode" in
  --mock|--deepseek) ;;
  *) printf 'Usage: bash scripts/try-bash.sh [--mock|--deepseek] [--allow-env-proxy]\n' >&2; exit 2 ;;
esac
bash_binary=
for candidate in "${REEDLINE_AI_BASH:-bash}" /opt/homebrew/bin/bash /usr/local/bin/bash; do
  if "$candidate" --noprofile --norc -c '(( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 3) ))' 2>/dev/null; then
    bash_binary=$candidate
    break
  fi
done
if [[ -z $bash_binary ]]; then
  printf '%s\n' 'Bash 4.3+ is required. On macOS: brew install bash. On Linux: update bash with your package manager.' >&2
  exit 1
fi
binary=${REEDLINE_AI_BIN:-}
if [[ -z $binary ]]; then
  if [[ -x $root/target/debug/reedline-ai ]]; then binary=$root/target/debug/reedline-ai
  elif [[ -x $root/target/release/reedline-ai ]]; then binary=$root/target/release/reedline-ai
  else binary=$(command -v reedline-ai || :)
  fi
fi
if [[ -z $binary ]]; then
  printf '%s\n' 'Build first with cargo build --locked, or set REEDLINE_AI_BIN to an installed executable.' >&2
  exit 1
fi
export REEDLINE_AI_BIN=$binary
if [[ $mode == --deepseek && -z ${DEEPSEEK_API_KEY:-} && -z ${REEDLINE_AI_API_KEY:-} ]]; then
  printf '%s\n' 'Set DEEPSEEK_API_KEY or REEDLINE_AI_API_KEY in your environment before using --deepseek.' >&2
  exit 1
fi
rc=$(mktemp "${TMPDIR:-/tmp}/reedline-ai-bash.XXXXXXXX")
trap 'rm -f -- "$rc"' EXIT
{
  # Load the user's configured Bash history locally, then prevent trial writes.
  # Do not infer zsh/PowerShell/Nushell history paths from the operating system.
  if [[ ${REEDLINE_AI_HISTORY:-1} == 1 ]]; then
    history_file=${REEDLINE_AI_HISTORY_FILE:-${HISTFILE:-$HOME/.bash_history}}
    printf 'if [[ -f %q ]]; then builtin history -r %q; fi\n' "$history_file" "$history_file"
  fi
  printf 'HISTFILE=/dev/null\n' 
  printf 'REEDLINE_AI_ARGS=(--backend mock)\n'
  if [[ $mode == --deepseek ]]; then
    key_env=DEEPSEEK_API_KEY
    [[ -n ${DEEPSEEK_API_KEY:-} ]] || key_env=REEDLINE_AI_API_KEY
    printf 'REEDLINE_AI_ARGS=(--backend http --provider deepseek --endpoint https://api.deepseek.com/chat/completions --model deepseek-flash --api-key-env %s)\n' "$key_env"
  fi
  if [[ $network_option == --allow-env-proxy && $mode == --deepseek ]]; then
    printf 'REEDLINE_AI_ARGS+=(--allow-env-proxy)\n'
  fi
  printf 'source %q || exit 1\n' "$root/integrations/bash/reedline-ai.bash"
  printf "PS1='[reedline-ai] \\w \\$ '\n"
  printf 'reedline_ai_help\n'
} > "$rc"
printf 'Starting Bash with %s. Type echo followed by a space, then Ctrl-x Ctrl-g. Exit returns to your shell.\n' "$mode"
"$bash_binary" --noprofile --rcfile "$rc" -i
