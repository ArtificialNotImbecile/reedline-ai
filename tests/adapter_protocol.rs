use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn run(input: &[u8], args: &[&str]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_reedline-ai"))
        .arg("--adapter-request")
        .args(args)
        .env_remove("REEDLINE_AI_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}
fn request(buffer: &str, cursor: usize) -> Value {
    json!({"protocol":1,"request_id":"fixture-1","shell":{"kind":"bash","version":"5.2"},"cwd":"/synthetic-do-not-read","buffer":buffer,"cursor":cursor})
}
#[test]
fn headless_protocol_echoes_identity_and_utf8_cursor() {
    let output = run(request("echo 世", 8).to_string().as_bytes(), &[]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let r: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(r["request_id"], "fixture-1");
    assert_eq!(r["buffer"], "echo 世");
    assert_eq!(r["result"], "echo 世界");
    assert_eq!(r["result_cursor"], 11);
    assert_eq!(r["additional_acceptance_required"], false);
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("synthetic-do-not-read")
    );
}
#[test]
fn blank_has_no_network_request() {
    let output = run(
        request("   ", 3).to_string().as_bytes(),
        &[
            "--backend",
            "http",
            "--endpoint",
            "http://127.0.0.1:1/v1/chat/completions",
            "--model",
            "test",
        ],
    );
    assert!(output.status.success());
    let r: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(r["insertion"], "");
}
#[test]
fn malformed_private_request_is_redacted() {
    for input in [
        b"{\"PRIVATE_SECRET\":1}".to_vec(),
        request("PRIVATE世", 8).to_string().into_bytes(),
    ] {
        let output = run(&input, &[]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            !String::from_utf8(output.stderr)
                .unwrap()
                .contains("PRIVATE")
        );
    }
}
#[test]
fn adapter_rejects_history_missing_config_and_shell_override() {
    for args in [
        vec!["--history-file", "synthetic-never-create"],
        vec!["--ai-history"],
        vec!["--shell", "cmd"],
        vec!["--global-config", "synthetic-never-read"],
        vec!["--complete", "pwd"],
    ] {
        let output = run(b"{}", &args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
#[test]
fn adapter_risk_gate_is_not_disabled_by_cli_policy() {
    let output = run(
        request("git st", 6).to_string().as_bytes(),
        &["--risk-policy", "off"],
    );
    assert!(output.status.success());
    let r: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(r["additional_acceptance_required"], true);
}

#[test]
fn explicit_workflow_v2_replaces_locally_and_v1_stays_insertion_only() {
    let memory = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/project-memory.synthetic.json"
    );
    for shell in ["bash", "powershell", "nushell", "cmd"] {
        let mut req = request("启动监控", 6);
        req["protocol"] = json!(2);
        req["shell"]["kind"] = json!(shell);
        let output = run(req.to_string().as_bytes(), &["--config", memory]);
        assert!(
            output.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(reply["protocol"], 2);
        assert_eq!(reply["insertion"], "");
        assert_eq!(reply["result"], reply["replacement"]);
        assert_eq!(
            reply["result_cursor"],
            reply["result"].as_str().unwrap().len()
        );
        assert!(reply["result"].as_str().unwrap().contains("app.py"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-do-not-read"));
        req["protocol"] = json!(1);
        assert!(
            !run(req.to_string().as_bytes(), &["--config", memory])
                .status
                .success()
        );
    }
}

#[test]
fn v2_without_explicit_memory_retains_strict_insertion() {
    let mut req = request("echo 世", 8);
    req["protocol"] = json!(2);
    let output = run(req.to_string().as_bytes(), &[]);
    assert!(output.status.success());
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(reply.get("replacement").is_none());
    assert_eq!(reply["result"], "echo 世界");
    assert_eq!(reply["result_cursor"], 11);
}
