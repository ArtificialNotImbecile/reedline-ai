//! Explicit, billable latency evaluation. Dry-run is the default.
//!
//! Zero-history requests use the real HttpBackend. Larger synthetic-history
//! requests use an isolated benchmark client because the production privacy
//! budget intentionally cannot carry 16k actual tokens of natural-language
//! history. Both share the current request_body and transport policy.
//! No user history/files are read. Responses are never printed or executed.
//! These are full nonstreaming completion times, NOT time to first token.
use clap::Parser;
use reedline_ai::{
    backend::{
        BackendError, HttpBackend, HttpCompletion, HttpNetworkOptions, HttpProvider, TokenUsage,
        request_body,
    },
    context::{Context, validate_insertion},
};
use reqwest::{
    blocking::Client,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{self, Read, Write},
    time::{Duration, Instant},
};

const ENDPOINT: &str = "https://api.deepseek.com/chat/completions";
const MODEL: &str = "deepseek-flash";
const KEY_ENV: &str = "REEDLINE_AI_API_KEY";
const MODEL_WINDOW: u64 = 1_048_576;
const OUTPUT_LIMIT: u64 = 128;
const INPUT_NANOCNY_PER_TOKEN: u64 = 2_000;
const OUTPUT_NANOCNY_PER_TOKEN: u64 = 8_000;
const CACHED_INPUT_NANOCNY_PER_TOKEN: u64 = 40;
const ATTEMPT_RESERVE: u64 =
    MODEL_WINDOW * INPUT_NANOCNY_PER_TOKEN + OUTPUT_LIMIT * OUTPUT_NANOCNY_PER_TOKEN;
const MAX_CAP: u64 = 20_000_000_000;
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;
const PREFIX: &str = "git ";
const SUFFIX: &str = " --short";
const MANIFEST: &str = include_str!("latency_fixture_manifest.json");

#[derive(Parser)]
#[command(about = "Synthetic DeepSeek benchmark; dry-run unless explicitly allowed. No retries.")]
struct Args {
    #[arg(long)]
    allow_billable_api: bool,
    /// Approved total cap, including prior accounted costs. Required for live calls.
    #[arg(long)]
    max_cost_cny: Option<String>,
    /// Prior successful upper-cost estimates plus unresolved attempt reserves.
    #[arg(long, default_value = "0")]
    prior_accounted_cny: String,
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=3))]
    samples: u8,
    /// Optional single synthetic-history local-token target: 0, 2000, 4000, 8000, 16000.
    #[arg(long)]
    only_target: Option<u64>,
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=60))]
    timeout_seconds: u64,
    #[arg(long)]
    allow_env_proxy: bool,
    #[arg(long)]
    trust_native_certs: bool,
}

#[derive(Deserialize)]
struct Manifest {
    schema: u8,
    tokenizer_source: String,
    tokenizer_sha256: String,
    token_count_note: String,
    targets: Vec<Target>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Target {
    nominal_history_tokens: u64,
    full_lines: usize,
    tail_terms: usize,
    local_history_json_tokens: u64,
}

fn synthetic_history(target: &Target) -> Vec<String> {
    let mut history = (0..target.full_lines)
        .map(|index| format!("git log --oneline --max-count=8 -- synthetic/fixture_{index:04}.txt"))
        .collect::<Vec<_>>();
    if target.tail_terms > 0 {
        history.push(format!("echo{}", " synthetic".repeat(target.tail_terms)));
    }
    history
}

fn fixed_context() -> Context {
    Context::new(&format!("{PREFIX}{SUFFIX}"), PREFIX.len()).expect("fixed synthetic context")
}

fn synthetic_body(target: &Target) -> Value {
    let mut body = request_body(MODEL, &fixed_context());
    if target.nominal_history_tokens > 0 {
        let mut context: Value =
            serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        context["history"] = json!(synthetic_history(target));
        body["messages"][1]["content"] = json!(serde_json::to_string(&context).unwrap());
    }
    body["thinking"] = json!({"type": "disabled"});
    body
}

fn parse_cny(text: &str) -> Result<u64, &'static str> {
    let (whole, fractional) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || fractional.len() > 9
        || !fractional.bytes().all(|c| c.is_ascii_digit())
    {
        return Err("CNY amounts must be nonnegative decimals with at most nine decimal places");
    }
    let whole: u64 = whole.parse().map_err(|_| "CNY amount is too large")?;
    let fraction: u64 = format!("{fractional:0<9}")
        .parse()
        .map_err(|_| "invalid CNY decimal")?;
    whole
        .checked_mul(1_000_000_000)
        .and_then(|n| n.checked_add(fraction))
        .ok_or("CNY amount is too large")
}

