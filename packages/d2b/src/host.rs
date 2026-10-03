//! Host ResourceType and host-maintenance commands.

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::{
    CliFailure,
    context::{CliContext, OutputMode, RequestDeadline, ZoneContext},
    dispatch::{GenericGetArgs, GenericListArgs},
    dispatch::{emit_host_error, host_error_envelope},
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
    Reset(HostResetArgs),
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

/// `d2b host reset`: the offline, ownership-bounded clean break.
///
/// The verb resolves no Zone and opens no daemon socket. It runs the
/// one-shot broker ownership runner, which admits the reset Operation from
/// the verified new deployment graph under explicit local operator
/// authority and never opens the previous release's SpecStore. `--dry-run`
/// reports the exact inventory; `--apply` is the only way to remove it.
#[derive(Debug, Args, Clone)]
pub(crate) struct HostResetArgs {
    #[arg(long, conflicts_with = "apply")]
    pub(crate) dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    pub(crate) apply: bool,
    /// The deployment root this reset is bounded to. It defaults to the
    /// broker's own state directory.
    #[arg(long, value_name = "PATH")]
    pub(crate) state_root: Option<std::path::PathBuf>,
    /// The cgroup hierarchy root the live-drain probe walks. It defaults
    /// to the broker's own managed slice.
    #[arg(long, value_name = "PATH")]
    pub(crate) cgroup_root: Option<std::path::PathBuf>,
}

/// The one-shot broker ownership runner `d2b host reset` invokes.
///
/// An operator override for a non-default install prefix, in the shape of
/// the existing `D2B_PUBLIC_SOCKET` and `D2B_BROKER_SOCKET_PATH` overrides.
const BROKER_BIN_ENV: &str = "D2B_BROKER_BIN";

/// The Tier-0 install path of the composed broker binary, the same
/// convention `d2bd` and the activation helper use.
const DEFAULT_BROKER_BIN: &str = "/run/current-system/sw/bin/d2b-broker";

#[derive(Debug, Args, Clone)]
pub(crate) struct HostReconcileArgs {
    #[arg(long)]
    pub(crate) network: bool,
    #[arg(long, conflicts_with = "apply")]
    pub(crate) dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    pub(crate) apply: bool,
}

/// The `host` verb a mutation-mode refusal names, for a subcommand that
/// mutates state and selected neither `--dry-run` nor `--apply`.
///
/// The rule belongs to the invocation rather than to the runtime, so
/// [`crate::dispatch::modern_run`] asks this before it resolves a Zone: a
/// missing mode is owed the `--apply-or-dry-run-required` envelope even when
/// the public socket cannot be reached.
pub(crate) fn missing_mutation_mode(command: &HostCommand) -> Option<&'static str> {
    let (verb, dry_run, apply) = match command {
        HostCommand::Prepare(args) => ("host prepare", args.dry_run, args.apply),
        HostCommand::Destroy(args) => ("host destroy", args.dry_run, args.apply),
        HostCommand::Reconcile(args) => ("host reconcile", args.dry_run, args.apply),
        HostCommand::Validate(args) => ("host validate", args.dry_run, args.apply),
        HostCommand::Reset(args) => ("host reset", args.dry_run, args.apply),
        HostCommand::Get(_) | HostCommand::List(_) | HostCommand::Status(_) => return None,
        HostCommand::Doctor(_) => return None,
    };
    (!dry_run && !apply).then_some(verb)
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
        // `reset` is dispatched offline, ahead of Zone discovery and ahead
        // of this function; a Zone-resolved `host` run is by definition not
        // the clean-break path, so refusing here keeps the two routes
        // impossible to confuse rather than quietly running the wrong one.
        HostCommand::Reset(_) => Err(CliFailure::new(
            1,
            "reset-needs-the-offline-dispatch: `d2b host reset` does not resolve a Zone",
        )),
    }
}

