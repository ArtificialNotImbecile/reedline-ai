# PowerShell: opt-in PSReadLine key adapter

Status: **experimental**, with a documented prompt-lifecycle limitation.

This is a native PSReadLine **explicit-request adapter**, not a replacement shell,
not a predictor plugin, and not an SSH integration. It edits the existing command
buffer only after a preview and a separate acceptance key. It never evaluates or
submits a suggestion. SSH work remains deferred until an environment is provided.

## Requirements and session-only setup

- PowerShell **5.1 or later**, using PSReadLine **2.0 or later**. Both Windows
  PowerShell (`powershell.exe`) and PowerShell Core (`pwsh`) have compatibility
  paths. Windows 5.1 still requires native Windows validation. Older PSReadLine
  releases without `-Chord` use a filtered binding listing.
- An interactive PSReadLine host in **Windows or Emacs editing mode**. Vi mode is
  rejected by this initial adapter. ISE and other non-PSReadLine hosts are not
  supported. OS/terminal delivery of Ctrl+Alt letters varies.
- A locally built `reedline-ai` executable, with the adapter protocol enabled.
- The trusted adjacent C# helper must be loadable with `Add-Type`; constrained
  language/security policies can prevent this. Do not weaken them to load it.

For self-contained extraction from an installed binary, follow [the installation guide](install.md#powershell). Import the extracted module instead of the source-checkout path below. The adjacent `AdapterProcess.cs` is included automatically.

Alternatively, from the checkout in a PowerShell session:

```powershell
cargo build --release
Import-Module ./integrations/powershell/ReedlineAI.psm1
Enable-ReedlineAI -Binary ./target/release/reedline-ai
# On Windows: -Binary .\target\release\reedline-ai.exe
```

Importing the module alone changes no key bindings and starts no worker. Enabling
sets three previously **unbound** keys, and refuses collisions. It
does not modify profiles, install software, replace Tab/Enter/arrows/Ctrl-C, read
history/files, or change native completion/prediction settings. Other PSReadLine
editing, history and completion remain owned by PSReadLine. Set custom unused
keys with `-RequestChord`, `-AcceptChord`, and `-CancelChord`. These accept
Ctrl+Alt+letter chords or F5–F12. For a terminal that does not transmit Ctrl+Alt
letters correctly, use `-RequestChord F6 -AcceptChord F7 -CancelChord F8`, after
checking those keys are not reserved by your terminal.

## Key flow

1. Type a nonblank single-line command. `Ctrl+Alt+a` starts one asynchronous
   request and returns immediately to native editing.
2. Press `Ctrl+Alt+a` again to collect a ready result and print a preview. If it
   is still pending, input is unchanged. No background thread draws on the host.
3. Press `Ctrl+Alt+a` a third time (or `Ctrl+Alt+g`) within **15 seconds** of the preview to insert text at the
   original cursor. The full original buffer, UTF-16 cursor, filesystem cwd and
   lack of a selection must still match. Insertion is one native undoable edit;
   the suffix is preserved. Nothing is submitted or executed.
4. `Ctrl+Alt+x` cancels the request and discards any preview without touching
   input. A cancelled worker retains its slot until it has stopped, so repeated
   keys do not create an unbounded process queue.

Try `echo 世`, then request, preview and accept. With the offline mock backend the
candidate should complete to `echo 世界`. Do not use `git ` as the initial demo:
Git's conservative risk review is incomplete, so this adapter blocks it.

A blank input starts **zero** processes/model calls. Multiline input, selections,
non-filesystem locations, terminal controls, bidi controls, invalid Unicode and
inputs over 8192 UTF-8 bytes are rejected. Backends may return only an insertion
of at most 1024 UTF-8 bytes with a matching full result and byte cursor.

### Prompt lifecycle limitation

PSReadLine's public buffer APIs do not expose a prompt-generation identifier.
This adapter deliberately does not wrap the user's prompt or native Ctrl-C.
Consequently, cancelling a line with native Ctrl-C and retyping an **identical**
buffer/cursor in the same cwd before the preview expires cannot be distinguished
from an unchanged buffer. Use `Ctrl+Alt+x` to discard AI state when abandoning a
line; old pending results expire after the configured timeout plus 15 seconds,
and shown previews expire after 15 seconds. This is a documented limitation, not
full prompt-lifecycle invalidation. Acceptance always requires a previously
shown preview and a separate acceptance action.

## Configuration and safety boundaries

The default is the deterministic **offline mock**, with no provider configuration
or credentials. `-BackendArguments` is an explicit array of individual CLI
arguments, not a command string. For example, keep mock mode and shorten the
request deadline:

```powershell
Enable-ReedlineAI -Binary ./target/release/reedline-ai `
    -BackendArguments @('--backend', 'mock') -TimeoutMilliseconds 5000
```

Disable an already enabled adapter before changing its configuration. The timeout
range is 100–60000 ms, default 10000 ms. Users can deliberately configure the
existing HTTP CLI options as individual array elements, but this adapter never
auto-selects an endpoint, reads a key itself, or enables AI history. The configured
backend remains responsible for its explicitly enabled network behavior. Never
put API secrets in arguments; use the backend's documented environment-variable
mechanism only when intentionally enabling a real provider.

The worker launches an executable using `.NET ProcessStartInfo`,
`UseShellExecute = false`, and `ArgumentList` where available. Windows .NET Framework uses individually
CRT-quoted arguments, byte-encoded UTF-8 stdin, and an event-based asynchronous
exit wait instead of newer .NET-only APIs. The request is UTF-8 JSON on stdin,
not interpolated into a shell, command line or temporary script. Stdout/stderr
are independently bounded to 65536 characters. Stderr is never displayed. On a
request deadline or cancellation the worker kills the process tree where the OS
allows it, with a parent-process fallback (parent-only on .NET Framework). Killing descendants is best-effort,
not an OS sandbox; configure only a trusted executable. A descendant that keeps
redirected handles after its parent has already exited may outlive the deadline;
.NET cannot reliably recover that process tree. Force-killing the PowerShell host
also does not guarantee adapter cleanup. Use `Disable-ReedlineAI` before closing
a session with active work; the supplied Rust bridge does not spawn subprocesses.
The worker and I/O run
off the editor thread, including cancellation, and time out without another
keypress. No provider polling, timers drawing into PSReadLine, or auto-triggering
on text changes occur.

The protocol is version 1: shell identity is explicitly `powershell` plus the
running version, cwd is local metadata, and cursors are UTF-8 byte offsets. The
adapter converts PSReadLine's UTF-16 index with strict encoding and rejects a
cursor splitting a surrogate pair. It checks the response request ID, original
buffer, original byte cursor, insertion, full result and result cursor before
previewing. Cwd is checked again locally; the shared bridge validates it without
sending it to the model or reading it.

Risk is **fail-closed**: any warning, incomplete review, missing/malformed review,
or `additional_acceptance_required` blocks the result. The CLI's general
`--risk-policy off` cannot relax this adapter gate. The adapter does not currently
provide an extra risk-confirmation key; candidates requiring it must be edited
manually. A lack of warnings is not proof a command is safe, especially with
user-defined aliases/functions; inspect every preview.

## Disable without profile changes

```powershell
Disable-ReedlineAI
Remove-Module ReedlineAI
```

Disable cancels pending work, clears the preview and removes only the adapter's
three bindings if they still identify its own handlers. It does not overwrite a
binding subsequently installed by another tool. No profile or persistent config
was created to remove. When changing editing modes, disable the adapter first.

## Verification

Run the dependency-free local transport/stub-editor suite in PowerShell 5.1+:

```powershell
pwsh -NoLogo -NoProfile -NonInteractive -File integrations/powershell/tests/Test-ReedlineAI.ps1
# Also exercise the real offline Rust bridge:
pwsh -NoLogo -NoProfile -NonInteractive -File integrations/powershell/tests/Test-ReedlineAI.ps1 `
    -Binary ./target/debug/reedline-ai
```

Verified locally on Linux with portable PowerShell **7.4.6**, PSReadLine **2.3.5**:

- The transport/stub-editor suite and real offline Rust bridge passed; native
  binding registration, collision refusal and disable restoration also passed.
- The actual Linux PTY smoke passed using **F6/F7/F8**, including request,
  preview, one-shot acceptance, Unicode FIM, native undo, blank input, stale
  edit rejection, cancellation, risk refusal, typing while pending, Tab and
  history. The harness uses a temporary home and no profiles.
- Default Ctrl+Alt byte sequences were **not reliably delivered** by that PTY;
  the F-key fallback was used, rather than claiming those chords were tested.
- Windows terminal behavior is **not verified**. macOS 7.4.6/2.3.5 PTY coverage was added on 2026-10-31 (see VALIDATION.md). The same-text/new-prompt
  limitation above still applies, including on the tested Linux runtime.

Run the POSIX-only native smoke with an existing local PowerShell executable:

```sh
python3 integrations/powershell/tests/pty_smoke.py /path/to/pwsh target/debug/reedline-ai
```

The suite covers UTF-16/UTF-8 conversion, FIM suffix preservation, invalid and
stale replies, risk blocking, malformed/oversized output, argument boundaries,
nonblocking request dispatch, deadline, cancellation, blank zero-call behavior,
single-worker capacity, explicit preview/accept, one-shot acceptance, expiry,
cancel, and source/cursor/cwd/selection
staleness. Its editor boundary is stubbed: a passing suite is **not** an
interactive PSReadLine or Windows-terminal pass.

Native interactive acceptance remains a separate matrix: confirm custom keys
arrive; keep typing while pending; move cursor and edit suffix; request on blank
input; cancel with the dedicated chord; test undo/redo; verify native Tab,
history, arrows and Enter remain unchanged; verify key collisions are refused;
disable/re-enable; verify no candidate executes until the user intentionally
submits the command. Windows Terminal, Linux terminals and macOS terminals must
be reported separately. No SSH or mobile terminal validation is claimed. Windows PowerShell 5.1 is a
compatibility target, not a verified Windows-terminal result.

## Official API references

Reviewed against Microsoft's PSReadLine and .NET documentation:

- [Custom PSReadLine key handlers](https://learn.microsoft.com/en-us/powershell/module/psreadline/set-psreadlinekeyhandler)
- [Buffer, selection, insertion and cursor APIs](https://learn.microsoft.com/en-us/powershell/module/psreadline/about/about_psreadline_functions)
- [Looking up an existing key binding](https://learn.microsoft.com/en-us/powershell/module/psreadline/get-psreadlinekeyhandler)
- [ArgumentList](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.processstartinfo.argumentlist)
- [Process.Kill and descendant termination limitations](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.process.kill)

## Local history and older runtimes

On each explicit request, the newest 200 PSReadLine history items are searched
locally for an exact prefix/suffix match, including persisted history already
loaded by PSReadLine. Older modules without `GetHistoryItems` fall back to current
session `Get-History`; this fallback cannot warm-start from earlier sessions.
`Enable-ReedlineAI -NoHistory` disables this. No paths are guessed or history files
modified, and no history enters model context. A local match uses the same Rust
validation and risk gate and makes zero model calls.

Doctor can probe a selected runtime using `REEDLINE_AI_PWSH`; on Windows the
default probe is `powershell.exe`, elsewhere `pwsh`. Enabling checks the required
editing methods before changing bindings. ISE, Vi editing and constrained
language hosts remain outside the supported adapter contract.
