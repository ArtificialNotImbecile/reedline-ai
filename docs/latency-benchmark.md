# DeepSeek latency snapshot, 2026-10-23

**Observed reused-client completion times were about 0.92–1.20 seconds.** First
requests on newly created clients took 4.44–10.79 seconds. These are full
**nonstreaming response times, not time to first token (TTFT)**. This small,
sequential experiment does not isolate connection setup, server load, or caching.

## Setup

- Release build, Linux cloud executor; official `deepseek-flash` API, thinking
  disabled, temperature 0, maximum output 128 tokens.
- Environment proxy and native certificate trust were explicitly enabled. TLS
  certificate verification stayed on; no redirects or automatic retries.
- Fixed current command at every level: prefix `git `, suffix ` --short`.
- Synthetic history only. No real shell history, repository contents, working
  directory, or user commands were transmitted.
- Five history sizes; one fresh client per size, followed by two more requests
  through **that same retained client in the same process**, 15 requests total.
- Zero history uses the actual application `HttpBackend`. Larger-history groups
  use an isolated experimental client with the same system prompt, JSON context
  field names, security settings, and output limit. They are not measurements of
  the production history selector or a license to bypass its privacy budget.
- Tested the **local uncommitted optional-history/benchmark worktree** based on
  commit `c9441392e532cfc6e70a4beccad92f66560f2393`, not the unchanged base commit or
  published `main` alone. This was an exploratory pre-commit snapshot.

## Measured results

Times below exclude separately measured client construction (0.30–0.90 ms).
`Fresh` means a new client, **not** a cleared server cache or guaranteed cold
connection. `Reused` means the same client object, not proof of socket reuse.

| Synthetic history tokens¹ | Actual full prompt tokens² | Fresh (s), n=1 | Reused 1 (s) | Reused 2 (s) | Reused median (s), n=2 | Cache-hit tokens: fresh / reused 1 / reused 2 |
| ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 0 | 130 | 10.790 | 0.964 | 1.012 | 0.988 | 0 / 0 / 0 |
| 2,000 | 2,133 | 5.718 | 1.063 | 0.936 | 1.000 | 0 / 1,920 / 1,920 |
| 4,000 | 4,133 | 5.435 | 1.056 | 0.922 | 0.989 | 2,048 / 3,968 / 3,968 |
| 8,000 | 8,133 | 4.436 | 1.098 | 0.961 | 1.030 | 4,096 / 7,936 / 7,936 |
| 16,000 | 16,133 | 4.510 | 1.205 | 1.039 | 1.122 | 8,064 / 15,999 / 15,999 |

¹ Exact counts of compact **synthetic history JSON only**, produced offline with
DeepSeek's official tokenizer. Zero means the history field is absent. These are
not byte counts, production budget estimates, or complete input-token counts.

² Authoritative full-prompt counts from the API `usage` field, identical across
all three trials at each size. Cache misses equal prompt tokens minus cache hits.
The two reused values explicitly show each group's observed range; their median
is the mean of those two observations. There are too few samples to estimate p95.

Output-token counts, in fresh/reused-1/reused-2 order, were `1/1/1`, `1/1/1`,
`4/1/1`, `4/4/4`, and `1/1/1`. All requests returned successfully, and all insertions
passed the application's single-line/control-character/output-length checks.

## Usage-based cost estimates

These totals cover **only these 15 successful calls**, not other activity or an
account balance. Peak prices checked on the test date were CNY 2/M uncached input,
CNY 0.04/M cached input, and CNY 8/M output. Estimates below use returned usage;
they are not a reconciled provider invoice. [Official pricing](https://api-docs.deepseek.com/zh-cn/quick_start/pricing/).

| Synthetic history tokens | Three-call all-uncached upper estimate (CNY) | Three-call cache-adjusted peak estimate (CNY) |
| ---: | ---: | ---: |
| 0 | 0.000804 | 0.00080400 |
| 2,000 | 0.012822 | 0.00529560 |
| 4,000 | 0.024846 | 0.00527736 |
| 8,000 | 0.048894 | 0.00975672 |
| 16,000 | 0.096822 | 0.01830048 |
| **All 15 calls** | **0.184188** | **0.03943416** |

Total usage was 91,986 prompt tokens and 27 output tokens. The conservative
benchmark guard reserves against all-uncached peak pricing; cache discounts are
reported separately rather than relied upon for authorization.

## Interpretation and limits

- Reused-client latency in this one run stayed near a second even as synthetic
  input grew. That is promising for asynchronous assistance, not instant native
  completion and not a promise for another network, time, model, or real history.
- The newly created clients' first calls were slower, but this is **not** an
  isolated experiment establishing how much client reuse alone saves. DNS/TLS,
  proxy behavior, service scheduling, and cache state were not independently
  controlled. The first calls at 4k, 8k, and 16k already had cache hits because
  ascending-size fixtures share prefixes.
- The history deliberately repeats synthetic `git log` commands to control token
  length. It is highly biased and is **not a relevance or completion-quality
  benchmark**. Exact `status` matches occurred on all zero-history trials and the
  first 16k trial, but not on the other 11 trials. A mismatch is not an output
  safety failure. Response text was not retained, so mismatches cannot be
  diagnosed from their byte lengths or assumed to be any particular command.
- A safe-looking insertion can still be semantically wrong. Nothing in this
  experiment executes suggestions or establishes their correctness.
- The application budgets are conservative byte-based token estimates with
  protocol/output reserves. They are intentionally different from actual model
  tokens. The 16k-token synthetic-history experiment does not weaken production
  context limits or imply those fixtures fit the default application budget.
- Preserve native Reedline history, completion, menus, and editing as the fast
  local path. Treat remote AI as an asynchronous fallback with stale-result suppression; this
  result does not justify replacing native behaviors with network-bound ones.

## Reproduction

See [benchmark instructions](../examples/LATENCY_BENCH.md), the
[runner](../examples/latency_bench.rs), and the small
[synthetic manifest](../examples/latency_fixture_manifest.json). Dry-run and tests
make no paid requests. A live run requires explicit billable opt-in and an
approved cost cap; do not run it as CI.

The manifest records tokenizer SHA-256
`89085f12ef79460ac5f66d1119325ddfc694b4ab209d80bbd81d35f081dc9614` and the
[official tokenizer archive](https://cdn.deepseek.com/api-docs/deepseek_v4_tokenizer.zip).
The API remains authoritative when local token counts differ, as described in
[DeepSeek's token documentation](https://api-docs.deepseek.com/quick_start/token_usage/).
