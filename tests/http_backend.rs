//! Deterministic localhost-only protocol tests. No credentials or paid APIs.
use reedline_ai::{
    backend::{Backend, BackendError, HttpBackend, HttpProvider},
    context::Context,
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn server(
    status: &str,
    body: String,
    extra_headers: &str,
    delay: Duration,
) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let (tx, rx) = mpsc::channel();
    let status = status.to_string();
    let extra_headers = extra_headers.to_string();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        let end;
        loop {
            let mut buffer = [0; 1024];
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                end = index + 4;
                break;
            }
        }
        let headers = String::from_utf8_lossy(&bytes[..end]);
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        while bytes.len() < end + length {
            let mut buffer = [0; 4096];
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&buffer[..read]);
        }
        let _ = tx.send(String::from_utf8(bytes).unwrap());
        thread::sleep(delay);
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
    });
    (endpoint, rx, handle)
}
fn response(text: &str) -> String {
    json!({"choices":[{"message":{"content":text},"finish_reason":"stop"}]}).to_string()
}
fn call(body: String) -> Result<String, BackendError> {
    let (endpoint, _request, handle) = server("200 OK", body, "", Duration::ZERO);
    let backend =
        HttpBackend::new(&endpoint, "local-test-model", None, Duration::from_secs(1)).unwrap();
    let result = backend.complete(&Context::new("git st", 6).unwrap());
    handle.join().unwrap();
    result
}

#[test]
fn request_contains_fim_context_and_only_explicit_auth() {
    let (endpoint, request, handle) = server("200 OK", response("status"), "", Duration::ZERO);
    let backend = HttpBackend::new(
        &endpoint,
        "test-model",
        Some("TEST_ONLY_NOT_A_REAL_KEY"),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(
        backend
            .complete(&Context::new("git  --short", 4).unwrap())
            .unwrap(),
        "status"
    );
    let request = request.recv().unwrap();
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer test_only_not_a_real_key")
    );
    let (_, body) = request.split_once("\r\n\r\n").unwrap();
    let body: Value = serde_json::from_str(body).unwrap();
    let context: Value =
        serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(context, json!({"prefix":"git ","suffix":" --short"}));
    assert_eq!(body["stream"], false);
    assert_eq!(body["max_tokens"], 128);
    assert!(body.get("thinking").is_none());
    assert!(!body.to_string().contains("TEST_ONLY_NOT_A_REAL_KEY"));
    handle.join().unwrap();
}

#[test]
fn deepseek_opt_in_disables_thinking_on_the_wire() {
    let (endpoint, request, handle) = server("200 OK", response("atus"), "", Duration::ZERO);
    let backend = HttpBackend::new(&endpoint, "deepseek-flash", None, Duration::from_secs(1))
        .unwrap()
        .with_provider(HttpProvider::DeepSeek);
    assert_eq!(
        backend
            .complete(&Context::new("git st", 6).unwrap())
            .unwrap(),
        "atus"
    );
    let request = request.recv().unwrap();
    let (_, body) = request.split_once("\r\n\r\n").unwrap();
    let body: Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["thinking"], json!({"type":"disabled"}));
    assert_eq!(body["max_tokens"], 128);
    assert_eq!(body["stream"], false);
    assert!(!request.to_ascii_lowercase().contains("authorization:"));
    handle.join().unwrap();
}

