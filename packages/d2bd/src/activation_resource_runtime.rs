//! Daemon-owned reconciliation for `NixosGeneration` resources.
//!
//! The activation Provider is a fixed daemon composition.  This module owns
//! only the Zone-scoped durable-resource adapter: it relists and watches
//! generation rows, applies the pure activation policy, and routes target
//! effects through the existing broker and guest-control boundaries.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use d2b_contracts::{
    broker_wire::{
        ApplyHostGenerationHandoffResponse, BrokerCallerRole, BrokerRequest, BrokerResponse,
    },
    host_generation::{
        ApplyHostGenerationHandoff, HandoffCallerRole, HandoffState,
        HostGenerationHandoffIntent, SourceGenerationCompatibilityFloorV1, target_fingerprint,
    },
    public_wire::{self, MutatingVerbOutcome, MutatingVerbResponse, MutationFlags},
    resource_proto as wire,
    v3::{
        ActivationDetail, ActivationMode, ActivationOutcomeCode, CanonicalJsonValue,
        NixosGenerationSpec, ResourceEnvelope, ResourcePhase, ResourceRef, ResourceTypeName,
        ZoneId, ZoneRevision,
    },
};
use d2b_provider_activation_nixos::{
    ActivationCaller, ActivationController, CallerRole, GenerationObservation, GenerationPhase,
};
use d2b_resource_api::watch::ResourceWatch;
use d2b_resource_api::{
    RedbBackend, UnregisteredResourceClient, service::UnavailableUpgradeDispatcher,
};
use d2b_resource_store::{
    StoreListRequest, StoreOperationContext, StoreProjection, StoreWatchRequest, StoredResource,
};
use d2b_resource_store_redb::RedbResourceStore;

use crate::{
    BrokerActivationMode, ServerState, dispatch_broker_request_as, dispatch_live_guest_activation,
};

const ACTIVATION_TYPE: &str = "activation-nixos.d2bus.org.NixosGeneration";
const ACTIVATION_FINALIZER: &str = "activation-nixos.d2bus.org/cleanup";
const RETAINED_GENERATIONS: usize = 3;

/// Stable failures for the daemon-owned activation path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivationResourceRuntimeError {
    /// A durable resource did not decode as the closed activation contract.
    InvalidResource,
    /// The durable store could not be listed or watched.
    Store,
    /// The activation policy refused the resource.
    Policy,
    /// A broker or target-local effect failed.
    Effect,
}

impl core::fmt::Display for ActivationResourceRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidResource => "activation-resource-invalid",
            Self::Store => "activation-resource-store-failed",
            Self::Policy => "activation-resource-policy-failed",
            Self::Effect => "activation-resource-effect-failed",
        })
    }
}

impl std::error::Error for ActivationResourceRuntimeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesiredRecord {
    resource: StoredResource,
    spec: NixosGenerationSpec,
    ordinal: u64,
}

impl DesiredRecord {
    fn key(&self) -> ResourceRef {
        self.resource.resource_ref.clone()
    }

    fn same_desired_state(&self, other: &Self) -> bool {
        self.resource.zone == other.resource.zone
            && self.resource.resource_ref == other.resource.resource_ref
            && self.resource.uid == other.resource.uid
            && self.resource.generation == other.resource.generation
            && self.spec == other.spec
            && self.ordinal == other.ordinal
    }

    fn deletion_requested(&self) -> bool {
        metadata_value(&self.resource, "deletionRequestedAt").is_some_and(|value| {
            !matches!(value, CanonicalJsonValue::Null)
        })
    }

    fn has_finalizer(&self) -> bool {
        metadata_value(&self.resource, "finalizers").is_some_and(|value| {
            matches!(
                value,
                CanonicalJsonValue::Array(values)
                    if values.iter().any(|value| {
                        matches!(value, CanonicalJsonValue::String(value) if value == ACTIVATION_FINALIZER)
                    })
            )
        })
    }
}

/// Durable activation registry for one Zone.
pub(crate) struct ActivationResourceRuntime {
    zone: ZoneId,
    controller: ActivationController,
    records: BTreeMap<ResourceRef, DesiredRecord>,
    status_client:
        Option<Arc<UnregisteredResourceClient<RedbBackend, UnavailableUpgradeDispatcher>>>,
}

impl core::fmt::Debug for ActivationResourceRuntime {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ActivationResourceRuntime")
            .field("zone", &self.zone)
            .field("record_count", &self.records.len())
            .finish()
    }
}

