# Shell-aware project memory (standalone prototype)

This is a standalone Reedline application and offline Rust CLI, **not** an installed Bash, PowerShell, CMD, or Nushell plugin. It never executes suggestions. No real user project paths or shell histories were inspected to build these examples.

## Quick start

```sh
cargo run --locked -- suggest --shell bash \
  --config examples/project-memory.synthetic.json --buffer 'run CCTV'
cargo run --locked -- suggest --shell cmd --buffer 'ls'
cargo run --locked -- suggest --shell powershell --shell-version 7.4 \
  --known-alias 'ls=Get-ChildItem' --buffer 'ls'
cargo run --locked -- --shell bash --config examples/project-memory.synthetic.json
```

The fixture's `/synthetic/projects/cctv` and `C:\Synthetic Projects\website` are placeholders, not discovered locations. The first preview replaces the whole phrase with:

```sh
cd -- '/synthetic/projects/cctv' && 'python' 'app.py'
```

The application does not run it, test whether those paths exist, install Python, or activate an environment. In interactive mode native history wins first. Tab shows a replacement in the existing Reedline menu, Enter accepts the selection, and a later Enter only prints the accepted line. Ctrl-F/Alt-F remain native suffix/word-hint controls; complex replacements never masquerade as suffix hints. Escape cancels the menu. An intervening buffer **or cursor** change invalidates the displayed replacement. The optional `--risk-policy confirm` adds F2 acknowledgment before Enter for generated/workflow candidates with warnings or incomplete review. Enter without acknowledgment does not insert and closes the menu; Tab reopens it. Editing, navigation, or cancellation invalidates acknowledgment. Whole/word AI hints cannot bypass this gate; native history hints/menu/search are unchanged. See [risk review](risk-review.md).

Blank or whitespace-only current input does not call AI, even with selected history. It removes queued work and makes in-flight results stale; a request already sent cannot be recalled. The default 180 ms debounce is unchanged.

## What is resolved

Only exact, normalized local facts:

- A workflow named `run` under project `CCTV` matches `run CCTV` and `run cameras` if `cameras` is a project alias.
- A workflow's `aliases` are complete phrases, such as `start CCTV` or `启动监控`.
- Matching trims/collapses whitespace and lowercases; executable paths and arguments are never normalized that way.
- Only `confirmed` participates. `suggestions` are inert, manually editable notes and are never automatically learned or promoted.
- Unknown projects/phrases get no guessed path or launch command. This is **not** arbitrary natural-language planning.
- Argument-free `ls` → `dir` and `pwd` → `cd` are offered for CMD; PowerShell uses `Get-ChildItem`/`Get-Location`. Explicit `--known-command`/`--known-alias NAME=TARGET` suppress these translations. Flags such as `ls -la` are deliberately not translated.

The HTTP API remains insertion-only; it does not generate structured replacements. The new structured replacements are local, deterministic workflows or the narrow intent translations above. `suggest` is always offline; where no local replacement matches, it can return the existing deterministic mock insertion. `--complete` retains its insertion-only behavior and core JSON fields, adds review metadata, and does not become a workflow resolver.

## Editable JSON and layers

JSON schema version 1 is used. TOML is not implemented. The provisional project directory is `.shell-ai/`, not a repository/package rename:

- Global: a user-selected file, e.g. a file in your normal private configuration directory.
- Project: `.shell-ai/config.json`, with root-relative paths and portable workflows.
- Local machine: `.shell-ai/local.json` (keep it out of Git), for personal absolute paths or platform-specific workflows.
- Explicit highest-precedence file: `--config PATH`.

Nothing is automatically searched in HOME, cwd, Git, or a parent directory. All selected files are explicit trust decisions. Precedence is:

```text
--config > --local-config > --project-config > --global-config
```

