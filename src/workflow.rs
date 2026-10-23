//! Offline, deterministic project workflows and conservative intent translation.
//! Only user-confirmed configuration is consulted; suggestions are not facts.
use crate::{
    context::Context,
    project_memory::{ProjectMemory, WorkflowStep, normalize_name},
    shell::{ShellContext, ShellKind, compose_steps, render_cd, render_command},
    structured_edit::{StructuredSuggestion, TextEdit},
};

#[derive(Clone)]
pub struct LocalSuggestion {
    pub suggestion: StructuredSuggestion,
    pub source: &'static str,
    pub explanation: &'static str,
    pub warnings: Vec<&'static str>,
    /// Original typed steps, not guessed by splitting shell source.
    pub steps: Vec<String>,
}

pub fn suggest_local(
    context: &Context,
    memory: Option<&ProjectMemory>,
) -> Result<Option<LocalSuggestion>, &'static str> {
    let Some(shell) = context.shell() else {
        return Ok(None);
    };
    shell.validate()?;
    let buffer = format!("{}{}", context.prefix(), context.suffix());
    let cursor = context.prefix().len();
    if let Some(memory) = memory {
        // Deserialized values from direct library consumers are revalidated.
        memory.validate().map_err(|_| "project memory is invalid")?;
        let needle = normalize_name(&buffer);
        let mut found = None;
        for (name, project) in &memory.confirmed {
            for (workflow_name, workflow) in &project.workflows {
                let generated_match = std::iter::once(name)
                    .chain(project.aliases.iter())
                    .any(|alias| needle == normalize_name(&format!("{workflow_name} {alias}")));
                let explicit_match = workflow
                    .aliases
                    .iter()
                    .any(|alias| needle == normalize_name(alias));
                if !generated_match && !explicit_match {
                    continue;
                }
                if found.is_some() {
                    return Err("workflow phrase is ambiguous; edit the confirmed aliases");
                }
                let mut steps = Vec::new();
                let mut commands = 0;
                for (index, step) in workflow.steps.iter().enumerate() {
                    steps.push(match step {
                        WorkflowStep::ChangeDirectory => {
                            if !absolute_project_path(&project.path) {
                                return Err("relative project path requires an explicitly supplied project root");
                            }
                            render_cd(shell.kind, &project.path)?
                        }
                        WorkflowStep::Command { program, args } => {
                            commands += 1;
                            if shell.kind == ShellKind::Nushell && index + 1 != workflow.steps.len() {
                                return Err("typed Nushell workflow requires its one external command last; use an explicit recipe otherwise");
                            }
                            render_command(shell.kind, program, args)?
                        }
                        WorkflowStep::Shell { shell: expected, command } => {
                            if *expected != shell.kind { return Err("configured recipe targets another shell; add a matching recipe"); }
                            if workflow.steps.len() != 1 { return Err("explicit shell source must be a single complete recipe"); }
                            command.clone()
                        }
                    });
                }
                let replacement = compose_steps(shell.kind, &steps, commands)?;
                let suggestion = StructuredSuggestion::new(
                    &buffer,
                    cursor,
                    TextEdit {
                        start: 0,
                        end: buffer.len(),
                        replacement,
                    },
                )?;
                let warnings = if project.environment.is_some() {
                    vec![
                        "Environment is a descriptive label, not implicit activation; configure shell integration or an explicit conda run command.",
                    ]
                } else {
                    Vec::new()
                };
                found = Some(LocalSuggestion {
                    suggestion,
                    source: "confirmed_workflow",
                    explanation: "Exact match from explicitly selected local configuration; review before accepting.",
                    warnings,
                    steps,
                });
            }
        }
        if found.is_some() {
            return Ok(found);
        }
    }
    translate_intent(&buffer, cursor, shell)
}

fn absolute_project_path(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with("\\\\")
        || (path.as_bytes().get(1) == Some(&b':')
            && path
                .as_bytes()
                .get(2)
                .is_some_and(|c| *c == b'\\' || *c == b'/'))
}

fn translate_intent(
    buffer: &str,
    cursor: usize,
    shell: &ShellContext,
) -> Result<Option<LocalSuggestion>, &'static str> {
    // Only these complete, argument-free intents have unambiguous semantics.
    // Flags and pipelines need shell-specific understanding, not substitutions.
    let word = buffer.trim();
    if shell.knows(word) {
        return Ok(None);
    }
    let replacement = match (shell.kind, word) {
        (ShellKind::Cmd, "ls") => "dir",
        (ShellKind::Cmd, "pwd") => "cd",
        (ShellKind::PowerShell, "ls") => "Get-ChildItem",
        (ShellKind::PowerShell, "pwd") => "Get-Location",
        _ => return Ok(None),
    };
    let start = buffer.len() - buffer.trim_start().len();
    let end = buffer.trim_end().len();
    Ok(Some(LocalSuggestion {
        suggestion: StructuredSuggestion::new(
            buffer,
            cursor,
            TextEdit {
                start,
                end,
                replacement: replacement.into(),
            },
        )?,
        source: "shell_intent",
        explanation: "Canonical target-shell equivalent; explicitly supplied commands and aliases take precedence.",
        warnings: Vec::new(),
        steps: vec![replacement.into()],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cmd_translations_replace_only_complete_intents_and_respect_real_commands() {
        let shell = ShellContext::new(ShellKind::Cmd);
        for (line, want) in [("ls", "dir"), ("pwd", "cd"), (" ls ", " dir ")] {
            let context = Context::new(line, line.len())
                .unwrap()
                .with_shell(shell.clone())
                .unwrap();
            let s = suggest_local(&context, None).unwrap().unwrap();
            assert_eq!(s.suggestion.apply(line, line.len()).unwrap().0, want);
        }
        for line in ["ls -la", "pwd | more", "please list files"] {
            let context = Context::new(line, line.len())
                .unwrap()
                .with_shell(shell.clone())
                .unwrap();
            assert!(suggest_local(&context, None).unwrap().is_none());
        }
        let mut shell = shell;
        shell.known_commands.push("ls".into());
        let context = Context::new("ls", 2).unwrap().with_shell(shell).unwrap();
        assert!(suggest_local(&context, None).unwrap().is_none());
    }
    #[test]
    fn powershell_aliases_are_not_overridden() {
        let mut shell = ShellContext::new(ShellKind::PowerShell);
        shell
            .known_aliases
            .insert("LS".into(), "Custom-Listing".into());
        let context = Context::new("ls", 2).unwrap().with_shell(shell).unwrap();
        assert!(suggest_local(&context, None).unwrap().is_none());
    }
}