impl ActivationResourceRuntime {
    /// Construct a registry over the fixed activation Provider policy.
    pub(crate) fn new(zone: ZoneId) -> Self {
        Self {
            zone,
            controller: ActivationController::new(RETAINED_GENERATIONS),
            records: BTreeMap::new(),
            status_client: None,
        }
    }

    pub(crate) fn set_status_client(
        &mut self,
        status_client: Arc<UnregisteredResourceClient<RedbBackend, UnavailableUpgradeDispatcher>>,
    ) {
        self.status_client = Some(status_client);
    }

    /// Reconcile a complete durable activation snapshot.
    pub(crate) async fn reconcile(
        &mut self,
        state: Arc<ServerState>,
        snapshot: Vec<StoredResource>,
    ) -> Result<(), ActivationResourceRuntimeError> {
        let desired = decode_snapshot(&self.zone, snapshot)?;
        let desired_keys = desired.keys().cloned().collect::<BTreeSet<_>>();
        self.records.retain(|key, _| desired_keys.contains(key));
        let observations_by_target = desired.values().fold(
            BTreeMap::<ResourceRef, Vec<GenerationObservation>>::new(),
            |mut observations, record| {
                observations
                    .entry(record.spec.execution_ref().clone())
                    .or_default()
                    .push(GenerationObservation::terminal(
                        record.resource.resource_ref.name().as_str(),
                        generation_phase(
                            status_phase(&record.resource).unwrap_or(ResourcePhase::Pending),
                        ),
                        record.ordinal,
                    ));
                observations
            },
        );

        for (key, mut record) in desired {
            let replace = self
                .records
                .get(&key)
                .is_some_and(|current| !current.same_desired_state(&record));
            if replace {
                self.records.remove(&key);
            }

            if !record.deletion_requested() && !record.has_finalizer() {
                record = self.ensure_finalizer(&record).await?;
            }

            if record.deletion_requested() {
                record = self
                    .publish_status(
                        &record,
                        ResourcePhase::Deleted,
                        ActivationDetail::Superseded,
                        None,
                    )
                    .await?;
                record = self.remove_finalizer(&record).await?;
                self.records.insert(key, record);
                continue;
            }

            let phase = status_phase(&record.resource).unwrap_or(ResourcePhase::Pending);
            if matches!(phase, ResourcePhase::Ready | ResourcePhase::Succeeded) {
                self.records.insert(key, record);
                continue;
            }

            let observed = GenerationObservation::terminal(
                record.resource.resource_ref.name().as_str(),
                generation_phase(phase),
                record.ordinal,
            );
            let prior = observations_by_target
                .get(record.spec.execution_ref())
                .cloned()
                .unwrap_or_default();
            let caller = ActivationCaller::new(
                CallerRole::Lifecycle,
                record.spec.execution_ref().clone(),
            );
            let planned = self
                .controller
                .reconcile(&record.spec, &caller, &prior, observed.clone())
                .map_err(|_| ActivationResourceRuntimeError::Policy)?;

            if planned.runner_requests().is_empty() {
                if record.spec.activation_mode() == ActivationMode::Adopt
                    && !matches!(phase, ResourcePhase::Ready | ResourcePhase::Succeeded)
                {
                    let applied = self
                        .controller
                        .apply_runner_result(
                            &record.spec,
                            ActivationOutcomeCode::Adopted,
                            observed,
                        )
                        .map_err(|_| ActivationResourceRuntimeError::Policy)?;
                    record = self
                        .publish_status(
                            &record,
                            applied.phase(),
                            ActivationDetail::Adopted,
                            applied.audit_codes().first().copied(),
                        )
                        .await?;
                }
                self.records.insert(key, record);
                continue;
            }

            record = self
                .publish_status(
                    &record,
                    ResourcePhase::Pending,
                    ActivationDetail::Staged,
                    None,
                )
                .await?;
            record = self
                .publish_status(
                    &record,
                    ResourcePhase::Pending,
                    ActivationDetail::Applying,
                    None,
                )
                .await?;

            let request = planned.runner_requests()[0].clone();
            let outcome = self
                .execute_runner(&state, &record, &request, &prior)
                .await;
            let applied = self
                .controller
                .apply_runner_result(&record.spec, outcome, observed)
                .map_err(|_| ActivationResourceRuntimeError::Policy)?;
            let detail = activation_detail(
                record.spec.activation_mode(),
                outcome,
                applied.phase(),
            );
            record = self
                .publish_status(
                    &record,
                    applied.phase(),
                    detail,
                    applied.audit_codes().first().copied(),
                )
                .await?;
            self.records.insert(key, record);
        }

        self.apply_retention().await?;
        Ok(())
    }

