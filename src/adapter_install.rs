//! Explicit, self-contained native adapter distribution. Never edits profiles.
use crate::shell::ShellKind;
use clap::{Args, Subcommand};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
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
        /// Acknowledge platform/runtime limits for PowerShell, Nushell and CMD+Clink.
        #[arg(long)]
        experimental: bool,
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
struct Package {
    files: &'static [(&'static str, &'static str)],
    entrypoint: &'static str,
    enable: &'static str,
    disable: &'static str,
    experimental: bool,
}
fn package(shell: ShellKind) -> Package {
    match shell {
        ShellKind::Bash => Package {
            files: &[
                (
                    "reedline-ai.bash",
                    include_str!("../integrations/bash/reedline-ai.bash"),
                ),
                ("bridge.py", include_str!("../integrations/bash/bridge.py")),
            ],
            entrypoint: "reedline-ai.bash",
            enable: "Set REEDLINE_AI_BIN to this installed binary, then source the entrypoint in interactive Bash.",
            disable: "Run reedline_ai_disable in that shell before uninstalling assets.",
            experimental: false,
        },
        ShellKind::PowerShell => Package {
            files: &[
                (
                    "ReedlineAI.psm1",
                    include_str!("../integrations/powershell/ReedlineAI.psm1"),
                ),
                (
                    "AdapterProcess.cs",
                    include_str!("../integrations/powershell/AdapterProcess.cs"),
                ),
            ],
            entrypoint: "ReedlineAI.psm1",
            enable: "Import-Module the entrypoint, then Enable-ReedlineAI -Binary PATH_TO_BINARY. See the PowerShell guide for experimental limits and terminal key fallback.",
            disable: "Run Disable-ReedlineAI before removing assets.",
            experimental: true,
        },
        ShellKind::Nushell => Package {
            files: &[
                (
                    "nushell/reedline-ai.nu",
                    include_str!("../integrations/nushell/reedline-ai.nu"),
                ),
                (
                    "shared/bridge.py",
                    include_str!("../integrations/shared/bridge.py"),
                ),
            ],
            entrypoint: "nushell/reedline-ai.nu",
            enable: "Set REEDLINE_AI_EXECUTABLE to this binary, source the entrypoint, then reedline-ai-enable. Requires Python and experimental Nu job APIs.",
            disable: "Run reedline-ai-disable before removing assets.",
            experimental: true,
        },
        ShellKind::Cmd => Package {
            files: &[
                (
                    "cmd/reedline-ai.lua",
                    include_str!("../integrations/cmd/reedline-ai.lua"),
                ),
                (
                    "shared/bridge.py",
                    include_str!("../integrations/shared/bridge.py"),
                ),
            ],
            entrypoint: "cmd/reedline-ai.lua",
            enable: "Requires separately installed Clink and Python on Windows. Follow the CMD guide for explicit dofile/enable configuration; stock CMD is not supported.",
            disable: "Set REEDLINE_AI_ENABLE=0 in CMD; the next prompt disables owned bindings. Then remove assets.",
            experimental: true,
        },
    }
}
fn directories(files: &[(&'static str, &'static str)]) -> BTreeSet<&'static str> {
    files
        .iter()
        .filter_map(|(name, _)| name.split_once('/').map(|(dir, _)| dir))
        .collect()
}
fn check_entries(directory: &Path, expected: &BTreeSet<&str>) -> Result<(), Error> {
    if !fs::symlink_metadata(directory)?.is_dir() {
        return Err("adapter directory must be an ordinary directory".into());
    }
    let entries = fs::read_dir(directory)?
        .take(expected.len() + 1)
        .collect::<Result<Vec<_>, _>>()?;
    if entries.len() != expected.len()
        || entries.iter().any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_none_or(|name| !expected.contains(name))
        })
    {
        return Err("adapter directory contains extra or missing files; nothing removed".into());
    }
    Ok(())
}
fn supported_clink_version(output: &[u8]) -> bool {
    String::from_utf8_lossy(output)
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .find_map(|part| {
            let numbers = part
                .split('.')
                .take(3)
                .map(str::parse::<u32>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            (numbers.len() == 3).then_some(numbers)
        })
        .is_some_and(|version| version.as_slice() >= [1, 7, 12].as_slice())
}
fn clink_probe() -> std::io::Result<std::process::Output> {
    let mut last = Err(std::io::Error::other("Clink executable unavailable"));
    for name in ["clink", "clink_x64.exe", "clink_x86.exe", "clink.bat"] {
        last = Command::new(name).arg("--version").output();
        if last.as_ref().is_ok_and(|output| output.status.success()) {
            break;
        }
    }
    last
}
fn doctor(shell: ShellKind) -> Result<(), Error> {
    let python_name = if cfg!(windows) && shell == ShellKind::Cmd {
        "python"
    } else {
        "python3"
    };
    let python = || {
        let probe = if shell == ShellKind::Bash {
            "import sys; print(sys.version.split()[0]); sys.exit(0 if sys.version_info >= (3,9) else 1)"
        } else {
            "import sys; print(sys.version.split()[0]); sys.exit(0 if sys.version_info >= (3,10) else 1)"
        };
        Command::new(python_name).args(["-I", "-c", probe]).output()
    };
    let (runtime, dependency, requirements) = match shell {
        ShellKind::Bash => (
            Command::new("bash").args(["--noprofile", "--norc", "-c", "printf '%s' \"$BASH_VERSION\"; (( BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1) ))"]).env_remove("BASH_ENV").env_remove("ENV").output(),
            Some(python()), "Bash >= 5.1 and Python >= 3.9; explicitly source in interactive Bash"),
        ShellKind::PowerShell => (
            Command::new("pwsh").args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "Import-Module PSReadLine -ErrorAction Stop; $PSVersionTable.PSVersion.ToString(); (Get-Module PSReadLine).Version.ToString(); if ($PSVersionTable.PSVersion -lt [version]'7.2' -or (Get-Module PSReadLine).Version -lt [version]'2.2') { exit 1 }"]).output(),
            None, "PowerShell >= 7.2 and PSReadLine >= 2.2; experimental, Windows/macOS untested"),
        ShellKind::Nushell => (
            Command::new("nu").args(["--no-config-file", "-c", "print (version | get version); help job spawn | ignore; help job recv | ignore; help commandline get-cursor | ignore"]).output(),
            Some(python()), "Nushell experimental job/commandline APIs (tested 0.116.1) and Python >= 3.10"),
        ShellKind::Cmd => (
            if cfg!(windows) { clink_probe() } else { Err(std::io::Error::other("Windows required")) },
            Some(python()), "Windows CMD with separately installed Clink >= 1.7.12 (clink/clink_x64.exe/clink_x86.exe/clink.bat on PATH) and Python >= 3.10; native Windows runtime unverified"),
    };
    let ok = runtime.as_ref().is_ok_and(|o| o.status.success())
        && (shell != ShellKind::Cmd
            || runtime
                .as_ref()
                .is_ok_and(|o| supported_clink_version(&o.stdout)))
        && dependency
            .as_ref()
            .is_none_or(|d| d.as_ref().is_ok_and(|o| o.status.success()));
    let version = |output: &std::io::Result<std::process::Output>| {
        output
            .as_ref()
            .ok()
            .map(|o| crate::context::safe_display(String::from_utf8_lossy(&o.stdout).trim()))
            .unwrap_or_else(|| "unavailable".into())
    };
    println!(
        "{}",
        serde_json::json!({"shell":shell,"runtime":version(&runtime),"dependency":dependency.as_ref().map(version),"ready":ok,"experimental":package(shell).experimental,"requirements":requirements,"backend":"offline mock by default","profile_modified":false,"note":"Prerequisite probe only; not a native-terminal or provider validation."})
    );
    if !ok {
        return Err("adapter prerequisites not met; inspect doctor output".into());
    }
    Ok(())
}
pub fn run(args: AdapterArgs) -> Result<(), Error> {
    match args.action {
        Action::Doctor { shell } => doctor(shell)?,
        Action::Install {
            shell,
            directory,
            experimental,
        } => {
            let package = package(shell);
            if package.experimental && !experimental {
                return Err("this adapter is experimental; read its platform guide and pass --experimental to extract it".into());
            }
            let files = package.files;
            if directory.to_str().is_none() {
                return Err("adapter directory must be a UTF-8 path".into());
            }
            // create_dir refuses an existing directory/symlink. No overwrite mode.
            fs::create_dir(&directory)
                .map_err(|_| "adapter directory must be new and its parent must exist")?;
            for child in directories(files) {
                fs::create_dir(directory.join(child))?;
            }
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
                serde_json::json!({"installed":true,"shell":shell,"experimental":package.experimental,"directory":directory,"entrypoint":directory.join(package.entrypoint),"profile_modified":false,"enable":package.enable,"disable":package.disable})
            );
        }
        Action::Uninstall { shell, directory } => {
            let package = package(shell);
            let files = package.files;
            let dirs = directories(files);
            let top = files
                .iter()
                .map(|(name, _)| name.split('/').next().expect("static asset path"))
                .collect();
            check_entries(&directory, &top)?;
            for child in &dirs {
                let expected = files
                    .iter()
                    .filter_map(|(name, _)| {
                        name.split_once('/')
                            .filter(|(dir, _)| dir == child)
                            .map(|(_, file)| file)
                    })
                    .collect();
                check_entries(&directory.join(child), &expected)?;
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
            for child in dirs {
                fs::remove_dir(directory.join(child))?;
            }
            fs::remove_dir(directory)?;
            println!(
                "Bundled adapter assets removed. No profiles, histories or credentials were changed."
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clink_version_probe_rejects_old_and_unparseable_versions() {
        assert!(supported_clink_version(b"Clink v1.9.34.abcdef"));
        assert!(supported_clink_version(b"1.7.12"));
        assert!(!supported_clink_version(b"Clink v1.7.11"));
        assert!(!supported_clink_version(b"unavailable"));
        assert!(!supported_clink_version(b"1.7"));
    }
}
