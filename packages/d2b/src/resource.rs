//! Generic Zone resource verbs.

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::{
    CliFailure,
    context::{
        OutputMode, RequestDeadline, ZoneContext, parse_resource_ref, parse_resource_type,
        read_spec,
    },
    dispatch::{
        GenericCreateArgs, GenericDeleteArgs, GenericGetArgs, GenericListArgs,
        GenericReconcileArgs, GenericStatusArgs, GenericUpdateSpecArgs, GenericUpgradeArgs,
        GenericWatchArgs,
    },
};

/// Typed noun commands reuse the generic resource verbs while documenting the
/// noun's default ResourceType in clap help.
#[derive(Debug, Args, Clone)]
pub(crate) struct TypedResourceArgs {
    #[command(subcommand)]
    pub(crate) command: TypedResourceCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum TypedResourceCommand {
    Get(TypedNameArgs),
    List(TypedListArgs),
    Watch(TypedWatchArgs),
    Create(TypedCreateArgs),
    #[command(name = "update-spec")]
    UpdateSpec(TypedUpdateSpecArgs),
    Delete(TypedNameMutationArgs),
    Status(TypedStatusArgs),
    Upgrade(TypedUpgradeArgs),
    Reconcile(TypedReconcileArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct TypedNameArgs {
    pub(crate) name: String,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct TypedListArgs {
    #[arg(long)]
    pub(crate) phase: Option<String>,
    #[arg(long = "label-selector")]
    pub(crate) label_selector: Option<String>,
    #[arg(long)]
    pub(crate) updates: bool,
    #[arg(long = "page-token")]
    pub(crate) page_token: Option<String>,
    #[arg(long)]
    pub(crate) limit: Option<u32>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct TypedWatchArgs {
    #[arg(long = "since-revision")]
    pub(crate) since_revision: Option<String>,
    #[arg(long)]
    pub(crate) phase: Option<String>,
    #[arg(long = "label-selector")]
    pub(crate) label_selector: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct TypedCreateArgs {
    #[arg(long = "spec-file", conflicts_with = "spec-stdin")]
    pub(crate) spec_file: Option<std::path::PathBuf>,
    #[arg(long = "spec-stdin", conflicts_with = "spec-file")]
    pub(crate) spec_stdin: bool,
    #[arg(long = "wait-for-reconcile")]
    pub(crate) wait_for_reconcile: bool,
    #[arg(long = "reconcile-deadline")]
    pub(crate) reconcile_deadline: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct TypedUpdateSpecArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) revision: Option<String>,
    #[arg(long = "spec-file", conflicts_with = "spec-stdin")]
    pub(crate) spec_file: Option<std::path::PathBuf>,
    #[arg(long = "spec-stdin", conflicts_with = "spec-file")]
    pub(crate) spec_stdin: bool,
    #[arg(long = "wait-for-reconcile")]
    pub(crate) wait_for_reconcile: bool,
    #[arg(long = "reconcile-deadline")]
    pub(crate) reconcile_deadline: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct TypedNameMutationArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) revision: Option<String>,
    #[arg(long = "wait-for-reconcile")]
    pub(crate) wait_for_reconcile: bool,
    #[arg(long = "reconcile-deadline")]
    pub(crate) reconcile_deadline: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct TypedStatusArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) watch: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct TypedUpgradeArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) recursive: bool,
    #[arg(long)]
    pub(crate) apply: bool,
    #[arg(long = "reconcile-deadline")]
    pub(crate) reconcile_deadline: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct TypedReconcileArgs {
    pub(crate) name: String,
    #[arg(long = "reconcile-deadline")]
    pub(crate) reconcile_deadline: Option<String>,
}

/// The `d2b resource` namespace also carries the generic authority read
/// projection. It never creates or mutates an authority.
#[derive(Debug, Args, Clone)]
pub(crate) struct ResourceArgs {
    #[command(subcommand)]
    pub(crate) command: ResourceCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ResourceCommand {
    Get(GenericGetArgs),
    List(GenericListArgs),
    Watch(GenericWatchArgs),
    Create(GenericCreateArgs),
    #[command(name = "update-spec")]
    UpdateSpec(GenericUpdateSpecArgs),
    Delete(GenericDeleteArgs),
    Status(GenericStatusArgs),
    Upgrade(GenericUpgradeArgs),
    Reconcile(GenericReconcileArgs),
    Authorities(AuthoritiesArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct AuthoritiesArgs {
    #[command(subcommand)]
    pub(crate) command: Option<AuthorityCommand>,
    #[arg(long)]
    pub(crate) scope: Option<String>,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum AuthorityCommand {
    Holders(AuthorityRefArgs),
    Conflict(AuthorityRefArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct AuthorityRefArgs {
    pub(crate) resource_ref: String,
}

pub(crate) fn get(
    context: &ZoneContext,
    args: &GenericGetArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let value = request_get(context, args, mode, deadline)?;
    context.emit(&value, mode)?;
    Ok(0)
}

pub(crate) fn request_get(
    context: &ZoneContext,
    args: &GenericGetArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<Value, CliFailure> {
    let resource_ref = parse_resource_ref(&args.resource_ref, None)?;
    context.invoke(
        "Get",
        json!({ "resourceRef": resource_ref.to_canonical_string() }),
        deadline,
        mode,
    )
}

pub(crate) fn list(
    context: &ZoneContext,
    args: &GenericListArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let value = request_list(context, args, mode, deadline)?;
    context.emit(&value, mode)?;
    Ok(0)
}

pub(crate) fn request_list(
    context: &ZoneContext,
    args: &GenericListArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<Value, CliFailure> {
    let resource_type = parse_resource_type(&args.resource_type)?;
    let payload = list_payload(
        resource_type.as_str(),
        args.phase.as_deref(),
        args.label_selector.as_deref(),
        args.updates,
        args.page_token.as_deref(),
        args.limit,
    )?;
    context.invoke("List", payload, deadline, mode)
}

pub(crate) fn watch(
    context: &ZoneContext,
    args: &GenericWatchArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    if !mode.is_json() {
        return Err(context.failure(
            "ref-invalid",
            "watch output is JSON-lines only",
            mode,
            2,
        ));
    }
    let resource_type = parse_resource_type(&args.resource_type)?;
    let payload = list_payload(
        resource_type.as_str(),
        args.phase.as_deref(),
        args.label_selector.as_deref(),
        false,
        None,
        None,
    )?
    .as_object()
    .cloned()
    .map(|mut object| {
        object.insert(
            "sinceRevision".to_owned(),
            args.since_revision
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        Value::Object(object)
    })
    .ok_or_else(|| CliFailure::new(1, "internal-error: invalid watch request"))?;
    let value = context.invoke("Watch", payload, deadline, mode)?;
    context.emit_stream(&value, mode)?;
    Ok(0)
}

pub(crate) fn create(
    context: &ZoneContext,
    args: &GenericCreateArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_type = parse_resource_type(&args.resource_type)?;
    let spec = read_spec(args.spec_file.as_deref(), args.spec_stdin)?;
    let value = context.invoke(
        "Create",
        json!({
            "resourceType": resource_type.as_str(),
            "spec": spec,
            "waitForReconcile": args.wait_for_reconcile,
            "reconcileDeadlineMs": args.reconcile_deadline.as_deref(),
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

pub(crate) fn update_spec(
    context: &ZoneContext,
    args: &GenericUpdateSpecArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = parse_resource_ref(&args.resource_ref, None)?;
    reject_endpoint_mutation(context, resource_ref.resource_type().as_str(), mode)?;
    let spec = read_spec(args.spec_file.as_deref(), args.spec_stdin)?;
    let value = context.invoke(
        "UpdateSpec",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "expectedRevision": args.revision,
            "spec": spec,
            "waitForReconcile": args.wait_for_reconcile,
            "reconcileDeadlineMs": args.reconcile_deadline.as_deref(),
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

pub(crate) fn delete(
    context: &ZoneContext,
    args: &GenericDeleteArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = parse_resource_ref(&args.resource_ref, None)?;
    reject_endpoint_mutation(context, resource_ref.resource_type().as_str(), mode)?;
    let value = context.invoke(
        "Delete",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "expectedRevision": args.revision,
            "waitForReconcile": args.wait_for_reconcile,
            "reconcileDeadlineMs": args.reconcile_deadline.as_deref(),
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

pub(crate) fn status(
    context: &ZoneContext,
    args: &GenericStatusArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = parse_resource_ref(&args.resource_ref, None)?;
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
        if !mode.is_json() {
            return Err(context.failure(
                "ref-invalid",
                "status --watch output is JSON-lines only",
                mode,
                2,
            ));
        }
        context.emit_stream(&value, mode)?;
    } else {
        context.emit(&value, mode)?;
    }
    Ok(0)
}

pub(crate) fn upgrade(
    context: &ZoneContext,
    args: &GenericUpgradeArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = parse_resource_ref(&args.resource_ref, None)?;
    let value = context.invoke(
        "Upgrade",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "recursive": args.recursive,
            "apply": args.apply,
            "reconcileDeadlineMs": args.reconcile_deadline.as_deref(),
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

pub(crate) fn reconcile(
    context: &ZoneContext,
    args: &GenericReconcileArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let resource_ref = parse_resource_ref(&args.resource_ref, None)?;
    let value = context.invoke(
        "Reconcile",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "reconcileDeadlineMs": args.reconcile_deadline.as_deref(),
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

pub(crate) fn typed(
    context: &ZoneContext,
    resource_type: &str,
    args: &TypedResourceArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        TypedResourceCommand::Get(args) => {
            let generic = GenericGetArgs {
                resource_ref: format!("{resource_type}/{}", args.name),
            };
            get(context, &generic, mode, deadline)
        }
        TypedResourceCommand::List(args) => {
            let generic = GenericListArgs {
                resource_type: resource_type.to_owned(),
                phase: args.phase.clone(),
                label_selector: args.label_selector.clone(),
                updates: args.updates,
                page_token: args.page_token.clone(),
                limit: args.limit,
            };
            list(context, &generic, mode, deadline)
        }
        TypedResourceCommand::Watch(args) => {
            let generic = GenericWatchArgs {
                resource_type: resource_type.to_owned(),
                since_revision: args.since_revision.clone(),
                phase: args.phase.clone(),
                label_selector: args.label_selector.clone(),
            };
            watch(context, &generic, mode, deadline)
        }
        TypedResourceCommand::Create(args) => {
            let generic = GenericCreateArgs {
                resource_type: resource_type.to_owned(),
                spec_file: args.spec_file.clone(),
                spec_stdin: args.spec_stdin,
                wait_for_reconcile: args.wait_for_reconcile,
                reconcile_deadline: args.reconcile_deadline.clone(),
            };
            create(context, &generic, mode, deadline)
        }
        TypedResourceCommand::UpdateSpec(args) => {
            let generic = GenericUpdateSpecArgs {
                resource_ref: format!("{resource_type}/{}", args.name),
                revision: args.revision.clone(),
                spec_file: args.spec_file.clone(),
                spec_stdin: args.spec_stdin,
                wait_for_reconcile: args.wait_for_reconcile,
                reconcile_deadline: args.reconcile_deadline.clone(),
            };
            if resource_type == "Endpoint" {
                return Err(context.failure(
                    "authorization-denied",
                    "controller-owned Endpoint specs are not operator-writable",
                    mode,
                    1,
                ));
            }
            update_spec(context, &generic, mode, deadline)
        }
        TypedResourceCommand::Delete(args) => {
            if resource_type == "Endpoint" {
                return Err(context.failure(
                    "authorization-denied",
                    "controller-owned Endpoints are not operator-deletable",
                    mode,
                    1,
                ));
            }
            let generic = GenericDeleteArgs {
                resource_ref: format!("{resource_type}/{}", args.name),
                revision: args.revision.clone(),
                wait_for_reconcile: args.wait_for_reconcile,
                reconcile_deadline: args.reconcile_deadline.clone(),
            };
            delete(context, &generic, mode, deadline)
        }
        TypedResourceCommand::Status(args) => {
            let generic = GenericStatusArgs {
                resource_ref: format!("{resource_type}/{}", args.name),
                watch: args.watch,
            };
            status(context, &generic, mode, deadline)
        }
        TypedResourceCommand::Upgrade(args) => {
            let generic = GenericUpgradeArgs {
                resource_ref: format!("{resource_type}/{}", args.name),
                recursive: args.recursive,
                apply: args.apply,
                reconcile_deadline: args.reconcile_deadline.clone(),
            };
            upgrade(context, &generic, mode, deadline)
        }
        TypedResourceCommand::Reconcile(args) => {
            let generic = GenericReconcileArgs {
                resource_ref: format!("{resource_type}/{}", args.name),
                reconcile_deadline: args.reconcile_deadline.clone(),
            };
            reconcile(context, &generic, mode, deadline)
        }
    }
}

pub(crate) fn run_resource(
    context: &ZoneContext,
    args: &ResourceArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        ResourceCommand::Get(args) => get(context, args, mode, deadline),
        ResourceCommand::List(args) => list(context, args, mode, deadline),
        ResourceCommand::Watch(args) => watch(context, args, mode, deadline),
        ResourceCommand::Create(args) => create(context, args, mode, deadline),
        ResourceCommand::UpdateSpec(args) => update_spec(context, args, mode, deadline),
        ResourceCommand::Delete(args) => delete(context, args, mode, deadline),
        ResourceCommand::Status(args) => status(context, args, mode, deadline),
        ResourceCommand::Upgrade(args) => upgrade(context, args, mode, deadline),
        ResourceCommand::Reconcile(args) => reconcile(context, args, mode, deadline),
        ResourceCommand::Authorities(args) => authorities(context, args, mode, deadline),
    }
}

fn authorities(
    context: &ZoneContext,
    args: &AuthoritiesArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let payload = match &args.command {
        None => json!({ "scope": args.scope }),
        Some(AuthorityCommand::Holders(args)) => {
            let resource_ref = parse_resource_ref(&args.resource_ref, None)?;
            json!({ "resourceRef": resource_ref.to_canonical_string() })
        }
        Some(AuthorityCommand::Conflict(args)) => {
            let resource_ref = parse_resource_ref(&args.resource_ref, None)?;
            json!({ "resourceRef": resource_ref.to_canonical_string() })
        }
    };
    let method = match &args.command {
        None => "ListAuthorities",
        Some(AuthorityCommand::Holders(_)) => "ListAuthorityHolders",
        Some(AuthorityCommand::Conflict(_)) => "GetAuthorityConflict",
    };
    let value = context.invoke(method, payload, deadline, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn list_payload(
    resource_type: &str,
    phase: Option<&str>,
    label_selector: Option<&str>,
    updates: bool,
    page_token: Option<&str>,
    limit: Option<u32>,
) -> Result<Value, CliFailure> {
    if let Some(selector) = label_selector {
        let (key, value) = selector
            .split_once('=')
            .ok_or_else(|| CliFailure::new(2, "label selector must use key=value"))?;
        if key.is_empty() || value.is_empty() || key.len() > 64 || value.len() > 256 {
            return Err(CliFailure::new(2, "label selector is outside its bounds"));
        }
    }
    if limit.is_some_and(|limit| limit == 0 || limit > 1024) {
        return Err(CliFailure::new(2, "list limit must be between 1 and 1024"));
    }
    Ok(json!({
        "resourceType": resource_type,
        "phase": phase,
        "labelSelector": label_selector,
        "updates": updates,
        "pageToken": page_token,
        "limit": limit,
    }))
}

fn reject_endpoint_mutation(
    context: &ZoneContext,
    resource_type: &str,
    mode: OutputMode,
) -> Result<(), CliFailure> {
    if resource_type == "Endpoint" {
        return Err(context.failure(
            "authorization-denied",
            "controller-owned Endpoints are not operator-writable",
            mode,
            1,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_limits_and_selectors_are_bounded() {
        assert!(list_payload("Guest", None, Some("env=dev"), false, None, Some(1)).is_ok());
        assert!(list_payload("Guest", None, Some("env"), false, None, None).is_err());
        assert!(list_payload("Guest", None, None, false, None, Some(1025)).is_err());
    }
}