    async fn execute_runner(
        &self,
        state: &ServerState,
        record: &DesiredRecord,
        request: &d2b_provider_activation_nixos::RunnerRequest,
        prior: &[GenerationObservation],
    ) -> ActivationOutcomeCode {
        match request.execution_ref.resource_type().as_str() {
            "Host" => self
                .execute_host_handoff(state, record, prior)
                .unwrap_or(ActivationOutcomeCode::HelperFailed),
            "Guest" => self.execute_guest_activation(state, record),
            _ => ActivationOutcomeCode::TargetMismatch,
        }
    }

    fn execute_host_handoff(
        &self,
        state: &ServerState,
        record: &DesiredRecord,
        prior: &[GenerationObservation],
    ) -> Result<ActivationOutcomeCode, ActivationResourceRuntimeError> {
        let source_generation = record
            .spec
            .prior_generation_ref()
            .and_then(|reference| {
                prior
                    .iter()
                    .find(|observation| observation.name() == reference.name().as_str())
                    .map(GenerationObservation::ordinal)
            })
            .unwrap_or_else(|| record.ordinal.saturating_sub(1));
        if source_generation == 0 || record.ordinal <= source_generation {
            return Ok(ActivationOutcomeCode::StaleGeneration);
        }
        let compatibility = SourceGenerationCompatibilityFloorV1::new(
            source_generation,
            target_fingerprint(
                record.spec.execution_ref(),
                record.spec.system_artifact_id(),
                record.ordinal,
            ),
        )
        .map_err(|_| ActivationResourceRuntimeError::Policy)?;
        let request = BrokerRequest::ApplyHostGenerationHandoff(ApplyHostGenerationHandoff {
            caller_role: HandoffCallerRole::Lifecycle,
            target: record.spec.execution_ref().clone(),
            intent: HostGenerationHandoffIntent {
                source_generation,
                target_generation: record.ordinal,
                system_artifact_id: record.spec.system_artifact_id().clone(),
                activation_mode: record.spec.activation_mode(),
                compatibility,
            },
        });
        match dispatch_broker_request_as(
            state,
            request,
            BrokerCallerRole::AdminUid {
                uid: state.daemon_uid,
            },
        ) {
            Ok(BrokerResponse::ApplyHostGenerationHandoff(response)) => {
                Ok(host_handoff_outcome(&response))
            }
            Ok(BrokerResponse::Error(_)) | Ok(_) | Err(_) => {
                Ok(ActivationOutcomeCode::HelperFailed)
            }
        }
    }

    fn execute_guest_activation(
        &self,
        state: &ServerState,
        record: &DesiredRecord,
    ) -> ActivationOutcomeCode {
        let mode = match record.spec.activation_mode() {
            ActivationMode::Switch => BrokerActivationMode::Switch,
            ActivationMode::Boot => BrokerActivationMode::Boot,
            ActivationMode::Test => BrokerActivationMode::Test,
            ActivationMode::Adopt => return ActivationOutcomeCode::Adopted,
        };
        let verb = match mode {
            BrokerActivationMode::Switch => "switch",
            BrokerActivationMode::Boot => "boot",
            BrokerActivationMode::Test => "test",
            BrokerActivationMode::Rollback => "rollback",
        };
        let response = dispatch_live_guest_activation(
            state,
            public_wire::ActivationRequest {
                vm: record.spec.execution_ref().name().as_str().to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
            },
            verb,
            mode,
            BrokerCallerRole::AdminUid {
                uid: state.daemon_uid,
            },
        );
        match response {
            Ok(value) => match serde_json::from_value::<MutatingVerbResponse>(value) {
                Ok(response) if response.outcome == MutatingVerbOutcome::Applied => {
                    ActivationOutcomeCode::Succeeded
                }
                Ok(response) if response.outcome == MutatingVerbOutcome::InvalidRequest => {
                    ActivationOutcomeCode::TargetMismatch
                }
                _ => ActivationOutcomeCode::HelperFailed,
            },
            Err(_) => ActivationOutcomeCode::HelperFailed,
        }
    }

