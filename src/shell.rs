//! Explicit shell facts and conservative, shell-specific rendering.
//!
//! Nothing here detects the operating system, reads PATH, or probes aliases.
use crate::context::unsafe_char;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ShellKind {
    Bash,
    #[value(name = "powershell")]
    PowerShell,
    Cmd,
    Nushell,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellContext {
    pub kind: ShellKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub known_aliases: BTreeMap<String, String>,
}

impl ShellContext {
    pub fn new(kind: ShellKind) -> Self {
        Self {
            kind,
            version: None,
            known_commands: Vec::new(),
            known_aliases: BTreeMap::new(),
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.known_commands.len() > 128 || self.known_aliases.len() > 128 {
            return Err("shell context exceeds the 128-command/alias limit");
        }
        for value in self
            .version
            .iter()
            .chain(self.known_commands.iter())
            .chain(self.known_aliases.keys())
            .chain(self.known_aliases.values())
        {
            if value.is_empty() || value.len() > 256 || value.chars().any(unsafe_char) {
                return Err("shell metadata must be 1–256 bytes without terminal controls");
            }
        }
        if serde_json::to_vec(self)
            .map_err(|_| "invalid shell metadata")?
            .len()
            > 8192
        {
            return Err("shell metadata exceeds the 8192-byte limit");
        }
        Ok(())
    }

    pub fn knows(&self, name: &str) -> bool {
        let matches = |candidate: &str| match self.kind {
            ShellKind::Cmd | ShellKind::PowerShell => candidate.eq_ignore_ascii_case(name),
            _ => candidate == name,
        };
        self.known_commands.iter().any(|s| matches(s))
            || self.known_aliases.keys().any(|s| matches(s))
    }
}

/// Literal arguments, not arbitrary source code. Unsupported CMD/Nu values are
/// rejected instead of applying Bash escaping to another shell.
pub fn quote_literal(shell: ShellKind, text: &str) -> Result<String, &'static str> {
    if text.chars().any(unsafe_char) {
        return Err("shell values may not contain terminal controls");
    }
    match shell {
        ShellKind::Bash => Ok(format!("'{}'", text.replace('\'', "'\\''"))),
        ShellKind::PowerShell => {
            // PowerShell treats all five Unicode single-quote characters as delimiters.
            let escaped = text
                .chars()
                .flat_map(|c| {
                    if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
                        vec![c, c]
                    } else {
                        vec![c]
                    }
                })
                .collect::<String>();
            Ok(format!("'{escaped}'"))
        }
        ShellKind::Cmd => {
            if text.contains(['"', '%', '!', '^']) {
                return Err(
                    "CMD literal contains unsupported expansion/quote characters; configure an explicit CMD recipe",
                );
            }
            // CMD syntax and target-program argument parsing are different.
            // Use this only for CMD's built-in cd; argv uses quote_cmd_arg.
            Ok(format!("\"{text}\""))
        }
        ShellKind::Nushell => {
            let mut delimiter = String::from("#");
            while text.contains(&format!("'{delimiter}")) {
                delimiter.push('#');
            }
            Ok(format!("r{delimiter}'{text}'{delimiter}"))
        }
    }
}

pub fn valid_program(program: &str) -> bool {
    let mut bytes = program.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

pub fn render_command(
    shell: ShellKind,
    program: &str,
    args: &[String],
) -> Result<String, &'static str> {
    if !valid_program(program) {
        return Err(
            "typed command requires a simple command name; use an explicit shell recipe for paths or syntax",
        );
    }
    if shell == ShellKind::Cmd
        && [
            "if", "for", "else", "call", "start", "set", "setlocal", "endlocal", "rem", "goto",
            "exit", "shift",
        ]
        .iter()
        .any(|name| program.eq_ignore_ascii_case(name))
    {
        return Err("CMD control builtins require an explicit shell recipe, not typed argv");
    }
    let prefix = match shell {
        ShellKind::Bash => format!("'{program}'"),
        ShellKind::PowerShell => format!("& '{program}'"),
        // Force an external command rather than accidentally calling a Nu
        // keyword/custom command with incompatible argument semantics.
        ShellKind::Nushell => format!("^{program}"),
        _ => program.to_owned(),
    };
    let args = args.iter().map(|arg| {
        if shell == ShellKind::PowerShell && (arg.is_empty() || arg.contains('"')) {
            return Err("typed PowerShell arguments with empty values or double quotes need an explicit version-specific recipe (legacy native argv limitation)");
        }
        let quoted = quote_literal(shell, arg)?;
        if shell == ShellKind::Cmd && arg.ends_with('\\') {
            // A quoted argv uses conventional Windows CRT quoting. Bare
            // trailing backslashes must be doubled before the closing quote.
            let trailing = arg.bytes().rev().take_while(|b| *b == b'\\').count();
            Ok(format!("{}{}\"", &quoted[..quoted.len()-1], "\\".repeat(trailing)))
        } else { Ok(quoted) }
    }).collect::<Result<Vec<_>, &'static str>>()?;
    Ok(if args.is_empty() {
        prefix
    } else {
        format!("{prefix} {}", args.join(" "))
    })
}

