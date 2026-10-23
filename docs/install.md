# Install and try native Bash completion

This milestone bundles the Bash adapter **inside the Rust binary**. `cargo install` does not require you to keep the source checkout for runtime use. No crates.io/apt package or prebuilt cross-platform release is claimed. PowerShell/Nushell/Clink integrations are separate experimental source integrations until their packaging and platform gates pass.

Requirements: Linux with Bash ≥5.1, Python ≥3.9 and a UTF-8 locale; source build additionally needs the repository's pinned Rust toolchain. Tested in isolated Linux Bash PTYs, not macOS's bundled Bash 3.2, Windows Git Bash, or arbitrary terminal/locale combinations.

From a checked-out reviewed revision:

```sh
cargo install --path . --locked
reedline-ai adapter doctor --shell bash
mkdir -p "$HOME/.local/share"
reedline-ai adapter install --shell bash --directory "$HOME/.local/share/reedline-ai-bash"
```

The install directory must be new; the installer never overwrites or edits an existing profile. If Cargo's bin directory is not on PATH, call `"$HOME/.cargo/bin/reedline-ai"` explicitly. The doctor is an offline prerequisite check, not proof that a terminal or remote provider works.

In the **interactive Bash session where you want suggestions**:

```bash
REEDLINE_AI_BIN="$HOME/.cargo/bin/reedline-ai"
REEDLINE_AI_ARGS=(--backend mock)
source "$HOME/.local/share/reedline-ai-bash/reedline-ai.bash"
```

Type `echo 世` (do not press Enter), then:

1. Ctrl-x Ctrl-g starts the request without blocking editing.
2. Ctrl-x Ctrl-a displays the ready preview; if still pending, press it again later.
3. Ctrl-x Ctrl-a again inserts `界` into the existing command line, without executing.
4. Ctrl-C cancels the line as usual. Native Tab, arrows and history remain Bash's.

For `git st`, the conservative review is incomplete, so after preview use Ctrl-x Ctrl-r to acknowledge, then Ctrl-x Ctrl-a to insert. This is not a safety guarantee. Enter remains native Bash execution; inspect the resulting command before pressing it.

## Disable and remove

```bash
reedline_ai_disable
reedline-ai adapter uninstall --shell bash --directory "$HOME/.local/share/reedline-ai-bash"
```

Disable first in each session using the adapter. Uninstall removes only unchanged bundled files and refuses modified, missing or extra contents. If assets came from a different binary revision, retain that matching binary or review/remove the directory manually; the installer does not guess which personal edits to delete. Cargo's binary can separately be removed with `cargo uninstall reedline-ai`. No history, credentials or profiles are removed. Reopen a clean shell if an earlier session remains loaded.

No automatic `.bashrc` edit is made. After testing, you may add the explicit source/configuration lines yourself if desired; remove those same lines yourself before removing assets. Do not put API keys in profiles, install files or repository configuration.

## Optional HTTP mode

First verify mock mode. HTTP sends only the invoked current buffer and explicitly supplied shell metadata to your configured provider; cwd is local-only and native history is not read. Set your key in the process environment using your existing secure mechanism, then select an explicit provider/endpoint/model:

```bash
REEDLINE_AI_ARGS=(--backend http --endpoint https://YOUR_PROVIDER/chat/completions --model YOUR_MODEL --timeout-seconds 8)
```

The existing default `REEDLINE_AI_API_KEY` environment variable is read by the backend. Never pass the key as an argument or write it into an adapter file. `REEDLINE_AI_TIMEOUT_SECONDS` sets the adapter wall deadline (default eight seconds, maximum sixty). Real provider performance, account access and paid API calls are not part of installation tests.

See [Bash keys, lifecycle, and caveats](bash-adapter.md) and [the shared JSON contract](adapter-protocol.md).