    async fn apply_retention(&self) -> Result<(), ActivationResourceRuntimeError> {
        let observations = self
            .records
            .values()
            .map(|record| {
                GenerationObservation::terminal(
                    record.resource.resource_ref.name().as_str(),
                    generation_phase(
                        status_phase(&record.resource).unwrap_or(ResourcePhase::Pending),
                    ),
                    record.ordinal,
                )
            })
            .collect::<Vec<_>>();
        let delete_names = self.controller.retention_plan(&observations);
        for name in delete_names.delete_names() {
            if let Some(record) = self
                .records
                .values()
                .find(|record| record.resource.resource_ref.name().as_str() == name)
                && !record.deletion_requested()
            {
                self.request_delete(record).await?;
            }
        }
        Ok(())
    }

    async fn publish_status(
        &self,
        record: &DesiredRecord,
        phase: ResourcePhase,
        detail: ActivationDetail,
        outcome: Option<ActivationOutcomeCode>,
    ) -> Result<DesiredRecord, ActivationResourceRuntimeError> {
        let Some(client) = &self.status_client else {
            return Ok(record.clone());
        };
        update_status(client, record, phase, detail, outcome).await
    }

    async fn ensure_finalizer(
        &self,
        record: &DesiredRecord,
    ) -> Result<DesiredRecord, ActivationResourceRuntimeError> {
        let Some(client) = &self.status_client else {
            return Ok(record.clone());
        };
        update_finalizers(client, record, true).await
    }

    async fn remove_finalizer(
        &self,
        record: &DesiredRecord,
    ) -> Result<DesiredRecord, ActivationResourceRuntimeError> {
        let Some(client) = &self.status_client else {
            return Ok(record.clone());
        };
        if !record.has_finalizer() {
            return Ok(record.clone());
        }
        update_finalizers(client, record, false).await
    }

    async fn request_delete(
        &self,
        record: &DesiredRecord,
    ) -> Result<(), ActivationResourceRuntimeError> {
        let Some(client) = &self.status_client else {
            return Ok(());
        };
        delete_resource(client, record).await
    }
}

fn host_handoff_outcome(response: &ApplyHostGenerationHandoffResponse) -> ActivationOutcomeCode {
    match response.state {
        HandoffState::Completed => {
            if response.target_generation == response.source_generation {
                ActivationOutcomeCode::StaleGeneration
            } else {
                ActivationOutcomeCode::Succeeded
            }
        }
        HandoffState::Refused => ActivationOutcomeCode::HelperRefused,
        HandoffState::RolledBack => ActivationOutcomeCode::RolledBack,
        _ => ActivationOutcomeCode::HelperFailed,
    }
}

fn activation_detail(
    mode: ActivationMode,
    outcome: ActivationOutcomeCode,
    phase: ResourcePhase,
) -> ActivationDetail {
    if outcome == ActivationOutcomeCode::Adopted {
        return ActivationDetail::Adopted;
    }
    if outcome == ActivationOutcomeCode::RolledBack {
        return ActivationDetail::RolledBack;
    }
    if outcome.is_success() {
        return match mode {
            ActivationMode::Boot => ActivationDetail::BootDefault,
            ActivationMode::Switch | ActivationMode::Test => ActivationDetail::Applied,
            ActivationMode::Adopt => ActivationDetail::Adopted,
        };
    }
    if phase == ResourcePhase::Ready {
        ActivationDetail::Superseded
    } else {
        ActivationDetail::Planning
    }
}

fn generation_phase(phase: ResourcePhase) -> GenerationPhase {
    match phase {
        ResourcePhase::Pending => GenerationPhase::Pending,
        ResourcePhase::Ready => GenerationPhase::Ready,
        ResourcePhase::Succeeded => GenerationPhase::Succeeded,
        ResourcePhase::Failed => GenerationPhase::Failed,
        ResourcePhase::Degraded => GenerationPhase::Degraded,
        ResourcePhase::Deleted => GenerationPhase::Deleted,
        ResourcePhase::Unknown => GenerationPhase::Pending,
    }
}

fn metadata_value(resource: &StoredResource, key: &str) -> Option<CanonicalJsonValue> {
    let value = CanonicalJsonValue::parse(&resource.canonical_json).ok()?;
    let CanonicalJsonValue::Object(root) = value else {
        return None;
    };
    let CanonicalJsonValue::Object(metadata) = root.get("metadata")? else {
        return None;
    };
    metadata.get(key).cloned()
}