#[test]
fn explicit_usage_reports_numeric_fields_only() {
    let body = json!({
        "choices": [{"message": {"content": "atus"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 5, "total_tokens": 105,
                  "prompt_cache_hit_tokens": 90, "prompt_cache_miss_tokens": 10,
                  "provider_private_message": "DO_NOT_REPORT"},
        "provider_private_message": "DO_NOT_REPORT"
    })
    .to_string();
    let (endpoint, _request, handle) = server("200 OK", body, "", Duration::ZERO);
    let backend = HttpBackend::new(&endpoint, "test", None, Duration::from_secs(1)).unwrap();
    let completion = backend
        .complete_with_usage(&Context::new("git st", 6).unwrap())
        .unwrap();
    assert_eq!(completion.insertion, "atus");
    let usage = serde_json::to_value(completion.usage.unwrap()).unwrap();
    assert_eq!(usage["prompt_tokens"], 100);
    assert_eq!(usage["completion_tokens"], 5);
    assert_eq!(usage["total_tokens"], 105);
    assert_eq!(usage["prompt_cache_hit_tokens"], 90);
    assert!(!usage.to_string().contains("DO_NOT_REPORT"));
    handle.join().unwrap();
}

#[test]
fn environment_proxy_is_used_only_after_explicit_opt_in() {
    for allow_proxy in [false, true] {
        let (endpoint, request, handle) = server("200 OK", response("atus"), "", Duration::ZERO);
        let proxy = if allow_proxy {
            endpoint.trim_end_matches("/v1/chat/completions")
        } else {
            "http://127.0.0.1:1"
        };
        let target = if allow_proxy {
            "http://127.0.0.2:9/v1/chat/completions"
        } else {
            &endpoint
        };
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_reedline-ai"));
        command.args([
            "--backend",
            "http",
            "--endpoint",
            target,
            "--model",
            "test",
            "--timeout-seconds",
            "2",
            "--complete",
            "git st",
        ]);
        command.env_remove("REEDLINE_AI_API_KEY");
        for name in [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ] {
            command.env_remove(name);
        }
        command
            .env("HTTP_PROXY", proxy)
            .env("http_proxy", proxy)
            .env("NO_PROXY", "")
            .env("no_proxy", "");
        if allow_proxy {
            command.arg("--allow-env-proxy");
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "local proxy fixture failed");
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["insertion"], "atus");
        let request = request.recv_timeout(Duration::from_secs(2)).unwrap();
        if allow_proxy {
            assert!(request.starts_with("POST http://127.0.0.2:9/v1/chat/completions HTTP/1.1"));
        } else {
            assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
        }
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        handle.join().unwrap();
    }
}

#[test]
fn provider_errors_do_not_echo_private_response_bodies() {
    let (endpoint, _request, handle) = server(
        "401 Unauthorized",
        "PRIVATE_SECRET_FROM_PROVIDER".into(),
        "",
        Duration::ZERO,
    );
    let backend = HttpBackend::new(&endpoint, "test", None, Duration::from_secs(1)).unwrap();
    let error = backend
        .complete(&Context::new("private", 7).unwrap())
        .unwrap_err();
    assert_eq!(error, BackendError::Status(401));
    assert!(!error.to_string().contains("PRIVATE"));
    handle.join().unwrap();
}

#[test]
fn redirects_are_not_followed_with_buffer_or_credentials() {
    let destination = TcpListener::bind("127.0.0.1:0").unwrap();
    destination.set_nonblocking(true).unwrap();
    let redirect = format!(
        "Location: http://{}/stolen\r\n",
        destination.local_addr().unwrap()
    );
    let (endpoint, _request, handle) = server(
        "307 Temporary Redirect",
        String::new(),
        &redirect,
        Duration::ZERO,
    );
    let backend =
        HttpBackend::new(&endpoint, "test", Some("TEST_ONLY"), Duration::from_secs(1)).unwrap();
    assert_eq!(
        backend.complete(&Context::new("private", 7).unwrap()),
        Err(BackendError::Status(307))
    );
    assert_eq!(
        destination.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    handle.join().unwrap();
}

#[test]
fn malformed_oversized_truncated_and_unsafe_outputs_fail_closed() {
    assert_eq!(call("not json".into()), Err(BackendError::InvalidResponse));
    assert_eq!(
        call(json!({"choices":[]}).to_string()),
        Err(BackendError::InvalidResponse)
    );
    assert_eq!(call("x".repeat(65_537)), Err(BackendError::BodyTooLarge));
    assert_eq!(
        call(
            json!({"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]})
                .to_string()
        ),
        Err(BackendError::InvalidResponse)
    );
    for text in ["\x1b[2J", "a\nb", "```bash", "x\u{202e}y"] {
        assert!(matches!(
            call(response(text)),
            Err(BackendError::UnsafeOutput(_))
        ));
    }
}

#[test]
fn slow_provider_times_out_with_redacted_error() {
    let (endpoint, _request, handle) =
        server("200 OK", response("late"), "", Duration::from_millis(200));
    let backend = HttpBackend::new(&endpoint, "test", None, Duration::from_millis(40)).unwrap();
    let now = Instant::now();
    assert_eq!(
        backend.complete(&Context::new("a", 1).unwrap()),
        Err(BackendError::Transport)
    );
    assert!(now.elapsed() < Duration::from_secs(1));
    handle.join().unwrap();
}
