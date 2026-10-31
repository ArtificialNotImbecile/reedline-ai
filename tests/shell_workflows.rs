//! Offline contract tests. All projects, paths and commands are synthetic.
use reedline::{Completer, Editor, Hinter, Menu, UndoBehavior};
use reedline_ai::{
    backend::{Backend, BackendError, HttpBackend, request_body},
    context::Context,
    editor::{AiCompleter, AiHinter, completion_menu},
    engine::{Engine, EngineConfig},
    local_history::LocalHistory,
    project_memory::ProjectMemory,
    shell::{ShellContext, ShellKind, quote_literal},
    workflow::suggest_local,
};
use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn memory() -> ProjectMemory {
    ProjectMemory::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/project-memory.synthetic.json"
    ))
    .unwrap()
}
fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_reedline-ai"))
        .args(args)
        .env_remove("REEDLINE_AI_API_KEY")
        .output()
        .unwrap()
}
struct Count(Arc<AtomicUsize>);
impl Backend for Count {
    fn complete(&self, _: &Context) -> Result<String, BackendError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok("unexpected".into())
    }
}
fn engine(calls: Arc<AtomicUsize>) -> Engine {
    Engine::new(
        Arc::new(Count(calls)),
        EngineConfig {
            debounce: Duration::ZERO,
            ..EngineConfig::default()
        },
    )
}

#[test]
fn configured_workflow_is_exact_local_and_never_inferred_from_notes() {
    let mem = memory();
    let context = Context::new("run CCTV", 8)
        .unwrap()
        .with_shell(ShellContext::new(ShellKind::Bash))
        .unwrap();
    let s = suggest_local(&context, Some(&mem)).unwrap().unwrap();
    assert_eq!(s.source, "confirmed_workflow");
    assert_eq!(
        s.suggestion.apply("run CCTV", 8).unwrap().0,
        "cd -- '/synthetic/projects/cctv' && 'python' 'app.py'"
    );
    assert!(!s.warnings.is_empty());
    for input in [
        "run unknown",
        "please run CCTV",
        "Synthetic unconfirmed note",
    ] {
        let context = Context::new(input, input.len())
            .unwrap()
            .with_shell(ShellContext::new(ShellKind::Bash))
            .unwrap();
        assert!(suggest_local(&context, Some(&mem)).unwrap().is_none());
    }
}

#[test]
fn explicit_shell_metadata_is_sent_but_memory_is_not_an_http_context_field() {
    let mut shell = ShellContext::new(ShellKind::PowerShell);
    shell.version = Some("7.4".into());
    shell
        .known_aliases
        .insert("ls".into(), "Get-ChildItem".into());
    let context = Context::new("git st", 6)
        .unwrap()
        .with_shell(shell)
        .unwrap();
    let body = request_body("test", &context);
    let user: serde_json::Value =
        serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(user["shell"]["kind"], "powershell");
    assert_eq!(user["shell"]["version"], "7.4");
    assert_eq!(user["shell"]["known_aliases"]["ls"], "Get-ChildItem");
    assert!(user.get("confirmed").is_none());
    assert!(!body.to_string().contains("/synthetic/projects"));
    let unspecified: serde_json::Value =
        serde_json::to_value(Context::new("git st", 6).unwrap()).unwrap();
    assert!(unspecified.get("shell").is_none());
}

#[test]
fn blank_http_context_with_history_never_connects() {
    let backend = HttpBackend::new(
        "http://127.0.0.1:1/chat/completions",
        "test",
        None,
        Duration::from_secs(1),
    )
    .unwrap();
    for (input, cursor) in [("", 0), ("   ", 1), ("\u{3000}", 3)] {
        let context = Context::new(input, cursor)
            .unwrap()
            .with_history(vec!["git status".into()])
            .unwrap();
        assert_eq!(backend.complete(&context).unwrap(), "");
    }
}

