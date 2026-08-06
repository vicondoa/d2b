//! Bounded shell completion generation.

use clap::{Args, ValueEnum};
use serde_json::json;
use std::time::Instant;

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext},
    dispatch::BUILTIN_COMMANDS,
    provider,
};

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

pub(crate) fn run(
    args: &CompleteArgs,
    context: Option<&ZoneContext>,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
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
    let provider_commands = context
        .map(|context| load_provider_commands(context, mode, deadline))
        .unwrap_or_default();
    let output = render_completion_names(shell, &provider_commands);
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
    let names = provider_commands
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect::<Vec<_>>();
    render_completion_names(shell, &names)
}

fn render_completion_names(shell: CompletionShell, provider_commands: &[String]) -> String {
    let mut names: Vec<String> = BUILTIN_COMMANDS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    for name in provider_commands {
        if valid_command_name(name) && !BUILTIN_COMMANDS.contains(&name.as_str()) {
            names.push(name.clone());
        }
    }
    names.sort();
    names.dedup();
    let words = names.join(" ");
    match shell {
        CompletionShell::Bash => format!(
            "_d2b_complete() {{\n  local cur=\"${{COMP_WORDS[COMP_CWORD]}}\"\n  COMPREPLY=( $(compgen -W '{words}' -- \"$cur\") )\n}}\ncomplete -F _d2b_complete d2b\n"
        ),
        CompletionShell::Zsh => format!("#compdef d2b\n_arguments '1:command:({words})'\n"),
        CompletionShell::Fish => format!("complete -c d2b -f -a '{words}'\n"),
    }
}

fn load_provider_commands(
    context: &ZoneContext,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Vec<String> {
    let started = Instant::now();
    let provider_deadline = ZoneContext::deadline(Some("2s")).unwrap_or(deadline);
    let Ok(value) = context.invoke(
        "List",
        json!({ "resourceType": "Provider", "readyOnly": true }),
        provider_deadline,
        mode,
    ) else {
        crate::print_stderr("d2b: Provider completion descriptors unavailable\n");
        return Vec::new();
    };
    let Some(items) = value.get("items").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for item in items {
        if started.elapsed().as_millis() > u128::from(MAX_PROVIDER_FETCH_MS) {
            crate::print_stderr("d2b: Provider completion deadline exceeded\n");
            break;
        }
        let Some(provider_ref) = item
            .get("resourceRef")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                item.pointer("/metadata/name")
                    .and_then(serde_json::Value::as_str)
            })
        else {
            continue;
        };
        let Ok(projection) = context.invoke(
            "InspectSchema",
            json!({ "resourceRef": provider_ref, "deadlineMs": MAX_PROVIDER_FETCH_MS }),
            provider_deadline,
            mode,
        ) else {
            crate::print_stderr("d2b: a Provider projection was unavailable\n");
            continue;
        };
        if provider::validate_projection_value(context, &projection, mode).is_err() {
            crate::print_stderr("d2b: a Provider projection was rejected\n");
            continue;
        }
        if let Some(name) = projection
            .pointer("/cliProjection/topLevel")
            .and_then(serde_json::Value::as_str)
            && valid_command_name(name)
            && !BUILTIN_COMMANDS.contains(&name)
        {
            names.push(name.to_owned());
        }
    }
    names.sort();
    names.dedup();
    names
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
        const _: () = assert!(MAX_PROVIDER_FETCH_MS < crate::context::MAX_REQUEST_LIFETIME_MS);
    }
}
