//! Guest and Process resource commands.

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext, parse_resource_ref},
    dispatch::{
        GenericCreateArgs, GenericDeleteArgs, GenericGetArgs, GenericListArgs,
        GenericStatusArgs, GenericUpdateSpecArgs,
    },
    resource,
};

#[derive(Debug, Args, Clone)]
pub(crate) struct GuestArgs {
    #[command(subcommand)]
    pub(crate) command: GuestCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum GuestCommand {
    Get(resource::TypedNameArgs),
    List(resource::TypedListArgs),
    Status(resource::TypedStatusArgs),
    Start(LifecycleArgs),
    Stop(LifecycleArgs),
    Restart(LifecycleArgs),
    Create(resource::TypedCreateArgs),
    #[command(name = "update-spec")]
    UpdateSpec(resource::TypedUpdateSpecArgs),
    Delete(resource::TypedNameMutationArgs),
    Console(TypedNameArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct LifecycleArgs {
    pub(crate) name: String,
    #[arg(long, conflicts_with = "apply")]
    pub(crate) dry_run: bool,
    #[arg(long, conflicts_with = "dry-run")]
    pub(crate) apply: bool,
    #[arg(long = "no-wait-ready", requires = "apply")]
    pub(crate) no_wait_ready: bool,
    #[arg(short = 'f', long)]
    pub(crate) force: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct TypedNameArgs {
    pub(crate) name: String,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ProcessArgs {
    #[command(subcommand)]
    pub(crate) command: ProcessCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ProcessCommand {
    Get(resource::TypedNameArgs),
    List(resource::TypedListArgs),
    Status(resource::TypedStatusArgs),
    Start(LifecycleArgs),
    Stop(LifecycleArgs),
    Create(resource::TypedCreateArgs),
    #[command(name = "update-spec")]
    UpdateSpec(resource::TypedUpdateSpecArgs),
    Delete(resource::TypedNameMutationArgs),
}

pub(crate) fn run_guest(
    context: &ZoneContext,
    args: &GuestArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        GuestCommand::Get(args) => {
            let generic = GenericGetArgs {
                resource_ref: format!("Guest/{}", args.name),
            };
            resource::get(context, &generic, mode, deadline)
        }
        GuestCommand::List(args) => {
            let generic = GenericListArgs {
                resource_type: "Guest".to_owned(),
                phase: args.phase.clone(),
                label_selector: args.label_selector.clone(),
                updates: args.updates,
                page_token: args.page_token.clone(),
                limit: args.limit,
            };
            let value = resource::request_list(context, &generic, mode, deadline)?;
            let value = filter_unsafe_local(value);
            context.emit(&value, mode)?;
            Ok(0)
        }
        GuestCommand::Status(args) => {
            let generic = GenericStatusArgs {
                resource_ref: format!("Guest/{}", args.name),
                watch: args.watch,
            };
            resource::status(context, &generic, mode, deadline)
        }
        GuestCommand::Start(args) => lifecycle(context, "start", args, mode, deadline),
        GuestCommand::Stop(args) => lifecycle(context, "stop", args, mode, deadline),
        GuestCommand::Restart(args) => lifecycle(context, "restart", args, mode, deadline),
        GuestCommand::Create(args) => {
            let generic = GenericCreateArgs {
                resource_type: "Guest".to_owned(),
                spec_file: args.spec_file.clone(),
                spec_stdin: args.spec_stdin,
                wait_for_reconcile: args.wait_for_reconcile,
                reconcile_deadline: args.reconcile_deadline.clone(),
            };
            resource::create(context, &generic, mode, deadline)
        }
        GuestCommand::UpdateSpec(args) => {
            let generic = GenericUpdateSpecArgs {
                resource_ref: format!("Guest/{}", args.name),
                revision: args.revision.clone(),
                spec_file: args.spec_file.clone(),
                spec_stdin: args.spec_stdin,
                wait_for_reconcile: args.wait_for_reconcile,
                reconcile_deadline: args.reconcile_deadline.clone(),
            };
            resource::update_spec(context, &generic, mode, deadline)
        }
        GuestCommand::Delete(args) => {
            let generic = crate::dispatch::GenericDeleteArgs {
                resource_ref: format!("Guest/{}", args.name),
                revision: args.revision.clone(),
                wait_for_reconcile: args.wait_for_reconcile,
                reconcile_deadline: args.reconcile_deadline.clone(),
            };
            resource::delete(context, &generic, mode, deadline)
        }
        GuestCommand::Console(args) => {
            let resource_ref = parse_resource_ref(&format!("Guest/{}", args.name), None)?;
            let value = context.invoke(
                "Console",
                json!({ "resourceRef": resource_ref.to_canonical_string() }),
                deadline,
                mode,
            )?;
            context.emit(&value, mode)?;
            Ok(0)
        }
    }
}

pub(crate) fn run_process(
    context: &ZoneContext,
    args: &ProcessArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        ProcessCommand::Get(args) => {
            let generic = GenericGetArgs {
                resource_ref: format!("Process/{}", args.name),
            };
            resource::get(context, &generic, mode, deadline)
        }
        ProcessCommand::List(args) => {
            let generic = GenericListArgs {
                resource_type: "Process".to_owned(),
                phase: args.phase.clone(),
                label_selector: args.label_selector.clone(),
                updates: args.updates,
                page_token: args.page_token.clone(),
                limit: args.limit,
            };
            resource::list(context, &generic, mode, deadline)
        }
        ProcessCommand::Status(args) => {
            let generic = GenericStatusArgs {
                resource_ref: format!("Process/{}", args.name),
                watch: args.watch,
            };
            resource::status(context, &generic, mode, deadline)
        }
        ProcessCommand::Start(args) => lifecycle(context, "start", args, mode, deadline),
        ProcessCommand::Stop(args) => lifecycle(context, "stop", args, mode, deadline),
        ProcessCommand::Create(args) => {
            let generic = GenericCreateArgs {
                resource_type: "Process".to_owned(),
                spec_file: args.spec_file.clone(),
                spec_stdin: args.spec_stdin,
                wait_for_reconcile: args.wait_for_reconcile,
                reconcile_deadline: args.reconcile_deadline.clone(),
            };
            resource::create(context, &generic, mode, deadline)
        }
        ProcessCommand::UpdateSpec(args) => {
            let generic = GenericUpdateSpecArgs {
                resource_ref: format!("Process/{}", args.name),
                revision: args.revision.clone(),
                spec_file: args.spec_file.clone(),
                spec_stdin: args.spec_stdin,
                wait_for_reconcile: args.wait_for_reconcile,
                reconcile_deadline: args.reconcile_deadline.clone(),
            };
            resource::update_spec(context, &generic, mode, deadline)
        }
        ProcessCommand::Delete(args) => {
            let generic = crate::dispatch::GenericDeleteArgs {
                resource_ref: format!("Process/{}", args.name),
                revision: args.revision.clone(),
                wait_for_reconcile: args.wait_for_reconcile,
                reconcile_deadline: args.reconcile_deadline.clone(),
            };
            resource::delete(context, &generic, mode, deadline)
        }
    }
}

fn lifecycle(
    context: &ZoneContext,
    action: &str,
    args: &LifecycleArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    if !args.dry_run && !args.apply {
        return Err(context.failure(
            "ref-invalid",
            "lifecycle commands require --dry-run or --apply",
            mode,
            2,
        ));
    }
    let resource_ref = parse_resource_ref(&format!("Guest/{}", args.name), None)?;
    let value = context.invoke(
        "UpdateSpec",
        json!({
            "resourceRef": resource_ref.to_canonical_string(),
            "lifecycle": action,
            "force": args.force,
            "dryRun": args.dry_run,
            "apply": args.apply,
            "waitForReady": !args.no_wait_ready,
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn filter_unsafe_local(mut value: Value) -> Value {
    if let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) {
        items.retain(|item| {
            let posture = item
                .pointer("/status/isolationPosture")
                .and_then(Value::as_str)
                .or_else(|| item.get("isolationPosture").and_then(Value::as_str));
            let provider = item
                .pointer("/spec/providerRef")
                .and_then(Value::as_str)
                .or_else(|| item.get("providerRef").and_then(Value::as_str));
            posture != Some("none") && provider != Some("Provider/unsafe-local")
        });
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_local_entries_never_appear_in_guest_lists() {
        let filtered = filter_unsafe_local(json!({
            "items": [
                {"resourceRef":"Guest/work", "status":{"phase":"Ready"}},
                {"resourceRef":"Host/alice", "status":{"isolationPosture":"none"}},
                {"resourceRef":"Guest/legacy", "spec":{"providerRef":"Provider/unsafe-local"}}
            ]
        }));
        assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["items"][0]["resourceRef"], "Guest/work");
    }
}
