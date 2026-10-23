# Local command-risk review

The standalone application reviews the **resulting command text** of generated insertions and deterministic workflow/replacement previews locally. It never executes a command, runs an extra model request for review, or installs a shell adapter. Review is a bounded lexical advisory check, not an exhaustive shell parser, malware detector, execution authorization layer, or guarantee of safety.

## Choose policy explicitly

The root CLI (interactive mode and `--complete`) and the offline `suggest` subcommand accept `--risk-policy`:

- `warn` (default): show advisory status/menu descriptions without an additional acceptance step.
- `confirm`: require an additional acknowledgment for a generated/workflow candidate if review has **any warnings or is incomplete**.
- `off`: suppress review UI warnings and the additional acceptance gate. JSON still reports the analysis as metadata.

```sh
# Interactive, explicit target shell and acknowledgment policy
cargo run --locked -- --shell bash --risk-policy confirm

# Insertion-only preview; no command execution
cargo run --locked -- --shell bash --risk-policy warn --complete 'git st'

# Deterministic offline workflow preview
cargo run --locked -- suggest --shell bash --risk-policy confirm \
  --config examples/project-memory.synthetic.json --buffer 'run CCTV'
```

Policy comes only from this explicit CLI choice and its default. There is no risk-policy environment override, automatic configuration discovery, or global policy file. The global/project/local/explicit **project-memory layers do not control policy**; repository configuration cannot weaken it. `--global-config` is a workflow-memory file, not security policy.

## Interactive acceptance

For a candidate that needs acknowledgment under `confirm`:

1. Press Tab to preview/select the candidate in the Reedline completion menu.
2. Read the resulting text and review description, then press F2 to acknowledge that candidate.
3. Press Enter to insert it into the editable buffer. A later Enter only prints the accepted text; it does not execute it.

Pressing Enter before F2 does not insert the guarded candidate. The native menu closes; press Tab to reopen it before acknowledging. Acknowledgment is tied to the preview and input state, not a blanket approval for future suggestions. Editing, navigation, changing the candidate, or canceling invalidates it.

Whole-candidate and word-level AI hint acceptance cannot bypass `confirm`: candidates requiring acknowledgment are routed through the menu. Native history hints, native history menus, reverse search, and manually entered commands are not intercepted or reclassified by this feature. It is not protection for every way text can enter a shell or terminal.

## What the review can and cannot see

The analyzer uses explicit Bash, PowerShell, CMD, or Nushell context, with a bounded scan of quoting, command boundaries, command names, arguments, and redirection. It recognizes selected obvious deletion, disk-formatting, overwrite/redirection, and Git work-discard patterns. Quoted literal text is not simply treated as another executable command, but dialect coverage is deliberately limited.

Review does not resolve the actual filesystem target, command implementation, shell functions, PATH, runtime aliases, variable values, wrappers, scripts, hooks, or external program behavior. Unsupported programs/syntax, expansions, known aliases, malformed or overly complex input, and exceeded analysis bounds can leave `incomplete: true`. Missing shell context is explicitly incomplete; it is not silently certified using the host operating system. Even common commands can have incomplete review, including Git, whose aliases/configuration/hooks are not resolved. Rendered workflow syntax can also exceed the reviewer's narrow subset.

Warnings may over-report and harmful commands may be missed. An empty `warnings` array means only that no configured warning was found. `incomplete: false` means the bounded checker handled its supported lexical subset, **not that execution is safe**. F2 records acknowledgment, not proof of safety. Always inspect the complete command before copying it into a real shell.

Terminal-control filtering, input/output size limits, stale-candidate checks, and no-execution behavior remain separate safeguards. The review adds no network call and does not change the existing opt-in HTTP/history-upload rules. Review findings and preview commands are visible output, so avoid secrets in command-line arguments or terminal logs.

## Preview metadata

Both `--complete` JSON and `suggest --format json` preserve their existing core fields and add:

- `risk_policy`: `"warn"`, `"confirm"`, or `"off"`.
- `risk_review`: an object containing `warnings` (strings) and `incomplete` (boolean).
- `additional_acceptance_required`: whether an actual generated candidate needs acknowledgment under `confirm`. It is false for `warn`, `off`, or no candidate.

These are preview metadata, not a persisted acceptance token or execution permission. Analysis remains available with `off`. A consumer must not interpret `additional_acceptance_required: false` as a safety verdict. For structured edits, consumers must still validate the complete original source buffer and UTF-8 byte cursor before accepting a replacement.

`suggest --format line` keeps result text and byte cursor on two stdout lines. With `warn` or `confirm`, advisory findings and required-acceptance notices use stderr. It does not provide the structured JSON acceptance contract. Never source or eval either output format.

## Scope of validation

This document describes the intended local CLI/editor contract, not a claim that every shell, command, or terminal has been runtime-validated. Use the repository's local tests and PTY checks to verify the specific build. Lexical unit tests and generated-string assertions cannot establish native-shell semantic safety or comprehensive detection. There is no mobile interaction mode, SSH workflow, shell adapter, global-policy-file support, or extra model-based classifier in this milestone.