fn cny(nanocny: u64) -> String {
    format!("{}.{:09}", nanocny / 1_000_000_000, nanocny % 1_000_000_000)
}

fn emit(value: Value) {
    // Only constructed metadata, numeric usage and booleans are passed here.
    println!("{value}");
    io::stdout()
        .flush()
        .expect("benchmark output must be writable");
}

fn validate_usage(usage: &TokenUsage) -> Result<u64, &'static str> {
    if usage.prompt_tokens > MODEL_WINDOW
        || usage.completion_tokens > OUTPUT_LIMIT
        || usage.total_tokens != usage.prompt_tokens + usage.completion_tokens
    {
        return Err("missing, inconsistent or out-of-bounds usage; reserve retained");
    }
    if usage
        .prompt_cache_hit_tokens
        .is_some_and(|n| n > usage.prompt_tokens)
        || usage
            .prompt_cache_miss_tokens
            .is_some_and(|n| n > usage.prompt_tokens)
        || matches!(
            (usage.prompt_cache_hit_tokens, usage.prompt_cache_miss_tokens),
            (Some(hit), Some(miss)) if hit + miss != usage.prompt_tokens
        )
    {
        return Err("inconsistent cache-token usage; reserve retained");
    }
    Ok(usage.prompt_tokens * INPUT_NANOCNY_PER_TOKEN
        + usage.completion_tokens * OUTPUT_NANOCNY_PER_TOKEN)
}

fn cache_adjusted_cost(usage: &TokenUsage) -> Option<u64> {
    usage.prompt_cache_hit_tokens.map(|hit| {
        hit * CACHED_INPUT_NANOCNY_PER_TOKEN
            + (usage.prompt_tokens - hit) * INPUT_NANOCNY_PER_TOKEN
            + usage.completion_tokens * OUTPUT_NANOCNY_PER_TOKEN
    })
}

struct Ledger {
    cap: u64,
    accounted: u64,
}

impl Ledger {
    fn reserve(&mut self) -> Result<(), &'static str> {
        if self.accounted > self.cap || ATTEMPT_RESERVE > self.cap - self.accounted {
            return Err("next conservative attempt reserve would exceed approved cap; stopped");
        }
        self.accounted += ATTEMPT_RESERVE;
        Ok(())
    }

    fn settle(&mut self, valid_upper_cost: u64) {
        assert!(valid_upper_cost <= ATTEMPT_RESERVE);
        self.accounted -= ATTEMPT_RESERVE - valid_upper_cost;
    }
}

enum TrialClient {
    Application(Box<HttpBackend>),
    Experimental(Client),
}

impl TrialClient {
    fn new(target: &Target, key: &str, args: &Args) -> Result<Self, BackendError> {
        let timeout = Duration::from_secs(args.timeout_seconds);
        if target.nominal_history_tokens == 0 {
            return HttpBackend::new_with_network(
                ENDPOINT,
                MODEL,
                Some(key),
                timeout,
                HttpNetworkOptions {
                    allow_env_proxy: args.allow_env_proxy,
                    trust_native_certs: args.trust_native_certs,
                },
            )
            .map(|backend| {
                Self::Application(Box::new(backend.with_provider(HttpProvider::DeepSeek)))
            });
        }
        let mut auth = HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| BackendError::Config("invalid authentication header"))?;
        auth.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, auth);
        let mut builder = Client::builder()
            .default_headers(headers)
            .tls_built_in_native_certs(args.trust_native_certs)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .timeout(timeout)
            .user_agent(concat!("reedline-ai/", env!("CARGO_PKG_VERSION")));
        if !args.allow_env_proxy {
            builder = builder.no_proxy();
        }
        builder
            .build()
            .map(Self::Experimental)
            .map_err(|_| BackendError::Config("HTTP client initialization failed"))
    }

    fn complete(&self, body: &Value) -> Result<HttpCompletion, BackendError> {
        if let Self::Application(backend) = self {
            return backend.complete_with_usage(&fixed_context());
        }
        let Self::Experimental(client) = self else {
            unreachable!()
        };
        let response = client
            .post(ENDPOINT)
            .json(body)
            .send()
            .map_err(|_| BackendError::Transport)?;
        if !response.status().is_success() {
            return Err(BackendError::Status(response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| BackendError::Transport)?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(BackendError::BodyTooLarge);
        }
        parse_completion(&bytes)
    }
}

