//! Provider-neutral Endpoint read and resolution projections.

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext, parse_resource_ref},
};

#[derive(Debug, Args, Clone)]
pub(crate) struct EndpointArgs {
    #[command(subcommand)]
    pub(crate) command: EndpointCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum EndpointCommand {
    Get(EndpointNameArgs),
    List(EndpointListArgs),
    Watch(EndpointWatchArgs),
    Status(EndpointStatusArgs),
    Resolve(EndpointNameArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct EndpointNameArgs {
    pub(crate) name: String,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct EndpointListArgs {
    #[arg(long = "endpoint-class")]
    pub(crate) endpoint_class: Option<String>,
    #[arg(long)]
    pub(crate) updates: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct EndpointWatchArgs {
    #[arg(long = "endpoint-class")]
    pub(crate) endpoint_class: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct EndpointStatusArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) watch: bool,
}

pub(crate) fn run(
    context: &ZoneContext,
    args: &EndpointArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        EndpointCommand::Get(args) => get(context, &args.name, mode, deadline),
        EndpointCommand::List(args) => list(context, args, mode, deadline),
        EndpointCommand::Watch(args) => watch(context, args, mode, deadline),
        EndpointCommand::Status(args) => status(context, args, mode, deadline),
        EndpointCommand::Resolve(args) => resolve(context, &args.name, mode, deadline),
    }
}

fn get(
    context: &ZoneContext,
    name: &str,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = endpoint_ref(name)?;
    let value = context.invoke(
        "Get",
        json!({ "resourceRef": resource_ref.to_canonical_string() }),
        deadline,
        mode,
    )?;
    let value = sanitize_endpoint_output(context, value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn list(
    context: &ZoneContext,
    args: &EndpointListArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    if let Some(class) = args.endpoint_class.as_deref() {
        validate_endpoint_class(context, class, mode)?;
    }
    let value = context.invoke(
        "List",
        json!({
            "resourceType": "Endpoint",
            "endpointClass": args.endpoint_class,
            "updates": args.updates,
        }),
        deadline,
        mode,
    )?;
    let value = sanitize_endpoint_output(context, value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn watch(
    context: &ZoneContext,
    args: &EndpointWatchArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    if !mode.is_json() {
        return Err(context.failure(
            "ref-invalid",
            "endpoint watch output is JSON-lines only",
            mode,
            2,
        ));
    }
    if let Some(class) = args.endpoint_class.as_deref() {
        validate_endpoint_class(context, class, mode)?;
    }
    let value = context.invoke(
        "Watch",
        json!({
            "resourceType": "Endpoint",
            "endpointClass": args.endpoint_class,
        }),
        deadline,
        mode,
    )?;
    let value = sanitize_endpoint_output(context, value, mode)?;
    context.emit_stream(&value, mode)?;
    Ok(0)
}

fn status(
    context: &ZoneContext,
    args: &EndpointStatusArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = endpoint_ref(&args.name)?;
    if args.watch && !mode.is_json() {
        return Err(context.failure(
            "ref-invalid",
            "endpoint status --watch output is JSON-lines only",
            mode,
            2,
        ));
    }
    let value = context.invoke(
        "Status",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "watch": args.watch,
        }),
        deadline,
        mode,
    )?;
    let value = sanitize_endpoint_output(context, value, mode)?;
    if args.watch {
        context.emit_stream(&value, mode)?;
    } else {
        context.emit(&value, mode)?;
    }
    Ok(0)
}

fn resolve(
    context: &ZoneContext,
    name: &str,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = endpoint_ref(name)?;
    let value = context.invoke(
        "ResolveEndpoint",
        json!({ "resourceRef": resource_ref.to_canonical_string() }),
        deadline,
        mode,
    )?;
    let value = endpoint_resolution_projection(value);
    let value = sanitize_endpoint_output(context, value, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn endpoint_ref(name: &str) -> Result<d2b_contracts::v3::ResourceRef, CliFailure> {
    let resource_ref = parse_resource_ref(name, Some("Endpoint"))?;
    if resource_ref.resource_type().as_str() != "Endpoint" {
        return Err(CliFailure::new(
            2,
            "ref-invalid: Endpoint reference required",
        ));
    }
    Ok(resource_ref)
}

fn validate_endpoint_class(
    context: &ZoneContext,
    class: &str,
    mode: OutputMode,
) -> Result<(), CliFailure> {
    if !matches!(
        class,
        "service" | "device" | "transport" | "control" | "data"
    ) {
        return Err(context.failure(
            "ref-invalid",
            "endpoint class must be service, device, transport, control, or data",
            mode,
            2,
        ));
    }
    Ok(())
}

fn endpoint_resolution_projection(mut value: Value) -> Value {
    let allowed = [
        "ok",
        "zoneRef",
        "schemaVersion",
        "operationId",
        "resourceRef",
        "producerRef",
        "endpointClass",
        "transport",
        "readiness",
        "capabilities",
        "locality",
        "observations",
        "status",
    ];
    if let Value::Object(object) = &mut value {
        object.retain(|key, _| allowed.contains(&key.as_str()));
    }
    value
}

fn sanitize_endpoint_output(
    context: &ZoneContext,
    value: Value,
    mode: OutputMode,
) -> Result<Value, CliFailure> {
    let mut value = value;
    if find_forbidden_endpoint_key(&value).is_some() {
        return Err(context.failure(
            "resource-schema-invalid",
            "Endpoint output contains a forbidden raw locator field",
            mode,
            1,
        ));
    }
    if let Some(provider) = value.pointer("/status/provider")
        && serde_json::to_vec(provider)
            .map(|bytes| bytes.len() > 32 * 1024)
            .unwrap_or(true)
    {
        return Err(context.failure(
            "resource-schema-invalid",
            "Endpoint Provider status projection exceeds its bound",
            mode,
            1,
        ));
    }
    redact_unknown_provider_details(&mut value);
    Ok(value)
}

fn find_forbidden_endpoint_key(value: &Value) -> Option<String> {
    const FORBIDDEN: &[&str] = &[
        "path",
        "address",
        "cid",
        "port",
        "fd",
        "credential",
        "secret",
        "token",
        "locator",
        "socket",
        "deviceNode",
        "hostPath",
    ];
    match value {
        Value::Object(object) => object.iter().find_map(|(key, value)| {
            let lower = key.to_ascii_lowercase();
            if FORBIDDEN
                .iter()
                .any(|part| lower == *part || lower.contains(part))
            {
                Some(key.clone())
            } else {
                find_forbidden_endpoint_key(value)
            }
        }),
        Value::Array(values) => values.iter().find_map(find_forbidden_endpoint_key),
        _ => None,
    }
}

fn redact_unknown_provider_details(value: &mut Value) {
    if let Some(provider) = value.pointer_mut("/status/provider")
        && let Value::Object(object) = provider
    {
        const ALLOWED_PROVIDER_FIELDS: &[&str] = &[
            "phase",
            "generation",
            "class",
            "readiness",
            "capabilities",
            "locality",
            "transportClass",
            "observations",
            "details",
        ];
        object.retain(|key, _| ALLOWED_PROVIDER_FIELDS.contains(&key.as_str()));
        if let Some(details) = object.get_mut("details") {
            if let Value::Object(details) = details {
                const ALLOWED_DETAIL_FIELDS: &[&str] = &[
                    "phase",
                    "generation",
                    "readiness",
                    "capabilities",
                    "locality",
                ];
                details.retain(|key, _| ALLOWED_DETAIL_FIELDS.contains(&key.as_str()));
            } else {
                *details = Value::Object(Map::new());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_short_and_full_refs_are_local() {
        assert_eq!(
            endpoint_ref("ready").unwrap().to_canonical_string(),
            "Endpoint/ready"
        );
        assert_eq!(
            endpoint_ref("Endpoint/ready")
                .unwrap()
                .to_canonical_string(),
            "Endpoint/ready"
        );
        assert!(endpoint_ref("Guest/ready").is_err());
    }

    #[test]
    fn endpoint_projection_drops_unrelated_fields() {
        let value = endpoint_resolution_projection(json!({
            "resourceRef": "Endpoint/ready",
            "producerRef": "Process/producer",
            "rawLocator": "/run/private.sock",
            "secret": "x"
        }));
        assert!(value.get("rawLocator").is_none());
        assert!(value.get("secret").is_none());
        assert!(value.get("producerRef").is_some());
    }

    #[test]
    fn endpoint_output_rejects_raw_locator_keys() {
        assert!(find_forbidden_endpoint_key(&json!({"status":{"addressClass":"x"}})).is_some());
        assert!(find_forbidden_endpoint_key(&json!({"endpointClass":"service"})).is_none());
    }

    #[test]
    fn unknown_provider_projection_details_are_redacted() {
        let mut value = json!({
            "status": {
                "provider": {
                    "phase": "Ready",
                    "unknownImplementationField": "hidden",
                    "details": {
                        "readiness": "ready",
                        "privateImplementationField": "hidden"
                    }
                }
            }
        });
        redact_unknown_provider_details(&mut value);
        assert!(
            value
                .pointer("/status/provider/unknownImplementationField")
                .is_none()
        );
        assert!(
            value
                .pointer("/status/provider/details/privateImplementationField")
                .is_none()
        );
        assert_eq!(
            value.pointer("/status/provider/details/readiness"),
            Some(&json!("ready"))
        );
    }
}
