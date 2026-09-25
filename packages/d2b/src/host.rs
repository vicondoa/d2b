//! Host ResourceType and host-maintenance commands.

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::{
    CliFailure,
    context::{CliContext, OutputMode, RequestDeadline, ZoneContext},
    dispatch::{GenericGetArgs, GenericListArgs},
    dispatch::{emit_host_error, host_error_envelope, missing_mutation_flag_envelope},
    doctor, host_validate, print_json, print_stdout, resource,
};

#[derive(Debug, Args, Clone)]
pub(crate) struct HostArgs {
    #[command(subcommand)]
    pub(crate) command: HostCommand,
}

#[derive(Debug, Subcommand, Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum HostCommand {
    Get(resource::TypedNameArgs),
    List(resource::TypedListArgs),
    Status(resource::TypedStatusArgs),
    Prepare(HostMutationArgs),
    Destroy(HostMutationArgs),
    Doctor(HostDoctorArgs),
    Reconcile(HostReconcileArgs),
    Validate(HostValidateArgs),
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
        HostCommand::Prepare(args) => mutation(context, "prepare", args, mode, deadline),
        HostCommand::Destroy(args) => mutation(context, "destroy", args, mode, deadline),
        HostCommand::Doctor(args) => {
            if !args.read_only {
                return emit_host_error(
                    &host_error_envelope(
                        "host doctor requires the explicit --read-only flag",
                        "--read-only-required",
                        78,
                        "host doctor invocation flags.",
                        "--read-only flag missing",
                        "Re-run as `d2b host doctor --read-only`. The doctor verb is read-only; mutation forms are future deliverables.",
                        "docs/reference/error-codes.md#--read-only-required",
                    ),
                    mode.is_json(),
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
        HostCommand::Validate(args) => validate(args, mode),
        HostCommand::Reconcile(args) => reconcile(context, args, mode, deadline),
    }
}

fn can_fallback_to_local_state(error: &CliFailure) -> bool {
    matches!(
        error.code.as_str(),
        "zone-unavailable" | "deadline-exceeded" | "exec-protocol-error"
    )
}

fn local_doctor(_args: &HostDoctorArgs, mode: OutputMode) -> Result<i32, CliFailure> {
    let context = CliContext::from_env()?;
    let report = doctor::run_doctor(&context);
    if mode.is_json() {
        print_json(&doctor::render_summary(&report))?;
    } else {
        print_stdout(&doctor::render_human(&report));
    }
    Ok(report.exit_code())
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
    let is_host = resource_ref.is_some_and(|value| {
        let resource_type = value
            .split_once('/')
            .map_or(value, |(resource_type, _)| resource_type);
        crate::generated::surface_catalog::is_no_isolation_target(resource_type)
    });
    let is_unsafe_local = provider.is_some_and(crate::generated::surface_catalog::is_unsafe_local_provider)
        || provider_kind.is_some_and(crate::generated::surface_catalog::is_unsafe_local_provider_kind)
        || object
            .get("status")
            .and_then(Value::as_object)
            .and_then(|status| status.get("isolationPosture"))
            .and_then(Value::as_str)
            .or_else(|| object.get("isolationPosture").and_then(Value::as_str))
            .is_some_and(crate::generated::surface_catalog::is_no_isolation_posture);
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

fn validate(args: &HostValidateArgs, mode: OutputMode) -> Result<i32, CliFailure> {
    if !args.dry_run && !args.apply {
        return emit_host_error(
            &missing_mutation_flag_envelope("host validate"),
            mode.is_json(),
        );
    }
    let validation_mode = if args.apply {
        host_validate::ValidateMode::Apply
    } else {
        host_validate::ValidateMode::DryRun
    };
    let mut request = host_validate::ValidateRequest::from_env_defaults(validation_mode);
    if let Some(directory) = &args.evidence_dir {
        request.evidence_dir = directory.clone();
    }
    if let Some(directory) = &args.scripts_dir {
        request.scripts_dir = directory.clone();
    }
    if let Some(wave) = &args.wave {
        request.only_wave = Some(wave.clone());
    }
    if let Some(signature) = &args.operator_signature {
        request.operator_signature = Some(signature.clone());
    }

    if let Some(only_wave) = &request.only_wave {
        let known = host_validate::WAVE_CATALOG
            .iter()
            .any(|spec| spec.wave == only_wave);
        if !known {
            let known_list: Vec<&str> = host_validate::WAVE_CATALOG
                .iter()
                .map(|spec| spec.wave)
                .collect();
            return emit_host_error(
                &host_error_envelope(
                    "host validate --wave value is not a known readiness wave",
                    "unknown-wave",
                    78,
                    "host validate --wave argument.",
                    format!("--wave {only_wave} is not in the readiness-wave catalog"),
                    format!(
                        "Re-run with one of: {}. The catalog mirrors readinessWaveSpecs in nixos-modules/options-daemon.nix.",
                        known_list.join(", ")
                    ),
                    "docs/reference/host-validate.md#waves",
                ),
                mode.is_json(),
            );
        }
    }

    let report = host_validate::run_host_validate(&request);
    let exit_code = host_validate::exit_code(&report);
    if mode.is_json() {
        let mut rendered = serde_json::to_string_pretty(&host_validate::render_summary(&report))
            .map_err(|error| {
                CliFailure::new(
                    1,
                    format!("failed to serialize host validate summary: {error}"),
                )
            })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&host_validate::render_human(&report));
    }
    Ok(exit_code)
}
