//! Provider resource inspection and dynamic CLI projection validation.

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext, parse_resource_ref},
    dispatch::BUILTIN_COMMANDS,
    resource,
};

pub(crate) const MAX_PROJECTION_BYTES: usize = 64 * 1024;
pub(crate) const MAX_PROJECTION_NAME_BYTES: usize = 32;
pub(crate) const MAX_PROJECTION_VERBS: usize = 32;
pub(crate) const MAX_PROJECTION_DESCRIPTION_BYTES: usize = 512;
pub(crate) const MAX_PROVIDER_PROJECTION_DEADLINE_MS: u64 = 2_000;

#[derive(Debug, Args, Clone)]
pub(crate) struct ProviderArgs {
    #[command(subcommand)]
    pub(crate) command: ProviderCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ProviderCommand {
    List(ProviderListArgs),
    Get(ProviderNameArgs),
    Status(ProviderStatusArgs),
    Inspect(ProviderNameArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ProviderListArgs {
    #[arg(long = "package-only")]
    pub(crate) package_only: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ProviderNameArgs {
    pub(crate) name: String,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ProviderStatusArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) watch: bool,
}

pub(crate) fn run(
    context: &ZoneContext,
    args: &ProviderArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        ProviderCommand::List(args) => {
            let value = context.invoke(
                "List",
                json!({
                    "resourceType": "Provider",
                    "packageOnly": args.package_only
                }),
                deadline,
                mode,
            )?;
            context.emit(&value, mode)?;
            Ok(0)
        }
        ProviderCommand::Get(args) => resource::get(
            context,
            &crate::dispatch::GenericGetArgs {
                resource_ref: format!("Provider/{}", args.name),
            },
            mode,
            deadline,
        ),
        ProviderCommand::Status(args) => {
            let resource_ref = parse_resource_ref(&format!("Provider/{}", args.name), None)?;
            let value = context.invoke(
                "Status",
                json!({
                    "resourceRef": resource_ref.to_canonical_string(),
                    "watch": args.watch,
                }),
                deadline,
                mode,
            )?;
            if args.watch {
                context.emit_stream(&value, mode)?;
            } else {
                context.emit(&value, mode)?;
            }
            Ok(0)
        }
        ProviderCommand::Inspect(args) => inspect(context, &args.name, mode, deadline),
    }
}

fn inspect(
    context: &ZoneContext,
    name: &str,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = parse_resource_ref(&format!("Provider/{name}"), None)?;
    let value = context.invoke(
        "InspectSchema",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "deadlineMs": MAX_PROVIDER_PROJECTION_DEADLINE_MS,
        }),
        deadline,
        mode,
    )?;
    validate_projection_value(context, &value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

pub(crate) fn validate_projection_value(
    context: &ZoneContext,
    value: &Value,
    mode: OutputMode,
) -> Result<(), CliFailure> {
    let bytes = serde_json::to_vec(value).map_err(|_| {
        context.failure(
            "resource-schema-invalid",
            "invalid Provider projection",
            mode,
            1,
        )
    })?;
    if bytes.len() > MAX_PROJECTION_BYTES {
        return Err(context.failure(
            "resource-schema-invalid",
            "Provider CLI projection exceeds 64 KiB",
            mode,
            1,
        ));
    }
    let Some(projection) = value.get("cliProjection") else {
        return Ok(());
    };
    let Some(top_level) = projection.get("topLevel").and_then(Value::as_str) else {
        return Err(context.failure(
            "resource-schema-invalid",
            "Provider CLI projection has no top-level name",
            mode,
            1,
        ));
    };
    validate_name(top_level, "top-level projection name")
        .map_err(|message| context.failure("resource-schema-invalid", &message, mode, 1))?;
    if BUILTIN_COMMANDS.contains(&top_level) {
        return Err(context.failure(
            "resource-schema-invalid",
            "Provider CLI projection collides with a built-in command",
            mode,
            1,
        ));
    }
    let verbs = projection
        .get("verbs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            context.failure(
                "resource-schema-invalid",
                "Provider CLI projection must declare verbs",
                mode,
                1,
            )
        })?;
    if verbs.len() > MAX_PROJECTION_VERBS {
        return Err(context.failure(
            "resource-schema-invalid",
            "Provider CLI projection declares too many verbs",
            mode,
            1,
        ));
    }
    for verb in verbs {
        let name = verb.get("name").and_then(Value::as_str).ok_or_else(|| {
            context.failure(
                "resource-schema-invalid",
                "Provider CLI projection verb has no name",
                mode,
                1,
            )
        })?;
        validate_name(name, "projection verb name")
            .map_err(|message| context.failure("resource-schema-invalid", &message, mode, 1))?;
        if let Some(description) = verb.get("description").and_then(Value::as_str)
            && description.len() > MAX_PROJECTION_DESCRIPTION_BYTES
        {
            return Err(context.failure(
                "resource-schema-invalid",
                "Provider CLI projection description exceeds 512 bytes",
                mode,
                1,
            ));
        }
        if let Some(arguments) = verb.get("arguments").and_then(Value::as_array)
            && arguments.len() > 16
        {
            return Err(context.failure(
                "resource-schema-invalid",
                "Provider CLI projection declares too many arguments",
                mode,
                1,
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_name(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_PROJECTION_NAME_BYTES
        || !value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!("{label} is invalid"));
    }
    Ok(())
}

/// The single bind-time collision authority used by install-time checks and
/// unit tests. Dispatch has no fallback for a rejected projection.
pub(crate) fn bind_projection(
    top_level: &str,
    provider: &str,
    already_bound: &[(&str, &str)],
) -> Result<(), &'static str> {
    validate_name(top_level, "top-level projection name").map_err(|_| "projection-name-invalid")?;
    if BUILTIN_COMMANDS.contains(&top_level) {
        return Err("projection-built-in-collision");
    }
    if already_bound
        .iter()
        .any(|(name, owner)| *name == top_level && *owner != provider)
    {
        return Err("projection-provider-collision");
    }
    Ok(())
}

pub(crate) fn sanitize_projection_text(value: &str) -> String {
    value
        .replace(['\n', '\r', '\t'], " ")
        .chars()
        .take(MAX_PROJECTION_DESCRIPTION_BYTES)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_rejects_provider_collisions_without_dispatch_fallback() {
        assert_eq!(
            bind_projection("get", "provider-a", &[]),
            Err("projection-built-in-collision")
        );
        assert_eq!(
            bind_projection("audio", "provider-b", &[("audio", "provider-a")]),
            Err("projection-provider-collision")
        );
        assert!(bind_projection("audio", "provider-a", &[("audio", "provider-a")]).is_ok());
    }

    #[test]
    fn projection_text_is_single_line_and_bounded() {
        let text = sanitize_projection_text("status\n\tready");
        assert_eq!(text, "status  ready");
        assert!(sanitize_projection_text(&"x".repeat(900)).len() <= 512);
    }
}
