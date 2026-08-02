//! Host ResourceType and host-maintenance commands.

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext},
    dispatch::{GenericGetArgs, GenericListArgs},
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
    Reconcile(HostReconcileArgs),
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
    #[arg(long, conflicts_with = "dry_run")]
    pub(crate) apply: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct HostDoctorArgs {
    #[arg(long)]
    pub(crate) read_only: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct HostInstallArgs {
    #[arg(long, conflicts_with_all = ["apply", "enable", "start", "no_start"])]
    pub(crate) dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    pub(crate) apply: bool,
    #[arg(long, requires = "apply", conflicts_with = "dry_run")]
    pub(crate) enable: bool,
    #[arg(long, requires = "apply", conflicts_with_all = ["dry_run", "no_start"])]
    pub(crate) start: bool,
    #[arg(long, requires = "apply", conflicts_with_all = ["dry_run", "start"])]
    pub(crate) no_start: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct HostValidateArgs {
    #[arg(long, conflicts_with = "apply")]
    pub(crate) dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
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

#[derive(Debug, Args, Clone)]
pub(crate) struct HostReconcileArgs {
    #[arg(long)]
    pub(crate) network: bool,
    #[arg(long, conflicts_with = "apply")]
    pub(crate) dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    pub(crate) apply: bool,
}

pub(crate) fn run(
    context: &ZoneContext,
    args: &HostArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        HostCommand::Get(args) => {
            let value = resource::request_get(
                context,
                &GenericGetArgs {
                    resource_ref: format!("Host/{}", args.name),
                },
                mode,
                deadline,
            )?;
            context.emit(&ensure_host_posture(value), mode)?;
            Ok(0)
        }
        HostCommand::List(args) => {
            let value = resource::request_list(
                context,
                &GenericListArgs {
                    resource_type: "Host".to_owned(),
                    execution_ref: None,
                    domain: None,
                    phase: args.phase.clone(),
                    label_selector: args.label_selector.clone(),
                    updates: args.updates,
                    page_token: args.page_token.clone(),
                    limit: args.limit,
                },
                mode,
                deadline,
            )?;
            context.emit(&ensure_host_posture(value), mode)?;
            Ok(0)
        }
        HostCommand::Status(args) => {
            if args.watch && !mode.is_json() {
                return Err(context.failure(
                    "ref-invalid",
                    "host status --watch output is JSON-lines only",
                    mode,
                    2,
                ));
            }
            let resource_ref =
                crate::context::parse_resource_ref(&format!("Host/{}", args.name), None)?;
            let value = context.invoke(
                "Status",
                json!({
                    "resourceRef": resource_ref.to_canonical_string(),
                    "watch": args.watch,
                }),
                deadline,
                mode,
            )?;
            let value = ensure_host_posture(value);
            if args.watch {
                context.emit_stream(&value, mode)?;
            } else {
                context.emit(&value, mode)?;
            }
            Ok(0)
        }
        HostCommand::Check(args) => local_host_check(args, mode),
        HostCommand::Prepare(args) => mutation(context, "prepare", args, mode, deadline),
        HostCommand::Destroy(args) => mutation(context, "destroy", args, mode, deadline),
        HostCommand::Doctor(args) => {
            if !args.read_only {
                // Keep the established command-local refusal envelope. This
                // helper only renders the mandatory-flag error; it does not
                // open a daemon, broker, SSH, or executable fallback path.
                let legacy = crate::LegacyContext::from_env()?;
                return crate::cmd_host_doctor(
                    &legacy,
                    &crate::HostDoctorArgs {
                        read_only: false,
                        json: mode.is_json(),
                        human: !mode.is_json(),
                    },
                );
            }
            let value = match context.invoke(
                "HostDoctor",
                json!({ "readOnly": args.read_only }),
                ZoneContext::deadline(Some("250ms"))?,
                mode,
            ) {
                Ok(value) => value,
                Err(error) if can_fallback_to_local_state(&error) => {
                    return local_doctor(args, mode);
                }
                Err(error) => return Err(error),
            };
            context.emit(&value, mode)?;
            Ok(0)
        }
        HostCommand::Install(args) => install(context, args, mode, deadline),
        HostCommand::Validate(args) => {
            let legacy = crate::LegacyContext::from_env()?;
            crate::cmd_host_validate(
                &legacy,
                &crate::HostValidateArgs {
                    dry_run: args.dry_run,
                    apply: args.apply,
                    wave: args.wave.clone(),
                    operator_signature: args.operator_signature.clone(),
                    evidence_dir: args.evidence_dir.clone(),
                    scripts_dir: args.scripts_dir.clone(),
                    json: mode.is_json(),
                    human: !mode.is_json(),
                },
            )
        }
        HostCommand::Reconcile(args) => reconcile(context, args, mode, deadline),
    }
}

fn can_fallback_to_local_state(error: &CliFailure) -> bool {
    matches!(
        error.message.split(':').next(),
        Some("zone-unavailable" | "deadline-exceeded" | "exec-protocol-error")
    )
}

fn local_doctor(args: &HostDoctorArgs, mode: OutputMode) -> Result<i32, CliFailure> {
    let context = crate::LegacyContext::from_env()?;
    crate::cmd_host_doctor(
        &context,
        &crate::HostDoctorArgs {
            read_only: args.read_only,
            json: mode.is_json(),
            human: !mode.is_json(),
        },
    )
}

fn ensure_host_posture(mut value: Value) -> Value {
    if value.get("items").and_then(Value::as_array).is_some() {
        if let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) {
            for item in items {
                mark_unsafe_local_host(item);
            }
        }
    } else {
        mark_unsafe_local_host(&mut value);
    }
    value
}

fn mark_unsafe_local_host(value: &mut Value) {
    let Value::Object(object) = value else {
        return;
    };
    let resource_ref = object
        .get("resourceRef")
        .and_then(Value::as_str)
        .or_else(|| object.get("type").and_then(Value::as_str));
    let provider = object
        .get("spec")
        .and_then(Value::as_object)
        .and_then(|spec| spec.get("providerRef"))
        .and_then(Value::as_str)
        .or_else(|| object.get("providerRef").and_then(Value::as_str))
        .or_else(|| {
            object
                .get("status")
                .and_then(Value::as_object)
                .and_then(|status| status.get("providerRef"))
                .and_then(Value::as_str)
        });
    let provider_kind = object
        .get("status")
        .and_then(Value::as_object)
        .and_then(|status| status.get("providerKind"))
        .and_then(Value::as_str)
        .or_else(|| object.get("providerKind").and_then(Value::as_str));
    let is_host = resource_ref.is_some_and(|value| value == "Host" || value.starts_with("Host/"));
    let is_unsafe_local = provider == Some("Provider/unsafe-local")
        || provider_kind == Some("unsafe-local")
        || object
            .get("status")
            .and_then(Value::as_object)
            .and_then(|status| status.get("isolationPosture"))
            .and_then(Value::as_str)
            .or_else(|| object.get("isolationPosture").and_then(Value::as_str))
            .is_some_and(|value| matches!(value, "none" | "unsafe-local"));
    if !(is_host && is_unsafe_local) {
        return;
    }
    object.insert(
        "isolationPosture".to_owned(),
        Value::String("none".to_owned()),
    );
    let status = object
        .entry("status".to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(status) = status {
        status.insert(
            "isolationPosture".to_owned(),
            Value::String("none".to_owned()),
        );
    }
}

fn local_host_check(args: &HostCheckArgs, mode: OutputMode) -> Result<i32, CliFailure> {
    if args.strict && !args.read_only {
        return Err(CliFailure::new(
            3,
            "ref-invalid: host check --strict requires --read-only",
        ));
    }
    let context = crate::LegacyContext::from_env()?;
    crate::cmd_host_check(
        &context,
        &crate::HostCheckArgs {
            read_only: args.read_only,
            strict: args.strict,
            json: mode.is_json(),
            human: !mode.is_json(),
        },
    )
}

fn mutation(
    context: &ZoneContext,
    operation: &str,
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
        "Reconcile",
        json!({
            "resourceRef": "Host/system",
            "operation": operation,
            "dryRun": args.dry_run,
            "apply": args.apply,
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn install(
    context: &ZoneContext,
    args: &HostInstallArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    if !args.dry_run && !args.apply {
        return Err(context.failure(
            "ref-invalid",
            "host install requires --dry-run or --apply",
            mode,
            78,
        ));
    }

    if args.apply {
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
        return Ok(0);
    }

    let value = json!({
        "command": "host install",
        "mode": "dry-run",
        "notes": "dry-run preview; --apply routes through the daemon → broker RunHostInstall path.",
        "planned_steps": [
            {
                "step": 1,
                "what": "place systemd units at /etc/systemd/system/d2bd.service + d2b-priv-broker.socket"
            },
            {
                "step": 2,
                "what": "write daemon-config.json to /etc/d2b/daemon-config.json with paths matching the daemon's compiled-in defaults"
            },
            {
                "step": 3,
                "what": "bind /run/d2b/public.sock + /run/d2b/priv.sock with socket ACLs (launcher / admin groups)"
            },
            {
                "step": 4,
                "what": if args.enable && args.start {
                    "systemctl enable --now d2bd.service"
                } else if args.enable {
                    "systemctl enable d2bd.service"
                } else if args.no_start {
                    "do NOT enable; operator starts manually"
                } else {
                    "neither --enable nor --start specified: leave service inactive"
                }
            },
            {
                "step": 5,
                "what": "smoke: d2b auth status against /run/d2b/public.sock"
            }
        ]
    });
    if mode.is_json() {
        context.emit(&value, mode)?;
    } else {
        crate::print_stdout(
            "host install --dry-run: would install d2bd at /etc/systemd/system/ and bind /run/d2b/public.sock (the live --apply path routes through the daemon → broker RunHostInstall path)\n",
        );
    }
    Ok(0)
}

fn reconcile(
    context: &ZoneContext,
    args: &HostReconcileArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    if !args.dry_run && !args.apply {
        return Err(context.failure(
            "ref-invalid",
            "host reconcile requires --dry-run or --apply",
            mode,
            78,
        ));
    }
    if !args.network {
        return Err(context.failure("ref-invalid", "host reconcile requires --network", mode, 78));
    }

    let value = context.invoke(
        "Reconcile",
        json!({
            "resourceRef": "Host/system",
            "operation": "reconcile",
            "network": args.network,
            "dryRun": args.dry_run,
            "apply": args.apply,
        }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}
