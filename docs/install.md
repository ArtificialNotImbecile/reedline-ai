# Install native shell adapters

The native adapters and their transport helpers are bundled **inside the Rust binary**. `cargo install` does not require you to keep the source checkout for runtime use. No crates.io/apt package or prebuilt cross-platform release is claimed. Bash has the first complete Linux installation gate. PowerShell, Nushell and CMD+Clink are explicitly experimental and require `--experimental` when extracting; their native platform limits remain below.

## Bash quickstart

Requirements: Linux/macOS with Bash ≥4.3, Python ≥3.9 and a UTF-8 locale; source build additionally needs the repository's pinned Rust toolchain. Tested in isolated Bash PTYs on Linux and macOS. macOS bundled Bash 3.2 cannot edit Readline buffers; use a newer Bash. Windows Git Bash and arbitrary terminal/locale combinations still need validation.

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
2. Ctrl-x Ctrl-g again displays the ready preview; if still pending, press it again later.
3. Ctrl-x Ctrl-g once more inserts `界` into the existing command line, without executing.
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

First verify mock mode. HTTP sends only the invoked current buffer and explicitly supplied shell metadata to your configured provider; cwd and the locally matched history candidate stay local. No history is uploaded. Set your key in the process environment using your existing secure mechanism, then select an explicit provider/endpoint/model:

```bash
REEDLINE_AI_ARGS=(--backend http --endpoint https://YOUR_PROVIDER/chat/completions --model YOUR_MODEL --timeout-seconds 8)
```

The existing default `REEDLINE_AI_API_KEY` environment variable is read by the backend. Never pass the key as an argument or write it into an adapter file. `REEDLINE_AI_TIMEOUT_SECONDS` sets the adapter wall deadline (default eight seconds, maximum sixty). Real provider performance, account access and paid API calls are not part of installation tests.

See [Bash keys, lifecycle, and caveats](bash-adapter.md) and [the shared JSON contract](adapter-protocol.md).


## Experimental packages

All packages default to the offline mock and must be explicitly enabled in the real shell. `adapter install` only extracts files. `adapter doctor --shell powershell|nushell|cmd` checks available prerequisites on PATH, not full terminal compatibility. No runtime or Clink installation, profile edits, API setup, or security-policy changes happen automatically.

### PowerShell

Requires PowerShell ≥5.1 and PSReadLine ≥2.0; PowerShell 7.4.6/PSReadLine 2.3.5 has native PTY coverage on Linux and macOS. Windows 5.1 still requires native validation. The module must be allowed to compile its adjacent trusted C# helper. Do not weaken execution/security policies for it.

```powershell
reedline-ai adapter doctor --shell powershell
reedline-ai adapter install --shell powershell --experimental --directory "$HOME/reedline-ai-powershell"
Import-Module "$HOME/reedline-ai-powershell/ReedlineAI.psm1"
Enable-ReedlineAI -Binary reedline-ai -RequestChord F6 -AcceptChord F7 -CancelChord F8
```

Type `echo 世`; F6 requests, F6 again shows the ready preview, F7 inserts, F8 discards. These function keys are the Linux-tested fallback; the default Ctrl+Alt chords may not arrive correctly from every terminal. Existing bindings cause enable to refuse rather than overwrite. Risky **and incomplete** results (including Git) are blocked, not silently accepted. Native Ctrl-C alone cannot invalidate an identically retyped buffer during the short preview lifetime: use the explicit discard key. Read [the exact PowerShell caveats](powershell-adapter.md).

```powershell
Disable-ReedlineAI
reedline-ai adapter uninstall --shell powershell --directory "$HOME/reedline-ai-powershell"
```

### Nushell

Requires the experimental job/commandline APIs tested on Nu 0.116.1 and Python ≥3.10. Extract first (the parent directory must already exist), then in Nu:

```nu
reedline-ai adapter doctor --shell nushell
reedline-ai adapter install --shell nushell --experimental --directory ($env.HOME | path join 'reedline-ai-nushell')
```

Finish the extraction command first. Nu resolves `source` while parsing, so do not paste installation and sourcing as one block on a fresh setup. Then run this separate block:

```nu
$env.REEDLINE_AI_EXECUTABLE = (which reedline-ai | first | get path)
source ~/reedline-ai-nushell/nushell/reedline-ai.nu
reedline-ai-enable
```

Use Ctrl+Alt+g to request, Ctrl+Alt+p to preview, Ctrl+Alt+a to accept, and Ctrl+Alt+x to discard. Native binding conflicts refuse enable. Keep the extracted `nushell` and `shared` directories together. Risky or incomplete suggestions are blocked. See [Nushell lifecycle and runtime limits](nushell-adapter.md).

```nu
reedline-ai-disable
reedline-ai adapter uninstall --shell nushell --directory ($env.HOME | path join 'reedline-ai-nushell')
```

### CMD with Clink

Stock CMD is not supported by a native predictor interface here. The adapter requires separately installed Clink ≥1.7.12 and Python ≥3.10 on Windows, explicit absolute executable paths, private temporary storage and working Job Object containment. **Windows/Clink native execution has not been validated in this Linux environment.** Portable transport tests and Lua mocks are not a Windows pass.

```bat
reedline-ai adapter doctor --shell cmd
reedline-ai adapter install --shell cmd --experimental --directory "%LOCALAPPDATA%\reedline-ai-clink"
```

Follow the [CMD/Clink guide](cmd-adapter.md) for its explicit session-only loading and disable steps. Keep its `cmd` and `shared` directories together. Do not add it to an existing Clink profile until you have tested it in a disposable session. Removal uses `adapter uninstall --shell cmd --directory` with the same path after disabling the session.

## Runtime compatibility and history

Bash 4.3–5.0 uses a scalar prompt hook; 5.1+ uses a prompt-command array.
`REEDLINE_AI_BASH` selects the doctor/launcher runtime. PowerShell 5.1+ with
PSReadLine 2.0+ has a compatibility path; `REEDLINE_AI_PWSH` selects the doctor
runtime. Enabling checks required editor APIs. Windows PowerShell 5.1, Git Bash,
and CMD/Clink remain native Windows validation targets.

Bash, PowerShell and Nushell use their own already-loaded history, locally first.
No first-run cloud analysis or cross-shell history import occurs. Bash history
is disabled with `REEDLINE_AI_HISTORY=0`, PowerShell with `-NoHistory`, and Nushell
with `$env.REEDLINE_AI_HISTORY = false`. CMD/Clink retains its native history;
this version does not add history context to its AI adapter.
