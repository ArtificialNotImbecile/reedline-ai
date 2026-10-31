//! Local synthetic stalled servers only: no provider, credentials, or real history.
use reedline_ai::{
    backend::{Backend, BackendError, Cancellation, HttpBackend},
    context::Context,
    engine::{Engine, EngineConfig, Outcome},
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

fn read_request(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    let end = loop {
        let mut buffer = [0; 1024];
        let n = stream.read(&mut buffer).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(i) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let size: usize = String::from_utf8_lossy(&bytes[..end])
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .unwrap();
    while bytes.len() < end + size {
        let mut buffer = [0; 1024];
        let n = stream.read(&mut buffer).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
    }
}

fn endpoint(listener: &TcpListener) -> String {
    format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    )
}
fn backend(url: &str) -> HttpBackend {
    HttpBackend::new(url, "synthetic-only", None, Duration::from_secs(10)).unwrap()
}
fn engine(url: &str) -> Engine {
    Engine::new(
        Arc::new(backend(url)),
        EngineConfig {
            debounce: Duration::ZERO,
            ..EngineConfig::default()
        },
    )
}
fn receive<T>(rx: &mpsc::Receiver<T>) -> T {
    rx.recv_timeout(Duration::from_secs(2)).unwrap()
}
fn wait_ready(engine: &Engine) -> Outcome {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        let outcome = engine.outcome();
        if !matches!(outcome, Outcome::Pending) {
            return outcome;
        }
        assert!(
            Instant::now() < until,
            "new request blocked behind stale HTTP"
        );
        thread::sleep(Duration::from_millis(5));
    }
}
// The server never finishes its response. Only client disconnect can complete it.
fn stalled(
    partial_body: bool,
) -> (
    String,
    mpsc::Receiver<()>,
    mpsc::Receiver<()>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = endpoint(&listener);
    let (started_tx, started_rx) = mpsc::channel();
    let (closed_tx, closed_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        if partial_body {
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nContent-Type: application/json\r\n\r\n{").unwrap();
            stream.flush().unwrap();
        }
        started_tx.send(()).unwrap();
        let mut byte = [0];
        match stream.read(&mut byte) {
            Ok(0) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ) => {}
            other => panic!("HTTP connection not cancelled: {other:?}"),
        }
        closed_tx.send(()).unwrap();
    });
    (url, started_rx, closed_rx, handle)
}

#[test]
fn new_generation_cancels_old_socket_and_completes_without_waiting_for_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = endpoint(&listener);
    let (started_tx, started_rx) = mpsc::channel();
    let (closed_tx, closed_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut old, _) = listener.accept().unwrap();
        read_request(&mut old);
        started_tx.send(()).unwrap();
        let mut byte = [0];
        assert_eq!(old.read(&mut byte).unwrap(), 0, "old HTTP was not closed");
        closed_tx.send(()).unwrap();
        let (mut new, _) = listener.accept().unwrap();
        read_request(&mut new);
        let body = r#"{"choices":[{"message":{"content":" done"},"finish_reason":"stop"}]}"#;
        write!(
            new,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let engine = engine(&url);
    engine.request("old", 3);
    receive(&started_rx);
    let start = Instant::now();
    engine.request("new", 3);
    receive(&closed_rx);
    assert_eq!(wait_ready(&engine), Outcome::Ready(" done".into()));
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(
        engine.outcome_for(&Context::new("old", 3).unwrap()),
        Outcome::Idle
    );
    handle.join().unwrap();
}

#[test]
fn clear_blank_invalid_and_native_preference_abort_pending_http() {
    for operation in 0..4 {
        let (url, started, closed, server) = stalled(false);
        let engine = engine(&url);
        engine.request("old", 3);
        receive(&started);
        match operation {
            0 => engine.clear(),
            1 => {
                assert_eq!(engine.request("  ", 1), Outcome::Idle);
            }
            2 => {
                assert!(matches!(engine.request("世", 1), Outcome::Error(_)));
            }
            _ => engine.prefer_local(Context::new("old", 3).unwrap(), " local", " local"),
        }
        receive(&closed);
        thread::sleep(Duration::from_millis(10));
        assert!(!matches!(engine.outcome(), Outcome::Ready(_)));
        if operation == 3 {
            assert_eq!(engine.take_rendered_hint(), Some(" local".into()));
        }
        server.join().unwrap();
    }
}

#[test]
fn body_wait_is_cancelled_not_only_connect_or_headers() {
    let (url, started, closed, server) = stalled(true);
    let engine = engine(&url);
    engine.request("old", 3);
    receive(&started);
    // Allow headers to arrive while leaving the body incomplete.
    thread::sleep(Duration::from_millis(30));
    engine.clear();
    receive(&closed);
    assert_eq!(engine.outcome(), Outcome::Idle);
    server.join().unwrap();
}

#[test]
fn dropping_last_engine_cancels_and_joins_promptly() {
    let (url, started, closed, server) = stalled(false);
    let engine = engine(&url);
    engine.request("old", 3);
    receive(&started);
    let start = Instant::now();
    drop(engine);
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "exit waited for HTTP timeout"
    );
    receive(&closed);
    server.join().unwrap();
}

#[test]
fn already_cancelled_request_never_connects() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let backend = backend(&endpoint(&listener));
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert_eq!(
        backend.complete_cancellable(&Context::new("x", 1).unwrap(), &cancellation),
        Err(BackendError::Cancelled)
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
