use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_reedline-ai"))
        .args(args)
        .env_remove("REEDLINE_AI_API_KEY")
        .output()
        .unwrap()
}

#[test]
fn headless_fim_returns_only_json_and_preserves_suffix() {
    let output = run(&["--complete", "git  --short", "--cursor", "4"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["insertion"], "status");
    assert_eq!(result["result"], "git status --short");
}

#[test]
fn invalid_cursor_error_does_not_echo_private_input() {
    let output = run(&["--complete", "PRIVATE世", "--cursor", "8"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("UTF-8"));
    assert!(!error.contains("PRIVATE"));
}

#[test]
fn empty_http_input_is_not_sent_even_to_an_unavailable_provider() {
    let output = run(&[
        "--backend",
        "http",
        "--endpoint",
        "http://127.0.0.1:1/v1/chat/completions",
        "--model",
        "test",
        "--complete",
        "",
    ]);
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["insertion"], "");
}

#[test]
fn missing_terminal_fails_with_headless_hint() {
    let output = run(&[]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("--complete")
    );
}

#[test]
fn provider_extensions_and_usage_require_explicit_http_mode() {
    for args in [
        vec!["--provider", "deepseek", "--complete", "git st"],
        vec!["--report-usage", "--complete", "git st"],
    ] {
        let output = run(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let output = run(&[
        "--backend",
        "http",
        "--provider",
        "deepseek",
        "--report-usage",
        "--endpoint",
        "http://127.0.0.1:1/chat/completions",
        "--model",
        "deepseek-flash",
        "--complete",
        "",
    ]);
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["insertion"], "");
    assert!(result["usage"].is_null());
}
