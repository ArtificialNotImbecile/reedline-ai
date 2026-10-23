# Nushell: opt-in native editor adapter

Status: **experimental**. This edits the real Nushell command line, not a
standalone imitation shell. It uses supported `commandline` APIs and experimental
background-job mailboxes. Linux Nushell **0.116.1** has native PTY coverage;
older Nu releases and Windows/macOS terminal behavior are not certified.
SSH integration is separate and remains deferred.

## Requirements and session-only setup

- Nushell 0.116.1 with `job spawn`, `job recv --timeout`, `job send`, and `job flush`.
- Python **3.10+**, the trusted bundled `shared/bridge.py`, and `reedline-ai`.
- A terminal that delivers Ctrl+Alt letters. Existing bindings on these keys or
  reserved adapter names cause enable to fail without changing any bindings.

The installed binary can extract its bundled assets without a source checkout:

```nu
reedline-ai adapter doctor --shell nushell
reedline-ai adapter install --shell nushell --experimental --directory '/absolute/new/nu-adapter'
```

Wait for extraction to finish before evaluating the next block. Nu resolves
`source` during parsing, so installing and sourcing a new file in one pasted
block fails before installation runs. Then enable:

```nu
$env.REEDLINE_AI_EXECUTABLE = '/absolute/path/to/reedline-ai'
source '/absolute/new/nu-adapter/nushell/reedline-ai.nu'
reedline-ai-enable
```

For development, source `integrations/nushell/reedline-ai.nu` from the checkout
instead. Keep its sibling `integrations/shared/bridge.py` in place. Sourcing alone
starts no request and binds no keys. Nothing edits `config.nu`, `env.nu`, shell
profiles, history, or SSH configuration, and nothing installs prerequisites.

Optional settings, set before enabling:

```nu
$env.REEDLINE_AI_PYTHON = '/absolute/path/to/python3'
$env.REEDLINE_AI_ARGUMENTS = ['--backend' 'mock']
# Only needed if moving the helper away from its bundled sibling location:
$env.REEDLINE_AI_TRANSPORT = '/absolute/path/to/shared/bridge.py'
```

The default backend is the offline mock. Provider use requires deliberate CLI
configuration; never put API secrets in the argument list. A remote request sends
the explicitly requested current buffer, not harvested history. The protocol also
contains local cwd and shell identity; the Rust backend does not add cwd to model
context. Do not request suggestions for a buffer containing secrets.

## Keys

1. **Ctrl+Alt+G** starts one asynchronous request for a nonblank line. Keep typing.
2. **Ctrl+Alt+P** collects a ready result and prints a JSON-escaped preview.
   If still pending it reports that without waiting for inference.
3. **Ctrl+Alt+A** inserts only an already shown, still-current safe preview.
   This never submits the line. Native **Enter** is a separate action.
4. **Ctrl+Alt+X** discards pending/preview state without changing the input.

Try `echo 世` with mock mode: request, preview, then accept should produce
`echo 世界`. `git st` is deliberately blocked because its risk review is
incomplete. The adapter has no extra risk-confirmation override: any warning,
incomplete review, missing/malformed review, or `additional_acceptance_required`
blocks acceptance, regardless of the CLI's general risk-policy setting.
Risk review is not a security guarantee; inspect every preview.

Tab, Enter, arrows, history search and existing completions remain native.
Bindings apply to Emacs, Vi insert and Vi normal modes. The adapter converts Nu's
grapheme-cluster cursor to protocol UTF-8 bytes and back, rejecting an insertion
whose result cursor splits a grapheme. Middle-of-line suffixes are preserved.

## Lifecycle, bounds and privacy

Only a key starts work. Blank input starts zero helpers/backend requests. Input
must be single-line, at most 8192 UTF-8 bytes, with no terminal/bidi controls.
Responses must match the request ID, exact original text/cursor, insertion-only
result and result cursor; insertion is limited to 1024 bytes. Code fences and
control characters are rejected. The current line, cursor, cwd and prompt must
still match when previewing or accepting. A new prompt, including native Ctrl-C,
discards old state even if identical text is retyped.

One Nu background job calls Python using JSON stdin, not shell interpolation.
Request/result payloads remain in memory. The helper bounds the request to
24 KiB, backend stdout to 64 KiB, discards stderr, and imposes a **15-second
backend deadline**, plus bounded local cleanup and process-start overhead. Nu's
`complete` only captures this bounded helper output, not arbitrary backend output.
No suggested text is evaluated and `commandline edit --accept` is never used.

Cancel discards the UI result, but lets an already sent request drain to its
deadline. New requests are refused until that single worker finishes. Disabling
also discards state; it does not pretend to recall data already sent remotely.
On POSIX, a small helper shim and private lifeline pipe keep a cleanup supervisor
alive if Nu hard-kills the shim during shell exit; it kills the backend process
group. On Windows the helper requires a kill-on-close Job Object before spawning
the backend, failing closed if assignment is unavailable. These are cleanup
controls for a trusted executable, not a sandbox against malicious programs
deliberately escaping supervision. Windows Job behavior still needs native tests.

## Disable and remove

```nu
reedline-ai-disable
reedline-ai adapter uninstall --shell nushell --directory '/absolute/new/nu-adapter'
```

Disable removes only unchanged adapter-owned bindings; later user/plugin changes
survive. One inert-when-disabled pre-prompt hook remains for this session so other
hooks are never removed. Let in-flight work drain before removing files. No
profile changes need reversing unless you independently added them yourself.

## Validation

Run without provider access, using an available real Nu executable:

```sh
python3 -m unittest discover -s integrations/shared -p 'test_*.py' -v
NU_BINARY=/path/to/nu REEDLINE_AI_TEST_BINARY=/path/to/reedline-ai \
  python3 -m unittest discover -s integrations/nushell -p 'test_*.py' -v
```

The Nu suite covers Unicode/grapheme conversion, middle insertion, preview-before-
accept, no execution, blank-zero calls, editing during inference, cursor/edit/
prompt staleness, cancellation/drain, risk/malformed/flood rejection,
timeout, native history/Tab/editing, rebindings and actual shell-exit cleanup.
The optional Rust mock test checks `echo 世` and blocked `git st`. Missing runtime
or platform capabilities are reported as skips, never as native passes. Set
`RAI_NUSHELL_ADAPTER_PATH` and `RAI_SHARED_BRIDGE_PATH` to test extracted assets.
Real Windows/macOS Nu terminals, OS key delivery and installed Python security
policies remain a manual validation gate.

## Upstream references checked

- [Nu commandline edit](https://www.nushell.sh/commands/docs/commandline_edit.html)
  and [line-editor bindings](https://www.nushell.sh/book/line_editor.html).
- [Background jobs and mailbox/lifecycle behavior](https://www.nushell.sh/book/background_jobs.html).
- [Actual grapheme cursor implementation](https://github.com/nushell/nushell/blob/0.116.1/crates/nu-cli/src/commands/commandline/get_cursor.rs).
- [Python subprocess lifecycle](https://docs.python.org/3/library/subprocess.html).
