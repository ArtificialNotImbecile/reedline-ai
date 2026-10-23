# Roadmap and implementation status

The immediate product is a standalone, native-first Reedline prototype that saves keystrokes. It is not a new shell, chat agent, Nushell fork, or universal editor plugin. The repository/package name remains `reedline-ai`; `.shell-ai/` is a provisional configuration directory name only.

## Implemented in this stage

- Explicit Bash/PowerShell/CMD/Nushell identity, optional version, explicitly supplied available commands/aliases. No inference from OS, PATH, parent process, or real user history.
- Existing native Reedline history, hint/word acceptance, search, completion menu and undo behavior preserved. Native results win; complex local replacements appear only as menu previews. Accepting never executes.
- Empty/whitespace-only current buffers make no AI request, cancel queued work and suppress stale results. Already-sent requests cannot be recalled. Existing 180 ms debounce retained pending user experience.
- Local JSON version-1 confirmed project memory separate from inert inferred notes. Explicit list/show/validate/add/update/remove, bounded reads, atomic private-file writes and documented platform limits.
- Explicit global/project/local/CLI-file layering. Project-relative paths require a supplied verified root; personal absolute paths belong in local/global files. No automatic repository discovery yet. Config cannot enable network, choose credentials, upload history, or execute/import code.
- Deterministic `run <project>`/configured exact phrases, narrow Bash-intent-to-CMD/PowerShell examples, shell-specific conservative rendering, no guessed paths/activation.
- UTF-8 byte-range replacement/insertion edits tied to the full source buffer and cursor. Headless offline JSON protocol; old `--complete` insertion contract remains.
- Bounded local shell-aware advisory review of generated/workflow suggestions: CLI-only `--risk-policy warn` (default), `confirm`, or `off`; JSON review metadata and explicit F2 acknowledgment before guarded menu insertion. Incomplete reviews also require acknowledgment under `confirm`. Native history is not intercepted, and no command is executed or extra model called. No global policy file or environment/repository policy override is implemented. This is not comprehensive malicious-command detection or a safety verdict.
- HTTP receives supplied shell metadata but remains insertion-only. No arbitrary freeform AI workflow planner, model-generated structured replacement, or automatic learning is implemented.

See [project memory](project-memory.md) and [risk review](risk-review.md) for exact syntax, rejects and verification limits. Tests of rendered strings are not proof of native-shell compatibility; only harmless Bash parser/literal checks run in this cloud Linux stage.

## Explicit mobile mode (implemented)

`--mode desktop|mobile` selects interaction explicitly. Desktop retains Reedline. Mobile provides bounded, narrow-screen numbered candidates, actual typed workflow steps, ordinary text accept/cancel and an extra text confirmation under risk policy `confirm`. Existing local-first completion and async backend are reused. No execution or SSH inference occurs. See [mobile-mode.md](mobile-mode.md) for canonical-terminal limits and exact coverage.

## Native adapter milestone

Opt-in adapters now share a bounded Rust stdin JSON bridge with explicit shell/cwd/buffer/cursor identity. The [Bash guide](bash-adapter.md) records the real bind-x request/preview/accept UX and isolated PTY coverage. This is not an inline predictor and does not replace native editing, completion or history. Model suggestions remain insertion-only; local structured workflow/memory integration is still future work for the adapters.

PowerShell and Nushell now have explicit-request experimental adapters with separate Linux native/installed-runtime tests. CMD+Clink has an experimental optional-dependency adapter, portable transport tests and Lua mocks only; Windows native execution remains unverified. All are bundled for explicit `--experimental` extraction, not automatic enabling. Do not interpret protocol or Bash tests as proof of other platforms. Real SSH work is deferred until the user provides an authorized environment.

## Platform API references and future predictor research

- Bash/Readline: an opt-in `bind -x` command can read/edit `READLINE_LINE` and `READLINE_POINT`; a full asynchronous inline predictor is separate work. Stock programmable completion is not an as-you-type predictor API. [Bash manual](https://www.gnu.org/s/bash/manual/bash.html)
- PowerShell: compiled `ICommandPredictor` module registered via SubsystemManager, normally PowerShell ≥7.2 and PSReadLine ≥2.2.2. Preserve HistoryAndPlugin/native behavior. Windows PowerShell 5.1 is not this plugin route. [Official predictor guide](https://learn.microsoft.com/en-us/powershell/scripting/dev-cross-plat/create-cmdline-predictor)
- Nushell: current hinter closure appeared in 0.112.1; feature/version-check and smoke-test before installing it. Cache/nonblock output and explicitly preserve native history fallback. Installing a binary does not replace Nu's compiled-in Reedline. [Line editor](https://www.nushell.sh/book/line_editor.html#hints), [0.112.1 release](https://www.nushell.sh/blog/2026-04-11-nushell_v0_112_1.html)
- CMD: stock shell lacks a comparable documented predictor plugin interface. A separate explicitly installed Clink Lua adapter is a possible route, not native CMD support delivered by this binary. [Clink suggestions](https://chrisant996.github.io/clink/clink.html#customizing-suggestions)

## Bounded SSH feasibility validation (offline contract only)

The isolated [SSH feasibility experiment](ssh-adapter-feasibility.md) validates local helper/relay/fallback contracts with a mock transport; real SSH, forwarding, interactive fallback and shell integration remain unverified. Explore an opt-in connection wrapper that invokes existing OpenSSH, not a protocol/client fork or a global `ssh` override. A temporary remote shell helper/adapter could carry explicitly selected remote shell/cwd facts back to an authorized company-side model relay; credentials remain on the appropriate company side rather than being copied to field hosts. Test only in an isolated environment with explicit trust, clean teardown, and ordinary SSH fallback when unsupported. No automatic upload/install on real hosts, company/field host access, SSH/Tailscale changes, or network configuration is authorized by this roadmap. Long paths must come from explicit memory or bounded verified discovery, never a model invention or a full-disk scan.

## Distribution

Local `cargo install --path . --locked` installs a binary with bundled native adapter assets. Bash is the first Linux-tested path; other shells require explicit `--experimental` extraction. `reedline-ai adapter install --shell bash --directory NEW_DIRECTORY` extracts them explicitly; it never edits profiles or enables itself. See [installation and removal](install.md). `cargo install --git ... --rev VERIFIED_COMMIT --locked` requires that revision to be published. No crates.io publication, apt repository/.deb, automatic shell setup, or name/trademark clearance is claimed. Keep source-build distribution truthful until each packaging route is created and verified. No hosted CI or paid model calls are part of local validation.
