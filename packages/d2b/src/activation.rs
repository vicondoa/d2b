//! Activation-NixOS Provider command namespace.

use clap::{Args, Subcommand};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    CliFailure,
    context::{OutputMode, RequestDeadline, ZoneContext, parse_resource_ref},
};
use d2b_provider_config_nixos::{
    ConfigApproveRequest, ConfigApproveResponse, ConfigDiffRequest, ConfigDiffResponse,
    ConfigRejectRequest, ConfigRejectResponse, ConfigStageRequest, ConfigStageResponse, ConfigStatusRequest,
    ConfigStatusResponse, ConfigSyncResponse, GuestConfigDocument, GUEST_CONFIG_IDENTIFIER,
};

#[derive(Debug, Args, Clone)]
pub(crate) struct ActivationArgs {
    #[command(subcommand)]
    pub(crate) command: ActivationCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ActivationCommand {
    Apply(ActivationApplyArgs),
    Build(GuestRefArgs),
    Generations(GuestRefArgs),
    Switch(ActivationTargetArgs),
    Boot(ActivationTargetArgs),
    Test(ActivationTargetArgs),
    Rollback(ActivationTargetArgs),
    Adopt(GuestRefArgs),
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
    #[arg(long, conflicts_with = "dry_run")]
    pub(crate) apply: bool,
    #[arg(long = "to-generation")]
    pub(crate) to_generation: Option<u64>,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ActivationApplyArgs {
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ActivationMutationArgs {
    #[arg(long, conflicts_with = "apply")]
    pub(crate) dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
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
        ActivationCommand::Apply(args) => {
            let value =
                context.invoke("Apply", json!({ "dryRun": args.dry_run }), deadline, mode)?;
            context.emit(&value, mode)?;
            Ok(0)
        }
        ActivationCommand::Build(args) => guest_call(context, "Build", args, mode, deadline),
        ActivationCommand::Generations(args) => {
            guest_call(context, "Generations", args, mode, deadline)
        }
        ActivationCommand::Switch(args) => target_call(context, "Switch", args, mode, deadline),
        ActivationCommand::Boot(args) => target_call(context, "Boot", args, mode, deadline),
        ActivationCommand::Test(args) => target_call(context, "Test", args, mode, deadline),
        ActivationCommand::Rollback(args) => target_call(context, "Rollback", args, mode, deadline),
        ActivationCommand::Adopt(args) => guest_call(context, "Adopt", args, mode, deadline),
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
            "toGeneration": args.to_generation,
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
                    "resourceRef": parse_guest_ref(context, &args.guest_ref, mode)?
                        .to_canonical_string(),
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
    let guest_ref = parse_guest_ref(context, name, mode)?;
    let value = context.invoke(
        method,
        json!({ "resourceRef": guest_ref.to_canonical_string() }),
        deadline,
        mode,
    )?;
    context.emit(&value, mode)?;
    Ok(0)
}

fn config(
    context: &ZoneContext,
    args: &ActivationConfigArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, CliFailure> {
    match &args.command {
        ActivationConfigCommand::Sync(args) => {
            let guest_ref = parse_guest_ref(context, &args.guest_ref, mode)?;
            let vm = guest_ref.name().as_str();
            crate::legacy::config_validate_vm_name(vm)?;
            let staging = crate::legacy::config_staging_path(vm);
            if args.dry_run {
                return context.emit(
                    &json!({
                        "command": "config sync",
                        "mode": "dry-run",
                        "resourceRef": guest_ref.to_canonical_string(),
                        "identifier": GUEST_CONFIG_IDENTIFIER,
                    }),
                    mode,
                ).map(|_| 0);
            }
            let value = context.invoke_service(
                d2b_resource_client::ZoneServiceKind::ConfigNixos,
                "ConfigNixosService/ReadGuestConfig",
                "invoke",
                json!({
                    "guestRef": guest_ref,
                    "identifier": GUEST_CONFIG_IDENTIFIER,
                }),
                deadline,
                mode,
            )?;
            let response: ConfigSyncResponse =
                serde_json::from_value(strip_config_response_envelope(value)).map_err(|_| {
                context.failure(
                    "config-document-encoding-failed",
                    "Zone returned an invalid config-nixos response",
                    mode,
                    1,
                )
            })?;
            let document = response.document().map_err(|error| {
                context.failure(error.code(), "config-nixos returned an invalid document", mode, 1)
            })?;
            if let Some(parent) = staging.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    context.failure(
                        "config-stage-failed",
                        &format!("config sync: create staging dir: {error}"),
                        mode,
                        1,
                    )
                })?;
            }
            crate::legacy::config_atomic_write(&staging, document.bytes())?;
            invoke_config_stage(context, guest_ref.clone(), &document, deadline, mode)?;
            context.emit(
                &json!({
                    "command": "config sync",
                    "transport": "component-session",
                    "resourceRef": guest_ref.to_canonical_string(),
                    "identifier": GUEST_CONFIG_IDENTIFIER,
                    "staging": staging.display().to_string(),
                    "bytes": document.len(),
                    "sha256": document.sha256(),
                }),
                mode,
            )?;
            Ok(0)
        }
        ActivationConfigCommand::Diff(args) => {
            let guest_ref = parse_guest_ref(context, &args.guest_ref, mode)?;
            crate::legacy::config_validate_vm_name(guest_ref.name().as_str())?;
            let staging = crate::legacy::config_staging_path(guest_ref.name().as_str());
            if !staging.exists() {
                return Err(context.failure(
                    "config-stage-missing",
                    "nothing staged for this Guest",
                    mode,
                    1,
                ));
            }
            let staged_document = read_staged_document(context, &staging, mode)?;
            invoke_config_stage(context, guest_ref.clone(), &staged_document, deadline, mode)?;
            let against_digest = config_view_identifier(&args.against, context, mode)?;
            let service_diff = invoke_config_diff(
                context,
                guest_ref.clone(),
                against_digest,
                deadline,
                mode,
            )?;
            let output = std::process::Command::new("diff")
                .arg("-u")
                .arg(&args.against)
                .arg(&staging)
                .output()
                .map_err(|error| {
                    context.failure(
                        "config-diff-failed",
                        &format!("config diff: spawn diff: {error}"),
                        mode,
                        1,
                    )
                })?;
            let code = output.status.code().unwrap_or(-1);
            if code > 1 {
                return Err(context.failure(
                    "config-diff-failed",
                    "config diff failed",
                    mode,
                    1,
                ));
            }
            let differs = code == 1;
            if service_diff.differs != differs {
                return Err(context.failure(
                    "config-diff-failed",
                    "config-nixos and local config views disagree",
                    mode,
                    1,
                ));
            }
            if mode.is_json() {
                context.emit(
                    &json!({
                        "command": "config diff",
                        "resourceRef": guest_ref.to_canonical_string(),
                        "against": args.against.display().to_string(),
                        "differs": differs,
                        "diff": String::from_utf8_lossy(&output.stdout),
                    }),
                    mode,
                )?;
            } else if differs {
                crate::print_stdout(&String::from_utf8_lossy(&output.stdout));
            } else {
                crate::print_stdout(&format!(
                    "config diff: staged config for '{}' is identical to {}\n",
                    guest_ref.name().as_str(),
                    args.against.display()
                ));
            }
            Ok(0)
        }
        ActivationConfigCommand::Approve(args) => {
            let guest_ref = parse_guest_ref(context, &args.guest_ref, mode)?;
            crate::legacy::config_validate_vm_name(guest_ref.name().as_str())?;
            let staging = crate::legacy::config_staging_path(guest_ref.name().as_str());
            let staged_document = read_staged_document(context, &staging, mode)?;
            invoke_config_stage(context, guest_ref.clone(), &staged_document, deadline, mode)?;
            let opaque_destination = config_destination_identifier(&args.destination);
            let service_approval = invoke_config_approve(
                context,
                guest_ref.clone(),
                opaque_destination,
                deadline,
                mode,
            )?;
            if service_approval.sha256 != staged_document.sha256() {
                return Err(context.failure(
                    "config-approve-failed",
                    "config-nixos approved a different staged document",
                    mode,
                    1,
                ));
            }
            let bytes = crate::legacy::config_approve_core_with_digest(
                &staging,
                &args.destination,
                Some(&service_approval.sha256),
            )?;
            if service_approval.bytes != bytes {
                return Err(context.failure(
                    "config-approve-failed",
                    "config-nixos approved a different staged document",
                    mode,
                    1,
                ));
            }
            context.emit(
                &json!({
                    "command": "config approve",
                    "resourceRef": guest_ref.to_canonical_string(),
                    "destination": args.destination.display().to_string(),
                    "bytes": bytes,
                }),
                mode,
            )?;
            Ok(0)
        }
        ActivationConfigCommand::Reject(args) => {
            let guest_ref = parse_guest_ref(context, &args.guest_ref, mode)?;
            crate::legacy::config_validate_vm_name(guest_ref.name().as_str())?;
            let staging = crate::legacy::config_staging_path(guest_ref.name().as_str());
            let staged_document = if staging.exists() {
                Some(read_staged_document(context, &staging, mode)?)
            } else {
                None
            };
            if let Some(document) = staged_document.as_ref() {
                invoke_config_stage(context, guest_ref.clone(), document, deadline, mode)?;
            }
            let removed = crate::legacy::config_reject_core(&staging)?;
            let service_rejection =
                invoke_config_reject(context, guest_ref.clone(), deadline, mode)?;
            context.emit(
                &json!({
                    "command": "config reject",
                    "resourceRef": guest_ref.to_canonical_string(),
                    "removed": removed || service_rejection.removed,
                }),
                mode,
            )?;
            Ok(0)
        }
        ActivationConfigCommand::Status(args) => {
            let guest_ref = parse_guest_ref(context, &args.guest_ref, mode)?;
            crate::legacy::config_validate_vm_name(guest_ref.name().as_str())?;
            let staging = crate::legacy::config_staging_path(guest_ref.name().as_str());
            if staging.exists() {
                let staged_document = read_staged_document(context, &staging, mode)?;
                invoke_config_stage(context, guest_ref.clone(), &staged_document, deadline, mode)?;
            }
            let status = invoke_config_status(context, guest_ref.clone(), deadline, mode)?;
            context.emit(
                &json!({
                    "command": "config status",
                    "resourceRef": guest_ref.to_canonical_string(),
                    "pending": status.pending,
                    "bytes": status.bytes,
                    "sha256": status.sha256,
                }),
                mode,
            )?;
            Ok(0)
        }
    }
}

fn strip_config_response_envelope(mut value: serde_json::Value) -> serde_json::Value {
    if let serde_json::Value::Object(object) = &mut value {
        for key in ["ok", "zoneRef", "schemaVersion"] {
            object.remove(key);
        }
    }
    value
}

fn read_staged_document(
    context: &ZoneContext,
    staging: &std::path::Path,
    mode: OutputMode,
) -> Result<GuestConfigDocument, CliFailure> {
    let bytes = std::fs::read(staging).map_err(|error| {
        context.failure(
            "config-stage-failed",
            &format!("config staging read failed: {error}"),
            mode,
            1,
        )
    })?;
    GuestConfigDocument::new(bytes).map_err(|error| {
        context.failure(
            error.code(),
            "config staging document failed validation",
            mode,
            1,
        )
    })
}

fn invoke_config_stage(
    context: &ZoneContext,
    guest_ref: d2b_contracts_resource::v3::ResourceRef,
    document: &GuestConfigDocument,
    deadline: RequestDeadline,
    mode: OutputMode,
) -> Result<ConfigStageResponse, CliFailure> {
    let request = ConfigStageRequest::new(guest_ref, document).map_err(|error| {
        context.failure(error.code(), "config stage request is invalid", mode, 1)
    })?;
    let value = context.invoke_service(
        d2b_resource_client::ZoneServiceKind::ConfigNixos,
        "ConfigNixosService/Stage",
        "invoke",
        serde_json::to_value(request).map_err(|_| {
            context.failure(
                "config-stage-failed",
                "config stage request encoding failed",
                mode,
                1,
            )
        })?,
        deadline,
        mode,
    )?;
    serde_json::from_value(strip_config_response_envelope(value)).map_err(|_| {
        context.failure(
            "config-stage-failed",
            "Zone returned an invalid config stage response",
            mode,
            1,
        )
    })
}

fn invoke_config_diff(
    context: &ZoneContext,
    guest_ref: d2b_contracts_resource::v3::ResourceRef,
    against: String,
    deadline: RequestDeadline,
    mode: OutputMode,
) -> Result<ConfigDiffResponse, CliFailure> {
    let request = ConfigDiffRequest::new(guest_ref, against).map_err(|error| {
        context.failure(error.code(), "config diff request is invalid", mode, 1)
    })?;
    let value = context.invoke_service(
        d2b_resource_client::ZoneServiceKind::ConfigNixos,
        "ConfigNixosService/Diff",
        "invoke",
        serde_json::to_value(request).map_err(|_| {
            context.failure(
                "config-diff-failed",
                "config diff request encoding failed",
                mode,
                1,
            )
        })?,
        deadline,
        mode,
    )?;
    serde_json::from_value(strip_config_response_envelope(value)).map_err(|_| {
        context.failure(
            "config-diff-failed",
            "Zone returned an invalid config diff response",
            mode,
            1,
        )
    })
}

fn invoke_config_approve(
    context: &ZoneContext,
    guest_ref: d2b_contracts_resource::v3::ResourceRef,
    destination: String,
    deadline: RequestDeadline,
    mode: OutputMode,
) -> Result<ConfigApproveResponse, CliFailure> {
    let request = ConfigApproveRequest::new(guest_ref, destination).map_err(|error| {
        context.failure(error.code(), "config approve request is invalid", mode, 1)
    })?;
    let value = context.invoke_service(
        d2b_resource_client::ZoneServiceKind::ConfigNixos,
        "ConfigNixosService/Approve",
        "invoke",
        serde_json::to_value(request).map_err(|_| {
            context.failure(
                "config-approve-failed",
                "config approve request encoding failed",
                mode,
                1,
            )
        })?,
        deadline,
        mode,
    )?;
    serde_json::from_value(strip_config_response_envelope(value)).map_err(|_| {
        context.failure(
            "config-approve-failed",
            "Zone returned an invalid config approval response",
            mode,
            1,
        )
    })
}

fn invoke_config_reject(
    context: &ZoneContext,
    guest_ref: d2b_contracts_resource::v3::ResourceRef,
    deadline: RequestDeadline,
    mode: OutputMode,
) -> Result<ConfigRejectResponse, CliFailure> {
    let request = ConfigRejectRequest::new(guest_ref).map_err(|error| {
        context.failure(error.code(), "config reject request is invalid", mode, 1)
    })?;
    let value = context.invoke_service(
        d2b_resource_client::ZoneServiceKind::ConfigNixos,
        "ConfigNixosService/Reject",
        "invoke",
        serde_json::to_value(request).map_err(|_| {
            context.failure(
                "config-reject-failed",
                "config reject request encoding failed",
                mode,
                1,
            )
        })?,
        deadline,
        mode,
    )?;
    serde_json::from_value(strip_config_response_envelope(value)).map_err(|_| {
        context.failure(
            "config-reject-failed",
            "Zone returned an invalid config rejection response",
            mode,
            1,
        )
    })
}

fn invoke_config_status(
    context: &ZoneContext,
    guest_ref: d2b_contracts_resource::v3::ResourceRef,
    deadline: RequestDeadline,
    mode: OutputMode,
) -> Result<ConfigStatusResponse, CliFailure> {
    let request = ConfigStatusRequest::new(guest_ref).map_err(|error| {
        context.failure(error.code(), "config status request is invalid", mode, 1)
    })?;
    let value = context.invoke_service(
        d2b_resource_client::ZoneServiceKind::ConfigNixos,
        "ConfigNixosService/Status",
        "invoke",
        serde_json::to_value(request).map_err(|_| {
            context.failure(
                "config-status-failed",
                "config status request encoding failed",
                mode,
                1,
            )
        })?,
        deadline,
        mode,
    )?;
    serde_json::from_value(strip_config_response_envelope(value)).map_err(|_| {
        context.failure(
            "config-status-failed",
            "Zone returned an invalid config status response",
            mode,
            1,
        )
    })
}

fn config_view_identifier(
    path: &std::path::Path,
    context: &ZoneContext,
    mode: OutputMode,
) -> Result<String, CliFailure> {
    let bytes = std::fs::read(path).map_err(|error| {
        context.failure(
            "config-diff-failed",
            &format!("config diff: read comparison view: {error}"),
            mode,
            1,
        )
    })?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

fn config_destination_identifier(path: &std::path::Path) -> String {
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    format!("target-{digest:x}")
}

fn parse_guest_ref(
    context: &ZoneContext,
    value: &str,
    mode: OutputMode,
) -> Result<d2b_contracts_resource::v3::ResourceRef, CliFailure> {
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
