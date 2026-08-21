//! Host ResourceType and host-maintenance commands.

use clap::{Args, Subcommand};
use d2b_contracts_broker::host_generation::ApplyHostGenerationHandoff;
use d2b_contracts_control::public_wire::{
    HostCutoverOperation, HostCutoverRequest, HostCutoverResetScope, HostCutoverResponse,
};
use d2b_contracts_resource::v3::CanonicalJsonValue;
use d2b_cutover::{
    OperationState, RunnerCommand, RunnerPaths, RunnerResponse, RunnerSocketError, RunnerStatus,
    send_command,
};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

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
    Cutover(HostCutoverArgs),
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

#[derive(Debug, Args, Clone)]
pub(crate) struct HostCutoverArgs {
    #[command(subcommand)]
    pub(crate) command: HostCutoverCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum HostCutoverCommand {
    Preview(HostCutoverActionArgs),
    Status(HostCutoverActionArgs),
    Hold(HostCutoverActionArgs),
    Resume(HostCutoverActionArgs),
    Apply(HostCutoverActionArgs),
    Rollback(HostCutoverActionArgs),
    Verify(HostCutoverActionArgs),
    Doctor(HostCutoverActionArgs),
    Finalize(HostCutoverActionArgs),
    Reset(HostCutoverResetArgs),
}

#[derive(Debug, Args, Clone, Default)]
pub(crate) struct HostCutoverActionArgs {
    #[arg(long = "operation-id")]
    pub(crate) operation_id: Option<String>,
    #[arg(long = "candidate-id")]
    pub(crate) candidate_id: Option<String>,
    #[arg(long = "revision-plan-id")]
    pub(crate) revision_plan_id: Option<String>,
    #[arg(long = "system-artifact-id")]
    pub(crate) system_artifact_id: Option<String>,
    #[arg(long = "source-system-artifact-id")]
    pub(crate) source_system_artifact_id: Option<String>,
    #[arg(long = "preview-digest")]
    pub(crate) preview_digest: Option<String>,
    #[arg(long = "recovery-digest")]
    pub(crate) recovery_digest: Option<String>,
    #[arg(long = "operator-id")]
    pub(crate) operator_id: Option<String>,
    #[arg(long = "consent-digest")]
    pub(crate) consent_digest: Option<String>,
    #[arg(long = "consent-file")]
    pub(crate) consent_file: Option<PathBuf>,
    #[arg(long = "destructive-consent-digest")]
    pub(crate) destructive_consent_digest: Option<String>,
    #[arg(long = "destructive-consent-file")]
    pub(crate) destructive_consent_file: Option<PathBuf>,
    #[arg(long = "destroy-durable-volumes")]
    pub(crate) destroy_durable_volumes: bool,
    #[arg(long = "recovery-attestation-file")]
    pub(crate) recovery_attestation_file: Option<PathBuf>,
    #[arg(long = "handoff-file")]
    pub(crate) handoff_file: Option<PathBuf>,
    #[arg(long = "finalization-file")]
    pub(crate) finalization_file: Option<PathBuf>,
    #[arg(long = "verification-file")]
    pub(crate) verification_file: Option<PathBuf>,
    #[arg(long = "host-digest")]
    pub(crate) host_digest: Option<String>,
    #[arg(long = "fresh-consent-digest")]
    pub(crate) fresh_consent_digest: Option<String>,
    #[arg(long)]
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct HostCutoverResetArgs {
    #[arg(long = "operation-id")]
    pub(crate) operation_id: Option<String>,
    #[arg(long = "candidate-id")]
    pub(crate) candidate_id: Option<String>,
    #[arg(long = "revision-plan-id")]
    pub(crate) revision_plan_id: Option<String>,
    #[arg(long = "preview-digest")]
    pub(crate) preview_digest: Option<String>,
    #[arg(long = "consent-digest")]
    pub(crate) consent_digest: Option<String>,
    #[arg(long = "consent-file")]
    pub(crate) consent_file: Option<PathBuf>,
    #[arg(long = "destructive-consent-digest")]
    pub(crate) destructive_consent_digest: Option<String>,
    #[arg(long = "destructive-consent-file")]
    pub(crate) destructive_consent_file: Option<PathBuf>,
    #[arg(long = "destroy-durable-volumes")]
    pub(crate) destroy_durable_volumes: bool,
    #[arg(long = "scope", value_enum)]
    pub(crate) scope: HostCutoverResetScopeArg,
    #[arg(long)]
    pub(crate) target: String,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum HostCutoverResetScopeArg {
    Zone,
    Provider,
    Guest,
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
        HostCommand::Cutover(args) => cutover(context, args, mode, deadline),
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

fn cutover(
    context: &ZoneContext,
    args: &HostCutoverArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let (operation, action) = match &args.command {
        HostCutoverCommand::Preview(action) => (HostCutoverOperation::Preview, action),
        HostCutoverCommand::Status(action) => (HostCutoverOperation::Status, action),
        HostCutoverCommand::Hold(action) => (HostCutoverOperation::Hold, action),
        HostCutoverCommand::Resume(action) => (HostCutoverOperation::Resume, action),
        HostCutoverCommand::Apply(action) => (HostCutoverOperation::Apply, action),
        HostCutoverCommand::Rollback(action) => (HostCutoverOperation::Rollback, action),
        HostCutoverCommand::Verify(action) => (HostCutoverOperation::Verify, action),
        HostCutoverCommand::Doctor(action) => (HostCutoverOperation::Doctor, action),
        HostCutoverCommand::Finalize(action) => (HostCutoverOperation::Finalize, action),
        HostCutoverCommand::Reset(reset) => {
            return cutover_reset(context, reset, mode, deadline);
        }
    };

    if context.has_explicit_zone()
        && matches!(
            operation,
            HostCutoverOperation::Preview
                | HostCutoverOperation::Apply
                | HostCutoverOperation::Verify
                | HostCutoverOperation::Finalize
        )
    {
        return Err(context.failure(
            "ref-invalid",
            "one-time host cutover commands do not accept --zone",
            mode,
            2,
        ));
    }

    if requires_cutover_operation_id(operation) && action.operation_id.is_none() {
        return Err(context.failure(
            "ref-invalid",
            "cutover continuation commands require --operation-id",
            mode,
            2,
        ));
    }

    let handoff = match action.handoff_file.as_ref() {
        Some(path)
            if matches!(
                operation,
                HostCutoverOperation::Apply | HostCutoverOperation::Rollback
            ) =>
        {
            let json = read_contract_file(context, Some(path), "handoff", mode)?
                .ok_or_else(|| context.failure("ref-invalid", "handoff file is empty", mode, 2))?;
            Some(
                serde_json::from_str::<ApplyHostGenerationHandoff>(&json).map_err(|_| {
                    context.failure(
                        "ref-invalid",
                        "handoff file is not a valid typed host-generation handoff",
                        mode,
                        2,
                    )
                })?,
            )
        }
        Some(_) => {
            return Err(context.failure(
                "ref-invalid",
                "--handoff-file is valid only with cutover apply or rollback",
                mode,
                2,
            ));
        }
        None => None,
    };

    if operation == HostCutoverOperation::Apply
        && let Some(operation_id) = action.operation_id.as_deref()
    {
        let operation_id = d2b_cutover::OperationId::new(operation_id.to_owned())
            .map_err(|_| context.failure("ref-invalid", "invalid operation id", mode, 2))?;
        if let Some(status) = probe_existing_cutover_runner(context, &operation_id, mode)? {
            let Some(handoff) = handoff.clone() else {
                return Err(context.failure(
                    "ref-invalid",
                    "retrying an admitted cutover apply requires --handoff-file",
                    mode,
                    2,
                ));
            };
            if apply_status_is_advanced(&status) {
                return emit_ambiguous_apply_response(
                    context,
                    RunnerResponse {
                        accepted: true,
                        status: Some(status),
                        error: None,
                    },
                    action.preview_digest.clone(),
                    mode,
                );
            }
            return apply_handoff_via_runner(
                context,
                operation,
                operation_id,
                handoff,
                action.preview_digest.clone(),
                mode,
            );
        }
    }

    if matches!(
        operation,
        HostCutoverOperation::Status
            | HostCutoverOperation::Hold
            | HostCutoverOperation::Resume
            | HostCutoverOperation::Rollback
            | HostCutoverOperation::Verify
            | HostCutoverOperation::Doctor
            | HostCutoverOperation::Finalize
    ) && let Some(operation_id) = action.operation_id.as_deref()
    {
        let root = std::env::var_os("D2B_CUTOVER_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/lib/d2b"));
        let socket_root = std::env::var_os("D2B_CUTOVER_SOCKET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/run/d2b"));
        let operation_id = d2b_cutover::OperationId::new(operation_id.to_owned())
            .map_err(|_| context.failure("ref-invalid", "invalid operation id", mode, 2))?;
        let paths = RunnerPaths::new_with_socket_root(root, socket_root, &operation_id);
        let command = match operation {
            HostCutoverOperation::Status => RunnerCommand::Status,
            HostCutoverOperation::Hold => RunnerCommand::Hold {
                reason: action.reason.clone().ok_or_else(|| {
                    context.failure("ref-invalid", "hold requires --reason", mode, 2)
                })?,
            },
            HostCutoverOperation::Resume => RunnerCommand::Resume {
                fresh_consent: action
                    .fresh_consent_digest
                    .as_deref()
                    .map(|digest| d2b_cutover::Digest::parse(digest.to_owned()))
                    .transpose()
                    .map_err(|_| {
                        context.failure("ref-invalid", "invalid fresh consent digest", mode, 2)
                    })?,
            },
            HostCutoverOperation::Rollback => RunnerCommand::Rollback {
                handoff: handoff.clone(),
            },
            HostCutoverOperation::Verify => {
                let observations = read_contract_file(
                    context,
                    action.verification_file.as_ref(),
                    "verification",
                    mode,
                )?
                .ok_or_else(|| {
                    context.failure(
                        "ref-invalid",
                        "verify requires --verification-file",
                        mode,
                        2,
                    )
                })?;
                let observations =
                    serde_json::from_str::<d2b_cutover::RunnerVerificationInput>(&observations)
                        .map_err(|_| {
                            context.failure(
                                "ref-invalid",
                                "verification file is not a valid observation set",
                                mode,
                                2,
                            )
                        })?;
                RunnerCommand::Verify { observations }
            }
            HostCutoverOperation::Doctor => RunnerCommand::Status,
            HostCutoverOperation::Finalize => {
                let consent_json =
                    read_contract_file(context, action.consent_file.as_ref(), "consent", mode)?
                        .ok_or_else(|| {
                            context.failure(
                                "ref-invalid",
                                "finalize requires --consent-file",
                                mode,
                                2,
                            )
                        })?;
                let consent =
                    d2b_cutover::FinalizationConsent::decode_json(consent_json.as_bytes())
                        .map_err(|_| {
                            context.failure(
                                "ref-invalid",
                                "consent file is not a valid finalization consent",
                                mode,
                                2,
                            )
                        })?;
                let plan_json = read_contract_file(
                    context,
                    action.finalization_file.as_ref(),
                    "finalization",
                    mode,
                )?
                .ok_or_else(|| {
                    context.failure(
                        "ref-invalid",
                        "finalize requires --finalization-file",
                        mode,
                        2,
                    )
                })?;
                let plan = serde_json::from_str::<d2b_cutover::FinalizationPlan>(&plan_json)
                    .map_err(|_| {
                        context.failure(
                            "ref-invalid",
                            "finalization file is not a valid approved disposition plan",
                            mode,
                            2,
                        )
                    })?;
                RunnerCommand::Finalize { consent, plan }
            }
            _ => unreachable!(),
        };
        match send_command(&paths.socket, &command) {
            Ok(response) => {
                let response = normalize_runner_response(operation, response)
                    .map_err(|error| runner_error_failure(context, error, mode))?;
                let value = serde_json::to_value(response).map_err(|_| {
                    context.failure(
                        "internal-error",
                        "failed to encode cutover response",
                        mode,
                        1,
                    )
                })?;
                let value = decorate_cutover_response(context, value);
                context.emit(&value, mode)?;
                return Ok(0);
            }
            Err(_)
                if matches!(
                    operation,
                    HostCutoverOperation::Status | HostCutoverOperation::Doctor
                ) => {}
            Err(_) => {
                return Err(context.failure(
                    "cutover-runner-unavailable",
                    "cutover runner socket unavailable",
                    mode,
                    69,
                ));
            }
        }
    }

    let request = HostCutoverRequest {
        operation,
        operation_id: action.operation_id.clone(),
        candidate_id: action.candidate_id.clone(),
        revision_plan_id: action.revision_plan_id.clone(),
        system_artifact_id: handoff
            .as_ref()
            .map(|handoff| handoff.intent.system_artifact_id.as_str().to_owned())
            .or_else(|| action.system_artifact_id.clone()),
        source_system_artifact_id: action.source_system_artifact_id.clone(),
        preview_digest: action.preview_digest.clone(),
        recovery_digest: action.recovery_digest.clone(),
        operator_id: action.operator_id.clone(),
        consent_digest: action.consent_digest.clone(),
        consent_json: read_contract_file(context, action.consent_file.as_ref(), "consent", mode)?,
        destructive_consent_digest: None,
        destructive_consent_json: None,
        destroy_durable_volumes: None,
        recovery_attestation_json: read_contract_file(
            context,
            action.recovery_attestation_file.as_ref(),
            "recovery attestation",
            mode,
        )?,
        host_digest: action.host_digest.clone(),
        fresh_consent_digest: action.fresh_consent_digest.clone(),
        reason: action.reason.clone(),
        reset_scope: None,
        target: None,
        zone: context
            .has_explicit_zone()
            .then(|| context.zone_name().to_owned()),
    };
    let payload = serde_json::to_value(request).map_err(|_| {
        context.failure(
            "internal-error",
            "failed to encode cutover request",
            mode,
            1,
        )
    })?;
    let value = match if matches!(
        operation,
        HostCutoverOperation::Preview | HostCutoverOperation::Status | HostCutoverOperation::Doctor
    ) {
        context.invoke("HostCutover", payload, deadline, mode)
    } else {
        context.invoke_mutating("HostCutover", payload, deadline, mode)
    } {
        Ok(value) => value,
        Err(_error)
            if matches!(
                operation,
                HostCutoverOperation::Status | HostCutoverOperation::Doctor
            ) =>
        {
            return Err(context.failure(
                "cutover-runner-unavailable",
                "cutover runner and daemon observation are unavailable",
                mode,
                69,
            ));
        }
        Err(error)
            if operation == HostCutoverOperation::Apply
                && handoff.is_some()
                && action.operation_id.is_some()
                && admission_transport_may_be_ambiguous(&error) =>
        {
            let operation_id = d2b_cutover::OperationId::new(
                action
                    .operation_id
                    .clone()
                    .expect("operation id checked above"),
            )
            .map_err(|_| context.failure("ref-invalid", "invalid operation id", mode, 2))?;
            return apply_handoff_via_runner(
                context,
                operation,
                operation_id,
                handoff.expect("handoff checked above"),
                action.preview_digest.clone(),
                mode,
            );
        }
        Err(error) => return Err(error),
    };
    if let Some(handoff) = handoff {
        let preview_digest = value
            .get("previewDigest")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let operation_id = value
            .get("operationId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                context.failure(
                    "cutover-runner-unavailable",
                    "cutover admission returned no operation id",
                    mode,
                    69,
                )
            })?;
        let operation_id = d2b_cutover::OperationId::new(operation_id.to_owned())
            .map_err(|_| context.failure("ref-invalid", "invalid operation id", mode, 2))?;
        return apply_handoff_via_runner(
            context,
            operation,
            operation_id,
            handoff,
            preview_digest,
            mode,
        );
    }
    context.emit(&value, mode)?;
    Ok(0)
}

