//! ResourceExport and ResourceImport operator surfaces.

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext, parse_resource_ref, read_spec},
    dispatch::GenericGetArgs,
    resource,
};

#[derive(Debug, Args, Clone)]
pub(crate) struct ExportArgs {
    #[command(subcommand)]
    pub(crate) command: ShareCommand,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ImportArgs {
    #[command(subcommand)]
    pub(crate) command: ImportCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ShareCommand {
    Get(ShareNameArgs),
    List(ExportListArgs),
    Watch(ExportWatchArgs),
    Status(ShareStatusArgs),
    Create(ShareCreateArgs),
    #[command(name = "update-spec")]
    UpdateSpec(ShareUpdateSpecArgs),
    Delete(ShareDeleteArgs),
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ImportCommand {
    Get(ShareNameArgs),
    List(ImportListArgs),
    Watch(ImportWatchArgs),
    Status(ShareStatusArgs),
    Projection(ShareNameArgs),
    Graph(ShareNameArgs),
    Create(ShareCreateArgs),
    #[command(name = "update-spec")]
    UpdateSpec(ShareUpdateSpecArgs),
    Delete(ShareDeleteArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ShareNameArgs {
    pub(crate) name: String,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ExportListArgs {
    #[arg(long = "exported-type")]
    pub(crate) exported_type: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ExportWatchArgs {
    #[arg(long = "exported-type")]
    pub(crate) exported_type: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ImportListArgs {
    #[arg(long = "expected-type")]
    pub(crate) expected_type: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ImportWatchArgs {
    #[arg(long = "expected-type")]
    pub(crate) expected_type: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ShareStatusArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) watch: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ShareCreateArgs {
    #[arg(long = "spec-file", conflicts_with = "spec-stdin")]
    pub(crate) spec_file: Option<std::path::PathBuf>,
    #[arg(long = "spec-stdin", conflicts_with = "spec-file")]
    pub(crate) spec_stdin: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ShareUpdateSpecArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) revision: Option<String>,
    #[arg(long = "spec-file", conflicts_with = "spec-stdin")]
    pub(crate) spec_file: Option<std::path::PathBuf>,
    #[arg(long = "spec-stdin", conflicts_with = "spec-file")]
    pub(crate) spec_stdin: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ShareDeleteArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) revision: Option<String>,
}

pub(crate) fn run_export(
    context: &ZoneContext,
    args: &ExportArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        ShareCommand::Get(args) => get(context, "ResourceExport", &args.name, mode, deadline),
        ShareCommand::List(args) => list(
            context,
            "ResourceExport",
            json!({ "exportedType": args.exported_type }),
            mode,
            deadline,
        ),
        ShareCommand::Watch(args) => watch(
            context,
            "ResourceExport",
            json!({ "exportedType": args.exported_type }),
            mode,
            deadline,
        ),
        ShareCommand::Status(args) => status(context, "ResourceExport", args, mode, deadline),
        ShareCommand::Create(args) => create(context, "ResourceExport", args, mode, deadline),
        ShareCommand::UpdateSpec(args) => {
            update_spec(context, "ResourceExport", args, mode, deadline)
        }
        ShareCommand::Delete(args) => delete(context, "ResourceExport", args, mode, deadline),
    }
}

pub(crate) fn run_import(
    context: &ZoneContext,
    args: &ImportArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        ImportCommand::Get(args) => get(context, "ResourceImport", &args.name, mode, deadline),
        ImportCommand::List(args) => list(
            context,
            "ResourceImport",
            json!({ "expectedType": args.expected_type }),
            mode,
            deadline,
        ),
        ImportCommand::Watch(args) => watch(
            context,
            "ResourceImport",
            json!({ "expectedType": args.expected_type }),
            mode,
            deadline,
        ),
        ImportCommand::Status(args) => status(context, "ResourceImport", args, mode, deadline),
        ImportCommand::Projection(args) => projection(context, &args.name, mode, deadline),
        ImportCommand::Graph(args) => graph(context, &args.name, mode, deadline),
        ImportCommand::Create(args) => create(context, "ResourceImport", args, mode, deadline),
        ImportCommand::UpdateSpec(args) => {
            update_spec(context, "ResourceImport", args, mode, deadline)
        }
        ImportCommand::Delete(args) => delete(context, "ResourceImport", args, mode, deadline),
    }
}

fn get(
    context: &ZoneContext,
    resource_type: &str,
    name: &str,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let generic = GenericGetArgs {
        resource_ref: format!("{resource_type}/{name}"),
    };
    let value = resource::request_get(context, &generic, mode, deadline)?;
    let value = sanitize_share_output(context, value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn list(
    context: &ZoneContext,
    resource_type: &str,
    filters: Value,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let mut payload = match filters {
        Value::Object(object) => object,
        _ => unreachable!("share filters are objects"),
    };
    payload.insert(
        "resourceType".to_owned(),
        Value::String(resource_type.to_owned()),
    );
    let value = context.invoke("List", Value::Object(payload), deadline, mode)?;
    let value = sanitize_share_output(context, value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn watch(
    context: &ZoneContext,
    resource_type: &str,
    filters: Value,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    if !mode.is_json() {
        return Err(context.failure(
            "ref-invalid",
            "share watch output is JSON-lines only",
            mode,
            2,
        ));
    }
    let mut payload = match filters {
        Value::Object(object) => object,
        _ => unreachable!("share filters are objects"),
    };
    payload.insert(
        "resourceType".to_owned(),
        Value::String(resource_type.to_owned()),
    );
    let value = context.invoke("Watch", Value::Object(payload), deadline, mode)?;
    let value = sanitize_share_output(context, value, mode)?;
    context.emit_stream(&value, mode)?;
    Ok(0)
}

fn status(
    context: &ZoneContext,
    resource_type: &str,
    args: &ShareStatusArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = parse_resource_ref(&format!("{resource_type}/{}", args.name), None)?;
    let value = context.invoke(
        "Status",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "watch": args.watch,
        }),
        deadline,
        mode,
    )?;
    let value = sanitize_share_output(context, value, mode)?;
    if args.watch {
        context.emit_stream(&value, mode)?;
    } else {
        context.emit(&value, mode)?;
    }
    Ok(0)
}

fn projection(
    context: &ZoneContext,
    name: &str,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = parse_resource_ref(&format!("ResourceImport/{name}"), None)?;
    let value = context.invoke(
        "GetProjection",
        json!({ "resourceRef": resource_ref.to_canonical_string() }),
        deadline,
        mode,
    )?;
    let value = sanitize_share_output(context, value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn graph(
    context: &ZoneContext,
    name: &str,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = parse_resource_ref(&format!("ResourceImport/{name}"), None)?;
    let value = context.invoke(
        "GetImportGraph",
        json!({ "resourceRef": resource_ref.to_canonical_string() }),
        deadline,
        mode,
    )?;
    let value = sanitize_share_output(context, value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn create(
    context: &ZoneContext,
    resource_type: &str,
    args: &ShareCreateArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let spec = read_spec(args.spec_file.as_deref(), args.spec_stdin)?;
    validate_share_spec(context, resource_type, &spec, mode)?;
    let value = context.invoke(
        "Create",
        json!({ "resourceType": resource_type, "spec": spec }),
        deadline,
        mode,
    )?;
    let value = sanitize_share_output(context, value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn update_spec(
    context: &ZoneContext,
    resource_type: &str,
    args: &ShareUpdateSpecArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = parse_resource_ref(&format!("{resource_type}/{}", args.name), None)?;
    let spec = read_spec(args.spec_file.as_deref(), args.spec_stdin)?;
    validate_share_spec(context, resource_type, &spec, mode)?;
    let value = context.invoke(
        "UpdateSpec",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "expectedRevision": args.revision,
            "spec": spec,
        }),
        deadline,
        mode,
    )?;
    let value = sanitize_share_output(context, value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn delete(
    context: &ZoneContext,
    resource_type: &str,
    args: &ShareDeleteArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = parse_resource_ref(&format!("{resource_type}/{}", args.name), None)?;
    let value = context.invoke(
        "Delete",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "expectedRevision": args.revision,
            "waitForBindings": true,
        }),
        deadline,
        mode,
    )?;
    let value = sanitize_share_output(context, value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn validate_share_spec(
    context: &ZoneContext,
    resource_type: &str,
    spec: &Value,
    mode: OutputMode,
) -> Result<(), CliFailure> {
    if contains_forbidden_share_key(spec) {
        return Err(context.failure(
            "resource-schema-invalid",
            "share specs cannot contain backing, remote, session, stream, descriptor, secret, path, locator, or byte fields",
            mode,
            1,
        ));
    }
    if spec.get("provider").is_some() {
        return Err(context.failure(
            "resource-schema-invalid",
            "share base specs cannot carry implementation Provider settings",
            mode,
            1,
        ));
    }
    if resource_type == "ResourceExport" {
        let owner = spec
            .get("resourceRef")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                context.failure(
                    "resource-schema-invalid",
                    "ResourceExport requires resourceRef",
                    mode,
                    1,
                )
            })?;
        let owner = parse_resource_ref(owner, None).map_err(|_| {
            context.failure(
                "resource-schema-invalid",
                "ResourceExport resourceRef is invalid",
                mode,
                1,
            )
        })?;
        if !owner.resource_type().as_str().contains("Service")
            || owner.resource_type().as_str().contains("Binding")
            || matches!(owner.resource_type().as_str(), "Device" | "Endpoint")
        {
            return Err(context.failure(
                "resource-schema-invalid",
                "ResourceExport.resourceRef must target a local factory-bound Service",
                mode,
                1,
            ));
        }
    }
    if resource_type == "ResourceImport" {
        let zone_link = spec.get("zoneLinkRef").and_then(Value::as_str);
        if zone_link.is_none() {
            return Err(context.failure(
                "resource-schema-invalid",
                "ResourceImport requires a local zoneLinkRef",
                mode,
                1,
            ));
        }
        if spec.get("remoteRef").is_some() || spec.get("remoteZone").is_some() {
            return Err(context.failure(
                "resource-schema-invalid",
                "ResourceImport cannot carry a remote reference",
                mode,
                1,
            ));
        }
    }
    Ok(())
}

fn sanitize_share_output(
    context: &ZoneContext,
    value: Value,
    mode: OutputMode,
) -> Result<Value, CliFailure> {
    if contains_forbidden_share_key(&value) {
        return Err(context.failure(
            "resource-schema-invalid",
            "share output contains forbidden implementation detail",
            mode,
            1,
        ));
    }
    Ok(value)
}

fn contains_forbidden_share_key(value: &Value) -> bool {
    const FORBIDDEN: &[&str] = &[
        "backing",
        "remoteRef",
        "remoteZone",
        "session",
        "stream",
        "fd",
        "secret",
        "path",
        "locator",
        "socket",
        "bytes",
        "credential",
        "token",
    ];
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            FORBIDDEN.iter().any(|forbidden| key == forbidden)
                || contains_forbidden_share_key(value)
        }),
        Value::Array(values) => values.iter().any(contains_forbidden_share_key),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_output_never_leaks_private_shapes() {
        assert!(contains_forbidden_share_key(&json!({
            "projection": {"remoteRef": "Zone/remote"},
            "graph": [{"ownerRef":"ResourceImport/mic"}]
        })));
        assert!(!contains_forbidden_share_key(
            &json!({"status":{"phase":"Ready"}})
        ));
        assert!(contains_forbidden_share_key(
            &json!({"status":{"bytes":"opaque"}})
        ));
    }
}
