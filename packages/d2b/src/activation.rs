//! Activation-NixOS Provider command namespace.

use clap::{Args, Subcommand};
use serde_json::json;

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext, parse_resource_ref},
};

#[derive(Debug, Args, Clone)]
pub(crate) struct ActivationArgs {
    #[command(subcommand)]
    pub(crate) command: ActivationCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ActivationCommand {
    Build(GuestRefArgs),
    Generations(GuestRefArgs),
    Switch(ActivationTargetArgs),
    Boot(ActivationTargetArgs),
    Test(ActivationTargetArgs),
    Rollback(ActivationTargetArgs),
    Gc(ActivationMutationArgs),
    Migrate(ActivationMutationArgs),
    Keys(ActivationKeysArgs),
    Trust(ActivationNameArgs),
    #[command(name = "rotate-known-host")]
    RotateKnownHost(ActivationNameArgs),
    Config(ActivationConfigArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct GuestRefArgs {
    pub(crate) guest_ref: String,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ActivationTargetArgs {
    pub(crate) guest_ref: String,
    #[arg(long, conflicts_with = "apply")]
    pub(crate) dry_run: bool,
    #[arg(long, conflicts_with = "dry-run")]
    pub(crate) apply: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ActivationMutationArgs {
    #[arg(long, conflicts_with = "apply")]
    pub(crate) dry_run: bool,
    #[arg(long, conflicts_with = "dry-run")]
    pub(crate) apply: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ActivationKeysArgs {
    #[command(subcommand)]
    pub(crate) command: ActivationKeysCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ActivationKeysCommand {
    List,
    Show(ActivationNameArgs),
    Rotate(ActivationTargetArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ActivationNameArgs {
    pub(crate) name: String,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ActivationConfigArgs {
    #[command(subcommand)]
    pub(crate) command: ActivationConfigCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ActivationConfigCommand {
    Sync(ActivationConfigTargetArgs),
    Diff(ActivationConfigDiffArgs),
    Approve(ActivationConfigApproveArgs),
    Reject(ActivationConfigTargetArgs),
    Status(ActivationConfigTargetArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ActivationConfigTargetArgs {
    pub(crate) guest_ref: String,
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ActivationConfigDiffArgs {
    pub(crate) guest_ref: String,
    #[arg(long)]
    pub(crate) against: std::path::PathBuf,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ActivationConfigApproveArgs {
    pub(crate) guest_ref: String,
    #[arg(long = "to")]
    pub(crate) destination: std::path::PathBuf,
}

pub(crate) fn run(
    context: &ZoneContext,
    args: &ActivationArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        ActivationCommand::Build(args) => guest_call(context, "Build", args, mode, deadline),
        ActivationCommand::Generations(args) => {
            guest_call(context, "Generations", args, mode, deadline)
        }
        ActivationCommand::Switch(args) => target_call(context, "Switch", args, mode, deadline),
        ActivationCommand::Boot(args) => target_call(context, "Boot", args, mode, deadline),
        ActivationCommand::Test(args) => target_call(context, "Test", args, mode, deadline),
        ActivationCommand::Rollback(args) => {
            target_call(context, "Rollback", args, mode, deadline)
        }
        ActivationCommand::Gc(args) => mutation_call(context, "Gc", args, mode, deadline),
        ActivationCommand::Migrate(args) => mutation_call(context, "Migrate", args, mode, deadline),
        ActivationCommand::Keys(args) => keys(context, args, mode, deadline),
        ActivationCommand::Trust(args) => named_call(context, "Trust", &args.name, mode, deadline),
        ActivationCommand::RotateKnownHost(args) => {
            named_call(context, "RotateKnownHost", &args.name, mode, deadline)
        }
        ActivationCommand::Config(args) => config(context, args, mode, deadline),
    }
}

fn guest_call(
    context: &ZoneContext,
    method: &str,
    args: &GuestRefArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let guest_ref = parse_guest_ref(context, &args.guest_ref, mode)?;
    let value = context.invoke(
        method,
        json!({ "resourceRef": guest_ref.to_canonical_string() }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn target_call(
    context: &ZoneContext,
    method: &str,
    args: &ActivationTargetArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    if !args.dry_run && !args.apply {
        return Err(context.failure(
            "ref-invalid",
            "activation mutation requires --dry-run or --apply",
            mode,
            2,
        ));
    }
    let guest_ref = parse_guest_ref(context, &args.guest_ref, mode)?;
    let value = context.invoke(
        method,
        json!({
            "resourceRef": guest_ref.to_canonical_string(),
            "dryRun": args.dry_run,
            "apply": args.apply,
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn mutation_call(
    context: &ZoneContext,
    method: &str,
    args: &ActivationMutationArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    if !args.dry_run && !args.apply {
        return Err(context.failure(
            "ref-invalid",
            "activation mutation requires --dry-run or --apply",
            mode,
            2,
        ));
    }
    let value = context.invoke(
        method,
        json!({ "dryRun": args.dry_run, "apply": args.apply }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn keys(
    context: &ZoneContext,
    args: &ActivationKeysArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let (method, payload) = match &args.command {
        ActivationKeysCommand::List => ("KeysList", json!({})),
        ActivationKeysCommand::Show(args) => ("KeysShow", json!({ "name": args.name })),
        ActivationKeysCommand::Rotate(args) => {
            if !args.dry_run && !args.apply {
                return Err(context.failure(
                    "ref-invalid",
                    "activation keys rotate requires --dry-run or --apply",
                    mode,
                    2,
                ));
            }
            (
                "KeysRotate",
                json!({
                    "name": args.guest_ref,
                    "dryRun": args.dry_run,
                    "apply": args.apply
                }),
            )
        }
    };
    let value = context.invoke(method, payload, deadline, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn named_call(
    context: &ZoneContext,
    method: &str,
    name: &str,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let value = context.invoke(method, json!({ "name": name }), deadline, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn config(
    context: &ZoneContext,
    args: &ActivationConfigArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let (method, payload) = match &args.command {
        ActivationConfigCommand::Sync(args) => (
            "ConfigSync",
            json!({
                "guestRef": parse_guest_ref(context, &args.guest_ref, mode)?.to_canonical_string(),
                "dryRun": args.dry_run,
            }),
        ),
        ActivationConfigCommand::Diff(args) => (
            "ConfigDiff",
            json!({
                "guestRef": parse_guest_ref(context, &args.guest_ref, mode)?.to_canonical_string(),
                "against": "<configured>",
            }),
        ),
        ActivationConfigCommand::Approve(args) => (
            "ConfigApprove",
            json!({
                "guestRef": parse_guest_ref(context, &args.guest_ref, mode)?.to_canonical_string(),
                "to": "<configured>",
            }),
        ),
        ActivationConfigCommand::Reject(args) => (
            "ConfigReject",
            json!({
                "guestRef": parse_guest_ref(context, &args.guest_ref, mode)?.to_canonical_string(),
            }),
        ),
        ActivationConfigCommand::Status(args) => (
            "ConfigStatus",
            json!({
                "guestRef": parse_guest_ref(context, &args.guest_ref, mode)?.to_canonical_string(),
            }),
        ),
    };
    let value = context.invoke(method, payload, deadline, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn parse_guest_ref(
    context: &ZoneContext,
    value: &str,
    mode: OutputMode,
) -> Result<d2b_contracts::v3::ResourceRef, CliFailure> {
    let resource_ref = parse_resource_ref(value, Some("Guest"))?;
    if resource_ref.resource_type().as_str() != "Guest" {
        return Err(context.failure(
            "ref-invalid",
            "activation commands require a Guest ResourceRef",
            mode,
            2,
        ));
    }
    Ok(resource_ref)
}

