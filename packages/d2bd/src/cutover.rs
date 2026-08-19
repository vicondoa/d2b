//! Daemon-side cutover admission and read-only observation.
//!
//! The daemon never owns the cutover journal, lock, or staged host paths.
//! Before drain it only validates an Admin request and asks the live broker
//! to launch the one-shot runner. After restart it may query the runner's
//! redaction-safe status socket, but it does not repair or adopt operation
//! state.

use std::{
    collections::BTreeSet,
    os::fd::AsRawFd,
    time::{SystemTime, UNIX_EPOCH},
};

use d2b_contracts::{
    broker_wire::{
        BrokerCallerRole, BrokerRequest, BrokerResponse, CanonicalAuditDigest,
        LaunchCutoverRunnerRequest,
    },
    public_wire::{
        HostCutoverInventorySummary, HostCutoverOperation, HostCutoverRequest, HostCutoverResponse,
    },
    types::BundleOpId,
};
use d2b_cutover::{
    BootstrapCapability, CandidateId, Consent, CutoverPreview, Digest, HostInventory,
    InventoryInputItem, InventoryItem, OperationId, OperationKind, OperationRequest,
    OperationState, OperatorId, RecoveryAttestation, ResetInventory, ResetScope, RevisionPlanId,
    RunnerBootstrap, RunnerCommand, RunnerPaths, RunnerStatus, ZoneInventory, send_command,
};
use nix::unistd::{pipe, write};
use serde_json::Value;
use uzers::{get_group_by_name, get_user_by_name};

use crate::{
    PeerIdentity, PeerRole, ServerState, TypedError, broker_socket_path,
    dispatch_broker_request_with_optional_request_fds,
};

/// Dispatch one authenticated host cutover command.
pub(crate) fn dispatch(
    state: &ServerState,
    peer: &PeerIdentity,
    request: HostCutoverRequest,
) -> Result<Value, TypedError> {
    reject_zone_selection(&request)?;
    match request.operation {
        HostCutoverOperation::Preview => {
            if !matches!(peer.role, PeerRole::Admin) {
                return Err(TypedError::AuthzNotAdmin {
                    verb: "hostCutover".to_owned(),
                });
            }
            preview(state, request)
        }
        HostCutoverOperation::Apply => apply(state, peer, request),
        HostCutoverOperation::Status | HostCutoverOperation::Doctor => observe(state, request),
        HostCutoverOperation::Hold
        | HostCutoverOperation::Resume
        | HostCutoverOperation::Rollback
        | HostCutoverOperation::Verify
        | HostCutoverOperation::Finalize => Err(TypedError::InternalConfig {
            detail: "cutover command must use the runner owner socket after admission".to_owned(),
        }),
        HostCutoverOperation::Reset => reset(state, peer, request),
    }
}

fn reject_zone_selection(request: &HostCutoverRequest) -> Result<(), TypedError> {
    if request.zone.is_some() && !matches!(request.operation, HostCutoverOperation::Reset) {
        return Err(TypedError::WireUnknownField {
            detail: "one-time host cutover commands do not accept --zone".to_owned(),
        });
    }
    Ok(())
}

fn preview(state: &ServerState, request: HostCutoverRequest) -> Result<Value, TypedError> {
    let operation_id =
        parse_operation_id(request.operation_id.as_deref().unwrap_or("cutover-preview"))?;
    let candidate_id = parse_candidate_id(
        request
            .candidate_id
            .as_deref()
            .unwrap_or("candidate-preview"),
    )?;
    let revision_plan_id = parse_revision_plan_id(
        request
            .revision_plan_id
            .as_deref()
            .unwrap_or("plan-current"),
    )?;
    let _operator_id =
        parse_operator_id(request.operator_id.as_deref().unwrap_or("operator-preview"))?;
    let (inventory, inventory_summary) = host_inventory(state)?;
    let recovery_digest = request
        .recovery_digest
        .as_deref()
        .map(Digest::parse)
        .transpose()
        .map_err(|_| invalid("recoveryDigest"))?;
    let preview = CutoverPreview::new(
        operation_id.clone(),
        OperationKind::Cutover,
        candidate_id,
        revision_plan_id,
        inventory,
        recovery_digest,
    )
    .map_err(|_| invalid("preview"))?;
    let preview_digest = preview
        .digest()
        .map_err(|_| invalid("previewDigest"))?
        .to_string();
    Ok(serde_json::to_value(HostCutoverResponse {
        operation: HostCutoverOperation::Preview,
        operation_id: Some(operation_id.to_string()),
        state: "planned".to_owned(),
        phase: 0,
        preview_digest: Some(preview_digest),
        summary: "host-wide cutover preview completed without mutation".to_owned(),
        mutation_accepted: false,
        inventory: Some(inventory_summary),
    })
    .expect("typed cutover response serializes"))
}