fn parse_completion(bytes: &[u8]) -> Result<HttpCompletion, BackendError> {
    let body: Value = serde_json::from_slice(bytes).map_err(|_| BackendError::InvalidResponse)?;
    let choice = body
        .pointer("/choices/0")
        .ok_or(BackendError::InvalidResponse)?;
    if choice
        .get("finish_reason")
        .is_some_and(|reason| !reason.is_null() && reason != "stop")
    {
        return Err(BackendError::InvalidResponse);
    }
    let insertion = choice
        .pointer("/message/content")
        .and_then(Value::as_str)
        .ok_or(BackendError::InvalidResponse)?;
    validate_insertion(insertion).map_err(BackendError::UnsafeOutput)?;
    let usage = body.get("usage").and_then(|usage| {
        Some(TokenUsage {
            prompt_tokens: usage.get("prompt_tokens")?.as_u64()?,
            completion_tokens: usage.get("completion_tokens")?.as_u64()?,
            total_tokens: usage.get("total_tokens")?.as_u64()?,
            prompt_cache_hit_tokens: usage.get("prompt_cache_hit_tokens").and_then(Value::as_u64),
            prompt_cache_miss_tokens: usage
                .get("prompt_cache_miss_tokens")
                .and_then(Value::as_u64),
        })
    });
    Ok(HttpCompletion {
        insertion: insertion.into(),
        usage,
    })
}

fn error_kind(error: &BackendError) -> &'static str {
    match error {
        BackendError::Config(_) => "configuration",
        BackendError::Transport => "transport_or_timeout",
        BackendError::Cancelled => "cancelled",
        BackendError::Status(_) => "provider_status",
        BackendError::BodyTooLarge => "response_size_limit",
        BackendError::InvalidResponse => "invalid_response",
        BackendError::UnsafeOutput(_) => "unsafe_output",
        BackendError::Budget(_) => "request_budget",
    }
}

fn summary(latencies: &[f64]) -> Value {
    if latencies.is_empty() {
        return json!({"n": 0});
    }
    let mut ordered = latencies.to_vec();
    ordered.sort_by(f64::total_cmp);
    let n = ordered.len();
    let median = if n.is_multiple_of(2) {
        (ordered[n / 2 - 1] + ordered[n / 2]) / 2.0
    } else {
        ordered[n / 2]
    };
    json!({"n": n, "median_ms": median, "min_ms": ordered[0], "max_ms": ordered[n-1]})
}

fn main() {
    if let Err(error) = run(Args::parse()) {
        emit(json!({"type": "stopped", "reason": error, "automatic_retries": 0}));
        std::process::exit(1);
    }
}

