# Bash / GNU Readline adapter

This is an **optional real-shell integration**, not a replacement shell. Native
Bash still owns editing, completion, history, parsing, and execution. The adapter
only obtains a suggestion, shows a preview, and inserts explicitly accepted text.
It never executes suggestions or calls `eval`.

## Install and enable

The binary bundles the adapter and bridge, so an installed binary does not need
this source checkout at runtime. From a checkout, build/install the CLI with
`cargo install --path . --locked`. Then use the explicit, profile-free setup:

```bash
reedline-ai adapter doctor --shell bash
mkdir -p "$HOME/.local/share/reedline-ai"
reedline-ai adapter install --shell bash --directory "$HOME/.local/share/reedline-ai/bash"
source "$HOME/.local/share/reedline-ai/bash/reedline-ai.bash"
```

The default backend is mock; type `echo ` and use the keys below. In the current
shell, `reedline_ai_disable` cancels work, restores the previous four key
bindings still owned by the adapter, and removes the prompt reset hook. Later
user/plugin key changes survive disable; calling disable again is a no-op. To remove the extracted files:

```bash
reedline_ai_disable
reedline-ai adapter uninstall --shell bash --directory "$HOME/.local/share/reedline-ai/bash"
```

Neither operation edits a shell profile. If you choose to add a `source` line to
your own profile later, remove that line yourself when uninstalling.

## Try it without changing a profile

Requires **Bash 4.3+**, **Python 3.9+**, and the built `reedline-ai` binary. The
macOS system Bash 3.2 is not supported. A UTF-8 locale is recommended for Unicode.
Run from the repository root:

```bash
cargo build
bash --noprofile --norc
REEDLINE_AI_BIN="$PWD/target/debug/reedline-ai"
REEDLINE_AI_ARGS=(--backend mock)
source integrations/bash/reedline-ai.bash
```

Nothing edits `.bashrc`, `.bash_profile`, `.inputrc`, history files, or SSH
configuration. To leave this isolated test shell, clear any unexecuted text and
run `exit`. The default backend is offline mock. Type `echo ` to try it.

## Keys and explicit acceptance

Repeat **Ctrl-x Ctrl-g** for the whole flow: request → preview → insert.
Each step is a separate keypress; a newly received candidate is never inserted
immediately. While pending, editing remains responsive. Pressing the request key
after editing cancels the obsolete request and starts one for the new input.
**Ctrl-x Ctrl-k** cancels AI state without clearing the command you are typing.

The existing **Ctrl-x Ctrl-a** remains an alias for review/insert. Candidates with
warnings or incomplete risk review still need **Ctrl-x Ctrl-r** to acknowledge
that exact displayed preview before insertion. Enter stays normal Bash execution.
Tab, arrows, backspace and native history search are unchanged. The four chords
are installed in Emacs and Vi insertion maps and restored on disable.

Previews display validated Unicode directly, including Chinese, with terminal
control and invisible formatting characters rejected before display. Run
`reedline_ai_help` for the shortcut reminder.

## Local history from the first session

By default, an explicit request searches the latest 200 entries already loaded
in Bash (`fc`), newest first. A match must preserve both the typed prefix and the
suffix after the cursor. Only one candidate is passed through a pipe to the local
Rust validator; it receives the same insertion and risk checks as model output.
A match makes **zero model calls**. If nothing matches, the configured backend is
used with the current line only. History is never uploaded, even on a miss.

Set `REEDLINE_AI_HISTORY=0` to disable this. The adapter does not open `.bash_history`,
merge other shells' histories, or rewrite history settings. Bash's own startup
and `HISTFILE` determine which history is loaded; a clean `--norc` test session has
no personal warm-start data. Space-prefixed, multiline, control-bearing and
oversized candidates are not offered. Local history is useful context, not a
promise that previously executed commands are safe to run again.

## Configuration

```bash
# A single executable path, not a shell command string:
REEDLINE_AI_BIN=/absolute/path/to/reedline-ai
# An indexed Bash array, never a space-delimited command string:
REEDLINE_AI_ARGS=(--backend mock)
# Bridge timeout in seconds, between 0.05 and 60; default 8:
REEDLINE_AI_TIMEOUT_SECONDS=8
```

Arguments are passed directly to the executable with `--adapter-request` added.
The adapter does not construct shell commands from candidate text. Network use
requires explicit HTTP backend configuration through the existing CLI; no real
provider or API key is used in the adapter tests. Avoid typing secrets into a
buffer you send to a configured remote provider. The request includes Bash kind
and version plus local `cwd`; the backend treats `cwd` as local metadata and does
not add it to model context. A single local history match may accompany the request, but never enters model context.

## Lifecycle and failure behavior

- Blank or whitespace-only lines invoke neither Python nor the backend.
- Inference uses an anonymous pipe and one bounded subprocess, not a temporary
  file, history entry, or plaintext argument containing the input buffer.
- Backend stdin/stdout are pipes, stderr is suppressed, and its process group is
  isolated from the terminal. The bridge enforces timeout and stdout size limits.
- The bridge validates protocol/version, request identity, exact source buffer,
  UTF-8 byte cursor boundaries, insertion-only result reconstruction, result
  cursor, and risk fields. Malformed responses never modify the line.