fn normalize_runner_response(
    operation: HostCutoverOperation,
    response: RunnerResponse,
) -> Result<HostCutoverResponse, RunnerSocketError> {
    if !response.accepted {
        return Err(response.error.unwrap_or(RunnerSocketError::Malformed));
    }
    if let Some(error) = response.error {
        return Err(error);
    }
    let status = response.status.ok_or(RunnerSocketError::Malformed)?;
    Ok(HostCutoverResponse {
        operation,
        operation_id: Some(status.operation_id.to_string()),
        state: runner_state_label(status.state).to_owned(),
        phase: status.phase.number(),
        preview_digest: Some(status.preview_digest.to_string()),
        summary: runner_summary(operation).to_owned(),
        mutation_accepted: !matches!(
            operation,
            HostCutoverOperation::Status | HostCutoverOperation::Doctor
        ),
        inventory: None,
    })
}

fn requires_cutover_operation_id(operation: HostCutoverOperation) -> bool {
    matches!(
        operation,
        HostCutoverOperation::Status
            | HostCutoverOperation::Hold
            | HostCutoverOperation::Resume
            | HostCutoverOperation::Rollback
            | HostCutoverOperation::Verify
            | HostCutoverOperation::Doctor
            | HostCutoverOperation::Finalize
    )
}