fn apply(
    state: &ServerState,
    peer: &PeerIdentity,
    request: HostCutoverRequest,
) -> Result<Value, TypedError> {
    if !matches!(peer.role, PeerRole::Admin) {
        return Err(TypedError::AuthzNotAdmin {
            verb: "hostCutover".to_owned(),
        });
    }
    let operation_id = parse_required_operation_id(request.operation_id.as_deref())?;
    let candidate_id = parse_required_candidate_id(request.candidate_id.as_deref())?;
    let revision_plan_id = parse_required_revision_plan_id(request.revision_plan_id.as_deref())?;
    let operator_id = parse_required_operator_id(request.operator_id.as_deref())?;
    let preview_digest = parse_required_digest(request.preview_digest.as_deref(), "previewDigest")?;
    let recovery_digest =
        parse_required_digest(request.recovery_digest.as_deref(), "recoveryDigest")?;
    let consent_digest = parse_required_digest(request.consent_digest.as_deref(), "consentDigest")?;
    let consent_json = request
        .consent_json
        .as_deref()
        .ok_or_else(|| invalid("consentJson"))?;
    let consent =
        Consent::decode_json(consent_json.as_bytes()).map_err(|_| invalid("consentJson"))?;
    if consent.digest().map_err(|_| invalid("consentJson"))? != consent_digest {
        return Err(invalid("consentDigest"));
    }
    let recovery = RecoveryAttestation::decode_json(
        request
            .recovery_attestation_json
            .as_deref()
            .ok_or_else(|| invalid("recoveryAttestationJson"))?
            .as_bytes(),
    )
    .map_err(|_| invalid("recoveryAttestationJson"))?;
    let host_digest = parse_required_digest(request.host_digest.as_deref(), "hostDigest")?;
    authorize_bound_operator(peer.uid, &operator_id)?;
    let (inventory, _) = host_inventory(state)?;
    let preview = CutoverPreview::new(
        operation_id.clone(),
        OperationKind::Cutover,
        candidate_id.clone(),
        revision_plan_id.clone(),
        inventory.clone(),
        Some(recovery_digest.clone()),
    )
    .map_err(|_| invalid("preview"))?;
    if preview.digest().map_err(|_| invalid("previewDigest"))? != preview_digest {
        return Err(TypedError::WireUnknownField {
            detail: "cutover preview digest is stale".to_owned(),
        });
    }
    let operation = OperationRequest::new_cutover(
        operation_id.clone(),
        candidate_id,
        revision_plan_id,
        operator_id.clone(),
        preview_digest,
        recovery_digest,
        inventory,
    )
    .map_err(|_| invalid("operation"))?;
    let now = now_ms();
    let nonce = Digest::derive(
        "d2b:cutover:bootstrap",
        format!("{}:{}:{}", operation.request_digest(), peer.uid, now).as_bytes(),
    );
    let lifecycle_gid =
        get_group_by_name(&state.config.public_socket_group).map(|group| group.gid());
    let capability = BootstrapCapability::new_with_identity_and_group(
        operation_id.clone(),
        operation.candidate_id().clone(),
        operator_id,
        OperationKind::Cutover,
        nonce,
        now,
        now.saturating_add(d2b_cutover::MAX_BOOTSTRAP_LIFETIME_MS),
        peer.uid,
        configured_admin_uids(state, peer.uid),
        lifecycle_gid,
    )
    .map_err(|_| invalid("capability"))?;
    let capability_digest =
        CanonicalAuditDigest::parse(capability.binding_digest().as_str().to_owned())
            .map_err(|_| invalid("capability"))?;
    let bootstrap = RunnerBootstrap {
        capability,
        request: operation,
        preview,
        consent: Some(consent),
        destructive_consent: None,
        recovery: Some(recovery),
        host_digest: Some(host_digest),
    };
    let bytes = bootstrap
        .canonical_bytes()
        .map_err(|_| invalid("bootstrap"))?;
    let (read_fd, write_fd) = pipe().map_err(|error| TypedError::InternalIo {
        context: "create cutover bootstrap pipe".to_owned(),
        detail: error.to_string(),
    })?;
    let written = write(&write_fd, &bytes).map_err(|error| TypedError::InternalIo {
        context: "write cutover bootstrap".to_owned(),
        detail: error.to_string(),
    })?;
    if written != bytes.len() {
        return Err(TypedError::InternalIo {
            context: "write cutover bootstrap".to_owned(),
            detail: "short bootstrap write".to_owned(),
        });
    }
    drop(write_fd);
    let (response, received_fds) = dispatch_broker_request_with_optional_request_fds(
        state,
        BrokerRequest::LaunchCutoverRunner(LaunchCutoverRunnerRequest {
            operation_id: BundleOpId::new(operation_id.as_str()),
            bootstrap_fd_index: 0,
            capability_digest,
            expires_at_ms: bootstrap.capability.expires_at_ms(),
        }),
        BrokerCallerRole::AdminUid { uid: peer.uid },
        &[read_fd.as_raw_fd()],
        std::time::Duration::from_secs(10),
    )?;
    drop(read_fd);
    crate::close_received_fds(&received_fds);
    let response = match response {
        BrokerResponse::LaunchCutoverRunner(response) => response,
        BrokerResponse::Error(error) => {
            return Err(TypedError::InternalBrokerUnavailable {
                path: broker_socket_path(state),
                detail: error.kind,
            });
        }
        _ => {
            return Err(TypedError::InternalBrokerUnavailable {
                path: broker_socket_path(state),
                detail: "cutover runner launch response mismatch".to_owned(),
            });
        }
    };
    Ok(serde_json::to_value(HostCutoverResponse {
        operation: HostCutoverOperation::Apply,
        operation_id: Some(response.operation_id.to_string()),
        state: "planned".to_owned(),
        phase: 0,
        preview_digest: request.preview_digest,
        summary: "cutover runner admitted before control-plane drain".to_owned(),
        mutation_accepted: true,
        inventory: None,
    })
    .expect("typed cutover response serializes"))
}

