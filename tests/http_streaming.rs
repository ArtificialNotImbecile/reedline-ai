//! Synthetic loopback streaming only; no credentials or provider usage.
use reedline::{Completer, FileBackedHistory, Hinter};
use reedline_ai::{
    backend::{Backend, BackendError, Cancellation, HttpBackend},
    context::Context,
    editor::{AiCompleter, AiHinter},
    engine::{Engine, EngineConfig, Outcome},
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};
fn request(socket: &mut TcpStream) -> serde_json::Value {
    socket
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut buf = [0; 2048];
        let n = socket.read(&mut buf).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buf[..n]);
        if let Some(i) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let len: usize = String::from_utf8_lossy(&bytes[..header_end])
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().unwrap())
        })
        .unwrap();
    while bytes.len() < header_end + len {
        let mut buf = [0; 2048];
        let n = socket.read(&mut buf).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buf[..n]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + len]).unwrap()
}
fn backend(url: &str) -> HttpBackend {
    HttpBackend::new(url, "synthetic", None, Duration::from_secs(5)).unwrap()
}
fn listener() -> (TcpListener, String) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/chat/completions", l.local_addr().unwrap());
    (l, url)
}
fn delta(text: &str) -> String {
    format!(
        "data: {}\r\n\r\n",
        serde_json::json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]})
    )
}
const END: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
fn headers(s: &mut TcpStream, len: usize, mime: &str) {
    write!(s,"HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n").unwrap();
}
fn response(body: String, mime: &'static str) -> (String, thread::JoinHandle<()>) {
    let (l, url) = listener();
    let handle = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let req = request(&mut s);
        assert_eq!(req["stream"], true);
        headers(&mut s, body.len(), mime);
        // Split every byte, including UTF-8 sequences and SSE delimiters.
        for byte in body.as_bytes() {
            if s.write_all(&[*byte]).is_err() {
                break;
            }
        }
    });
    (url, handle)
}
fn wait(engine: &Engine, predicate: impl Fn(&Outcome) -> bool) -> Outcome {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let out = engine.outcome();
        if predicate(&out) {
            return out;
        }
        assert!(Instant::now() < deadline, "{out:?}");
        thread::sleep(Duration::from_millis(3));
    }
}
#[test]
fn previews_are_incremental_but_not_acceptable_until_validated_done() {
    let (l, url) = listener();
    let (release, gate) = mpsc::channel();
    let prefix = delta("世");
    let tail = format!("{}{END}", delta("界"));
    let handle = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        assert_eq!(request(&mut s)["stream"], true);
        headers(
            &mut s,
            prefix.len() + tail.len(),
            "text/event-stream; charset=utf-8",
        );
        s.write_all(prefix.as_bytes()).unwrap();
        s.flush().unwrap();
        gate.recv_timeout(Duration::from_secs(4)).unwrap();
        s.write_all(tail.as_bytes()).unwrap();
    });
    let engine = Engine::new(
        Arc::new(backend(&url)),
        EngineConfig {
            debounce: Duration::ZERO,
            ..Default::default()
        },
    );
    let mut hinter = AiHinter::new(engine.clone());
    let history = FileBackedHistory::new(0).unwrap();
    hinter.handle("echo ", 5, &history, false, "");
    assert_eq!(
        wait(&engine, |o| matches!(o, Outcome::Streaming(_))),
        Outcome::Streaming("世".into())
    );
    assert_eq!(
        hinter.handle("echo ", 5, &history, false, ""),
        "世 [receiving; not ready]"
    );
    assert_eq!(engine.take_rendered_hint(), None);
    assert_eq!(engine.take_rendered_hint_word(), None);
    assert_eq!(hinter.complete_hint(), "");
    assert_eq!(engine.poll(), (false, true));
    assert!(
        AiCompleter::new(engine.clone())
            .complete("echo ", 5)
            .is_pending()
    );
    release.send(()).unwrap();
    assert_eq!(
        wait(&engine, |o| matches!(o, Outcome::Ready(_))),
        Outcome::Ready("世界".into())
    );
    assert_eq!(hinter.handle("echo ", 5, &history, false, ""), "世界");
    assert_eq!(engine.take_rendered_hint(), Some("世界".into()));
    handle.join().unwrap();
}
#[test]
fn chunk_boundaries_comments_role_usage_and_unicode_are_supported() {
    let body = format!(
        ": keepalive\r\nevent: message\r\n\r\ndata: {{\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\"}},\"finish_reason\":null}}]}}\r\n\r\n{}{}data: {{\"choices\":[],\"usage\":{{\"total_tokens\":8}}}}\n\n{END}",
        delta("世"),
        delta("界")
    );
    let (url, handle) = response(body, "text/event-stream");
    let mut seen = vec![];
    let out = backend(&url)
        .complete_progressive(
            &Context::new("echo ", 5).unwrap(),
            &Cancellation::default(),
            &mut |text| seen.push(text.to_string()),
        )
        .unwrap();
    assert_eq!(out, "世界");
    assert_eq!(seen, vec!["世", "世界"]);
    handle.join().unwrap();
}
#[test]
fn truncated_tool_refusal_unsafe_and_malformed_streams_are_rejected() {
    for body in [
        delta("partial"),
        format!("{}data: [DONE]\n\n", delta("partial")),
        "data: not-json\n\n".into(),
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\n".into(),
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[]}}]}\n\n".into(),
        "data: {\"choices\":[{\"delta\":{\"refusal\":\"no\"}}]}\n\n".into(),
        format!("{}{END}", delta("\u{1b}[2J")),
        format!("{}{END}", delta(&"x".repeat(1025))),
        format!(
            "data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n{}{END}",
            delta("late")
        ),
    ] {
        let (url, handle) = response(body, "text/event-stream");
        assert!(
            backend(&url)
                .complete_progressive(
                    &Context::new("x", 1).unwrap(),
                    &Cancellation::default(),
                    &mut |_| {}
                )
                .is_err()
        );
        handle.join().unwrap();
    }
}
#[test]
fn same_response_json_fallback_no_second_request_and_original_api_stays_nonstreaming() {
    let body = r#"{"choices":[{"message":{"content":" ok"},"finish_reason":"stop"}]}"#;
    let (url, handle) = response(body.into(), "application/json");
    let mut previews = 0;
    assert_eq!(
        backend(&url)
            .complete_progressive(
                &Context::new("x", 1).unwrap(),
                &Cancellation::default(),
                &mut |_| previews += 1
            )
            .unwrap(),
        " ok"
    );
    assert_eq!(previews, 0);
    handle.join().unwrap();
    let (l, url) = listener();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        assert_eq!(request(&mut s)["stream"], false);
        headers(&mut s, body.len(), "application/json");
        s.write_all(body.as_bytes()).unwrap();
    });
    assert_eq!(
        backend(&url)
            .complete(&Context::new("x", 1).unwrap())
            .unwrap(),
        " ok"
    );
    h.join().unwrap();
}
#[test]
fn cancelling_partial_stream_clears_preview_and_closes_socket() {
    let (l, url) = listener();
    let (closed, closed_rx) = mpsc::channel();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        request(&mut s);
        headers(&mut s, 600, "text/event-stream");
        s.write_all(delta(" partial").as_bytes()).unwrap();
        s.flush().unwrap();
        let mut b = [0];
        match s.read(&mut b) {
            Ok(0) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ) => {}
            other => panic!("socket not closed {other:?}"),
        }
        closed.send(()).unwrap();
    });
    let engine = Engine::new(
        Arc::new(backend(&url)),
        EngineConfig {
            debounce: Duration::ZERO,
            ..Default::default()
        },
    );
    engine.request("echo", 4);
    wait(&engine, |o| matches!(o, Outcome::Streaming(_)));
    engine.clear();
    closed_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(engine.outcome(), Outcome::Idle);
    assert!(engine.take_rendered_hint().is_none());
    h.join().unwrap();
}
#[test]
fn oversized_sse_line_is_bounded() {
    let (url, h) = response(
        format!("data: {}\n\n", "x".repeat(17000)),
        "text/event-stream",
    );
    assert_eq!(
        backend(&url).complete_progressive(
            &Context::new("x", 1).unwrap(),
            &Cancellation::default(),
            &mut |_| {}
        ),
        Err(BackendError::BodyTooLarge)
    );
    h.join().unwrap();
}