fn status_phase(resource: &StoredResource) -> Option<ResourcePhase> {
    let value = CanonicalJsonValue::parse(&resource.canonical_json).ok()?;
    let CanonicalJsonValue::Object(root) = value else {
        return None;
    };
    let CanonicalJsonValue::Object(status) = root.get("status")? else {
        return None;
    };
    let CanonicalJsonValue::String(phase) = status.get("phase")? else {
        return None;
    };
    match phase.as_str() {
        "Pending" => Some(ResourcePhase::Pending),
        "Ready" => Some(ResourcePhase::Ready),
        "Succeeded" => Some(ResourcePhase::Succeeded),
        "Degraded" => Some(ResourcePhase::Degraded),
        "Failed" => Some(ResourcePhase::Failed),
        "Deleted" => Some(ResourcePhase::Deleted),
        "Unknown" => Some(ResourcePhase::Unknown),
        _ => None,
    }
}

fn ordinal_from_resource(resource: &StoredResource) -> u64 {
    resource
        .resource_ref
        .name()
        .as_str()
        .rsplit('-')
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| resource.generation.get())
}

fn decode_snapshot(
    zone: &ZoneId,
    resources: Vec<StoredResource>,
) -> Result<BTreeMap<ResourceRef, DesiredRecord>, ActivationResourceRuntimeError> {
    let mut desired = BTreeMap::new();
    for resource in resources {
        if resource.zone != *zone {
            return Err(ActivationResourceRuntimeError::InvalidResource);
        }
        if resource.resource_ref.resource_type().as_str() != ACTIVATION_TYPE {
            continue;
        }
        let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
            .map_err(|_| ActivationResourceRuntimeError::InvalidResource)?;
        let spec = serde_json::from_slice::<NixosGenerationSpec>(
            &envelope.spec().base().to_canonical_bytes(),
        )
        .map_err(|_| ActivationResourceRuntimeError::InvalidResource)?;
        let record = DesiredRecord {
            ordinal: ordinal_from_resource(&resource),
            resource,
            spec,
        };
        if desired.insert(record.key(), record).is_some() {
            return Err(ActivationResourceRuntimeError::InvalidResource);
        }
    }
    Ok(desired)
}

fn phase_json(phase: ResourcePhase) -> CanonicalJsonValue {
    CanonicalJsonValue::String(
        match phase {
            ResourcePhase::Pending => "Pending",
            ResourcePhase::Ready => "Ready",
            ResourcePhase::Succeeded => "Succeeded",
            ResourcePhase::Degraded => "Degraded",
            ResourcePhase::Failed => "Failed",
            ResourcePhase::Deleted => "Deleted",
            ResourcePhase::Unknown => "Unknown",
        }
        .to_owned(),
    )
}

fn detail_json(detail: ActivationDetail) -> CanonicalJsonValue {
    CanonicalJsonValue::String(
        match detail {
            ActivationDetail::Planning => "planning",
            ActivationDetail::Staged => "staged",
            ActivationDetail::Applying => "applying",
            ActivationDetail::Applied => "applied",
            ActivationDetail::BootDefault => "boot-default",
            ActivationDetail::Adopted => "adopted",
            ActivationDetail::RolledBack => "rolled-back",
            ActivationDetail::Superseded => "superseded",
        }
        .to_owned(),
    )
}

fn outcome_json(outcome: ActivationOutcomeCode) -> CanonicalJsonValue {
    CanonicalJsonValue::String(
        match outcome {
            ActivationOutcomeCode::Succeeded => "succeeded",
            ActivationOutcomeCode::Adopted => "adopted",
            ActivationOutcomeCode::Unauthorized => "unauthorized",
            ActivationOutcomeCode::StaleGeneration => "stale-generation",
            ActivationOutcomeCode::TargetMismatch => "target-mismatch",
            ActivationOutcomeCode::HelperRefused => "helper-refused",
            ActivationOutcomeCode::HelperFailed => "helper-failed",
            ActivationOutcomeCode::RolledBack => "rolled-back",
        }
        .to_owned(),
    )
}

fn now_timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let seconds = millis / 1_000;
    let day = seconds / 86_400;
    let day_seconds = seconds % 86_400;
    let (year, month, day_of_month) = civil_from_days(day as i64);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!(
        "{year:04}-{month:02}-{day_of_month:02}T{hour:02}:{minute:02}:{second:02}.{:03}Z",
        millis % 1_000
    )
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

