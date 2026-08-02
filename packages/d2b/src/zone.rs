//! Zone self-resource and compiler-topology projections.

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext, parse_resource_ref},
};

#[derive(Debug, Args, Clone)]
pub(crate) struct ZoneArgs {
    #[command(subcommand)]
    pub(crate) command: ZoneCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ZoneCommand {
    Get(ZoneGetArgs),
    List,
    Status(ZoneStatusArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ZoneGetArgs {
    pub(crate) name: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ZoneStatusArgs {
    pub(crate) name: Option<String>,
    #[arg(long)]
    pub(crate) watch: bool,
}

pub(crate) fn run(
    context: &ZoneContext,
    args: &ZoneArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        ZoneCommand::Get(args) => get(context, args, mode, deadline),
        ZoneCommand::List => list(context, mode, deadline),
        ZoneCommand::Status(args) => status(context, args, mode, deadline),
    }
}

fn get(
    context: &ZoneContext,
    args: &ZoneGetArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let name = args.name.as_deref().unwrap_or_else(|| context.zone_name());
    let resource_ref = parse_resource_ref(&format!("Zone/{name}"), None)?;
    let value = context.invoke(
        "ZoneGet",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "current": name == context.zone_name(),
        }),
        deadline,
        mode,
    )?;
    let value = sanitize_topology(context, value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn list(
    context: &ZoneContext,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let value = context.invoke("ZoneList", json!({}), deadline, mode)?;
    let value = sanitize_topology(context, value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn status(
    context: &ZoneContext,
    args: &ZoneStatusArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let name = args.name.as_deref().unwrap_or_else(|| context.zone_name());
    if args.watch && !mode.is_json() {
        return Err(context.failure(
            "ref-invalid",
            "zone status --watch output is JSON-lines only",
            mode,
            2,
        ));
    }
    let value = context.invoke(
        "ZoneStatus",
        json!({
            "resourceRef": format!("Zone/{name}"),
            "watch": args.watch,
        }),
        deadline,
        mode,
    )?;
    let value = sanitize_topology(context, value, mode)?;
    if args.watch {
        context.emit_stream(&value, mode)?;
    } else {
        context.emit(&value, mode)?;
    }
    Ok(0)
}

fn sanitize_topology(
    context: &ZoneContext,
    value: Value,
    mode: OutputMode,
) -> Result<Value, CliFailure> {
    if contains_forbidden_topology_field(&value) {
        return Err(context.failure(
            "resource-schema-invalid",
            "Zone topology output contains child-local ZoneLink details",
            mode,
            1,
        ));
    }
    Ok(value)
}

fn contains_forbidden_topology_field(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let lower = key.to_ascii_lowercase();
            matches!(
                lower.as_str(),
                "zonelink"
                    | "zonelinkref"
                    | "zonelinkuid"
                    | "zonelinkstatus"
                    | "zonelinks"
                    | "providerref"
                    | "transportsetting"
            ) || contains_forbidden_topology_field(value)
        }),
        Value::Array(values) => values.iter().any(contains_forbidden_topology_field),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_projection_rejects_child_local_link_fields() {
        assert!(!contains_forbidden_topology_field(&json!({
            "childZone":"dev",
            "parentZone":"local-root",
            "routeStatus":"ready"
        })));
        assert!(contains_forbidden_topology_field(&json!({
            "childZone":"dev",
            "zoneLinkRef":"ZoneLink/uplink"
        })));
    }
}