- `READLINE_POINT` uses UTF-8 byte offsets in Bash 4.x and locale-character offsets in Bash 5.0+.
  The adapter selects the correct conversion for each version, including when
  accepting text and matching local history.
  Thus multibyte text before the cursor and preserved suffixes both work.
- Input buffers are limited to 8,192 bytes, insertions to 1,024 bytes, backend
  stdout to 128 KiB, and escaped previews to 16 KiB. Fenced output, newlines,
  terminal control characters, and invisible formatting controls are rejected.
  Multiline shell constructs are outside this first adapter's scope.
- Editing or moving the cursor invalidates a mismatched snapshot. Changing the
  directory also invalidates it. Editing away and back to an exactly identical
  snapshot within the same prompt is treated as the same buffer.
- A small reset function is prepended to `PROMPT_COMMAND` (a scalar on Bash 4.3–5.0; native array on 5.1+).
  Every new primary prompt, including after Ctrl-C, cancels the pending pipe and
  clears previews and acknowledgments. Existing scalar/array prompt commands
  remain in order; the reset preserves the previous command's exit status.
  Readonly, nameref, or associative `PROMPT_COMMAND` configurations are refused.
  Configure prompt frameworks before sourcing this adapter, and do not replace
  `PROMPT_COMMAND` afterward, because that would remove lifecycle protection.
- Closing/cancelling the result pipe causes the bridge to kill the backend
  process group; timeout is a second bound. No adapter traps are installed.
- Backend failures expose only fixed local status messages, never raw provider
  errors or model output. The current command line remains unchanged.

This uses an explicit readiness/review key instead of unsafe
asynchronous terminal repaint or a blocking model call inside `bind -x`.

## Offline validation

```bash
bash -n integrations/bash/reedline-ai.bash
python3 -m py_compile integrations/bash/bridge.py scripts/pty_bash_adapter.py
python3 scripts/pty_bash_adapter.py target/debug/reedline-ai
```

The PTY suite starts `bash --noprofile --norc` with isolated HOME and inputrc. It
checks the actual Rust mock backend, explicit preview then insertion, risk
acknowledgment, native completion/history/editing, Unicode cursor conversion,
suffix preservation, inert shell metacharacters, zero-call blank input,
responsive typing during inference, stale snapshots, cancellation, timeout, and
invalid/oversized/control-bearing backend responses. All fixture text is
synthetic; no generated suggestion is submitted to Bash for execution.

Verified locally on Linux with Bash 5.2.37, Python 3.12.14, and a POSIX PTY using
`TERM=xterm-256color`. The default Emacs path is runtime-tested. The Vi insertion
bindings are installed but full Vi-mode, arbitrary terminal emulators and SSH paths are not claimed as tested.
The 2026-10-31 macOS Bash 4.3/5.0/5.2 and DeepSeek coverage is recorded in
[VALIDATION.md](VALIDATION.md). The installed-assets gate is
`python3 scripts/pty_installed_bash.py`, which uses a disposable HOME and tests the
embedded copy through the same PTY suite (including disable/uninstall).

## Official integration references

- [GNU Bash `bind -x` and its buffer/cursor update contract](https://www.gnu.org/s/bash/manual/html_node/Bash-Builtins.html)
- [Bash Readline variables and `PROMPT_COMMAND`](https://www.gnu.org/software/bash/manual/html_node/Bash-Variables.html)
- [Bash 5.1 release: native `PROMPT_COMMAND` arrays](https://lists.gnu.org/archive/html/info-gnu/2020-12/msg00003.html)

The manual establishes the callback contract; the PTY regression test establishes
this implementation's Unicode offset behavior on the tested runtime.

## One-command trial and runtime selection

From a built checkout: `bash scripts/try-bash.sh` starts an offline trial; add
`--deepseek` to explicitly use the official `deepseek-flash` API with an existing
`DEEPSEEK_API_KEY` or `REEDLINE_AI_API_KEY`. No credential or profile is written.
The launcher searches PATH and the Apple Silicon/Intel Homebrew locations for
Bash 4.3+. `REEDLINE_AI_BASH=/path/to/bash` selects a runtime. Exit returns to the
original shell. The trial reads `REEDLINE_AI_HISTORY_FILE`, exported `HISTFILE`, or Bash's
standard `~/.bash_history` locally, and does not save history. Set
`REEDLINE_AI_HISTORY=0` for a completely fresh trial. The normal sourced adapter
itself still only uses history that Bash has already loaded. Network environments
can explicitly add `--allow-env-proxy`; no proxy or certificate settings are changed.

macOS's system Bash 3.2 lacks the editable Readline contract this adapter needs;
install a newer Bash instead of silently loading a nonfunctional plugin. Linux
and Windows WSL/Git Bash use the same adapter when Bash/Python meet requirements;
Windows CMD uses the separate experimental Clink integration.

For a version matrix, set `RAI_BASH=/path/to/bash` when running the PTY suite.
A separately authorized live test is
`python3 scripts/pty_deepseek_bash.py --allow-billable-api target/debug/reedline-ai`.
It sends only three synthetic inputs; no generated command is executed.
