//! Explicit user setup; one OpenAI-compatible protocol, no implicit network use.
use crate::backend::validate_endpoint;
use clap::Args;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
};
type Error = Box<dyn std::error::Error>;
const MAX_SETTINGS: u64 = 16 * 1024;

#[derive(Args)]
pub struct SetupArgs {
    /// Non-secret settings directory; defaults to the current user's app config directory.
    #[arg(long)]
    pub directory: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u8,
    pub endpoint: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

pub fn directory() -> Result<PathBuf, Error> {
    if let Some(path) = std::env::var_os("REEDLINE_AI_SETTINGS_DIR") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("REEDLINE_AI_SETTINGS_DIR must be an absolute path".into());
        }
        return Ok(path);
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }
    .ok_or("cannot locate app settings directory; set REEDLINE_AI_SETTINGS_DIR")?;
    if !base.is_absolute() {
        return Err("app settings base directory must be absolute".into());
    }
    Ok(base.join("reedline-ai"))
}

pub fn endpoint_from_base(base: &str) -> Result<String, Error> {
    let mut url = validate_endpoint(base.trim())?;
    let path = url.path().trim_end_matches('/');
    if path.ends_with("/completions") && !path.ends_with("/chat/completions") {
        return Err("use an OpenAI-compatible Base URL (for example https://api.openai.com/v1), not legacy /completions".into());
    }
    let path = if path.ends_with("/chat/completions") {
        path.to_string()
    } else {
        format!("{path}/chat/completions")
    };
    url.set_path(&path);
    Ok(url.to_string())
}

impl Settings {
    pub fn validate(&self) -> Result<(), Error> {
        if self.version != 1 {
            return Err("unsupported settings version; run reedline-ai setup again".into());
        }
        validate_endpoint(&self.endpoint)?;
        if !self
            .endpoint
            .trim_end_matches('/')
            .ends_with("/chat/completions")
        {
            return Err(
                "settings endpoint must use /chat/completions; run reedline-ai setup".into(),
            );
        }
        if self.model.trim().is_empty()
            || self.model.len() > 128
            || self.model.chars().any(crate::context::unsafe_char)
        {
            return Err("model must be 1-128 bytes without control characters".into());
        }
        if self
            .api_key
            .as_ref()
            .is_some_and(|k| k.len() > 4096 || k.chars().any(|c| c.is_control()) || !k.is_ascii())
        {
            return Err(
                "API key must be at most 4096 ASCII bytes without control characters".into(),
            );
        }
        Ok(())
    }
}

pub(crate) fn private_directory(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o077 != 0 {
                    return Err("settings directory is accessible by other users; choose a private directory (permissions 0700)".into());
                }
            }
            Ok(())
        }
        Ok(_) => Err("settings directory must be an ordinary directory, not a symlink".into()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or("settings directory needs a parent")?;
            fs::create_dir_all(parent)?;
            let mut builder = fs::DirBuilder::new();
            builder.recursive(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(path)?;
            Ok(())
        }
        Err(_) => Err("cannot access settings directory".into()),
    }
}

pub fn load(path: &Path) -> Result<Settings, Error> {
    let meta = fs::symlink_metadata(path)
        .map_err(|_| "settings unavailable; run reedline-ai setup first")?;
    if !meta.is_file() || meta.len() > MAX_SETTINGS {
        return Err("settings must be a regular file no larger than 16 KiB".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(
                "settings permissions must be private (0600); refusing to load credentials".into(),
            );
        }
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_SETTINGS + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SETTINGS as usize {
        return Err("settings exceed 16 KiB".into());
    }
    let settings: Settings = serde_json::from_slice(&bytes)
        .map_err(|_| "invalid settings JSON; run reedline-ai setup again")?;
    settings.validate()?;
    Ok(settings)
}

pub fn save(path: &Path, settings: &Settings) -> Result<(), Error> {
    settings.validate()?;
    let encoded = serde_json::to_vec_pretty(settings)?;
    if encoded.len() + 1 > MAX_SETTINGS as usize {
        return Err("settings exceed 16 KiB; shorten the Base URL or API key".into());
    }
    let parent = path.parent().ok_or("settings path needs a parent")?;
    private_directory(parent)?;
    if let Ok(meta) = fs::symlink_metadata(path)
        && !meta.is_file()
    {
        return Err("refusing to replace a non-regular settings file".into());
    }
    // Fixed app-private temporary path; create_new refuses stale files and symlinks.
    let tmp = parent.join("settings.json.new");
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(|_| "cannot create private settings file; check settings.json.new and directory permissions")?;
    let result = (|| -> Result<(), Error> {
        file.write_all(&encoded)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

fn ask(prompt: &str) -> Result<String, Error> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().take(8193).read_line(&mut line)?;
    if line.len() > 8192 {
        return Err("input too long".into());
    }
    Ok(line.trim().to_string())
}

/// Read without echo. Escape/Ctrl-C cancels and Drop restores terminal mode.
fn secret() -> Result<String, Error> {
    terminal::enable_raw_mode()?;
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = terminal::disable_raw_mode();
        }
    }
    let restore = Restore;
    print!("API key (hidden; Enter to use environment/no authentication): ");
    io::stdout().flush()?;
    let mut value = String::new();
    loop {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match key.code {
                KeyCode::Enter => break,
                KeyCode::Esc => return Err("setup cancelled; settings unchanged".into()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err("setup cancelled; settings unchanged".into());
                }
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL) && !c.is_control() =>
                {
                    if value.len() + c.len_utf8() > 4096 {
                        return Err("API key too long".into());
                    }
                    value.push(c);
                }
                _ => {}
            }
        }
    }
    drop(restore);
    println!();
    Ok(value)
}

