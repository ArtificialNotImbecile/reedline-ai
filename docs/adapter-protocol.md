# Native shell bridge protocols v1 and v2

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

History import flags and explicit CLI shell override conflict with adapter mode. Explicit configuration flags (`--config`, `--global-config`, `--project-config` with `--project-root`, `--local-config`) are accepted and remain local. Existing backend options (`--backend http`, endpoint/model/provider, key environment variable, network opt-ins, timeout) remain available. Default is the offline deterministic mock. Backend HTTP timeout defaults to eight seconds and is bounded to 1–60 seconds; callers also need process and stale-result deadlines. No adapter enables HTTP automatically or modifies profiles. Bash, PowerShell and Nushell may select a local native-history candidate as described below; it is never uploaded.

## Privacy and lifecycle

The current buffer and supplied shell metadata go to the selected provider only in explicitly configured HTTP mode. Do not invoke on secrets. Cwd stays local. An adapter may use private ephemeral transport files when its shell cannot safely exchange asynchronous stdin/stdout; its platform guide states this boundary. Nothing is written to native history by the adapter. Shell history may naturally record a command only when the user later executes it.

The initial native adapters provide explicit-key completion, not automatically refreshed inline AI ghost text. Version 2 connects exact, confirmed local workflow phrases from explicitly selected configuration; it does not discover projects or learn from history. Native editing, completion and history continue to belong to each real host shell.

## Version 2: explicit local whole-line workflows

Version 1 is unchanged and insertion-only. Version 2 requests use the same fields
with `protocol: 2`; replies must echo the requested version. Ordinary backend
suggestions still obey the exact v1 insertion equations and limits. A matched
confirmed workflow instead emits `replacement` (a string), empty `insertion`,
`result == replacement`, and `result_cursor == UTF-8 byte length of replacement`.
Replacement is a full-line edit, at most 8192 UTF-8 bytes, with the same control
character/code-fence rejection as structured edits. A `replacement` field is
forbidden in v1, including null or empty variants. A workflow match requested
through v1 fails closed rather than becoming an insertion or an HTTP request.

The native adapters request v2. Both transports validate the mutually exclusive
replacement/insertion forms; Clink uses the `RLA2R` frame marker for replacements
and retains the `RLA1` insertion frame. Request identity, complete original
buffer, original cursor, current directory, selection, generation, cancellation
and expiry gates still apply. Acceptance changes the buffer in one edit only;
it never sends Enter. PowerShell converts the accepted full line to UTF-16 editor
indices; the wire cursor remains UTF-8. Risk review cannot be disabled by CLI flags.

Only explicit configuration is loaded, with existing global → project → personal
→ explicit override precedence. No cwd-based discovery, filesystem scan, raw
history, unconfirmed memory or implicit environment activation is added. The
configured workflow result stays local and skips the backend entirely. A phrase
not matching a confirmed workflow follows ordinary completion; memory and cwd
are not included in its provider context. Protocol v2 does not add provider-driven
rewrites or natural-language path discovery.

For now pass config flags through the documented adapter backend-argument array,
for example Bash: `REEDLINE_AI_ARGS=(--backend mock --config /absolute/path/memory.json)`
before sourcing the adapter. Use absolute paths so changing directories cannot
select a different configuration. `init` does not yet persist workflow settings.

### Optional local history candidate

Versions 1 and 2 accept optional `history_candidate: string`: one complete command
selected locally by a shell from at most its newest 200 entries. It is never
added to model `Context`. If it matches both the prefix and suffix, the Rust
bridge validates the insertion and returns it without calling the model;
otherwise the normal current-line-only backend path is used. Blank input still
makes no model call. Existing request payloads without this field work unchanged.
All response identity, insertion-only, size and risk checks still apply.
