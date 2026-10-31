# Native shell bridge protocol v1

The standalone Reedline playground remains unchanged. Opt-in adapters call the same Rust backend without replacing the host shell or evaluating model output. `reedline-ai --adapter-request` consumes **one JSON object from stdin until EOF** and emits one JSON object to stdout, then exits. The caller must close stdin and enforce a wall-clock deadline. No JSON is passed through shell source or model-derived command arguments.

Request (all fields required):

```json
{"protocol":1,"request_id":"example-1","shell":{"kind":"bash","version":"5.2"},"cwd":"/synthetic/project","buffer":"echo 世","cursor":8}
```

- `shell.kind`: `bash`, `powershell`, `nushell`, or `cmd`, explicitly supplied by the adapter. Optional `version`, `known_commands`, `known_aliases` follow the existing shell metadata limits. Adapters must not infer the target from the backend host OS.
- `buffer`: single-line UTF-8, at most 8192 bytes; `cursor` is a UTF-8 byte boundary, not a screen column, UTF-16 offset, or grapheme index.
- `request_id`: 1–128 ASCII alphanumerics, hyphen, dot or underscore. Caller generates a fresh identity for each request.
- `cwd`: required nonempty single-line metadata, at most 4096 bytes. It is validated locally, **not read, echoed or sent to the model**. It does not opt into filesystem context.
- Entire JSON input is limited to 24 KiB; unknown fields are rejected. Errors do not echo the request or raw parser/provider diagnostics.

Response:

```json
{"protocol":1,"request_id":"example-1","buffer":"echo 世","cursor":8,"insertion":"界","result":"echo 世界","result_cursor":11,"risk_review":{"warnings":[],"incomplete":false},"additional_acceptance_required":false}
```

`result` is exactly prefix + insertion + suffix. `result_cursor` is the byte cursor after insertion. Adapters validate schema, request identity, source buffer and cursor, insertion safety, and result consistency before preview/accept. Empty or whitespace-only requests never invoke the backend. Adapters additionally skip starting a process for blank input.

Risk review always uses the conservative `confirm` rule in this protocol, even if `--risk-policy off` is passed. Warnings **or incomplete review** require an additional explicit gesture or are blocked by that adapter. `git` intentionally has incomplete review because hooks/aliases/config are unresolved. No warning is not a safety guarantee. Acceptance only changes the editor buffer; the user retains responsibility for execution via the host shell's normal Enter behavior.

History/configuration import flags and explicit CLI shell override conflict with adapter mode. Existing backend options (`--backend http`, endpoint/model/provider, key environment variable, network opt-ins, timeout) remain available. Default is the offline deterministic mock. Backend HTTP timeout defaults to eight seconds and is bounded to 1–60 seconds; callers also need process and stale-result deadlines. No adapter enables HTTP automatically or modifies profiles. Bash, PowerShell and Nushell may select a local native-history candidate as described below; it is never uploaded.

## Privacy and lifecycle

The current buffer and supplied shell metadata go to the selected provider only in explicitly configured HTTP mode. Do not invoke on secrets. Cwd stays local. An adapter may use private ephemeral transport files when its shell cannot safely exchange asynchronous stdin/stdout; its platform guide states this boundary. Nothing is written to native history by the adapter. Shell history may naturally record a command only when the user later executes it.

The initial native adapters provide explicit-key completion, not automatically refreshed inline AI ghost text. They do not yet connect the prototype's local workflow replacement/memory commands. Native editing, completion and history continue to belong to each real host shell.

### Optional local history candidate

Version 1 accepts optional `history_candidate: string`: one complete command
selected locally by a shell from at most its newest 200 entries. It is never
added to model `Context`. If it matches both the prefix and suffix, the Rust
bridge validates the insertion and returns it without calling the model;
otherwise the normal current-line-only backend path is used. Blank input still
makes no model call. Existing request payloads without this field work unchanged.
All response identity, insertion-only, size and risk checks still apply.
