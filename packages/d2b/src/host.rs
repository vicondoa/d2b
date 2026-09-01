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
use d2b_contracts_control::cli_output::{
    HostCheckFindingV2, HostCheckOutputV2, HostCheckSeverityV2, HostCheckSummaryV2,
};
use d2b_core::host_check;

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
        HostCommand::Install(args) => install(context, args, mode, deadline),
        HostCommand::Validate(args) => validate(args, mode),
        HostCommand::Reconcile(args) => reconcile(context, args, mode, deadline),
    }
}

fn can_fallback_to_local_state(error: &CliFailure) -> bool {
    matches!(
        error.message.split(':').next(),
        Some("zone-unavailable" | "deadline-exceeded" | "exec-protocol-error")
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
    let context = CliContext::from_env()?;
    let bundle = context.load_bundle_context()?.ok_or_else(|| {
        CliFailure::new(
            1,
            format!(
                "{} is required for host check",
                context.bundle_path.display()
            ),
        )
    })?;
    let host = bundle
        .host
        .as_ref()
        .ok_or_else(|| CliFailure::new(1, "bundle did not include host.json"))?;
    let report = host_check::run(host, bundle.closures.values(), args.strict)
        .map_err(CliFailure::host_check_probe_error)?;
    let output = map_host_check_report(report);
    if mode.is_json() {
        print_json(&output)?;
    } else {
        print_stdout(&render_host_check_human(&output));
    }
    Ok(i32::from(output.exit_code))
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
                "what": "place systemd units at /etc/systemd/system/d2bd.service + d2b-broker.socket"
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
                    &format!("--wave {only_wave} is not in the readiness-wave catalog"),
                    &format!(
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

pub(crate) fn map_host_check_report(report: host_check::HostCheckReport) -> HostCheckOutputV2 {
    HostCheckOutputV2 {
        mode: "read-only".to_owned(),
        strict: report.strict,
        summary: HostCheckSummaryV2 {
            pass: report.summary.pass,
            warn: report.summary.warn,
            fail: report.summary.fail,
        },
        exit_code: report.exit_code(),
        findings: report
            .findings
            .into_iter()
            .map(map_host_check_finding)
            .collect(),
    }
}

fn map_host_check_finding(finding: host_check::HostCheckFinding) -> HostCheckFindingV2 {
    HostCheckFindingV2 {
        id: finding.id,
        severity: map_host_check_severity(finding.severity),
        message: finding.message,
        remediation: finding.remediation,
        vm: finding.vm,
        detail: finding.detail,
        details: finding.details,
    }
}

fn map_host_check_severity(severity: host_check::HostCheckSeverity) -> HostCheckSeverityV2 {
    match severity {
        host_check::HostCheckSeverity::Pass => HostCheckSeverityV2::Pass,
        host_check::HostCheckSeverity::Warn => HostCheckSeverityV2::Warn,
        host_check::HostCheckSeverity::Fail => HostCheckSeverityV2::Fail,
    }
}

pub(crate) fn render_host_check_human(output: &HostCheckOutputV2) -> String {
    let mut text = String::new();
    let _ = std::fmt::Write::write_fmt(
        &mut text,
        format_args!(
            "mode: {}\nstrict: {}\nsummary: pass={} warn={} fail={}\nexit-code: {}\n",
            output.mode,
            output.strict,
            output.summary.pass,
            output.summary.warn,
            output.summary.fail,
            output.exit_code
        ),
    );
    for severity in [
        HostCheckSeverityV2::Pass,
        HostCheckSeverityV2::Warn,
        HostCheckSeverityV2::Fail,
    ] {
        let label = match severity {
            HostCheckSeverityV2::Pass => "PASS",
            HostCheckSeverityV2::Warn => "WARN",
            HostCheckSeverityV2::Fail => "FAIL",
        };
        let matching = output
            .findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            continue;
        }
        let _ = std::fmt::Write::write_fmt(&mut text, format_args!("{label}\n"));
        for finding in matching {
            if let Some(vm) = &finding.vm {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!("- [{vm}] {}: {}\n", finding.id, finding.message),
                );
            } else {
                let _ = std::fmt::Write::write_fmt(
                    &mut text,
                    format_args!("- {}: {}\n", finding.id, finding.message),
                );
            }
            let _ = std::fmt::Write::write_fmt(
                &mut text,
                format_args!("  hint: {}\n", finding.remediation),
            );
        }
        text.push('\n');
    }
    text
}