pub fn render_cd(shell: ShellKind, path: &str) -> Result<String, &'static str> {
    let literal = quote_literal(shell, path)?;
    Ok(match shell {
        ShellKind::Bash => format!("cd -- {literal}"),
        ShellKind::PowerShell => format!("Set-Location -LiteralPath {literal} -ErrorAction Stop"),
        ShellKind::Cmd => format!("cd /d {literal}"),
        ShellKind::Nushell => format!("cd {literal}"),
    })
}

/// Commands are already rendered, trusted steps, never model-generated source.
/// Nu's external-command failure semantics vary by version: only one final
/// external command is automatically composed after built-in cd steps.
pub fn compose_steps(
    shell: ShellKind,
    steps: &[String],
    command_count: usize,
) -> Result<String, &'static str> {
    if steps.is_empty() {
        return Err("workflow has no steps");
    }
    if steps.len() == 1 {
        return Ok(steps[0].clone());
    }
    match shell {
        ShellKind::Bash | ShellKind::Cmd => Ok(steps.join(" && ")),
        ShellKind::PowerShell => {
            let mut tail = steps.last().expect("nonempty").clone();
            for step in steps[..steps.len() - 1].iter().rev() {
                tail = format!("{step}; if ($?) {{ {tail} }}");
            }
            Ok(format!("& {{ {tail} }}"))
        }
        ShellKind::Nushell if command_count <= 1 => Ok(format!("do {{ {} }}", steps.join("; "))),
        ShellKind::Nushell => Err(
            "multiple external Nushell steps require one explicit shell recipe with failure handling",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_literals_preserve_spaces_metacharacters_and_unicode() {
        let text = "a b;$x&|<>世界";
        assert_eq!(
            quote_literal(ShellKind::Bash, text).unwrap(),
            format!("'{text}'")
        );
        assert_eq!(
            quote_literal(ShellKind::PowerShell, text).unwrap(),
            format!("'{text}'")
        );
        assert_eq!(
            quote_literal(ShellKind::Cmd, text).unwrap(),
            format!("\"{text}\"")
        );
        assert_eq!(
            quote_literal(ShellKind::Nushell, text).unwrap(),
            format!("r#'{text}'#")
        );
        assert_eq!(quote_literal(ShellKind::Bash, "a'b").unwrap(), "'a'\\''b'");
        assert_eq!(
            quote_literal(ShellKind::PowerShell, "a'b").unwrap(),
            "'a''b'"
        );
        assert_eq!(
            quote_literal(ShellKind::Nushell, "a'b").unwrap(),
            "r#'a'b'#"
        );
        assert_eq!(
            quote_literal(ShellKind::Nushell, "a'#b").unwrap(),
            "r##'a'#b'##"
        );
        assert_eq!(
            quote_literal(ShellKind::PowerShell, "a’b").unwrap(),
            "'a’’b'"
        );
        for text in ["a\"b", "%PATH%", "!X!", "a^b"] {
            assert!(quote_literal(ShellKind::Cmd, text).is_err());
        }
        for shell in [
            ShellKind::Bash,
            ShellKind::PowerShell,
            ShellKind::Cmd,
            ShellKind::Nushell,
        ] {
            assert!(quote_literal(shell, "x\ny").is_err());
            assert!(render_command(shell, "echo;bad", &[]).is_err());
        }
    }
    #[test]
    fn composition_is_not_universal_semicolon_concatenation() {
        let steps = vec!["first".into(), "second".into()];
        assert_eq!(
            compose_steps(ShellKind::Bash, &steps, 2).unwrap(),
            "first && second"
        );
        assert_eq!(
            compose_steps(ShellKind::Cmd, &steps, 2).unwrap(),
            "first && second"
        );
        assert_eq!(
            compose_steps(ShellKind::PowerShell, &steps, 2).unwrap(),
            "& { first; if ($?) { second } }"
        );
        assert!(compose_steps(ShellKind::Nushell, &steps, 2).is_err());
        assert_eq!(
            render_command(ShellKind::Cmd, "python", &["C:\\space dir\\".into()]).unwrap(),
            "python \"C:\\space dir\\\\\""
        );
    }
    #[test]
    fn actual_commands_and_aliases_have_shell_specific_case_rules() {
        let mut shell = ShellContext::new(ShellKind::PowerShell);
        shell.known_aliases.insert("LS".into(), "My-Listing".into());
        assert!(shell.knows("ls"));
        shell.kind = ShellKind::Bash;
        assert!(!shell.knows("ls"));
    }
}
