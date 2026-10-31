use reedline_ai::settings::{Settings, load, save};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "reedline-settings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn settings(endpoint: &str) -> Settings {
    Settings {
        version: 1,
        endpoint: endpoint.into(),
        model: "synthetic-model".into(),
        api_key: Some("SYNTHETIC-SECRET".into()),
    }
}
fn run(dir: &Temp, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_reedline-ai"))
        .args(args)
        .env("REEDLINE_AI_SETTINGS_DIR", dir.0.join("app"))
        .env_remove("REEDLINE_AI_API_KEY")
        .output()
        .unwrap()
}
#[test]
fn saves_private_settings_and_refuses_symlinks() {
    let dir = Temp::new();
    let file = dir.0.join("app/settings.json");
    save(&file, &settings("https://example.test/v1/chat/completions")).unwrap();
    assert_eq!(load(&file).unwrap().model, "synthetic-model");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(file.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let link = dir.0.join("link");
        symlink(&file, &link).unwrap();
        assert!(load(&link).is_err());
        assert!(save(&link, &settings("https://example.test/v1/chat/completions")).is_err());
    }
}
#[test]
fn broken_settings_and_keys_never_echo_secrets() {
    let dir = Temp::new();
    let file = dir.0.join("app/settings.json");
    save(&file, &settings("https://example.test/v1/chat/completions")).unwrap();
    fs::write(&file, b"{\"api_key\":\"SYNTHETIC-SECRET\",\"bad\":").unwrap();
    let out = run(&dir, &["--configured", "--complete", ""]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("SYNTHETIC"));
}
#[test]
fn configuration_requires_explicit_opt_in_and_conflicts_fail() {
    let dir = Temp::new();
    save(
        &dir.0.join("app/settings.json"),
        &settings("http://127.0.0.1:1/chat/completions"),
    )
    .unwrap();
    let offline = run(&dir, &["--complete", "git st"]);
    assert!(offline.status.success());
    assert!(String::from_utf8_lossy(&offline.stdout).contains("git status"));
    let empty = run(&dir, &["--configured", "--complete", ""]);
    assert!(
        empty.status.success(),
        "{}",
        String::from_utf8_lossy(&empty.stderr)
    );
    let mixed = run(
        &dir,
        &["--configured", "--backend", "mock", "--complete", "git st"],
    );
    assert!(!mixed.status.success());
    let setup = run(&dir, &["setup"]);
    assert!(!setup.status.success());
    assert!(String::from_utf8_lossy(&setup.stderr).contains("interactive terminal"));
}
#[test]
fn configured_request_uses_generic_chat_protocol_and_environment_override() {
    for override_key in [None, Some("OVERRIDE-SECRET")] {
        let dir = Temp::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        save(
            &dir.0.join("app/settings.json"),
            &settings(&format!(
                "http://{}/v1/chat/completions",
                listener.local_addr().unwrap()
            )),
        )
        .unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut data = Vec::new();
            let mut buf = [0; 1024];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buf[..n]);
                if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..end]);
                    let len = headers
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(|n| n.parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if data.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            let body = r#"{"choices":[{"message":{"content":"atus"}}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            data
        });
        let mut command = Command::new(env!("CARGO_BIN_EXE_reedline-ai"));
        command
            .args(["--configured", "--complete", "git st"])
            .env("REEDLINE_AI_SETTINGS_DIR", dir.0.join("app"))
            .env_remove("REEDLINE_AI_API_KEY");
        if let Some(key) = override_key {
            command.env("REEDLINE_AI_API_KEY", key);
        }
        let out = command.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let request = String::from_utf8(server.join().unwrap()).unwrap();
        assert!(request.starts_with("POST /v1/chat/completions "));
        assert!(request.contains(&format!(
            "Bearer {}",
            override_key.unwrap_or("SYNTHETIC-SECRET")
        )));
        let body: serde_json::Value =
            serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["model"], "synthetic-model");
        assert!(body.get("thinking").is_none());
        assert!(body.get("messages").is_some());
        assert!(!String::from_utf8_lossy(&out.stdout).contains("SECRET"));
    }
}
#[test]
fn init_is_idempotent_quotes_paths_and_refuses_modified_assets() {
    let dir = Temp::new();
    let first = run(&dir, &["init", "--shell", "bash", "--mock"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("REEDLINE_AI_ARGS=(--backend mock)"));
    let second = run(&dir, &["init", "--shell", "bash", "--mock"]);
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
    // Windows may resolve `bash` to the WSL launcher. Allow an explicit
    // installed Bash for this optional syntax check without changing PATH.
    let bash = std::env::var_os("REEDLINE_AI_TEST_BASH").unwrap_or_else(|| "bash".into());
    let syntax = Command::new(bash)
        .args([
            "--noprofile",
            "--norc",
            "-n",
            "-c",
            &String::from_utf8(first.stdout).unwrap(),
        ])
        .output();
    if let Ok(out) = syntax {
        assert!(
            out.status.success(),
            "Bash syntax check failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let adapter = fs::read_dir(dir.0.join("app/adapters"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("reedline-ai.bash");
    fs::write(adapter, b"modified").unwrap();
    let rejected = run(&dir, &["init", "--shell", "bash"]);
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
}

#[test]
fn oversize_save_keeps_previous_settings_unchanged() {
    let dir = Temp::new();
    let path = dir.0.join("app/settings.json");
    save(&path, &settings("https://example.test/v1/chat/completions")).unwrap();
    let before = fs::read(&path).unwrap();
    let huge = settings(&format!(
        "https://example.test/{}/chat/completions",
        "a".repeat(17000)
    ));
    assert!(save(&path, &huge).is_err());
    assert_eq!(fs::read(path).unwrap(), before);
}
