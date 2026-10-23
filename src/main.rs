use clap::{Parser, ValueEnum};
use reedline::{
    FileBackedHistory, KeyCode, KeyModifiers, Reedline, ReedlineEvent, ReedlineMenu, Signal,
    default_emacs_keybindings,
};
use reedline_ai::{
    backend::{Backend, HttpBackend, MockBackend},
    context::{Context, safe_display},
    editor::{AiCompleter, AiHinter, AiPrompt, SafeEmacs},
    engine::{Engine, EngineConfig, Outcome},
};
use std::{
    io::{self, IsTerminal},
    sync::Arc,
    time::Duration,
};

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum BackendKind {
    Mock,
    Http,
}

#[derive(Parser)]
#[command(
    version,
    about = "AI suffix/FIM completion playground. Never executes commands.",
    after_help = "Default: offline deterministic mock. HTTP mode automatically sends the current line's prefix and suffix to the explicitly selected endpoint after a pause. Do not type secrets. No history, cwd, or files are sent."
)]
struct Args {
    #[arg(long, value_enum, default_value = "mock")]
    backend: BackendKind,
    /// Full chat-completions URL. HTTPS, or loopback HTTP only.
    #[arg(long, required_if_eq("backend", "http"))]
    endpoint: Option<String>,
    #[arg(long, required_if_eq("backend", "http"))]
    model: Option<String>,
    /// Read a bearer key from this environment variable; never pass the key as an argument.
    #[arg(long, default_value = "REEDLINE_AI_API_KEY")]
    api_key_env: String,
    #[arg(long, default_value_t = 180, value_parser = clap::value_parser!(u64).range(0..=10_000))]
    debounce_ms: u64,
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u64).range(1..=60))]
    timeout_seconds: u64,
    /// One-shot completion as JSON, without opening an interactive terminal.
    #[arg(long)]
    complete: Option<String>,
    /// UTF-8 byte offset for --complete (default: end of input).
    #[arg(long, requires = "complete")]
    cursor: Option<usize>,
    #[arg(long)]
    no_color: bool,
}

fn main() -> std::process::ExitCode {
    match run(Args::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("reedline-ai: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let backend: Arc<dyn Backend> = match args.backend {
        BackendKind::Mock => {
            if args.endpoint.is_some() || args.model.is_some() {
                return Err("--endpoint and --model require --backend http".into());
            }
            Arc::new(MockBackend)
        }
        BackendKind::Http => {
            let key = std::env::var(&args.api_key_env).ok();
            Arc::new(HttpBackend::new(
                args.endpoint.as_deref().expect("clap endpoint"),
                args.model.as_deref().expect("clap model"),
                key.as_deref(),
                Duration::from_secs(args.timeout_seconds),
            )?)
        }
    };
    if let Some(buffer) = &args.complete {
        let cursor = args.cursor.unwrap_or(buffer.len());
        let context = Context::new(buffer, cursor)?;
        let insertion = if context.is_empty() {
            String::new()
        } else {
            backend.complete(&context)?
        };
        let result = context.insert(&insertion)?;
        println!(
            "{}",
            serde_json::json!({"insertion": insertion, "result": result, "cursor": cursor})
        );
        return Ok(());
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("interactive mode needs a terminal; use --complete for headless mode".into());
    }
    let network = args.backend == BackendKind::Http;
    if network {
        println!(
            "HTTP mode: the current buffer prefix/suffix is sent after a pause. Never type secrets."
        );
    } else {
        println!(
            "Offline mock mode. Try: git st  or  echo \"hello\" (move before the closing quote)."
        );
    }
    println!(
        "Ctrl-F accepts an end hint. Tab opens FIM; Enter accepts it. Escape closes the menu."
    );
    println!(
        "Enter only prints the line, never executes it. Ctrl-C clears; Ctrl-D on empty or :quit exits.\n"
    );
    let engine = Engine::new(
        backend,
        EngineConfig {
            debounce: Duration::from_millis(args.debounce_ms),
            ..EngineConfig::default()
        },
    );
    let mut bindings = default_emacs_keybindings();
    bindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("ai_menu".into()),
            ReedlineEvent::MenuNext,
        ]),
    );
    let menu = reedline_ai::editor::completion_menu();
    let mut editor = Reedline::create()
        .with_completer(Box::new(AiCompleter::new(engine.clone())))
        .with_hinter(Box::new(AiHinter::new(engine.clone())))
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(menu)))
        .with_history(Box::new(FileBackedHistory::new(0)?))
        .with_edit_mode(Box::new(SafeEmacs::new(bindings, engine.clone())))
        .with_quick_completions(false)
        .with_partial_completions(false)
        .with_poll_interval(Duration::from_millis(33))
        .with_ansi_colors(!args.no_color);
    let repaint = editor.repaint_signal();
    engine.set_repaint(move || repaint.request_repaint());
    let prompt = AiPrompt {
        engine: engine.clone(),
        network,
    };
    loop {
        match editor.read_line(&prompt)? {
            Signal::Success(line) => {
                let outcome = engine.outcome();
                engine.clear();
                if line.trim() == ":quit" {
                    break;
                }
                if let Outcome::Error(error) = outcome {
                    eprintln!("Completion unavailable: {error}");
                }
                if !line.is_empty() {
                    println!("Accepted (not executed): {}", safe_display(&line));
                }
            }
            Signal::CtrlC => engine.clear(),
            Signal::CtrlD => {
                engine.clear();
                break;
            }
            _ => {}
        }
    }
    Ok(())
}
