//! Bounded shell completion generation.

use clap::{Args, ValueEnum};
use serde_json::json;

use crate::{CliFailure, dispatch::BUILTIN_COMMANDS};

pub(crate) const MAX_COMPLETION_BYTES: usize = 256 * 1024;
pub(crate) const MAX_PROVIDER_FETCH_MS: u64 = 10_000;

#[derive(Debug, Args, Clone)]
pub(crate) struct CompleteArgs {
    pub(crate) shell: Option<CompletionShell>,
    #[arg(long = "list-commands")]
    pub(crate) list_commands: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

pub(crate) fn run(args: &CompleteArgs) -> Result<i32, CliFailure> {
    if args.list_commands {
        let mut output = serde_json::to_string(&json!({
            "schemaVersion": 1,
            "commands": BUILTIN_COMMANDS,
        }))
        .map_err(|_| CliFailure::new(1, "failed to render completion commands"))?;
        output.push('\n');
        crate::print_stdout(&output);
        return Ok(0);
    }
    let shell = args
        .shell
        .ok_or_else(|| CliFailure::new(2, "complete requires bash, zsh, or fish"))?;
    let output = render_completion(shell, &[]);
    if output.len() > MAX_COMPLETION_BYTES {
        return Err(CliFailure::new(
            1,
            "completion output exceeds its 256 KiB bound",
        ));
    }
    crate::print_stdout(&output);
    Ok(0)
}

pub(crate) fn render_completion(
    shell: CompletionShell,
    provider_commands: &[(&str, &[&str])],
) -> String {
    let mut names: Vec<String> = BUILTIN_COMMANDS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    for (name, _) in provider_commands {
        if valid_command_name(name) && !BUILTIN_COMMANDS.contains(name) {
            names.push((*name).to_owned());
        }
    }
    names.sort();
    names.dedup();
    let words = names.join(" ");
    let output = match shell {
        CompletionShell::Bash => format!(
            "_d2b_complete() {{\n  local cur=\"${{COMP_WORDS[COMP_CWORD]}}\"\n  COMPREPLY=( $(compgen -W '{words}' -- \"$cur\") )\n}}\ncomplete -F _d2b_complete d2b\n"
        ),
        CompletionShell::Zsh => format!("#compdef d2b\n_arguments '1:command:({words})'\n"),
        CompletionShell::Fish => format!("complete -c d2b -f -a '{words}'\n"),
    };
    output
        .chars()
        .map(|character| {
            if character == '\n' {
                character
            } else {
                character
            }
        })
        .collect()
}

fn valid_command_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtin_completions_are_bounded_and_shell_safe() {
        for shell in [
            CompletionShell::Bash,
            CompletionShell::Zsh,
            CompletionShell::Fish,
        ] {
            let output = render_completion(shell, &[("audio", &["status"]), ("bad;rm", &["x"])]);
            assert!(output.len() < MAX_COMPLETION_BYTES);
            assert!(output.contains("audio"));
            assert!(!output.contains("bad;rm"));
            assert!(!output.contains('\r'));
        }
    }

    #[test]
    fn completion_deadline_is_not_longer_than_request_bound() {
        assert!(MAX_PROVIDER_FETCH_MS < crate::context::MAX_REQUEST_LIFETIME_MS);
    }
}
