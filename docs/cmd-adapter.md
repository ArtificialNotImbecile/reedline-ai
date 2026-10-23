# Windows CMD: optional Clink adapter

Status: **experimental, native Windows validation pending**. This is a Lua
adapter for actual `cmd.exe` with **Clink installed separately**. Stock CMD has no
supported equivalent editor-buffer API and is not supported by this adapter.
The standalone Rust demonstration is not represented as CMD integration.

## Prerequisites

- Windows, Clink **1.7.12+**, and its asynchronous prompt setting already enabled.
  Current official API documentation was checked against Clink 1.9.34.
- Python **3.10+** and a built `reedline-ai.exe`.
- A local temporary volume supporting persistent Windows ACLs. The helper must
  be allowed to create a protected temporary directory and join a Windows Job
  Object. Unsupported security policy/job nesting fails closed; do not weaken
  system security settings to make this run.
- Absolute drive-local Python/backend/helper paths without CMD expansion or
  metacharacters (`% ! ^ & | < > "`). Spaces are supported; UNC paths are not.

No prerequisites, autorun entries or profiles are installed automatically. Existing
Clink settings, completion, history, native editing and auto-suggestions remain
Clink's responsibility.

## Explicit session-only setup

Extract the bundled assets into a new directory:

```bat
reedline-ai.exe adapter doctor --shell cmd
reedline-ai.exe adapter install --shell cmd --experimental --directory C:\Tools\reedline-ai-cmd
```

In a fresh interactive CMD window without Clink already injected, using your
actual installed paths:

```bat
set "REEDLINE_AI_PYTHON=C:\Python\python.exe"
set "REEDLINE_AI_EXECUTABLE=C:\Tools\reedline-ai.exe"
set "REEDLINE_AI_ENABLE=1"
C:\Clink\clink_x64.exe inject --scripts C:\Tools\reedline-ai-cmd\cmd
```

For a checkout, point `--scripts` at `integrations\cmd` and retain the adjacent
`integrations\shared\bridge.py`. Quote paths containing spaces. This uses Clink's
documented session injection/script directory option, not `autorun install` or a
persistent script-path setting. If Clink is already injected, use a fresh window
rather than assuming reinjection reloads this script.

Loading the Lua file alone starts no requests. The explicit `ENABLE=1` opt-in
installs only previously unbound adapter keys at the next prompt. Conflicting
keys cause enable to fail. Advanced Clink Lua configurations may instead call
`reedline_ai_enable({python=..., backend=..., helper=..., arguments={...}})` from
their own script; these are separate arguments, never a command string. There is
no built-in CMD command that evaluates arbitrary Lua configuration.

The default backend is offline mock. Real providers require deliberate backend
configuration. Never put API secrets in CLI arguments or request suggestions for
a buffer containing secrets. The adapter reads no history or repository files.

## Request, preview, accept

Use these two-stroke sequences in Emacs or Vi insert mode. The second key is a
plain **lowercase** letter, not Ctrl+letter:

1. **Ctrl-X, g**: request one asynchronous suggestion.
2. **Ctrl-X, p**: collect and display its preview, without inserting anything.
3. **Ctrl-X, a**: insert that previously shown preview only if still current.
4. **Ctrl-X, x**: discard pending/preview state without changing the line.

Try `echo 世`; the mock result is `echo 世界`. Enter remains separate native CMD
execution. No `accept-line`, `eval`, suggested command execution or synthetic
Enter is used. Insertion is one native undo group and preserves the suffix.
Tab, history keys, arrows, Enter and Vi command mode are not rebound.

Blank/Unicode-whitespace-only input starts zero helpers/backend requests. Active
selections and multiline/control-character input are refused. Native cursor
positions are converted from Clink's one-based UTF-8 byte positions to the
protocol's zero-based byte offsets. Editing, moving the cursor, changing cwd or
starting a new prompt invalidates pending acceptance, including retyping the
same original text. Acceptance requires the exact matching request identity,
original input/cursor, insertion-only full result and result cursor.

