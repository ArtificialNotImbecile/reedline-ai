# MVP validation record

Local validation: 2026-10-23, Linux x86_64, Rust 1.99.1, Reedline 0.52.1.

Commands run:

- `bash scripts/ci-local.sh` (local-first aggregate runner)
- `cargo fmt --all -- --check`
- `cargo clippy --locked --all-targets -- -D warnings`
- `cargo test --locked`
- `cargo test --locked --release`
- `cargo build --locked --release`
- Headless CLI suffix, FIM and Unicode examples in README
- `python3 scripts/pty_smoke.py`
- `python3 scripts/pty_smoke.py target/release/reedline-ai`

The POSIX PTY harness drives the actual binary, responds to terminal cursor-position queries, and checks accepted text for suffix hints, FIM menus and Unicode. It also checks Ctrl-C and clean exit. The default-color regression cases cover bracketed-paste terminal-control rejection, editing plus hint acceptance in one input batch, and repeated hint acceptance through Ctrl-F, Right, Ctrl-E and Ctrl-Right. It is not a visual screenshot review, a graphical terminal emulator matrix, or proof of compatibility with all terminals.

The HTTP tests use a local disposable TCP service. They check request content/auth, infill context, response parsing, body limits, timeouts, redirect refusal, and redacted errors. No real provider credentials, remote model inference, purchases or paid API requests were used.

The DeepSeek opt-in preparation adds localhost coverage for `thinking: {"type":"disabled"}`, unchanged generic payloads, numeric-only token usage, CLI option boundaries, and environment proxy opt-in versus default bypass. Explicit reqwest retries are disabled. The updated local pipeline passed all 34 Rust tests in both debug/release, all 9 offline Python tests, formatting, Clippy, builds, headless examples, and both PTY smoke runs. Python tests use dummy in-memory responses, never real credentials or remote APIs. The live runner is not part of CI; its endpoint/model and dated price assumptions are documented in README.

An unauthenticated connectivity diagnostic on 2026-10-23 reached the official DeepSeek endpoint and received the expected HTTP 401 using explicit environment proxy/platform CA opt-ins, with the API key removed from that process. This checks transport only, not authenticated inference. TLS verification remained enabled; no proxy URLs or CA paths were logged.

## Limited real-provider smoke test

On 2026-10-23, a separate manual run against the official DeepSeek `deepseek-flash` chat endpoint, with thinking disabled and explicit environment proxy/platform CA opt-ins, completed three fixed synthetic cases:

- Suffix: `git st` + `atus` to `git status`; 113 prompt tokens, 1 completion token; 6,120 ms.
- FIM: `git ` + `status` + ` --short` to `git status --short`; 113 prompt tokens, 1 completion token; 4,410 ms.
- Unicode: `echo 世` + `界` to `echo 世界`; 114 prompt tokens, 1 completion token; 4,100 ms.

All three preserved the input context and matched the illustrative expected insertion. Nothing was executed. Each request used a fresh CLI process and HTTP connection; the reported elapsed times include process/network overhead and are not a latency benchmark.

The successful batch reported 340 prompt tokens and 3 completion tokens. At the dated peak uncached prices documented in README, its usage-based upper estimate is CNY 0.000704, not a verified bill. An earlier direct-network attempt failed without returned token usage and is excluded from that estimate; its billing outcome is unknown. There were no automated retries. This small sample does not establish broad completion quality, provider reliability, shell safety, or compatibility with other models.

## Native-first iteration

The 2026-10-23 native-first iteration restores a 10,000-entry native local history store, DefaultHinter prefix matching, history navigation/search/menu and native editing semantics. AI is a fallback, not a replacement or asynchronous reordering of local suggestions. History upload is separately OFF by default; all history fixtures in automated validation are synthetic.

The final local aggregate passed on Linux: formatting, Clippy (`--all-targets -D warnings`), 62 Rust tests in each of debug/release including the latency example tests, 11 offline Python tests, debug/release builds, three headless examples, and both AI and native PTY suites against debug and release binaries. The PTY harness allows a 250 ms render/terminal-processing window; this is a test synchronization allowance, not a measured native completion latency. No hosted workflow or remote model API was invoked by this pipeline.

New regressions cover immediate local hints and FIM while AI debounce is 10 seconds; next-word acceptance; Up/Down and Ctrl-P/N; Ctrl-R accepted through Enter and Ctrl-F; undo; selection guards; active-menu Ctrl-D/Ctrl-F/Escape; stale/repeated/mixed full-word keys in one batch; persistence, owner-private Unix creation/recreation, and read-only previews; opt-in history on actual localhost HTTP payloads; deduplication/exclusions; conservative UTF-8/JSON-budget accounting and zero connections on over-budget rejection. The production budget is explicitly an estimate, not measured tokenizer tokens.

Independent review found and corrected native-mode regressions in the first iteration. Hint acceptance now stages only the once-rendered text and returns the original native Reedline hint event, so Reedline itself checks selection, cursor, menu and undo behavior. Later parsed edits cannot revoke an earlier staged acceptance; no hint is inserted directly by the wrapper. Native search limits remain on the ordinary safe-history path, so a prefix hint does not clone the entire 10,000-entry history on every keypress. Unsafe persisted entries are filtered without rendering their controls.

Optional persistence still inherits the upstream file backend's automatic destructor sync. It is intended for a trusted, stable, app-owned path, not as a race-safe security boundary against another local process replacing that path. In-memory default history avoids this filesystem issue. Existing file permissions are not silently changed.