async fn update_status(
    client: &UnregisteredResourceClient<RedbBackend, UnavailableUpgradeDispatcher>,
    record: &DesiredRecord,
    phase: ResourcePhase,
    detail: ActivationDetail,
    outcome: Option<ActivationOutcomeCode>,
) -> Result<DesiredRecord, ActivationResourceRuntimeError> {
    let canonical = status_payload(record, phase, detail, outcome)?;
    let envelope = ResourceEnvelope::from_json(&canonical)
        .map_err(|_| ActivationResourceRuntimeError::InvalidResource)?;
    let digest = envelope
        .digest()
        .map_err(|_| ActivationResourceRuntimeError::InvalidResource)?;
    let mut resource = wire::ResourceEnvelopeBytes::new();
    resource.identity = protobuf::MessageField::some(resource_identity(record));
    resource.canonical_json = canonical;
    resource.payload_digest = digest;

    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_STATUS);
    mutation.target = protobuf::MessageField::some(resource_identity(record));
    mutation.precondition = protobuf::MessageField::some(exact_precondition(record));
    mutation.resource = protobuf::MessageField::some(resource);
    let operation = format!(
        "activation-runtime-status-{}-{}",
        record.key().to_canonical_string(),
        record.resource.revision.get()
    );
    let mut request = wire::UpdateStatusRequest::new();
    request.meta = protobuf::MessageField::some(request_meta(&operation));
    request.mutation = protobuf::MessageField::some(mutation);
    let response = client.update_status(request).await;
    if response.error.is_some() {
        return Err(ActivationResourceRuntimeError::Store);
    }
    let response_resource = response
        .resource
        .as_ref()
        .ok_or(ActivationResourceRuntimeError::Store)?;
    let mut updated = record.clone();
    updated.resource.canonical_json = response_resource.canonical_json.clone();
    updated.resource.payload_digest = response_resource.payload_digest.clone();
    updated.resource.revision = ZoneRevision::new(response.revision);
    Ok(updated)
}

fn status_payload(
    record: &DesiredRecord,
    phase: ResourcePhase,
    detail: ActivationDetail,
    outcome: Option<ActivationOutcomeCode>,
) -> Result<Vec<u8>, ActivationResourceRuntimeError> {
    let mut value = CanonicalJsonValue::parse(&record.resource.canonical_json)
        .map_err(|_| ActivationResourceRuntimeError::InvalidResource)?;
    let CanonicalJsonValue::Object(root) = &mut value else {
        return Err(ActivationResourceRuntimeError::InvalidResource);
    };
    let Some(CanonicalJsonValue::Object(status)) = root.get_mut("status") else {
        return Err(ActivationResourceRuntimeError::InvalidResource);
    };
    let now = now_timestamp();
    status.insert("phase".to_owned(), phase_json(phase));
    status.insert(
        "observedGeneration".to_owned(),
        CanonicalJsonValue::Integer(record.resource.generation.get() as i64),
    );
    status.insert(
        "lastReconciledAt".to_owned(),
        CanonicalJsonValue::String(now.clone()),
    );
    status.insert(
        "outcome".to_owned(),
        outcome
            .map(|outcome| {
                let mut result = BTreeMap::new();
                result.insert(
                    "code".to_owned(),
                    outcome_json(outcome),
                );
                result.insert(
                    "message".to_owned(),
                    CanonicalJsonValue::String(activation_outcome_message(outcome).to_owned()),
                );
                result.insert(
                    "retryable".to_owned(),
                    CanonicalJsonValue::Bool(false),
                );
                result.insert(
                    "occurredAt".to_owned(),
                    CanonicalJsonValue::String(now.clone()),
                );
                CanonicalJsonValue::Object(result)
            })
            .unwrap_or(CanonicalJsonValue::Null),
    );
    let resource_status = match status.get_mut("resource") {
        Some(CanonicalJsonValue::Object(resource_status)) => resource_status,
        Some(_) => return Err(ActivationResourceRuntimeError::InvalidResource),
        None => {
            status.insert(
                "resource".to_owned(),
                CanonicalJsonValue::Object(BTreeMap::new()),
            );
            match status.get_mut("resource") {
                Some(CanonicalJsonValue::Object(resource_status)) => resource_status,
                _ => return Err(ActivationResourceRuntimeError::InvalidResource),
            }
        }
    };
    resource_status.insert("activationDetail".to_owned(), detail_json(detail));
    resource_status.insert(
        "observedGeneration".to_owned(),
        CanonicalJsonValue::Integer(record.resource.generation.get() as i64),
    );
    if let Some(outcome) = outcome {
        resource_status.insert("outcome".to_owned(), outcome_json(outcome));
    } else {
        resource_status.remove("outcome");
    }
    let canonical = value.to_canonical_bytes();
    ResourceEnvelope::from_json(&canonical)
        .map_err(|_| ActivationResourceRuntimeError::InvalidResource)?;
    Ok(canonical)
}

