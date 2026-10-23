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

#[test]
fn preview_is_offline_excludable_read_only_and_does_not_need_api_configuration() {
    let dir = std::env::temp_dir().join(format!("reedline-preview-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("synthetic-history");
    let original = (0..10_002)
        .map(|i| format!("echo synthetic-{i}\n"))
        .collect::<String>()
        + "echo PRIVATE-MARKER\n";
    std::fs::write(&path, &original).unwrap();
    let output = run(&[
        "--history-file",
        path.to_str().unwrap(),
        "--history-preview",
        "echo",
        "--history-exclude",
        "PRIVATE-MARKER",
    ]);
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["network_request"], false);
    assert_eq!(result["history_upload_enabled"], false);
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("PRIVATE-MARKER")
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_budget_is_rejected_without_echoing_input() {
    let output = run(&["--request-budget", "9000", "--complete", "PRIVATE-COMMAND"]);
    assert!(!output.status.success());
    assert!(
        !String::from_utf8(output.stderr)
            .unwrap()
            .contains("PRIVATE-COMMAND")
    );
}

#[test]
fn risk_policy_is_explicit_and_headless_reports_unknown_dialect() {
    for policy in ["warn", "confirm", "off"] {
        let output = run(&["--risk-policy", policy, "--complete", "git st"]);
        assert!(output.status.success(), "{:?}", output.stderr);
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["risk_policy"], policy);
        assert_eq!(result["risk_review"]["incomplete"], true);
        assert_eq!(
            result["additional_acceptance_required"],
            policy == "confirm"
        );
    }
    let output = run(&[
        "--risk-policy",
        "confirm",
        "--shell",
        "bash",
        "--complete",
        "echo 世",
    ]);
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["additional_acceptance_required"], false);
    assert!(
        !run(&["--risk-policy", "safe", "--complete", "git st"])
            .status
            .success()
    );
}

#[test]
fn risk_preview_reviews_full_result_not_only_insertion() {
    let output = run(&[
        "--risk-policy",
        "confirm",
        "--shell",
        "bash",
        "--complete",
        "echo 世 > /synthetic/output",
        "--cursor",
        "8",
    ]);
    assert!(output.status.success(), "{:?}", output.stderr);
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["insertion"], "");
    assert_eq!(result["additional_acceptance_required"], false);
    assert!(
        !result["risk_review"]["warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn repository_memory_cannot_override_risk_policy() {
    let path = std::env::temp_dir().join(format!(
        "reedline-ai-risk-policy-{}.json",
        std::process::id()
    ));
    std::fs::write(&path, r#"{"version":1,"confirmed":{},"risk_policy":"off"}"#).unwrap();
    let output = run(&[
        "suggest",
        "--shell",
        "bash",
        "--risk-policy",
        "confirm",
        "--config",
        path.to_str().unwrap(),
        "--buffer",
        "git st",
    ]);
    std::fs::remove_file(path).unwrap();
    assert!(!output.status.success());
}
