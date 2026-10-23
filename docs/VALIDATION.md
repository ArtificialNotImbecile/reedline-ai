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

## Scope and review

The initial published commit `459b8747ae6b7b605826b3f24f635ba6c0d65b44` passed [hosted checks on Linux, macOS and Windows](https://github.com/ArtificialNotImbecile/reedline-ai/actions/runs/39926660746), including Linux PTY smoke, on 2026-10-23. Hosted CI is now manual-only (`workflow_dispatch`), with no push or pull-request trigger. Routine validation uses `bash scripts/ci-local.sh` on Linux/macOS, without starting hosted workflows or calling remote model APIs. A local pass verifies that host only; it does not establish a new cross-platform result.

Known limitations remain intentional: no command execution, no native shell integration, no native FIM-token adapter, no streaming or active HTTP cancellation, and no guarantee a model's suggestion is semantically correct or safe to copy into a shell. See README for privacy and resource limits.

Independent review found two bugs in the first draft: raw pasted controls could reach default ANSI rendering before validation, and Reedline input batching could accept stale or repeated hints. The iteration introduced pre-render input filtering and input-ordered, one-shot rendered-hint acceptance, with dedicated Rust and PTY regression coverage. This record describes checks, not a formal security certification.

Dependency advisory check: `cargo-audit` 0.22.2 checked the lockfile on 2026-10-23 against 1,349 loaded RustSec advisories. It reported zero known vulnerabilities and no warnings. This is a dated advisory lookup, not a guarantee that dependencies contain no vulnerabilities; rerun against the current database before a release.
