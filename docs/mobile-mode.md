# Explicit mobile interaction

Run `reedline-ai --mode mobile --shell bash` on the **server** you already reach through your own terminal/SSH connection. Choose the actual server shell (`bash`, `powershell`, `cmd`, or `nushell`), not the phone OS. There is no SSH detection, login, Tailscale setup, server installation or command execution in this milestone. `--mode desktop` is the default and retains the existing Reedline keys, native history and editor.

For configured workflows:

```sh
reedline-ai --mode mobile --shell bash --config examples/project-memory.synthetic.json --risk-policy confirm
```

All example paths are synthetic. Mobile is a separate canonical, line-oriented terminal UI, not an embedded shell or a full-screen editor. No Ctrl, Alt, arrow or function keys are needed:

1. Enter `run CCTV`, then press Enter. This requests candidates, not execution.
2. Enter a listed number (`1` through `5`) to see the complete preview. A configured typed workflow shows its actual numbered steps, then the exact composed shell command. Explicit shell recipes are shown as one recipe, never split heuristically on separators.
3. Enter `accept`. With risk policy `confirm`, a risky or incompletely reviewed candidate requires a second, distinct `confirm` line. `confirm` alone cannot accept an unreviewed candidate.
4. The accepted text is printed and optionally stored in this application's explicitly selected history file. It is **never executed** or inserted into another shell. The candidate is then consumed.
5. `cancel` discards the candidate and pending request. `quit` exits. EOF also exits. A running HTTP call may take up to its configured timeout to shut down; cancellation cannot recall data already sent.

Other text commands:

- `edit TEXT` replaces the source text without calling a model. It also escapes reserved words or numeric input: `edit accept`, then `suggest` requests a completion for the literal word `accept`.
- `suggest` requests candidates for the current text again.
- `help` lists commands.
- `history-clear` (or `:history-clear`) clears app history and the completion cache, not provider records.

Any intervening input, including a number, blank line or `help`, invalidates an armed risk confirmation. Editing, retrying or canceling invalidates the entire candidate snapshot and any late response. Selecting a different candidate requires reviewing it again. Acceptance rechecks the complete original source and cursor, not just a prefix. Mobile reviews history candidates too; desktop native history behavior is unchanged. `warn` shows advisory findings without an extra confirmation; `off` hides them. No review is a safety guarantee.

## Local-first and network behavior

Up to five distinct recent local-history prefix matches win immediately. If none match, exact confirmed workflows and narrow shell-intent mappings are tried. Only then is the existing asynchronous completion backend used. Workflow memory remains deterministic and explicitly selected; this is not arbitrary AI planning or automatic project discovery. The mobile version completes at the end of the text; desktop retains cursor-aware FIM and native editing.

Mock remains the default and uses no network. In explicit HTTP mode, plain text + Enter or `suggest` requests a completion. Typing without Enter, `edit TEXT`, blank input, preview, accept and cancel never start an AI request. History upload remains separately opt-in with `--ai-history`; existing budget and exclusion rules apply. Leading spaces do not prevent upload of a submitted current line. Never enter secrets. Use the existing headless `--history-preview TEXT` to inspect history selection offline before enabling upload.

## Small-screen rendering and limits

Output is plain, terminal-control-sanitized text, wrapped to the current terminal width (up to 80 columns, 40-column fallback). Unicode display widths are respected. Candidate shortlists are bounded and abbreviated; selecting a number displays full command text before acceptance. Visual wrapping does not change the stored command or create real command newlines. Do not copy wrapped display lines into a shell as though they were an execution script.

The app bounds source text at 8 KiB and drains/rejects oversized input received by its reader. The terminal's canonical line discipline may have a **smaller line limit and may truncate a paste before the app sees it**. Keep mobile inputs short and inspect the full preview. Input echo/editing is managed by the terminal, not Reedline. A completion arriving while you type can interleave with terminal echo; press Enter to submit the new line, or use `cancel` followed by `edit TEXT`. No keystroke-level cancellation is claimed before Enter. No automatic prompt redraw or terminal capability negotiation is implemented.

Validation uses synthetic Linux POSIX PTYs at 20 columns, Unicode unit tests, all three risk policies, pending cancellation/editing, empty input, backend errors and EOF. Actual phone terminals, SSH/Tailscale links, Windows consoles and remote shells have not been tested in this milestone.
