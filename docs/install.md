# Install and configure reedline-ai

## Simple setup (recommended)

A [prebuilt Linux package](linux-release.md) can be installed without Rust. On other platforms, use a verified platform package if supplied, or the source-build instructions below. Do not treat a Linux binary as a Windows build. The native adapters are bundled in the executable, so the source checkout is not needed at runtime.

Run once from an interactive terminal:

```sh
reedline-ai setup
```

The wizard asks only for **Base URL**, **model name**, and an optional **hidden API key**. The protocol is always OpenAI-compatible **chat completions**, not legacy `/completions`. For example, `https://api.openai.com/v1` becomes `https://api.openai.com/v1/chat/completions`; a full `/chat/completions` endpoint is also accepted. Use the model identifier supplied by your service. There is no provider/protocol selector and setup makes no API request.

Settings are stored only after you answer `y` to the save confirmation. A supplied API key is **unencrypted local data**: Unix creates app directory/file with 0700/0600 permissions; Windows inherits directory ACLs, which this program does not tighten or independently verify. Do not sync/share/commit the settings directory. Local user/admin access and backups can expose the key. To avoid disk storage, leave the key blank and supply `REEDLINE_AI_API_KEY` through your existing secure environment mechanism; that environment variable always overrides a saved key. A blank key on a later setup run removes the previously saved key. Escape/Ctrl-C during hidden key entry or declining save leaves existing settings unchanged.

Default location: `$XDG_CONFIG_HOME/reedline-ai/settings.json`, otherwise `~/.config/reedline-ai/settings.json` on Unix; `%APPDATA%\reedline-ai\settings.json` on Windows. `REEDLINE_AI_SETTINGS_DIR` can select another absolute, private directory. Config is separate from project/workflow memory. Run setup again to replace it; there is no key-printing command.

### Enable in the current shell

Bash (Bash ≥5.1 and Python ≥3.9):

```bash
eval "$(reedline-ai init --shell bash)"
```

PowerShell 7 (PSReadLine ≥2.2; platform-specific test limits still apply):

```powershell
Invoke-Expression (& reedline-ai init --shell powershell | Out-String)
```

The init command extracts content-verified, version-specific adapter assets under the private app directory and prints quoted initialization code. It never modifies shell profiles, invokes an API, or executes suggestions. Read the printed code first if desired by omitting `eval`/`Invoke-Expression`. Existing/changed assets are checked before loading; modified files cause refusal instead of overwrite. PowerShell uses F6=request/preview, F7=accept, F8=cancel; occupied bindings refuse activation.

After setup, init explicitly selects the configured HTTP backend. Without settings, it selects offline mock. Use `init --shell bash --mock` (or `powershell --mock`) to force mock even when configured. Plain `reedline-ai` still starts the offline standalone demo; `reedline-ai --configured` explicitly uses saved settings there. Native shell integrations only send a request when you invoke their request key, not for each keystroke. The request contains the current command and explicit shell metadata; do not type secrets. Native shell history is not uploaded. The real shell still executes normally when **you** press Enter.

Bash keys: Ctrl-x Ctrl-g requests; Ctrl-x Ctrl-a previews, then again accepts; Ctrl-x Ctrl-r acknowledges risky/incomplete previews before acceptance. Disable with `reedline_ai_disable` before re-running init; repeated init refuses to change an active adapter. PowerShell disables with `Disable-ReedlineAI`; also run `Remove-Module ReedlineAI` before re-running init in the same session. No profile is edited, so opening a fresh shell also starts without this activation. Nushell and CMD currently use the explicit extraction instructions below rather than one-line init.

### Remove settings and init assets

Disable active adapters before uninstalling a binary or assets. Initialization assets are under the app directory's `adapters/` subdirectories; use the matching binary's `adapter uninstall --shell SHELL --directory EXACT_SUBDIRECTORY` to remove unchanged assets, or inspect them and delete manually. Settings and saved keys deliberately survive binary/package uninstall. To remove credentials, rerun setup with a blank key and save, or delete the app's `settings.json` yourself. Do not delete a shared configuration parent or unrelated files.

## Explicit adapter installation (advanced / source build)

The native adapters and their transport helpers are bundled **inside the Rust binary**. `cargo install` does not require you to keep the source checkout for runtime use. No crates.io/apt package or universal cross-platform release is claimed. Bash has the first complete Linux installation gate. PowerShell, Nushell and CMD+Clink are explicitly experimental and require `--experimental` when extracting; their native platform limits remain below.