fn run(args: Args) -> Result<(), &'static str> {
    let manifest: Manifest =
        serde_json::from_str(MANIFEST).map_err(|_| "invalid bundled manifest")?;
    if manifest.schema != 1 {
        return Err("unsupported bundled manifest");
    }
    let targets: Vec<_> = manifest
        .targets
        .iter()
        .filter(|t| {
            args.only_target
                .is_none_or(|n| n == t.nominal_history_tokens)
        })
        .collect();
    if targets.is_empty() {
        return Err("--only-target must be 0, 2000, 4000, 8000 or 16000");
    }
    let cap = parse_cny(args.max_cost_cny.as_deref().unwrap_or("20"))?;
    let prior = parse_cny(&args.prior_accounted_cny)?;
    if cap > MAX_CAP || prior > cap {
        return Err("cap must be at most CNY 20 and cover prior accounted costs");
    }
    let mut ledger = Ledger {
        cap,
        accounted: prior,
    };
    emit(json!({
        "type": "plan", "dry_run": !args.allow_billable_api,
        "model": MODEL, "endpoint": ENDPOINT, "samples_per_target": args.samples,
        "requests_max": targets.len() * usize::from(args.samples),
        "measurement": "full_nonstreaming_completion_latency_not_TTFT",
        "streaming": false, "automatic_retries": 0,
        "connection_comparison": "fresh client first request vs later calls on that same client; server cache is not flushed; socket reuse is not guaranteed",
        "history_source": "fixed synthetic generator only; no user history or filesystem content",
        "tokenizer_source": manifest.tokenizer_source,
        "tokenizer_sha256": manifest.tokenizer_sha256,
        "token_count_note": manifest.token_count_note,
        "output_token_limit": OUTPUT_LIMIT,
        "budget_cap_cny": cny(cap), "prior_accounted_cny": cny(prior),
        "per_attempt_reserve_cny": cny(ATTEMPT_RESERVE),
        "price_checked": "2026-10-23", "price_source": "https://api-docs.deepseek.com/zh-cn/quick_start/pricing/",
        "input_peak_cny_per_million": 2, "output_peak_cny_per_million": 8,
        "cached_input_peak_cny_per_million": "0.04",
        "budget_accounting": "Reserve full model window plus output before each attempt; release difference only after valid usage; all-uncached peak upper estimate. Unknown outcomes retain reserve and stop.",
        "allow_env_proxy": args.allow_env_proxy, "trust_native_certs": args.trust_native_certs,
        "tls_certificate_verification": true,
    }));
    if !args.allow_billable_api {
        for target in targets {
            let body = synthetic_body(target);
            emit(json!({"type": "fixture", "target": target,
                "request_body_bytes": serde_json::to_vec(&body).unwrap().len(),
                "path": if target.nominal_history_tokens == 0 { "application_HttpBackend" } else { "experimental_history_scaling_only" }}));
        }
        return Ok(());
    }
    if args.max_cost_cny.is_none() {
        return Err("live calls require explicit --max-cost-cny; no request made");
    }
    if cap - prior < ATTEMPT_RESERVE {
        return Err("budget cannot cover first conservative reserve; no request made");
    }
    let key =
        std::env::var(KEY_ENV).map_err(|_| "REEDLINE_AI_API_KEY is required; no request made")?;
    if key.is_empty() {
        return Err("REEDLINE_AI_API_KEY must be nonempty; no request made");
    }
    let mut completed = 0;
    for target in targets {
        let body = synthetic_body(target);
        let build_start = Instant::now();
        let client = TrialClient::new(target, &key, &args)
            .map_err(|_| "client initialization failed; no request made for this group")?;
        let client_build_ms = build_start.elapsed().as_secs_f64() * 1000.0;
        let mut latencies = Vec::new();
        for sample in 0..args.samples {
            ledger.reserve()?;
            emit(
                json!({"type": "attempt_reserved", "target_history_tokens": target.nominal_history_tokens,
                "sample": sample + 1, "accounted_upper_cny": cny(ledger.accounted)}),
            );
            let start = Instant::now();
            let result = client.complete(&body);
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            let completion = match result {
                Ok(value) => value,
                Err(error) => {
                    emit(
                        json!({"type": "failed_trial", "target_history_tokens": target.nominal_history_tokens,
                        "sample": sample + 1, "complete_total_ms": elapsed_ms, "error_kind": error_kind(&error),
                        "http_status": if let BackendError::Status(status) = error { Some(status) } else { None },
                        "accounted_upper_cny": cny(ledger.accounted), "unknown_attempt_reserve_retained": true,
                        "completed_requests": completed}),
                    );
                    return Err(
                        "request failed; stopped without retry; response and error details redacted",
                    );
                }
            };
            let Some(usage) = completion.usage else {
                emit(
                    json!({"type": "unknown_usage", "accounted_upper_cny": cny(ledger.accounted)}),
                );
                return Err("missing usage; reserve retained; stopped without retry");
            };
            let upper_cost = match validate_usage(&usage) {
                Ok(cost) => cost,
                Err(error) => {
                    emit(
                        json!({"type": "unknown_usage", "accounted_upper_cny": cny(ledger.accounted)}),
                    );
                    return Err(error);
                }
            };
            ledger.settle(upper_cost);
            latencies.push(elapsed_ms);
            completed += 1;
            emit(
                json!({"type": "trial", "target_history_tokens": target.nominal_history_tokens,
                "local_history_json_tokens": target.local_history_json_tokens,
                "path": if target.nominal_history_tokens == 0 { "application_HttpBackend" } else { "experimental_history_scaling_only" },
                "sample": sample + 1, "client_state": if sample == 0 { "fresh_client_first_request" } else { "reused_client_request" },
                "complete_total_ms": elapsed_ms, "client_build_ms": if sample == 0 { client_build_ms } else { 0.0 },
                "full_nonstreaming_not_TTFT": true, "usage": usage,
                "insertion_nonempty": !completion.insertion.is_empty(), "expected_insertion_match": completion.insertion == "status",
                "insertion_bytes": completion.insertion.len(),
                "estimated_all_uncached_peak_upper_cny": cny(upper_cost),
                "estimated_cache_adjusted_peak_cny": cache_adjusted_cost(&usage).map(cny),
                "accounted_upper_cny": cny(ledger.accounted)}),
            );
        }
        emit(
            json!({"type": "target_summary", "target_history_tokens": target.nominal_history_tokens,
            "all_requests": summary(&latencies), "fresh_client": summary(&latencies[..1]),
            "reused_client": summary(&latencies[1..]),
            "note": "Small sequential exploratory sample, no p95 or causal connection/cache claims. All values are full nonstreaming response latency."}),
        );
    }
    emit(json!({"type": "complete", "completed_requests": completed,
        "new_requests_estimated_peak_upper_cny": cny(ledger.accounted - prior),
        "accounted_upper_cny": cny(ledger.accounted),
        "billing_note": "Usage-based dated-price estimates, not provider billing reconciliation."}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_parsing_is_exact_and_rejects_unsafe_values() {
        assert_eq!(parse_cny("20"), Ok(MAX_CAP));
        assert_eq!(parse_cny("2.100704"), Ok(2_100_704_000));
        assert_eq!(parse_cny("0.000000001"), Ok(1));
        for invalid in ["NaN", "inf", "-1", "+20", "1e3", "1.0000000001", "", " 1"] {
            assert!(parse_cny(invalid).is_err());
        }
    }

    #[test]
    fn budget_reserves_before_request_and_only_releases_valid_cost() {
        let mut ledger = Ledger {
            cap: ATTEMPT_RESERVE + 100,
            accounted: 0,
        };
        ledger.reserve().unwrap();
        assert_eq!(ledger.accounted, ATTEMPT_RESERVE);
        assert!(ledger.reserve().is_err());
        ledger.settle(90);
        ledger.reserve().unwrap();
        // An unknown outcome is left reserved; the caller stops, with no retry.
        assert_eq!(ledger.accounted, ATTEMPT_RESERVE + 90);
        assert!(ledger.reserve().is_err());
    }

    #[test]
    fn payload_changes_only_synthetic_history_not_current_command() {
        let manifest: Manifest = serde_json::from_str(MANIFEST).unwrap();
        assert_eq!(manifest.targets.len(), 5);
        for target in manifest.targets {
            let body = synthetic_body(&target);
            let context: Value =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            assert_eq!(context["prefix"], PREFIX);
            assert_eq!(context["suffix"], SUFFIX);
            assert_eq!(body["max_tokens"], OUTPUT_LIMIT);
            assert_eq!(body["stream"], false);
            assert_eq!(body["thinking"]["type"], "disabled");
            assert_eq!(
                target.nominal_history_tokens,
                target.local_history_json_tokens
            );
            assert!(target.full_lines < 1200 && target.tail_terms <= 128);
            if target.nominal_history_tokens == 0 {
                assert!(context.get("history").is_none());
            } else {
                assert_eq!(context["history"], json!(synthetic_history(&target)));
            }
        }
    }

    #[test]
    fn usage_validation_rejects_overflow_and_inconsistent_cache() {
        let mut usage = TokenUsage {
            prompt_tokens: 2000,
            completion_tokens: 2,
            total_tokens: 2002,
            prompt_cache_hit_tokens: Some(1500),
            prompt_cache_miss_tokens: Some(500),
        };
        assert_eq!(validate_usage(&usage), Ok(4_016_000));
        assert_eq!(cache_adjusted_cost(&usage), Some(1_076_000));
        usage.prompt_cache_miss_tokens = Some(501);
        assert!(validate_usage(&usage).is_err());
        usage.prompt_tokens = u64::MAX;
        assert!(validate_usage(&usage).is_err());
    }

    #[test]
    fn parser_never_exposes_or_accepts_unsafe_response_content() {
        let response = json!({"choices": [{"finish_reason": "stop", "message": {"content": "status"}}],
            "usage": {"prompt_tokens": 4, "completion_tokens": 1, "total_tokens": 5}});
        let completion = parse_completion(&serde_json::to_vec(&response).unwrap()).unwrap();
        assert_eq!(completion.insertion, "status");
        for content in ["status\nsecret", "\u{1b}[31m", "```bash"] {
            let mut unsafe_response = response.clone();
            unsafe_response["choices"][0]["message"]["content"] = json!(content);
            assert!(parse_completion(&serde_json::to_vec(&unsafe_response).unwrap()).is_err());
        }
    }

    #[test]
    fn summaries_report_small_sample_counts_without_percentiles() {
        assert_eq!(summary(&[9.0, 1.0, 5.0])["median_ms"], 5.0);
        assert_eq!(summary(&[1.0, 9.0])["median_ms"], 5.0);
        assert_eq!(summary(&[]), json!({"n": 0}));
        assert!(summary(&[1.0]).get("p95").is_none());
    }
}