pub fn run(args: SetupArgs) -> Result<(), Error> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("setup requires an interactive terminal; API keys are never accepted as command arguments".into());
    }
    let dir = match args.directory {
        Some(dir) => dir,
        None => directory()?,
    };
    if !dir.is_absolute() {
        return Err("settings directory must be absolute".into());
    }
    let path = dir.join("settings.json");
    println!("OpenAI-compatible chat completions. Setup makes no network request.");
    if path.exists() {
        println!(
            "This replaces previous app settings only after confirmation. Blank API key removes a previously saved key."
        );
    }
    let base = ask("Base URL [https://api.openai.com/v1]: ")?;
    let endpoint = endpoint_from_base(if base.is_empty() {
        "https://api.openai.com/v1"
    } else {
        &base
    })?;
    let model = ask("Model name (required, as listed by your provider): ")?;
    let key = secret()?;
    let settings = Settings {
        version: 1,
        endpoint,
        model,
        api_key: (!key.is_empty()).then_some(key),
    };
    settings.validate()?;
    println!(
        "Settings location: {}",
        crate::context::safe_display(&path.to_string_lossy())
    );
    if settings.api_key.is_some() {
        println!(
            "The API key will be saved UNENCRYPTED in this local app settings file. Keep it out of backups, sharing, and source control."
        );
        #[cfg(unix)]
        println!(
            "New app directory/files use permissions 0700/0600; this does not protect against your user or administrators."
        );
        #[cfg(windows)]
        println!(
            "Windows uses inherited directory ACLs; this program has NOT verified or tightened them. Use a private user directory or cancel and use REEDLINE_AI_API_KEY instead."
        );
    }
    println!(
        "Only --configured or the init command opts into sending requested command text to this endpoint. History stays local unless separately enabled."
    );
    if !ask("Save these settings locally? [y/N]: ")?.eq_ignore_ascii_case("y") {
        println!("Cancelled; settings unchanged.");
        return Ok(());
    }
    save(&path, &settings)?;
    println!("Saved. No API request was made. REEDLINE_AI_API_KEY overrides a saved key.");
    if dir != directory()? {
        println!(
            "For this custom directory, set REEDLINE_AI_SETTINGS_DIR before init or --configured."
        );
    }
    let binary = std::env::current_exe()?;
    let binary = binary.to_str().ok_or("binary path must be UTF-8")?;
    let bash_binary = format!("'{}'", binary.replace('\'', "'\"'\"'"));
    let ps_binary = format!("'{}'", binary.replace('\'', "''"));
    println!("Bash: eval \"$({bash_binary} init --shell bash)\"");
    println!(
        "PowerShell 7: Invoke-Expression (& {ps_binary} init --shell powershell | Out-String)"
    );
    println!(
        "The init command enables suggestions only for the current shell; it does not edit your profile or execute suggested commands."
    );
    Ok(())
}

use std::io::BufRead;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_only_chat_completions() {
        for (base, expected) in [
            (
                "https://api.example.test/v1/",
                "https://api.example.test/v1/chat/completions",
            ),
            (
                "https://api.example.test/chat/completions",
                "https://api.example.test/chat/completions",
            ),
            (
                "http://127.0.0.1:55/v1",
                "http://127.0.0.1:55/v1/chat/completions",
            ),
        ] {
            assert_eq!(endpoint_from_base(base).unwrap(), expected);
        }
        for bad in [
            "https://example.test/v1/completions",
            "https://user:secret@example.test/v1",
            "https://example.test/v1?api_key=secret",
            "http://example.test/v1",
        ] {
            assert!(endpoint_from_base(bad).is_err());
        }
    }
}