## Bash quickstart

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

First verify mock mode. HTTP sends only the invoked current buffer and explicitly supplied shell metadata to your configured provider; cwd is local-only and native history is not read. The recommended setup/init flow above avoids hand-written flags. For an explicit advanced override, set your key in the process environment using your existing secure mechanism, then select an endpoint/model:

```bash
REEDLINE_AI_ARGS=(--backend http --endpoint https://YOUR_PROVIDER/chat/completions --model YOUR_MODEL --timeout-seconds 8)
```

The existing default `REEDLINE_AI_API_KEY` environment variable is read by the backend. Never pass the key as an argument or write it into an adapter file. `REEDLINE_AI_TIMEOUT_SECONDS` sets the adapter wall deadline (default eight seconds, maximum sixty). Real provider performance, account access and paid API calls are not part of installation tests.

See [Bash keys, lifecycle, and caveats](bash-adapter.md) and [the shared JSON contract](adapter-protocol.md).


## Experimental packages

All packages default to the offline mock and must be explicitly enabled in the real shell. `adapter install` only extracts files. `adapter doctor --shell powershell|nushell|cmd` checks available prerequisites on PATH, not full terminal compatibility. No runtime or Clink installation, profile edits, API setup, or security-policy changes happen automatically.

### PowerShell

Requires PowerShell ≥7.2 and PSReadLine ≥2.2; tested on Linux PowerShell 7.4.6/PSReadLine 2.3.5 and Windows 10.0.19045.3803 with PowerShell 7.6.5; macOS remains untested. The module must be allowed to compile its adjacent trusted C# helper. Do not weaken execution/security policies for it.

```powershell
reedline-ai adapter doctor --shell powershell
reedline-ai adapter install --shell powershell --experimental --directory "$HOME/reedline-ai-powershell"
Import-Module "$HOME/reedline-ai-powershell/ReedlineAI.psm1"
Enable-ReedlineAI -Binary reedline-ai -RequestChord F6 -AcceptChord F7 -CancelChord F8
```

Type `echo 世`; F6 requests, F6 again shows the ready preview, F7 inserts, F8 discards. These function keys were tested on the listed Linux and Windows hosts; the default Ctrl+Alt chords may not arrive correctly from every terminal. Existing bindings cause enable to refuse rather than overwrite. Risky **and incomplete** results (including Git) are blocked, not silently accepted. Native Ctrl-C alone cannot invalidate an identically retyped buffer during the short preview lifetime: use the explicit discard key. Read [the exact PowerShell caveats](powershell-adapter.md).

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

Stock CMD is not supported by a native predictor interface here. The adapter requires separately installed Clink ≥1.7.12 and Python ≥3.10 on Windows, explicit absolute executable paths, private temporary storage and working Job Object containment. Native validation passed on Windows 10.0.19045.3803 with Clink 1.9.35, including explicit request/preview/accept, Unicode, v2 local-workflow replacement, native Tab/history, disable and helper/timeout descendant cleanup. Portable transport tests and Lua mocks alone are not a Windows pass.

```bat
reedline-ai adapter doctor --shell cmd
reedline-ai adapter install --shell cmd --experimental --directory "%LOCALAPPDATA%\reedline-ai-clink"
```

Follow the [CMD/Clink guide](cmd-adapter.md) for its explicit session-only loading and disable steps. Keep its `cmd` and `shared` directories together. Do not add it to an existing Clink profile until you have tested it in a disposable session. Removal uses `adapter uninstall --shell cmd --directory` with the same path after disabling the session.

## Windows validation scope (2026-10-26)

The supplied Windows package was tested on Windows **10.0.19045.3803**,
PowerShell **7.6.5** and Clink **1.9.35**, including setup, native PowerShell/CMD
request-preview-accept flows and restricted v2 local-workflow replacement.
The Windows binary requires the Microsoft Visual C++ x64 Runtime (14.50 was
used in validation); that runtime is not bundled.
This is not an all-Windows compatibility claim. Windows Nushell, interactive
Bash at a supported version, killing the entire CMD host, and Windows Terminal
GUI behavior remain unvalidated. Existing experimental, risk-review and
prompt-lifecycle caveats still apply; no candidate is executed automatically.