fn activation_outcome_message(outcome: ActivationOutcomeCode) -> &'static str {
    match outcome {
        ActivationOutcomeCode::Succeeded => "target generation activated",
        ActivationOutcomeCode::Adopted => "existing target generation adopted",
        ActivationOutcomeCode::Unauthorized => "activation caller was not authorized",
        ActivationOutcomeCode::StaleGeneration => "target generation is stale",
        ActivationOutcomeCode::TargetMismatch => "activation target did not match",
        ActivationOutcomeCode::HelperRefused => "target activation helper refused the request",
        ActivationOutcomeCode::HelperFailed => "target activation helper failed",
        ActivationOutcomeCode::RolledBack => "target activation rolled back to the source",
    }
}

async fn update_finalizers(
    client: &UnregisteredResourceClient<RedbBackend, UnavailableUpgradeDispatcher>,
    record: &DesiredRecord,
    add: bool,
) -> Result<DesiredRecord, ActivationResourceRuntimeError> {
    let mut mutation = wire::Mutation::new();
    mutation.kind =
        protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_FINALIZERS);
    mutation.target = protobuf::MessageField::some(resource_identity(record));
    mutation.precondition = protobuf::MessageField::some(exact_precondition(record));
    if add {
        mutation.add_finalizers.push(ACTIVATION_FINALIZER.to_owned());
    } else {
        mutation
            .remove_finalizers
            .push(ACTIVATION_FINALIZER.to_owned());
    }
    let operation = format!(
        "activation-runtime-finalizer-{}-{}-{}",
        record.key().to_canonical_string(),
        record.resource.revision.get(),
        if add { "add" } else { "remove" }
    );
    let mut request = wire::UpdateFinalizersRequest::new();
    request.meta = protobuf::MessageField::some(request_meta(&operation));
    request.mutation = protobuf::MessageField::some(mutation);
    let response = client.update_finalizers(request).await;
    if response.error.is_some() {
        return Err(ActivationResourceRuntimeError::Store);
    }
    let response_resource = response
        .resource
        .as_ref()
        .ok_or(ActivationResourceRuntimeError::Store)?;
    let mut updated = record.clone();
    updated.resource.canonical_json = response_resource.canonical_json.clone();
    updated.resource.payload_digest = response_resource.payload_digest.clone();
    updated.resource.revision = ZoneRevision::new(response.revision);
    Ok(updated)
}

async fn delete_resource(
    client: &UnregisteredResourceClient<RedbBackend, UnavailableUpgradeDispatcher>,
    record: &DesiredRecord,
) -> Result<(), ActivationResourceRuntimeError> {
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_DELETE);
    mutation.target = protobuf::MessageField::some(resource_identity(record));
    mutation.precondition = protobuf::MessageField::some(exact_precondition(record));
    let operation = format!(
        "activation-runtime-delete-{}-{}",
        record.key().to_canonical_string(),
        record.resource.revision.get()
    );
    let mut request = wire::DeleteRequest::new();
    request.meta = protobuf::MessageField::some(request_meta(&operation));
    request.mutation = protobuf::MessageField::some(mutation);
    let response = client.delete(request).await;
    if response.error.is_some() {
        return Err(ActivationResourceRuntimeError::Store);
    }
    Ok(())
}

fn resource_identity(record: &DesiredRecord) -> wire::ResourceIdentity {
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = record.resource.zone.to_canonical_string();
    identity.resource_type = record
        .resource
        .resource_ref
        .resource_type()
        .to_canonical_string();
    identity.name = record.resource.resource_ref.name().to_canonical_string();
    identity.uid = Some(record.resource.uid.as_str().to_owned());
    identity.generation = Some(record.resource.generation.get());
    identity.revision = Some(record.resource.revision.get());
    identity
}

fn exact_precondition(record: &DesiredRecord) -> wire::Precondition {
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(record.resource.revision.get());
    precondition.expected_uid = Some(record.resource.uid.as_str().to_owned());
    precondition
}

fn request_meta(operation: &str) -> wire::RequestMeta {
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = operation.to_owned();
    meta.idempotency_key = operation.to_owned();
    meta.correlation_id = operation.to_owned();
    meta.trace_id = operation.to_owned();
    meta.deadline_ms = 10_000;
    meta
}