A higher layer replaces the **whole same-named project record**, rather than merging individual fields/workflows. Unconfirmed notes are appended and remain inert. Ambiguous aliases are rejected. The project layer requires `--project-root`, a caller-selected existing real directory, and accepts relative paths only. The root is verified/canonicalized; relative `..` cannot escape it. Target project paths are not probed. Other layers require explicit absolute paths. No environment-variable or tilde expansion occurs.

```sh
reedline-ai suggest --shell bash --buffer 'run CCTV' \
  --global-config /explicit/private/global.json \
  --project-root /explicit/worktree \
  --project-config /explicit/worktree/.shell-ai/config.json \
  --local-config /explicit/worktree/.shell-ai/local.json
```

The schema accepts no endpoint, API-key variable, upload toggle, executable import, or arbitrary extension keys. Repository data cannot enable HTTP/history upload, execute anything, or change risk policy. `--risk-policy warn|confirm|off` is explicit CLI-only configuration (default `warn`); neither these memory layers nor environment variables override it, and no global policy file is implemented. Files remain local; they are not attached to model requests. Explicit shell metadata is part of HTTP context if HTTP was selected. An accepted expanded command is ordinary current-buffer text: in opted-in HTTP mode it may be sent just like text you typed. Use offline mode for private paths/commands. Never put credentials or secrets in memory, alias arguments, or workflow source; there is no reliable secret detector.

Example project-only document (path `.` is anchored to the supplied root):

```json
{
  "version": 1,
  "confirmed": {
    "CCTV": {
      "path": ".",
      "aliases": ["cameras"],
      "environment": "video, descriptive only",
      "workflows": {
        "run": {
          "aliases": ["start CCTV", "启动监控"],
          "steps": [
            {"action": "change_directory"},
            {"action": "command", "program": "python", "args": ["app.py"]}
          ]
        }
      }
    }
  },
  "suggestions": []
}
```

## Management

Every command names its file. Show/list/validate never rewrite it. The management CLI validates raw documents, so a relative-path project document remains editable; resolution requires `--project-config` plus `--project-root` later.

```sh
reedline-ai memory --config /explicit/private/memory.json list
reedline-ai memory --config /explicit/private/memory.json show CCTV
reedline-ai memory --config /explicit/private/memory.json validate
reedline-ai memory --config /explicit/private/memory.json add CCTV --project-file /explicit/project-object.json
reedline-ai memory --config /explicit/private/memory.json update CCTV --project-file /explicit/replacement-project-object.json
reedline-ai memory --config /explicit/private/memory.json remove CCTV
```

`--project-file` contains one project object (`path`, `aliases`, `environment`, `workflows`), not the enclosing version/confirmed document. Add creates an absent config, but never creates parent directories. Update replaces the named project as a whole; remove removes only that confirmed project. Edit workflows and inert notes in your text editor, or replace a project with update. No autonomous history mining, cloud synchronization, or secret persistence exists.

Writes are bounded (256 KiB), validated first, same-directory temporary-file + atomic publication/replacement, and synced. On Unix files must be private mode 0600 with one hard link; the parent must be owner-controlled and not group/world writable. Symlink destinations/parents are rejected. If a manually created file has broader permissions, review it and explicitly fix permissions before using a write command. Reads do not silently chmod. Windows uses the user's existing directory ACLs: this prototype does not inspect/set Windows ACLs. Use a private user-owned directory. Standard-library checks are not a portable directory-handle compare-and-swap; a hostile process under the same account can still race replacement. No multi-writer transaction/merge guarantee is claimed.

## Shell lowering boundaries

