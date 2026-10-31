//! Offline CLI surface for local previews, memory, and explicit adapter distribution.
use crate::{
    backend::{Backend, MockBackend},
    context::Context,
    project_memory::{Project, ProjectMemory},
    risk::{RiskPolicy, review},
    shell::{ShellContext, ShellKind},
    structured_edit::{StructuredSuggestion, TextEdit},
    workflow::suggest_local,
};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::json;
use std::{collections::BTreeMap, io::Read, path::PathBuf};

type Error = Box<dyn std::error::Error>;

#[derive(Clone, Default, Args)]
pub struct ShellArgs {
    /// Target dialect. Never inferred from the host OS or parent process.
    #[arg(long, value_enum)]
    pub shell: Option<ShellKind>,
    #[arg(long, requires = "shell")]
    pub shell_version: Option<String>,
    /// Explicit command name available in the target shell (repeatable).
    #[arg(long, requires = "shell")]
    pub known_command: Vec<String>,
    /// Explicit alias NAME=TARGET (repeatable); never probes the target shell.
    #[arg(long, requires = "shell")]
    pub known_alias: Vec<String>,
}
impl ShellArgs {
    pub fn context(&self) -> Result<Option<ShellContext>, Error> {
        let Some(kind) = self.shell else {
            return Ok(None);
        };
        let mut aliases = BTreeMap::new();
        for alias in &self.known_alias {
            let (name, target) = alias
                .split_once('=')
                .ok_or("--known-alias requires NAME=TARGET")?;
            if aliases
                .insert(name.to_string(), target.to_string())
                .is_some()
            {
                return Err("duplicate shell alias".into());
            }
        }
        let shell = ShellContext {
            kind,
            version: self.shell_version.clone(),
            known_commands: self.known_command.clone(),
            known_aliases: aliases,
        };
        shell.validate()?;
        Ok(Some(shell))
    }
}

#[derive(Clone, Default, Args)]
pub struct RiskArgs {
    /// Local advisory review: warn (default), confirm (F2 then Enter), or off.
    /// Only explicit CLI settings control this; project memory cannot override it.
    #[arg(long, value_enum, default_value = "warn")]
    pub risk_policy: RiskPolicy,
}

#[derive(Clone, Default, Args)]
pub struct ConfigArgs {
    /// Explicit highest-precedence local JSON memory file. Never auto-discovered/uploaded.
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub global_config: Option<PathBuf>,
    /// Explicitly trusted project file, typically ROOT/.shell-ai/config.json.
    #[arg(long, requires = "project_root")]
    pub project_config: Option<PathBuf>,
    /// Existing root anchoring relative project paths; no Git/cwd discovery.
    #[arg(long, requires = "project_config")]
    pub project_root: Option<PathBuf>,
    #[arg(long)]
    pub local_config: Option<PathBuf>,
}
impl ConfigArgs {
    pub fn load(&self) -> Result<Option<ProjectMemory>, Error> {
        if self.config.is_none()
            && self.global_config.is_none()
            && self.project_config.is_none()
            && self.local_config.is_none()
        {
            return Ok(None);
        }
        let project = self
            .project_config
            .as_deref()
            .zip(self.project_root.as_deref());
        Ok(Some(ProjectMemory::load_layers(
            self.global_config.as_deref(),
            project,
            self.local_config.as_deref(),
            self.config.as_deref(),
        )?))
    }
}

#[derive(Subcommand)]
pub enum Commands {
    /// Configure Base URL, model and an optional hidden API key. No network calls.
    Setup(crate::settings::SetupArgs),
    /// Print session-only initialization code for Bash or PowerShell.
    Init(crate::adapter_install::InitArgs),
    /// Install, check or remove explicitly selected native-shell adapter assets.
    Adapter(crate::adapter_install::AdapterArgs),
    /// Offline structured-edit preview. No process execution or HTTP calls.
    Suggest(SuggestArgs),
    /// Inspect or explicitly edit local confirmed memory (JSON only).
    Memory(MemoryArgs),
}
#[derive(Args)]
pub struct SuggestArgs {
    #[command(flatten)]
    pub risk: RiskArgs,
    #[arg(long)]
    pub buffer: String,
    /// UTF-8 byte cursor; defaults to end of buffer.
    #[arg(long)]
    pub cursor: Option<usize>,
    #[command(flatten)]
    pub shell: ShellArgs,
    #[command(flatten)]
    pub config: ConfigArgs,
    /// JSON is the identity-checked protocol; line output is preview text only.
    #[arg(long, value_enum, default_value = "json")]
    pub format: SuggestFormat,
}
#[derive(Clone, Copy, ValueEnum)]
pub enum SuggestFormat {
    Json,
    Line,
}
#[derive(Args)]
pub struct MemoryArgs {
    /// Explicit target required even for reads; writes never pick a default path.
    #[arg(long)]
    pub config: PathBuf,
    #[command(subcommand)]
    pub command: MemoryCommand,
}
#[derive(Subcommand)]
pub enum MemoryCommand {
    List,
    Show {
        name: Option<String>,
    },
    Validate,
    /// Add a confirmed project from a JSON object; creates missing config.
    Add {
        name: String,
        #[arg(long)]
        project_file: PathBuf,
    },
    /// Replace the complete named project with an explicitly selected JSON object.
    Update {
        name: String,
        #[arg(long)]
        project_file: PathBuf,
    },
    Remove {
        name: String,
    },
}

