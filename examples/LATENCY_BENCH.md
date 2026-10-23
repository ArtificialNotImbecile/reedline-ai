# Explicit DeepSeek latency benchmark

This example is **not part of normal application use** and is never called by CI.
It defaults to a dry-run. It never reads shell history, the filesystem for context,
or user commands. The only live credential input is the existing
`REEDLINE_AI_API_KEY` environment variable; never put a key in a command argument,
source file, result, or shell-history entry.

## Build and preview (no model API)

```sh
cargo test --locked --example latency_bench
cargo clippy --locked --example latency_bench -- -D warnings
cargo build --locked --release --example latency_bench
target/release/examples/latency_bench
```

For an explicitly authorized paid run, after reviewing the dated pricing constants
and arranging the key securely in the process environment:

```sh
target/release/examples/latency_bench \
  --allow-billable-api --max-cost-cny 20 --prior-accounted-cny 0
```

Set `--prior-accounted-cny` to the sum of all earlier successful conservative cost
estimates **and every unresolved request reserve** covered by the same approval.
It is not automatically recovered across process runs. Persist the JSONL output
so that an interrupted attempt is not forgotten. A run emits its reserved amount
before sending every request. Do not restart a failed run without accounting for
its retained reserve or reconciling it with the provider.

`--allow-env-proxy` and `--trust-native-certs` independently opt in to environment
proxy routing and platform/`SSL_CERT_FILE` trust when your network requires them.
Certificate verification stays enabled. Redirects and automatic retries are
always disabled. An uncertain failure stops the entire run, retaining its reserve.

Optional controls:

- `--samples 1`, `2`, or `3` (default `3`).
- `--only-target 0`, `2000`, `4000`, `8000`, or `16000`.
- `--timeout-seconds 1..60` (default `30`).

## What the numbers mean

- The fixed command is prefix `git ` and suffix ` --short` at every level.
- Zero history uses the application's actual `HttpBackend`.
- The other four levels use the **current application request constructor** with
  a separate synthetic `history` array and equivalent client/TLS/retry/output
  settings. They are explicitly experimental context-scaling requests. Natural
  16k-token history exceeds the application's conservative privacy budgets; the
  experiment does **not** disable or weaken those production limits.
- Each level creates one fresh HTTP client and then retains that exact client for
  subsequent requests in the same process. The first request is labeled
  `fresh_client_first_request`; subsequent requests are `reused_client_request`.
  This does not guarantee a reused socket, clear DNS/TLS state, or clear provider
  caches. Do not call the first sample a guaranteed cold-cache measurement.
- `complete_total_ms` measures the whole nonstreaming completion call. Client
  construction is reported separately as `client_build_ms`. Neither is TTFT.
  This example makes no first-token or streaming claims.
- The compact manifest was generated with the official DeepSeek tokenizer. Its
  2k/4k/8k/16k values count **synthetic history JSON only**, not the entire prompt.
  The actual full input, output, and cache-hit counts are the returned API `usage`.
- With three samples, summaries report median/min/max and sample count. There are
  only two reused-client samples per level. No p95 or causal performance claim is
  justified. Group order, output length, server load, and cache hits can affect
  these exploratory observations.
- Outputs contain only sanitized measurements, numeric usage, and result-length
  and expected-match indicators. No provider text, secrets, or suggested command
  is printed or executed.

## Cost guard

The fixed model is `deepseek-flash`, with `max_tokens=128`, thinking disabled,
and the official `https://api.deepseek.com/chat/completions` endpoint.
Peak prices checked 2026-10-23 are CNY 2/M uncached input, 0.04/M cached input,
and 8/M output. Recheck [official pricing](https://api-docs.deepseek.com/zh-cn/quick_start/pricing/)
before using these assumptions at a later date.

Before each attempt the ledger reserves CNY 2.098176, covering a full 1,048,576
input tokens plus 128 output tokens at all-uncached peak prices. It sends nothing
unless prior accounted amounts plus that reserve fit the explicitly approved cap
(at most CNY 20). Only valid, consistent usage releases the unused part. Actual
usage is reported both at the conservative all-uncached upper estimate and, when
cache-hit metadata exists, the cache-adjusted estimate. These are dated-price
estimates, not a provider billing statement or an account-wide spending limit.

## Reproduce synthetic token targets offline

The generator is optional and not needed to build or run the benchmark. It needs
the separately installed `tokenizers` Python package and a local tokenizer JSON
from the [official tokenizer archive](https://cdn.deepseek.com/api-docs/deepseek_v4_tokenizer.zip):

```sh
python3 scripts/generate_latency_fixtures.py /path/to/tokenizer.json
```

It loads tokenizer data only, never executes bundled tokenizer Python code, and
records the source and SHA-256 in `latency_fixture_manifest.json`. Do not commit
the large tokenizer or dependencies. [DeepSeek's token documentation](https://api-docs.deepseek.com/quick_start/token_usage/)
identifies the API usage as authoritative when local estimates differ.