fn reset(
    state: &ServerState,
    peer: &PeerIdentity,
    request: HostCutoverRequest,
) -> Result<Value, TypedError> {
    if !matches!(peer.role, PeerRole::Admin) {
        return Err(TypedError::AuthzNotAdmin {
            verb: "hostCutoverReset".to_owned(),
        });
    }
    let operation_id = parse_required_operation_id(request.operation_id.as_deref())?;
    let candidate_id = parse_required_candidate_id(request.candidate_id.as_deref())?;
    let revision_plan_id = parse_required_revision_plan_id(request.revision_plan_id.as_deref())?;
    let preview_digest = parse_required_digest(request.preview_digest.as_deref(), "previewDigest")?;
    let consent_digest = parse_required_digest(request.consent_digest.as_deref(), "consentDigest")?;
    let destroy_durable_volumes = request.destroy_durable_volumes.unwrap_or(false);
    let scope = match request.reset_scope.ok_or_else(|| invalid("resetScope"))? {
        d2b_contracts::public_wire::HostCutoverResetScope::Zone => ResetScope::Zone,
        d2b_contracts::public_wire::HostCutoverResetScope::Provider => ResetScope::Provider,
        d2b_contracts::public_wire::HostCutoverResetScope::Guest => ResetScope::Guest,
    };
    let target = request.target.ok_or_else(|| invalid("target"))?;
    let inventory = ResetInventory::new(scope, target)
        .map_err(|_| invalid("resetTarget"))?
        .with_preserve_durable_volumes(!destroy_durable_volumes)
        .with_destroy_durable_consent(destroy_durable_volumes);
    let preview = CutoverPreview::new_reset(
        operation_id.clone(),
        OperationKind::ScopedReset(scope),
        candidate_id.clone(),
        revision_plan_id.clone(),
        inventory.clone(),
    )
    .map_err(|_| invalid("preview"))?;
    if preview.digest().map_err(|_| invalid("previewDigest"))? != preview_digest {
        return Err(TypedError::WireUnknownField {
            detail: "reset preview digest is stale".to_owned(),
        });
    }
    let consent_json = request
        .consent_json
        .as_deref()
        .ok_or_else(|| invalid("consentJson"))?;
    let consent =
        Consent::decode_json(consent_json.as_bytes()).map_err(|_| invalid("consentJson"))?;
    if consent.digest().map_err(|_| invalid("consentJson"))? != consent_digest {
        return Err(invalid("consentDigest"));
    }
    let destructive_consent = if destroy_durable_volumes {
        let digest = parse_required_digest(
            request.destructive_consent_digest.as_deref(),
            "destructiveConsentDigest",
        )?;
        let json = request
            .destructive_consent_json
            .as_deref()
            .ok_or_else(|| invalid("destructiveConsentJson"))?;
        let consent =
            Consent::decode_json(json.as_bytes()).map_err(|_| invalid("destructiveConsentJson"))?;
        if consent
            .digest()
            .map_err(|_| invalid("destructiveConsentJson"))?
            != digest
        {
            return Err(invalid("destructiveConsentDigest"));
        }
        Some(consent)
    } else {
        if request.destructive_consent_digest.is_some()
            || request.destructive_consent_json.is_some()
        {
            return Err(invalid("destructiveConsent"));
        }
        None
    };
    let operator_id = parse_operator_id(&format!("uid-{}", peer.uid))?;
    let operation = OperationRequest::new_reset(
        operation_id.clone(),
        scope,
        candidate_id,
        revision_plan_id,
        operator_id,
        preview_digest,
        inventory,
    )
    .map_err(|_| invalid("operation"))?;
    if let Some(consent) = &destructive_consent
        && consent.binding() != &operation.consent_binding()
    {
        return Err(invalid("destructiveConsentBinding"));
    }
    let now = now_ms();
    let nonce = Digest::derive(
        "d2b:cutover:reset-bootstrap",
        format!("{}:{}:{}", operation.request_digest(), peer.uid, now).as_bytes(),
    );
    let lifecycle_gid =
        get_group_by_name(&state.config.public_socket_group).map(|group| group.gid());
    let capability = BootstrapCapability::new_with_identity_and_group(
        operation_id.clone(),
        operation.candidate_id().clone(),
        operation.operator_id().clone(),
        OperationKind::ScopedReset(scope),
        nonce,
        now,
        now.saturating_add(d2b_cutover::MAX_BOOTSTRAP_LIFETIME_MS),
        peer.uid,
        configured_admin_uids(state, peer.uid),
        lifecycle_gid,
    )
    .map_err(|_| invalid("capability"))?;
    let capability_digest =
        CanonicalAuditDigest::parse(capability.binding_digest().as_str().to_owned())
            .map_err(|_| invalid("capability"))?;
    let bootstrap = RunnerBootstrap {
        capability,
        request: operation,
        preview,
        consent: Some(consent),
        destructive_consent,
        recovery: None,
        host_digest: None,
    };
    let bytes = bootstrap
        .canonical_bytes()
        .map_err(|_| invalid("bootstrap"))?;
    let (read_fd, write_fd) = pipe().map_err(|error| TypedError::InternalIo {
        context: "create reset bootstrap pipe".to_owned(),
        detail: error.to_string(),
    })?;
    let written = write(&write_fd, &bytes).map_err(|error| TypedError::InternalIo {
        context: "write reset bootstrap".to_owned(),
        detail: error.to_string(),
    })?;
    if written != bytes.len() {
        return Err(TypedError::InternalIo {
            context: "write reset bootstrap".to_owned(),
            detail: "short bootstrap write".to_owned(),
        });
    }
    drop(write_fd);
    let (response, received_fds) = dispatch_broker_request_with_optional_request_fds(
        state,
        BrokerRequest::LaunchCutoverRunner(LaunchCutoverRunnerRequest {
            operation_id: BundleOpId::new(operation_id.as_str()),
            bootstrap_fd_index: 0,
            capability_digest,
            expires_at_ms: bootstrap.capability.expires_at_ms(),
        }),
        BrokerCallerRole::AdminUid { uid: peer.uid },
        &[read_fd.as_raw_fd()],
        std::time::Duration::from_secs(10),
    )?;
    drop(read_fd);
    crate::close_received_fds(&received_fds);
    let response = match response {
        BrokerResponse::LaunchCutoverRunner(response) => response,
        BrokerResponse::Error(error) => {
            return Err(TypedError::InternalBrokerUnavailable {
                path: broker_socket_path(state),
                detail: error.kind,
            });
        }
        _ => {
            return Err(TypedError::InternalBrokerUnavailable {
                path: broker_socket_path(state),
                detail: "reset runner launch response mismatch".to_owned(),
            });
        }
    };
    Ok(serde_json::to_value(HostCutoverResponse {
        operation: HostCutoverOperation::Reset,
        operation_id: Some(response.operation_id.to_string()),
        state: "planned".to_owned(),
        phase: 0,
        preview_digest: request.preview_digest,
        summary: "scoped reset runner admitted with a distinct capability".to_owned(),
        mutation_accepted: true,
        inventory: None,
    })
    .expect("typed reset response serializes"))
}