fn probe_existing_cutover_runner(
    context: &ZoneContext,
    operation_id: &d2b_cutover::OperationId,
    mode: OutputMode,
) -> Result<Option<RunnerStatus>, CliFailure> {
    let root = std::env::var_os("D2B_CUTOVER_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/d2b"));
    let socket_root = std::env::var_os("D2B_CUTOVER_SOCKET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/d2b"));
    let paths = RunnerPaths::new_with_socket_root(root, socket_root, operation_id);
    let response = match send_command(&paths.socket, &RunnerCommand::Status) {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };
    if !response.accepted {
        return Err(runner_error_failure(
            context,
            response.error.unwrap_or(RunnerSocketError::Malformed),
            mode,
        ));
    }
    if let Some(error) = response.error {
        return Err(runner_error_failure(context, error, mode));
    }
    let status = response
        .status
        .ok_or_else(|| runner_error_failure(context, RunnerSocketError::Malformed, mode))?;
    if status.operation_id != *operation_id {
        return Err(runner_error_failure(
            context,
            RunnerSocketError::Malformed,
            mode,
        ));
    }
    Ok(Some(status))
}

fn admission_transport_may_be_ambiguous(error: &CliFailure) -> bool {
    error.admission_recovery
}

fn apply_handoff_via_runner(
    context: &ZoneContext,
    operation: HostCutoverOperation,
    operation_id: d2b_cutover::OperationId,
    handoff: ApplyHostGenerationHandoff,
    preview_digest: Option<String>,
    mode: OutputMode,
) -> Result<i32, CliFailure> {
    let root = std::env::var_os("D2B_CUTOVER_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/d2b"));
    let socket_root = std::env::var_os("D2B_CUTOVER_SOCKET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/d2b"));
    let paths = RunnerPaths::new_with_socket_root(root, socket_root, &operation_id);
    let command = RunnerCommand::Apply { handoff };
    let mut ambiguous_status = None;
    for _ in 0..20 {
        match send_command(&paths.socket, &command) {
            Ok(response) => match normalize_runner_response(operation, response) {
                Ok(response) => {
                    validate_admitted_preview_digest(
                        context,
                        &response,
                        preview_digest.as_deref(),
                        mode,
                    )?;
                    let value = serde_json::to_value(response).map_err(|_| {
                        context.failure(
                            "internal-error",
                            "failed to encode cutover response",
                            mode,
                            1,
                        )
                    })?;
                    let value = decorate_cutover_response(context, value);
                    context.emit(&value, mode)?;
                    return Ok(0);
                }
                Err(error) if should_reconcile_ambiguous_apply(&error) => {
                    match send_command(&paths.socket, &RunnerCommand::Status) {
                        Ok(status)
                            if status.accepted
                                && status.status.as_ref().is_some_and(apply_status_is_advanced) =>
                        {
                            ambiguous_status = Some(status);
                            break;
                        }
                        Ok(_) => {
                            return Err(runner_error_failure(context, error, mode));
                        }
                        Err(_) => {
                            return Err(context.failure(
                                "cutover-runner-unavailable",
                                "cutover runner status unavailable after ambiguous apply",
                                mode,
                                69,
                            ));
                        }
                    }
                }
                Err(error) => {
                    return Err(runner_error_failure(context, error, mode));
                }
            },
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(25)),
        }
    }
    if let Some(response) = ambiguous_status {
        return emit_ambiguous_apply_response(context, response, preview_digest.clone(), mode);
    }
    if let Ok(response) = send_command(&paths.socket, &RunnerCommand::Status)
        && response.accepted
        && response
            .status
            .as_ref()
            .is_some_and(apply_status_is_advanced)
    {
        return emit_ambiguous_apply_response(context, response, preview_digest, mode);
    }
    Err(context.failure(
        "cutover-runner-unavailable",
        "cutover runner apply socket unavailable",
        mode,
        69,
    ))
}