/// Run the offline ownership-bounded reset.
///
/// `d2b host reset` resolves no Zone and opens no daemon socket: it
/// spawns the one-shot broker ownership runner, which admits the reset
/// Operation from the verified new deployment graph under explicit local
/// operator authority, and relays the runner's single envelope unchanged.
/// The `--dry-run`/`--apply` boundary is enforced here first, so a
/// missing mode is refused at exit 78 even on a host with no broker and no
/// d2bd at all - which is exactly the host this verb exists for.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub(crate) fn run_reset(args: &HostResetArgs, mode: OutputMode) -> Result<i32, CliFailure> {
    if !args.dry_run && !args.apply {
        return emit_host_error(
            &crate::dispatch::missing_mutation_flag_envelope("host reset"),
            mode.is_json(),
        );
    }
    let broker = broker_binary();
    let mut command = std::process::Command::new(&broker);
    command.arg("reset");
    if args.apply {
        command.arg("--apply");
    } else {
        command.arg("--dry-run");
    }
    if let Some(state_root) = args.state_root.as_ref() {
        command.arg("--state-dir").arg(state_root);
    }
    if let Some(cgroup_root) = args.cgroup_root.as_ref() {
        command.arg("--cgroup-root").arg(cgroup_root);
    }
    let output = command.output().map_err(|error| {
        CliFailure::new(
            1,
            format!(
                "reset-runner-unavailable: {}: {error}",
                broker.display()
            ),
        )
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let envelope = stdout
        .lines()
        .last()
        .filter(|line| !line.trim().is_empty())
        .and_then(|line| serde_json::from_str::<Value>(line).ok());
    match envelope {
        Some(value) => {
            if mode.is_json() {
                print_json(&value)?;
            } else {
                print_stdout(&render_reset_human(&value));
            }
            Ok(exit_code_for(&value, output.status.code()))
        }
        None => Err(CliFailure::new(
            1,
            format!(
                "reset-runner-produced-no-envelope: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        )),
    }
}

/// The one-shot broker ownership runner's binary.
fn broker_binary() -> std::path::PathBuf {
    std::env::var_os(BROKER_BIN_ENV)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(DEFAULT_BROKER_BIN))
}

/// The process exit code the runner's envelope implies.
///
/// A refused boundary keeps the runner's own refusal code, so the caller
/// reports the reason rather than a transport failure that would send an
/// operator looking at the daemon.
fn exit_code_for(envelope: &Value, status: Option<i32>) -> i32 {
    if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
        return 0;
    }
    status.unwrap_or(1)
}

/// Render the runner's envelope for a terminal.
///
/// Every field is already operator-facing and bounded: the inventory is a
/// path list the runner decided on, and the refusal carries the exact code
/// the boundary refused with.
fn render_reset_human(envelope: &Value) -> String {
    let mut out = String::new();
    if envelope.get("ok").and_then(Value::as_bool) == Some(true) {
        let mode = envelope
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("inspect");
        out.push_str(&format!(
            "d2b host reset: ownership boundary verified ({mode})\n"
        ));
        if let Some(root) = envelope.get("deploymentRoot").and_then(Value::as_str) {
            out.push_str(&format!("  deployment root : {root}\n"));
        }
        if let Some(id) = envelope.get("ownershipId").and_then(Value::as_str) {
            out.push_str(&format!("  ownership id     : {id}\n"));
        }
        let empty = Vec::new();
        let inventory = envelope
            .get("inventory")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        out.push_str(&format!(
            "  inventory        : {} owned path(s), {} present\n",
            inventory.len(),
            inventory
                .iter()
                .filter(|entry| entry.get("present").and_then(Value::as_bool) == Some(true))
                .count()
        ));
        for entry in inventory {
            let path = entry.get("path").and_then(Value::as_str).unwrap_or("");
            let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("");
            let removed = entry
                .get("removed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let state = if removed {
                "removed"
            } else if entry.get("present").and_then(Value::as_bool) == Some(true) {
                "present"
            } else {
                "absent"
            };
            out.push_str(&format!("    [{kind}] {path} ({state})\n"));
        }
        if let Some(subject) = envelope.get("admittedSubject").and_then(Value::as_str) {
            out.push_str(&format!("  admitted as      : {subject}\n"));
        }
        if let Some(incarnation) = envelope.get("incarnation")
            && let Some(store) = incarnation.get("storeIncarnation").and_then(Value::as_str)
        {
            out.push_str(&format!("  new incarnation  : {store}\n"));
        }
    } else {
        let code = envelope.get("code").and_then(Value::as_str).unwrap_or("unknown");
        let detail = envelope
            .get("detail")
            .and_then(Value::as_str)
            .unwrap_or("");
        out.push_str(&format!("d2b host reset refused: {code}\n"));
        if let Some(path) = envelope.get("path").and_then(Value::as_str) {
            out.push_str(&format!("  path             : {path}\n"));
        }
        out.push_str(&format!("  detail           : {detail}\n"));
        out.push_str(
            "  remediation      : nothing was removed. Resolve the refusal above, or re-run `d2b host reset --dry-run` for the exact inventory.\n",
        );
    }
    out
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
