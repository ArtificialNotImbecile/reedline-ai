//! Bounded local risk hints, not a full parser or a safety verdict. No execution.
use crate::shell::{ShellContext, ShellKind};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum RiskPolicy {
    Off,
    #[default]
    Warn,
    Confirm,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskReview {
    pub warnings: Vec<String>,
    pub incomplete: bool,
}

impl RiskReview {
    pub fn requires_confirmation(&self, policy: RiskPolicy) -> bool {
        policy == RiskPolicy::Confirm && (self.incomplete || !self.warnings.is_empty())
    }
    fn warn(&mut self, message: &str) {
        if !self.warnings.iter().any(|warning| warning == message) {
            self.warnings.push(message.to_owned());
        }
    }
}

const MAX_BYTES: usize = 32 * 1024;
const MAX_WORDS: usize = 512;

/// Only a small literal-command subset is recognized. No warnings does not
/// mean safe; unknown syntax, programs, wrappers, and aliases stay incomplete.
pub fn review(shell: Option<&ShellContext>, command: &str) -> RiskReview {
    let mut result = RiskReview {
        incomplete: shell.is_none(),
        ..RiskReview::default()
    };
    if shell.is_none() {
        result.warn("Shell is unspecified; command review is incomplete.");
    }
    if command.len() > MAX_BYTES {
        result.incomplete = true;
        return result;
    }
    let kind = shell.map_or(ShellKind::Bash, |shell| shell.kind);
    let mut chars = command.chars().peekable();
    let mut quote = None;
    let mut word = String::new();
    let mut words = Vec::new();
    let mut started = false;
    let mut count = 0;
    while let Some(c) = chars.next() {
        if c.is_control() && !matches!(c, '\n' | '\r' | '\t') {
            result.incomplete = true;
        }
        let escape = match kind {
            ShellKind::Bash | ShellKind::Nushell => '\\',
            ShellKind::PowerShell => '`',
            ShellKind::Cmd => '^',
        };
        if let Some(delimiter) = quote {
            if c == delimiter {
                if kind == ShellKind::PowerShell && delimiter == '\'' && chars.peek() == Some(&'\'')
                {
                    chars.next();
                    word.push('\'');
                } else {
                    quote = None;
                }
            } else if c == escape && delimiter == '"' && kind != ShellKind::Cmd {
                if let Some(next) = chars.next() {
                    if kind == ShellKind::Bash && !matches!(next, '$' | '`' | '"' | '\\' | '\n') {
                        word.push(c);
                    }
                    if next != '\n' {
                        word.push(next);
                    }
                } else {
                    result.incomplete = true;
                }
            } else {
                if (delimiter == '"' && matches!(c, '$' | '`') && kind != ShellKind::Cmd)
                    || (kind == ShellKind::Cmd && matches!(c, '%' | '!'))
                    || (kind == ShellKind::Bash && delimiter == '"' && c == '!')
                {
                    result.incomplete = true;
                }
                word.push(c);
            }
            continue;
        }
        if c == '"' || (c == '\'' && kind != ShellKind::Cmd) {
            quote = Some(c);
            started = true;
        } else if c == escape {
            started = true;
            if kind == ShellKind::Nushell {
                result.incomplete = true;
            }
            if let Some(next) = chars.next() {
                if next != '\n' {
                    word.push(next);
                }
            } else {
                result.incomplete = true;
            }
        } else if c.is_whitespace() || matches!(c, ';' | '&' | '|') {
            if started {
                words.push(std::mem::take(&mut word));
                started = false;
                count += 1;
            }
            if matches!(c, '\n' | '\r' | ';' | '&' | '|') {
                inspect(shell, kind, &words, &mut result);
                words.clear();
                if (c == '&' && kind == ShellKind::PowerShell)
                    || (c == ';' && kind == ShellKind::Cmd)
                {
                    result.incomplete = true;
                }
            }
        } else if c == '>' {
            result.warn("Output redirection may overwrite files.");
            result.incomplete = true;
        } else {
            if matches!(
                c,
                '$' | '`' | '(' | ')' | '{' | '}' | '<' | '*' | '?' | '[' | ']' | '~' | '#' | '!'
            ) || (kind == ShellKind::Cmd && matches!(c, '%' | '!'))
                || (kind == ShellKind::PowerShell && matches!(c, '@' | '\u{2018}'..='\u{201f}'))
                || (kind == ShellKind::Nushell && c == '^')
            {
                result.incomplete = true;
            }
            word.push(c);
            started = true;
        }
        if count >= MAX_WORDS {
            result.incomplete = true;
            return result;
        }
    }
    if quote.is_some() {
        result.incomplete = true;
    }
    if started {
        words.push(word);
    }
    inspect(shell, kind, &words, &mut result);
    result
}

fn inspect(
    shell: Option<&ShellContext>,
    kind: ShellKind,
    words: &[String],
    result: &mut RiskReview,
) {
    let Some(program) = words.first() else {
        return;
    };
    if program.contains(['/', '\\']) || words.iter().any(|word| word == "--%") {
        result.incomplete = true;
    }
    let insensitive = matches!(kind, ShellKind::Cmd | ShellKind::PowerShell);
    if shell.is_some_and(|shell| {
        shell.known_aliases.keys().any(|alias| {
            if insensitive {
                alias.eq_ignore_ascii_case(program)
            } else {
                alias == program
            }
        })
    }) {
        result.incomplete = true;
    }
    let normalized = if insensitive {
        program.to_ascii_lowercase()
    } else {
        program.clone()
    };
    let base = normalized.rsplit(['/', '\\']).next().unwrap_or("");
    let base = if insensitive {
        base.strip_suffix(".exe").unwrap_or(base)
    } else {
        base
    };
    let args: Vec<String> = words[1..]
        .iter()
        .map(|word| {
            if insensitive {
                word.to_ascii_lowercase()
            } else {
                word.clone()
            }
        })
        .collect();
    match base {
        "rm" | "remove-item" | "ri" | "del" | "erase" | "rmdir" | "rd" | "unlink" => {
            result.warn("Deletion command may permanently remove files or directories.")
        }
        "format" | "diskpart" | "mkfs" | "new-partition" | "format-volume" | "clear-disk" => {
            result.warn("Disk formatting or partition changes may destroy data.")
        }
        name if name.starts_with("mkfs.") => result.warn("Disk formatting may destroy data."),
        "dd" => {
            if args.iter().any(|arg| arg.starts_with("of=")) {
                result.warn("dd output may overwrite a file or disk device.");
            }
            result.incomplete = true;
        }
        "git" => {
            // Search all argv so global options cannot hide destructive verbs.
            let has = |value: &str| args.iter().any(|arg| arg == value);
            if (has("reset") && args.iter().any(|arg| arg.starts_with("--hard")))
                || has("clean")
                || has("restore")
                || (has("checkout") && has("--"))
            {
                result.warn("Git command may discard uncommitted work or remove files.");
            }
            // Aliases, config options, hooks, and extensions are not resolved.
            result.incomplete = true;
        }
        "echo" | "printf" | "pwd" | "ls" | "dir" | "cat" | "head" | "tail" | "wc" | "true"
        | "false" | "cd" | "get-location" | "get-childitem" | "write-output" => {}
        _ => result.incomplete = true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(kind: ShellKind, command: &str) -> RiskReview {
        review(Some(&ShellContext::new(kind)), command)
    }
    #[test]
    fn destructive_commands_and_steps() {
        for command in [
            "rm -rf /",
            "echo ok; rm x",
            "true && rmdir x",
            "ls | rm x",
            "echo ok\nrm x",
            "/bin/rm x",
            "r\\m x",
            "'rm' x",
            "r\"m\" x",
            "mkfs.ext4 /dev/sda",
            "dd if=x of=/dev/sda",
            "git reset --hard",
            "git -C somewhere clean -fd",
            "git restore x",
            "git checkout -- x",
            "echo hello > file",
            "printf x 2>file",
        ] {
            assert!(
                !check(ShellKind::Bash, command).warnings.is_empty(),
                "{command}"
            );
        }
    }
    #[test]
    fn literal_data_not_commands_or_redirects() {
        for command in [
            "echo 'rm -rf /'",
            "echo \"rm -rf /\"",
            "echo 'a > b; rm x'",
            "printf '%s' 'git reset --hard'",
            "echo a\\>b",
            "echo '$(rm x)'",
        ] {
            let review = check(ShellKind::Bash, command);
            assert!(review.warnings.is_empty(), "{command}: {review:?}");
            assert!(!review.incomplete, "{command}: {review:?}");
        }
    }
    #[test]
    fn expansions_wrappers_unknown_syntax() {
        for command in [
            "sudo rm x",
            "env rm x",
            "command rm x",
            "sh -c 'rm x'",
            "eval 'rm x'",
            "x=rm; $x file",
            "echo $(rm x)",
            "echo \"$(rm x)\"",
            "echo `rm x`",
            "echo \"`rm x`\"",
            "echo <(rm x)",
            "echo ${x}",
            "echo *",
            "echo 'oops",
            "echo hi\\",
            "function x() { rm file; }",
            "python -c 'pass'",
            "unknown",
            "/tmp/echo hi",
            "echo !123",
        ] {
            assert!(check(ShellKind::Bash, command).incomplete, "{command}");
        }
    }
    #[test]
    fn shell_specific_escaping_and_case() {
        for command in [
            "Remove-Item x",
            "r`m x",
            "FORMAT-VOLUME x",
            "C:\\bin\\RM.EXE x",
        ] {
            assert!(
                !check(ShellKind::PowerShell, command).warnings.is_empty(),
                "{command}"
            );
        }
        for command in ["DEL x", "d^el x", "RMDIR x", "FORMAT C:", "DISKPART"] {
            assert!(
                !check(ShellKind::Cmd, command).warnings.is_empty(),
                "{command}"
            );
        }
        for command in ["echo %COMMAND%", "echo !COMMAND!"] {
            assert!(check(ShellKind::Cmd, command).incomplete);
        }
        for command in ["echo '$x > rm'", "echo 'it''s > rm'", "echo `> rm"] {
            assert!(
                check(ShellKind::PowerShell, command).warnings.is_empty(),
                "{command}"
            );
        }
        assert!(check(ShellKind::PowerShell, "echo \"$x\"").incomplete);
        assert!(!check(ShellKind::Nushell, "rm file").warnings.is_empty());
        for command in ["^rm file", "echo r#'x'#", "do { rm file }"] {
            assert!(check(ShellKind::Nushell, command).incomplete);
        }
    }
    #[test]
    fn policies_aliases_bounds_unknown_shell() {
        let mut shell = ShellContext::new(ShellKind::Bash);
        shell.known_aliases.insert("ls".into(), "rm".into());
        let alias = review(Some(&shell), "ls");
        assert!(alias.incomplete);
        assert!(alias.requires_confirmation(RiskPolicy::Confirm));
        assert!(!alias.requires_confirmation(RiskPolicy::Warn));
        assert!(!alias.requires_confirmation(RiskPolicy::Off));
        assert_eq!(RiskPolicy::default(), RiskPolicy::Warn);
        assert!(review(None, "echo hi").incomplete);
        assert!(check(ShellKind::Bash, &"x".repeat(MAX_BYTES + 1)).incomplete);
        assert!(check(ShellKind::Bash, &"echo x; ".repeat(MAX_WORDS)).incomplete);
        assert!(!check(ShellKind::Bash, "echo hi").requires_confirmation(RiskPolicy::Confirm));
        assert!(check(ShellKind::Bash, "rm x").requires_confirmation(RiskPolicy::Confirm));
        assert_eq!(
            serde_json::to_string(&RiskPolicy::Confirm).unwrap(),
            "\"confirm\""
        );
    }
}