fn normalize_ambiguous_apply_response(
    response: RunnerResponse,
) -> Result<HostCutoverResponse, RunnerSocketError> {
    let mut response = normalize_runner_response(HostCutoverOperation::Status, response)?;
    response.operation = HostCutoverOperation::Apply;
    response.summary = "cutover apply response lost; runner state observed".to_owned();
    Ok(response)
}

fn emit_ambiguous_apply_response(
    context: &ZoneContext,
    response: RunnerResponse,
    preview_digest: Option<String>,
    mode: OutputMode,
) -> Result<i32, CliFailure> {
    let response = normalize_ambiguous_apply_response(response)
        .map_err(|error| runner_error_failure(context, error, mode))?;
    validate_admitted_preview_digest(context, &response, preview_digest.as_deref(), mode)?;
    let value = serde_json::to_value(response).map_err(|_| {
        context.failure(
            "internal-error",
            "failed to encode cutover response",
            mode,
            1,
        )
    })?;
    let value = decorate_cutover_response(context, value);
    context.emit(&value, mode)?;
    Ok(0)
}

fn validate_admitted_preview_digest(
    context: &ZoneContext,
    response: &HostCutoverResponse,
    expected: Option<&str>,
    mode: OutputMode,
) -> Result<(), CliFailure> {
    if let Some(expected) = expected
        && response.preview_digest.as_deref() != Some(expected)
    {
        return Err(context.failure(
            "ref-invalid",
            "cutover preview digest does not match admitted runner state",
            mode,
            2,
        ));
    }
    Ok(())
}

