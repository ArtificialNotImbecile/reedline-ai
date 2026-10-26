use reedline_ai::{backend::request_body, context::Context};
use std::process::Command;

#[test]
fn diagnostic_product_body_matches_public_rust_builder() {
    let output = Command::new(if cfg!(windows) { "python" } else { "python3" })
        .args(["-c", "import sys,json; sys.path.insert(0, 'scripts'); import api_latency_segments as b; print(json.dumps(b.product_body()))"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("python3 available for offline diagnostic contract test");
    assert!(output.status.success());
    let diagnostic: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let context = Context::new("git  --short", 4).unwrap();
    assert_eq!(diagnostic, request_body("deepseek-flash", &context));
}

/// Explicit local-only microbenchmark; use an optimized build and --nocapture.
/// Does not read shell history, initialize an HTTP client, or use a key.
#[test]
#[ignore = "local performance observation, not a timing assertion"]
fn measure_local_preparation_segments() {
    use reedline_ai::backend::estimate_request;
    use reedline_ai::history_context::{HistoryContextConfig, select_history};
    use std::{hint::black_box, time::Instant};

    let history: Vec<String> = (0..500)
        .map(|index| format!("git log --oneline --max-count=8 -- synthetic/fixture_{index:04}.txt"))
        .collect();
    for enabled in [false, true] {
        let config = HistoryContextConfig {
            enabled,
            ..HistoryContextConfig::default()
        };
        let mut context_times = Vec::new();
        let mut selection_times = Vec::new();
        let mut preparation_times = Vec::new();
        let mut parse_times = Vec::new();
        let mut selected_entries = 0;
        let mut body_bytes = 0;
        let fake_response = br#"{"choices":[{"message":{"content":"status"}}],"usage":{"prompt_tokens":140,"completion_tokens":1,"total_tokens":141}}"#;
        for _ in 0..1000 {
            let start = Instant::now();
            let context = Context::new(black_box("git  --short"), black_box(4)).unwrap();
            context_times.push(start.elapsed().as_secs_f64() * 1e6);
            let start = Instant::now();
            let preview = select_history(&context, black_box(&history), &config).unwrap();
            selected_entries = preview.entries.len();
            let context = preview.into_context(context).unwrap();
            selection_times.push(start.elapsed().as_secs_f64() * 1e6);
            let start = Instant::now();
            black_box(estimate_request(&context, config.budget).unwrap());
            let mut body = request_body("deepseek-flash", &context);
            body["thinking"] = serde_json::json!({"type": "disabled"});
            let bytes = serde_json::to_vec(&body).unwrap();
            body_bytes = black_box(bytes).len();
            preparation_times.push(start.elapsed().as_secs_f64() * 1e6);
            let start = Instant::now();
            let parsed: serde_json::Value =
                serde_json::from_slice(black_box(fake_response)).unwrap();
            let insertion = parsed["choices"][0]["message"]["content"].as_str().unwrap();
            reedline_ai::context::validate_insertion(black_box(insertion)).unwrap();
            black_box(parsed);
            parse_times.push(start.elapsed().as_secs_f64() * 1e6);
        }
        fn summary(mut samples: Vec<f64>) -> serde_json::Value {
            samples.sort_by(f64::total_cmp);
            serde_json::json!({"p50_us": samples[500], "p95_us": samples[950], "max_us": samples[999]})
        }
        println!(
            "{}",
            serde_json::json!({
                "local_only": true, "optimized_build": !cfg!(debug_assertions), "samples": 1000,
                "synthetic_history_candidates": if enabled { 500 } else { 0 },
                "selected_history_entries": selected_entries, "body_bytes": body_bytes,
                "context_snapshot": summary(context_times), "history_selection_and_attachment": summary(selection_times),
                "budget_body_and_serialization": summary(preparation_times),
                "synthetic_response_parse_and_validation": summary(parse_times),
                "ui_rendering_measured": false, "network_measured": false
            })
        );
    }
}