See the separate [15-request synthetic latency report](latency-benchmark.md). Those paid measurements were manually run outside local CI; a local CI pass never invokes them.

## Scope and review

The initial published commit `459b8747ae6b7b605826b3f24f635ba6c0d65b44` passed [hosted checks on Linux, macOS and Windows](https://github.com/ArtificialNotImbecile/reedline-ai/actions/runs/39926660746), including Linux PTY smoke, on 2026-10-23. Hosted CI is now manual-only (`workflow_dispatch`), with no push or pull-request trigger. Routine validation uses `bash scripts/ci-local.sh` on Linux/macOS, without starting hosted workflows or calling remote model APIs. A local pass verifies that host only; it does not establish a new cross-platform result.

Known limitations remain intentional: no automatic command execution, no universal inline shell predictor, no native FIM-token adapter, no streaming or active HTTP cancellation, and no guarantee a model's suggestion is semantically correct or safe to copy into a shell. See README for privacy and resource limits.

Independent review found two bugs in the first draft: raw pasted controls could reach default ANSI rendering before validation, and Reedline input batching could accept stale or repeated hints. The iteration introduced pre-render input filtering and input-ordered, one-shot rendered-hint acceptance, with dedicated Rust and PTY regression coverage. This record describes checks, not a formal security certification.

Dependency advisory check: `cargo-audit` 0.22.2 checked the lockfile on 2026-10-23 against 1,349 loaded RustSec advisories. It reported zero known vulnerabilities and no warnings. This is a dated advisory lookup, not a guarantee that dependencies contain no vulnerabilities; rerun against the current database before a release.

## Mobile milestone

`bash scripts/ci-local.sh` now includes `scripts/pty_mobile.py` against debug and release builds, using 20-column Linux POSIX PTYs, synthetic project memory, all three risk policies, Unicode previews, cancellation/edit invalidation, empty input, ordinary-text quit and EOF. The script verifies that accepting a synthetic `touch` candidate does not create its target file. Unit tests cover bounded sanitized Unicode wrapping, line-reader errors/draining, backend errors, stale results, exact typed workflow steps, local-history priority and no blank AI calls. Desktop smoke suites remain in the aggregate gate. These checks do not validate actual phone apps, SSH/Tailscale, Windows terminals or non-Bash execution.

## Bounded SSH adapter contract experiment (2026-10-23)

The isolated [SSH feasibility harness](ssh-adapter-feasibility.md) passed eight
new offline contract tests. It executes a fixed local helper in a synthetic cwd
and uses an in-process relay; it does not connect over SSH, deploy to a remote
host, open a listener, use credentials, or validate interactive fallback. Real
SSH/forwarding/PTY integration remains unverified pending an approved disposable
server/authentication fixture.

The final local aggregate passed on Linux: 27 Python tests; 118 Rust tests in
each debug/release profile (one existing ignored test per profile); formatting,
Clippy, both builds, headless examples, and AI/native/risk/mobile PTY suites for
both builds. No hosted workflow or real model API was invoked.

An initial aggregate attempt failed an existing batched-hint PTY assertion:
`git st` was submitted before the expected `git status` completion. The test
previously waited a fixed 250 ms rather than requiring an asynchronously rendered
hint. Its setup now waits up to three seconds for a fresh visible `atus` hint;
regression input batches and acceptance assertions remain unchanged and are not
retried. A focused PTY run and the final full aggregate passed. This fixes a test
synchronization gap, not a claimed production bug or latency guarantee.

## Native Bash and self-contained installation milestone (2026-10-23)

The final local aggregate passed on Linux Bash 5.2.37 / Python 3.12.14:
27 Python unit tests; 131 Rust tests in each debug/release profile (one existing
ignored test per profile); formatting, Clippy, both builds, headless examples,
and all previous AI/native/risk/mobile PTYs. New real-Bash PTYs pass against both
builds and a separately installed binary. No provider API or hosted CI ran.

The clean-HOME installation gate performs offline `cargo install --path . --root
TEMP --debug --locked --offline`, runs the installed binary's doctor, extracts
bundled assets to a path containing spaces, sources those extracted assets in a
profile-free real Bash, exercises the full PTY suite including disable, and
uninstalls the unchanged assets. It asserts no profile or history file was
created. The runtime adapter does not depend on keeping the checkout.

Coverage includes native Tab/history/editing, asynchronous request and separate
preview/accept, risk acknowledgment, Unicode cursor conversion and preserved
suffix, blank zero-call behavior, malformed/oversize/control/fenced responses,
stale buffer/cursor/cwd/new-prompt rejection, Ctrl-C and terminal/SIGHUP cleanup,
no model-text execution, original key/prompt restoration, preservation of later
key changes, exported prompt arrays and idempotent disable. Installation tests
also cover overwrite refusal, changed/extra-file preservation and symlink refusal.

Independent review caught and resolved request-identity mismatch, character-vs-
byte cursor semantics, cross-prompt stale acknowledgment, terminal-exit orphan
cleanup, and disable restoration edge cases. One initial expanded aggregate hit
a test timing race while a synthetic backend was still pending; the stale-result
test now uses a deterministic release barrier and bounded readiness instead of a
fixed inference sleep. The final complete aggregate passed without retrying a
failed assertion. This is regression evidence, not a security certification.

Minimum Bash is 5.1 because native PROMPT_COMMAND arrays preserve prompt code.
Full Vi editing, macOS/Git Bash/Windows terminals, real HTTP-provider latency and
SSH remain unverified. Other adapters have separate experimental validation
records and are not implied validated by this Bash milestone.