/// Build the generic activation relist request.
pub(crate) fn activation_list_request(zone: &ZoneId) -> StoreListRequest {
    StoreListRequest {
        operation: StoreOperationContext {
            operation_id: "activation-resource-reconcile".to_owned(),
            idempotency_key: None,
            correlation_id: "activation-resource-reconcile".to_owned(),
            trace_id: None,
            deadline_ms: 10_000,
        },
        zone: zone.clone(),
        resource_types: vec![
            ResourceTypeName::parse(ACTIVATION_TYPE).expect("static activation type"),
        ],
        resource_names: Vec::new(),
        filters: Vec::new(),
        page_size: 256,
        cursor: None,
        projection: StoreProjection::Full,
    }
}

/// Build the generic activation watch request.
pub(crate) fn activation_watch_request(zone: &ZoneId) -> StoreWatchRequest {
    StoreWatchRequest {
        operation: StoreOperationContext {
            operation_id: "activation-resource-watch".to_owned(),
            idempotency_key: None,
            correlation_id: "activation-resource-watch".to_owned(),
            trace_id: None,
            deadline_ms: 10_000,
        },
        zone: zone.clone(),
        resource_types: vec![
            ResourceTypeName::parse(ACTIVATION_TYPE).expect("static activation type"),
        ],
        resource_names: Vec::new(),
        filters: Vec::new(),
        after_revision: ZoneRevision::new(0),
        initial_credits: 64,
        projection: StoreProjection::Full,
    }
}

/// Relist all activation resources, preserving snapshot pagination.
pub(crate) async fn list_activation_snapshot(
    store: &RedbResourceStore,
    zone: &ZoneId,
) -> Result<Vec<StoredResource>, ActivationResourceRuntimeError> {
    let mut request = activation_list_request(zone);
    let mut resources = Vec::new();
    loop {
        let result = store
            .list(request.clone())
            .await
            .map_err(|_| ActivationResourceRuntimeError::Store)?;
        resources.extend(result.resources);
        let Some(cursor) = result.next_cursor else {
            break;
        };
        request.cursor = Some(cursor);
    }
    Ok(resources)
}

/// Run the relist/watch reconciliation loop for one Zone.
pub(crate) async fn run_activation_watch(
    mut watch: ResourceWatch,
    store: Arc<RedbResourceStore>,
    zone: ZoneId,
    state: Arc<ServerState>,
    registry: Arc<Mutex<Option<ActivationResourceRuntime>>>,
) {
    loop {
        match tokio::time::timeout(Duration::from_secs(1), watch.recv()).await {
            Ok(Some(batch)) => {
                if watch.acknowledge(batch.revision()).await.is_err() {
                    return;
                }
            }
            Ok(None) => {
                if watch.resume().await.is_err() {
                    return;
                }
            }
            Err(_) => {}
        }
        let Ok(snapshot) = list_activation_snapshot(&store, &zone).await else {
            continue;
        };
        let runtime = match registry.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => return,
        };
        let Some(mut runtime) = runtime else {
            continue;
        };
        let result = runtime.reconcile(Arc::clone(&state), snapshot).await;
        if let Ok(mut guard) = registry.lock() {
            *guard = Some(runtime);
        } else {
            return;
        }
        if result.is_err() {
            tracing::warn!(zone = %zone.as_str(), "activation resource reconciliation degraded");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_requests_are_zone_scoped_and_qualified() {
        let zone = ZoneId::parse("dev").expect("valid zone");
        let request = activation_list_request(&zone);
        assert_eq!(request.resource_types.len(), 1);
        assert_eq!(request.resource_types[0].as_str(), ACTIVATION_TYPE);
        assert_eq!(request.zone, zone);
    }

    #[test]
    fn generation_ordinals_are_taken_from_bounded_names() {
        let resource_ref =
            ResourceRef::parse("activation-nixos.d2bus.org.NixosGeneration/dev-vm--gen-7")
                .expect("valid generation reference");
        let resource = StoredResource {
            resource_ref,
            zone: ZoneId::parse("dev").expect("zone"),
            uid: d2b_contracts::v3::ResourceUid::parse(
                "123e4567-e89b-42d3-a456-426614174000",
            )
            .expect("uid"),
            generation: d2b_contracts::v3::ResourceGeneration::new(1).expect("generation"),
            revision: ZoneRevision::new(1),
            canonical_json: Vec::new(),
            payload_digest: "sha256:".to_owned(),
        };
        assert_eq!(ordinal_from_resource(&resource), 7);
    }
}