#[test]
fn stale_noncooperative_callbacks_do_not_resurrect_cleared_preview() {
    struct LateBackend {
        started: mpsc::Sender<()>,
        gate: std::sync::Mutex<mpsc::Receiver<()>>,
        finished: mpsc::Sender<()>,
    }
    impl Backend for LateBackend {
        fn complete(&self, _: &Context) -> Result<String, BackendError> {
            unreachable!()
        }
        fn complete_progressive(
            &self,
            _: &Context,
            _: &Cancellation,
            preview: &mut dyn FnMut(&str),
        ) -> Result<String, BackendError> {
            preview("early");
            self.started.send(()).unwrap();
            self.gate.lock().unwrap().recv().unwrap();
            preview("late");
            self.finished.send(()).unwrap();
            Ok("late".into())
        }
    }
    let (started, start) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (finished, finish) = mpsc::channel();
    let engine = Engine::new(
        Arc::new(LateBackend {
            started,
            gate: std::sync::Mutex::new(gate),
            finished,
        }),
        EngineConfig {
            debounce: Duration::ZERO,
            ..EngineConfig::default()
        },
    );
    engine.request("echo ", 5);
    start.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(engine.outcome(), Outcome::Streaming("early".into()));
    engine.clear();
    release.send(()).unwrap();
    finish.recv_timeout(Duration::from_secs(2)).unwrap();
    thread::sleep(Duration::from_millis(20));
    assert_eq!(engine.outcome(), Outcome::Idle);
    assert_eq!(engine.take_rendered_hint(), None);
}