fn should_reconcile_ambiguous_apply(error: &RunnerSocketError) -> bool {
    matches!(error, RunnerSocketError::InvalidTransition)
}

fn apply_status_is_advanced(status: &RunnerStatus) -> bool {
    status.phase.number() > d2b_cutover::CutoverPhase::Disposition.number()
}

fn runner_state_label(state: OperationState) -> &'static str {
    match state {
        OperationState::Planned => "planned",
        OperationState::Held => "held",
        OperationState::Applying(_) => "applying",
        OperationState::CutoverSucceeded => "cutover-succeeded",
        OperationState::Finalizing => "finalizing",
        OperationState::RolledBack => "rolled-back",
        OperationState::RestoreRequired => "restore-required",
        OperationState::Closed => "closed",
        OperationState::Failed => "failed",
    }
}

fn runner_summary(operation: HostCutoverOperation) -> &'static str {
    match operation {
        HostCutoverOperation::Status | HostCutoverOperation::Doctor => {
            "read-only cutover runner observation"
        }
        HostCutoverOperation::Hold => "cutover safety hold accepted",
        HostCutoverOperation::Resume => "cutover runner resumed",
        HostCutoverOperation::Rollback => "cutover rollback accepted",
        HostCutoverOperation::Verify => "cutover verification accepted",
        HostCutoverOperation::Finalize => "cutover finalization accepted",
        _ => "cutover runner command accepted",
    }
}