#[test]
fn menu_replacement_is_native_preview_and_rejects_stale_buffer_and_cursor() {
    let calls = Arc::new(AtomicUsize::new(0));
    let engine = engine(calls.clone());
    let mut completer = AiCompleter::new(engine)
        .with_local_context(Some(ShellContext::new(ShellKind::Bash)), Some(memory()));
    let mut menu = completion_menu();
    let mut editor = Editor::default();
    let set = |editor: &mut Editor, text: &str, cursor: usize| {
        editor.edit_buffer(
            |b| {
                b.set_buffer(text.into());
                b.set_insertion_point(cursor);
            },
            UndoBehavior::CreateUndoPoint,
        )
    };
    set(&mut editor, "run CCTV", 8);
    menu.update_values(&mut editor, &mut completer);
    assert_eq!(menu.get_values().len(), 1);
    assert_eq!(editor.get_buffer(), "run CCTV");
    set(&mut editor, "run CCTV", 4);
    menu.replace_in_buffer(&mut editor);
    assert_eq!(editor.get_buffer(), "run CCTV");
    set(&mut editor, "run CCTV", 8);
    menu.update_values(&mut editor, &mut completer);
    set(&mut editor, "run OTHER", 9);
    menu.replace_in_buffer(&mut editor);
    assert_eq!(editor.get_buffer(), "run OTHER");
    set(&mut editor, "run CCTV", 8);
    menu.update_values(&mut editor, &mut completer);
    menu.replace_in_buffer(&mut editor);
    assert_eq!(
        editor.get_buffer(),
        "cd -- '/synthetic/projects/cctv' && 'python' 'app.py'"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn blank_hinter_and_completer_and_known_workflow_do_not_queue_ai() {
    let calls = Arc::new(AtomicUsize::new(0));
    let engine = engine(calls.clone());
    let mut hinter = AiHinter::new(engine.clone())
        .with_local_context(Some(ShellContext::new(ShellKind::Bash)), Some(memory()));
    let mut completer = AiCompleter::new(engine)
        .with_local_context(Some(ShellContext::new(ShellKind::Bash)), Some(memory()));
    let history = LocalHistory::new(None).unwrap();
    for (line, cursor) in [("", 0), ("   ", 1), ("run CCTV", 8)] {
        assert_eq!(hinter.handle(line, cursor, &history, false, ""), "");
        let _ = completer.complete(line, cursor);
    }
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cli_suggestions_and_whitespace_are_offline_and_legacy_completion_still_works() {
    let output = run(&["suggest", "--shell", "cmd", "--buffer", "ls"]);
    assert!(output.status.success(), "{:?}", output.stderr);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["edit"]["start"], 0);
    assert_eq!(value["edit"]["end"], 2);
    assert_eq!(value["result"], "dir");
    assert_eq!(value["executes"], false);
    let output = run(&[
        "suggest",
        "--shell",
        "powershell",
        "--known-alias",
        "ls=Custom-Listing",
        "--buffer",
        "ls",
    ]);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value["edit"].is_null());
    for args in [
        vec![
            "suggest", "--shell", "bash", "--buffer", "   ", "--cursor", "1",
        ],
        vec![
            "--backend",
            "http",
            "--endpoint",
            "http://127.0.0.1:1/chat/completions",
            "--model",
            "test",
            "--complete",
            "   ",
            "--cursor",
            "1",
        ],
    ] {
        let output = run(&args);
        assert!(output.status.success(), "{:?}", output.stderr);
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["result"], "   ");
    }
    assert!(!run(&["suggest", "--buffer", "ls"]).status.success());
    let output = run(&["--complete", "git st"]);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["insertion"], "atus");
}

#[cfg(unix)]
#[test]
fn bash_literal_roundtrips_real_parser_without_expansion() {
    for literal in [
        "a b",
        "single'quote",
        "double\"quote",
        "$(printf INJECTED); & | < >",
        "百分比% ! 世界",
        "",
    ] {
        let quoted = quote_literal(ShellKind::Bash, literal).unwrap();
        let source = format!("printf '%s' {quoted}");
        assert!(
            Command::new("bash")
                .args(["--noprofile", "--norc", "-n", "-c", &source])
                .status()
                .unwrap()
                .success()
        );
        let output = Command::new("bash")
            .args(["--noprofile", "--norc", "-c", &source])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), literal);
    }
}

