# Segmented API latency diagnostic

This optional, manually authorized diagnostic is separate from the app and never
runs in CI with network access. It uses only a fixed synthetic command:
`prefix="git "`, `suffix=" --short"`. No real history or user files are sent.

## Observed results: 2026-10-23, cloud executor

Six requests succeeded. All reused-connection observations had `NUM_CONNECTS=0`
and the same handle-local connection ID as their fresh request. Both fresh
requests had `NUM_CONNECTS=1` and successful proxy CONNECT responses. IDs are
handle-local, so ID 0 in both groups does not mean the groups shared one socket.

All times below are milliseconds **from each request start**, not additive
segments. DNS/TCP/AppConnect/pretransfer/TTFB use libcurl's timer; content/DONE/end
use the monotonic client timer and include small callback/parsing overhead.

| Body / connection | DNS complete | TCP complete | AppConnect complete | Pretransfer | TTFB | First content | SSE DONE | End |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| minimal / fresh | 0.498 | 0.782 | 4591.574 | 4592.144 | 5089.818 | 5437.668 | 5528.039 | 5528.180 |
| minimal / reused 1 | 0.000 | 0.000 | 0.000 | 0.246 | 355.971 | 880.125 | 963.974 | 966.238 |
| minimal / reused 2 | 0.000 | 0.000 | 0.000 | 0.189 | 306.490 | 989.489 | 1089.995 | 1096.259 |
| product / fresh | 0.041 | 0.150 | 2998.502 | 2998.984 | 3487.887 | 4006.392 | 4076.156 | 4079.056 |
| product / reused 1 | 0.000 | 0.000 | 0.000 | 0.242 | 316.825 | 884.197 | 982.210 | 987.318 |
| product / reused 2 | 0.000 | 0.000 | 0.000 | 0.159 | 590.176 | 1136.915 | 1208.461 | 1211.204 |

### Interpretation

- The user's TCP intuition is supported for the path we can observe: fresh DNS
  plus TCP completed within 0.782 ms and 0.150 ms. TCP after DNS was about
  0.284 ms and 0.109 ms. **These are connections to the configured proxy**, not
  a direct measurement of the proxy's upstream TCP connection to DeepSeek.
- The fresh slow portion was after that local/proxy TCP connection but before
  AppConnect completed: about 4,590.792 ms and 2,998.352 ms. This includes tunnel
  and TLS/upstream setup effects that this experiment cannot separate. It is
  incorrect to call it “several seconds of TCP connect,” “pure TLS,” or entirely
  “DeepSeek inference.”
- Once reused, first answer content arrived in **880–989 ms** for the minimal
  prompt and **884–1,137 ms** for the product prompt. The latter was only 130
  input tokens versus the minimal prompt's 20. There is no evidence here that
  the product prompt itself adds seconds, but two reused samples per body are
  not enough to quantify a prompt-latency effect or a stable percentile.
- Subtracting the pretransfer checkpoint from the first-content observation on
  the two fresh requests leaves approximately 846 ms and 1,007 ms, consistent
  with the broad reused-request range. This is only a client-observed remainder,
  not an isolated server-compute measurement.
- All six responses generated one output token, delivered in one content chunk,
  with six content bytes. Cached prompt tokens were zero throughout. The remaining
  first-content-to-response-end interval was about 73–107 ms. This tiny-output
  test therefore shows a limited first-content advantage versus waiting for the
  completed streaming response; it does not estimate longer-generation savings.
- API latency can be near one second on a reused connection here, but is not
  guaranteed below one second. Different origin/proxy paths, provider queueing,
  and the previous Rust nonstreaming protocol can change observed latency.

### Actual local Rust work, optimized build

These are 1,000 local-only observations per scenario. Values below are median /
p95 in microseconds; their sums are not percentiles of total latency.

| Component | History upload disabled | 500 synthetic candidates, 50 selected |
|---|---:|---:|
| Context snapshot | 0.066 / 0.085 | 0.070 / 0.150 |
| History selection and attachment | 0.216 / 0.310 | 156.974 / 295.773 |
| Budget checks, body construction and serialization | 1.616 / 1.790 | 12.688 / 14.378 |
| Fixed tiny response parse and insertion validation | 0.806 / 0.993 | 1.246 / 1.829 |

The measured local work is normally sub-millisecond, consistent with the user's
expectation. The largest single observed selection duration was 1.823 ms; timing
outliers on a shared executor are not necessarily CPU computation. The test does
not measure history disk I/O, real UI rendering, queueing or scheduler latency.
The app's separate **180 ms debounce** is intentional waiting, and its **33 ms
poll interval** is a setting, not measured processing cost. None of these local
CPU measurements explain a several-second delay.

Practical conclusion: maintain persistent client/connection reuse, separately
review the intentional debounce, and keep measuring the actual request path.
Do not attribute multi-second cold-start observations to slow local command
processing or assume a streaming refactor alone would eliminate tunnel setup.

## Questions it can answer

- Request start to the first nonempty `choices[].delta.content` received locally.
  Role-only chunks, empty strings, and reasoning deltas are **not** first content.