#[test]
fn failed_partial_stream_is_not_published_as_ready_or_accepted() {
    let (url, handle) = response(
        format!("{}data: broken-json\n\n", delta("unsafe-to-accept")),
        "text/event-stream",
    );
    let engine = Engine::new(
        Arc::new(backend(&url)),
        EngineConfig {
            debounce: Duration::ZERO,
            ..EngineConfig::default()
        },
    );
    engine.request("echo ", 5);
    wait(&engine, |out| matches!(out, Outcome::Error(_)));
    assert_eq!(engine.take_rendered_hint(), None);
    assert_eq!(engine.take_rendered_hint_word(), None);
    handle.join().unwrap();
}

#[test]
fn json_fallback_also_rejects_tool_calls_and_refusals() {
    for field in ["tool_calls", "function_call", "refusal"] {
        let mut body = serde_json::json!({"choices":[{"message":{"content":"looks-safe"},"finish_reason":"stop"}]});
        body["choices"][0]["message"][field] = serde_json::json!("unexpected");
        let (url, handle) = response(body.to_string(), "application/json");
        let result = backend(&url).complete_progressive(
            &Context::new("echo ", 5).unwrap(),
            &Cancellation::default(),
            &mut |_| panic!("JSON fallback must not preview"),
        );
        assert!(matches!(result, Err(BackendError::InvalidResponse)));
        handle.join().unwrap();
    }
}

#[test]
fn both_response_shapes_reject_mixed_choices_wrong_index_and_wrong_role() {
    for streaming in [false, true] {
        for mutation in ["multiple", "index", "role"] {
            let mut choice = if streaming {
                serde_json::json!({"index":0,"delta":{"content":"text","role":"assistant"},"finish_reason":null})
            } else {
                serde_json::json!({"index":0,"message":{"content":"text","role":"assistant"},"finish_reason":"stop"})
            };
            if mutation == "index" {
                choice["index"] = serde_json::json!(1);
            }
            if mutation == "role" {
                choice[if streaming { "delta" } else { "message" }]["role"] =
                    serde_json::json!("user");
            }
            let choices = if mutation == "multiple" {
                vec![choice.clone(), choice]
            } else {
                vec![choice]
            };
            let body = serde_json::json!({"choices":choices}).to_string();
            let (body, mime) = if streaming {
                (format!("data: {body}\n\n{END}"), "text/event-stream")
            } else {
                (body, "application/json")
            };
            let (url, handle) = response(body, mime);
            let result = backend(&url).complete_progressive(
                &Context::new("echo ", 5).unwrap(),
                &Cancellation::default(),
                &mut |_| {},
            );
            assert!(
                matches!(result, Err(BackendError::InvalidResponse)),
                "{streaming} {mutation}: {result:?}"
            );
            handle.join().unwrap();
        }
    }
}

#[test]
fn completed_stream_keeps_final_risk_confirmation_gate() {
    use reedline_ai::{review_menu::ReviewGate, risk::RiskPolicy};
    let (url, handle) = response(
        format!("{}{END}", delta(" && rm -rf /tmp/example")),
        "text/event-stream",
    );
    let engine = Engine::new(
        Arc::new(backend(&url)),
        EngineConfig {
            debounce: Duration::ZERO,
            ..EngineConfig::default()
        },
    );
    let mut hinter =
        AiHinter::new(engine.clone()).with_risk_review(RiskPolicy::Confirm, ReviewGate::default());
    let history = FileBackedHistory::new(0).unwrap();
    hinter.handle("echo safe", 9, &history, false, "");
    wait(&engine, |out| matches!(out, Outcome::Ready(_)));
    assert_eq!(hinter.handle("echo safe", 9, &history, false, ""), "");
    assert_eq!(engine.take_rendered_hint(), None);
    assert_eq!(engine.take_rendered_hint_word(), None);
    handle.join().unwrap();
}
