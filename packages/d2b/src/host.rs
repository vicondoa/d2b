//! Host ResourceType and host-maintenance commands.

use clap::{Args, Subcommand};
use serde_json::json;

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext},
    dispatch::{
        GenericGetArgs, GenericListArgs, GenericStatusArgs,
    },
    resource,
};

#[derive(Debug, Args, Clone)]
pub(crate) struct HostArgs {
    #[command(subcommand)]
    pub(crate) command: HostCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum HostCommand {
    Get(resource::TypedNameArgs),
    List(resource::TypedListArgs),
    Status(resource::TypedStatusArgs),
    Check(HostCheckArgs),
    Prepare(HostMutationArgs),
    Destroy(HostMutationArgs),
    Doctor(HostDoctorArgs),
    Install(HostInstallArgs),
    Reconcile(HostMutationArgs),
    Validate(HostValidateArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct HostCheckArgs {
    #[arg(long)]
    pub(crate) read_only: bool,
    #[arg(long)]
    pub(crate) strict: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct HostMutationArgs {
    #[arg(long, conflicts_with = "apply")]
    pub(crate) dry_run: bool,
    #[arg(long, conflicts_with = "dry-run")]
    pub(crate) apply: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct HostDoctorArgs {
    #[arg(long)]
    pub(crate) read_only: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct HostInstallArgs {
    #[arg(long, conflicts_with_all = ["apply", "enable", "start", "no-start"])]
    pub(crate) dry_run: bool,
    #[arg(long, conflicts_with = "dry-run")]
    pub(crate) apply: bool,
    #[arg(long, requires = "apply", conflicts_with = "dry-run")]
    pub(crate) enable: bool,
    #[arg(long, requires = "apply", conflicts_with_all = ["dry-run", "no-start"])]
    pub(crate) start: bool,
    #[arg(long, requires = "apply", conflicts_with_all = ["dry-run", "start"])]
    pub(crate) no_start: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct HostValidateArgs {
    #[arg(long, conflicts_with = "apply")]
    pub(crate) dry_run: bool,
    #[arg(long, conflicts_with = "dry-run")]
    pub(crate) apply: bool,
    #[arg(long)]
    pub(crate) wave: Option<String>,
    #[arg(long = "evidence-dir")]
    pub(crate) evidence_dir: Option<std::path::PathBuf>,
    #[arg(long = "scripts-dir")]
    pub(crate) scripts_dir: Option<std::path::PathBuf>,
    #[arg(long = "operator-signature")]
    pub(crate) operator_signature: Option<String>,
}

pub(crate) fn run(
    context: &ZoneContext,
    args: &HostArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        HostCommand::Get(args) => resource::get(
            context,
            &GenericGetArgs {
                resource_ref: format!("Host/{}", args.name),
            },
            mode,
            deadline,
        ),
        HostCommand::List(args) => resource::list(
            context,
            &GenericListArgs {
                resource_type: "Host".to_owned(),
                phase: args.phase.clone(),
                label_selector: args.label_selector.clone(),
                updates: args.updates,
                page_token: args.page_token.clone(),
                limit: args.limit,
            },
            mode,
            deadline,
        ),
        HostCommand::Status(args) => resource::status(
            context,
            &GenericStatusArgs {
                resource_ref: format!("Host/{}", args.name),
                watch: args.watch,
            },
            mode,
            deadline,
        ),
        HostCommand::Check(args) => {
            if args.strict && !args.read_only {
                return Err(context.failure(
                    "ref-invalid",
                    "host check --strict requires --read-only",
                    mode,
                    3,
                ));
            }
            let value = context.invoke(
                "HostCheck",
                json!({ "readOnly": args.read_only, "strict": args.strict }),
                deadline,
                mode,
            )?;
            context.emit(&value, mode)?;
            Ok(0)
        }
        HostCommand::Prepare(args) => mutation(context, "HostPrepare", args, mode, deadline),
        HostCommand::Destroy(args) => mutation(context, "HostDestroy", args, mode, deadline),
        HostCommand::Reconcile(args) => mutation(context, "HostReconcile", args, mode, deadline),
        HostCommand::Doctor(args) => {
            let value = match context.invoke(
                "HostDoctor",
                json!({ "readOnly": args.read_only }),
                deadline,
                mode,
            ) {
                Ok(value) => value,
                Err(error) if error.exit_code == 1 => json!({
                    "ok": true,
                    "zoneRef": context.zone_ref(),
                    "schemaVersion": 1,
                    "degraded": true,
                    "source": "local-state",
                    "message": "Zone runtime unavailable; local host diagnostics are incomplete"
                }),
                Err(error) => return Err(error),
            };
            context.emit(&value, mode)?;
            Ok(0)
        }
        HostCommand::Install(args) => {
            if !args.dry_run && !args.apply {
                return Err(context.failure(
                    "ref-invalid",
                    "host install requires --dry-run or --apply",
                    mode,
                    2,
                ));
            }
            let value = context.invoke(
                "HostInstall",
                json!({
                    "dryRun": args.dry_run,
                    "apply": args.apply,
                    "enable": args.enable,
                    "start": args.start,
                    "noStart": args.no_start,
                }),
                deadline,
                mode,
            )?;
            context.emit(&value, mode)?;
            Ok(0)
        }
        HostCommand::Validate(args) => {
            if !args.dry_run && !args.apply {
                return Err(context.failure(
                    "ref-invalid",
                    "host validate requires --dry-run or --apply",
                    mode,
                    2,
                ));
            }
            let value = context.invoke(
                "HostValidate",
                json!({
                    "dryRun": args.dry_run,
                    "apply": args.apply,
                    "wave": args.wave,
                    "evidenceDir": args.evidence_dir.as_ref().map(|_| "<configured>"),
                    "scriptsDir": args.scripts_dir.as_ref().map(|_| "<configured>"),
                    "operatorSignature": args.operator_signature.as_ref().map(|_| "<provided>"),
                }),
                deadline,
                mode,
            )?;
            context.emit(&value, mode)?;
            Ok(0)
        }
    }
}

fn mutation(
    context: &ZoneContext,
    method: &str,
    args: &HostMutationArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    if !args.dry_run && !args.apply {
        return Err(context.failure(
            "ref-invalid",
            "host mutation requires --dry-run or --apply",
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

