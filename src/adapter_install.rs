//! Explicit, self-contained native adapter distribution. Never edits profiles.
use crate::shell::ShellKind;
use clap::{Args, Subcommand};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::Command,
};
type Error = Box<dyn std::error::Error>;

#[derive(Args)]
pub struct AdapterArgs {
    #[command(subcommand)]
    action: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Extract bundled adapter assets to a NEW directory. Never edits a profile.
    Install {
        #[arg(long, value_enum)]
        shell: ShellKind,
        #[arg(long)]
        directory: PathBuf,
    },
    /// Check local prerequisites without API calls or reading shell profiles.
    Doctor {
        #[arg(long, value_enum)]
        shell: ShellKind,
    },
    /// Remove only unchanged bundled assets. Refuses modified or extra files.
    Uninstall {
        #[arg(long, value_enum)]
        shell: ShellKind,
        #[arg(long)]
        directory: PathBuf,
    },
}
fn assets(shell: ShellKind) -> Result<&'static [(&'static str, &'static str)], Error> {
    match shell {
        ShellKind::Bash => Ok(&[
            ("reedline-ai.bash", include_str!("../integrations/bash/reedline-ai.bash")),
            ("bridge.py", include_str!("../integrations/bash/bridge.py")),
        ]),
        _ => Err("this install milestone bundles Bash only; other adapters remain experimental source integrations".into()),
    }
}
pub fn run(args: AdapterArgs) -> Result<(), Error> {
    match args.action {
        Action::Doctor { shell } => {
            assets(shell)?;
            let bash = Command::new("bash").args(["--noprofile", "--norc", "-c", "printf '%s' \"$BASH_VERSION\"; (( BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1) ))"])
                .env_remove("BASH_ENV").env_remove("ENV").output();
            let python = Command::new("python3").args(["-I", "-c", "import sys; print(sys.version.split()[0]); sys.exit(0 if sys.version_info >= (3,9) else 1)"]).output();
            let ok = bash.as_ref().is_ok_and(|o| o.status.success())
                && python.as_ref().is_ok_and(|o| o.status.success());
            let version = |output: &std::io::Result<std::process::Output>| {
                output
                    .as_ref()
                    .ok()
                    .map(|o| {
                        crate::context::safe_display(String::from_utf8_lossy(&o.stdout).trim())
                    })
                    .unwrap_or_else(|| "unavailable".into())
            };
            println!(
                "{}",
                serde_json::json!({"shell":"bash","bash":version(&bash),"python3":version(&python),"ready":ok,"requirements":"Bash >= 5.1, Python >= 3.9; source explicitly in interactive Bash","backend":"offline mock by default","profile_modified":false})
            );
            if !ok {
                return Err("Bash adapter prerequisites not met; inspect doctor output".into());
            }
        }
        Action::Install { shell, directory } => {
            let files = assets(shell)?;
            // create_dir refuses an existing directory/symlink. No overwrite mode.
            fs::create_dir(&directory)
                .map_err(|_| "adapter directory must be new and its parent must exist")?;
            for (name, contents) in files {
                let path = directory.join(name);
                let mut options = fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(path)?;
                file.write_all(contents.as_bytes())?;
                file.sync_all()?;
            }
            println!(
                "{}",
                serde_json::json!({"installed":true,"shell":"bash","directory":directory,"entrypoint":directory.join("reedline-ai.bash"),"profile_modified":false,"enable":"Set REEDLINE_AI_BIN to this installed binary, then source the entrypoint in interactive Bash.","disable":"Run reedline_ai_disable in that shell before uninstalling assets."})
            );
        }
        Action::Uninstall { shell, directory } => {
            let files = assets(shell)?;
            if !fs::symlink_metadata(&directory)?.is_dir() {
                return Err("adapter directory must be an ordinary directory".into());
            }
            let entries = fs::read_dir(&directory)?
                .take(files.len() + 1)
                .collect::<Result<Vec<_>, _>>()?;
            if entries.len() != files.len() {
                return Err(
                    "adapter directory contains extra or missing files; nothing removed".into(),
                );
            }
            for (name, contents) in files {
                let path = directory.join(name);
                let metadata = fs::symlink_metadata(&path)?;
                if !metadata.is_file() || metadata.len() != contents.len() as u64 {
                    return Err(
                        "adapter assets were modified or have a different version; nothing removed"
                            .into(),
                    );
                }
                let mut existing = Vec::new();
                fs::File::open(&path)?
                    .take(contents.len() as u64 + 1)
                    .read_to_end(&mut existing)?;
                if existing != contents.as_bytes() {
                    return Err(
                        "adapter assets were modified or have a different version; nothing removed"
                            .into(),
                    );
                }
            }
            for (name, _) in files {
                fs::remove_file(directory.join(name))?;
            }
            fs::remove_dir(directory)?;
            println!(
                "Bundled adapter assets removed. No profiles, histories or credentials were changed."
            );
        }
    }
    Ok(())
}