fn runner_error_spec(error: RunnerSocketError) -> (&'static str, &'static str, i32) {
    match error {
        RunnerSocketError::Unauthorized | RunnerSocketError::OperatorMismatch => (
            "authz-not-admin",
            "cutover runner authorization refused",
            77,
        ),
        RunnerSocketError::Malformed => ("ref-invalid", "cutover runner command was malformed", 2),
        RunnerSocketError::ArtifactBindingMismatch => (
            "resource-schema-invalid",
            "cutover handoff artifact does not match the admitted candidate",
            2,
        ),
        RunnerSocketError::InvalidTransition => {
            ("internal-error", "cutover runner rejected the command", 1)
        }
        RunnerSocketError::AuditUnavailable | RunnerSocketError::JournalUnavailable => (
            "internal-error",
            "cutover runner could not durably record the command",
            1,
        ),
    }
}

fn runner_error_failure(
    context: &ZoneContext,
    error: RunnerSocketError,
    mode: OutputMode,
) -> CliFailure {
    let (class, message, exit_code) = runner_error_spec(error);
    context.failure(class, message, mode, exit_code)
}

fn decorate_cutover_response(context: &ZoneContext, mut value: Value) -> Value {
    if let Value::Object(object) = &mut value {
        object.insert("ok".to_owned(), Value::Bool(true));
        object.insert("zoneRef".to_owned(), Value::String(context.zone_ref()));
        object.insert(
            "schemaVersion".to_owned(),
            serde_json::Value::Number(serde_json::Number::from(
                crate::context::JSON_SCHEMA_VERSION,
            )),
        );
    }
    value
}

fn read_contract_file(
    context: &ZoneContext,
    path: Option<&PathBuf>,
    label: &str,
    mode: OutputMode,
) -> Result<Option<String>, CliFailure> {
    let Some(path) = path else {
        return Ok(None);
    };
    let bytes = std::fs::read(path).map_err(|error| {
        context.failure(
            "ref-invalid",
            &format!("unable to read {label} file: {error}"),
            mode,
            2,
        )
    })?;
    if bytes.len() > d2b_cutover::MAX_RUNNER_FRAME_BYTES {
        return Err(context.failure(
            "ref-invalid",
            &format!("{label} file exceeds the bounded evidence size"),
            mode,
            2,
        ));
    }
    let text = String::from_utf8(bytes).map_err(|_| {
        context.failure(
            "ref-invalid",
            &format!("{label} file is not UTF-8"),
            mode,
            2,
        )
    })?;
    canonical_contract_text(text.as_bytes())
        .map(Some)
        .map_err(|_| {
            context.failure(
                "ref-invalid",
                &format!("{label} file is not canonical JSON"),
                mode,
                2,
            )
        })
}

fn canonical_contract_text(bytes: &[u8]) -> Result<String, ()> {
    let value = CanonicalJsonValue::parse(bytes).map_err(|_| ())?;
    String::from_utf8(value.to_canonical_bytes()).map_err(|_| ())
}