pub fn run(command: Commands) -> Result<(), Error> {
    match command {
        Commands::Setup(args) => crate::settings::run(args)?,
        Commands::Init(args) => crate::adapter_install::init(args)?,
        Commands::Adapter(args) => crate::adapter_install::run(args)?,
        Commands::Suggest(args) => {
            let shell = args
                .shell
                .context()?
                .ok_or("suggest requires --shell; a dialect is never inferred")?;
            let cursor = args.cursor.unwrap_or(args.buffer.len());
            let context = Context::new(&args.buffer, cursor)?.with_shell(shell.clone())?;
            let memory = args.config.load()?;
            let local = suggest_local(&context, memory.as_ref())?;
            let (suggestion, source, explanation, warnings) = if let Some(local) = local {
                (
                    Some(local.suggestion),
                    local.source,
                    local.explanation,
                    local.warnings,
                )
            } else {
                let insertion = MockBackend.complete(&context)?;
                let suggestion = if insertion.is_empty() {
                    None
                } else {
                    Some(StructuredSuggestion::new(
                        &args.buffer,
                        cursor,
                        TextEdit {
                            start: cursor,
                            end: cursor,
                            replacement: insertion,
                        },
                    )?)
                };
                let source = if suggestion.is_some() {
                    "offline_mock_insertion"
                } else {
                    "none"
                };
                (
                    suggestion,
                    source,
                    "No model or shell process was called.",
                    Vec::new(),
                )
            };
            let (result, result_cursor) = match &suggestion {
                Some(s) => s.apply(&args.buffer, cursor)?,
                None => (args.buffer.clone(), cursor),
            };
            let risk = review(Some(&shell), &result);
            let confirmation_required =
                suggestion.is_some() && risk.requires_confirmation(args.risk.risk_policy);
            match args.format {
                SuggestFormat::Json => println!(
                    "{}",
                    json!({"protocol_version":1,"preview_only":true,"executes":false,"network_request":false,"shell":shell,"source_buffer":args.buffer,"source_cursor":cursor,"edit":suggestion.as_ref().map(|s| &s.edit),"result":result,"cursor":result_cursor,"source":source,"explanation":explanation,"warnings":warnings,"risk_policy":args.risk.risk_policy,"risk_review":risk,"additional_acceptance_required":confirmation_required})
                ),
                // Never source/eval this output. It is ordinary single-line text.
                SuggestFormat::Line => {
                    if args.risk.risk_policy != RiskPolicy::Off {
                        for warning in &risk.warnings {
                            eprintln!("Risk review: {warning}");
                        }
                        if risk.incomplete {
                            eprintln!("Risk review incomplete; not a safety guarantee.");
                        }
                        if confirmation_required {
                            eprintln!("Additional acceptance required by policy; preview only.");
                        }
                    }
                    println!("{result}\n{result_cursor}");
                }
            }
        }
        Commands::Memory(args) => {
            let mut memory = match ProjectMemory::load_document(&args.config) {
                Ok(memory) => memory,
                Err(e)
                    if e.kind() == std::io::ErrorKind::NotFound
                        && matches!(args.command, MemoryCommand::Add { .. }) =>
                {
                    ProjectMemory::default()
                }
                Err(e) => return Err(e.into()),
            };
            match args.command {
                MemoryCommand::List => println!(
                    "{}",
                    json!({"version":memory.version,"confirmed":memory.confirmed.keys().collect::<Vec<_>>(),"inferred_suggestion_count":memory.suggestions.len()})
                ),
                MemoryCommand::Show { name } => {
                    if let Some(name) = name {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(
                                memory
                                    .confirmed
                                    .get(&name)
                                    .ok_or("confirmed project not found")?
                            )?
                        );
                    } else {
                        println!("{}", serde_json::to_string_pretty(&memory)?);
                    }
                }
                MemoryCommand::Validate => {
                    memory.validate()?;
                    println!(
                        "{}",
                        json!({"valid":true,"version":memory.version,"network_request":false})
                    );
                }
                MemoryCommand::Add { name, project_file } => {
                    memory.add_project(name, read_project(&project_file)?)?;
                    memory.write_atomic(&args.config)?;
                    println!(
                        "{}",
                        json!({"updated":true,"operation":"add","network_request":false})
                    );
                }
                MemoryCommand::Update { name, project_file } => {
                    memory.update_project(&name, read_project(&project_file)?)?;
                    memory.write_atomic(&args.config)?;
                    println!(
                        "{}",
                        json!({"updated":true,"operation":"update","network_request":false})
                    );
                }
                MemoryCommand::Remove { name } => {
                    memory.remove_project(&name)?;
                    memory.write_atomic(&args.config)?;
                    println!(
                        "{}",
                        json!({"updated":true,"operation":"remove","network_request":false})
                    );
                }
            }
        }
    }
    Ok(())
}
fn read_project(path: &std::path::Path) -> Result<Project, Error> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "could not inspect project input")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("project input must be a regular, non-symlink file".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "could not read project input")?
        .take(262_145)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read project input")?;
    if bytes.len() > 262_144 {
        return Err("project input exceeds 256 KiB".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "project input is not a valid project JSON object".into())
}
