use clap::{Parser, ValueEnum};
use reedline::{
    History, KeyCode, KeyModifiers, MenuBuilder, Reedline, ReedlineEvent, ReedlineMenu, Signal,
    default_emacs_keybindings,
};
use reedline_ai::{
    backend::{Backend, HttpBackend, HttpNetworkOptions, HttpProvider, MockBackend},
    commands::{Commands, ConfigArgs, ShellArgs},
    context::{Context, safe_display},
    editor::{AiCompleter, AiHinter, AiPrompt, SafeEmacs},
    engine::{Engine, EngineConfig, Outcome},
    history_context::{HistoryContextConfig, RequestBudget, select_history},
    local_history::LocalHistory,
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

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum ProviderKind {
    Generic,
    Deepseek,
}

#[derive(Parser)]
#[command(
    version,
    args_conflicts_with_subcommands = true,
    about = "AI suffix/FIM completion playground. Never executes commands.",
    after_help = "Default: offline deterministic mock. HTTP mode automatically sends the current line's prefix and suffix to the explicitly selected endpoint after a pause. Do not type secrets. History stays local unless --ai-history is explicitly enabled. No cwd or files are sent."
)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
    #[command(flatten)]
    shell: ShellArgs,
    #[command(flatten)]
    config: ConfigArgs,
    #[arg(long, value_enum, default_value = "mock")]
    backend: BackendKind,
    /// Full chat-completions URL. HTTPS, or loopback HTTP only.
    #[arg(long, required_if_eq("backend", "http"))]
    endpoint: Option<String>,
    #[arg(long, required_if_eq("backend", "http"))]
    model: Option<String>,
    /// Explicit provider extension. Deepseek disables reasoning; generic adds no extra fields.
    #[arg(long, value_enum, default_value = "generic")]
    provider: ProviderKind,
    /// Explicitly allow HTTP(S)/ALL_PROXY and NO_PROXY environment configuration.
    #[arg(long)]
    allow_env_proxy: bool,
    /// Explicitly trust platform certificates (including SSL_CERT_FILE/SSL_CERT_DIR).
    #[arg(long)]
    trust_native_certs: bool,
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
    /// Include numeric provider token usage in one-shot HTTP JSON output.
    #[arg(long, requires = "complete")]
    report_usage: bool,
    /// UTF-8 byte offset for --complete (default: end of input).
    #[arg(long, requires = "complete")]
    cursor: Option<usize>,
    /// Persist ONLY this application's history to an explicitly selected file.
    #[arg(long)]
    history_file: Option<std::path::PathBuf>,
    /// Explicitly send selected local history to the configured model (OFF by default).
    #[arg(long)]
    ai_history: bool,
    /// Exclude history containing this case-sensitive substring from AI context (repeatable).
    #[arg(long)]
    history_exclude: Vec<String>,
    /// Offline JSON preview of history selected for this input; never calls a model.
    #[arg(long, conflicts_with_all = ["complete", "report_usage", "ai_history"])]
    history_preview: Option<String>,
    /// Conservative request allowance, NOT measured tokens; includes output/protocol reserves.
    #[arg(long, default_value_t = 8192, value_parser = clap::value_parser!(u32).range(8192..=16384))]
    request_budget: u32,
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
    if let Some(command) = args.command {
        return reedline_ai::commands::run(command);
    }
    let shell = args.shell.context()?;
    let project_memory = args.config.load()?;
    if project_memory.is_some() && shell.is_none() {
        return Err("project workflows require an explicit --shell".into());
    }
    if ![8192, 16384].contains(&args.request_budget) {
        return Err("--request-budget must be 8192 or 16384 (conservative estimate units, not actual tokens)".into());
    }
    let mut history = if args.complete.is_some() || args.history_preview.is_some() {
        LocalHistory::read_only(args.history_file.clone())?
    } else {
        LocalHistory::new(args.history_file.clone())?
    };
    let history_config = HistoryContextConfig {
        enabled: args.ai_history,
        budget: RequestBudget::new(args.request_budget as usize, 4096)?,
        exclusions: args.history_exclude.clone(),
    };
    if let Some(input) = &args.history_preview {
        print_history_preview(input, &history, &history_config)?;
        return Ok(());
    }
    let context_for =
        |buffer: &str, cursor: usize| -> Result<Context, Box<dyn std::error::Error>> {
            let mut context = Context::new(buffer, cursor)?;
            if let Some(shell) = &shell {
                context = context.with_shell(shell.clone())?;
            }
            Ok(
                select_history(&context, &history.recent(500), &history_config)?
                    .into_context(context)?,
            )
        };
    let backend: Arc<dyn Backend> = match args.backend {
        BackendKind::Mock => {
            if args.endpoint.is_some()
                || args.model.is_some()
                || args.provider != ProviderKind::Generic
                || args.report_usage
                || args.allow_env_proxy
                || args.trust_native_certs
            {
                return Err("HTTP configuration options require --backend http".into());
            }
            Arc::new(MockBackend)
        }
        BackendKind::Http => {
            let key = std::env::var(&args.api_key_env).ok();
            let http = HttpBackend::new_with_network(
                args.endpoint.as_deref().expect("clap endpoint"),
                args.model.as_deref().expect("clap model"),
                key.as_deref(),
                Duration::from_secs(args.timeout_seconds),
                HttpNetworkOptions {
                    allow_env_proxy: args.allow_env_proxy,
                    trust_native_certs: args.trust_native_certs,
                },
            )?
            .with_request_budget(history_config.budget)?
            .with_provider(match args.provider {
                ProviderKind::Generic => HttpProvider::Generic,
                ProviderKind::Deepseek => HttpProvider::DeepSeek,
            });
            if args.report_usage {
                let buffer = args.complete.as_deref().expect("clap complete");
                let cursor = args.cursor.unwrap_or(buffer.len());
                let context = context_for(buffer, cursor)?;
                let (insertion, usage) = if context.is_empty() {
                    (String::new(), None)
                } else {
                    let completion = http.complete_with_usage(&context)?;
                    (completion.insertion, completion.usage)
                };
                let result = context.insert(&insertion)?;
                println!(
                    "{}",
                    serde_json::json!({"insertion": insertion, "result": result, "cursor": cursor, "usage": usage})
                );
                return Ok(());
            }
            Arc::new(http)
        }
    };
    if let Some(buffer) = &args.complete {
        let cursor = args.cursor.unwrap_or(buffer.len());
        let context = context_for(buffer, cursor)?;
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
        "Native local history: Up/Down, Ctrl-R search, Alt-F next hint word, Ctrl-X history menu."
    );
    println!(
        "History upload: {}. :history-preview PREFIX previews; :history-clear clears local history.",
        if args.ai_history {
            "ON (selected history may be sent)"
        } else {
            "OFF"
        }
    );
    if shell.is_some() {
        println!(
            "Explicit shell context enabled. Tab previews configured workflows or shell-intent replacements; Enter accepts only."
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
    bindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('x'),
        ReedlineEvent::Menu("history_menu".into()),
    );
    let menu = reedline_ai::editor::completion_menu();
    let mut editor = Reedline::create()
        .with_completer(Box::new(
            AiCompleter::with_history(engine.clone(), history.clone(), history_config.clone())
                .with_local_context(shell.clone(), project_memory.clone()),
        ))
        .with_hinter(Box::new(
            AiHinter::with_history_config(engine.clone(), history_config.clone())
                .with_local_context(shell.clone(), project_memory.clone()),
        ))
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(menu)))
        .with_menu(ReedlineMenu::HistoryMenu(Box::new(
            reedline::ListMenu::default().with_name("history_menu"),
        )))
        .with_history(Box::new(history.clone()))
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
                if line.trim() == ":history-clear" {
                    history.clear()?;
                    history.sync()?;
                    println!(
                        "Local history and AI cache cleared. Already-sent requests cannot be recalled."
                    );
                    continue;
                }
                if let Some(input) = line.strip_prefix(":history-preview") {
                    if let Err(error) = print_history_preview(
                        input.strip_prefix(' ').unwrap_or(input),
                        &history,
                        &history_config,
                    ) {
                        eprintln!("History preview unavailable: {error}");
                    }
                    continue;
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

fn print_history_preview(
    input: &str,
    history: &LocalHistory,
    config: &HistoryContextConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = Context::new(input, input.len())?;
    let preview_config = HistoryContextConfig {
        enabled: true,
        ..config.clone()
    };
    let preview = select_history(&context, &history.recent(500), &preview_config)?;
    println!(
        "{}",
        serde_json::json!({"network_request": false, "history_upload_enabled": config.enabled, "preview": preview})
    );
    Ok(())
}