fn observe(state: &ServerState, request: HostCutoverRequest) -> Result<Value, TypedError> {
    let operation_id = parse_required_operation_id(request.operation_id.as_deref())?;
    let state_root = std::env::var_os("D2B_CUTOVER_STATE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/var/lib/d2b"));
    let socket_root = state
        .config
        .public_socket_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/run/d2b"));
    let paths = RunnerPaths::new_with_socket_root(state_root, socket_root, &operation_id);
    let response = send_command(&paths.socket, &RunnerCommand::Status).map_err(|error| {
        TypedError::InternalBrokerUnavailable {
            path: paths.socket,
            detail: error.to_string(),
        }
    })?;
    let status: RunnerStatus = response.status.ok_or_else(|| TypedError::InternalIo {
        context: "decode cutover runner status".to_owned(),
        detail: "runner returned no status".to_owned(),
    })?;
    Ok(serde_json::to_value(HostCutoverResponse {
        operation: request.operation,
        operation_id: Some(status.operation_id.to_string()),
        state: state_label(status.state).to_owned(),
        phase: status.phase.number(),
        preview_digest: None,
        summary: "read-only cutover runner observation".to_owned(),
        mutation_accepted: false,
        inventory: None,
    })
    .expect("typed cutover response serializes"))
}

fn host_inventory(
    state: &ServerState,
) -> Result<(HostInventory, HostCutoverInventorySummary), TypedError> {
    let resolver = crate::load_bundle_resolver(state)?;
    let configured =
        crate::authoritative_zone_ids(&resolver).map_err(|detail| TypedError::InternalConfig {
            detail: detail.to_owned(),
        })?;
    let zones = configured
        .iter()
        .map(|zone| ZoneInventory::empty(zone.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid("zoneInventory"))?;
    let configured_cutover = configured
        .iter()
        .map(|zone| d2b_cutover::ZoneId::new(zone.as_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid("zoneInventory"))?;
    let shared_items = resolver
        .bundle
        .artifact_hashes
        .as_ref()
        .map(|artifacts| {
            artifacts
                .keys()
                .map(|key| {
                    let identity = Digest::derive("d2b:cutover:artifact-identity", key.as_bytes());
                    InventoryItem::unclassified(identity.as_str()).map(InventoryInputItem::Item)
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
        .map_err(|_| invalid("sharedInventory"))?
        .unwrap_or_default();
    let inventory = HostInventory::build(configured_cutover, zones, shared_items)
        .map_err(|_| invalid("hostInventory"))?;
    let digest = inventory
        .digest()
        .map_err(|_| invalid("inventoryDigest"))?
        .to_string();
    let zone_count = u32::try_from(inventory.zones().len()).unwrap_or(u32::MAX);
    let item_count = u32::try_from(
        inventory
            .zones()
            .iter()
            .map(|zone| zone.items().len())
            .sum::<usize>()
            + inventory.shared_items().len(),
    )
    .unwrap_or(u32::MAX);
    Ok((
        inventory,
        HostCutoverInventorySummary {
            zone_count,
            item_count,
            inventory_digest: digest,
            complete: true,
        },
    ))
}

fn configured_admin_uids(state: &ServerState, bound_uid: u32) -> BTreeSet<u32> {
    let mut uids = BTreeSet::from([bound_uid, state.daemon_uid]);
    for name in &state.config.admin_users {
        if let Some(user) = get_user_by_name(name) {
            uids.insert(user.uid());
        }
    }
    uids
}

fn parse_operation_id(value: &str) -> Result<OperationId, TypedError> {
    OperationId::new(value).map_err(|_| invalid("operationId"))
}

fn parse_required_operation_id(value: Option<&str>) -> Result<OperationId, TypedError> {
    parse_operation_id(value.ok_or_else(|| invalid("operationId"))?)
}

fn parse_candidate_id(value: &str) -> Result<CandidateId, TypedError> {
    CandidateId::new(value).map_err(|_| invalid("candidateId"))
}

fn parse_required_candidate_id(value: Option<&str>) -> Result<CandidateId, TypedError> {
    parse_candidate_id(value.ok_or_else(|| invalid("candidateId"))?)
}

fn parse_revision_plan_id(value: &str) -> Result<RevisionPlanId, TypedError> {
    RevisionPlanId::new(value).map_err(|_| invalid("revisionPlanId"))
}

fn parse_required_revision_plan_id(value: Option<&str>) -> Result<RevisionPlanId, TypedError> {
    parse_revision_plan_id(value.ok_or_else(|| invalid("revisionPlanId"))?)
}

fn parse_operator_id(value: &str) -> Result<OperatorId, TypedError> {
    OperatorId::new(value).map_err(|_| invalid("operatorId"))
}

fn parse_required_operator_id(value: Option<&str>) -> Result<OperatorId, TypedError> {
    parse_operator_id(value.ok_or_else(|| invalid("operatorId"))?)
}

fn parse_required_digest(value: Option<&str>, field: &'static str) -> Result<Digest, TypedError> {
    value
        .ok_or_else(|| invalid(field))?
        .parse::<String>()
        .ok()
        .and_then(|value| Digest::parse(value).ok())
        .ok_or_else(|| invalid(field))
}

fn invalid(field: &str) -> TypedError {
    TypedError::WireUnknownField {
        detail: format!("hostCutover requires valid {field}"),
    }
}

fn authorize_bound_operator(peer_uid: u32, operator_id: &OperatorId) -> Result<(), TypedError> {
    let expected_operator =
        OperatorId::new(format!("uid-{peer_uid}")).map_err(|_| invalid("operatorId"))?;
    if operator_id != &expected_operator {
        return Err(TypedError::AuthzNotAdmin {
            verb: "hostCutover".to_owned(),
        });
    }
    Ok(())
}

fn state_label(state: OperationState) -> &'static str {
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::public_wire::{HostCutoverOperation, HostCutoverRequest};

    fn request(operation: HostCutoverOperation, zone: Option<&str>) -> HostCutoverRequest {
        HostCutoverRequest {
            operation,
            operation_id: None,
            candidate_id: None,
            revision_plan_id: None,
            preview_digest: None,
            recovery_digest: None,
            operator_id: None,
            consent_digest: None,
            consent_json: None,
            destructive_consent_digest: None,
            destructive_consent_json: None,
            destroy_durable_volumes: None,
            recovery_attestation_json: None,
            host_digest: None,
            fresh_consent_digest: None,
            reason: None,
            reset_scope: None,
            target: None,
            zone: zone.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn one_time_cutover_rejects_zone_selection() {
        assert!(
            reject_zone_selection(&request(HostCutoverOperation::Preview, Some("dev"),)).is_err()
        );
        assert!(
            reject_zone_selection(&request(HostCutoverOperation::Apply, Some("dev"),)).is_err()
        );
        assert!(
            reject_zone_selection(&request(HostCutoverOperation::Verify, Some("dev"),)).is_err()
        );
        assert!(
            reject_zone_selection(&request(HostCutoverOperation::Finalize, Some("dev"),)).is_err()
        );
        assert!(reject_zone_selection(&request(HostCutoverOperation::Reset, Some("dev"),)).is_ok());
    }

    #[test]
    fn operator_binding_rejects_a_different_admin_identity() {
        let wrong = OperatorId::new("uid-2000").expect("operator");
        let error = authorize_bound_operator(1000, &wrong).expect_err("wrong operator");
        assert!(matches!(error, TypedError::AuthzNotAdmin { .. }));
    }
}