Any risk warning, incomplete review, malformed review or
`additional_acceptance_required` blocks acceptance. There is no extra-confirmation
override, even if the CLI's general risk policy is off. `git st` is therefore
blocked by the mock's incomplete Git review. No warnings is not proof of safety,
especially with aliases or custom executables; inspect each preview.

## Async transport and privacy

Prompt coroutines and `io.popenyield` run the helper off the editor path. If
asynchronous prompts are disabled, no request starts. One worker is allowed;
cancellation discards the result but an in-flight worker drains before another
request can start. It has a **15-second backend deadline**, with bounded local
cleanup and process-start overhead. Requests are at most 24 KiB, input 8192 UTF-8
bytes, insertion 1024 bytes, and backend stdout 64 KiB. Backend stderr is dropped.

Clink's asynchronous process API is read-only and internally spools stdout to a
temporary file. Therefore this adapter sends **only an opaque temporary-directory
path and fixed status markers** over that channel, never the command buffer or
result. Request/response payloads instead use a freshly created private temporary
directory. Python creates it atomically with a protected DACL granting the current
user and SYSTEM inheritable full access. The request is opened exclusively,
validated, read, then removed before inference; the result file is read and
removed by Lua. Payload files are not secure-erased, and abnormal host/process
termination may leave a private directory for manual cleanup. Do not mistake
these files for stdin-only transport or the Clink status spool for DACL-protected
payload storage.

The helper passes JSON to the configured backend via stdin with `shell=False`.
Only explicitly configured paths and opaque temporary paths reach CMD's launcher;
buffer, candidates, cwd and backend options are never interpolated there. Before
spawning the backend, the helper joins a kill-on-close Windows Job Object so its
exit cleans descendants. This is containment for a trusted backend, not a general
sandbox. Native Windows ACL inheritance, Job behavior, terminal/Clink callbacks
and Unicode editing have **not yet been exercised in this Linux environment**.

## Disable and remove

```bat
set "REEDLINE_AI_ENABLE=0"
```

At the next prompt this removes only unchanged adapter-owned bindings. Later
user/plugin rebindings survive. The existing Lua `reedline_ai_disable()` hook is
also available to custom scripts. Setting `REEDLINE_AI_ENABLE=1` again enables
the adapter on the next prompt; a failed enable is not retried every prompt.
Allow in-flight work to finish before uninstalling:

```bat
reedline-ai.exe adapter uninstall --shell cmd --directory C:\Tools\reedline-ai-cmd
```

Closing the session removes session-only injection/configuration. There is no
registry/profile edit to undo from these instructions.

## Validation and remaining native gate

Portable tests currently cover the shared bridge's UTF-8/FIM/protocol/risk gates,
output caps, timeout, safe stdin, private-file cleanup and POSIX process-tree
cleanup. Mocked Lua tests cover asynchronous callback handoff, preview/accept,
staleness, native bindings, selections, risk rejection and session disable:

```sh
python3 -m unittest discover -s integrations/shared -p 'test_*.py' -v
texlua integrations/cmd/test_adapter.lua
```

The Lua test accepts an extracted script path as its first argument. These are
not Windows/Clink native passes. Before relying on this integration, run the key
flow in a real Windows CMD+Clink terminal, verify the private temporary-directory
DACL and inherited files, test timeout/host-close process cleanup, and check
Unicode middle insertion, undo, blank-zero requests, native Tab/history, Ctrl-C
staleness, disabled async mode, conflicting bindings and risk blocking.

## Official references checked

- [Clink Lua API, editor buffers, prompt coroutines and lifecycle callbacks](https://chrisant996.github.io/clink/clink.html).
- [Clink inject script-directory options](https://github.com/chrisant996/clink/blob/master/clink/app/src/loader/inject.cpp).
- [Clink asynchronous output-spooling implementation](https://github.com/chrisant996/clink/blob/master/clink/lua/src/io_api.cpp).
- [Windows directory creation security attributes](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createdirectoryw).
- [Windows Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects).