- Request start to SSE `[DONE]`, and to libcurl returning after response end.
- libcurl's cumulative DNS, TCP-connect, TLS/app-connect, pre-transfer and
  first-byte checkpoints, plus complete transfer time.
- Whether a same-handle request actually reused a connection: `NUM_CONNECTS == 0`
  on a successful transfer, with `CONN_ID` reported where supported. Merely using
  the same client is not evidence of reuse.
- Local preparation, separately from the network timer.

`curl_starttransfer_time_ms` is TTFB, **not TTFT**. A response header or keepalive
can arrive before any model content. The content timer ends only after a complete
SSE event containing nonempty answer text has arrived and been parsed. Network
chunking or proxy buffering can affect it; it is observed API latency, not pure
server inference time.

The raw curl checkpoints are cumulative from transfer start. On a fresh HTTPS
connection, `connect - namelookup` approximates TCP establishment and
`appconnect - connect` approximates subsequent negotiation. Through a proxy the
DNS/TCP target may be the proxy and the latter interval can include HTTP CONNECT
and origin TLS. These are **not** separately measured direct-to-DeepSeek TCP/TLS
segments. On reused connections these setup checkpoints can be zero. Do not
subtract them and label the remainder “model compute.”

## Comparison scope

Two small streaming requests are compared, three requests each:

1. Minimal single-user-message prompt for the fixed missing command text.
2. The product's actual public `request_body` shape and system/context prompt,
   with only `stream=true`, `stream_options.include_usage=true`, and the same
   explicit DeepSeek `thinking.type=disabled` settings applied.

Each group gets a new libcurl handle for its first request and reuses that handle
for its next two. No parallel requests, redirects, or application-level retries.
Stop at the first transport/status/parser/usage failure. `max_tokens=128`,
`temperature=0`, model `deepseek-flash`, official endpoint only. Token usage and
cache hits can differ across calls, so these six observations are a diagnostic,
not a controlled population benchmark or a p95 estimate.

The offline Rust contract test verifies that the Python product body before
streaming/provider modifications equals `backend::request_body` for this context.
The production Rust app remains **nonstreaming** and waits for a full response.
This diagnostic is Python/libcurl with streaming. Do not call its first-content
latency the app's current display latency or directly attribute differences from
previous Rust nonstreaming totals only to networking.

## Running

Default invocation is entirely offline and does not read the API key:

```sh
python3 scripts/api_latency_segments.py
python3 -m unittest discover -s scripts -p test_api_latency_segments.py
cargo test --locked --test api_latency_body
cargo test --locked --release --test api_latency_body -- --ignored --nocapture
```

The ignored release test measures the **actual Rust** context creation, history
selection/attachment, budget/body serialization, and parsing/validation of one
fixed tiny synthetic response, each separately over 1000 iterations. It checks
zero uploaded history and the maximum 500 synthetic selection candidates. It is
not a measured UI pipeline or a performance guarantee; it never sends requests.

`local_python_preparation` in diagnostic output measures only the diagnostic's
Python JSON construction and, for the product group, loading the public fixed
prompt from source. It is not a measurement of Rust application overhead.

After authorization, install `pycurl` from its official Python package registry
if needed. The process environment supplies `REEDLINE_AI_API_KEY` transiently;
never put the value in shell arguments, files, reports, or version control:

```sh
python3 scripts/api_latency_segments.py --allow-billable-api \
  --allow-env-proxy --trust-native-certs \
  --max-cost-cny YOUR_APPROVED_CAP --prior-accounted-cny PRIOR_ACCOUNTED_COST
```

For this executor, the installed dependency can be selected with
`PYTHONPATH=/tmp/reedline-api-latency-deps`. That path is environment-local, not a
portable dependency lock. TLS verification remains enabled for both origin and
proxy. Environment proxies and native/SSL_CERT_FILE trust require explicit flags.
Neither proxy URLs, headers, raw provider bodies, nor generated text are printed.

A conservative CNY 2.098176 is reserved **before each attempt** based on the full
1,048,576-token model window at CNY 2/M input plus 128 tokens at CNY 8/M output.
A successful, validated usage record replaces that reserve with the uncached
peak-price upper estimate. Missing/inconsistent usage retains the reserve and
stops. Prior accounted costs (including unresolved reserves) must be included.
The maximum total cap is CNY 20. This is cost accounting, not a provider invoice.

## Application latency not measured by this script

The app's default 180 ms debounce is an intentional scheduling delay, not CPU
cost. The configured 33 ms UI poll interval is a scheduling setting, not a measured
render duration or guaranteed upper bound. Worker queueing, scheduler stalls,
terminal rendering and user-perceived display latency are not measured here.
Do not explain multi-second delays as TCP reconnect overhead without the measured
connection checkpoints. Do not promise subsecond API latency from these samples.

## Primary references

- [DeepSeek chat API and streaming usage](https://api-docs.deepseek.com/api/create-chat-completion/)
- [DeepSeek current CNY pricing](https://api-docs.deepseek.com/zh-cn/quick_start/pricing/), checked 2026-10-23
- [libcurl timing and connection observations](https://curl.se/libcurl/c/curl_easy_getinfo.html)