fn cutover_reset(
    context: &ZoneContext,
    args: &HostCutoverResetArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    let reset_scope = match args.scope {
        HostCutoverResetScopeArg::Zone => HostCutoverResetScope::Zone,
        HostCutoverResetScopeArg::Provider => HostCutoverResetScope::Provider,
        HostCutoverResetScopeArg::Guest => HostCutoverResetScope::Guest,
    };
    let request = HostCutoverRequest {
        operation: HostCutoverOperation::Reset,
        operation_id: args.operation_id.clone(),
        candidate_id: args.candidate_id.clone(),
        revision_plan_id: args.revision_plan_id.clone(),
        system_artifact_id: None,
        source_system_artifact_id: None,
        preview_digest: args.preview_digest.clone(),
        recovery_digest: None,
        operator_id: None,
        consent_digest: args.consent_digest.clone(),
        consent_json: read_contract_file(context, args.consent_file.as_ref(), "consent", mode)?,
        destructive_consent_digest: args.destructive_consent_digest.clone(),
        destructive_consent_json: read_contract_file(
            context,
            args.destructive_consent_file.as_ref(),
            "destructive consent",
            mode,
        )?,
        destroy_durable_volumes: Some(args.destroy_durable_volumes),
        recovery_attestation_json: None,
        host_digest: None,
        fresh_consent_digest: None,
        reason: None,
        reset_scope: Some(reset_scope),
        target: Some(args.target.clone()),
        zone: context
            .has_explicit_zone()
            .then(|| context.zone_name().to_owned()),
    };
    let payload = serde_json::to_value(request).map_err(|_| {
        context.failure("internal-error", "failed to encode reset request", mode, 1)
    })?;
    let value = context.invoke_mutating("HostCutover", payload, deadline, mode)?;
    context.emit(&value, mode)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::CanonicalJsonObject;
    use d2b_cutover::{
        CutoverPhase, OperationId, OperationState, RunnerResponse, RunnerSocketError, RunnerStatus,
    };

    #[test]
    fn cutover_contract_text_is_canonical_before_resource_transport() {
        let consent = canonical_contract_text(
            br#"{ "binding": {} }
"#,
        )
        .expect("contract JSON");
        assert_eq!(consent, r#"{"binding":{}}"#);

        let payload = serde_json::json!({
            "operation": "apply",
            "consentJson": consent,
        });
        let bytes = serde_json::to_vec(&payload).expect("request JSON");
        CanonicalJsonObject::parse(&bytes).expect("canonical resource request");
    }

    fn runner_status() -> RunnerStatus {
        RunnerStatus {
            operation_id: OperationId::new("op-normalized").expect("operation"),
            preview_digest: d2b_cutover::Digest::parse("sha256:".to_owned() + &"a".repeat(64))
                .expect("preview digest"),
            state: OperationState::Applying(CutoverPhase::Preflight),
            phase: CutoverPhase::Preflight,
            hold_active: false,
            terminal: false,
        }
    }

    #[test]
    fn direct_runner_status_is_normalized_to_public_cutover_shape() {
        let response = normalize_runner_response(
            HostCutoverOperation::Status,
            RunnerResponse {
                accepted: true,
                status: Some(runner_status()),
                error: None,
            },
        )
        .expect("normalized status");
        let value = serde_json::to_value(response).expect("response JSON");

        assert_eq!(value["operation"], "status");
        assert_eq!(value["operationId"], "op-normalized");
        assert_eq!(value["state"], "applying");
        assert_eq!(value["phase"], 0);
        assert_eq!(value["mutationAccepted"], false);
        assert_eq!(value["summary"], "read-only cutover runner observation");
        assert!(value.get("status").is_none());
        assert!(value.get("accepted").is_none());
    }

    #[test]
    fn direct_runner_doctor_uses_the_public_read_only_shape() {
        let response = normalize_runner_response(
            HostCutoverOperation::Doctor,
            RunnerResponse {
                accepted: true,
                status: Some(runner_status()),
                error: None,
            },
        )
        .expect("normalized doctor");
        assert_eq!(response.operation, HostCutoverOperation::Doctor);
        assert!(!response.mutation_accepted);
        assert_eq!(
            response.summary,
            "read-only cutover runner observation".to_owned()
        );
    }

    #[test]
    fn refused_runner_apply_is_a_typed_failure_without_a_success_response() {
        let error = normalize_runner_response(
            HostCutoverOperation::Apply,
            RunnerResponse {
                accepted: false,
                status: None,
                error: Some(RunnerSocketError::InvalidTransition),
            },
        )
        .expect_err("refused apply");
        assert_eq!(error, RunnerSocketError::InvalidTransition);
    }

    #[test]
    fn admission_recovery_only_accepts_transport_ambiguity() {
        let mut ambiguous = CliFailure::new(
            1,
            "resource-conflict: resource mutation outcome was ambiguous",
        );
        ambiguous.admission_recovery = true;
        assert!(admission_transport_may_be_ambiguous(&ambiguous));
        assert!(!admission_transport_may_be_ambiguous(&CliFailure::new(
            1,
            "resource-conflict: resource revision conflict"
        )));
        assert!(!admission_transport_may_be_ambiguous(&CliFailure::new(
            2,
            "resource-schema-invalid: cutover preview digest is stale"
        )));
    }

    #[test]
    fn invalid_transition_is_reconciled_before_reporting_apply_refusal() {
        assert!(should_reconcile_ambiguous_apply(
            &RunnerSocketError::InvalidTransition
        ));
        assert!(!should_reconcile_ambiguous_apply(
            &RunnerSocketError::Unauthorized
        ));
    }

    #[test]
    fn only_runner_status_after_disposition_proves_ambiguous_apply() {
        let mut status = runner_status();
        assert!(!apply_status_is_advanced(&status));
        status.phase = CutoverPhase::ResourceStore;
        assert!(apply_status_is_advanced(&status));
    }

    #[test]
    fn normalized_runner_response_can_preserve_the_admission_preview_digest() {
        let response = normalize_runner_response(
            HostCutoverOperation::Apply,
            RunnerResponse {
                accepted: true,
                status: Some(runner_status()),
                error: None,
            },
        )
        .expect("normalized apply");
        let value = decorate_cutover_response(
            &ZoneContext::local_only(),
            serde_json::to_value(response).expect("response JSON"),
        );
        assert_eq!(
            value["previewDigest"],
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(value["ok"], true);
    }

    #[test]
    fn ambiguous_apply_response_exposes_runner_state_without_claiming_mutation() {
        let response = normalize_ambiguous_apply_response(RunnerResponse {
            accepted: true,
            status: Some(runner_status()),
            error: None,
        })
        .expect("ambiguous apply response");
        assert_eq!(response.operation, HostCutoverOperation::Apply);
        assert!(!response.mutation_accepted);
        assert_eq!(
            response.preview_digest.as_deref(),
            Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            response.summary,
            "cutover apply response lost; runner state observed"
        );
    }

    #[test]
    fn admitted_preview_digest_mismatch_is_rejected_on_retry() {
        let response = normalize_runner_response(
            HostCutoverOperation::Apply,
            RunnerResponse {
                accepted: true,
                status: Some(runner_status()),
                error: None,
            },
        )
        .expect("normalized apply");
        let error = validate_admitted_preview_digest(
            &ZoneContext::local_only(),
            &response,
            Some("sha256:".to_owned() + &"b".repeat(64)).as_deref(),
            OutputMode::Json,
        )
        .expect_err("mismatched preview digest");
        assert!(error.message.contains("preview digest"));
    }

    #[test]
    fn direct_runner_mutations_share_the_flat_cutover_response_shape() {
        for operation in [
            HostCutoverOperation::Hold,
            HostCutoverOperation::Resume,
            HostCutoverOperation::Rollback,
            HostCutoverOperation::Verify,
            HostCutoverOperation::Finalize,
        ] {
            let response = normalize_runner_response(
                operation,
                RunnerResponse {
                    accepted: true,
                    status: Some(runner_status()),
                    error: None,
                },
            )
            .expect("normalized mutation");
            assert_eq!(response.operation, operation);
            assert_eq!(response.operation_id.as_deref(), Some("op-normalized"));
            assert_eq!(response.state, "applying");
            assert_eq!(response.phase, 0);
            assert!(response.mutation_accepted);
            assert!(response.preview_digest.is_some());
            assert!(response.inventory.is_none());
        }
    }

    #[test]
    fn direct_runner_errors_map_to_redacted_cli_classes() {
        assert_eq!(
            runner_error_spec(RunnerSocketError::Unauthorized),
            (
                "authz-not-admin",
                "cutover runner authorization refused",
                77
            )
        );
        assert_eq!(
            runner_error_spec(RunnerSocketError::OperatorMismatch),
            (
                "authz-not-admin",
                "cutover runner authorization refused",
                77
            )
        );
        assert_eq!(
            runner_error_spec(RunnerSocketError::Malformed),
            ("ref-invalid", "cutover runner command was malformed", 2)
        );
        assert_eq!(
            runner_error_spec(RunnerSocketError::ArtifactBindingMismatch),
            (
                "resource-schema-invalid",
                "cutover handoff artifact does not match the admitted candidate",
                2
            )
        );
        assert_eq!(
            runner_error_spec(RunnerSocketError::InvalidTransition),
            ("internal-error", "cutover runner rejected the command", 1)
        );
        assert_eq!(
            runner_error_spec(RunnerSocketError::AuditUnavailable),
            (
                "internal-error",
                "cutover runner could not durably record the command",
                1
            )
        );
        assert_eq!(
            runner_error_spec(RunnerSocketError::JournalUnavailable),
            (
                "internal-error",
                "cutover runner could not durably record the command",
                1
            )
        );
    }

    #[test]
    fn continuation_commands_require_an_operation_id() {
        for operation in [
            HostCutoverOperation::Status,
            HostCutoverOperation::Hold,
            HostCutoverOperation::Resume,
            HostCutoverOperation::Rollback,
            HostCutoverOperation::Verify,
            HostCutoverOperation::Doctor,
            HostCutoverOperation::Finalize,
        ] {
            assert!(requires_cutover_operation_id(operation));
        }
        assert!(!requires_cutover_operation_id(
            HostCutoverOperation::Preview
        ));
        assert!(!requires_cutover_operation_id(HostCutoverOperation::Apply));
        assert!(!requires_cutover_operation_id(HostCutoverOperation::Reset));
    }
}