#[test]
fn memory_cli_create_update_relative_document_and_remove_roundtrip() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!(
            "reedline-memory-cli-{}-{unique}",
            std::process::id()
        ));
    std::fs::create_dir(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let config = dir.join("memory.json");
    let object = dir.join("project.json");
    let project = serde_json::json!({"path":"/synthetic/private-cctv","aliases":["cameras"],"workflows":{"run":{"aliases":["启动监控"],"steps":[{"action":"change_directory"},{"action":"command","program":"python","args":["app.py"]}]}}});
    std::fs::write(&object, serde_json::to_vec(&project).unwrap()).unwrap();
    let file = config.to_str().unwrap();
    let input = object.to_str().unwrap();
    let add = run(&[
        "memory",
        "--config",
        file,
        "add",
        "CCTV",
        "--project-file",
        input,
    ]);
    assert!(add.status.success(), "{:?}", add.stderr);
    let original = std::fs::read(&config).unwrap();
    assert!(
        !run(&[
            "memory",
            "--config",
            file,
            "add",
            "CCTV",
            "--project-file",
            input
        ])
        .status
        .success()
    );
    assert_eq!(std::fs::read(&config).unwrap(), original);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(std::fs::metadata(&config).unwrap().mode() & 0o777, 0o600);
    }
    let mut relative = project;
    relative["path"] = serde_json::json!(".");
    std::fs::write(&object, serde_json::to_vec(&relative).unwrap()).unwrap();
    let update = run(&[
        "memory",
        "--config",
        file,
        "update",
        "CCTV",
        "--project-file",
        input,
    ]);
    assert!(update.status.success(), "{:?}", update.stderr);
    for args in [
        vec!["memory", "--config", file, "list"],
        vec!["memory", "--config", file, "show", "CCTV"],
        vec!["memory", "--config", file, "validate"],
    ] {
        let output = run(&args);
        assert!(output.status.success(), "{:?}", output.stderr);
    }
    assert!(
        !run(&[
            "suggest", "--shell", "bash", "--config", file, "--buffer", "run CCTV"
        ])
        .status
        .success()
    );
    let preview = run(&[
        "suggest",
        "--shell",
        "bash",
        "--project-config",
        file,
        "--project-root",
        dir.to_str().unwrap(),
        "--buffer",
        "启动监控",
    ]);
    assert!(preview.status.success(), "{:?}", preview.stderr);
    let value: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(value["source"], "confirmed_workflow");
    assert!(
        value["result"]
            .as_str()
            .unwrap()
            .contains(dir.to_str().unwrap())
    );
    assert!(
        run(&["memory", "--config", file, "remove", "CCTV"])
            .status
            .success()
    );
    let show = run(&["memory", "--config", file, "show"]);
    let value: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(value["confirmed"], serde_json::json!({}));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn safe_shell_renderings_and_rejections_remain_explicit() {
    use reedline_ai::shell::{compose_steps, render_cd, render_command};
    assert_eq!(
        render_cd(ShellKind::Cmd, "C:\\Synthetic Projects\\世界").unwrap(),
        "cd /d \"C:\\Synthetic Projects\\世界\""
    );
    assert_eq!(
        render_cd(ShellKind::PowerShell, "/synthetic/a[1]").unwrap(),
        "Set-Location -LiteralPath '/synthetic/a[1]' -ErrorAction Stop"
    );
    for smart in ['\u{2018}', '\u{2019}', '\u{201a}', '\u{201b}'] {
        assert_eq!(
            quote_literal(ShellKind::PowerShell, &format!("a{smart}b")).unwrap(),
            format!("'a{smart}{smart}b'")
        );
    }
    for arg in ["", "embedded\"quote"] {
        assert!(render_command(ShellKind::PowerShell, "python", &[arg.into()]).is_err());
    }
    for keyword in ["if", "FOR", "call", "start"] {
        assert!(render_command(ShellKind::Cmd, keyword, &["x".into()]).is_err());
    }
    assert_eq!(
        render_command(ShellKind::Bash, "if", &["x".into()]).unwrap(),
        "'if' 'x'"
    );
    assert!(compose_steps(ShellKind::Nushell, &["^one".into(), "^two".into()], 2).is_err());
}