- Bash: single-quoted literal arguments, apostrophe-safe escaping, quoted program names to avoid keyword parsing, `cd --`, ordered `&&` steps. Native aliases are not expanded for typed program names; use a shell recipe when alias/function-specific syntax is intended.
- PowerShell: `Set-Location -LiteralPath ... -ErrorAction Stop`, call operator, literal single-quoted arguments including all five recognized single-quote characters. Nested immediate `if ($?)` checks work without assuming PowerShell 7 pipeline-chain syntax. Empty or embedded-double-quote typed arguments are conservatively rejected because legacy native argv binding differs; use an explicitly authored, version-tested recipe. The application does not inspect `$PSNativeCommandArgumentPassing`.
- CMD: `cd /d`, `&&`, conservative quoted argv with conventional Windows trailing-backslash handling. `%`, `!`, `^`, and double quotes are rejected in typed values because expansion/quoting differs; control builtins require a recipe. Custom executable parsers and `.bat`/`.cmd` secondary parsing are not universally representable. Use explicit shell source for these cases; no arbitrary-argv guarantee is made.
- Nushell: raw-string literals with a collision-free delimiter and `^program` for external commands. A `do { ... }` scope can contain built-in directory steps followed by one final external command. Multiple external steps or external-then-cd are rejected unless supplied as one complete explicit recipe, rather than pretending historical external failure semantics are uniform. The temporary `do` scope does not promise to retain directory changes outside the generated recipe.

An explicit recipe is one step, for example `{"action":"shell","shell":"bash","command":"..."}`. It is offered only for the matching target shell, kept exactly as authored, and never mixed automatically with typed steps. Its author owns source semantics and failure handling. Validation checks schema, bounds and terminal-control safety, **not whether arbitrary code is benign**. The local advisory [risk review](risk-review.md) also examines the resulting preview, but it is not a full shell parser or safety guarantee. Unknown syntax, expansions, aliases, and unsupported programs may leave review incomplete; under `confirm`, incompleteness requires acknowledgment even without a specific warning.

`environment` is descriptive, not automatic `conda activate`. Activation needs shell integration and cannot be guessed for all shells. An explicitly configured `conda run -n NAME --cwd PATH -- PROGRAM ...` command can be used where installed and tested; this project neither discovers nor installs Conda. No path or environment name is inferred.

## Structured protocol

`suggest --format json` returns protocol version 1, explicit shell facts, `source_buffer`, `source_cursor`, `edit: {start,end,replacement}` (or null), resulting `result`/`cursor`, provenance/explanation/warnings and `preview_only: true`, `executes: false`, `network_request: false`. JSON also includes `risk_policy`, `risk_review: {warnings, incomplete}`, and `additional_acceptance_required`. Analysis remains in JSON even with policy `off`; the extra acceptance flag is true only for a present candidate requiring acknowledgment under `confirm`. A preview never records user acknowledgment or authorizes execution. All offsets are UTF-8 **bytes**. The library validates the whole original buffer and original cursor again at acceptance; clients must do the same, not just compare a prefix.

`--format line` returns exactly the result line and a decimal resulting byte cursor on two lines. Warnings and required-review notices go to stderr unless policy is `off`, leaving those two stdout lines unchanged. It is a preview convenience, not an identity-checked acceptance protocol. Never source/eval it. There is no adapter installation or automatic shell execution, no Python/jq dependency in either CLI format, and no model call from this subcommand.

## Verification scope and sources

Rust unit/integration tests cover all four dialect renderings, rejected values, structured Unicode edits, protocol/CRUD/layers, no-request blank input, and actual Reedline replacement/stale guards. Harmless synthetic Bash syntax/literal roundtrips run through Bash. PowerShell, CMD and Nushell runtime execution are not claimed verified here; generated syntax tests are not native-shell integration tests.

Language design references: [Bash quoting](https://www.gnu.org/software/bash/manual/html_node/Quoting.html), [PowerShell quoting](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_quoting_rules), [PowerShell native argument parsing](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_parsing), [PowerShell chaining equivalence](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_pipeline_chain_operators), [Nushell strings](https://www.nushell.sh/book/working_with_strings.html), [Nushell 0.98 error behavior](https://www.nushell.sh/blog/2024-09-17-nushell_0_98_0.html), [Conda run](https://docs.conda.io/projects/conda/en/26.3.x/commands/run.html).
