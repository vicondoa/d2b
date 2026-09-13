//! Production Zone resource-plane ownership for `d2bd`.
//!
//! U14: the durable store is gone. A Zone runtime opens from its verified
//! bundle authority alone and is completed when the composition publishes
//! the Zone's manager plane ([`ZoneResourceRuntime::attach_v3_planes`], then
//! [`ZoneResourceRuntime::activate_published_bundle`]). The runtime owns the
//! API, core-process readiness, and restart lifecycle as one Zone-scoped
//! value.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(test)]
use std::sync::atomic::AtomicUsize;

use crate::audio_resource_runtime::{AudioBindingRuntimeStatus, AudioResourceRuntime};
use crate::credential_driver::{
    AgentReadyFuture, CredentialDependencyFacts, CredentialDriverEffects,
    ProductionCredentialDriverEffects,
};
use crate::credential_resource_runtime::{
    CredentialSession, CredentialSessionRegistry, ComponentCredentialSession,
    is_credential_provider_ref,
};
use async_trait::async_trait;
use d2b_bus::{
    BusAuthorizer, BusConfig, BusIngress, CommittedControllerProcessSubjectInput,
    CommittedInteractionSubjectInstall, CommittedInteractionSubjectIssuer, ZoneBus, ZoneRegistrar,
};
#[cfg(test)]
use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_contracts_provider::v3::provider::ProviderSpec;
use d2b_contracts_resource::resource_proto as wire;
use d2b_contracts_resource::v3::identity::{
    AuthenticatedSubjectContext, EvidenceClass, ReconnectGeneration,
};
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ControllerGeneration, DesiredLifecycle,
    PlacementTargetKind, ResourceBundleGenerationId, ResourceEnvelope, ResourceGeneration,
    ResourceErrorKind, ResourcePhase, ResourceRef, ResourceTypeName, ResourceUid,
    ZoneId, ZoneRevision,
    process::ProcessSpec,
    volume::VolumeSpec,
};
use d2b_contracts_resource::v3::guest::GuestSpec;
use d2b_contracts_zone_session::v3::{ZoneStatusResource, resource_bundle::ResourceBundle};
use d2b_core_controller::authority::{
    AuthorityOperationState, AuthorityRequest, AuthorityReservation, ExternalNicClaimRequest,
    ExternalNicReservation, HostGlobalAuthorityIndex,
};
use d2b_core_controller::authority_persistence::{
    AuthorityFuture, AuthorityPersistence, AuthorityPersistenceError, AuthorityRecoveryCoordinator,
};
use d2b_core_controller::controller_assignment::{
    AssignmentError, AssignmentIdentity, AssignmentPhase, AssignmentRequest, AssignmentTarget,
    CONTROLLER_ASSIGNMENT_STREAM_CREDIT, CONTROLLER_ASSIGNMENT_STREAM_ID,
    ControllerAssignmentGrant, ControllerRoleContract, ControllerSessionBinding,
    ResourceClientLease,
};
use d2b_core_controller::controllers::HandlerPhase;
use d2b_core_controller::main::{
    CoreProcess, RecoverySnapshot, RuntimeReadiness as CoreRuntimeReadiness, StartupStage,
};
use d2b_core_controller::migration::LegacyTpmMigrationDecision;
use d2b_core_controller::zone_status::{
    SystemCoreStatusEmitter, ZoneRuntimeMetadata, ZoneStatusInput,
};
use d2b_provider_clipboard_wayland::Policy as ClipboardPolicy;
use d2b_provider_display_wayland::WaylandSessionSpec;
use d2b_provider_network_local::{
    ExternalNicAdmissionError, ExternalNicClaim, admit_external_nic_claims,
    controller::{
        NetworkAdmissionIntent, NetworkAdmissionKey, NetworkAdmissionProof,
        NetworkEffectError,
    },
    observe::HostNetworkOccupancy,
    routes::RouteTuple,
};
use d2b_provider_notification_desktop::{Category, GuestSourceConfig, NotificationProviderConfig};
use d2b_provider_toolkit::{
    PROVIDER_BOOTSTRAP_STREAM_CREDIT, PROVIDER_BOOTSTRAP_STREAM_ID,
    PROVIDER_DELIVERY_KEY_STREAM_CREDIT, PROVIDER_DELIVERY_KEY_STREAM_ID, PROVIDER_READY_MARKER,
    PROVIDER_READY_STREAM_CREDIT, PROVIDER_READY_STREAM_ID, ProviderSessionMetadata,
};
use d2b_provider_runtime_cloud_hypervisor::{
    AuthenticatedResourceApiAdapter, AuthenticatedResourceSession, BootstrapGraph, ChildRole,
    CloudHypervisorConfig, CloudHypervisorController, CloudHypervisorResourceApiError,
    CloudHypervisorResourceRequest, CloudHypervisorResourceResponse, FencedChild,
    GuestChildCommitResponse, GuestDependencySnapshot, GuestFinalizationInput, GuestGenerationSet,
    GuestSessionEvidence, GuestSessionEvidenceBinding, GuestSetupDescriptor,
    GuestSetupDescriptorVerifier, GuestSnapshot, OwnedChildSnapshot, ProcessState, SessionState,
    VerifiedGuestSetupDescriptor, deterministic_child_ref,
};
use d2b_resource_api::{
    ResourceApiClient, ResourceBusAdapter, ResourceService,
    authz::{AuthorizationState, BoundSubject, NativeAuthorizer, PolicySet},
    service::UnavailableUpgradeDispatcher,
};
use d2b_contracts_resource::v3::{PolicySnapshot, StoredResource, Timestamp};
use d2b_session::{
    ComponentSessionDriver, HandshakeCredentials, OwnedTransport, SessionDriverHandle,
    SessionEngine, SessionServerError, StreamEvent, StreamId, TransportEvidence,
};
use d2b_session_unix::{
    AncillaryCapacity, CONTROLLER_BOOTSTRAP_TIMEOUT, PeerCredentials, SeqpacketSocket,
    VerifiedUnixPeer, controller_bootstrap_attachment_policy, controller_credit_scopes,
    controller_resource_endpoint_policy, credential_provider_endpoint_policy,
};
use d2bd_runtime::authority_persistence::{
    AuthorityOwnerProvenance, ZoneAuthorityLedger,
};
pub use d2bd_runtime::resource_api::ResourceRuntimeError;
use d2bd_runtime::resource_api::{parse_list_request, route_service_matches};
use d2bd_runtime::resource_operator_activation::{
    Wave6AcceptanceReport, Wave6Dependencies, Wave6ProviderBoundary, Wave6ReconcileResult,
    select_wave6_resources,
};
use d2bd_runtime::resource_runtime_support::{
    AssignmentRegistry, PolicySubjectFingerprint, SystemCoreReconcileResult, ZoneApiBackend,
    configuration_cleanup_pending, current_status_timestamp, encode_public_get_response,
    encode_public_list_response, encode_public_resource, handler_phase_to_zone_phase,
    initial_policy_snapshot, map_startup_error, new_assignment_registry, public_list_request,
    public_operation_id, public_request_meta, refreshed_policy_subject_fingerprints,
    register_system_core_session, runtime_authorizer, runtime_policy, unix_transport,
};
use d2bd_runtime::guest_component_session::COMPONENT_SESSION_RETRY_BACKOFF;
pub use d2bd_runtime::resource_runtime_support::{ZoneRuntimeReadiness, bounded_operation_id};
use d2bd_runtime::target_runtime::{DaemonMode, ProviderDeployment};
use d2bd_runtime::zone_authority::{
    ZONE_GENERATION_PUBLICATION_OPERATION_PREFIX, ZoneAuthorityIdentity,
    complete_generation_set_digest,
};
use protobuf::{EnumOrUnknown, MessageField};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod volume_effect_adapter;
pub(crate) mod plane_controller_bridge;
pub(crate) mod interaction_effects;
use plane_controller_bridge::{
    ChildMutationFailure, ChildMutationRoute, ControllerPlaneView, LiveControllerSessionEvidence,
    ManagerControllerPlaneView, PlaneChildMutations, PublishedPlaneControllerView,
    child_mutation_route, child_type_route,
};
use d2b_resource_runtime::manager::ResourceView;
pub use volume_effect_adapter::{
    AnchoredVolumeEffectAdapter, FdRootResolver, ResolvedVolumeRoot, VolumeRootResolver,
};
pub(crate) use interaction_effects::ProductionInteractionDriverEffects;
use crate::interaction_driver::{INTERACTION_PROVIDER_REFS, INTERACTION_TYPES};

/// Bounded attempts when a policy-input change races the authorization
/// policy projection refresh. The projection compiles the committed policy
/// rows against the policy snapshot; a policy change landing under the
/// refresh must not surface as a failed public mutation.
const POLICY_REFRESH_ATTEMPTS: u32 = 8;
const POLICY_REFRESH_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(50);

fn trusted_provider_resource_types() -> Result<Vec<ResourceTypeName>, ResourceRuntimeError> {
    let mut resource_types = BTreeSet::new();
    for resource_type in crate::interaction_driver::INTERACTION_TYPES
        .iter()
        .copied()
        .filter(|resource_type| resource_type.contains(".d2bus.org."))
    {
        resource_types.insert(
            ResourceTypeName::parse(resource_type.to_owned())
                .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
        );
    }
    Ok(resource_types.into_iter().collect())
}

fn trusted_catalog_resource_types(
    resource_types: impl IntoIterator<Item = ResourceTypeName>,
) -> Result<Vec<ResourceTypeName>, ResourceRuntimeError> {
    // Qualified API extensions come only from trusted driver declarations.
    let trusted_provider_types = trusted_provider_resource_types()?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut catalog_resource_types = Vec::new();
    for resource_type in resource_types {
        if resource_type.as_str().contains(".d2bus.org.")
            && !trusted_provider_types.contains(&resource_type)
        {
            return Err(ResourceRuntimeError::AuthorizationUnavailable);
        }
        catalog_resource_types.push(resource_type);
    }
    catalog_resource_types.extend(trusted_provider_types);
    catalog_resource_types.sort();
    catalog_resource_types.dedup();
    Ok(catalog_resource_types)
}

/// Compat assignment-epoch value written into every newly constructed
/// assignment fence and stored AssignmentRecord. Epochs no longer take part
/// in any decision (succession is read off provider/controller/session
/// generations plus resource revisions, and reconnect reconciliation adopts
/// every fence not strictly newer); the constant only keeps the retained
/// schema and stored-record validation (a nonzero epoch) intact.
pub(super) const ASSIGNMENT_EPOCH: u64 = 1;

/// The manager rows of one type, rendered through the same projection the
/// manager-backed API serves (U12 reader bridge, mirroring G5).
///
/// A manager RPC failure is an error - never reported as absence.
async fn bridge_manager_rows(
    plane: &dyn ControllerPlaneView,
    resource_type: &str,
) -> Result<Vec<StoredResource>, ResourceRuntimeError> {
    let views = plane.rows_of_type(resource_type).await.map_err(|error| {
        tracing::warn!(
            resource_type,
            error = %error,
            "manager reader bridge: row list failed",
        );
        ResourceRuntimeError::StoreReadFailed
    })?;
    views
        .iter()
        .map(|view| {
            d2b_resource_api::manager_backend::manager_row_stored(view)
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)
        })
        .collect()
}

/// One row read from its authority (the manager).
///
/// - the manager holds the row: it is returned;
/// - the manager does not hold it: `Ok(None)` - the honest not-committed
///   answer, never a store `ResourceNotFound` for a row the store does not
///   own;
/// - the manager RPC fails: an error the caller retries - never absence.
async fn bridge_manager_row(
    plane: &dyn ControllerPlaneView,
    target: &ResourceRef,
) -> Result<Option<StoredResource>, ResourceRuntimeError> {
    Ok(bridge_manager_rows(plane, target.resource_type().as_str())
        .await?
        .into_iter()
        .find(|row| row.resource_ref == *target))
}

/// The committed policy inputs for one Zone: the manager-served rows of the
/// closed policy type set (U12 bridge).
///
/// Role/RoleBinding/Zone/Provider and the subject rows are manager rows, so
/// without them a RoleBinding whose Role row moved returns
/// `AuthorizationUnavailable` and its subjects are dropped - i.e. the Zone's
/// committed policy would empty out. A miss stays the loud, fail-closed
/// compile failure it is today.
async fn committed_policy_resources(
    plane: &dyn ControllerPlaneView,
) -> Result<Vec<StoredResource>, ResourceRuntimeError> {
    let mut resources = Vec::new();
    for resource_type in d2bd_runtime::resource_runtime_support::COMMITTED_POLICY_RESOURCE_TYPES {
        resources.extend(bridge_manager_rows(plane, resource_type).await?);
    }
    Ok(resources)
}

/// The committed identities of the requested `Provider` refs, read from the
/// manager.
///
/// The controller session/policy paths compare a bootstrap context against
/// the committed Provider identity, and `Provider` is a manager row now.
async fn committed_controller_provider_identities(
    zone: &ZoneId,
    plane: &dyn ControllerPlaneView,
    provider_refs: BTreeSet<ResourceRef>,
) -> Result<BTreeMap<ResourceRef, (ResourceUid, ResourceGeneration)>, ResourceRuntimeError> {
    if provider_refs.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut identities = BTreeMap::new();
    for row in bridge_manager_rows(plane, "Provider").await? {
        if !provider_refs.contains(&row.resource_ref) {
            continue;
        }
        let expected_ref = row.resource_ref.clone();
        let (_, uid, generation, _, _) = committed_provider_spec(zone, &row, &expected_ref)?;
        identities.insert(expected_ref, (uid, generation));
    }
    Ok(identities)
}

async fn credential_dependency_facts(
    plane: &dyn ControllerPlaneView,
    provider_ref: &ResourceRef,
    execution_ref: &ResourceRef,
) -> Option<CredentialDependencyFacts> {
    let provider = credential_dependency_row(plane, provider_ref).await?;
    let execution = credential_dependency_row(plane, execution_ref).await?;
    Some(CredentialDependencyFacts {
        provider_uid: provider.uid.as_str().to_owned(),
        provider_generation: provider.generation.get(),
        provider_ready: credential_row_ready(&provider),
        execution_ready: credential_row_ready(&execution),
    })
}

async fn credential_dependency_row(
    plane: &dyn ControllerPlaneView,
    target: &ResourceRef,
) -> Option<StoredResource> {
    bridge_manager_row(plane, target).await.ok().flatten()
}


/// The policy snapshot the committed manager rows imply (U14).
///
/// The durable store metadata that used to carry a Zone's policy snapshot is
/// gone, so the snapshot a compile is fenced at is derived from the very rows
/// being compiled: the bootstrap snapshot supplies the fixed catalog and
/// configuration revisions, and the highest committed row generation becomes
/// the policy revision. The derivation is not monotone on its own - removing
/// the row that carried the highest generation lowers the maximum - so a
/// compile that installs must hold the revision through
/// [`PolicyProjection::next_policy_revision`].
fn policy_snapshot_for_rows(
    resources: &[StoredResource],
) -> Result<PolicySnapshot, ResourceRuntimeError> {
    let mut snapshot = initial_policy_snapshot()?;
    if let Some(generation) = resources
        .iter()
        .map(|resource| resource.generation.get())
        .max()
    {
        snapshot.policy_revision = generation.max(snapshot.policy_revision);
    }
    Ok(snapshot)
}

/// The digest of the committed policy-input rows (U14).
///
/// The derived revision alone cannot tell a changed row set from an unchanged
/// one: removing the row that carried the highest generation - the Guest row
/// when its teardown completes - lowers it, and a row that is only tombstoned
/// keeps its generation and uid. The digest is the projection's change
/// signal, over exactly the row facts the compile reads: identity, desired
/// spec, generation, and the tombstone phase that decides subject evidence.
/// Status churn the policy does not consume stays out of it, so a refresh
/// that finds nothing to compile keeps the installed projection.
fn policy_inputs_digest(resources: &[StoredResource]) -> [u8; 32] {
    let mut rows = resources.iter().collect::<Vec<_>>();
    rows.sort_by(|left, right| left.resource_ref.cmp(&right.resource_ref));
    let mut digest = Sha256::new();
    for resource in rows {
        let (spec_digest, tombstone) = policy_input_content(resource);
        digest.update(resource.generation.get().to_le_bytes());
        digest.update(tombstone);
        for field in [
            resource.zone.as_str(),
            resource.resource_ref.resource_type().as_str(),
            resource.resource_ref.name().as_str(),
            resource.uid.as_str(),
        ] {
            digest.update((field.len() as u64).to_le_bytes());
            digest.update(field.as_bytes());
        }
        digest.update(spec_digest);
    }
    digest.finalize().into()
}

/// The desired-spec digest and tombstone class of one policy-input row: the
/// payload facts a committed-policy compile consumes.
fn policy_input_content(resource: &StoredResource) -> ([u8; 32], &'static [u8]) {
    match ResourceEnvelope::from_json(&resource.canonical_json) {
        Ok(envelope) => (
            Sha256::digest(envelope.spec().base().to_canonical_bytes()).into(),
            match envelope.status().phase() {
                ResourcePhase::Deleted => b"deleted",
                ResourcePhase::Failed => b"failed",
                _ => b"bindable",
            },
        ),
        // An undecodable row is not stable input for the compile either; hash
        // its bytes so any change to it still moves the digest.
        Err(_) => (Sha256::digest(&resource.canonical_json).into(), b"invalid"),
    }
}

/// The manager view over an already published plane table, when the
/// composition has published one (U14: the table is the only authority a
/// reader built before publication can resolve lazily).
fn published_plane_view(
    planes: &Arc<
        Mutex<
            Option<
                Arc<
                    parking_lot::Mutex<
                        std::collections::HashMap<
                            String,
                            Arc<crate::resource_plane_v3::ResourcePlaneV3>,
                        >,
                    >,
                >,
            >,
        >,
    >,
    zone: &ZoneId,
) -> Option<Arc<dyn ControllerPlaneView>> {
    let table = planes.lock().ok().and_then(|slot| slot.clone())?;
    Some(Arc::new(PublishedPlaneControllerView::new(
        table,
        zone.clone(),
    )))
}

/// The Zone status projection's runtime metadata for one committed policy
/// snapshot. U14: the snapshot is the manager-served policy projection; the
/// durable store metadata that used to feed this is gone.
fn zone_runtime_metadata(
    policy: &PolicySnapshot,
    total_resource_count: u32,
    generation_cleanup_pending: bool,
    cleanup_pending_count: u32,
    last_reconciled_at: Option<Timestamp>,
) -> ZoneRuntimeMetadata {
    ZoneRuntimeMetadata {
        api_catalog_revision: policy.api_catalog_revision,
        policy_revision: policy.policy_revision,
        configuration_revision: policy.active_configuration_revision.get(),
        installed_provider_count: 0,
        ready_provider_count: 0,
        total_resource_count,
        active_configuration_generation: policy.active_configuration_revision.get(),
        generation_cleanup_pending,
        cleanup_pending_count,
        last_reconciled_at,
    }
}

fn credential_row_ready(resource: &StoredResource) -> bool {
    serde_json::from_slice::<serde_json::Value>(&resource.canonical_json)
        .ok()
        .is_some_and(|value| {
            value
                .pointer("/status/phase")
                .and_then(serde_json::Value::as_str)
                == Some("Ready")
                && value
                    .pointer("/status/observedGeneration")
                    .and_then(serde_json::Value::as_u64)
                    == Some(resource.generation.get())
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InteractionState {
    Absent,
    Ready,
    Refused,
}

#[derive(Clone)]
pub(crate) struct CommittedInteractionProviderConfiguration {
    clipboard: Option<CommittedClipboardProviderConfiguration>,
    notification: Option<CommittedNotificationProviderConfiguration>,
}

#[derive(Clone)]
pub(crate) struct CommittedInteractionIdentity {
    zone: ZoneId,
    wayland_session_ref: ResourceRef,
    wayland_session_uid: ResourceUid,
    subject_ref: ResourceRef,
    subject_uid: ResourceUid,
    host_execution_ref: ResourceRef,
    user_ref: ResourceRef,
    allowed_guest_sources: BTreeMap<ResourceRef, ResourceUid>,
    display_provider_generation: ResourceGeneration,
    clipboard_provider_generation: Option<ResourceGeneration>,
    clipboard_provider_uid: Option<ResourceUid>,
    notification_provider_generation: Option<ResourceGeneration>,
    notification_provider_uid: Option<ResourceUid>,
}

impl CommittedInteractionIdentity {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn for_test(
        zone: ZoneId,
        subject_ref: ResourceRef,
        subject_uid: ResourceUid,
        host_execution_ref: ResourceRef,
        user_ref: ResourceRef,
        allowed_guest_sources: BTreeMap<ResourceRef, ResourceUid>,
        display_provider_generation: ResourceGeneration,
        clipboard_provider_generation: Option<ResourceGeneration>,
        clipboard_provider_uid: Option<ResourceUid>,
        notification_provider_generation: Option<ResourceGeneration>,
        notification_provider_uid: Option<ResourceUid>,
    ) -> Self {
        Self {
            zone,
            wayland_session_ref: ResourceRef::parse(
                "display-wayland.d2bus.org.WaylandSession/display-wayland",
            )
            .expect("fixed test WaylandSession reference"),
            wayland_session_uid: ResourceUid::parse("33333333-3333-4333-8333-333333333333")
                .expect("fixed test WaylandSession UID"),
            subject_ref,
            subject_uid,
            host_execution_ref,
            user_ref,
            allowed_guest_sources,
            display_provider_generation,
            clipboard_provider_generation,
            clipboard_provider_uid,
            notification_provider_generation,
            notification_provider_uid,
        }
    }

    pub(crate) fn seal_interaction_subject_install(
        &self,
        issuer: CommittedInteractionSubjectIssuer,
        expected_peer_uid: u32,
    ) -> d2b_session::Result<CommittedInteractionSubjectInstall> {
        issuer.seal(
            self.zone.clone(),
            self.subject_ref.clone(),
            self.subject_uid.clone(),
            expected_peer_uid,
            self.host_execution_ref.clone(),
            self.display_provider_generation,
            self.clipboard_provider_generation,
            self.notification_provider_generation,
            self.clipboard_provider_uid.clone(),
            self.notification_provider_uid.clone(),
        )
    }

    pub(crate) const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    pub(crate) fn wayland_session_ref(&self) -> &ResourceRef {
        &self.wayland_session_ref
    }

    pub(crate) fn wayland_session_uid(&self) -> &ResourceUid {
        &self.wayland_session_uid
    }

    pub(crate) fn subject_ref(&self) -> &ResourceRef {
        &self.subject_ref
    }

    pub(crate) fn subject_uid(&self) -> &ResourceUid {
        &self.subject_uid
    }

    pub(crate) fn host_execution_ref(&self) -> &ResourceRef {
        &self.host_execution_ref
    }

    pub(crate) fn user_ref(&self) -> &ResourceRef {
        &self.user_ref
    }

    pub(crate) fn allowed_guest_sources(&self) -> &BTreeMap<ResourceRef, ResourceUid> {
        &self.allowed_guest_sources
    }

    pub(crate) const fn display_provider_generation(&self) -> ResourceGeneration {
        self.display_provider_generation
    }

    pub(crate) fn clipboard_provider_uid(&self) -> Option<&ResourceUid> {
        self.clipboard_provider_uid.as_ref()
    }

    pub(crate) fn notification_provider_uid(&self) -> Option<&ResourceUid> {
        self.notification_provider_uid.as_ref()
    }
}

impl CommittedInteractionProviderConfiguration {
    pub(crate) fn clipboard(&self) -> Option<&CommittedClipboardProviderConfiguration> {
        self.clipboard.as_ref()
    }

    pub(crate) fn notification(&self) -> Option<&CommittedNotificationProviderConfiguration> {
        self.notification.as_ref()
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.clipboard
            .as_ref()
            .is_none_or(|config| config.is_integrity_bound())
            && self
                .notification
                .as_ref()
                .is_none_or(|config| config.is_integrity_bound())
    }
}

impl core::fmt::Debug for CommittedInteractionProviderConfiguration {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("CommittedInteractionProviderConfiguration(<redacted>)")
    }
}

#[derive(Clone)]
pub(crate) struct CommittedClipboardProviderConfiguration {
    policy: ClipboardPolicy,
    audit_capacity: usize,
    host_execution_ref: ResourceRef,
    host_user_ref: ResourceRef,
    display_wayland_ref: ResourceRef,
    guest_sources: BTreeSet<ResourceRef>,
    resource_uid: ResourceUid,
    resource_generation: ResourceGeneration,
    resource_revision: ZoneRevision,
    provenance_digest: String,
}

impl CommittedClipboardProviderConfiguration {
    pub(crate) fn policy(&self) -> ClipboardPolicy {
        self.policy.clone()
    }

    pub(crate) const fn audit_capacity(&self) -> usize {
        self.audit_capacity
    }

    pub(crate) fn resource_uid(&self) -> &ResourceUid {
        &self.resource_uid
    }

    pub(crate) fn guest_sources(&self) -> impl Iterator<Item = &ResourceRef> {
        self.guest_sources.iter()
    }


    pub(crate) fn matches_display(
        &self,
        display: &d2b_provider_clipboard_wayland::DisplayDependencyEvidence,
    ) -> bool {
        display.host_execution_ref() == &self.host_execution_ref
            && display.user_ref() == &self.host_user_ref
            && display.provider_ref() == &self.display_wayland_ref
    }

    fn is_integrity_bound(&self) -> bool {
        committed_resource_is_integrity_bound(
            &self.resource_uid,
            self.resource_generation,
            self.resource_revision,
            &self.provenance_digest,
        )
    }
}

impl core::fmt::Debug for CommittedClipboardProviderConfiguration {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CommittedClipboardProviderConfiguration")
            .field("guest_source_count", &self.guest_sources.len())
            .field("resource_generation", &self.resource_generation)
            .field("resource_revision", &self.resource_revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(crate) struct CommittedNotificationProviderConfiguration {
    config: NotificationProviderConfig,
    host_execution_ref: ResourceRef,
    resource_uid: ResourceUid,
    resource_generation: ResourceGeneration,
    resource_revision: ZoneRevision,
    provenance_digest: String,
}

impl CommittedNotificationProviderConfiguration {
    pub(crate) fn config(&self) -> NotificationProviderConfig {
        self.config.clone()
    }

    pub(crate) fn observer_user_ref(&self) -> &ResourceRef {
        self.config
            .host_user_ref()
            .expect("committed notification configuration always binds a host User")
    }

    pub(crate) fn resource_uid(&self) -> &ResourceUid {
        &self.resource_uid
    }

    pub(crate) fn guest_sources(&self) -> impl Iterator<Item = &ResourceRef> {
        self.config
            .guest_sources()
            .iter()
            .map(|source| source.source_ref())
    }

    fn is_integrity_bound(&self) -> bool {
        committed_resource_is_integrity_bound(
            &self.resource_uid,
            self.resource_generation,
            self.resource_revision,
            &self.provenance_digest,
        )
    }
}

fn committed_resource_is_integrity_bound(
    uid: &ResourceUid,
    generation: ResourceGeneration,
    revision: ZoneRevision,
    digest: &str,
) -> bool {
    !uid.as_str().is_empty()
        && generation.get() > 0
        && revision.get() > 0
        && digest.starts_with("sha256:")
}

impl core::fmt::Debug for CommittedNotificationProviderConfiguration {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CommittedNotificationProviderConfiguration")
            .field("resource_generation", &self.resource_generation)
            .field("resource_revision", &self.resource_revision)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClipboardProviderConfigWire {
    controller_execution_ref: ResourceRef,
    host_execution_ref: ResourceRef,
    host_user_ref: ResourceRef,
    display_wayland_ref: ResourceRef,
    guest_sources: Vec<ClipboardGuestSourceWire>,
    #[serde(default)]
    caps: ClipboardCapsWire,
    #[serde(default)]
    policy: ClipboardPolicyWire,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClipboardGuestSourceWire {
    guest_ref: ResourceRef,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClipboardCapsWire {
    #[serde(default = "default_clipboard_history_entries")]
    max_history_entries: usize,
    #[serde(default = "default_clipboard_item_bytes")]
    max_item_bytes: usize,
    #[serde(default = "default_clipboard_total_bytes")]
    max_total_bytes: usize,
    #[serde(default = "default_clipboard_concurrent_fds")]
    max_concurrent_fds: usize,
    #[serde(default = "default_clipboard_guest_rate")]
    max_guest_rate_per_min: u32,
    #[serde(default = "default_clipboard_fd_timeout")]
    fd_write_timeout_seconds: u64,
}

impl Default for ClipboardCapsWire {
    fn default() -> Self {
        Self {
            max_history_entries: default_clipboard_history_entries(),
            max_item_bytes: default_clipboard_item_bytes(),
            max_total_bytes: default_clipboard_total_bytes(),
            max_concurrent_fds: default_clipboard_concurrent_fds(),
            max_guest_rate_per_min: default_clipboard_guest_rate(),
            fd_write_timeout_seconds: default_clipboard_fd_timeout(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClipboardPolicyWire {
    #[serde(default = "default_true")]
    allow_host_capture: bool,
    #[serde(default = "default_true")]
    allow_guest_capture: bool,
    #[serde(default = "default_true")]
    require_picker_for_paste: bool,
    #[serde(default = "default_true")]
    suppress_echo: bool,
    #[serde(default)]
    cross_zone: ClipboardCrossZoneWire,
}

impl Default for ClipboardPolicyWire {
    fn default() -> Self {
        Self {
            allow_host_capture: true,
            allow_guest_capture: true,
            require_picker_for_paste: true,
            suppress_echo: true,
            cross_zone: ClipboardCrossZoneWire::default(),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClipboardCrossZoneWire {
    #[serde(default)]
    enable: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NotificationProviderConfigWire {
    controller_execution_ref: ResourceRef,
    host_execution_ref: ResourceRef,
    host_user_ref: ResourceRef,
    display_wayland_ref: ResourceRef,
    guest_sources: Vec<NotificationGuestSourceWire>,
    #[serde(default = "default_notification_pending")]
    max_pending_notifications: usize,
    #[serde(default = "default_notification_nonce_ttl")]
    action_nonce_ttl_secs: u64,
    #[serde(default = "default_notification_nonce_store")]
    action_nonce_store_size: usize,
    #[serde(default = "default_notification_ack_timeout")]
    acknowledge_timeout_secs: u64,
    #[serde(default = "default_true")]
    dbus_sink_enabled: bool,
    #[serde(default = "default_true")]
    observer_enabled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NotificationGuestSourceWire {
    guest_ref: ResourceRef,
    categories: Vec<Category>,
}

const fn default_true() -> bool {
    true
}

const fn default_clipboard_history_entries() -> usize {
    20
}

const fn default_clipboard_item_bytes() -> usize {
    8 * 1024 * 1024
}

const fn default_clipboard_total_bytes() -> usize {
    64 * 1024 * 1024
}

const fn default_clipboard_concurrent_fds() -> usize {
    32
}

const fn default_clipboard_guest_rate() -> u32 {
    60
}

const fn default_clipboard_fd_timeout() -> u64 {
    30
}

const fn default_notification_pending() -> usize {
    64
}

const fn default_notification_nonce_ttl() -> u64 {
    120
}

const fn default_notification_nonce_store() -> usize {
    256
}

const fn default_notification_ack_timeout() -> u64 {
    3_600
}

fn generation_publication_operation_id(set_generation: &ResourceBundleGenerationId) -> String {
    format!(
        "{ZONE_GENERATION_PUBLICATION_OPERATION_PREFIX}{}",
        set_generation.as_str()
    )
}

fn generation_publication_payload(
    set_generation: &ResourceBundleGenerationId,
    binding_digest: &str,
    generations: &BTreeMap<ZoneId, ResourceBundleGenerationId>,
) -> Result<Vec<u8>, ResourceRuntimeError> {
    let generation_set = generations
        .iter()
        .map(|(zone, generation)| (zone.as_str().to_owned(), generation.as_str().to_owned()))
        .collect::<BTreeMap<_, _>>();
    serde_json::to_vec(&json!({
        "claimDigest": set_generation.as_str(),
        "storeBindingDigest": binding_digest,
        "publication": "zone-resource-plane",
        "generationSet": generation_set,
        "state": "pending"
    }))
    .map_err(|_| ResourceRuntimeError::HandlerNotReady)
}

fn generation_publication_payload_matches(
    payload: &[u8],
    set_generation: &ResourceBundleGenerationId,
    binding_digest: &str,
    generations: &BTreeMap<ZoneId, ResourceBundleGenerationId>,
) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(payload) else {
        return false;
    };
    let expected_generation_set = generations
        .iter()
        .map(|(zone, generation)| (zone.as_str().to_owned(), generation.as_str().to_owned()))
        .collect::<BTreeMap<_, _>>();
    value.get("claimDigest").and_then(Value::as_str) == Some(set_generation.as_str())
        && value.get("storeBindingDigest").and_then(Value::as_str) == Some(binding_digest)
        && value.get("publication").and_then(Value::as_str) == Some("zone-resource-plane")
        && value.get("generationSet") == serde_json::to_value(expected_generation_set).ok().as_ref()
}

struct ControllerSession {
    context: crate::process_provider_runtime::ControllerBootstrapContext,
    binding: ControllerSessionBinding,
    ingress: BusIngress,
    driver: SessionDriverHandle,
    _backend_lease: Option<Arc<dyn crate::process_provider_runtime::GuestCredentialBackendLease>>,
    resource_client: Option<Arc<ResourceApiClient<ZoneApiBackend, UnavailableUpgradeDispatcher>>>,
    service_task: tokio::task::JoinHandle<Result<(), SessionServerError>>,
    assignments: BTreeMap<ResourceUid, ResourceClientLease>,
    assignment_stream_open: bool,
    assignments_revoked: bool,
    transport_closed: bool,
    ingress_revoked: bool,
}

impl ControllerSession {
    fn cancel_backend_lease(&mut self) {
        if let Some(lease) = self._backend_lease.take() {
            lease.cancel();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControllerAssignmentRefreshError {
    Retryable,
    Failed(ResourceRuntimeError),
}

#[derive(Debug, PartialEq, Eq)]
enum ControllerAssignmentRefreshAction<'a> {
    Retryable {
        context: &'a crate::process_provider_runtime::ControllerBootstrapContext,
    },
    Failed {
        context: &'a crate::process_provider_runtime::ControllerBootstrapContext,
        error: ResourceRuntimeError,
    },
}

#[derive(Clone)]
struct ControllerSessionCoordinator {
    zone: ZoneId,
    bundle_resource_types: Vec<ResourceTypeName>,
    /// The zone plane's manager view. Set by
    /// [`ZoneResourceRuntime::attach_v3_planes`] once the composition
    /// publishes the plane; a read with no published plane is an error, never
    /// an absent row (U14: there is no durable store to fall back to).
    plane_view: Arc<Mutex<Option<Arc<dyn ControllerPlaneView>>>>,
    /// The manager-backed Resource API service a live controller session
    /// serves: the runtime's slot, filled when the Zone's plane activates.
    api: Arc<
        Mutex<Option<Arc<ResourceService<ZoneApiBackend>>>>,
    >,
    authorizer: Arc<NativeAuthorizer>,
    authorization_state: Arc<Mutex<Option<AuthorizationState>>>,
    policy_projection: Arc<PolicyProjection>,
    registrar: Arc<Mutex<Option<ZoneRegistrar>>>,
    assignments: AssignmentRegistry,
    controller_sessions: Arc<Mutex<BTreeMap<ResourceRef, ControllerSession>>>,
    pending_controller_session_clears:
        Arc<Mutex<BTreeMap<ResourceRef, crate::process_provider_runtime::ControllerBootstrapContext>>>,
    credential_sessions: CredentialSessionRegistry,
    controller_session_lock: Arc<tokio::sync::Mutex<()>>,
}

type CloudHypervisorResourceClient = ResourceApiClient<ZoneApiBackend, UnavailableUpgradeDispatcher>;




#[derive(Clone)]
struct PolicyProjection {
    authorizer: Arc<NativeAuthorizer>,
    /// The manager plane's authorizer: same catalog and policy as the
    /// primary, its own mutation seal for the manager-backed API service
    /// (U8/U9 F1 wiring). `None` for runtimes that never serve the v3 plane.
    manager_authorizer: Option<Arc<NativeAuthorizer>>,
    /// The Zone bus, enrolled when the Zone activates over its published
    /// manager plane (U14: the bus is built with the system-core session,
    /// which needs the manager-backed Resource API service).
    bus: Arc<Mutex<Option<Arc<ZoneBus>>>>,
    authorization_state: Arc<Mutex<Option<AuthorizationState>>>,
    policy_refresh: Arc<Mutex<()>>,
    policy_loaded: Arc<Mutex<bool>>,
    installed_controller_subjects: Arc<Mutex<BTreeSet<BoundSubject>>>,
    /// The digest of the committed policy rows the installed projection was
    /// compiled from (U14). The derived revision is not a change signal on
    /// its own, so the refresh compares this digest to decide whether the
    /// installed projection may be kept as is.
    installed_policy_inputs: Arc<Mutex<Option<[u8; 32]>>>,
}

impl PolicyProjection {
    /// Serialize the installed native policy with its matching trusted
    /// authorization state. This mutex is the projection's linearization
    /// point; callers that need to pair the authorizer with state must use
    /// this snapshot instead of reading the shadow state independently.
    fn installed_state(&self) -> Result<AuthorizationState, ResourceRuntimeError> {
        let _install = self
            .policy_refresh
            .lock()
            .map_err(|_| ResourceRuntimeError::PolicyUnavailable)?;
        let loaded = *self
            .policy_loaded
            .lock()
            .map_err(|_| ResourceRuntimeError::PolicyUnavailable)?;
        if !loaded {
            return Err(ResourceRuntimeError::PolicyUnavailable);
        }
        self.authorization_state
            .lock()
            .map_err(|_| ResourceRuntimeError::PolicyUnavailable)?
            .clone()
            .ok_or(ResourceRuntimeError::PolicyUnavailable)
    }

    /// The digest of the policy rows the installed projection was compiled
    /// from; `None` while no projection is installed.
    fn installed_policy_inputs(&self) -> Option<[u8; 32]> {
        self.installed_policy_inputs
            .lock()
            .ok()
            .and_then(|inputs| *inputs)
    }

    /// The revision a compile of the given policy-input rows may be installed
    /// at (U14).
    ///
    /// The Zone bus fences every install at the installed revision, so the
    /// revision a compile is handed must never regress - and the retired
    /// durable store's commit counter never handed out one. The rows'
    /// highest generation is not monotone by itself (removing the row that
    /// carried it lowers it, and a tombstone keeps it), so a changed input
    /// set advances past the installed revision while an unchanged one holds
    /// it; either way a policy change still supersedes the previous
    /// projection, exactly as the store's commit counter did.
    fn next_policy_revision(&self, derived: u64, inputs_changed: bool) -> u64 {
        let installed = match self.installed_state() {
            Ok(state) => state.zone_policy_revision.get(),
            Err(_) => return derived,
        };
        if inputs_changed {
            derived.max(installed.saturating_add(1))
        } else {
            derived.max(installed)
        }
    }

    fn installed_controller_subjects(
        &self,
    ) -> Result<BTreeSet<BoundSubject>, ResourceRuntimeError> {
        let _install = self
            .policy_refresh
            .lock()
            .map_err(|_| ResourceRuntimeError::PolicyUnavailable)?;
        self.installed_controller_subjects
            .lock()
            .map_err(|_| ResourceRuntimeError::PolicyUnavailable)
            .map(|subjects| subjects.clone())
    }

    fn install(
        &self,
        policy: PolicySet,
        state: AuthorizationState,
        controller_subjects: BTreeSet<BoundSubject>,
        policy_inputs: [u8; 32],
    ) -> Result<(), ResourceRuntimeError> {
        let _install = self
            .policy_refresh
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
        // The policy refresh mutex linearizes the complete projection:
        // authorizer/ZoneBus replacement and the shadow state are published
        // under one guard. Validation failures happen before mutation and
        // therefore leave the last-known-good projection installed.
        let manager_policy = policy.clone();
        let bus = self
            .bus
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?
            .clone();
        let install_result = if let Some(bus) = &bus {
            bus.replace_policy(policy, state.clone())
                .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)
        } else {
            self.authorizer
                .replace_policy(policy, &state)
                .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)
        };
        if let Err(error) = install_result {
            // The bus fence refuses a regressed revision; name the error so a
            // refused install is diagnosable instead of a bare code.
            tracing::warn!(
                incoming_revision = state.zone_policy_revision.get(),
                installed_revision = ?self
                    .authorization_state
                    .lock()
                    .ok()
                    .and_then(|slot| slot.as_ref().map(|state| state.zone_policy_revision.get())),
                error = ?error,
                "policy projection install failed",
            );
            return Err(error);
        }
        // Mirror the installed policy into the manager plane's authorizer so
        // the manager-backed API service evaluates the same facts. The
        // primary projection is already installed; a mirror failure only
        // disables the v3 surface.
        if let Some(manager) = &self.manager_authorizer
            && let Err(error) = manager.replace_policy(manager_policy, &state)
        {
            tracing::warn!(
                error = ?error,
                "manager plane policy mirror failed; the manager-backed API surface stays unavailable",
            );
            manager.mark_policy_unavailable();
        }
        if let Ok(mut installed) = self.authorization_state.lock() {
            *installed = Some(state);
        } else {
            self.mark_unavailable();
            return Err(ResourceRuntimeError::AuthorizationUnavailable);
        }
        if let Ok(mut installed) = self.installed_policy_inputs.lock() {
            *installed = Some(policy_inputs);
        } else {
            self.mark_unavailable();
            return Err(ResourceRuntimeError::AuthorizationUnavailable);
        }
        if let Ok(mut installed) = self.installed_controller_subjects.lock() {
            *installed = controller_subjects;
        } else {
            self.mark_unavailable();
            return Err(ResourceRuntimeError::AuthorizationUnavailable);
        }
        if let Ok(mut loaded) = self.policy_loaded.lock() {
            *loaded = true;
        } else {
            self.mark_unavailable();
            return Err(ResourceRuntimeError::AuthorizationUnavailable);
        }
        Ok(())
    }

    fn mark_unavailable(&self) {
        let bus = self.bus.lock().ok().and_then(|bus| bus.clone());
        if let Some(bus) = &bus {
            bus.mark_policy_unavailable();
        } else {
            self.authorizer.mark_policy_unavailable();
        }
        if let Some(manager) = &self.manager_authorizer {
            manager.mark_policy_unavailable();
        }
        if let Ok(mut installed) = self.authorization_state.lock() {
            *installed = None;
        }
        if let Ok(mut inputs) = self.installed_policy_inputs.lock() {
            *inputs = None;
        }
        if let Ok(mut subjects) = self.installed_controller_subjects.lock() {
            subjects.clear();
        }
        if let Ok(mut loaded) = self.policy_loaded.lock() {
            *loaded = false;
        }
    }
}

/// Authenticated daemon-side Resource API adapter for the Cloud Hypervisor
/// controller. The adapter owns no store or broker capability in the
/// controller crate; those remain behind this d2bd composition seam.
struct CloudHypervisorResourceSession {
    client: Arc<CloudHypervisorResourceClient>,
    /// Capture point for the provider controller's Guest status write. The
    /// converted Guest's status is actor-local (R11), so there is no durable
    /// row to write; the effect call that drove this session takes it from
    /// here and publishes it as the row's status projection.
    status_sink: Option<crate::guest_driver::GuestStatusSink>,
    /// U17 child bridge: the session's converted children live in the
    /// manager, so their commits (and the reads of them) route through the
    /// published plane; `None` keeps every read on the durable store path.
    plane_children: Option<PlaneChildMutations>,
    providers: Arc<crate::process_provider_runtime::ProductionProcessProviders>,
    guest_sessions: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<
                crate::GuestComponentSessionKey,
                Arc<d2bd_runtime::guest_component_session::GuestComponentSessionClient>,
            >,
        >,
    >,
    closed_guest_sessions: Arc<tokio::sync::Mutex<BTreeSet<crate::GuestComponentSessionKey>>>,
    zone: ZoneId,
    zone_uid: ResourceUid,
    policy_revision: u64,
    provider_ref: ResourceRef,
    execution_ref: ResourceRef,
    descriptor: VerifiedGuestSetupDescriptor,
    controller_generation: ControllerGeneration,
    session_target: Option<crate::CommittedGuestSessionTarget>,
    session_evidence: Option<GuestSessionEvidence>,
    suppress_finalizer_clear: bool,
    finalizer_clear_requested: Arc<AtomicBool>,
}

pub(crate) struct CatalogDescriptorVerifier {
    pub(crate) expected_key: String,
}

impl GuestSetupDescriptorVerifier for CatalogDescriptorVerifier {
    fn verify(
        &self,
        key_fingerprint: &d2b_contracts_resource::v3::SchemaFingerprint,
        _descriptor_digest: &d2b_contracts_resource::v3::SchemaFingerprint,
        signature: &str,
    ) -> bool {
        key_fingerprint.as_str() == self.expected_key && signature == "catalog-signature"
    }
}

/// The exact non-secret evidence binding of one live Guest session.
///
/// `accepted_generation` is the session's live generation. It advances on
/// every reconnect by design - the Guest admits only a strictly newer one
/// (`session_generation_is_fresh`, `next_session_generation`) - so it is
/// carried as the binding's `session_generation`, the freshness marker the
/// lifecycle plans consume. The binding's `reconnect_generation`, the value
/// the Guest incarnation fence reads (`snapshot_from_stored` ->
/// `GuestGenerationSet::session`), is the *enrolled* identity generation the
/// live session was admitted under: a reconnect does not change the Guest's
/// incarnation, and the Guest itself re-verifies that enrolled generation as
/// the floor of every acceptance (`GuestIdentity::validate_route`). Feeding
/// the live generation into the fence made a legitimately reconnected Guest
/// report `Pending` with `runtimeReady=false` forever (host-integration
/// `runtime-cloud-hypervisor-guest-preflight`, 2026-09-11).
fn guest_session_evidence_binding(
    identity: &d2bd_runtime::guest_mode::GuestIdentity,
    accepted_generation: u64,
    descriptor_digest: &str,
    endpoint_generation: u64,
) -> Option<GuestSessionEvidenceBinding> {
    GuestSessionEvidenceBinding::new(
        identity.guest_uid().to_canonical_string(),
        descriptor_digest,
        identity.schema_fingerprint().as_str(),
        identity.provider_generation(),
        identity.controller_generation(),
        accepted_generation,
        identity.reconnect_generation().get(),
        endpoint_generation,
        1,
    )
    .ok()
}

fn guest_session_evidence(
    guest_ref: &ResourceRef,
    session: &d2bd_runtime::guest_component_session::GuestComponentSessionClient,
    descriptor: &VerifiedGuestSetupDescriptor,
    target: &crate::CommittedGuestSessionTarget,
    vmm_ready: bool,
) -> Option<GuestSessionEvidence> {
    let identity = session.identity();
    if identity.zone() != target.zone()
        || identity.guest_ref() != guest_ref
        || identity.guest_uid() != target.guest_uid()
        || identity.provider_generation() != target.provider_generation().get()
    {
        return None;
    }
    let boot_digest = identity
        .boot_identity()
        .digest()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let route = session.route_binding();
    if !route.liveness().is_live() {
        return None;
    }
    let Some(binding) = guest_session_evidence_binding(
        identity,
        session.generation(),
        descriptor.descriptor().descriptor_digest().as_str(),
        target.endpoint_generation().get(),
    ) else {
        tracing::debug!(
            guest = %guest_ref.to_canonical_string(),
            "guest session evidence binding construction failed",
        );
        return None;
    };
    GuestSessionEvidence::current_bound(
        guest_ref.clone(),
        format!("sha256:{boot_digest}"),
        ["resource-commit".to_owned(), "resource-watch".to_owned()],
        vmm_ready,
        true,
        true,
        binding,
    )
    .inspect_err(|_| {
        tracing::debug!(
            guest = %guest_ref.to_canonical_string(),
            "guest session evidence construction failed",
        );
    })
    .ok()
}

/// The Guest incarnation fence of one session snapshot
/// ([`GuestGenerationSet`]): every member is an incarnation-scoped
/// generation, and the session member is the enrolled identity generation the
/// evidence carries (`guest_session_evidence_binding`) - never the live
/// accepted one, which advances on every reconnect by design.
fn guest_incarnation_generations(
    provider_generation: u64,
    controller_generation: u64,
    guest_generation: u64,
    session_evidence: Option<&GuestSessionEvidence>,
) -> GuestGenerationSet {
    GuestGenerationSet {
        provider: provider_generation,
        descriptor: provider_generation,
        controller: controller_generation,
        child: guest_generation,
        session: session_evidence
            .and_then(GuestSessionEvidence::reconnect_generation)
            .unwrap_or(0),
    }
}

impl std::fmt::Debug for CloudHypervisorResourceSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CloudHypervisorResourceSession(<redacted>)")
    }
}

/// The plane one `CloudHypervisorResourceSession` read resolved a row from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoredRowOrigin {
    /// The manager-served converted plane.
    Manager,
    /// The pre-v3 durable store.
    Durable,
}

/// Whether the provider controller's custody gate may treat one Guest row as
/// carrying its controller finalizer.
///
/// U12: on the converted plane the durable controller finalizer is replaced
/// by the manager's deleting-row hold (F3) - the daemon's ensure/clear
/// requests are idempotent no-ops and the row's authored finalizers stay
/// empty. Reading the authored list alone therefore never admits the
/// controller, which leaves every child (VMM Process, endpoints, setup
/// Volume) uncommitted forever, so a manager-served row reports the plane's
/// own ownership guarantee instead. A durable row keeps the exact authored
/// signal the pre-v3 plane always used.
fn guest_controller_finalizer_present<'a>(
    origin: StoredRowOrigin,
    authored: impl Iterator<Item = &'a str>,
) -> bool {
    origin == StoredRowOrigin::Manager
        || authored.into_iter().any(|finalizer| {
            finalizer == d2b_provider_runtime_cloud_hypervisor::GUEST_CONTROLLER_FINALIZER
        })
}

/// Project one manager-boundary child failure onto the provider session's
/// closed error shape: fences stay conflicts, integrity stays invalid, and
/// everything the manager could not answer stays retryable.
fn child_mutation_failure(failure: ChildMutationFailure) -> CloudHypervisorResourceApiError {
    match failure {
        ChildMutationFailure::Conflict => CloudHypervisorResourceApiError::Conflict,
        ChildMutationFailure::NotFound => CloudHypervisorResourceApiError::NotFound,
        ChildMutationFailure::Invalid => CloudHypervisorResourceApiError::InvalidResponse,
        ChildMutationFailure::Unavailable => CloudHypervisorResourceApiError::Transport,
    }
}

/// Decode one wire resource envelope into the stored projection. Relocated
/// from the retired activation module: the cloud-hypervisor resource API
/// below is its remaining consumer.
fn stored_resource_from_wire(resource: &wire::ResourceEnvelopeBytes) -> Option<StoredResource> {
    let identity = resource.identity.as_ref()?;
    let uid = ResourceUid::parse(identity.uid.as_deref()?).ok()?;
    let generation = ResourceGeneration::new(identity.generation?).ok()?;
    let revision = ZoneRevision::new(identity.revision?);
    let zone = ZoneId::parse(&identity.zone).ok()?;
    let resource_ref_text = format!("{}/{}", identity.resource_type, identity.name);
    let resource_ref = ResourceRef::parse(&resource_ref_text).ok()?;
    Some(StoredResource {
        resource_ref,
        zone,
        uid,
        owner_uid: None,
        owner_generation: None,
        generation,
        revision,
        canonical_json: resource.canonical_json.clone(),
        payload_digest: resource.payload_digest.clone(),
    })
}

/// Fold the manager's rendered rows over the durable leg: where both planes
/// hold a reference the manager rendering wins (the session's own child
/// commits land only there). The manager answers its list completely - an
/// owner-scoped child relist sees exactly this owner's rows - so unlike the
/// durable leg this merge is not bound by the durable page cap: an owned
/// child the manager holds is never dropped as an overflow of the union.
fn merge_manager_rows(resources: &mut Vec<StoredResource>, manager_rows: Vec<StoredResource>) {
    for row in manager_rows {
        match resources
            .iter_mut()
            .find(|existing| existing.resource_ref == row.resource_ref)
        {
            Some(existing) => *existing = row,
            None => resources.push(row),
        }
    }
}

/// The child-relist owner fence (issue #507).
///
/// `RelistOwnedChildren` derives the durable owner filter from the request's
/// `guest_ref`, but the manager leg of [`CloudHypervisorResourceSession::list_stored`]
/// is scoped to this session's own owner: a session for Guest A sending
/// `guest_ref = Guest/B` would answer with A's manager rows for B's request
/// (and B's manager-only rows would be silently dropped). The request's owner
/// must therefore be the plane owner, exactly the fence the `UpdateSpec` arm
/// applies before its manager write. A session with no published plane has no
/// manager leg, so the durable leg's own `guest_ref`-derived filter stands.
fn fence_relist_owner(
    plane_owner: Option<&ResourceRef>,
    guest_ref: &ResourceRef,
) -> Result<(), CloudHypervisorResourceApiError> {
    match plane_owner {
        Some(owner) if owner != guest_ref => Err(CloudHypervisorResourceApiError::Conflict),
        _ => Ok(()),
    }
}

/// Whether one registered session table entry is a live session of the given
/// Guest identity.
fn is_live_guest_identity_session(
    key: &crate::GuestComponentSessionKey,
    session: &d2bd_runtime::guest_component_session::GuestComponentSessionClient,
    zone: &ZoneId,
    guest_ref: &ResourceRef,
    guest_uid: &ResourceUid,
) -> bool {
    key.is_guest_identity(zone, guest_ref, guest_uid)
        && session.identity().zone() == zone
        && session.identity().guest_ref() == guest_ref
        && session.identity().guest_uid() == guest_uid
}

impl CloudHypervisorResourceSession {
    fn api_error() -> CloudHypervisorResourceApiError {
        CloudHypervisorResourceApiError::Transport
    }

    fn session_key(
        &self,
        guest_ref: &ResourceRef,
        guest_uid: &ResourceUid,
    ) -> Result<crate::GuestComponentSessionKey, CloudHypervisorResourceApiError> {
        let Some(target) = self.session_target.as_ref() else {
            return Err(CloudHypervisorResourceApiError::Authentication);
        };
        if target.zone() != &self.zone
            || target.guest_ref() != guest_ref
            || target.guest_uid() != guest_uid
        {
            return Err(CloudHypervisorResourceApiError::Conflict);
        }
        Ok(target.key())
    }

    /// The manager-served row of a converted type (U17 child bridge), or
    /// `None` when the type is unconverted, the plane is unpublished, or the
    /// manager does not hold the row: each of those keeps the caller on the
    /// durable store path. A manager or render failure is never reported as
    /// absence (G5) - it is a retryable transport failure.
    async fn plane_row(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<StoredResource>, CloudHypervisorResourceApiError> {
        let Some(children) = &self.plane_children else {
            return Ok(None);
        };
        if child_mutation_route(target) != ChildMutationRoute::Manager {
            return Ok(None);
        }
        children
            .current(target)
            .await
            .map_err(child_mutation_failure)
    }

    /// The manager rows of the requested converted types owned by this
    /// session's owner (the child relist read), rendered through the same
    /// canonical projection the manager-backed API serves. The manager
    /// resolves the owner scope itself, so the answer is exactly this
    /// owner's children.
    async fn plane_owned_rows(
        &self,
        resource_types: &[&str],
    ) -> Result<Vec<StoredResource>, CloudHypervisorResourceApiError> {
        let Some(children) = &self.plane_children else {
            return Ok(Vec::new());
        };
        children
            .rows_of_types(resource_types)
            .await
            .map_err(child_mutation_failure)
    }

    /// Every manager row of the requested converted types in this Zone (the
    /// finalization read, which must also see other owners' children),
    /// rendered through the same canonical projection the manager-backed API
    /// serves.
    async fn plane_rows(
        &self,
        resource_types: &[&str],
    ) -> Result<Vec<StoredResource>, CloudHypervisorResourceApiError> {
        let Some(children) = &self.plane_children else {
            return Ok(Vec::new());
        };
        children
            .zone_rows_of_types(resource_types)
            .await
            .map_err(child_mutation_failure)
    }

    /// Whether the durable store still serves one resource type for this
    /// session (issue #507, reader path).
    ///
    /// From the moment the zone's manager plane is published, the legacy
    /// binding refuses every converted type: the manager's row or its honest
    /// absence is final, so a converted-type read must never fall back to the
    /// durable mirror - the refused read would surface as a retryable
    /// transport failure forever. Before publication (legacy boot and unit
    /// fixtures, `plane_children` is `None`) the durable plane still answers,
    /// exactly as the legacy binding's publication latch admits it.
    fn durable_plane_serves(&self, resource_type: &str) -> bool {
        self.plane_children.is_none()
            || child_type_route(resource_type) == ChildMutationRoute::Legacy
    }

    async fn get_stored(
        &self,
        target: &ResourceRef,
        operation: &str,
    ) -> Result<StoredResource, CloudHypervisorResourceApiError> {
        Ok(self.get_stored_with_origin(target, operation).await?.0)
    }

    /// [`Self::get_stored`] with the plane the row was read from. The
    /// origin matters where the two planes differ beyond the row shape: the
    /// converted plane has no durable controller finalizer (see
    /// [`Self::snapshot_from_stored`]).
    async fn get_stored_with_origin(
        &self,
        target: &ResourceRef,
        operation: &str,
    ) -> Result<(StoredResource, StoredRowOrigin), CloudHypervisorResourceApiError> {
        // U17 child bridge: a converted row is the manager's (the session's
        // own child commits land there), so the manager is consulted first;
        // an unconverted type, or any type before the plane is published,
        // falls through to the durable store.
        if let Some(resource) = self.plane_row(target).await? {
            return Ok((resource, StoredRowOrigin::Manager));
        }
        if !self.durable_plane_serves(target.resource_type().as_str()) {
            // The published manager does not hold the row, and the legacy
            // binding refuses a converted type: the honest answer is absence
            // (issue #507), never a durable mirror read that must fail.
            return Err(CloudHypervisorResourceApiError::NotFound);
        }
        let mut request = wire::GetRequest::new();
        request.meta = MessageField::some(public_request_meta(operation));
        request.target = MessageField::some(ch_identity(&self.zone, target, None, None, None));
        let mut projection = wire::Projection::new();
        projection.kind = EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
        request.projection = MessageField::some(projection);
        let response = self.client.get(request).await;
        if response.error.is_some() {
            return Err(CloudHypervisorResourceApiError::NotFound);
        }
        let resource = response
            .resource
            .as_ref()
            .and_then(stored_resource_from_wire)
            .ok_or_else(Self::api_error)?;
        if resource.zone != self.zone || resource.resource_ref != *target {
            return Err(CloudHypervisorResourceApiError::InvalidResponse);
        }
        Ok((resource, StoredRowOrigin::Durable))
    }

    async fn list_stored(
        &self,
        resource_types: &[&str],
        owner_uid: Option<&ResourceUid>,
        operation: &str,
    ) -> Result<Vec<StoredResource>, CloudHypervisorResourceApiError> {
        // U17 child bridge, issue #507: a converted type is served by the
        // published manager plane alone - the legacy binding refuses its
        // durable list outright - so only the types the durable plane still
        // serves are listed there; the manager rows are merged below.
        let durable_types = resource_types
            .iter()
            .copied()
            .filter(|resource_type| self.durable_plane_serves(resource_type))
            .collect::<Vec<_>>();
        let mut request = wire::ListRequest::new();
        request.meta = MessageField::some(public_request_meta(operation));
        request.resource_types = durable_types
            .iter()
            .map(|value| (*value).to_owned())
            .collect();
        request.page_size = 256;
        let mut projection = wire::Projection::new();
        projection.kind = EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
        request.projection = MessageField::some(projection);
        if let Some(owner_uid) = owner_uid {
            let mut owner_uid_filter = wire::ListFilter::new();
            owner_uid_filter.field = "owner.resourceUid".to_owned();
            owner_uid_filter.values = vec![owner_uid.as_str().to_owned()];
            request.filters.push(owner_uid_filter);
        }
        let mut resources = Vec::new();
        while !durable_types.is_empty() {
            let response = self.client.list(request.clone()).await;
            if response.error.is_some() || response.truncated {
                return Err(Self::api_error());
            }
            for resource in &response.resources {
                if resources.len() >= 256 {
                    return Err(CloudHypervisorResourceApiError::Truncated);
                }
                let resource = stored_resource_from_wire(resource).ok_or_else(Self::api_error)?;
                if resource.zone != self.zone {
                    return Err(CloudHypervisorResourceApiError::InvalidResponse);
                }
                resources.push(resource);
            }
            let Some(cursor) = response.next_cursor.as_ref() else {
                break;
            };
            request.cursor = MessageField::some(cursor.clone());
        }
        // U17 child bridge: converted rows the session itself committed exist
        // only in the manager; the manager-rendered row wins where both
        // planes hold the reference. An owner-scoped read asks the manager
        // for exactly this owner's children, so the answer never depends on
        // how many rows the Zone holds for other owners, and the manager's
        // complete list is folded in past the durable leg's page bound.
        let manager_rows = if owner_uid.is_some() {
            self.plane_owned_rows(resource_types).await?
        } else {
            self.plane_rows(resource_types).await?
        };
        merge_manager_rows(&mut resources, manager_rows);
        Ok(resources)
    }

    async fn guest_for_fenced_operation(
        &self,
        guest_ref: &ResourceRef,
        guest_uid: &ResourceUid,
        operation: &str,
    ) -> Result<StoredResource, CloudHypervisorResourceApiError> {
        let guest = self.get_stored(guest_ref, operation).await?;
        if guest.uid != *guest_uid {
            return Err(CloudHypervisorResourceApiError::Conflict);
        }
        Ok(guest)
    }

    /// Every live authenticated session the daemon holds for one Guest
    /// identity, found by identity rather than through the session key's full
    /// fence.
    ///
    /// The deletion finalization observes the session this way (the
    /// `ObserveFinalization` arm reads the session table by Guest identity)
    /// and plans `DrainGuestLocal`/`CloseSession` from that observation, so
    /// those steps resolve the same session. The committed guest-control
    /// Endpoint a target key is fenced on is the Guest's owned child: the
    /// manager's deletion cascade retires it on its own schedule, which can
    /// precede the Guest's finalization steps.
    async fn live_guest_identity_sessions(
        &self,
        guest_ref: &ResourceRef,
        guest_uid: &ResourceUid,
    ) -> Vec<(
        crate::GuestComponentSessionKey,
        Arc<d2bd_runtime::guest_component_session::GuestComponentSessionClient>,
    )> {
        self.guest_sessions
            .lock()
            .await
            .iter()
            .filter(|(key, session)| {
                is_live_guest_identity_session(key, session, &self.zone, guest_ref, guest_uid)
            })
            .map(|(key, session)| (key.clone(), Arc::clone(session)))
            .collect()
    }

    /// Close the authenticated Guest session.
    ///
    /// The deletion finalization plans this step from the identity-scoped
    /// live observation (`ObserveFinalization`), so the close uses the same
    /// scope: every live session registered for the Guest identity is closed
    /// and recorded as closed. The Guest row and its committed uid stay the
    /// fence (`guest_for_fenced_operation`); a Guest the daemon holds no live
    /// session for has nothing left to close, exactly as observing it reports
    /// `Closed`.
    async fn close_guest_session(
        &self,
        guest_ref: &ResourceRef,
        guest_uid: &ResourceUid,
    ) -> Result<(), CloudHypervisorResourceApiError> {
        let _ = self
            .guest_for_fenced_operation(guest_ref, guest_uid, "cloud-hypervisor-guest-session")
            .await?;
        let mut sessions = self.guest_sessions.lock().await;
        let removed: Vec<crate::GuestComponentSessionKey> = sessions
            .iter()
            .filter(|(key, session)| {
                is_live_guest_identity_session(key, session, &self.zone, guest_ref, guest_uid)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in &removed {
            sessions.remove(key);
        }
        drop(sessions);
        if !removed.is_empty() {
            let mut closed = self.closed_guest_sessions.lock().await;
            for key in removed {
                closed.insert(key);
            }
        }
        Ok(())
    }

    async fn list_guest_local_resources(
        &self,
        session: &d2bd_runtime::guest_component_session::GuestComponentSessionClient,
        operation: &str,
    ) -> Result<Vec<wire::ResourceEnvelopeBytes>, CloudHypervisorResourceApiError> {
        let client = session.resource_service_client();
        let mut request = wire::ListRequest::new();
        request.meta = MessageField::some(public_request_meta(operation));
        request.resource_types = d2b_provider_runtime_cloud_hypervisor::GUEST_SEED_RESOURCE_TYPES
            .iter()
            .map(|resource_type| (*resource_type).to_owned())
            .collect();
        request.page_size = 256;
        let mut projection = wire::Projection::new();
        projection.kind = EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
        request.projection = MessageField::some(projection);
        let mut resources = Vec::new();
        loop {
            let response = client
                .list(ttrpc::context::Context::default(), &request)
                .await
                .inspect_err(|error| {
                    tracing::warn!(
                        error = ?error,
                        operation,
                        generation = session.generation(),
                        "Guest Resource API list transport failed",
                    );
                })
                .map_err(|_| CloudHypervisorResourceApiError::Transport)?;
            if response.error.is_some() || response.truncated {
                tracing::warn!(
                    error = ?response.error,
                    truncated = response.truncated,
                    operation,
                    generation = session.generation(),
                    "Guest Resource API list response was refused",
                );
                return Err(if response.truncated {
                    CloudHypervisorResourceApiError::Truncated
                } else {
                    CloudHypervisorResourceApiError::Transport
                });
            }
            for resource in response.resources {
                if resources.len() >= 256 {
                    return Err(CloudHypervisorResourceApiError::Truncated);
                }
                resources.push(resource);
            }
            let Some(cursor) = response.next_cursor.as_ref() else {
                break;
            };
            request.cursor = MessageField::some(cursor.clone());
        }
        Ok(resources)
    }

    async fn drain_guest_local_resources(
        &self,
        guest_ref: &ResourceRef,
        guest_uid: &ResourceUid,
    ) -> Result<(), CloudHypervisorResourceApiError> {
        let _ = self
            .guest_for_fenced_operation(guest_ref, guest_uid, "cloud-hypervisor-guest-session")
            .await?;
        // The planning observation (`ObserveFinalization`) reads the session
        // table by Guest identity, so this step resolves the same session; see
        // `live_guest_identity_sessions`.
        let Some((_key, session)) = self
            .live_guest_identity_sessions(guest_ref, guest_uid)
            .await
            .into_iter()
            .next()
        else {
            // No live session remains for the Guest: nothing is left to
            // drain over it.
            return Ok(());
        };
        let resources = self
            .list_guest_local_resources(&session, "cloud-hypervisor-drain-list")
            .await?;
        let client = session.resource_service_client();
        for resource in resources {
            let identity = resource
                .identity
                .as_ref()
                .ok_or(CloudHypervisorResourceApiError::InvalidResponse)?;
            let uid = ResourceUid::parse(
                identity
                    .uid
                    .as_deref()
                    .ok_or(CloudHypervisorResourceApiError::InvalidResponse)?,
            )
            .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
            let revision = ZoneRevision::new(
                identity
                    .revision
                    .ok_or(CloudHypervisorResourceApiError::InvalidResponse)?,
            );
            let mut mutation = wire::Mutation::new();
            mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_DELETE);
            mutation.target = MessageField::some(identity.clone());
            mutation.precondition = MessageField::some(ch_exact_precondition(&uid, revision));
            let mut request = wire::DeleteRequest::new();
            request.meta = MessageField::some(public_request_meta(&format!(
                "cloud-hypervisor-drain-delete-{}",
                uid.as_str()
            )));
            request.mutation = MessageField::some(mutation);
            let response = client
                .delete(ttrpc::context::Context::default(), &request)
                .await
                .map_err(|_| CloudHypervisorResourceApiError::Transport)?;
            if response.error.is_some() {
                return Err(CloudHypervisorResourceApiError::Conflict);
            }
        }
        if self
            .list_guest_local_resources(&session, "cloud-hypervisor-drain-verify")
            .await?
            .is_empty()
        {
            Ok(())
        } else {
            Err(CloudHypervisorResourceApiError::Conflict)
        }
    }

    fn snapshot_from_stored(
        &self,
        guest: &StoredResource,
        origin: StoredRowOrigin,
    ) -> Result<GuestSnapshot, CloudHypervisorResourceApiError> {
        let envelope = ResourceEnvelope::from_json(&guest.canonical_json).map_err(|_| {
            tracing::warn!("Cloud Hypervisor Guest snapshot failed: envelope");
            CloudHypervisorResourceApiError::InvalidResponse
        })?;
        let system_artifact_id =
            serde_json::from_slice::<GuestSpec>(&envelope.spec().base().to_canonical_bytes())
                .map_err(|_| {
                    tracing::warn!("Cloud Hypervisor Guest snapshot failed: spec");
                    CloudHypervisorResourceApiError::InvalidResponse
                })?
                .system_artifact_id()
                .map(|value| value.as_str().to_owned());
        let deleting = serde_json::from_slice::<Value>(&guest.canonical_json)
            .ok()
            .and_then(|value| value.get("metadata").cloned())
            .and_then(|metadata| metadata.get("deletionRequestedAt").cloned())
            .is_some_and(|value| !value.is_null());
        if deleting {
            tracing::debug!(
                guest = %guest.resource_ref.to_canonical_string(),
                finalizers = ?envelope
                    .metadata()
                    .finalizers()
                    .iter()
                    .map(|finalizer| finalizer.as_str())
                    .collect::<Vec<_>>(),
                "Cloud Hypervisor deleting Guest finalizers observed",
            );
        }
        let snapshot = GuestSnapshot::new(
            self.zone.clone(),
            self.zone_uid.clone(),
            guest.resource_ref.clone(),
            guest.uid.clone(),
            guest.generation,
            guest.revision,
            self.execution_ref.clone(),
            self.provider_ref.clone(),
            system_artifact_id,
            guest_incarnation_generations(
                self.descriptor.descriptor().provider_generation().get(),
                self.controller_generation.get(),
                guest.generation.get(),
                self.session_evidence.as_ref(),
            ),
            deleting,
        )
        .map_err(|_| {
            tracing::warn!("Cloud Hypervisor Guest snapshot failed: construction");
            CloudHypervisorResourceApiError::InvalidResponse
        })?
        .with_controller_finalizer_present(guest_controller_finalizer_present(
            origin,
            envelope.metadata().finalizers().iter().map(|f| f.as_str()),
        ));
        Ok(match self.session_evidence.clone() {
            Some(evidence) => snapshot.with_session_evidence(evidence),
            None => snapshot,
        })
    }
}

#[async_trait]
impl AuthenticatedResourceSession for CloudHypervisorResourceSession {
    async fn call(
        &self,
        request: CloudHypervisorResourceRequest,
    ) -> Result<CloudHypervisorResourceResponse, CloudHypervisorResourceApiError> {
        let operation = match &request {
            CloudHypervisorResourceRequest::Register { .. } => "register",
            CloudHypervisorResourceRequest::GetGuest { .. } => "get-guest",
            CloudHypervisorResourceRequest::RelistOwnedChildren { .. } => "relist-children",
            CloudHypervisorResourceRequest::ObserveDependencies { .. } => "observe-dependencies",
            CloudHypervisorResourceRequest::CommitBatch { .. } => "commit-batch",
            CloudHypervisorResourceRequest::UpdateSpec { .. } => "update-spec",
            CloudHypervisorResourceRequest::UpdateStatus { .. } => "update-status",
            CloudHypervisorResourceRequest::ObserveProcessAdoption { .. } => "observe-adoption",
            CloudHypervisorResourceRequest::AssessUpdate { .. } => "assess-update",
            CloudHypervisorResourceRequest::ObserveFinalization { .. } => "observe-finalization",
            CloudHypervisorResourceRequest::DrainGuestLocal { .. } => "drain-guest-local",
            CloudHypervisorResourceRequest::CloseGuestSession { .. } => "close-guest-session",
            CloudHypervisorResourceRequest::DeleteChild { .. } => "delete-child",
            CloudHypervisorResourceRequest::InvalidateGuestSession { .. } => "invalidate-session",
            CloudHypervisorResourceRequest::EnsureGuestFinalizer { .. } => "ensure-finalizer",
            CloudHypervisorResourceRequest::ClearGuestFinalizer { .. } => "clear-finalizer",
        };
        tracing::debug!(operation, "Cloud Hypervisor Resource API call");
        match request {
            CloudHypervisorResourceRequest::Register { registration } => {
                if registration.provider_ref() != &self.provider_ref
                    || registration.provider_generation()
                        != self.descriptor.descriptor().provider_generation()
                    || registration.descriptor_digest()
                        != self.descriptor.descriptor().descriptor_digest()
                {
                    return Err(CloudHypervisorResourceApiError::Authentication);
                }
                Ok(CloudHypervisorResourceResponse::Registered)
            }
            CloudHypervisorResourceRequest::GetGuest { guest_ref } => {
                let (guest, origin) = self
                    .get_stored_with_origin(&guest_ref, "cloud-hypervisor-get-guest")
                    .await?;
                Ok(CloudHypervisorResourceResponse::Guest(
                    self.snapshot_from_stored(&guest, origin)?,
                ))
            }
            CloudHypervisorResourceRequest::RelistOwnedChildren {
                guest_ref,
                expected_refs,
            } => {
                fence_relist_owner(
                    self.plane_children
                        .as_ref()
                        .map(PlaneChildMutations::owner_ref),
                    &guest_ref,
                )?;
                let owner = self
                    .get_stored(&guest_ref, "cloud-hypervisor-owner-fence")
                    .await?;
                let children = self
                    .list_stored(
                        &["Process", "Endpoint", "Volume"],
                        Some(&owner.uid),
                        "cloud-hypervisor-list-children",
                    )
                    .await?;
                let mut result = Vec::new();
                for resource in children {
                    if !expected_refs.contains(&resource.resource_ref) {
                        continue;
                    }
                    let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
                        .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                    if envelope.metadata().owner_ref() != Some(&guest_ref) {
                        continue;
                    }
                    let desired_lifecycle =
                        if resource.resource_ref.resource_type().as_str() == "Process" {
                            serde_json::from_slice::<ProcessSpec>(
                                &envelope.spec().base().to_canonical_bytes(),
                            )
                            .ok()
                            .map(|spec| spec.desired_lifecycle())
                        } else {
                            None
                        };
                    let spec_digest = envelope
                        .spec()
                        .digest()
                        .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                    result.push(
                        OwnedChildSnapshot::new(
                            resource.resource_ref,
                            resource.zone,
                            guest_ref.clone(),
                            resource.uid,
                            resource.generation,
                            resource.revision,
                            spec_digest,
                            envelope.status().phase(),
                            desired_lifecycle,
                            envelope.status().phase() == ResourcePhase::Ready,
                        )
                        .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?
                        .with_owner_uid(owner.uid.clone()),
                    );
                }
                Ok(CloudHypervisorResourceResponse::OwnedChildren(result))
            }
            CloudHypervisorResourceRequest::ObserveDependencies { guest_ref, graph } => {
                let mut devices = Vec::new();
                for resource_ref in &graph.devices {
                    let phase = self
                        .get_stored(resource_ref, "cloud-hypervisor-device-dependency")
                        .await
                        .ok()
                        .and_then(|resource| {
                            ResourceEnvelope::from_json(&resource.canonical_json)
                                .ok()
                                .map(|envelope| envelope.status().phase())
                        })
                        .unwrap_or(ResourcePhase::Pending);
                    devices.push((resource_ref.clone(), phase));
                }
                let mut networks = Vec::new();
                for resource_ref in &graph.networks {
                    let phase = self
                        .get_stored(resource_ref, "cloud-hypervisor-network-dependency")
                        .await
                        .ok()
                        .and_then(|resource| {
                            ResourceEnvelope::from_json(&resource.canonical_json)
                                .ok()
                                .map(|envelope| envelope.status().phase())
                        })
                        .unwrap_or(ResourcePhase::Pending);
                    networks.push((resource_ref.clone(), phase));
                }
                let mut volumes = Vec::new();
                for resource_ref in &graph.volumes {
                    let phase = self
                        .get_stored(resource_ref, "cloud-hypervisor-volume-dependency")
                        .await
                        .ok()
                        .and_then(|resource| {
                            ResourceEnvelope::from_json(&resource.canonical_json)
                                .ok()
                                .map(|envelope| envelope.status().phase())
                        })
                        .unwrap_or(ResourcePhase::Pending);
                    volumes.push((resource_ref.clone(), phase));
                }
                let mut bindings = Vec::new();
                for resource_ref in &graph.bindings {
                    let current = self
                        .get_stored(resource_ref, "cloud-hypervisor-binding-dependency")
                        .await
                        .map(|binding| {
                            crate::binding_child_resource_runtime::binding_readiness_current(
                                &binding,
                            )
                        })
                        .unwrap_or(false);
                    bindings.push((resource_ref.clone(), current));
                }
                let setup_volume_ref = deterministic_child_ref(&guest_ref, ChildRole::SystemVolume)
                    .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                let setup_ready = self
                    .get_stored(&setup_volume_ref, "cloud-hypervisor-setup-dependency")
                    .await
                    .ok()
                    .and_then(|resource| {
                        ResourceEnvelope::from_json(&resource.canonical_json)
                            .ok()
                            .map(|envelope| envelope.status().phase() == ResourcePhase::Ready)
                    })
                    .unwrap_or(false);
                let dependencies = GuestDependencySnapshot::new(
                    devices,
                    networks,
                    volumes,
                    bindings,
                    setup_ready,
                )
                .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                Ok(CloudHypervisorResourceResponse::Dependencies(dependencies))
            }
            CloudHypervisorResourceRequest::CommitBatch { batch } => {
                let owner = self
                    .get_stored(batch.owner_ref(), "cloud-hypervisor-commit-owner")
                    .await?;
                if owner.uid != *batch.owner_uid() || owner.revision != batch.owner_revision() {
                    return Err(CloudHypervisorResourceApiError::Conflict);
                }
                let mut committed = Vec::with_capacity(batch.mutations().len());
                for mutation in batch.mutations() {
                    let canonical = batch
                        .canonical_payload(mutation.target())
                        .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                    // The authored owner reference is the child's ownership
                    // fence on both planes (the store mutation carried it in
                    // its `owner` field).
                    if plane_controller_bridge::child_envelope_owner(&canonical).as_ref()
                        != Some(batch.owner_ref())
                    {
                        return Err(CloudHypervisorResourceApiError::Conflict);
                    }
                    match child_mutation_route(mutation.target()) {
                        // U17: a converted child is the manager's, so the
                        // create-absent commit is an owner-scoped manager
                        // ensure - the row an actor (and the Process driver)
                        // is spawned for.
                        ChildMutationRoute::Manager => {
                            let children = self
                                .plane_children
                                .as_ref()
                                .ok_or(CloudHypervisorResourceApiError::Transport)?;
                            let stored = children
                                .ensure(mutation.target(), &canonical)
                                .await
                                .map_err(child_mutation_failure)?;
                            committed.push(
                                d2b_provider_runtime_cloud_hypervisor::CommittedChild::new(
                                    stored.resource_ref,
                                    batch.owner_ref().clone(),
                                    stored.zone,
                                    stored.uid,
                                    stored.revision,
                                )
                                .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?,
                            );
                        }
                        ChildMutationRoute::Legacy => {
                            // Every Cloud Hypervisor child role is a
                            // converted type, so this arm is unreachable for
                            // a Guest this plane serves; it refuses closed
                            // rather than writing a row no actor would
                            // launch (KTD4).
                            let _ = (&canonical, batch);
                            return Err(CloudHypervisorResourceApiError::Conflict);
                        }
                    }
                }
                if committed.len() != batch.mutations().len() {
                    return Ok(CloudHypervisorResourceResponse::Committed(
                        GuestChildCommitResponse::Uncertain,
                    ));
                }
                Ok(CloudHypervisorResourceResponse::Committed(
                    GuestChildCommitResponse::Committed(committed),
                ))
            }
            CloudHypervisorResourceRequest::UpdateSpec { update } => {
                let current = self
                    .get_stored(update.target(), "cloud-hypervisor-update-child")
                    .await?;
                let current_value: Value = serde_json::from_slice(&current.canonical_json)
                    .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                let merged_spec = merge_cloud_hypervisor_child_spec(
                    &current_value,
                    update.body(),
                    update.desired_lifecycle(),
                )?;
                let payload = replace_public_field(&current_value, "spec", merged_spec)
                    .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                let mut operation_payload = format!(
                    "{}:{}:",
                    update.expected_uid().as_str(),
                    update.expected_revision().get(),
                )
                .into_bytes();
                operation_payload.extend_from_slice(&payload);
                let payload_operation_digest =
                    d2b_contracts_resource::v3::resource_schema::canonical_digest(
                        d2b_contracts_resource::v3::resource_schema::RESOURCE_ENVELOPE_DOMAIN_TAG,
                        &operation_payload,
                    );
                let operation_id = format!(
                    "ch-update-child-{}",
                    payload_operation_digest.trim_start_matches("sha256:")
                );
                let envelope = ResourceEnvelope::from_json(&current.canonical_json)
                    .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                let owner_ref = envelope
                    .metadata()
                    .owner_ref()
                    .cloned()
                    .ok_or(CloudHypervisorResourceApiError::Conflict)?;
                if owner_ref.resource_type().as_str() != "Guest" {
                    return Err(CloudHypervisorResourceApiError::Conflict);
                }
                if child_mutation_route(update.target()) == ChildMutationRoute::Manager {
                    // U17: a converted child's spec update is an exact-fence
                    // manager ensure; the manager row's uid/revision is the
                    // fence the durable commit's precondition carried.
                    let children = self
                        .plane_children
                        .as_ref()
                        .ok_or(CloudHypervisorResourceApiError::Transport)?;
                    if children.owner_ref() != &owner_ref {
                        return Err(CloudHypervisorResourceApiError::Conflict);
                    }
                    let stored = children
                        .update(
                            update.target(),
                            update.expected_uid(),
                            update.expected_revision(),
                            &payload,
                        )
                        .await
                        .map_err(child_mutation_failure)?;
                    return Ok(CloudHypervisorResourceResponse::Updated(
                        d2b_provider_runtime_cloud_hypervisor::CommittedChild::new(
                            stored.resource_ref,
                            update.target().clone(),
                            stored.zone,
                            stored.uid,
                            stored.revision,
                        )
                        .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?,
                    ));
                }
                // Every Cloud Hypervisor child role is a converted type, so
                // an update that did not route to the manager names a row
                // this plane does not serve: refuse closed rather than write
                // a pre-v3 row no actor would launch (KTD4).
                let _ = (&owner_ref, &payload, &operation_id);
                Err(CloudHypervisorResourceApiError::Conflict)
            }
            CloudHypervisorResourceRequest::UpdateStatus { guest_ref, status } => {
                let current = self
                    .get_stored(&guest_ref, "cloud-hypervisor-update-status")
                    .await?;
                let current_value: Value = serde_json::from_slice(&current.canonical_json)
                    .map_err(|_| {
                        tracing::warn!("Cloud Hypervisor status update failed: current-resource");
                        CloudHypervisorResourceApiError::InvalidResponse
                    })?;
                let mut desired_status = serde_json::to_value(status.status()).map_err(|_| {
                    tracing::warn!("Cloud Hypervisor status update failed: status-serialization");
                    CloudHypervisorResourceApiError::InvalidResponse
                })?;
                let provider_phase = desired_status
                    .get("phase")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or(CloudHypervisorResourceApiError::InvalidResponse)?;
                let public_phase = Value::String(
                    if provider_phase == "Deleting" {
                        "Draining"
                    } else {
                        provider_phase.as_str()
                    }
                    .to_owned(),
                );
                let current_status = current_value
                    .get("status")
                    .and_then(Value::as_object)
                    .ok_or(CloudHypervisorResourceApiError::InvalidResponse)?;
                if provider_phase == "Deleting"
                    && let Some(current_provider_phase) = current_status
                        .get("resource")
                        .and_then(Value::as_object)
                        .and_then(|resource| resource.get("phase"))
                        .cloned()
                    && let Some(resource) = desired_status.as_object_mut()
                {
                    resource.insert("phase".to_owned(), current_provider_phase);
                }
                if current_status.get("resource") == Some(&desired_status)
                    && current_status.get("phase") == Some(&public_phase)
                    && current_status
                        .get("observedGeneration")
                        .and_then(Value::as_u64)
                        == Some(current.generation.get())
                {
                    return Ok(CloudHypervisorResourceResponse::StatusUpdated);
                }
                let mut payload_value = current_value;
                let base_status = payload_value
                    .get_mut("status")
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| {
                        tracing::warn!("Cloud Hypervisor status update failed: status-replacement");
                        CloudHypervisorResourceApiError::InvalidResponse
                    })?;
                base_status.insert("resource".to_owned(), desired_status.clone());
                base_status.insert("phase".to_owned(), public_phase);
                base_status.insert(
                    "observedGeneration".to_owned(),
                    Value::from(current.generation.get()),
                );
                let payload_bytes = serde_json::to_vec(&payload_value)
                    .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                let payload = CanonicalJsonValue::parse(&payload_bytes)
                    .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?
                    .to_canonical_bytes();
                let mut operation_payload =
                    format!("{}:{}:", current.uid.as_str(), current.revision.get()).into_bytes();
                operation_payload.extend_from_slice(&payload);
                let payload_operation_digest =
                    d2b_contracts_resource::v3::resource_schema::canonical_digest(
                        d2b_contracts_resource::v3::resource_schema::RESOURCE_ENVELOPE_DOMAIN_TAG,
                        &operation_payload,
                    );
                let operation_id = format!(
                    "ch-update-status-{}",
                    payload_operation_digest.trim_start_matches("sha256:")
                );
                // U12 status decision: `Guest` is a converted type, so its
                // row has no durable status to write - the row's actor owns
                // status (R11). The controller's layered status is captured
                // by the effect call that drove this session and published as
                // the row's `status.resource` projection; a session with no
                // capture point (an explicit lifecycle relist) acknowledges
                // the write without persisting it. Converted children never
                // receive a provider-written status either: the Process and
                // Endpoint drivers' `Ready` is the only publication, and
                // writing one here as well would be a dual-write.
                let _ = &current;
                let _ = &payload;
                let _ = &operation_id;
                if let Some(sink) = self.status_sink.as_ref() {
                    *sink.lock() = Some(desired_status);
                } else {
                    tracing::debug!(
                        zone = %self.zone.as_str(),
                        guest = %guest_ref.name().as_str(),
                        "Cloud Hypervisor status write observed without a capture point",
                    );
                }
                Ok(CloudHypervisorResourceResponse::StatusUpdated)
            }
            CloudHypervisorResourceRequest::ObserveProcessAdoption {
                guest_ref,
                guest_uid,
                process_ref,
                process_uid,
                process_revision,
            } => {
                let process = self
                    .get_stored(&process_ref, "cloud-hypervisor-process-adoption")
                    .await;
                let status = match process {
                    Ok(resource) => {
                        if resource.uid != process_uid || resource.revision != process_revision {
                            return Ok(CloudHypervisorResourceResponse::ProcessAdoption(
                                d2b_provider_runtime_cloud_hypervisor::ProcessAdoptionStatus::Quarantined,
                            ));
                        }
                        let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
                            .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                        if envelope.metadata().owner_ref() != Some(&guest_ref) {
                            return Ok(CloudHypervisorResourceResponse::ProcessAdoption(
                                d2b_provider_runtime_cloud_hypervisor::ProcessAdoptionStatus::Quarantined,
                            ));
                        }
                        let owner = self
                            .get_stored(&guest_ref, "cloud-hypervisor-process-owner-fence")
                            .await?;
                        if owner.uid != guest_uid {
                            return Ok(CloudHypervisorResourceResponse::ProcessAdoption(
                                d2b_provider_runtime_cloud_hypervisor::ProcessAdoptionStatus::Quarantined,
                            ));
                        }
                        let spec = serde_json::from_slice::<ProcessSpec>(
                            &envelope.spec().base().to_canonical_bytes(),
                        )
                        .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                        let provider_ref = envelope
                            .spec()
                            .provider_ref()
                            .cloned()
                            .ok_or(CloudHypervisorResourceApiError::InvalidResponse)?;
                        let owner_ref = envelope.metadata().owner_ref().cloned();
                        let descriptor_digest =
                            self.descriptor.descriptor().descriptor_digest().clone();
                        let context = crate::process_provider_runtime::ProcessResourceContext::new(
                            self.zone.clone(),
                            &resource.resource_ref,
                            &resource.uid,
                            resource.generation,
                            resource.revision,
                            &provider_ref,
                            self.controller_generation,
                            Some(guest_ref.clone()),
                        )
                        .with_lifecycle_identity(
                            Some(self.zone_uid.clone()),
                            Some(self.policy_revision),
                            None,
                        )
                        .with_owner_ref(owner_ref)
                        .with_guest_descriptor_digest(Some(&descriptor_digest));
                        let liveness = self
                            .providers
                            .probe_resource(context, &spec)
                            .await
                            .map_err(|error| {
                                tracing::warn!(
                                    error = %error,
                                    "Cloud Hypervisor VMM adoption probe failed",
                                );
                                CloudHypervisorResourceApiError::Transport
                            })?;
                        match liveness {
                            crate::process_provider_runtime::ProviderLiveness::Alive => {
                                d2b_provider_runtime_cloud_hypervisor::ProcessAdoptionStatus::Current
                            }
                            crate::process_provider_runtime::ProviderLiveness::Exited => {
                                d2b_provider_runtime_cloud_hypervisor::ProcessAdoptionStatus::Absent
                            }
                            crate::process_provider_runtime::ProviderLiveness::Unknown => {
                                d2b_provider_runtime_cloud_hypervisor::ProcessAdoptionStatus::Unavailable
                            }
                        }
                    }
                    Err(CloudHypervisorResourceApiError::NotFound) => {
                        d2b_provider_runtime_cloud_hypervisor::ProcessAdoptionStatus::Absent
                    }
                    Err(error) => {
                        tracing::debug!(
                            error = ?error,
                            guest = %guest_ref.to_canonical_string(),
                            "adoption probe failed; status unavailable",
                        );
                        d2b_provider_runtime_cloud_hypervisor::ProcessAdoptionStatus::Unavailable
                    }
                };
                Ok(CloudHypervisorResourceResponse::ProcessAdoption(status))
            }
            CloudHypervisorResourceRequest::AssessUpdate { .. } => {
                Ok(CloudHypervisorResourceResponse::UpdateAssessment(None))
            }
            CloudHypervisorResourceRequest::ObserveFinalization {
                guest_ref,
                guest_uid,
                children,
            } => {
                self.guest_for_fenced_operation(
                    &guest_ref,
                    &guest_uid,
                    "cloud-hypervisor-observe-finalization",
                )
                .await?;
                let all_children = self
                    .list_stored(
                        &["Process", "Endpoint", "Volume"],
                        None,
                        "cloud-hypervisor-list-finalization-descendants",
                    )
                    .await?;
                let current = children
                    .iter()
                    .map(|child| (child.resource_ref().clone(), child.uid().clone()))
                    .collect::<BTreeMap<_, _>>();
                let direct_refs = current.keys().cloned().collect::<BTreeSet<_>>();
                let mut transitive_descendants_present = false;
                let mut foreign_children_present = false;
                for resource in &all_children {
                    let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
                        .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                    match envelope.metadata().owner_ref() {
                        Some(owner) if owner == &guest_ref => {
                            if direct_refs.contains(&resource.resource_ref)
                                && current.get(&resource.resource_ref) != Some(&resource.uid)
                            {
                                foreign_children_present = true;
                            } else if !direct_refs.contains(&resource.resource_ref) {
                                tracing::debug!(
                                    guest = %guest_ref.to_canonical_string(),
                                    resource = %resource.resource_ref.to_canonical_string(),
                                    "non-CH resource still claims deleting Guest as owner",
                                );
                            }
                        }
                        Some(owner) if direct_refs.contains(owner) => {
                            transitive_descendants_present = true;
                        }
                        _ => {}
                    }
                }
                let process = children
                    .iter()
                    .find(|child| child.resource_ref().resource_type().as_str() == "Process");
                let process_state = match process {
                    Some(process)
                        if process.phase() == ResourcePhase::Ready
                            && process.desired_lifecycle() == Some(DesiredLifecycle::Running) =>
                    {
                        ProcessState::Running {
                            identity_verified: true,
                        }
                    }
                    Some(process)
                        if process.desired_lifecycle() == Some(DesiredLifecycle::Stopped) =>
                    {
                        ProcessState::Stopped
                    }
                    Some(_) => ProcessState::Unknown,
                    None => ProcessState::Absent,
                };
                let direct_children = children
                    .iter()
                    .filter_map(|child| {
                        let role = d2b_provider_runtime_cloud_hypervisor::child_role_for_ref(
                            child.resource_ref(),
                        )?;
                        let (deletion_requested, finalizers_pending, uid, revision) = all_children
                            .iter()
                            .find(|resource| resource.resource_ref == *child.resource_ref())
                            .map(|resource| {
                                let value =
                                    serde_json::from_slice::<Value>(&resource.canonical_json).ok();
                                let metadata =
                                    value.as_ref().and_then(|value| value.get("metadata"));
                                let deletion_requested = metadata
                                    .and_then(|metadata| metadata.get("deletionRequestedAt"))
                                    .is_some_and(|value| !value.is_null());
                                let finalizers_pending = metadata
                                    .and_then(|metadata| metadata.get("finalizers"))
                                    .and_then(Value::as_array)
                                    .is_some_and(|finalizers| !finalizers.is_empty());
                                (
                                    deletion_requested,
                                    finalizers_pending,
                                    resource.uid.clone(),
                                    resource.revision,
                                )
                            })
                            .unwrap_or_else(|| {
                                (false, false, child.uid().clone(), child.revision())
                            });
                        Some(
                            FencedChild::new(role, child.resource_ref().clone(), uid, revision)
                                .ok()?
                                .with_deletion_requested(deletion_requested)
                                .with_finalizers_pending(finalizers_pending),
                        )
                    })
                    .collect();
                let session = self
                    .guest_sessions
                    .lock()
                    .await
                    .iter()
                    .find(|(key, _)| key.is_guest_identity(&self.zone, &guest_ref, &guest_uid))
                    .map(|(_, session)| Arc::clone(session));
                let closed = self
                    .closed_guest_sessions
                    .lock()
                    .await
                    .iter()
                    .any(|key| key.is_guest_identity(&self.zone, &guest_ref, &guest_uid));
                let (session_state, guest_local_drained) = match session {
                    Some(session) => match tokio::time::timeout(
                        d2bd_runtime::guest_component_session::COMPONENT_SESSION_ATTEMPT_CAP,
                        self.list_guest_local_resources(
                            &session,
                            "cloud-hypervisor-finalization-local-list",
                        ),
                    )
                    .await
                    {
                        Ok(Ok(resources)) => (SessionState::Active, resources.is_empty()),
                        Ok(Err(error)) => {
                            tracing::warn!(
                                error = ?error,
                                "Guest finalization resource list failed",
                            );
                            self.guest_sessions.lock().await.retain(|key, _| {
                                !key.is_guest_identity(&self.zone, &guest_ref, &guest_uid)
                            });
                            (SessionState::Dead, false)
                        }
                        Err(_) => {
                            tracing::warn!("Guest finalization resource list timed out");
                            self.guest_sessions.lock().await.retain(|key, _| {
                                !key.is_guest_identity(&self.zone, &guest_ref, &guest_uid)
                            });
                            (SessionState::Dead, false)
                        }
                    },
                    None if closed => (SessionState::Closed, true),
                    None => (SessionState::Unknown, false),
                };
                let observation = GuestFinalizationInput::new(
                    guest_uid,
                    session_state,
                    guest_local_drained,
                    process_state,
                    direct_children,
                    transitive_descendants_present,
                    children
                        .iter()
                        .any(|child| child.resource_ref().resource_type().as_str() == "Volume"),
                    foreign_children_present,
                )
                .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                Ok(CloudHypervisorResourceResponse::Finalization(observation))
            }
            CloudHypervisorResourceRequest::DrainGuestLocal {
                guest_ref,
                guest_uid,
            } => {
                self.drain_guest_local_resources(&guest_ref, &guest_uid)
                    .await?;
                Ok(CloudHypervisorResourceResponse::LifecycleApplied)
            }
            CloudHypervisorResourceRequest::CloseGuestSession {
                guest_ref,
                guest_uid,
            } => {
                self.close_guest_session(&guest_ref, &guest_uid).await?;
                Ok(CloudHypervisorResourceResponse::LifecycleApplied)
            }
            CloudHypervisorResourceRequest::InvalidateGuestSession {
                guest_ref,
                guest_uid,
                minimum_generation,
            } => {
                self.guest_for_fenced_operation(
                    &guest_ref,
                    &guest_uid,
                    "cloud-hypervisor-invalidate-session",
                )
                .await?;
                let key = self.session_key(&guest_ref, &guest_uid)?;
                let mut sessions = self.guest_sessions.lock().await;
                let removed = if sessions.get(&key).is_some_and(|session| {
                    session.identity().guest_uid() == &guest_uid
                        && session.generation() < minimum_generation
                }) {
                    sessions.remove(&key);
                    true
                } else {
                    false
                };
                drop(sessions);
                if removed {
                    self.closed_guest_sessions.lock().await.insert(key);
                }
                Ok(CloudHypervisorResourceResponse::LifecycleApplied)
            }
            CloudHypervisorResourceRequest::DeleteChild {
                guest_ref,
                guest_uid,
                child,
            } => {
                let owner = self
                    .guest_for_fenced_operation(
                        &guest_ref,
                        &guest_uid,
                        "cloud-hypervisor-delete-child",
                    )
                    .await?;
                let current = self
                    .get_stored(child.target(), "cloud-hypervisor-delete-child-fence")
                    .await?;
                if current.uid != *child.uid() || current.revision != child.revision() {
                    return Err(CloudHypervisorResourceApiError::Conflict);
                }
                let envelope = ResourceEnvelope::from_json(&current.canonical_json)
                    .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                if envelope.metadata().owner_ref() != Some(&guest_ref) {
                    return Err(CloudHypervisorResourceApiError::Conflict);
                }
                let _operation_id = format!(
                    "cloud-hypervisor-delete-child-{}-{}",
                    child.uid().as_str(),
                    child.revision().get(),
                );
                if child_mutation_route(child.target()) == ChildMutationRoute::Manager {
                    // U17: the manager marks a converted child deleting and
                    // cascades; the child's own actor owns the cleanup.
                    self.plane_children
                        .as_ref()
                        .ok_or(CloudHypervisorResourceApiError::Transport)?
                        .remove(child.target())
                        .await
                        .map_err(child_mutation_failure)?;
                    return Ok(CloudHypervisorResourceResponse::LifecycleApplied);
                }
                // As above: a converted child's deletion routes to the
                // manager; anything else is refused closed.
                let _ = &owner;
                Err(CloudHypervisorResourceApiError::Conflict)
            }
            CloudHypervisorResourceRequest::ClearGuestFinalizer {
                guest_ref,
                guest_uid,
                guest_revision,
                finalizer_present,
            } => {
                if !finalizer_present {
                    return Ok(CloudHypervisorResourceResponse::LifecycleApplied);
                }
                if self.suppress_finalizer_clear {
                    self.finalizer_clear_requested.store(true, Ordering::Release);
                    return Ok(CloudHypervisorResourceResponse::LifecycleApplied);
                }
                let current = self
                    .guest_for_fenced_operation(
                        &guest_ref,
                        &guest_uid,
                        "cloud-hypervisor-clear-finalizer",
                    )
                    .await?;
                if current.revision != guest_revision {
                    return Err(CloudHypervisorResourceApiError::Conflict);
                }
                // U12: `Guest` is a converted type and the new plane replaced
                // the durable controller finalizer with the manager's
                // deleting-row hold (F3) plus the driver's own delete
                // sequencing, so there is nothing durable to clear. The
                // provider's finalization gate reads the row's authored
                // finalizers, which stay empty on this plane.
                Ok(CloudHypervisorResourceResponse::LifecycleApplied)
            }
            CloudHypervisorResourceRequest::EnsureGuestFinalizer {
                guest_ref,
                guest_uid,
                guest_revision,
            } => {
                let current = self
                    .guest_for_fenced_operation(
                        &guest_ref,
                        &guest_uid,
                        "cloud-hypervisor-ensure-finalizer",
                    )
                    .await?;
                if current.revision != guest_revision {
                    return Err(CloudHypervisorResourceApiError::Conflict);
                }
                // U12: as above - the converted plane holds the row through
                // the manager, not through a durable finalizer, so the
                // request is acknowledged idempotently.
                let _ = current;
                Ok(CloudHypervisorResourceResponse::LifecycleApplied)
            }
        }
    }
}

fn ch_identity(
    zone: &ZoneId,
    resource_ref: &ResourceRef,
    uid: Option<&ResourceUid>,
    generation: Option<u64>,
    revision: Option<u64>,
) -> wire::ResourceIdentity {
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = zone.as_str().to_owned();
    identity.resource_type = resource_ref.resource_type().as_str().to_owned();
    identity.name = resource_ref.name().as_str().to_owned();
    identity.uid = uid.map(|value| value.as_str().to_owned());
    identity.generation = generation;
    identity.revision = revision;
    identity
}

fn ch_exact_precondition(uid: &ResourceUid, revision: ZoneRevision) -> wire::Precondition {
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_uid = Some(uid.as_str().to_owned());
    precondition.expected_revision = Some(revision.get());
    precondition
}

fn ch_resource_body(
    zone: &ZoneId,
    resource_ref: &ResourceRef,
    uid: Option<&ResourceUid>,
    canonical: &[u8],
) -> Result<wire::ResourceEnvelopeBytes, CloudHypervisorResourceApiError> {
    let canonical = CanonicalJsonValue::parse(canonical)
        .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?
        .to_canonical_bytes();
    let mut body = wire::ResourceEnvelopeBytes::new();
    body.identity = MessageField::some(ch_identity(zone, resource_ref, uid, None, None));
    body.payload_digest = d2b_contracts_resource::v3::resource_schema::canonical_digest(
        d2b_contracts_resource::v3::resource_schema::RESOURCE_ENVELOPE_DOMAIN_TAG,
        &canonical,
    );
    body.canonical_json = canonical;
    Ok(body)
}

fn merge_cloud_hypervisor_child_spec(
    current: &Value,
    body: &d2b_provider_runtime_cloud_hypervisor::ChildCreateBody,
    desired_lifecycle: Option<DesiredLifecycle>,
) -> Result<Value, CloudHypervisorResourceApiError> {
    let body =
        serde_json::to_value(body).map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
    let desired_spec = body
        .get("spec")
        .and_then(Value::as_object)
        .ok_or(CloudHypervisorResourceApiError::InvalidResponse)?;
    let mut merged_spec = current
        .get("spec")
        .and_then(Value::as_object)
        .cloned()
        .ok_or(CloudHypervisorResourceApiError::InvalidResponse)?;
    merged_spec.extend(desired_spec.clone());
    if let Some(desired_lifecycle) = desired_lifecycle {
        merged_spec.insert(
            "desiredLifecycle".to_owned(),
            serde_json::to_value(desired_lifecycle)
                .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?,
        );
    }
    Ok(Value::Object(merged_spec))
}

/// A production Resource API and core-controller runtime for one Zone.
pub struct ZoneResourceRuntime {
    zone: ZoneId,
    authority_identity: Option<ZoneAuthorityIdentity>,
    authorizer: Arc<NativeAuthorizer>,
    authorization_state: Arc<Mutex<Option<AuthorizationState>>>,
    policy_projection: Arc<PolicyProjection>,
    bundle_resource_types: Vec<ResourceTypeName>,
    /// The published per-zone v3 planes (F1 wiring): the manager-backed API
    /// service resolves its manager client and watch hub from here. The
    /// inner lock is the composition's published plane table. The slot is
    /// shared with the reader closures built before publication.
    v3_planes: Arc<
        Mutex<
            Option<
                Arc<
                    parking_lot::Mutex<
                        std::collections::HashMap<
                            String,
                            Arc<crate::resource_plane_v3::ResourcePlaneV3>,
                        >,
                    >,
                >,
            >,
        >,
    >,
    /// The manager-backed Resource API service for this Zone: built once,
    /// after the Zone's v3 plane has been published. Shared with the
    /// controller-session coordinator, whose live sessions serve it.
    v3_api: Arc<
        Mutex<Option<Arc<ResourceService<d2b_resource_api::manager_backend::ManagerBackend>>>>,
    >,
    manager_authorizer: Arc<NativeAuthorizer>,
    policy_subject_fingerprints:
        Mutex<BTreeMap<(ResourceRef, ResourceRef), PolicySubjectFingerprint>>,
    bus: Option<Arc<ZoneBus>>,
    registrar: Arc<Mutex<Option<ZoneRegistrar>>>,
    ingress: Mutex<Option<BusIngress>>,
    service_task: Mutex<Option<tokio::task::JoinHandle<Result<(), SessionServerError>>>>,
    process_status_client:
        Arc<Mutex<Option<Arc<ResourceApiClient<ZoneApiBackend, UnavailableUpgradeDispatcher>>>>>,
    core_controller_subject: Mutex<Option<AuthenticatedSubjectContext>>,
    system_core_rebind_pending: AtomicBool,
    credential_sessions: CredentialSessionRegistry,
    core: Mutex<CoreProcess>,
    readiness: ZoneRuntimeReadiness,
    /// The fixed bootstrap policy snapshot (U14). The Zone opens on it and the
    /// manager-served committed policy replaces it at activation; it remains
    /// the reported snapshot while no manager-served projection is installed.
    bootstrap_policy_snapshot: PolicySnapshot,
    policy_installed: bool,
    controller_endpoint_registered: bool,
    watch_admitted: bool,
    assignments: AssignmentRegistry,
    authority_index: Arc<tokio::sync::Mutex<HostGlobalAuthorityIndex>>,
    /// The process-local Zone authority operation ledger (U14): generation
    /// publication and every other admission-barrier operation live here now
    /// that the durable store is gone. Restart recovery is the drivers'
    /// probe/adopt path, not a replayed checkpoint.
    authority_ledger: Arc<ZoneAuthorityLedger>,
    authority_recovery: Arc<AuthorityRecoveryCoordinator>,
    zone_status: Mutex<ZoneStatusResource>,
    audio_runtime: Arc<Mutex<Option<AudioResourceRuntime>>>,
    guest_setup_descriptors: BTreeMap<String, Vec<u8>>,
    guest_setup_descriptor_catalog_keys: BTreeMap<String, String>,
    closed_guest_sessions: Arc<tokio::sync::Mutex<BTreeSet<crate::GuestComponentSessionKey>>>,
    controller_deployment: ProviderDeployment,
    controller_session_providers:
        Mutex<Option<Arc<crate::process_provider_runtime::ProductionProcessProviders>>>,
    controller_sessions: Arc<Mutex<BTreeMap<ResourceRef, ControllerSession>>>,
    controller_session_reconcile_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    controller_session_reconcile_wake: Arc<tokio::sync::Notify>,
    controller_session_reconcile_shutdown: Arc<AtomicBool>,
    controller_session_coordinator:
        Arc<Mutex<Option<Arc<ControllerSessionCoordinator>>>>,
    controller_session_lock: Arc<tokio::sync::Mutex<()>>,
    controller_reconcile_lock: Arc<tokio::sync::Mutex<()>>,
    cloud_hypervisor_reconcile_lock: Arc<tokio::sync::Mutex<()>>,
    interaction_provider_configuration: Option<CommittedInteractionProviderConfiguration>,
    interaction_identity: Option<CommittedInteractionIdentity>,
    interaction_state: InteractionState,
}

/// Store-derived admission evidence for one security-key Device effect.
///
/// This contains only the exact values validated against the authoritative
/// resource record. It is consumed by the Device effect adapter before it can
/// request a broker-opened descriptor.
#[allow(dead_code)]
#[allow(dead_code)]
pub(crate) struct SecurityKeyDeviceAdmission {
    pub(crate) zone_ref: ResourceRef,
    pub(crate) device_uid: ResourceUid,
    pub(crate) holder_ref: ResourceRef,
    pub(crate) selector_id: String,
}

/// Request fields that select the Device admission record to validate.
#[allow(dead_code)]
#[allow(dead_code)]
pub(crate) struct SecurityKeyDeviceAdmissionRequest<'a> {
    pub(crate) device_uid: &'a ResourceUid,
    pub(crate) device_ref: &'a ResourceRef,
    pub(crate) request_zone_ref: &'a ResourceRef,
    pub(crate) holder_ref: &'a ResourceRef,
    pub(crate) vm_id: &'a str,
    pub(crate) selector_id: &'a str,
    pub(crate) operation_id: &'a str,
}

impl core::fmt::Debug for ZoneResourceRuntime {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ZoneResourceRuntime")
            .field("zone", &self.zone)
            .field("has_authority_identity", &self.authority_identity.is_some())
            .field("readiness", &self.readiness)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudHypervisorReconcileOutcome {
    Ready,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloudHypervisorEndpointOutcome {
    Ready,
    Pending,
}

/// Outcome of the Cloud Hypervisor setup-Volume stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloudHypervisorSetupVolumeOutcome {
    /// The setup Volume's manager row resolved and its store view was synced.
    Ready,
    /// The guest's provider controller has not committed the system Volume
    /// yet (the row belongs to the manager, so the pre-v3 store would answer
    /// absent forever): the stage stays pending and retries, it is never a
    /// read failure.
    Pending,
}

impl ZoneResourceRuntime {
    /// Open one Zone runtime bound to its verified bundle authority.
    ///
    /// U14: the Zone has exactly one plane and no durable store. The runtime
    /// opens with its bundle-bound authorizer and the bootstrap policy
    /// installed; everything the manager plane owns - the Resource API
    /// service, the system-core session, the committed policy projection and
    /// readiness - is built when the composition publishes the Zone's plane
    /// ([`Self::attach_v3_planes`], then
    /// [`Self::activate_published_bundle`]).
    pub(crate) async fn open_production_with_identity(
        zone: ZoneId,
        desired_bundle: ResourceBundle,
        authority_identity: ZoneAuthorityIdentity,
    ) -> Result<Self, ResourceRuntimeError> {
        if desired_bundle.zone != zone {
            return Err(ResourceRuntimeError::HandlerNotReady);
        }
        if desired_bundle.zone_uid() != Some(authority_identity.zone_uid())
            || desired_bundle.integrity().content_hash
                != authority_identity.bundle_generation().as_str()
        {
            return Err(ResourceRuntimeError::HandlerNotReady);
        }
        let bundle_resource_types = trusted_catalog_resource_types(
            desired_bundle
                .resources
                .iter()
                .map(|resource| resource.resource_type().clone())
                .collect::<Vec<_>>(),
        )?;
        let authorizer = Arc::new(runtime_authorizer(&bundle_resource_types)?);
        // The manager plane's own authorizer: same catalog, its own mutation
        // seal (an authorizer hands out exactly one), so the manager-backed
        // Resource API service serves this Zone beside the bus admission
        // path.
        let manager_authorizer = Arc::new(runtime_authorizer(&bundle_resource_types)?);
        let assignments = new_assignment_registry();
        let authority_ledger = Arc::new(ZoneAuthorityLedger::new(&zone));
        let authority_recovery = Arc::new(
            AuthorityRecoveryCoordinator::recover_with_provenance(
                Arc::clone(&authority_ledger) as Arc<dyn AuthorityPersistence>,
                authority_ledger.as_ref(),
            )
            .await
            .map_err(|_| ResourceRuntimeError::AuthorityUnavailable)?,
        );
        let authority_index = authority_recovery.index();
        // The bundle authority owns the Zone identity; the committed policy
        // is compiled from the manager rows when the Zone activates, and the
        // bootstrap snapshot keeps the fixed policy installed until then.
        let bootstrap_snapshot = initial_policy_snapshot()?;
        let (policy, state) = runtime_policy(
            &zone,
            &bootstrap_snapshot,
            ZoneRevision::new(u64::from(bootstrap_snapshot.policy_revision)),
            &bundle_resource_types,
        )
        .inspect_err(|error| {
            tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime policy setup failed");
        })?;
        authorizer
            .replace_policy(policy, &state)
            .map_err(|error| {
                tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime policy installation failed");
                ResourceRuntimeError::AuthorizationUnavailable
            })?;
        let authorization_state = Arc::new(Mutex::new(Some(state)));
        let policy_projection = Arc::new(PolicyProjection {
            authorizer: Arc::clone(&authorizer),
            manager_authorizer: Some(Arc::clone(&manager_authorizer)),
            bus: Arc::new(Mutex::new(None)),
            authorization_state: Arc::clone(&authorization_state),
            policy_refresh: Arc::new(Mutex::new(())),
            policy_loaded: Arc::new(Mutex::new(true)),
            installed_controller_subjects: Arc::new(Mutex::new(BTreeSet::new())),
            installed_policy_inputs: Arc::new(Mutex::new(None)),
        });
        let core = CoreProcess::new();
        let core_stage = core.stage();
        let zone_status = SystemCoreStatusEmitter::new()
            .emit(
                ZoneStatusInput::new(ResourcePhase::Pending, Vec::new()).with_runtime_metadata(
                    zone_runtime_metadata(
                        &bootstrap_snapshot,
                        0,
                        false,
                        0,
                        Some(current_status_timestamp()),
                    ),
                ),
            )
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        let runtime = Self {
            zone,
            authority_identity: Some(authority_identity),
            authorizer,
            authorization_state,
            policy_projection,
            bundle_resource_types,
            v3_planes: Arc::new(Mutex::new(None)),
            v3_api: Arc::new(Mutex::new(None)),
            manager_authorizer,
            policy_subject_fingerprints: Mutex::new(BTreeMap::new()),
            bus: None,
            registrar: Arc::new(Mutex::new(None)),
            ingress: Mutex::new(None),
            service_task: Mutex::new(None),
            process_status_client: Arc::new(Mutex::new(None)),
            core_controller_subject: Mutex::new(None),
            system_core_rebind_pending: AtomicBool::new(false),
            credential_sessions: CredentialSessionRegistry::default(),
            core: Mutex::new(core),
            bootstrap_policy_snapshot: bootstrap_snapshot,
            readiness: ZoneRuntimeReadiness {
                resource_api_ready: false,
                local_session_ready: false,
                provider_path_ready: false,
                authority_ready: true,
                core_stage,
            },
            policy_installed: true,
            controller_endpoint_registered: false,
            watch_admitted: false,
            assignments,
            authority_index,
            authority_ledger,
            authority_recovery,
            zone_status: Mutex::new(zone_status),
            audio_runtime: Arc::new(Mutex::new(None)),
            guest_setup_descriptors: BTreeMap::new(),
            guest_setup_descriptor_catalog_keys: BTreeMap::new(),
            closed_guest_sessions: Arc::new(tokio::sync::Mutex::new(BTreeSet::new())),
            controller_deployment: ProviderDeployment::new(
                DaemonMode::Host,
                d2bd_runtime::target_runtime::AdmissionLimits::host_default(),
            )
            .map_err(|_| ResourceRuntimeError::CoreStartupFailed)?,
            controller_session_providers: Mutex::new(None),
            controller_sessions: Arc::new(Mutex::new(BTreeMap::new())),
            controller_session_reconcile_task: Arc::new(Mutex::new(None)),
            controller_session_reconcile_wake: Arc::new(tokio::sync::Notify::new()),
            controller_session_reconcile_shutdown: Arc::new(AtomicBool::new(false)),
            controller_session_coordinator: Arc::new(Mutex::new(None)),
            controller_session_lock: Arc::new(tokio::sync::Mutex::new(())),
            controller_reconcile_lock: Arc::new(tokio::sync::Mutex::new(())),
            cloud_hypervisor_reconcile_lock: Arc::new(tokio::sync::Mutex::new(())),
            interaction_provider_configuration: None,
            interaction_identity: None,
            interaction_state: InteractionState::Absent,
        };
        let coordinator = Arc::new(runtime.build_controller_session_coordinator()?);
        *runtime
            .controller_session_coordinator
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? = Some(coordinator);
        tracing::info!(
            zone = %runtime.zone.as_str(),
            desired_resource_count = desired_bundle.resources.len(),
            "resource runtime opened; awaiting the manager plane publication",
        );
        Ok(runtime)
    }

    /// Durably stage the complete local generation set in the coordinator
    /// Zone's operation ledger. The operation is idempotent across daemon
    /// restarts and retired rows do not fence a later generation.
    pub(crate) async fn prepare_generation_publication(
        &self,
        set_generation: &ResourceBundleGenerationId,
        generations: &BTreeMap<ZoneId, ResourceBundleGenerationId>,
    ) -> Result<(), ResourceRuntimeError> {
        let zones = generations.keys().cloned().collect::<BTreeSet<_>>();
        let expected_set_generation = complete_generation_set_digest(&zones, generations)
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        if &expected_set_generation != set_generation {
            return Err(ResourceRuntimeError::HandlerNotReady);
        }
        let authority = self
            .authority_identity
            .as_ref()
            .ok_or(ResourceRuntimeError::IdentityUnbound)?;
        if generations.get(&self.zone) != Some(authority.bundle_generation()) {
            return Err(ResourceRuntimeError::HandlerNotReady);
        }
        let operation_id = generation_publication_operation_id(set_generation);
        let binding_digest = self
            .authority_ledger
            .authority_binding_digest(set_generation.as_str());
        let payload = generation_publication_payload(set_generation, &binding_digest, generations)?;
        let operations = self.authority_ledger.authority_operations();
        if operations.iter().any(|operation| {
            operation
                .operation_id
                .starts_with(ZONE_GENERATION_PUBLICATION_OPERATION_PREFIX)
                && operation.operation_id != operation_id
                && !matches!(
                    operation.state,
                    AuthorityOperationState::Closed | AuthorityOperationState::Released
                )
        }) {
            return Err(ResourceRuntimeError::HandlerNotReady);
        }
        if let Some(operation) = operations
            .iter()
            .find(|operation| operation.operation_id == operation_id)
        {
            if !generation_publication_payload_matches(
                &operation.payload,
                set_generation,
                &binding_digest,
                generations,
            ) {
                return Err(ResourceRuntimeError::HandlerNotReady);
            }
            return Ok(());
        }
        self.authority_ledger
            .prepare_authority_operation(operation_id, payload, set_generation.as_str())
            .map(|_| ())
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)
    }

    /// Mark a fully materialized generation set as published, then close and
    /// release its marker so a later generation can be admitted.
    pub(crate) async fn commit_generation_publication(
        &self,
        set_generation: &ResourceBundleGenerationId,
        generations: &BTreeMap<ZoneId, ResourceBundleGenerationId>,
    ) -> Result<(), ResourceRuntimeError> {
        let zones = generations.keys().cloned().collect::<BTreeSet<_>>();
        let expected_set_generation = complete_generation_set_digest(&zones, generations)
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        if &expected_set_generation != set_generation {
            return Err(ResourceRuntimeError::HandlerNotReady);
        }
        let authority = self
            .authority_identity
            .as_ref()
            .ok_or(ResourceRuntimeError::IdentityUnbound)?;
        if generations.get(&self.zone) != Some(authority.bundle_generation()) {
            return Err(ResourceRuntimeError::HandlerNotReady);
        }
        let operation_id = generation_publication_operation_id(set_generation);
        let binding_digest = self
            .authority_ledger
            .authority_binding_digest(set_generation.as_str());
        let operation = self
            .authority_ledger
            .authority_operations()
            .into_iter()
            .find(|operation| operation.operation_id == operation_id)
            .ok_or(ResourceRuntimeError::HandlerNotReady)?;
        if !generation_publication_payload_matches(
            &operation.payload,
            set_generation,
            &binding_digest,
            generations,
        ) {
            return Err(ResourceRuntimeError::HandlerNotReady);
        }
        let state = operation.state;
        if matches!(
            state,
            AuthorityOperationState::Closed | AuthorityOperationState::Released
        ) {
            return Ok(());
        }
        let capability = self
            .authority_ledger
            .resume_authority_operation(operation_id, &binding_digest)
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        match state {
            AuthorityOperationState::Pending | AuthorityOperationState::EffectRetryable => {
                capability
                    .record_effect(AuthorityOperationState::EffectConfirmed)
                    .await
                    .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
                capability
                    .record_close()
                    .await
                    .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
            }
            AuthorityOperationState::EffectConfirmed => {
                capability
                    .record_close()
                    .await
                    .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
            }
            AuthorityOperationState::Closing => {}
            AuthorityOperationState::EffectTerminal => {
                return Err(ResourceRuntimeError::HandlerNotReady);
            }
            AuthorityOperationState::Closed | AuthorityOperationState::Released => {
                unreachable!("terminal publication state returned above")
            }
        }
        capability
            .release()
            .await
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)
    }

    /// Finish deferred startup once the composition has published this
    /// Zone's manager plane: the Resource API service, the system-core
    /// session, the committed policy projection and the readiness checklist
    /// are all built here (U14: there is no durable store to activate).
    pub(crate) async fn activate_published_bundle(&mut self) -> Result<(), ResourceRuntimeError> {
        let api = self.manager_api_service()?;
        self.refresh_authorization_policy().await?;
        let state = self.policy_projection.installed_state()?;
        let bus_authorizer = BusAuthorizer::from_shared(Arc::clone(&self.authorizer), state.clone())
            .map(|authorizer| authorizer.with_assignment_registry(Arc::clone(&self.assignments)))
            .map_err(|error| {
                tracing::error!(zone = %self.zone.as_str(), error = ?error, "resource runtime bus authorizer setup failed");
                ResourceRuntimeError::AuthorizationUnavailable
            })?;
        let (zone_bus, mut zone_registrar) =
            ZoneBus::new(self.zone.clone(), bus_authorizer, BusConfig::default()).map_err(
                |error| {
                    tracing::error!(zone = %self.zone.as_str(), error = ?error, "resource runtime Zone bus setup failed");
                    ResourceRuntimeError::AuthenticationUnavailable
                },
            )?;
        let (zone_ingress, zone_service_task, status_client, subject_context) =
            register_system_core_session(
                &mut zone_registrar,
                api,
                Arc::clone(&self.authorizer),
                state.clone(),
            )
            .await
            .inspect_err(|error| {
                tracing::error!(zone = %self.zone.as_str(), error = ?error, "resource runtime system-core session registration failed");
            })?;
        let zone_bus = Arc::new(zone_bus);
        *self
            .process_status_client
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? =
            Some(Arc::clone(&status_client));
        *self
            .core_controller_subject
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? =
            Some(subject_context);
        *self
            .policy_projection
            .bus
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)? =
            Some(Arc::clone(&zone_bus));
        self.bus = Some(zone_bus);
        *self
            .registrar
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? = Some(zone_registrar);
        *self
            .ingress
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? = Some(zone_ingress);
        *self
            .service_task
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? =
            Some(zone_service_task);
        self.policy_installed = true;
        self.controller_endpoint_registered = true;
        self.watch_admitted = true;
        self.activate_committed_plane_state().await
    }

    /// Load the committed interaction configuration and the Core startup
    /// summary from the manager plane and publish readiness.
    async fn activate_committed_plane_state(&mut self) -> Result<(), ResourceRuntimeError> {
        let plane = self.manager_plane_view()?;
        let policy = self.committed_policy_snapshot();
        let mut interaction_provider_configuration_refused = false;
        self.interaction_provider_configuration = match load_interaction_provider_configuration(
            plane.as_ref(),
            &self.zone,
        )
        .await
        {
            Ok(None) => None,
            Ok(Some(configuration)) if configuration.is_complete() => Some(configuration),
            Ok(Some(_)) => {
                tracing::error!(
                    zone = %self.zone.as_str(),
                    "resource runtime committed interaction Provider configuration is incomplete",
                );
                interaction_provider_configuration_refused = true;
                None
            }
            Err(error) => {
                tracing::error!(
                    zone = %self.zone.as_str(),
                    error = %error,
                    "resource runtime committed interaction Provider configuration load failed",
                );
                interaction_provider_configuration_refused = true;
                None
            }
        };
        self.interaction_identity = match load_committed_interaction_identity(
            plane.as_ref(),
            &self.zone,
            self.interaction_provider_configuration.as_ref(),
        )
        .await
        {
            Ok(identity) => identity,
            Err(error) => {
                tracing::error!(
                    zone = %self.zone.as_str(),
                    error = %error,
                    "resource runtime committed interaction identity load failed",
                );
                interaction_provider_configuration_refused = true;
                None
            }
        };
        let interaction_present = interaction_resources_present(plane.as_ref()).await?;
        self.interaction_state = derive_interaction_state(
            interaction_present,
            self.interaction_provider_configuration.as_ref(),
            self.interaction_identity.as_ref(),
            interaction_provider_configuration_refused,
        );
        let system_core = system_core_startup_result(plane.as_ref(), &policy)
            .await
            .inspect_err(|error| {
                tracing::error!(
                    zone = %self.zone.as_str(),
                    error = ?error,
                    "resource runtime system-core startup summary failed",
                );
            })?;
        let aggregate_handler_phase = if system_core.host_phase == HandlerPhase::Ready
            && system_core.user_phase == HandlerPhase::Ready
        {
            HandlerPhase::Ready
        } else {
            HandlerPhase::Degraded
        };
        {
            let recovered_authority = self.authority_index.lock().await;
            let mut core = self
                .core
                .lock()
                .map_err(|_| ResourceRuntimeError::CoreStartupFailed)?;
            core.start_production(
                CoreRuntimeReadiness {
                    store_ready: true,
                    resource_api_ready: true,
                    local_bus_ready: true,
                    controller_endpoint_registered: true,
                    authenticated_system_core_session: true,
                },
                RecoverySnapshot {
                    startup_epoch: 0,
                    checkpoint_revision: u64::from(policy.policy_revision),
                    active_configuration_revision: policy
                        .active_configuration_revision
                        .get(),
                    provider_lease_count: 0,
                    controller_lease_count: 0,
                    ambiguous_operation_count: 0,
                    watch_admitted: true,
                },
                &recovered_authority,
            )
            .map_err(map_startup_error)?;
            d2bd_runtime::resource_runtime_support::mark_core_handlers(
                &mut core,
                aggregate_handler_phase,
                u64::from(policy.policy_revision),
            )?;
        };
        self.zone_status = Mutex::new(
            SystemCoreStatusEmitter::new()
                .emit(
                    ZoneStatusInput::new(system_core.core_phase, Vec::new())
                        .with_system_core_phases(
                            handler_phase_to_zone_phase(system_core.host_phase),
                            handler_phase_to_zone_phase(system_core.user_phase),
                        )
                        .with_runtime_metadata(zone_runtime_metadata(
                            &policy,
                            system_core.total_resource_count,
                            system_core.generation_cleanup_pending,
                            system_core.cleanup_pending_count,
                            Some(current_status_timestamp()),
                        )),
                )
                .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
        );
        let stage = {
            let mut core = self
                .core
                .lock()
                .map_err(|_| ResourceRuntimeError::CoreStartupFailed)?;
            core.publish_readiness().map_err(map_startup_error)?
        };
        self.readiness = ZoneRuntimeReadiness {
            resource_api_ready: true,
            local_session_ready: true,
            provider_path_ready: self.readiness.provider_path_ready,
            authority_ready: true,
            core_stage: stage,
        };
        Ok(())
    }

    /// Borrow the authoritative Zone identity.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the immutable Zone UID bound at production startup.
    pub(crate) fn authority_zone_uid(&self) -> Option<&ResourceUid> {
        self.authority_identity
            .as_ref()
            .map(ZoneAuthorityIdentity::zone_uid)
    }

    /// The Zone UID bound at production startup, resolved eagerly.
    ///
    /// U14: the durable store metadata that also carried the bound Zone UID is
    /// gone, and the bundle authority is the one Zone identity binding.
    fn bound_zone_uid(&self) -> Result<ResourceUid, ResourceRuntimeError> {
        self.authority_zone_uid()
            .cloned()
            .ok_or(ResourceRuntimeError::IdentityUnbound)
    }

    /// Borrow the content-addressed bundle generation bound at production
    /// startup.
    pub(crate) fn authority_bundle_generation(&self) -> Option<&ResourceBundleGenerationId> {
        self.authority_identity
            .as_ref()
            .map(ZoneAuthorityIdentity::bundle_generation)
    }

    /// Return the startup readiness projection.
    pub const fn readiness(&self) -> ZoneRuntimeReadiness {
        self.readiness
    }

    /// Borrow the Zone-scoped Core assignment registry.
    pub fn assignment_registry(&self) -> AssignmentRegistry {
        Arc::clone(&self.assignments)
    }

    /// Admit one controller assignment through the Zone-owned registry.
    ///
    /// Controller deployment supplies only the committed resource, signed
    /// role, installed generations, and authenticated session generation.
    /// The registry remains the single owner of target conflicts; callers
    /// conflicts; callers never receive a store handle. The session must
    /// already be present in the active controller-session table.
    pub fn admit_controller_assignment(
        &self,
        request: AssignmentRequest<'_>,
    ) -> Result<ResourceClientLease, AssignmentError> {
        let binding = request.session_binding()?;
        let active = self
            .controller_sessions
            .lock()
            .map(|sessions| {
                sessions
                    .get(binding.session_owner())
                    .is_some_and(|session| {
                        controller_session_matches(
                            &session.binding,
                            &binding,
                            session.service_task.is_finished(),
                        )
                    })
            })
            .unwrap_or(false);
        if !active {
            return Err(AssignmentError::SessionRevoked);
        }
        self.assignments
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .admit(request)
    }

    /// Revoke assignments bound to one exact controller session.
    pub fn revoke_controller_assignments(&self, binding: &ControllerSessionBinding) {
        if !d2b_provider_runtime_cloud_hypervisor::is_provider_ref(binding.provider_ref()) {
            return;
        }
        self.assignments
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .revoke_session_for(binding);
        let revocation_batch = match self.controller_sessions.lock() {
            Ok(sessions) => sessions
                .get(binding.session_owner())
                .filter(|session| &session.binding == binding)
                .map(|session| {
                    let frames = session
                        .assignments
                        .values()
                        .filter_map(|lease| {
                            ControllerAssignmentGrant::encode_revocation(
                                lease.provider_ref(),
                                lease.identity(),
                            )
                            .ok()
                        })
                        .collect();
                    (session.driver.clone(), frames)
                }),
            Err(_) => {
                tracing::warn!(
                    provider = %binding.provider_ref().to_canonical_string(),
                    "controller session registry lock poisoned; assignment revocation batch skipped",
                );
                None
            }
        };
        if let Some((driver, frames)) = revocation_batch {
            self.schedule_assignment_revocations(driver, frames);
        }
    }

    /// Mark one assignment as draining before a target or generation handoff.
    pub fn drain_controller_assignment(
        &self,
        identity: &AssignmentIdentity,
    ) -> Result<(), AssignmentError> {
        let result = self
            .assignments
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin_drain(identity);
        if result.is_ok() {
            self.schedule_controller_assignment_revocation(identity);
        }
        result
    }

    /// Release a drained assignment after Core has verified its child index.
    pub fn release_controller_assignment(
        &self,
        identity: &AssignmentIdentity,
    ) -> Result<(), AssignmentError> {
        let revocation = self.controller_assignment_revocation(identity);
        let result = self
            .assignments
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .release(identity);
        if result.is_ok()
            && let Some((driver, bytes)) = revocation
        {
            self.schedule_assignment_revocations(driver, vec![bytes]);
        }
        result
    }

    fn controller_assignment_revocation(
        &self,
        identity: &AssignmentIdentity,
    ) -> Option<(SessionDriverHandle, Vec<u8>)> {
        let sessions = self.controller_sessions.lock().ok()?;
        sessions.values().find_map(|session| {
            let lease = session
                .assignments
                .values()
                .find(|lease| lease.identity() == identity)?;
            let bytes =
                ControllerAssignmentGrant::encode_revocation(lease.provider_ref(), identity)
                    .ok()?;
            Some((session.driver.clone(), bytes))
        })
    }

    fn schedule_controller_assignment_revocation(&self, identity: &AssignmentIdentity) {
        let Some((driver, bytes)) = self.controller_assignment_revocation(identity) else {
            return;
        };
        self.schedule_assignment_revocations(driver, vec![bytes]);
    }

    fn schedule_assignment_revocations(&self, driver: SessionDriverHandle, frames: Vec<Vec<u8>>) {
        if frames.is_empty() {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let stream = match StreamId::new(CONTROLLER_ASSIGNMENT_STREAM_ID) {
            Ok(stream) => stream,
            Err(_) => return,
        };
        handle.spawn(async move {
            for frame in frames {
                if let Err(error) = driver.send_named_stream(stream, frame).await {
                    tracing::warn!(
                        error = %error,
                        "controller assignment revocation delivery failed",
                    );
                    let _ = driver.reset_named_stream(stream).await;
                    break;
                }
            }
        });
    }

    /// The policy snapshot of the installed authorization projection; the
    /// fixed bootstrap snapshot until the manager-served policy rows are
    /// compiled in (U14: there is no durable store metadata to read).
    ///
    /// Interaction Providers bind this snapshot instead of carrying a
    /// route-derived policy placeholder.
    pub fn committed_policy_snapshot(&self) -> PolicySnapshot {
        self.policy_projection
            .installed_state()
            .map(|state| state.snapshot)
            .unwrap_or(self.bootstrap_policy_snapshot)
    }

    /// Return the committed policy revision interaction evidence is fenced
    /// against: the installed projection's, or the bootstrap snapshot's while
    /// no manager-served projection is installed.
    pub fn current_revision(&self) -> ZoneRevision {
        self.authorization_state
            .lock()
            .ok()
            .and_then(|state| state.as_ref().map(|state| state.zone_policy_revision))
            .unwrap_or(ZoneRevision::new(
                self.bootstrap_policy_snapshot.policy_revision,
            ))
    }

    /// Refresh the native authorization projection from the current committed
    /// Role, RoleBinding, and local subject rows before admitting mutations or
    /// session transitions. Read-only public requests use the installed
    /// projection directly and never call this method.
    pub(crate) async fn refresh_authorization_policy(&self) -> Result<(), ResourceRuntimeError> {
        // All policy projections and controller-session transitions use this
        // lock order: controller session, then policy install.
        let _session_guard = self.controller_session_lock.lock().await;
        // A policy-input change landing between the refresh's reads
        // supersedes the rows it compiled; retry it here (bounded) instead
        // of failing the caller's mutation on the internal race.
        let mut attempt = 0;
        loop {
            match self.refresh_authorization_policy_locked().await {
                Err(ResourceRuntimeError::PolicyUnavailable)
                    if attempt + 1 < POLICY_REFRESH_ATTEMPTS =>
                {
                    attempt += 1;
                    tokio::time::sleep(POLICY_REFRESH_RETRY_BACKOFF).await;
                }
                result => return result,
            }
        }
    }

    async fn refresh_authorization_policy_locked(&self) -> Result<(), ResourceRuntimeError> {
        // U14: the manager-served policy rows are the whole authority, so the
        // snapshot the compile is fenced at is derived from those rows (there
        // is no durable store metadata left to read). The derived revision is
        // not a change signal on its own (see `policy_inputs_digest`), so the
        // refresh compares the row digest and keeps the installed projection
        // when the inputs did not move.
        let resources = self.committed_policy_resources().await?;
        let policy_inputs = policy_inputs_digest(&resources);
        let inputs_changed =
            self.policy_projection.installed_policy_inputs() != Some(policy_inputs);
        let policy_loaded = self.policy_projection.installed_state().is_ok();
        let controller_subjects = match self.current_controller_policy_subjects().await {
            Ok(subjects) => subjects,
            Err(error) => {
                tracing::warn!(
                    zone = self.zone.as_str(),
                    error = ?error,
                    "authorization policy refresh: controller policy subjects unavailable",
                );
                return Err(error);
            }
        };
        let installed_controller_subjects = self
            .policy_projection
            .installed_controller_subjects()?;
        if policy_loaded
            && !inputs_changed
            && !self.system_core_rebind_pending.load(Ordering::Acquire)
            && installed_controller_subjects == controller_subjects
        {
            return Ok(());
        }
        let mut snapshot = policy_snapshot_for_rows(&resources)?;
        snapshot.policy_revision = self
            .policy_projection
            .next_policy_revision(snapshot.policy_revision, inputs_changed);
        let current_revision = ZoneRevision::new(snapshot.policy_revision);
        let previous = self
            .policy_subject_fingerprints
            .lock()
            .map_err(|_| ResourceRuntimeError::IdentityUnbound)?
            .clone();
        let fingerprints = refreshed_policy_subject_fingerprints(&resources, &previous)?;
        let (policy, state) =
            d2bd_runtime::resource_runtime_support::compile_committed_policy_with_subjects(
                &self.zone,
                snapshot,
                current_revision,
                &self.bundle_resource_types,
                &resources,
                controller_subjects.iter().cloned(),
            )?;
        let rebind_core = self
            .system_core_rebind_pending
            .load(Ordering::Acquire)
            || self
                .core_controller_subject
                .lock()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
                .is_some();
        if rebind_core {
            self.system_core_rebind_pending.store(true, Ordering::Release);
        }
        self.install_policy_projection(policy, state.clone(), controller_subjects, policy_inputs)?;
        if let Err(error) = self.refresh_system_core_session_locked(state.clone()).await {
            // The policy projection was installed atomically before the
            // session rebind. Keep that complete projection available while
            // the retryable session failure is surfaced to the caller.
            self.system_core_rebind_pending.store(true, Ordering::Release);
            return Err(error);
        }
        *self
            .policy_subject_fingerprints
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)? = fingerprints;
        if rebind_core {
            self.system_core_rebind_pending
                .store(false, Ordering::Release);
        }
        Ok(())
    }
    async fn current_controller_policy_subjects(
        &self,
    ) -> Result<BTreeSet<BoundSubject>, ResourceRuntimeError> {
        let providers = self
            .controller_session_providers
            .lock()
            .map_err(|_| ResourceRuntimeError::PolicyUnavailable)?
            .clone();
        let subjects = self
            .current_controller_policy_subjects_from(providers.as_deref())
            .await?;
        if subjects.is_empty() && providers.is_none() {
            // Before Process Providers attach, preserve the last validated
            // external projection rather than replacing it with no subjects.
            return self.policy_projection.installed_controller_subjects();
        }
        Ok(subjects)
    }

    async fn current_controller_policy_subjects_from(
        &self,
        providers: Option<
            &crate::process_provider_runtime::ProductionProcessProviders,
        >,
    ) -> Result<BTreeSet<BoundSubject>, ResourceRuntimeError> {
        let plane = self.manager_plane_view()?;
        load_controller_policy_subjects(
            &self.zone,
            plane.as_ref(),
            providers,
            &self.controller_sessions,
        )
        .await
    }

    fn install_policy_projection(
        &self,
        policy: PolicySet,
        state: AuthorizationState,
        controller_subjects: BTreeSet<BoundSubject>,
        policy_inputs: [u8; 32],
    ) -> Result<(), ResourceRuntimeError> {
        self.policy_projection
            .install(policy, state, controller_subjects, policy_inputs)
    }

    /// Re-enroll the fixed internal system-core session after a policy
    /// revision change so its old lease cannot continue past the fence.
    #[allow(dead_code)]
    async fn refresh_system_core_session(
        &self,
        state: AuthorizationState,
    ) -> Result<(), ResourceRuntimeError> {
        let _session_guard = self.controller_session_lock.lock().await;
        self.refresh_system_core_session_locked(state).await
    }

    async fn refresh_system_core_session_locked(
        &self,
        state: AuthorizationState,
    ) -> Result<(), ResourceRuntimeError> {
        let existing_session = self
            .system_core_rebind_pending
            .load(Ordering::Acquire)
            || self
                .core_controller_subject
                .lock()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
                .is_some();
        let session_state = (
            self.registrar
                .lock()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
                .is_some(),
            self.ingress
                .lock()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
                .is_some(),
            self.service_task
                .lock()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
                .is_some(),
        );
        if session_state == (false, false, false) && !existing_session {
            return Ok(());
        }
        if !existing_session && session_state != (false, false, false) {
            return Err(ResourceRuntimeError::AuthenticationUnavailable);
        }
        *self
            .process_status_client
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? = None;
        let mut registrar = self
            .registrar
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .take()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        let ingress = self
            .ingress
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .take();
        let task = self
            .service_task
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        if let Some(mut ingress) = ingress {
            if let Err(error) = registrar.revoke_in_place(&mut ingress).await {
                tracing::warn!(
                    error = ?error,
                    "system-core session ingress revoke failed during rebind",
                );
                *self
                    .registrar
                    .lock()
                    .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? =
                    Some(registrar);
                return Err(ResourceRuntimeError::AuthenticationUnavailable);
            }
        }
        let (new_ingress, new_task, status_client, subject_context) =
            match register_system_core_session(
                &mut registrar,
                self.manager_api_service()?,
                Arc::clone(&self.authorizer),
                state,
            )
            .await
            {
                Ok(session) => session,
                Err(error) => {
                    *self
                        .registrar
                        .lock()
                        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? =
                        Some(registrar);
                    return Err(error);
                }
            };
        *self
            .registrar
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? = Some(registrar);
        *self
            .ingress
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? = Some(new_ingress);
        *self
            .service_task
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? = Some(new_task);
        *self
            .process_status_client
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? = Some(status_client);
        *self
            .core_controller_subject
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? = Some(subject_context);
        Ok(())
    }

    /// Resolve one public peer uid to its Zone `User`.
    ///
    /// `User` is a converted type: the manager is its identity and status
    /// authority (R11), so resolution reads the manager's `User` rows.
    async fn resolve_public_user(
        &self,
        peer_uid: u32,
        _operation_id: &str,
    ) -> Result<d2bd_runtime::resource_runtime_support::ResolvedZoneUser, ResourceRuntimeError>
    {
        let rows = self.manager_stored_rows("User").await?;
        d2bd_runtime::resource_runtime_support::resolve_zone_user_from_rows(
            &self.zone,
            peer_uid,
            &rows,
        )
    }

    /// Issue one sealed Guest lifecycle lease from the authenticated local
    /// peer and the current store identities.
    pub(crate) async fn admit_guest_lifecycle(
        &self,
        peer_uid: u32,
        target: ResourceRef,
        operation_id: &str,
    ) -> Result<d2b_resource_api::service::GuestLifecycleAdmission, ResourceRuntimeError> {
        self.refresh_authorization_policy().await?;
        let resolved_user = self.resolve_public_user(peer_uid, operation_id).await?;
        let context = d2bd_runtime::resource_runtime_support::local_user_subject_context(
            &self.zone,
            &resolved_user,
            operation_id,
        )?;
        let state = self.policy_projection.installed_state()?;
        let subject = self
            .authorizer
            .issue_authenticated_subject(context, state)
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
        self.manager_api_service()?
            .admit_guest_lifecycle(&subject, target, operation_id.to_owned())
            .await
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)
    }

    /// Issue a lifecycle lease through the already enrolled system-core
    /// ComponentSession for daemon-owned autostart.
    pub(crate) async fn admit_internal_guest_lifecycle(
        &self,
        target: ResourceRef,
        operation_id: &str,
    ) -> Result<d2b_resource_api::service::GuestLifecycleAdmission, ResourceRuntimeError> {
        let client = self.process_resource_client().ok_or_else(|| {
            tracing::warn!(
                zone = %self.zone.as_str(),
                guest = %target.to_canonical_string(),
                reason = "the internal system-core session holds no Resource API client",
                "internal Guest lifecycle admission refused",
            );
            ResourceRuntimeError::AuthenticationUnavailable
        })?;
        client
            .admit_guest_lifecycle(target.clone(), operation_id.to_owned())
            .await
            .map_err(|error| {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    error_kind = error.kind().as_str(),
                    reason = error.reason().as_str(),
                    "internal Guest lifecycle admission refused",
                );
                ResourceRuntimeError::AuthorizationUnavailable
            })
    }

    /// Read the immutable Guest and Provider identities needed by the
    /// guarded host-shutdown stop capability.
    pub(crate) async fn guest_lifecycle_identity(
        &self,
        target: &ResourceRef,
    ) -> Result<
        (
            ResourceUid,
            ResourceUid,
            ResourceGeneration,
            ResourceGeneration,
        ),
        ResourceRuntimeError,
    > {
        if target.resource_type().as_str() != "Guest" {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        // `Guest` is a converted type (U12): the manager is the authority
        // for its row, so the read merges manager-first exactly as
        // `committed_resource_value` does.
        let guest = self
            .committed_resource_stored(target, "guest-lifecycle-identity")
            .await?;
        if guest.zone != self.zone
            || guest.resource_ref != *target
            || guest.uid.as_str().is_empty()
            || guest.generation.get() == 0
        {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        let envelope = ResourceEnvelope::from_json(&guest.canonical_json)
            .map_err(|_| ResourceRuntimeError::RequestInvalid)?;
        if envelope.resource_type().as_str() != "Guest"
            || envelope.metadata().zone() != &self.zone
            || envelope.metadata().uid() != &guest.uid
            || envelope.metadata().generation() != guest.generation
            || envelope.metadata().revision() != guest.revision
            || envelope
                .digest()
                .map_err(|_| ResourceRuntimeError::RequestInvalid)?
                != guest.payload_digest
        {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        let provider_ref = envelope
            .spec()
            .provider_ref()
            .cloned()
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        // `Provider` is a converted type too (U12): manager-first.
        let provider = self
            .committed_resource_stored(&provider_ref, "guest-lifecycle-provider-identity")
            .await?;
        if provider.zone != self.zone
            || provider.resource_ref.resource_type().as_str() != "Provider"
            || provider.generation.get() == 0
        {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        let provider_envelope = ResourceEnvelope::from_json(&provider.canonical_json)
            .map_err(|_| ResourceRuntimeError::RequestInvalid)?;
        if provider_envelope.resource_type().as_str() != "Provider"
            || provider_envelope.metadata().zone() != &self.zone
            || provider_envelope.metadata().uid() != &provider.uid
            || provider_envelope.metadata().generation() != provider.generation
            || provider_envelope.metadata().revision() != provider.revision
            || provider_envelope
                .digest()
                .map_err(|_| ResourceRuntimeError::RequestInvalid)?
                != provider.payload_digest
        {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        Ok((
            self.bound_zone_uid()?,
            guest.uid,
            guest.generation,
            provider.generation,
        ))
    }

    /// Read the owning `Guest` row's durable uid for one canonical Guest
    /// reference.
    ///
    /// The pre-v3 store linked every owned row to its owner by uid: it
    /// resolved the row's `metadata.ownerRef` to the owner row's uid and
    /// carried that value in `record.owner_uid`
    /// (`@@REDB-D@@::transaction::resolve_uid_in_read`), and the
    /// old Process descriptor composer read it as the launch ticket's owner
    /// identity. `Guest` stays on this plane, so a converted Process row
    /// whose authored owner is a Guest can only reproduce the same durable
    /// value from here.
    pub(crate) async fn guest_owner_uid(
        &self,
        target: &ResourceRef,
    ) -> Result<ResourceUid, ResourceRuntimeError> {
        if target.resource_type().as_str() != "Guest" {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        // `Guest` is a converted type (U12): the manager row is the owner
        // authority, merged exactly as `committed_resource_value` merges it.
        let guest = self
            .committed_resource_stored(target, "guest-owner-uid")
            .await?;
        if guest.zone != self.zone
            || guest.resource_ref != *target
            || guest.uid.as_str().is_empty()
            || guest.generation.get() == 0
        {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        Ok(guest.uid)
    }

    /// Read the committed Provider route for one Guest without consulting
    /// the legacy manifest or process-DAG connector.
    pub(crate) async fn guest_provider_ref(
        &self,
        target: &ResourceRef,
    ) -> Result<ResourceRef, ResourceRuntimeError> {
        if target.resource_type().as_str() != "Guest" {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        let guest = self
            .committed_resource_stored(target, "guest-provider-route")
            .await?;
        let envelope = ResourceEnvelope::from_json(&guest.canonical_json)
            .map_err(|_| ResourceRuntimeError::RequestInvalid)?;
        let provider_ref = envelope
            .spec()
            .provider_ref()
            .cloned()
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        Ok(provider_ref)
    }

    /// Attach the published per-zone v3 planes (F1 wiring from the
    /// composition). Until the Zone's plane is published the manager-backed
    /// service cannot be built and converted types stay unavailable here.
    pub(crate) fn attach_v3_planes(
        &self,
        planes: Arc<
            parking_lot::Mutex<
                std::collections::HashMap<String, Arc<crate::resource_plane_v3::ResourcePlaneV3>>,
            >,
        >,
    ) {
        if let Ok(mut slot) = self.v3_planes.lock() {
            *slot = Some(Arc::clone(&planes));
        }
        // The manager is the only authority: the controller-session path (and
        // the Provider's dependency observation) read manager-served rows
        // through the same published table, so the session coordinator gets
        // the zone plane's manager view seam here, before activation. This
        // runs before the composition fills the table (it is published after
        // the per-zone loop), so the seam resolves the zone's plane per read.
        self.controller_session_coordinator()
            .attach_plane_view(Arc::new(PublishedPlaneControllerView::new(
                Arc::clone(&planes),
                self.zone.clone(),
            )));
    }

    /// The live controller-session reconnect generation, when one is
    /// enrolled (old shared Runner session generation).
    pub(crate) fn controller_session_generation(
        &self,
    ) -> Option<d2b_contracts_resource::v3::identity::ReconnectGeneration> {
        self.core_controller_subject
            .lock()
            .ok()
            .and_then(|subject| {
                subject
                    .as_ref()
                    .map(|subject| subject.reconnect_generation())
            })
    }

    /// The zone authority-index handle (read-only seam for the converted
    /// shared-provider effects).
    pub(crate) fn authority_index(
        &self,
    ) -> &Arc<tokio::sync::Mutex<HostGlobalAuthorityIndex>> {
        &self.authority_index
    }

    /// The published v3 plane for this Zone: the manager-backed live rows and
    /// status the shared-provider effects read (KTD3: the manager is the only
    /// status authority; this is a read-only view seam).
    pub(crate) fn v3_plane(
        &self,
    ) -> Result<Arc<crate::resource_plane_v3::ResourcePlaneV3>, ResourceRuntimeError> {
        let planes = self
            .v3_planes
            .lock()
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?
            .clone()
            .ok_or(ResourceRuntimeError::CapabilityUnavailable)?;
        planes
            .lock()
            .get(self.zone.as_str())
            .cloned()
            .ok_or(ResourceRuntimeError::CapabilityUnavailable)
    }

    /// The manager-served durable rows of one resource type, rendered through
    /// the same projection the manager-backed API serves (U12 reader bridge,
    /// mirroring G5).
    ///
    /// Converted types are written to the manager, not to the pre-v3 store, so
    /// the readers that still take a store handle merge these in. The
    /// contract is `Ok(empty)`-keeps-the-old-path: an unconverted type, an
    /// unpublished plane, or a manager without the row leaves the caller on
    /// the durable store. A manager RPC failure is an error - never reported
    /// as absence (G5).
    pub(crate) async fn manager_stored_rows(
        &self,
        resource_type: &str,
    ) -> Result<Vec<StoredResource>, ResourceRuntimeError> {
        let plane = self.manager_plane_view()?;
        bridge_manager_rows(plane.as_ref(), resource_type).await
    }

    /// Every committed `Process`/`EphemeralProcess` row, read from the
    /// manager (the controller-session fence's generic Process list).
    pub(crate) async fn committed_process_rows(
        &self,
    ) -> Result<Vec<StoredResource>, ResourceRuntimeError> {
        let mut rows = self.manager_stored_rows("Process").await?;
        rows.extend(self.manager_stored_rows("EphemeralProcess").await?);
        Ok(rows)
    }

    /// The manager's authoritative identity for one committed ref, when the
    /// manager holds the row. A manager read failure is an error, never
    /// absence.
    pub(crate) async fn committed_manager_identity(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<(ResourceUid, ResourceGeneration)>, ResourceRuntimeError> {
        let plane = self.manager_plane_view()?;
        let Some(row) = bridge_manager_row(plane.as_ref(), target).await? else {
            return Ok(None);
        };
        if row.zone != self.zone || row.generation.get() == 0 {
            return Err(ResourceRuntimeError::StoreReadFailed);
        }
        Ok(Some((row.uid, row.generation)))
    }

    /// The plane-view seam over the published per-zone plane. A missing
    /// plane is a read failure the callers retry: with one plane there is no
    /// second authority to fall back to.
    fn manager_plane_view(&self) -> Result<Arc<dyn ControllerPlaneView>, ResourceRuntimeError> {
        let plane = self.v3_plane()?;
        Ok(Arc::new(ManagerControllerPlaneView::new(
            plane.client().clone(),
            self.zone.clone(),
        )))
    }

    /// The committed policy inputs for this Zone, read from the manager.
    pub(crate) async fn committed_policy_resources(
        &self,
    ) -> Result<Vec<StoredResource>, ResourceRuntimeError> {
        let plane = self.manager_plane_view()?;
        committed_policy_resources(plane.as_ref()).await
    }

    /// The production Core-driver effects port (U12): this zone's live
    /// controller-session coordinator, the same seam the G5 reader bridge
    /// uses. The plane wires it into its Core resource driver factory.
    pub(crate) fn core_driver_effects(&self) -> Arc<dyn crate::core_driver::CoreDriverEffects> {
        self.controller_session_coordinator()
    }

    fn manager_api_service(
        &self,
    ) -> Result<Arc<ResourceService<d2b_resource_api::manager_backend::ManagerBackend>>, ResourceRuntimeError>
    {
        if let Some(service) = self
            .v3_api
            .lock()
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?
            .clone()
        {
            return Ok(service);
        }
        let planes = self
            .v3_planes
            .lock()
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?
            .clone()
            .ok_or(ResourceRuntimeError::CapabilityUnavailable)?;
        let plane = planes
            .lock()
            .get(self.zone.as_str())
            .cloned()
            .ok_or(ResourceRuntimeError::CapabilityUnavailable)?;
        let zone_uid = self
            .authority_identity
            .as_ref()
            .map(|identity| identity.zone_uid().clone());
        let acceptor = self
            .manager_authorizer
            .take_store_seal(manager_plane_seal_identity(&self.zone, zone_uid.clone())?)
            .map_err(|_| ResourceRuntimeError::StoreSealUnavailable)?;
        let backend = d2b_resource_api::manager_backend::ManagerBackend::new(
            plane.client().clone(),
            Arc::clone(plane.hub()),
            acceptor,
        );
        let service = Arc::new(
            ResourceService::new_with_zone_uid(
                Arc::new(backend),
                Arc::clone(&self.manager_authorizer),
                zone_uid,
            )
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?,
        );
        let mut slot = self
            .v3_api
            .lock()
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?;
        *slot = Some(Arc::clone(&service));
        Ok(service)
    }

    /// The manager plane's authorizer (same catalog, own seal); `None` when
    /// this runtime never serves the v3 plane.
    fn manager_plane_authorizer(&self) -> Result<Arc<NativeAuthorizer>, ResourceRuntimeError> {
        self.policy_projection
            .manager_authorizer
            .as_ref()
            .map(Arc::clone)
            .ok_or(ResourceRuntimeError::AuthorizationUnavailable)
    }

    /// Borrow the daemon-owned Resource API client used by the target-local
    /// process reconciler. The client is present only after the Zone's
    /// authenticated system-core session has been enrolled.
    pub(crate) fn process_resource_client(
        &self,
    ) -> Option<Arc<ResourceApiClient<ZoneApiBackend, UnavailableUpgradeDispatcher>>> {
        if self.system_core_rebind_pending.load(Ordering::Acquire) {
            return None;
        }
        self.process_status_client
            .lock()
            .ok()
            .and_then(|client| client.clone())
    }

    /// Gate a Guest-local typed Credential session through this Zone's
    /// authenticated ResourceService before it can read a lease.
    #[allow(dead_code)]
    pub(crate) fn scoped_credential_client(
        &self,
        session: Option<&d2bd_runtime::guest_component_session::GuestComponentSessionClient>,
        delegate: Arc<dyn d2b_provider_transport_azure_relay::ScopedCredentialClient>,
    ) -> Result<
        Arc<crate::credential_resource_runtime::SameZoneScopedCredentialClient>,
        ResourceRuntimeError,
    > {
        let Some(session) = session else {
            return Err(ResourceRuntimeError::ResourceApiBindFailed);
        };
        crate::credential_resource_runtime::SameZoneScopedCredentialClient::with_component_session(
            self.zone.clone(),
            session,
            delegate,
        )
        .map(Arc::new)
        .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)
    }

    fn status_client(
        &self,
    ) -> Result<
        Arc<ResourceApiClient<ZoneApiBackend, UnavailableUpgradeDispatcher>>,
        ResourceRuntimeError,
    > {
        self.process_status_client
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)
    }

    /// The production effect port for the v3 `Credential` driver (U12 KTD3).
    ///
    /// Provider and execution-target facts read the same durable rows the old
    /// U10 runner's dependency snapshots carried, so the driver's readiness
    /// fence is the preserved `phase == Ready` at the row's current
    /// generation. The managed-identity agent probe is supplied by the
    /// composition unit: the agent is a v3 manager row, so its live status
    /// belongs to the per-zone plane's manager (R11). Lease facts have no
    /// in-tree writer while the old status surface is being deleted, so the
    /// port reports them absent - the old "no lease state" case, which skips
    /// revocation rather than guessing.
    pub(crate) fn credential_driver_effects(
        &self,
        agent_ready: Arc<dyn for<'a> Fn(&'a ResourceRef) -> AgentReadyFuture<'a> + Send + Sync>,
    ) -> Arc<dyn CredentialDriverEffects> {
        let facts_planes = Arc::clone(&self.v3_planes);
        let facts_zone = self.zone.clone();
        Arc::new(ProductionCredentialDriverEffects::new(
            Arc::new(move |provider_ref: &ResourceRef, execution_ref: &ResourceRef| {
                let planes = Arc::clone(&facts_planes);
                let zone = facts_zone.clone();
                let provider_ref = provider_ref.clone();
                let execution_ref = execution_ref.clone();
                Box::pin(async move {
                    let plane = published_plane_view(&planes, &zone)?;
                    credential_dependency_facts(plane.as_ref(), &provider_ref, &execution_ref).await
                })
            }),
            Arc::new(|_credential_ref: &ResourceRef| Box::pin(async { None })),
            agent_ready,
            self.credential_sessions.clone(),
        ))
    }

    /// Drive the complete Wave 6 acceptance sequence through the
    /// authenticated public Resource API and the production Provider
    /// boundary.
    ///
    /// This is intentionally an explicit orchestration entry point rather
    /// than a second controller implementation. The Resource API selects the
    /// durable objects, while the supplied boundary invokes the shipped
    /// Volume, Network, Device TPM, and Cloud Hypervisor controllers.
    pub async fn reconcile_wave6_operator_acceptance<B>(
        &self,
        client: &ResourceApiClient<ZoneApiBackend, UnavailableUpgradeDispatcher>,
        boundary: &B,
    ) -> Result<Wave6AcceptanceReport, ResourceRuntimeError>
    where
        B: Wave6ProviderBoundary,
    {
        if !self.readiness.is_ready() {
            return Err(ResourceRuntimeError::PlaneUnavailable);
        }
        let resources = select_wave6_resources(client)
            .await
            .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?;

        let require_ready = |result: Wave6ReconcileResult| {
            if matches!(result, Wave6ReconcileResult::Ready) {
                Ok(())
            } else {
                Err(ResourceRuntimeError::Wave6AcceptanceFailed)
            }
        };

        require_ready(
            boundary
                .reconcile_volume(&resources.volume)
                .await
                .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?,
        )?;
        require_ready(
            boundary
                .reconcile_device_tpm(&resources.device_tpm)
                .await
                .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?,
        )?;

        if !matches!(
            boundary
                .reconcile_network(
                    &resources.network,
                    Wave6Dependencies::network_waiting_for_volume(),
                )
                .await
                .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?,
            Wave6ReconcileResult::Waiting
        ) {
            return Err(ResourceRuntimeError::Wave6AcceptanceFailed);
        }
        if !matches!(
            boundary
                .reconcile_cloud_hypervisor_guest(
                    &resources.cloud_hypervisor_guest,
                    Wave6Dependencies::guest_waiting_for_network(),
                )
                .await
                .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?,
            Wave6ReconcileResult::Waiting
        ) {
            return Err(ResourceRuntimeError::Wave6AcceptanceFailed);
        }

        require_ready(
            boundary
                .reconcile_network(
                    &resources.network,
                    Wave6Dependencies::network_ready_for_guest(),
                )
                .await
                .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?,
        )?;
        require_ready(
            boundary
                .reconcile_cloud_hypervisor_guest(
                    &resources.cloud_hypervisor_guest,
                    Wave6Dependencies::guest_ready_for_adoption(),
                )
                .await
                .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?,
        )?;
        require_ready(
            boundary
                .reconcile_network(
                    &resources.network,
                    Wave6Dependencies::guest_ready_for_adoption(),
                )
                .await
                .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?,
        )?;

        boundary
            .adopt_after_restart(&resources)
            .await
            .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?;
        boundary
            .remove_cloud_hypervisor_guest(&resources.cloud_hypervisor_guest)
            .await
            .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?;
        boundary
            .remove_network(&resources.network)
            .await
            .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?;
        let device_state_retained = boundary
            .remove_device_tpm(&resources.device_tpm)
            .await
            .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?;
        if !device_state_retained {
            return Err(ResourceRuntimeError::Wave6AcceptanceFailed);
        }
        boundary
            .remove_volume(&resources.volume)
            .await
            .map_err(|_| ResourceRuntimeError::Wave6AcceptanceFailed)?;

        Ok(Wave6AcceptanceReport {
            resources,
            ready: true,
            adopted_after_restart: true,
            removed: true,
            device_state_retained,
        })
    }

    /// Borrow sealed, committed interaction Provider configuration when the
    /// Zone declares the complete interaction Provider set.
    pub(crate) fn interaction_provider_configuration(
        &self,
    ) -> Option<&CommittedInteractionProviderConfiguration> {
        self.interaction_provider_configuration.as_ref()
    }

    pub(crate) fn interaction_identity(&self) -> Option<&CommittedInteractionIdentity> {
        self.interaction_identity.as_ref()
    }

    pub(crate) const fn interaction_state(&self) -> InteractionState {
        self.interaction_state
    }

    /// Resolve the one committed WaylandSession that owns a VM's display
    /// lifecycle. A missing row is reported separately so VM start can fail
    /// closed without inventing a display process or session identity.
    pub(crate) async fn committed_wayland_session_for_vm(
        &self,
        vm: &str,
    ) -> Result<Option<(ResourceRef, ResourceUid, WaylandSessionSpec)>, ResourceRuntimeError> {
        if !self.readiness.resource_api_ready {
            return Err(ResourceRuntimeError::PlaneUnavailable);
        }
        let expected_guest = ResourceRef::parse(&format!("Guest/{vm}"))
            .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
        let Some(identity) = self.interaction_identity.as_ref() else {
            return Ok(None);
        };
        if identity.subject_ref() != &expected_guest {
            return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
        }
        let plane = self.manager_plane_view()?;
        let resource = current_committed_resource(
            plane.as_ref(),
            &self.zone,
            identity.wayland_session_ref(),
            "interaction-wayland-session-current",
        )
        .await?;
        let spec = committed_wayland_session_spec(&self.zone, &resource)?;
        if spec.guest_ref() != &expected_guest || !spec.cross_domain_trusted() {
            return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
        }
        if identity.wayland_session_ref() != &resource.resource_ref
            || identity.wayland_session_uid() != &resource.uid
            || identity.subject_ref() != spec.guest_ref()
            || identity.host_execution_ref() != spec.host_ref()
            || identity.user_ref() != spec.user_ref()
        {
            return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
        }
        let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
            .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
        let deletion_requested = CanonicalJsonValue::parse(&resource.canonical_json)
            .ok()
            .is_some_and(|value| match value {
                CanonicalJsonValue::Object(root) => root
                    .get("metadata")
                    .and_then(CanonicalJsonValue::as_object)
                    .and_then(|metadata| metadata.get("deletionRequestedAt"))
                    .is_some_and(|value| !matches!(value, CanonicalJsonValue::Null)),
                _ => false,
            });
        if matches!(
            envelope.status().phase(),
            ResourcePhase::Failed | ResourcePhase::Deleted
        ) || deletion_requested
        {
            return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
        }
        Ok(Some((resource.resource_ref, resource.uid, spec)))
    }

    /// Return the current core-controller stage.
    pub fn core_stage(&self) -> Result<StartupStage, ResourceRuntimeError> {
        self.core
            .lock()
            .map(|core| core.stage())
            .map_err(|_| ResourceRuntimeError::CoreStartupFailed)
    }

    /// Borrow the production Zone status projection.
    pub fn zone_status(&self) -> Result<ZoneStatusResource, ResourceRuntimeError> {
        self.zone_status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)
    }

    /// Read committed resources for a root-owned Provider admission scan.
    ///
    /// This bypasses caller authorization intentionally: the result is used
    /// only by the root supervisor to resolve same-Zone attachment
    /// relationships before host effects. It never crosses the public API.
    pub(crate) async fn committed_resources_of_type(
        &self,
        resource_type: &str,
    ) -> Result<Vec<Value>, ResourceRuntimeError> {
        ResourceTypeName::parse(resource_type.to_owned())
            .map_err(|_| ResourceRuntimeError::RequestInvalid)?;
        self.manager_stored_rows(resource_type)
            .await?
            .into_iter()
            .map(|row| {
                serde_json::from_slice::<Value>(&row.canonical_json)
                    .map_err(|_| ResourceRuntimeError::StoreReadFailed)
            })
            .collect()
    }

    pub(crate) async fn committed_resource_value(
        &self,
        target: &ResourceRef,
        _operation_id: &str,
    ) -> Result<Value, ResourceRuntimeError> {
        self.committed_resource_optional(target)
            .await?
            .ok_or(ResourceRuntimeError::StoreReadFailed)
    }

    /// [`Self::committed_resource_value`] with an explicit absence answer for
    /// callers whose "not ready" case is exactly "the row is not there"
    /// (a provider controller has not committed it yet). A manager failure is
    /// still an error, never absence.
    pub(crate) async fn committed_resource_optional(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<Value>, ResourceRuntimeError> {
        let plane = self.manager_plane_view()?;
        match bridge_manager_row(plane.as_ref(), target).await? {
            Some(row) => serde_json::from_slice::<Value>(&row.canonical_json)
                .map(Some)
                .map_err(|_| ResourceRuntimeError::StoreReadFailed),
            None => Ok(None),
        }
    }

    /// [`Self::committed_resource_optional`] for a row whose authority is the
    /// manager: a row the manager does not hold is `Ok(None)` - the honest
    /// not-committed answer; an unpublished plane or a manager RPC failure is
    /// an error the callers retry, never absence.
    pub(crate) async fn committed_manager_row_optional(
        &self,
        target: &ResourceRef,
    ) -> Result<Option<Value>, ResourceRuntimeError> {
        let plane = self.manager_plane_view()?;
        let Some(row) = bridge_manager_row(plane.as_ref(), target).await? else {
            return Ok(None);
        };
        serde_json::from_slice::<Value>(&row.canonical_json)
            .map(Some)
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)
    }

    /// The committed row as the store-shaped record. Callers that need the
    /// row's identity fields (uid, generation, revision) read them here.
    pub(crate) async fn committed_resource_stored(
        &self,
        target: &ResourceRef,
        _operation_id: &str,
    ) -> Result<StoredResource, ResourceRuntimeError> {
        let plane = self.manager_plane_view()?;
        bridge_manager_row(plane.as_ref(), target)
            .await?
            .ok_or(ResourceRuntimeError::StoreReadFailed)
    }

    /// Publish a validated status projection from the real system-core
    /// handler observations.
    pub fn publish_zone_status(&self, input: ZoneStatusInput) -> Result<(), ResourceRuntimeError> {
        let status = SystemCoreStatusEmitter::new()
            .emit(input)
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        self.zone_status
            .lock()
            .map(|mut current| {
                *current = status;
            })
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)
    }

    /// Refresh provider counts and other live metadata without replacing the
    /// currently observed handler phases.
    pub fn publish_runtime_metadata(
        &self,
        runtime: ZoneRuntimeMetadata,
    ) -> Result<(), ResourceRuntimeError> {
        let current = self.zone_status()?;
        self.publish_zone_status(
            ZoneStatusInput::new(current.core_controller_phase(), current.handlers().to_vec())
                .with_runtime_metadata(runtime),
        )
    }

    /// Publish the provider registry's live counts while retaining store and
    /// handler metadata already projected into status.
    pub fn publish_provider_counts(
        &self,
        installed_provider_count: u32,
        ready_provider_count: u32,
    ) -> Result<(), ResourceRuntimeError> {
        let current = self.zone_status()?;
        let mut runtime = zone_runtime_metadata(
            &self.committed_policy_snapshot(),
            current.total_resource_count(),
            current.generation_cleanup_pending(),
            current.cleanup_pending_count(),
            Some(current_status_timestamp()),
        );
        runtime.installed_provider_count = installed_provider_count;
        runtime.ready_provider_count = ready_provider_count;
        self.publish_runtime_metadata(runtime)
    }

    /// Mark the trusted Provider path after the daemon has configured it.
    ///
    /// Provider configuration is loaded outside this Zone store boundary, so
    /// `open` cannot claim this bit from the descriptor alone.
    pub fn set_provider_path_ready(&mut self, ready: bool) {
        self.readiness.provider_path_ready = ready;
    }

    /// Install the integrity-pinned semantic Guest setup descriptors supplied
    /// by the trusted artifact catalog.
    pub(crate) fn set_guest_setup_descriptors(
        &mut self,
        descriptors: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) {
        self.guest_setup_descriptors = descriptors.into_iter().collect();
    }

    pub(crate) fn set_guest_setup_descriptor_catalog_keys(
        &mut self,
        keys: impl IntoIterator<Item = (String, String)>,
    ) {
        self.guest_setup_descriptor_catalog_keys = keys.into_iter().collect();
    }

    /// Reconcile Cloud Hypervisor Guests through the controller-owned child
    /// graph. The controller receives only an authenticated Resource API
    /// adapter and a verified private descriptor.
    pub(crate) async fn reconcile_cloud_hypervisor_guests(
        &self,
        state: Arc<crate::ServerState>,
    ) -> Result<(), ResourceRuntimeError> {
        self.reconcile_cloud_hypervisor_guests_inner(state, None, None)
            .await
            .map(|_| ())
    }

    /// Reconcile one Cloud Hypervisor Guest selected by the converted Guest
    /// driver.
    ///
    /// `status_sink` is the driver's capture point for the provider
    /// controller's Guest status write: `Guest` is a converted type, so the
    /// row's actor owns its status (R11) and there is no durable row to
    /// write. The legacy relist helper remains available to explicit
    /// lifecycle commands with no sink.
    pub(crate) async fn reconcile_cloud_hypervisor_guest_with_status(
        &self,
        state: Arc<crate::ServerState>,
        guest_ref: &ResourceRef,
        status_sink: Option<crate::guest_driver::GuestStatusSink>,
    ) -> Result<CloudHypervisorReconcileOutcome, ResourceRuntimeError> {
        self.reconcile_cloud_hypervisor_guests_inner(state, Some(guest_ref), status_sink)
            .await
    }

    async fn reconcile_cloud_hypervisor_guests_inner(
        &self,
        state: Arc<crate::ServerState>,
        selected_guest: Option<&ResourceRef>,
        status_sink: Option<crate::guest_driver::GuestStatusSink>,
    ) -> Result<CloudHypervisorReconcileOutcome, ResourceRuntimeError> {
        if !self.readiness.resource_api_ready {
            return Ok(CloudHypervisorReconcileOutcome::Pending);
        }
        let _guard = self.cloud_hypervisor_reconcile_lock.lock().await;
        let client = self.cloud_hypervisor_resource_client().inspect_err(|error| {
            tracing::warn!(error = ?error, "Cloud Hypervisor reconcile stage failed: controller-client");
        })?;
        let guests = match selected_guest {
            Some(guest_ref) => vec![guest_ref.clone()],
            None => self
                .list_cloud_hypervisor_guests()
                .await
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)?,
        };
        let mut overall_outcome = CloudHypervisorReconcileOutcome::Ready;
        for guest_ref in guests {
            if selected_guest.is_some_and(|selected| selected != &guest_ref) {
                continue;
            }
            let Some(descriptor_bytes) =
                self.guest_setup_descriptors.get(guest_ref.name().as_str())
            else {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    guest = %guest_ref.name().as_str(),
                    "Cloud Hypervisor Guest controller descriptor is unavailable",
                );
                continue;
            };
            let Some(expected_key) = self
                .guest_setup_descriptor_catalog_keys
                .get(guest_ref.name().as_str())
            else {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    guest = %guest_ref.name().as_str(),
                    "Cloud Hypervisor Guest descriptor catalog key is unavailable",
                );
                continue;
            };
            let mut guest_outcome = CloudHypervisorReconcileOutcome::Ready;
            let descriptor = GuestSetupDescriptor::from_canonical_bytes(descriptor_bytes)
                .map_err(|_| {
                    tracing::warn!("Cloud Hypervisor reconcile stage failed: descriptor-decode");
                    ResourceRuntimeError::CapabilityUnavailable
                })?
                .verify_with(&CatalogDescriptorVerifier {
                    expected_key: expected_key.clone(),
                })
                .map_err(|_| {
                    tracing::warn!("Cloud Hypervisor reconcile stage failed: descriptor-verify");
                    ResourceRuntimeError::CapabilityUnavailable
                })?;
            let (provider_ref, execution_ref, config, graph) =
                self.cloud_hypervisor_inputs(&guest_ref).await.inspect_err(|error| {
                    tracing::warn!(error = ?error, "Cloud Hypervisor reconcile stage failed: inputs");
                })?;
            let (_, guest_uid, guest_generation, provider_assignment_generation) = self
                .guest_lifecycle_identity(&guest_ref)
                .await
                .inspect_err(|error| {
                    tracing::warn!(error = ?error, "Cloud Hypervisor reconcile stage failed: lifecycle-identity");
                })?;
            let lifecycle_intent = match state.provider_runtime.latest_v3_lifecycle_operation(
                &provider_ref,
                &self.bound_zone_uid()?,
                &guest_ref,
                &guest_uid,
                guest_generation,
                provider_assignment_generation,
                self.committed_policy_snapshot().policy_revision,
            ) {
                Ok(intent) => intent,
                Err(
                    crate::provider_effects::ProviderEffectError::ProviderNotRegistered
                    | crate::provider_effects::ProviderEffectError::RegistryUnavailable,
                ) => None,
                Err(error) => {
                    tracing::warn!(
                        error = ?error,
                        "Cloud Hypervisor reconcile stage failed: lifecycle-intent",
                    );
                    return Err(ResourceRuntimeError::CapabilityUnavailable);
                }
            }
            .map(|operation| match operation {
                crate::provider_effects::GuestLifecycleOperation::Stop => DesiredLifecycle::Stopped,
                crate::provider_effects::GuestLifecycleOperation::Start
                | crate::provider_effects::GuestLifecycleOperation::Restart => {
                    DesiredLifecycle::Running
                }
            });
            let guest_session_target =
                crate::resolve_committed_guest_session_target(self, &guest_ref)
                    .await
                    .inspect_err(|error| {
                        tracing::debug!(
                            error = ?error,
                            guest = %guest_ref.to_canonical_string(),
                            "guest session target resolution failed during reconcile",
                        );
                    })
                    .ok();
            // Deletion may reuse the already authenticated live session below,
            // but it never creates a new session solely to clear a finalizer.
            // A durable Closed marker therefore remains terminal for session
            // custody during finalization.
            let guest_session = match guest_session_target.as_ref() {
                Some(target) => state
                    .guest_component_sessions
                    .lock()
                    .await
                    .get(&target.key())
                    .cloned(),
                None => None,
            };
            let session_vmm_ready = match (
                guest_session_target.as_ref(),
                guest_session.as_ref(),
                crate::load_bundle_resolver(&state),
            ) {
                (Some(target), Some(_), Ok(resolver)) => {
                    crate::resolve_component_session_endpoint_for_guest(
                        &state, &resolver, self, target,
                    )
                    .await
                    .is_ok()
                }
                _ => false,
            };
            let session_evidence = guest_session.as_ref().and_then(|session| {
                guest_session_target.as_ref().and_then(|target| {
                    guest_session_evidence(
                        &guest_ref,
                        session.as_ref(),
                        &descriptor,
                        target,
                        session_vmm_ready,
                    )
                })
            });
            let finalizer_clear_requested = Arc::new(AtomicBool::new(false));
            self.ensure_cloud_hypervisor_controller_deployment(
                &provider_ref,
                &config,
            )
            .await
            .inspect_err(|error| {
                tracing::warn!(error = ?error, "Cloud Hypervisor reconcile stage failed: deployment");
            })?;
            // U17 child bridge: the controller's deterministic children are
            // converted types, so their rows belong to the manager - the
            // pre-v3 store has no actor to launch them. The plane is
            // published before any controller session runs; a plane that is
            // not there yet leaves the child mutations refused (retryable),
            // never silently written to the pre-v3 store.
            let plane_children = self.v3_plane().ok().map(|plane| {
                PlaneChildMutations::new(plane, self.zone.clone(), guest_ref.clone())
            });
            let session = CloudHypervisorResourceSession {
                client: Arc::clone(&client),
                status_sink: status_sink.clone(),
                plane_children,
                providers: state
                    .provider_runtime
                    .process_providers()
                    .ok_or_else(|| {
                        tracing::warn!(
                            zone = %self.zone.as_str(),
                            guest = %guest_ref.name().as_str(),
                            stage = "process-providers",
                            "Cloud Hypervisor reconcile stage failed",
                        );
                        ResourceRuntimeError::ProviderPathUnavailable
                    })?,
                guest_sessions: Arc::clone(&state.guest_component_sessions),
                closed_guest_sessions: Arc::clone(&self.closed_guest_sessions),
                zone: self.zone.clone(),
                zone_uid: self.bound_zone_uid()?,
                policy_revision: self.committed_policy_snapshot().policy_revision,
                provider_ref,
                execution_ref,
                descriptor: descriptor.clone(),
                controller_generation: self
                    .committed_policy_snapshot()
                    .controller_generation
                    .unwrap_or_else(|| ControllerGeneration::new(1).expect("generation one")),
                session_target: guest_session_target,
                session_evidence,
                suppress_finalizer_clear: selected_guest.is_some(),
                finalizer_clear_requested: Arc::clone(&finalizer_clear_requested),
            };
            let adapter = AuthenticatedResourceApiAdapter::new(Arc::new(session));
            let mut controller = CloudHypervisorController::from_verified_descriptor(
                config,
                graph,
                descriptor,
                Arc::new(adapter),
            )
            .map(|controller| controller.with_lifecycle_intent(lifecycle_intent))
            .map_err(|_| {
                tracing::warn!("Cloud Hypervisor reconcile stage failed: controller-construction");
                ResourceRuntimeError::CapabilityUnavailable
            })?;
            controller
                .register()
                .await
                .map_err(|error| {
                    tracing::warn!(
                        zone = %self.zone.as_str(),
                        guest = %guest_ref.name().as_str(),
                        stage = "controller-register",
                        error = ?error,
                        "Cloud Hypervisor reconcile stage failed",
                    );
                    ResourceRuntimeError::AuthenticationUnavailable
                })?;
            if let Err(error) = controller.reconcile(&guest_ref).await {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    guest = %guest_ref.name().as_str(),
                    error = %error,
                    "Cloud Hypervisor Guest controller reconcile refused",
                );
                return Err(ResourceRuntimeError::CapabilityUnavailable);
            }
            let deleting_or_gone = self
                .committed_resource_value(&guest_ref, "cloud-hypervisor-deletion-state")
                .await
                .map(|value| {
                    value
                        .pointer("/metadata/deletionRequestedAt")
                        .is_some_and(|value| !value.is_null())
                })
                .unwrap_or(true);
            if deleting_or_gone {
                if selected_guest.is_some()
                    && !finalizer_clear_requested.load(Ordering::Acquire)
                {
                    return Err(ResourceRuntimeError::CapabilityUnavailable);
                }
                continue;
            }
            match self
                .reconcile_cloud_hypervisor_setup_volume(&state, &guest_ref)
                .await
            {
                Ok(CloudHypervisorSetupVolumeOutcome::Ready) => {}
                Ok(CloudHypervisorSetupVolumeOutcome::Pending) => {
                    overall_outcome = CloudHypervisorReconcileOutcome::Pending;
                    tracing::debug!(
                        zone = %self.zone.as_str(),
                        guest = %guest_ref.name().as_str(),
                        stage = "setup-volume",
                        "Cloud Hypervisor setup Volume is not committed yet",
                    );
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        zone = %self.zone.as_str(),
                        guest = %guest_ref.name().as_str(),
                        stage = "setup-volume",
                        error = ?error,
                        "Cloud Hypervisor reconcile stage failed",
                    );
                    return Err(error);
                }
            }
            controller.reconcile(&guest_ref).await.map_err(|error| {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    guest = %guest_ref.name().as_str(),
                    stage = "post-setup-controller-reconcile",
                    error = %error,
                    "Cloud Hypervisor Guest post-setup reconcile refused",
                );
                ResourceRuntimeError::CapabilityUnavailable
            })?;
            match self.reconcile_cloud_hypervisor_endpoints(&guest_ref).await {
                Ok(CloudHypervisorEndpointOutcome::Ready) => {}
                Ok(CloudHypervisorEndpointOutcome::Pending) => {
                    guest_outcome = CloudHypervisorReconcileOutcome::Pending;
                    overall_outcome = CloudHypervisorReconcileOutcome::Pending;
                    tracing::debug!(
                        zone = %self.zone.as_str(),
                        guest = %guest_ref.name().as_str(),
                        stage = "endpoint-publication",
                        "Cloud Hypervisor endpoint publication remains pending",
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        zone = %self.zone.as_str(),
                        guest = %guest_ref.name().as_str(),
                        stage = "endpoint-publication",
                        error = ?error,
                        "Cloud Hypervisor reconcile stage failed",
                    );
                    return Err(error);
                }
            }
            if guest_outcome == CloudHypervisorReconcileOutcome::Pending {
                continue;
            }
            match crate::resolve_committed_guest_session_target(self, &guest_ref).await {
                Ok(target) => {
                    if let Err(error) =
                        crate::connect_guest_component_session_for_guest(&state, &target).await
                    {
                        tracing::warn!(
                            error = %error,
                            "Cloud Hypervisor Guest ComponentSession connection failed",
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "Cloud Hypervisor Guest ComponentSession target resolution failed",
                    );
                }
            }
            controller.reconcile(&guest_ref).await.map_err(|error| {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    guest = %guest_ref.name().as_str(),
                    stage = "post-process-controller-reconcile",
                    error = %error,
                    "Cloud Hypervisor Guest post-Process reconcile refused",
                );
                ResourceRuntimeError::CapabilityUnavailable
            })?;
        }
        Ok(overall_outcome)
    }

    async fn reconcile_cloud_hypervisor_endpoints(
        &self,
        guest_ref: &ResourceRef,
    ) -> Result<CloudHypervisorEndpointOutcome, ResourceRuntimeError> {
        let process_ref = deterministic_child_ref(guest_ref, ChildRole::VmmProcess)
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        // U12/G5 reader bridge: `Process` is manager-owned (KTD3/R11) and the
        // pre-v3 store keeps only the mirror the bundle materialization wrote,
        // which nothing updates once the row is manager-served. Reading the
        // mirror here defers endpoint publication behind a VMM Process the
        // manager reports `Ready`; the manager row is authoritative where both
        // hold the reference, exactly as `committed_resource_value` merges it.
        let process = self
            .committed_resource_value(&process_ref, "cloud-hypervisor-endpoint-process")
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        let process_bytes =
            serde_json::to_vec(&process).map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
        let process_envelope = ResourceEnvelope::from_json(&process_bytes)
            .map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
        if process_envelope.metadata().owner_ref() != Some(guest_ref) {
            tracing::warn!(
                zone = %self.zone.as_str(),
                guest = %guest_ref.name().as_str(),
                owner = ?process_envelope
                    .metadata()
                    .owner_ref()
                    .map(ResourceRef::to_canonical_string),
                "Cloud Hypervisor endpoint publication refused: VMM Process owner mismatch",
            );
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
        let process_phase = process_envelope.status().phase();
        match child_publication_gate(process_phase, row_status_failure_is_retryable(&process)) {
            ChildPublicationGate::Ready => {}
            ChildPublicationGate::Pending => {
                // A `Failed` row whose actor classified the failure retryable
                // is the child's own retry in progress (R13: retryable
                // failures requeue), so the endpoints wait exactly as they
                // wait for the process to come up. Only a terminal failure is
                // refused here.
                tracing::debug!(
                    zone = %self.zone.as_str(),
                    guest = %guest_ref.name().as_str(),
                    phase = ?process_phase,
                    retrying = row_status_failure_is_retryable(&process),
                    "Cloud Hypervisor endpoint publication deferred until VMM Process is Ready",
                );
                return Ok(CloudHypervisorEndpointOutcome::Pending);
            }
            ChildPublicationGate::Terminal => {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    guest = %guest_ref.name().as_str(),
                    phase = ?process_phase,
                    "Cloud Hypervisor endpoint publication refused: terminal VMM Process phase",
                );
                return Err(ResourceRuntimeError::CapabilityUnavailable);
            }
        }
        let provider_ref = ResourceRef::parse("Provider/runtime-cloud-hypervisor")
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        for role in [ChildRole::ChApiEndpoint, ChildRole::GuestControlEndpoint] {
            let endpoint_ref = deterministic_child_ref(guest_ref, role)
                .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
            // U17: `Endpoint` is manager-owned and the row's actor owns its
            // status (KTD3/R11, AE6), so this stage has no durable status to
            // publish - the plane's endpoint realization is the only writer.
            // It observes the published phase and defers until the endpoint
            // (and therefore the API socket it carries) is Ready. A
            // retryable `Failed` is that actor's own retry in progress - the
            // realize effect's bounded wait for the VMM evidence ends
            // retryable while the VMM is still coming up - and defers too.
            let endpoint = self
                .committed_resource_value(&endpoint_ref, "cloud-hypervisor-endpoint")
                .await
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
            let endpoint_bytes =
                serde_json::to_vec(&endpoint).map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
            let envelope = ResourceEnvelope::from_json(&endpoint_bytes)
                .map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
            if envelope.metadata().owner_ref() != Some(guest_ref)
                || envelope.spec().provider_ref() != Some(&provider_ref)
            {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    guest = %guest_ref.name().as_str(),
                    endpoint = %endpoint_ref.to_canonical_string(),
                    owner = ?envelope
                        .metadata()
                        .owner_ref()
                        .map(ResourceRef::to_canonical_string),
                    provider = ?envelope
                        .spec()
                        .provider_ref()
                        .map(ResourceRef::to_canonical_string),
                    "Cloud Hypervisor endpoint publication refused: endpoint identity mismatch",
                );
                return Err(ResourceRuntimeError::CapabilityUnavailable);
            }
            let endpoint_phase = envelope.status().phase();
            match child_publication_gate(
                endpoint_phase,
                row_status_failure_is_retryable(&endpoint),
            ) {
                ChildPublicationGate::Ready => {}
                ChildPublicationGate::Pending => {
                    tracing::debug!(
                        zone = %self.zone.as_str(),
                        guest = %guest_ref.name().as_str(),
                        endpoint = %endpoint_ref.to_canonical_string(),
                        phase = ?endpoint_phase,
                        retrying = row_status_failure_is_retryable(&endpoint),
                        "Cloud Hypervisor endpoint publication deferred until the plane reports the endpoint Ready",
                    );
                    return Ok(CloudHypervisorEndpointOutcome::Pending);
                }
                ChildPublicationGate::Terminal => {
                    tracing::warn!(
                        zone = %self.zone.as_str(),
                        guest = %guest_ref.name().as_str(),
                        endpoint = %endpoint_ref.to_canonical_string(),
                        phase = ?endpoint_phase,
                        "Cloud Hypervisor endpoint publication refused: terminal endpoint phase",
                    );
                    return Err(ResourceRuntimeError::CapabilityUnavailable);
                }
            }
        }
        Ok(CloudHypervisorEndpointOutcome::Ready)
    }

    async fn reconcile_cloud_hypervisor_setup_volume(
        &self,
        state: &crate::ServerState,
        guest_ref: &ResourceRef,
    ) -> Result<CloudHypervisorSetupVolumeOutcome, ResourceRuntimeError> {
        let volume_ref =
            deterministic_child_ref(guest_ref, ChildRole::SystemVolume).map_err(|_| {
                tracing::warn!("Cloud Hypervisor setup Volume ref derivation failed");
                ResourceRuntimeError::CapabilityUnavailable
            })?;
        // U17: the setup Volume is a converted child committed by the guest's
        // provider controller, so the manager is its authority; the pre-v3
        // store has no row for it. `committed_resource_optional` reads the
        // manager first (the durable store stays the fallback for a row the
        // manager does not serve), an absent row is the honest
        // not-committed-yet answer, and a manager/store read failure stays an
        // error (never reported as absence).
        let Some(volume) = self
            .committed_resource_optional(&volume_ref)
            .await
            .map_err(|_| {
                tracing::warn!("Cloud Hypervisor setup Volume read failed");
                ResourceRuntimeError::StoreReadFailed
            })?
        else {
            return Ok(CloudHypervisorSetupVolumeOutcome::Pending);
        };
        let volume_bytes =
            serde_json::to_vec(&volume).map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
        let envelope = ResourceEnvelope::from_json(&volume_bytes).map_err(|_| {
            tracing::warn!("Cloud Hypervisor setup Volume decode failed");
            ResourceRuntimeError::ResponseInvalid
        })?;
        if envelope.metadata().owner_ref() != Some(guest_ref)
            || envelope.spec().provider_ref()
                != Some(
                    &ResourceRef::parse("Provider/volume-local")
                        .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?,
                )
        {
            tracing::warn!("Cloud Hypervisor setup Volume ownership validation failed");
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
        let resolver = crate::load_bundle_resolver(state).map_err(|_| {
            tracing::warn!("Cloud Hypervisor setup Volume bundle reload failed");
            ResourceRuntimeError::ProviderPathUnavailable
        })?;
        let intent = resolver
            .find_store_view_intent_for_zone(&self.zone, guest_ref.name().as_str())
            .ok_or_else(|| {
                tracing::warn!("Cloud Hypervisor setup Volume store-view intent is unavailable");
                ResourceRuntimeError::ProviderPathUnavailable
            })?;
        let request = d2b_contracts_broker::broker_wire::BrokerRequest::StoreSync(
            d2b_contracts_broker::broker_wire::StoreSyncRequest {
                vm_id: d2b_contracts::types::VmId::new(guest_ref.name().as_str()),
                bundle_closure_ref: d2b_contracts::types::BundleClosureRef::new(
                    intent.intent_id.clone(),
                ),
                generation_token: u32::try_from(intent.generation)
                    .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?,
                tracing_span_id: None,
            },
        );
        match crate::dispatch_broker_request_as(
            state,
            request,
            d2b_contracts_broker::broker_wire::BrokerCallerRole::AdminUid {
                uid: state.daemon_uid,
            },
        ) {
            Ok(d2b_contracts_broker::broker_wire::BrokerResponse::StoreSync(_)) => {}
            Ok(d2b_contracts_broker::broker_wire::BrokerResponse::Error(error)) => {
                tracing::warn!(
                    broker_kind = %error.kind,
                    broker_operation = %error.operation,
                    broker_message = %error.message,
                    broker_action = %error.action,
                    "Cloud Hypervisor setup Volume store sync failed",
                );
                return Err(ResourceRuntimeError::CapabilityUnavailable);
            }
            Ok(_) => {
                tracing::warn!("Cloud Hypervisor setup Volume store sync returned wrong response");
                return Err(ResourceRuntimeError::CapabilityUnavailable);
            }
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "Cloud Hypervisor setup Volume store sync dispatch failed",
                );
                return Err(ResourceRuntimeError::CapabilityUnavailable);
            }
        }
        Ok(CloudHypervisorSetupVolumeOutcome::Ready)
    }

    async fn ensure_cloud_hypervisor_controller_deployment(
        &self,
        provider_ref: &ResourceRef,
        config: &CloudHypervisorConfig,
    ) -> Result<(), ResourceRuntimeError> {
        if !self
            .controller_deployment
            .controller_processes()
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?
            .is_empty()
        {
            return Ok(());
        }
        let provider = self
            .committed_resource_stored(provider_ref, "cloud-hypervisor-controller-deployment")
            .await?;
        let manifest = d2b_provider_runtime_cloud_hypervisor::provider_manifest()
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        let controller_generation = self
            .committed_policy_snapshot()
            .controller_generation
            .unwrap_or_else(|| ControllerGeneration::new(1).expect("generation one"));
        let process_provider_ref = ResourceRef::parse("Provider/system-minijail")
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        let controllers = crate::provider_registry::deploy_target_local_controllers(
            &self.controller_deployment,
            self.zone.clone(),
            provider_ref.clone(),
            &manifest,
            provider.generation,
            provider.generation,
            controller_generation,
            ReconnectGeneration::new(1).map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?,
            provider.revision,
            ResourceRef::parse(&config.controller_execution_ref.to_canonical_string())
                .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?,
            process_provider_ref,
            true,
        )
        .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        if controllers.is_empty() {
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
        Ok(())
    }

    fn cloud_hypervisor_resource_client(
        &self,
    ) -> Result<Arc<CloudHypervisorResourceClient>, ResourceRuntimeError> {
        if let Ok(sessions) = self.controller_sessions.lock()
            && let Some(session) = sessions.values().find(|session| {
                d2b_provider_runtime_cloud_hypervisor::is_provider_ref(
                    session.context.provider_owner_ref(),
                ) && !session.service_task.is_finished()
            })
        {
            if let Some(client) = session.resource_client.as_ref() {
                return Ok(Arc::clone(client));
            }
        }
        self.status_client()
    }

    /// Route a v3 Guest lifecycle request to its controller-owned VMM
    /// Process child. No legacy process-DAG lookup or direct VMM effect is
    /// permitted on this path.
    pub(crate) async fn apply_cloud_hypervisor_lifecycle(
        &self,
        state: Arc<crate::ServerState>,
        guest_ref: &ResourceRef,
        expected_guest_uid: &ResourceUid,
        expected_guest_generation: ResourceGeneration,
        operation: crate::provider_effects::GuestLifecycleOperation,
    ) -> Result<(), ResourceRuntimeError> {
        if guest_ref.resource_type().as_str() != "Guest" {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        let (_, guest_uid, guest_generation, _) = self.guest_lifecycle_identity(guest_ref).await?;
        if &guest_uid != expected_guest_uid || guest_generation != expected_guest_generation {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        self.reconcile_cloud_hypervisor_guests(Arc::clone(&state))
            .await?;
        if matches!(
            operation,
            crate::provider_effects::GuestLifecycleOperation::Start
                | crate::provider_effects::GuestLifecycleOperation::Restart
        ) {
            let (_, _, _, graph) = self.cloud_hypervisor_inputs(guest_ref).await?;
            if !self.cloud_hypervisor_dependencies_ready(&graph).await? {
                return Err(ResourceRuntimeError::CapabilityUnavailable);
            }
        }
        let process_ref = deterministic_child_ref(guest_ref, ChildRole::VmmProcess)
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        match operation {
            crate::provider_effects::GuestLifecycleOperation::Start => {
                self.update_cloud_hypervisor_process_lifecycle(
                    guest_ref,
                    &process_ref,
                    DesiredLifecycle::Running,
                )
                .await?;
            }
            crate::provider_effects::GuestLifecycleOperation::Stop => {
                self.update_cloud_hypervisor_process_lifecycle(
                    guest_ref,
                    &process_ref,
                    DesiredLifecycle::Stopped,
                )
                .await?;
            }
            crate::provider_effects::GuestLifecycleOperation::Restart => {
                self.update_cloud_hypervisor_process_lifecycle(
                    guest_ref,
                    &process_ref,
                    DesiredLifecycle::Stopped,
                )
                .await?;
                self.update_cloud_hypervisor_process_lifecycle(
                    guest_ref,
                    &process_ref,
                    DesiredLifecycle::Running,
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn cloud_hypervisor_dependencies_ready(
        &self,
        graph: &BootstrapGraph,
    ) -> Result<bool, ResourceRuntimeError> {
        for resource_ref in graph
            .devices
            .iter()
            .chain(graph.networks.iter())
            .chain(graph.volumes.iter())
        {
            // U17: the declared dependencies are converted types owned by the
            // manager; the pre-v3 store keeps only the mirror. Absence is the
            // honest "not ready" answer here (the provider controller may not
            // have committed the row yet); a read failure is not.
            let value = match self.committed_resource_optional(resource_ref)
                .await
            {
                Ok(Some(value)) => value,
                Ok(None) => return Ok(false),
                Err(error) => {
                    tracing::debug!(
                        error = %error,
                        resource = %resource_ref.to_canonical_string(),
                        "Cloud Hypervisor dependency read failed",
                    );
                    return Err(ResourceRuntimeError::StoreReadFailed);
                }
            };
            let bytes =
                serde_json::to_vec(&value).map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
            let envelope = ResourceEnvelope::from_json(&bytes)
                .map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
            if envelope.status().phase() != ResourcePhase::Ready {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(crate) async fn wait_cloud_hypervisor_lifecycle(
        &self,
        state: Arc<crate::ServerState>,
        guest_ref: &ResourceRef,
        expected_guest_uid: &ResourceUid,
        expected_guest_generation: ResourceGeneration,
        operation: crate::provider_effects::GuestLifecycleOperation,
    ) -> Result<(), ResourceRuntimeError> {
        let (_, _, config, _) = self.cloud_hypervisor_inputs(guest_ref).await?;
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(u64::from(config.startup_deadline_ms));
        loop {
            let (_, guest_uid, guest_generation, _) =
                self.guest_lifecycle_identity(guest_ref).await?;
            if &guest_uid != expected_guest_uid || guest_generation != expected_guest_generation {
                return Err(ResourceRuntimeError::RequestInvalid);
            }
            self.reconcile_cloud_hypervisor_guests(Arc::clone(&state))
                .await?;
            match crate::resolve_committed_guest_session_target(self, guest_ref).await {
                Ok(target) => {
                    if let Err(error) =
                        crate::connect_guest_component_session_for_guest(&state, &target).await
                    {
                        tracing::debug!(
                            guest = %guest_ref.to_canonical_string(),
                            error = %error,
                            "guest component session connect failed during lifecycle wait",
                        );
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        guest = %guest_ref.to_canonical_string(),
                        error = ?error,
                        "guest session target unavailable during lifecycle wait; skipping connect",
                    );
                }
            }
            let actual = self
                .cloud_hypervisor_lifecycle_state(Arc::clone(&state), guest_ref)
                .await?;
            let lifecycle_satisfied = match operation {
                crate::provider_effects::GuestLifecycleOperation::Start
                | crate::provider_effects::GuestLifecycleOperation::Restart => {
                    actual == crate::provider_effects::GuestLifecycleState::Started
                        && self.cloud_hypervisor_guest_ready(guest_ref).await?
                }
                crate::provider_effects::GuestLifecycleOperation::Stop => {
                    actual == crate::provider_effects::GuestLifecycleState::Stopped
                }
            };
            if lifecycle_satisfied {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(ResourceRuntimeError::CapabilityUnavailable);
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    async fn cloud_hypervisor_guest_ready(
        &self,
        guest_ref: &ResourceRef,
    ) -> Result<bool, ResourceRuntimeError> {
        let guest = self
            .committed_resource_stored(guest_ref, "cloud-hypervisor-guest-ready")
            .await?;
        let envelope = ResourceEnvelope::from_json(&guest.canonical_json)
            .map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
        Ok(envelope.status().phase() == ResourcePhase::Ready)
    }

    async fn update_cloud_hypervisor_process_lifecycle(
        &self,
        guest_ref: &ResourceRef,
        process_ref: &ResourceRef,
        desired: DesiredLifecycle,
    ) -> Result<(), ResourceRuntimeError> {
        let current = self
            .committed_resource_stored(process_ref, "cloud-hypervisor-vmm-lifecycle")
            .await
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        let envelope = ResourceEnvelope::from_json(&current.canonical_json)
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        if envelope.resource_type().as_str() != "Process"
            || envelope.metadata().owner_ref() != Some(guest_ref)
        {
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
        let current_value: Value = serde_json::from_slice(&current.canonical_json)
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        let mut spec = current_value
            .get("spec")
            .and_then(Value::as_object)
            .cloned()
            .ok_or(ResourceRuntimeError::CapabilityUnavailable)?;
        spec.insert(
            "desiredLifecycle".to_owned(),
            serde_json::to_value(desired)
                .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?,
        );
        let payload = replace_public_field(&current_value, "spec", Value::Object(spec))
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        if child_mutation_route(process_ref) == ChildMutationRoute::Manager {
            // U17: the VMM Process is manager-owned, so the lifecycle verb is
            // an exact-fence manager ensure under the guest owner - the same
            // mutation shape the provider controller's UpdateSpec takes.
            let plane = self
                .v3_plane()
                .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
            PlaneChildMutations::new(plane, self.zone.clone(), guest_ref.clone())
                .update(process_ref, &current.uid, current.revision, &payload)
                .await
                .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
            return Ok(());
        }
        let client = self.status_client()?;
        let mut request = wire::UpdateSpecRequest::new();
        request.meta = MessageField::some(public_request_meta("cloud-hypervisor-vmm-lifecycle"));
        let mut mutation = wire::Mutation::new();
        mutation.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_SPEC);
        mutation.target = MessageField::some(ch_identity(
            &current.zone,
            process_ref,
            Some(&current.uid),
            Some(current.generation.get()),
            Some(current.revision.get()),
        ));
        mutation.precondition =
            MessageField::some(ch_exact_precondition(&current.uid, current.revision));
        mutation.resource = MessageField::some(
            ch_resource_body(&current.zone, process_ref, Some(&current.uid), &payload)
                .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?,
        );
        request.mutation = MessageField::some(mutation);
        let response = client.update_spec(request).await;
        if response.error.is_some() {
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
        Ok(())
    }

    pub(crate) async fn cloud_hypervisor_lifecycle_state(
        &self,
        state: Arc<crate::ServerState>,
        guest_ref: &ResourceRef,
    ) -> Result<crate::provider_effects::GuestLifecycleState, ResourceRuntimeError> {
        let process_ref = deterministic_child_ref(guest_ref, ChildRole::VmmProcess)
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        // U17: the VMM Process is manager-owned; the lifecycle probe reads the
        // manager row (per the G5 reader bridge) and keeps its identity.
        let process = self
            .committed_resource_stored(&process_ref, "cloud-hypervisor-vmm-state")
            .await
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        let envelope = ResourceEnvelope::from_json(&process.canonical_json)
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        let spec =
            serde_json::from_slice::<ProcessSpec>(&envelope.spec().base().to_canonical_bytes())
                .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        if envelope.metadata().owner_ref() != Some(guest_ref) {
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
        let provider_ref = envelope
            .spec()
            .provider_ref()
            .cloned()
            .ok_or(ResourceRuntimeError::CapabilityUnavailable)?;
        let providers = state
            .provider_runtime
            .process_providers()
            .ok_or(ResourceRuntimeError::ProviderPathUnavailable)?;
        let descriptor_digest = self
            .guest_setup_descriptors
            .get(guest_ref.name().as_str())
            .and_then(|bytes| {
                GuestSetupDescriptor::from_canonical_bytes(bytes)
                    .ok()
                    .map(|descriptor| descriptor.descriptor_digest().clone())
            });
        let context = crate::process_provider_runtime::ProcessResourceContext::new(
            self.zone.clone(),
            &process.resource_ref,
            &process.uid,
            process.generation,
            process.revision,
            &provider_ref,
            self.committed_policy_snapshot()
                .controller_generation
                .unwrap_or_else(|| ControllerGeneration::new(1).expect("generation one")),
            Some(guest_ref.clone()),
        )
        .with_lifecycle_identity(
            Some(self.bound_zone_uid()?),
            Some(self.committed_policy_snapshot().policy_revision),
            None,
        )
        .with_owner_ref(Some(guest_ref.clone()))
        .with_guest_descriptor_digest(descriptor_digest.as_ref());
        match providers.probe_resource(context, &spec).await {
            Ok(crate::process_provider_runtime::ProviderLiveness::Alive) => {
                Ok(crate::provider_effects::GuestLifecycleState::Started)
            }
            Ok(crate::process_provider_runtime::ProviderLiveness::Exited) => {
                Ok(crate::provider_effects::GuestLifecycleState::Stopped)
            }
            Ok(crate::process_provider_runtime::ProviderLiveness::Unknown) => {
                tracing::debug!(
                    guest = %guest_ref.to_canonical_string(),
                    "Cloud Hypervisor lifecycle probe returned unknown liveness",
                );
                Err(ResourceRuntimeError::CapabilityUnavailable)
            }
            Err(error) => {
                tracing::debug!(
                    guest = %guest_ref.to_canonical_string(),
                    error = ?error,
                    "Cloud Hypervisor lifecycle probe failed",
                );
                Err(ResourceRuntimeError::CapabilityUnavailable)
            }
        }
    }

    pub(crate) async fn list_cloud_hypervisor_guests(
        &self,
    ) -> Result<Vec<ResourceRef>, ResourceRuntimeError> {
        let mut guests = Vec::new();
        // `Guest` is a converted type (U12): the manager rows are the
        // authority and the only source (U14: the durable mirror is gone).
        for resource in self.manager_stored_rows("Guest").await? {
            let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
                .map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
            if envelope
                .spec()
                .provider_ref()
                .is_some_and(d2b_provider_runtime_cloud_hypervisor::is_provider_ref)
            {
                guests.push(resource.resource_ref);
            }
        }
        Ok(guests)
    }

    /// VolumeBinding references that gate one Guest's start: bindings
    /// targeting the Guest AND admitted by their referenced Volume.
    /// Bindings whose Volume cannot be read or parsed stay gating
    /// (fail-closed); pairs admitted by no Volume are excluded so forged
    /// bindings cannot wedge the gate.
    async fn admitted_guest_binding_refs(
        &self,
        bindings: &[StoredResource],
        guest_ref: &ResourceRef,
    ) -> Vec<ResourceRef> {
        let mut kept = Vec::new();
        for binding in bindings {
            if binding.resource_ref.resource_type().as_str()
                != d2b_contracts_resource::v3::VOLUME_BINDING_RESOURCE_TYPE
            {
                continue;
            }
            let Some(spec) =
                crate::binding_child_resource_runtime::parsed_binding_spec(binding)
            else {
                tracing::debug!(
                    binding = %binding.resource_ref.to_canonical_string(),
                    "volume binding spec unparseable; gate kept",
                );
                kept.push(binding.resource_ref.clone());
                continue;
            };
            if spec.execution_ref() != guest_ref {
                continue;
            }
            match self.volume_admits_binding(&spec, &binding.resource_ref).await {
                Ok(true) => kept.push(binding.resource_ref.clone()),
                Ok(false) => {}
                Err(error) => {
                    tracing::debug!(
                        error = ?error,
                        binding = %binding.resource_ref.to_canonical_string(),
                        "volume binding admission check failed; gate kept",
                    );
                    kept.push(binding.resource_ref.clone());
                }
            }
        }
        kept
    }

    /// Whether the referenced Volume admits a binding of this identity:
    /// the binding name must be among the intents the Volume's declared
    /// attachments mint.  Unreadable or unparseable Volumes fail closed
    /// to the caller.
    async fn volume_admits_binding(
        &self,
        spec: &d2b_contracts_resource::v3::volume_binding::VolumeBindingSpec,
        binding_name: &ResourceRef,
    ) -> Result<bool, ResourceRuntimeError> {
        let volume = self
            .committed_resource_stored(spec.volume_ref(), "cloud-hypervisor-binding-admission")
            .await?;
        let volume_value = serde_json::from_slice::<serde_json::Value>(&volume.canonical_json)
            .map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
        let volume_spec = volume_value
            .get("spec")
            .cloned()
            .and_then(|spec| {
                let mut object = spec.as_object()?.clone();
                object.remove("providerRef");
                object.remove("updatePolicy");
                object.remove("provider");
                serde_json::from_value::<VolumeSpec>(serde_json::Value::Object(object)).ok()
            })
            .ok_or(ResourceRuntimeError::ResponseInvalid)?;
        Ok(Self::binding_admitted_by_volume_spec(
            spec,
            binding_name.name().as_str(),
            &volume_spec,
        ))
    }

    /// Whether the referenced Volume admits a binding of this identity:
    /// the binding name must be among the intents the Volume's declared
    /// attachments mint.  Computation failures fail closed to the caller.
    fn binding_admitted_by_volume_spec(
        spec: &d2b_contracts_resource::v3::volume_binding::VolumeBindingSpec,
        binding_name: &str,
        volume_spec: &VolumeSpec,
    ) -> bool {
        d2b_provider_volume_local::desired_binding_intents(
            spec.volume_ref().clone(),
            volume_spec,
            false,
        )
        .map(|intents| {
            intents
                .iter()
                .any(|intent| intent.name().as_str() == binding_name)
        })
        .unwrap_or(true)
    }

    async fn cloud_hypervisor_inputs(
        &self,
        guest_ref: &ResourceRef,
    ) -> Result<
        (
            ResourceRef,
            ResourceRef,
            CloudHypervisorConfig,
            BootstrapGraph,
        ),
        ResourceRuntimeError,
    > {
        // `Guest` and `Provider` are converted types (U12): both reads merge
        // manager-first, the same order `committed_resource_value` applies.
        let guest = self
            .committed_resource_stored(guest_ref, "cloud-hypervisor-guest-inputs")
            .await?;
        let envelope = ResourceEnvelope::from_json(&guest.canonical_json)
            .map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
        let provider_ref = envelope
            .spec()
            .provider_ref()
            .cloned()
            .filter(|reference| d2b_provider_runtime_cloud_hypervisor::is_provider_ref(reference))
            .ok_or(ResourceRuntimeError::CapabilityUnavailable)?;
        let provider = self
            .committed_resource_stored(&provider_ref, "cloud-hypervisor-guest-inputs")
            .await?;
        let guest_spec =
            serde_json::from_slice::<GuestSpec>(&envelope.spec().base().to_canonical_bytes())
                .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        let (provider_spec, _, _, _, _) =
            committed_provider_spec(&self.zone, &provider, &provider_ref)?;
        let config = serde_json::from_slice::<CloudHypervisorConfig>(
            &provider_spec.config().to_canonical_bytes(),
        )
        .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        let mut volumes = Vec::new();
        for value in guest_spec.policy().volume_attachment_defaults() {
            if let Some(reference) = value.get("volumeRef").and_then(|value| match value {
                CanonicalJsonValue::String(value) => ResourceRef::parse(value).ok(),
                _ => None,
            }) {
                volumes.push(reference);
            }
        }
        let mut binding_refs = Vec::new();
        // `VolumeBinding` is a converted type (U14): the manager rows are the
        // only source, and the gate's admission check reads the manager's
        // Volume row for each of them.
        let bindings = self
            .manager_stored_rows(d2b_contracts_resource::v3::VOLUME_BINDING_RESOURCE_TYPE)
            .await?;
        binding_refs.extend(self.admitted_guest_binding_refs(&bindings, guest_ref).await);
        let graph = BootstrapGraph::new(
            guest_spec
                .policy()
                .device_attachments()
                .iter()
                .map(|attachment| attachment.device_ref().clone())
                .collect(),
            guest_spec
                .policy()
                .network_attachments()
                .iter()
                .map(|attachment| attachment.network_ref().clone())
                .collect(),
            volumes,
            binding_refs,
            Vec::new(),
        )
        .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        Ok((
            provider_ref,
            config.controller_execution_ref.clone(),
            config,
            graph,
        ))
    }

    fn build_controller_session_coordinator(
        &self,
    ) -> Result<ControllerSessionCoordinator, ResourceRuntimeError> {
        Ok(ControllerSessionCoordinator {
            zone: self.zone.clone(),
            bundle_resource_types: self.bundle_resource_types.clone(),
            plane_view: Arc::new(Mutex::new(None)),
            api: Arc::clone(&self.v3_api),
            authorizer: Arc::clone(&self.authorizer),
            authorization_state: self.authorization_state.clone(),
            policy_projection: Arc::clone(&self.policy_projection),
            registrar: Arc::clone(&self.registrar),
            assignments: Arc::clone(&self.assignments),
            controller_sessions: Arc::clone(&self.controller_sessions),
            pending_controller_session_clears: Arc::new(Mutex::new(BTreeMap::new())),
            credential_sessions: self.credential_sessions.clone(),
            controller_session_lock: Arc::clone(&self.controller_session_lock),
        })
    }

    fn controller_session_coordinator(&self) -> Arc<ControllerSessionCoordinator> {
        self.controller_session_coordinator
            .lock()
            .ok()
            .and_then(|coordinator| coordinator.clone())
            .expect("controller session coordinator initialized")
    }
}

/// How one child row's observed status gates the endpoint-publication stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildPublicationGate {
    /// The child is realized; the stage may proceed.
    Ready,
    /// The child is coming up (or retrying its own failure); defer.
    Pending,
    /// The child reported a state no retry will move: refuse.
    Terminal,
}

/// Classify one child row's observed phase for the endpoint-publication
/// stage.
///
/// A `Failed` phase whose manager status carries the actor's retryable
/// classification (`row_status_failure_is_retryable`) is a retry in progress
/// - the endpoint actor's bounded realize effect waiting for the VMM evidence
/// fails retryably while the VMM is still coming up - so it defers exactly
/// like `Pending`; the child's own actor owns the retry and the stage sees
/// `Ready` once it converges. Every other unrecognized phase (a terminal
/// `Failed`, `Deleted`, a tombstone) refuses.
fn child_publication_gate(
    phase: ResourcePhase,
    failure_retryable: bool,
) -> ChildPublicationGate {
    match phase {
        ResourcePhase::Ready => ChildPublicationGate::Ready,
        ResourcePhase::Pending | ResourcePhase::Unknown | ResourcePhase::Degraded => {
            ChildPublicationGate::Pending
        }
        ResourcePhase::Failed if failure_retryable => ChildPublicationGate::Pending,
        _ => ChildPublicationGate::Terminal,
    }
}

/// Whether one committed child row's status reports a failure the row's own
/// actor will retry: the manager view stamps a failed actor's closed
/// classification under `status.resource.driverFailure` (the converted plane
/// has no durable status, R11/AE6), so this is where a reader can tell a
/// child's retry in progress from a terminal child failure.
fn row_status_failure_is_retryable(resource: &Value) -> bool {
    resource
        .pointer("/status/resource/driverFailure/retryable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn schedule_controller_session_reconcile(
    task_slot: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    wake: Arc<tokio::sync::Notify>,
    shutdown: Arc<AtomicBool>,
    coordinator: Arc<ControllerSessionCoordinator>,
    providers: Arc<crate::process_provider_runtime::ProductionProcessProviders>,
) -> Result<(), ResourceRuntimeError> {
    if shutdown.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut slot = task_slot
        .lock()
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    if shutdown.load(Ordering::Acquire) {
        return Ok(());
    }
    if slot.as_ref().is_some_and(|task| !task.is_finished()) {
        wake.notify_one();
        return Ok(());
    }
    let wake_for_task = Arc::clone(&wake);
    *slot = Some(tokio::spawn(async move {
        loop {
            if shutdown.load(Ordering::Acquire) {
                break;
            }
            let notified = wake_for_task.notified();
            let mut notified = std::pin::pin!(notified);
            notified.as_mut().enable();
            if let Err(error) = coordinator
                .reconcile_controller_sessions(Arc::clone(&providers), true)
                .await
            {
                tracing::warn!(
                    error = %error,
                    "external Provider controller session reconciliation degraded",
                );
                tokio::time::sleep(COMPONENT_SESSION_RETRY_BACKOFF).await;
                continue;
            }
            notified.await;
        }
    }));
    Ok(())
}

impl ControllerSessionCoordinator {
    /// The zone plane's manager view.
    ///
    /// U14: the manager is the only authority, so a read before the
    /// composition publishes the plane is a retryable failure - never an
    /// absent row and never a fallback to a second plane.
    fn plane_view(&self) -> Result<Arc<dyn ControllerPlaneView>, ResourceRuntimeError> {
        self.plane_handle()
            .ok_or(ResourceRuntimeError::CapabilityUnavailable)
    }

    /// The manager-backed Resource API service a live controller session
    /// serves (the runtime's slot, filled when the Zone's plane activates).
    fn api(&self) -> Result<Arc<ResourceService<ZoneApiBackend>>, ResourceRuntimeError> {
        self.api
            .lock()
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?
            .clone()
            .ok_or(ResourceRuntimeError::ResourceApiBindFailed)
    }



    fn controller_provider_identity_error_is_global(error: &ResourceRuntimeError) -> bool {
        matches!(
            error,
            ResourceRuntimeError::StoreReadFailed
                | ResourceRuntimeError::AuthenticationUnavailable
                | ResourceRuntimeError::AuthorizationUnavailable
                | ResourceRuntimeError::PolicyUnavailable
        )
    }

    fn controller_session_evidence_error_is_context_local(
        error: &ResourceRuntimeError,
    ) -> bool {
        let kind = match error {
            ResourceRuntimeError::ResourceGetFailed(kind)
            | ResourceRuntimeError::ResourceStatusUpdateFailed(kind) => *kind,
            _ => return false,
        };
        matches!(
            kind,
            ResourceErrorKind::ResourceConflict
                | ResourceErrorKind::RevisionExpired
                | ResourceErrorKind::Backpressure
                | ResourceErrorKind::Timeout
                | ResourceErrorKind::Cancelled
                | ResourceErrorKind::ResourcePlaneUnavailable
        )
    }

    /// Adopt the zone plane's manager view seam (G5).
    fn attach_plane_view(&self, plane: Arc<dyn ControllerPlaneView>) {
        if let Ok(mut slot) = self.plane_view.lock() {
            *slot = Some(plane);
        }
    }

    /// The adopted plane-view seam, when the composition published one.
    fn plane_handle(&self) -> Option<Arc<dyn ControllerPlaneView>> {
        self.plane_view.lock().ok().and_then(|slot| slot.clone())
    }

    /// The committed policy inputs for this Zone, read from the manager (U14:
    /// the manager is the only authority).
    pub(crate) async fn committed_policy_resources(
        &self,
    ) -> Result<Vec<StoredResource>, ResourceRuntimeError> {
        let plane = self.plane_view()?;
        committed_policy_resources(plane.as_ref()).await
    }

    /// The committed `Provider` identities for the requested refs, read from
    /// the manager.
    pub(crate) async fn committed_controller_provider_identities(
        &self,
        provider_refs: BTreeSet<ResourceRef>,
    ) -> Result<BTreeMap<ResourceRef, (ResourceUid, ResourceGeneration)>, ResourceRuntimeError> {
        let plane = self.plane_view()?;
        committed_controller_provider_identities(&self.zone, plane.as_ref(), provider_refs).await
    }
    /// The policy snapshot the Zone's committed manager rows imply (U14: the
    /// durable store metadata that carried it is gone).
    async fn committed_policy_snapshot(&self) -> Result<PolicySnapshot, ResourceRuntimeError> {
        let resources = self.committed_policy_resources().await?;
        policy_snapshot_for_rows(&resources)
    }

    /// The manager's row for one controller Process key. `Ok(None)` means the
    /// manager does not hold the row; a manager failure is reported, never
    /// folded into absence.
    async fn controller_plane_row(
        &self,
        process_ref: &ResourceRef,
    ) -> Result<Option<ResourceView>, ResourceRuntimeError> {
        let plane = self.plane_view()?;
        plane.process_view(process_ref).await.map_err(|error| {
            tracing::debug!(
                process = %process_ref.to_canonical_string(),
                error = %error,
                "controller plane view unavailable",
            );
            ResourceRuntimeError::StoreReadFailed
        })
    }

    async fn persist_controller_session_evidence_with_retry(
        &self,
        context: &crate::process_provider_runtime::ControllerBootstrapContext,
        session_generation: Option<ReconnectGeneration>,
    ) -> Result<(), ResourceRuntimeError> {
        for attempt in 0..2 {
            match self
                .persist_controller_session_evidence(context, session_generation)
                .await
            {
                Err(error)
                    if attempt == 0
                        && Self::controller_session_evidence_error_is_context_local(&error) =>
                {
                    tracing::debug!(
                        process = %context.process_ref(),
                        "controller-session evidence conflicted; rereading before retry",
                    );
                }
                result => return result,
            }
        }
        unreachable!("controller-session evidence retry must return a result");
    }

    async fn persist_controller_session_evidence(
        &self,
        context: &crate::process_provider_runtime::ControllerBootstrapContext,
        session_generation: Option<ReconnectGeneration>,
    ) -> Result<(), ResourceRuntimeError> {
        let Some(view) = self.controller_plane_row(context.process_ref()).await? else {
            // The manager does not hold the row: there is no status copy to
            // write or clear. Clearing evidence for an absent row is a no-op;
            // asserting a session for one is an unbound identity.
            if session_generation.is_none() {
                return Ok(());
            }
            return Err(ResourceRuntimeError::IdentityUnbound);
        };
        // A manager-served row's status is never durable (R11/AE6), so there is
        // nothing to write or clear here - the evidence is the live admitted
        // session, which the Provider's dependency observation reads on every
        // pass. The identity check keeps the old semantics: evidence for a row
        // this context does not describe still refuses.
        controller_session_evidence_identity_check(
            controller_plane_resource_matches(context, &view),
            session_generation.is_none(),
        )?;
        tracing::debug!(
            process = %context.process_ref(),
            "controller-session evidence for a manager-served row is the live session",
        );
        Ok(())
    }

    fn queue_controller_session_clear(
        &self,
        context: &crate::process_provider_runtime::ControllerBootstrapContext,
    ) -> Result<(), ResourceRuntimeError> {
        self.pending_controller_session_clears
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .insert(context.process_ref().clone(), context.clone());
        Ok(())
    }

    fn remove_queued_controller_session_clear(
        &self,
        process_ref: &ResourceRef,
        context: &crate::process_provider_runtime::ControllerBootstrapContext,
    ) -> Result<(), ResourceRuntimeError> {
        let mut pending = self
            .pending_controller_session_clears
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
        if pending
            .get(process_ref)
            .is_some_and(|queued| queued == context)
        {
            pending.remove(process_ref);
        }
        Ok(())
    }

    async fn retry_queued_controller_session_clears(
        &self,
    ) -> Result<(), ResourceRuntimeError> {
        let pending = self
            .pending_controller_session_clears
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .iter()
            .map(|(process_ref, context)| (process_ref.clone(), context.clone()))
            .collect::<Vec<_>>();
        for (process_ref, context) in pending {
            let session_state = self
                .controller_sessions
                .lock()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
                .get(&process_ref)
                .map(|session| session.service_task.is_finished());
            match session_state {
                Some(false) => {
                    self.remove_queued_controller_session_clear(&process_ref, &context)?;
                    continue;
                }
                Some(true) => continue,
                None => {}
            }
            match self
                .persist_controller_session_evidence_with_retry(&context, None)
                .await
            {
                Ok(()) => {
                    self.remove_queued_controller_session_clear(&process_ref, &context)?;
                }
                Err(error) if Self::controller_session_clear_error_is_stale_identity(&error) => {
                    self.remove_queued_controller_session_clear(&process_ref, &context)?;
                }
                Err(error)
                    if Self::controller_session_evidence_error_is_context_local(&error) =>
                {
                    tracing::warn!(
                        process = %process_ref,
                        error = %error,
                        "controller-session evidence clear remains deferred after transport teardown",
                    );
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn revoke_controller_assignments(&self, binding: &ControllerSessionBinding) {
        if d2b_provider_runtime_cloud_hypervisor::is_provider_ref(binding.provider_ref()) {
            self.assignments
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .revoke_session_for(binding);
        }
    }

    fn controller_session_clear_error_is_stale_identity(error: &ResourceRuntimeError) -> bool {
        matches!(
            error,
            ResourceRuntimeError::IdentityUnbound
                | ResourceRuntimeError::ResourceGetFailed(ResourceErrorKind::ResourceNotFound)
                | ResourceRuntimeError::ResourceStatusUpdateFailed(
                    ResourceErrorKind::ResourceNotFound
                )
        )
    }

    fn isolate_controller_session_clear_error(
        result: Result<(), ResourceRuntimeError>,
    ) -> Result<(), ResourceRuntimeError> {
        match result {
            Ok(()) => Ok(()),
            Err(error)
                if Self::controller_session_clear_error_is_stale_identity(&error)
                    || Self::controller_session_evidence_error_is_context_local(&error) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    async fn clear_controller_session_for_reconcile(
        &self,
        process_ref: &ResourceRef,
        expected: Option<&crate::process_provider_runtime::ControllerBootstrapContext>,
    ) -> Result<(), ResourceRuntimeError> {
        Self::isolate_controller_session_clear_error(
            self.remove_controller_session(process_ref, expected).await,
        )
    }

    async fn fence(
        &self,
        providers: &crate::process_provider_runtime::ProductionProcessProviders,
    ) -> Result<(), ResourceRuntimeError> {
        self.retry_queued_controller_session_clears().await?;
        let bootstrap_contexts = providers.controller_bootstrap_contexts(&self.zone);
        let bootstrap_contexts = bootstrap_contexts
            .into_iter()
            .map(|context| (context.process_ref().clone(), context))
            .collect::<BTreeMap<_, _>>();
        let stale_sessions = self
            .controller_sessions
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .iter()
            .filter(|(process_ref, session)| {
                crate::process_provider_runtime::controller_session_needs_fence(
                    bootstrap_contexts.get(*process_ref),
                    &session.context,
                    session.service_task.is_finished(),
                )
            })
            .map(|(process_ref, session)| (process_ref.clone(), session.context.clone()))
            .collect::<Vec<_>>();
        for (process_ref, context) in stale_sessions {
            providers.fail_controller_bootstrap(&context);
            self.clear_controller_session_for_reconcile(&process_ref, Some(&context))
                .await?;
        }

        let active_sessions = self
            .controller_sessions
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        for context in providers
            .controller_bootstrap_establishing_contexts(&self.zone)
            .into_iter()
            .filter(|context| !active_sessions.contains(context.process_ref()))
        {
            providers.fail_controller_bootstrap(&context);
        }

        let contexts = providers.controller_bootstrap_contexts(&self.zone);
        if contexts.is_empty() {
            return Ok(());
        }
        let current_controller_generation = self
            .committed_policy_snapshot()
            .await?
            .controller_generation;
        let provider_refs = contexts
            .iter()
            .map(|context| context.provider_owner_ref().clone())
            .collect::<BTreeSet<_>>();
        for provider_ref in provider_refs {
            match self
                .committed_controller_provider_identities(BTreeSet::from([provider_ref.clone()]))
                .await
            {
                Ok(identities) => {
                    for context in contexts.iter().filter(|context| {
                        context.provider_owner_ref() == &provider_ref
                            && identities.get(&provider_ref).is_none_or(
                                |(provider_uid, provider_generation)| {
                                    provider_uid != context.provider_uid()
                                        || *provider_generation != context.provider_generation()
                                },
                            )
                    }) {
                        providers.fail_controller_bootstrap(context);
                        self.clear_controller_session_for_reconcile(
                            context.process_ref(),
                            Some(context),
                        )
                        .await?;
                    }
                }
                Err(error) => {
                    for context in contexts
                        .iter()
                        .filter(|context| context.provider_owner_ref() == &provider_ref)
                    {
                        providers.fail_controller_bootstrap(context);
                        self.clear_controller_session_for_reconcile(
                            context.process_ref(),
                            Some(context),
                        )
                        .await?;
                    }
                    tracing::warn!(
                        provider = %provider_ref,
                        error = %error,
                        "external Provider controller identity projection failed",
                    );
                    if Self::controller_provider_identity_error_is_global(&error) {
                        return Err(error);
                    }
                }
            }
        }
        let contexts = providers.controller_bootstrap_contexts(&self.zone);
        for context in contexts {
            if controller_generation_is_stale(
                current_controller_generation,
                context.controller_generation(),
            ) {
                providers.fail_controller_bootstrap(&context);
                self.clear_controller_session_for_reconcile(
                    context.process_ref(),
                    Some(&context),
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn fence_process_resources(
        &self,
        providers: &crate::process_provider_runtime::ProductionProcessProviders,
        resources: &[StoredResource],
    ) -> Result<(), ResourceRuntimeError> {
        let _session_guard = self.controller_session_lock.lock().await;
        let sessions = {
            let sessions = self
                .controller_sessions
                .lock()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
            sessions
                .iter()
                .map(|(process_ref, session)| (process_ref.clone(), session.context.clone()))
                .collect::<Vec<_>>()
        };
        let stale_sessions = controller_session_resource_fences(sessions, resources);
        for (process_ref, context) in stale_sessions {
            // G5: a fence candidate the durable list cannot see may be a
            // manager-served controller row; the zone manager is its
            // authority, and only a row it also does not hold (or does not
            // match) is genuinely stale.
            if let Some(view) = self.controller_plane_row(&process_ref).await?
                && controller_plane_resource_matches(&context, &view)
            {
                continue;
            }
            providers.fail_controller_bootstrap(&context);
            self.clear_controller_session_for_reconcile(&process_ref, Some(&context))
                .await?;
        }
        Ok(())
    }

    async fn reconcile_controller_sessions(
        &self,
        providers: Arc<crate::process_provider_runtime::ProductionProcessProviders>,
        establish: bool,
    ) -> Result<(), ResourceRuntimeError> {
        let _session_guard = self.controller_session_lock.lock().await;
        self.reconcile_controller_sessions_locked(providers, establish)
            .await
    }

    async fn reconcile_controller_sessions_locked(
        &self,
        providers: Arc<crate::process_provider_runtime::ProductionProcessProviders>,
        establish: bool,
    ) -> Result<(), ResourceRuntimeError> {
        self.fence(&providers).await?;
        if !establish {
            self.refresh_controller_policy(&providers).await?;
            self.reconcile_controller_assignments(&providers).await?;
            return Ok(());
        }
        let bootstrap_contexts = providers.controller_bootstrap_contexts(&self.zone);
        let active_processes = self
            .controller_sessions
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        for context in &bootstrap_contexts {
            if !active_processes.contains(context.process_ref()) {
                match self
                    .persist_controller_session_evidence_with_retry(context, None)
                    .await
                {
                    Ok(()) => {}
                    Err(error)
                        if Self::controller_session_clear_error_is_stale_identity(&error) =>
                    {
                        providers.fail_controller_bootstrap(context);
                        tracing::debug!(
                            process = %context.process_ref(),
                            "controller session evidence identity stale; bootstrap failed",
                        );
                        continue;
                    }
                    Err(error)
                        if Self::controller_session_evidence_error_is_context_local(&error) =>
                    {
                        tracing::warn!(
                            process = %context.process_ref(),
                            "controller-session evidence clear conflicted; retaining context for retry",
                        );
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        let provider_refs = bootstrap_contexts
            .iter()
            .map(|context| context.provider_owner_ref().clone())
            .collect::<BTreeSet<_>>();
        let mut provider_identities = BTreeMap::new();
        for provider_ref in provider_refs {
            match self
                .committed_controller_provider_identities(BTreeSet::from([provider_ref.clone()]))
                .await
            {
                Ok(identities) => {
                    let mismatched = bootstrap_contexts
                        .iter()
                        .filter(|context| {
                            context.provider_owner_ref() == &provider_ref
                                && identities.get(&provider_ref).is_none_or(
                                    |(provider_uid, provider_generation)| {
                                        provider_uid != context.provider_uid()
                                            || *provider_generation != context.provider_generation()
                                    },
                                )
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    for context in mismatched {
                        providers.fail_controller_bootstrap(&context);
                        self.clear_controller_session_for_reconcile(
                            context.process_ref(),
                            Some(&context),
                        )
                        .await?;
                    }
                    provider_identities.extend(identities);
                }
                Err(error) => {
                    for context in bootstrap_contexts
                        .iter()
                        .filter(|context| context.provider_owner_ref() == &provider_ref)
                    {
                        providers.fail_controller_bootstrap(context);
                        self.clear_controller_session_for_reconcile(
                            context.process_ref(),
                            Some(context),
                        )
                        .await?;
                    }
                    tracing::warn!(
                        provider = %provider_ref,
                        error = %error,
                        "external Provider controller identity projection failed",
                    );
                    if Self::controller_provider_identity_error_is_global(&error) {
                        return Err(error);
                    }
                }
            }
        }
        let surviving_contexts = bootstrap_contexts
            .into_iter()
            .filter(|context| providers.has_controller_bootstrap(context.process_ref(), context))
            .collect::<Vec<_>>();
        let provider_subjects = surviving_contexts
            .iter()
            .filter_map(|context| {
                provider_identities
                    .get(context.provider_owner_ref())
                    .map(|(provider_uid, _)| d2b_resource_api::authz::BoundSubject {
                        subject_ref: context.provider_owner_ref().clone(),
                        subject_uid: provider_uid.clone(),
                    })
            })
            .collect::<BTreeSet<_>>();
        let policy_resources = self.committed_policy_resources().await?;
        let (snapshot, policy_inputs) = self.policy_snapshot_for_install(&policy_resources)?;
        let (policy, state) =
            match d2bd_runtime::resource_runtime_support::compile_committed_policy_with_subjects(
                &self.zone,
                snapshot,
                ZoneRevision::new(snapshot.policy_revision),
                &self.bundle_resource_types,
                &policy_resources,
                provider_subjects.iter().cloned(),
            ) {
                Ok(policy) => policy,
                Err(error) => {
                    for context in surviving_contexts {
                        providers.fail_controller_bootstrap(&context);
                        self.clear_controller_session_for_reconcile(
                            context.process_ref(),
                            Some(&context),
                        )
                        .await?;
                    }
                    tracing::warn!(
                        error = %error,
                        "external Provider controller policy projection failed",
                    );
                    return Err(error);
                }
            };
        if let Err(error) = self
            .policy_projection
            .install(policy, state, provider_subjects, policy_inputs)
        {
            for context in surviving_contexts {
                providers.fail_controller_bootstrap(&context);
                self.clear_controller_session_for_reconcile(
                    context.process_ref(),
                    Some(&context),
                )
                .await?;
            }
            return Err(error);
        }

        for context in surviving_contexts {
            let process_ref = context.process_ref().clone();
            let existing = self
                .controller_sessions
                .lock()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
                .get(&process_ref)
                .map(|session| {
                    (
                        session.context.clone(),
                        session.service_task.is_finished(),
                        session.binding.session_generation(),
                    )
                });
            if let Some((context, finished, session_generation)) = existing {
                if finished {
                    tracing::warn!(
                        process = %process_ref,
                        session_generation = session_generation.get(),
                        "controller session service task finished; tearing down session",
                    );
                    providers.fail_controller_bootstrap(&context);
                    self.clear_controller_session_for_reconcile(
                        &process_ref,
                        Some(&context),
                    )
                    .await?;
                    continue;
                }
                match self
                    .persist_controller_session_evidence_with_retry(
                        &context,
                        Some(session_generation),
                    )
                    .await
                {
                    Ok(()) => {}
                    Err(error)
                        if Self::controller_session_evidence_error_is_context_local(&error) =>
                    {
                        tracing::warn!(
                            process = %process_ref,
                            "controller-session evidence conflicted; retaining session for retry",
                        );
                        continue;
                    }
                    Err(error)
                        if Self::controller_session_clear_error_is_stale_identity(&error) =>
                    {
                        providers.fail_controller_bootstrap(&context);
                        self.clear_controller_session_for_reconcile(
                            &process_ref,
                            Some(&context),
                        )
                        .await?;
                        continue;
                    }
                    Err(error) => return Err(error),
                }
                if providers.has_controller_bootstrap(&process_ref, &context) {
                    continue;
                }
                self.clear_controller_session_for_reconcile(
                    &process_ref,
                    Some(&context),
                )
                .await?;
            }

            if !providers.has_controller_bootstrap(&process_ref, &context) {
                continue;
            }
            if !providers.controller_bootstrap_ready(&self.zone, &process_ref) {
                continue;
            }
            let mut registrar = match self.registrar.lock() {
                Ok(mut registrar) => match registrar.take() {
                    Some(registrar) => registrar,
                    None => {
                        tracing::debug!(
                            process = %context.process_ref(),
                            "controller session registrar unavailable; bootstrap deferred",
                        );
                        continue;
                    }
                },
                Err(_) => {
                    return Err(ResourceRuntimeError::AuthenticationUnavailable);
                }
            };
            let Some(endpoint) =
                providers.begin_controller_bootstrap_if_matches(&self.zone, &context)
            else {
                *self
                    .registrar
                    .lock()
                    .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? =
                    Some(registrar);
                tracing::debug!(
                    process = %context.process_ref(),
                    "controller bootstrap begin refused; endpoint re-armed for retry",
                );
                continue;
            };
            let context = endpoint.context().clone();
            // A transient bootstrap failure re-arms the endpoint (the
            // establish attempt dups the pre-armed socket) instead of
            // orphaning the retrying controller.
            let establish_started = std::time::Instant::now();
            let setup = self
                .establish_controller_session(
                    &providers,
                    &endpoint,
                    &mut registrar,
                )
                .await;
            tracing::warn!(
                provider = %context.provider_owner_ref().to_canonical_string(),
                elapsed_ms = establish_started.elapsed().as_millis(),
                ok = setup.is_ok(),
                "controller session establish timing",
            );
            let mut registrar = Some(registrar);
            let restored = match self.registrar.lock() {
                Ok(mut slot) => {
                    *slot = registrar.take();
                    true
                }
                Err(_) => false,
            };
            let mut setup = Some(setup);
            if !restored {
                if let Some(Ok((
                    ingress,
                    driver,
                    _resource_client,
                    service_task,
                    _session_generation,
                    _route,
                    backend_lease,
                ))) = setup.take()
                {
                    if let Some(backend_lease) = backend_lease {
                        backend_lease.cancel();
                    }
                    let _ = driver
                        .close(
                            d2b_contracts_zone_session::v3::component_session::CloseReason::RoleMismatch,
                            d2b_contracts_zone_session::v3::component_session::Remediation::ReplaceGeneration,
                        )
                        .await;
                    service_task.abort();
                    let _ = service_task.await;
                    drop(ingress);
                }
                // Re-arm instead of dropping: the controller retries its
                // bootstrap send for as long as it lives, and one failed
                // receive must not orphan it.
                providers.rearm_controller_bootstrap(endpoint);
                return Err(ResourceRuntimeError::AuthenticationUnavailable);
            }
            match setup.expect("controller setup result present") {
                Ok((
                    ingress,
                    driver,
                    resource_client,
                    service_task,
                    session_generation,
                    route,
                    backend_lease,
                )) => {
                    let current = match self
                        .controller_context_is_current(&providers, &context)
                        .await
                    {
                        Ok(current) => current,
                        Err(ResourceRuntimeError::IdentityUnbound) => false,
                        Err(error) => {
                            let _ = driver
                                .close(
                                    d2b_contracts_zone_session::v3::component_session::CloseReason::RoleMismatch,
                                    d2b_contracts_zone_session::v3::component_session::Remediation::ReplaceGeneration,
                                )
                                .await;
                            service_task.abort();
                            let _ = service_task.await;
                            providers.fail_controller_bootstrap(&context);
                            self.revoke_controller_ingress(ingress).await?;
                            return Err(error);
                        }
                    };
                    if !current || !providers.activate_controller_bootstrap(&context) {
                        let _ = driver
                            .close(
                                d2b_contracts_zone_session::v3::component_session::CloseReason::RoleMismatch,
                                d2b_contracts_zone_session::v3::component_session::Remediation::ReplaceGeneration,
                            )
                            .await;
                        service_task.abort();
                        let _ = service_task.await;
                        providers.fail_controller_bootstrap(&context);
                        self.revoke_controller_ingress(ingress).await?;
                        if current {
                            tracing::warn!(
                                provider = %context.provider_owner_ref().to_canonical_string(),
                                process = %context.process_ref().to_canonical_string(),
                                "controller session establish rejected: bootstrap activation refused",
                            );
                        } else {
                            tracing::warn!(
                                provider = %context.provider_owner_ref().to_canonical_string(),
                                process = %context.process_ref().to_canonical_string(),
                                "controller session establish rejected: context not current",
                            );
                        }
                        continue;
                    }
                    let context_for_cleanup = context.clone();
                    let binding = controller_session_binding(&context, session_generation)?;
                    let credential_session = if is_credential_provider_ref(context.provider_owner_ref()) {
                        Some(Arc::new(
                            ComponentCredentialSession::new(route.clone(), Arc::new(driver.clone()))
                                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
                        ) as Arc<dyn CredentialSession>)
                    } else {
                        None
                    };
                    if let Some(session) = credential_session.as_ref() {
                        self.credential_sessions.register(
                            context.provider_owner_ref().clone(),
                            session_generation,
                            Arc::clone(session),
                        )
                        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
                    }
                    let mut session = Some(ControllerSession {
                        context: context.clone(),
                        binding,
                        ingress,
                        driver: driver.clone(),
                        _backend_lease: backend_lease,
                        resource_client,
                        service_task,
                        assignments: BTreeMap::new(),
                        assignment_stream_open: false,
                        assignments_revoked: false,
                        transport_closed: false,
                        ingress_revoked: false,
                    });
                    let inserted = match self.controller_sessions.lock() {
                        Ok(mut sessions) => {
                            if sessions.contains_key(&process_ref) {
                                false
                            } else {
                                sessions.insert(
                                    process_ref.clone(),
                                    session.take().expect("controller session present"),
                                );
                                true
                            }
                        }
                        Err(_) => {
                            tracing::debug!(
                                process = %context.process_ref(),
                                "controller session registry lock poisoned; admitted session torn down",
                            );
                            false
                        }
                    };
                    if inserted {
                        if let Err(error) = self
                            .persist_controller_session_evidence_with_retry(
                                &context,
                                Some(session_generation),
                            )
                            .await
                        {
                            if Self::controller_session_evidence_error_is_context_local(&error) {
                                tracing::warn!(
                                    process = %process_ref,
                                    "controller-session evidence conflicted; retaining admitted session for retry",
                                );
                                continue;
                            }
                            providers.fail_controller_bootstrap(&context);
                            match error {
                                error
                                    if Self::controller_session_clear_error_is_stale_identity(
                                        &error,
                                    ) =>
                                {
                                    self.clear_controller_session_for_reconcile(
                                        &process_ref,
                                        Some(&context),
                                    )
                                    .await?;
                                    continue;
                                }
                                error => {
                                    self.clear_controller_session_for_reconcile(
                                        &process_ref,
                                        Some(&context),
                                    )
                                    .await?;
                                    return Err(error);
                                }
                            }
                        }
                        tracing::info!(
                            zone = %self.zone.as_str(),
                            provider = %context.provider_owner_ref(),
                            process = %context.process_ref(),
                            session_generation = session_generation.get(),
                            "external Provider controller ResourceV3 session live",
                        );
                    }
                    if !inserted {
                        if credential_session.is_some() {
                            self.credential_sessions.remove(
                                context.provider_owner_ref(),
                                session_generation,
                            );
                        }
                        providers.fail_controller_bootstrap(&context_for_cleanup);
                        if let Some(mut session) = session {
                            session.cancel_backend_lease();
                            let _ = session
                                .driver
                                .close(
                                    d2b_contracts_zone_session::v3::component_session::CloseReason::RoleMismatch,
                                    d2b_contracts_zone_session::v3::component_session::Remediation::ReplaceGeneration,
                                )
                                .await;
                            session.service_task.abort();
                            let _ = session.service_task.await;
                            self.revoke_controller_ingress(session.ingress).await?;
                        }
                        continue;
                    }
                }
                Err(error) => {
                    // The controller retries its bootstrap send forever;
                    // re-arm so the next reconcile pass answers it instead
                    // of orphaning the controller.
                    providers.rearm_controller_bootstrap(endpoint);
                    tracing::warn!(
                        error = %error,
                        "external Provider controller ResourceV3 session setup failed",
                    );
                }
            }
        }
        self.reconcile_controller_assignments(&providers).await?;
        self.refresh_controller_policy(&providers).await?;
        Ok(())
    }

    async fn reconcile_controller_assignments(
        &self,
        providers: &crate::process_provider_runtime::ProductionProcessProviders,
    ) -> Result<(), ResourceRuntimeError> {
        let sessions = self
            .controller_sessions
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .iter()
            .map(|(_, session)| (session.context.clone(), session.binding.clone()))
            .collect::<Vec<_>>();
        let mut first_error = None;
        for (context, binding) in sessions {
            if let Err(error) = self
                .reconcile_controller_assignments_for_session(&context, &binding)
                .await
            {
                if let Err(error) = self
                    .handle_controller_assignment_refresh_error(providers, &context, error)
                    .await
                {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn controller_context_is_current(
        &self,
        providers: &crate::process_provider_runtime::ProductionProcessProviders,
        context: &crate::process_provider_runtime::ControllerBootstrapContext,
    ) -> Result<bool, ResourceRuntimeError> {
        if !providers.has_controller_bootstrap(context.process_ref(), context) {
            return Ok(false);
        }
        let metadata = self.committed_policy_snapshot().await?;
        if controller_generation_is_stale(
            metadata.controller_generation,
            context.controller_generation(),
        ) {
            return Ok(false);
        }
        let identities = match self
            .committed_controller_provider_identities(BTreeSet::from([
                context.provider_owner_ref().clone(),
            ]))
            .await
        {
            Ok(identities) => identities,
            Err(error) if !Self::controller_provider_identity_error_is_global(&error) => {
                tracing::warn!(
                    provider = %context.provider_owner_ref(),
                    error = %error,
                    "external Provider controller identity is not admissible",
                );
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let Some((provider_uid, provider_generation)) =
            identities.get(context.provider_owner_ref())
        else {
            return Ok(false);
        };
        if provider_uid != context.provider_uid()
            || *provider_generation != context.provider_generation()
        {
            return Ok(false);
        }
        // The manager is the only authority (U14): the controller Process row
        // lives in the manager, and a row it does not hold is not current.
        let Some(view) = self.controller_plane_row(context.process_ref()).await? else {
            return Ok(false);
        };
        Ok(controller_plane_resource_matches(context, &view))
    }

    async fn handle_controller_assignment_refresh_error(
        &self,
        providers: &crate::process_provider_runtime::ProductionProcessProviders,
        context: &crate::process_provider_runtime::ControllerBootstrapContext,
        error: ControllerAssignmentRefreshError,
    ) -> Result<(), ResourceRuntimeError> {
        match controller_assignment_refresh_action(context, error) {
            ControllerAssignmentRefreshAction::Retryable { .. } => {
                tracing::warn!("external Provider controller assignment reconciliation will retry");
                Ok(())
            }
            ControllerAssignmentRefreshAction::Failed { context, error } => {
                providers.fail_controller_bootstrap(context);
                self.clear_controller_session_for_reconcile(
                    context.process_ref(),
                    Some(context),
                )
                .await?;
                Err(error)
            }
        }
    }

    async fn reconcile_controller_assignments_for_session(
        &self,
        context: &crate::process_provider_runtime::ControllerBootstrapContext,
        binding: &ControllerSessionBinding,
    ) -> Result<(), ControllerAssignmentRefreshError> {
        if !d2b_provider_runtime_cloud_hypervisor::is_provider_ref(context.provider_owner_ref()) {
            return Ok(());
        }
        let manifest =
            d2b_provider_runtime_cloud_hypervisor::provider_manifest().map_err(|_| {
                ControllerAssignmentRefreshError::Failed(
                    ResourceRuntimeError::AuthenticationUnavailable,
                )
            })?;
        let role_ref =
            ResourceRef::parse(d2b_provider_runtime_cloud_hypervisor::CONTROLLER_ROLE_REF)
                .map_err(|_| {
                    ControllerAssignmentRefreshError::Failed(
                        ResourceRuntimeError::AuthenticationUnavailable,
                    )
                })?;
        let role = ControllerRoleContract::from_signed_manifest(
            context.provider_owner_ref().clone(),
            role_ref,
            &manifest,
        )
        .map_err(|_| {
            ControllerAssignmentRefreshError::Failed(
                ResourceRuntimeError::AuthenticationUnavailable,
            )
        })?;
        let expected_target = AssignmentTarget::Execution {
            kind: PlacementTargetKind::Host,
            reference: context.execution_ref().clone(),
        };
        if binding.target() != &expected_target {
            return Err(ControllerAssignmentRefreshError::Failed(
                ResourceRuntimeError::AuthenticationUnavailable,
            ));
        }
        let resources = self
            .list_assignment_resources(&role, context.provider_owner_ref())
            .await?;
        let driver = self.ensure_controller_assignment_stream(binding).await?;
        let mut resources_by_uid = BTreeMap::new();
        for (index, resource) in resources.iter().enumerate() {
            if resources_by_uid
                .insert(resource.metadata().uid().clone(), index)
                .is_some()
            {
                return Err(ControllerAssignmentRefreshError::Failed(
                    ResourceRuntimeError::AuthorizationUnavailable,
                ));
            }
        }
        let (retained, stale) = {
            let sessions = self.controller_sessions.lock().map_err(|_| {
                ControllerAssignmentRefreshError::Failed(
                    ResourceRuntimeError::AuthenticationUnavailable,
                )
            })?;
            let Some(session) = sessions.get(binding.session_owner()).filter(|session| {
                &session.binding == binding && !session.service_task.is_finished()
            }) else {
                return Err(ControllerAssignmentRefreshError::Retryable);
            };
            let mut retained = BTreeSet::new();
            let mut stale = Vec::new();
            for (resource_uid, lease) in &session.assignments {
                if resources_by_uid
                    .get(resource_uid)
                    .and_then(|index| resources.get(*index))
                    .is_some_and(|resource| {
                        lease.phase() == AssignmentPhase::Assigned
                            && assignment_resource_matches(
                                lease.resource_ref(),
                                lease.identity().resource_uid(),
                                lease.resource_generation(),
                                lease.identity().resource_revision(),
                                resource,
                            )
                    })
                {
                    retained.insert(resource_uid.clone());
                } else {
                    stale.push((
                        resource_uid.clone(),
                        lease.identity().clone(),
                        lease.provider_ref().clone(),
                    ));
                }
            }
            (retained, stale)
        };

        for (resource_uid, identity, provider_ref) in stale {
            self.revoke_recorded_assignment(binding, &resource_uid, &identity, &provider_ref)
                .await?;
        }

        let mut degraded = false;
        let stream = StreamId::new(CONTROLLER_ASSIGNMENT_STREAM_ID).map_err(|_| {
            ControllerAssignmentRefreshError::Failed(
                ResourceRuntimeError::AuthenticationUnavailable,
            )
        })?;
        for resource in &resources {
            if retained.contains(resource.metadata().uid()) {
                continue;
            }
            if !self.controller_session_is_live(binding) {
                return Err(ControllerAssignmentRefreshError::Retryable);
            }
            let request = AssignmentRequest::new(
                resource,
                &role,
                context.provider_generation(),
                context.controller_generation(),
                binding.session_generation(),
                true,
            )
            .with_expected_target(binding.target().clone())
            .with_session_owner(binding.session_owner().clone());
            let lease = match admit_assignment_or_skip(&self.assignments, request) {
                Ok(Some(lease)) => lease,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(
                        error = ?error,
                        "external Provider controller assignment admission failed",
                    );
                    degraded = true;
                    continue;
                }
            };
            let encoded = match lease.assignment_grant().encode() {
                Ok(encoded) => encoded,
                Err(error) => {
                    tracing::debug!(
                        error = ?error,
                        resource = %lease.identity().resource_uid().as_str(),
                        "controller assignment grant encode failed; revoking unrecorded lease",
                    );
                    self.revoke_unrecorded_assignment(&lease, false).await;
                    degraded = true;
                    continue;
                }
            };
            match send_controller_assignment_frame(&driver, stream, encoded, || {
                self.revoke_unrecorded_assignment_local(&lease);
            })
            .await
            {
                Ok(()) => {}
                Err(ControllerAssignmentRefreshError::Retryable) => {
                    self.mark_controller_assignment_stream_closed(binding)?;
                    return Err(ControllerAssignmentRefreshError::Retryable);
                }
                Err(error) => return Err(error),
            }
            if !self.controller_session_is_live(binding) {
                self.revoke_unrecorded_assignment(&lease, true).await;
                return Err(ControllerAssignmentRefreshError::Retryable);
            }
            if let Err(lease) = self.record_controller_assignment(binding, context, lease) {
                self.revoke_unrecorded_assignment(&lease, true).await;
                tracing::debug!(
                    resource = %lease.identity().resource_uid().as_str(),
                    "controller assignment record failed; revoking unrecorded lease",
                );
                degraded = true;
            }
        }
        if degraded {
            Err(ControllerAssignmentRefreshError::Retryable)
        } else {
            Ok(())
        }
    }

    async fn ensure_controller_assignment_stream(
        &self,
        binding: &ControllerSessionBinding,
    ) -> Result<SessionDriverHandle, ControllerAssignmentRefreshError> {
        let (driver, stream_open) = {
            let sessions = self.controller_sessions.lock().map_err(|_| {
                ControllerAssignmentRefreshError::Failed(
                    ResourceRuntimeError::AuthenticationUnavailable,
                )
            })?;
            let Some(session) = sessions.get(binding.session_owner()).filter(|session| {
                &session.binding == binding && !session.service_task.is_finished()
            }) else {
                return Err(ControllerAssignmentRefreshError::Retryable);
            };
            (session.driver.clone(), session.assignment_stream_open)
        };
        if stream_open {
            return Ok(driver);
        }
        let stream = StreamId::new(CONTROLLER_ASSIGNMENT_STREAM_ID).map_err(|_| {
            ControllerAssignmentRefreshError::Failed(
                ResourceRuntimeError::AuthenticationUnavailable,
            )
        })?;
        driver
            .open_named_stream(
                stream,
                CONTROLLER_ASSIGNMENT_STREAM_CREDIT,
                CONTROLLER_ASSIGNMENT_STREAM_CREDIT,
            )
            .await
            .map_err(|_| {
                ControllerAssignmentRefreshError::Failed(
                    ResourceRuntimeError::AuthenticationUnavailable,
                )
            })?;
        let mut sessions = self.controller_sessions.lock().map_err(|_| {
            ControllerAssignmentRefreshError::Failed(
                ResourceRuntimeError::AuthenticationUnavailable,
            )
        })?;
        let Some(session) = sessions
            .get_mut(binding.session_owner())
            .filter(|session| &session.binding == binding && !session.service_task.is_finished())
        else {
            return Err(ControllerAssignmentRefreshError::Retryable);
        };
        session.assignment_stream_open = true;
        Ok(driver)
    }

    fn mark_controller_assignment_stream_closed(
        &self,
        binding: &ControllerSessionBinding,
    ) -> Result<(), ControllerAssignmentRefreshError> {
        let mut sessions = self.controller_sessions.lock().map_err(|_| {
            ControllerAssignmentRefreshError::Failed(
                ResourceRuntimeError::AuthenticationUnavailable,
            )
        })?;
        let Some(session) = sessions
            .get_mut(binding.session_owner())
            .filter(|session| &session.binding == binding)
        else {
            return Err(ControllerAssignmentRefreshError::Failed(
                ResourceRuntimeError::AuthenticationUnavailable,
            ));
        };
        session.assignment_stream_open = false;
        Ok(())
    }

    fn controller_session_is_live(&self, binding: &ControllerSessionBinding) -> bool {
        self.controller_sessions
            .lock()
            .ok()
            .and_then(|sessions| {
                sessions.get(binding.session_owner()).map(|session| {
                    controller_session_matches(
                        &session.binding,
                        binding,
                        session.service_task.is_finished(),
                    )
                })
            })
            .unwrap_or(false)
    }

    fn record_controller_assignment(
        &self,
        binding: &ControllerSessionBinding,
        context: &crate::process_provider_runtime::ControllerBootstrapContext,
        lease: ResourceClientLease,
    ) -> Result<(), ResourceClientLease> {
        let mut sessions = match self.controller_sessions.lock() {
            Ok(sessions) => sessions,
            Err(_) => return Err(lease),
        };
        let Some(session) = sessions.get_mut(binding.session_owner()).filter(|session| {
            &session.context == context
                && controller_session_matches(
                    &session.binding,
                    binding,
                    session.service_task.is_finished(),
                )
        }) else {
            return Err(lease);
        };
        let resource_uid = lease.identity().resource_uid().clone();
        if session.assignments.contains_key(&resource_uid) {
            return Err(lease);
        }
        session.assignments.insert(resource_uid, lease);
        Ok(())
    }

    fn revoke_unrecorded_assignment_local(&self, lease: &ResourceClientLease) {
        self.assignments
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .revoke_assignment(lease.identity());
    }

    async fn revoke_unrecorded_assignment(&self, lease: &ResourceClientLease, notify: bool) {
        self.revoke_unrecorded_assignment_local(lease);
        if notify
            && let Ok(bytes) =
                ControllerAssignmentGrant::encode_revocation(lease.provider_ref(), lease.identity())
            && let Ok(stream) = StreamId::new(CONTROLLER_ASSIGNMENT_STREAM_ID)
            && let Some(driver) = self.controller_sessions.lock().ok().and_then(|sessions| {
                sessions
                    .get(lease.identity().session_owner())
                    .map(|session| session.driver.clone())
            })
        {
            if let Err(error) = driver.send_named_stream(stream, bytes).await {
                tracing::debug!(
                    error = %error,
                    "unrecorded assignment revocation delivery failed",
                );
                if driver.reset_named_stream(stream).await.is_ok() {
                    let _ = self.mark_controller_assignment_stream_closed(
                        lease.identity().session_binding(),
                    );
                }
            }
        }
    }

    async fn revoke_recorded_assignment(
        &self,
        binding: &ControllerSessionBinding,
        resource_uid: &ResourceUid,
        identity: &AssignmentIdentity,
        provider_ref: &ResourceRef,
    ) -> Result<(), ControllerAssignmentRefreshError> {
        self.assignments
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .revoke_assignment(identity);
        let (driver, bytes) = self
            .controller_sessions
            .lock()
            .ok()
            .and_then(|sessions| {
                let session = sessions
                    .get(binding.session_owner())
                    .filter(|session| &session.binding == binding)?;
                let lease = session.assignments.get(resource_uid)?;
                if lease.identity() != identity {
                    return None;
                }
                let bytes =
                    ControllerAssignmentGrant::encode_revocation(provider_ref, identity).ok()?;
                Some((session.driver.clone(), bytes))
            })
            .ok_or(ControllerAssignmentRefreshError::Retryable)?;
        let stream = StreamId::new(CONTROLLER_ASSIGNMENT_STREAM_ID).map_err(|_| {
            ControllerAssignmentRefreshError::Failed(
                ResourceRuntimeError::AuthenticationUnavailable,
            )
        })?;
        if driver.send_named_stream(stream, bytes).await.is_err() {
            reset_controller_assignment_stream(&driver, stream).await?;
            self.mark_controller_assignment_stream_closed(binding)?;
            return Err(ControllerAssignmentRefreshError::Retryable);
        }
        if let Ok(mut sessions) = self.controller_sessions.lock()
            && let Some(session) = sessions
                .get_mut(binding.session_owner())
                .filter(|session| &session.binding == binding)
            && session
                .assignments
                .get(resource_uid)
                .is_some_and(|lease| lease.identity() == identity)
        {
            session.assignments.remove(resource_uid);
        }
        Ok(())
    }

    async fn list_assignment_resources(
        &self,
        role: &ControllerRoleContract,
        provider_ref: &ResourceRef,
    ) -> Result<Vec<ResourceEnvelope>, ControllerAssignmentRefreshError> {
        let plane = self.plane_view().map_err(|_| {
            ControllerAssignmentRefreshError::Failed(ResourceRuntimeError::StoreReadFailed)
        })?;
        let mut resources = Vec::new();
        let mut resource_uids = BTreeSet::new();
        for resource_type in role.resource_types() {
            let rows =
                bridge_manager_rows(plane.as_ref(), resource_type.as_str())
                    .await
                    .map_err(|_| {
                        ControllerAssignmentRefreshError::Failed(
                            ResourceRuntimeError::StoreReadFailed,
                        )
                    })?;
            for stored in rows {
                let Some(envelope) = validate_assignment_row(&stored, &self.zone, provider_ref)?
                else {
                    continue;
                };
                if resources.len() >= d2b_core_controller::controller_assignment::MAX_ASSIGNMENTS
                    || !resource_uids.insert(envelope.metadata().uid().clone())
                {
                    return Err(ControllerAssignmentRefreshError::Failed(
                        ResourceRuntimeError::AuthorizationUnavailable,
                    ));
                }
                resources.push(envelope);
            }
        }
        Ok(resources)
    }

    async fn remove_controller_session(
        &self,
        process_ref: &ResourceRef,
        expected: Option<&crate::process_provider_runtime::ControllerBootstrapContext>,
    ) -> Result<(), ResourceRuntimeError> {
        let (context, mut session) = {
            let mut sessions = self
                .controller_sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(session) = sessions.get(process_ref).filter(|session| {
                expected.is_none_or(|expected| &session.context == expected)
            }) else {
                return Ok(());
            };
            let context = session.context.clone();
            let session = sessions
                .remove(process_ref)
                .expect("matching controller session remains in the map");
            (context, session)
        };

        if !session.ingress_revoked {
            if let Err(error) = self
                .revoke_controller_ingress_in_place(&mut session.ingress)
                .await
            {
                let mut sessions = self
                    .controller_sessions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if sessions.get(process_ref).is_none() {
                    sessions.insert(process_ref.clone(), session);
                }
                return Err(error);
            }
            session.ingress_revoked = true;
        }

        self.credential_sessions.remove(
            session.binding.provider_ref(),
            session.binding.session_generation(),
        );

        if !session.assignments_revoked {
            self.revoke_controller_assignments(&session.binding);
            send_controller_assignment_revocations(&session.driver, &session.assignments).await;
            session.assignments_revoked = true;
        }

        if !session.transport_closed {
            session.cancel_backend_lease();
            let _ = session
                .driver
                .close(
                    d2b_contracts_zone_session::v3::component_session::CloseReason::RoleMismatch,
                    d2b_contracts_zone_session::v3::component_session::Remediation::ReplaceGeneration,
                )
                .await;
            session.service_task.abort();
            let _ = (&mut session.service_task).await;
            session.transport_closed = true;
        }

        if let Err(error) = self
            .persist_controller_session_evidence_with_retry(&context, None)
            .await
        {
            if Self::controller_session_clear_error_is_stale_identity(&error) {
                return Ok(());
            }
            if Self::controller_session_evidence_error_is_context_local(&error) {
                self.queue_controller_session_clear(&context)?;
                tracing::warn!(
                    process = %process_ref,
                    error = %error,
                    "controller-session transport teardown completed; durable evidence clear deferred",
                );
                return Ok(());
            }
            let mut sessions = self
                .controller_sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if sessions.get(process_ref).is_none() {
                sessions.insert(process_ref.clone(), session);
            }
            return Err(error);
        }
        Ok(())
    }

    async fn revoke_controller_ingress(
        &self,
        mut ingress: BusIngress,
    ) -> Result<(), ResourceRuntimeError> {
        self.revoke_controller_ingress_in_place(&mut ingress).await
    }

    async fn revoke_controller_ingress_in_place(
        &self,
        ingress: &mut BusIngress,
    ) -> Result<(), ResourceRuntimeError> {
        let mut registrar = self
            .registrar
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .take()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        let result = registrar.revoke_in_place(ingress).await;
        let restored = self
            .registrar
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)
            .map(|mut slot| {
                *slot = Some(registrar);
            })
            .is_ok();
        if !restored {
            return Err(ResourceRuntimeError::AuthenticationUnavailable);
        }
        result.map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)
    }

    #[allow(clippy::type_complexity)]
    async fn establish_controller_session(
        &self,
        providers: &crate::process_provider_runtime::ProductionProcessProviders,
        endpoint: &crate::process_provider_runtime::ControllerBootstrapEndpoint,
        registrar: &mut ZoneRegistrar,
    ) -> Result<
        (
            BusIngress,
            SessionDriverHandle,
            Option<Arc<ResourceApiClient<ZoneApiBackend, UnavailableUpgradeDispatcher>>>,
            tokio::task::JoinHandle<Result<(), SessionServerError>>,
            ReconnectGeneration,
            d2b_session::AuthenticatedSessionRouteBinding,
            Option<Arc<dyn crate::process_provider_runtime::GuestCredentialBackendLease>>,
        ),
        ResourceRuntimeError,
    > {
        let authentication_error = |stage: &'static str| {
            tracing::warn!(
                zone = %self.zone.as_str(),
                stage,
                "external Provider controller authentication failed",
            );
            ResourceRuntimeError::AuthenticationUnavailable
        };
        // Capture the underlying handshake cause alongside the stage; a
        // bare stage cannot distinguish load flakes from real breakage.
        let authentication_error_caused = |stage: &'static str, error: &dyn core::fmt::Debug| {
            tracing::warn!(
                zone = %self.zone.as_str(),
                stage,
                error = ?error,
                "external Provider controller authentication failed",
            );
            ResourceRuntimeError::AuthenticationUnavailable
        };
        let context = endpoint.context().clone();
        let (delivery_key_handoff, backend_lease) = endpoint.handles();
        let daemon_socket = endpoint
            .daemon_socket()
            .map_err(|_| authentication_error("bootstrap-socket"))?;
        let (resource_socket, credentials) = receive_controller_bootstrap(&daemon_socket)
            .await
            .map_err(|error| authentication_error_caused("bootstrap-receive", &error))?;
        let peer_pid = credentials.pid().as_raw_nonzero().get();
        if !providers
            .controller_peer_matches(&context, peer_pid)
            .map_err(|error| authentication_error_caused("peer-process-observation", &error))?
        {
            return Err(authentication_error("peer-process-mismatch"));
        }
        let verified_peer = VerifiedUnixPeer::verify_inherited_seqpacket(&resource_socket)
            .map_err(|_| authentication_error("resource-peer-verification"))?;
        if verified_peer.credentials() != credentials {
            return Err(authentication_error("resource-peer-mismatch"));
        }

        // `Provider` is served by the v3 plane (U12): the committed identity
        // is the manager-first bridged one - the same authority the session
        // fence compares against. The durable pre-v3 mirror carries an
        // unrelated uid and must never answer here.
        let mut provider_identities = self
            .committed_controller_provider_identities(BTreeSet::from([
                context.provider_owner_ref().clone(),
            ]))
            .await
            .map_err(|_| authentication_error("provider-resource-load"))?;
        let (provider_uid, provider_generation) = provider_identities
            .remove(context.provider_owner_ref())
            .ok_or_else(|| authentication_error("provider-resource-identity"))?;
        if &provider_uid != context.provider_uid()
            || provider_generation != context.provider_generation()
        {
            return Err(authentication_error("provider-context-mismatch"));
        }
        let zone_ref = ResourceRef::parse(&format!("Zone/{}", self.zone.as_str()))
            .map_err(|_| authentication_error("zone-reference"))?;
        let credential_session = is_credential_provider_ref(context.provider_owner_ref());
        let committed_subject = CommittedControllerProcessSubjectInput {
            provider_ref: context.provider_owner_ref().clone(),
            provider_uid,
            process_ref: context.process_ref().clone(),
            zone_ref,
            execution_ref: context.execution_ref().clone(),
            provider_generation,
            controller_generation: context.controller_generation(),
        };
        if credential_session {
            registrar
                .install_committed_controller_process_subject_for_service(
                    &verified_peer,
                    committed_subject,
                    d2b_contracts_zone_session::v3::component_session::ServicePackage::CredentialV3,
                )
        } else {
            registrar.install_committed_controller_process_subject(
                &verified_peer,
                committed_subject,
            )
        }
        .map_err(|_| authentication_error("controller-subject-install"))?;
        let policy = if credential_session {
            credential_provider_endpoint_policy()
        } else {
            controller_resource_endpoint_policy()
        };
        let acceptor = registrar
            .component_session_acceptor(policy.clone(), verified_peer)
            .map_err(|_| authentication_error("session-acceptor"))?;
        let evidence = TransportEvidence::new(
            EvidenceClass::UnixPeer,
            crate::interaction_composition::policy_channel_binding_digest(&policy)
                .ok_or_else(|| authentication_error("binding-digest"))?,
        );
        let transport = unix_transport(resource_socket, &policy)?;
        let mut responder = SessionEngine::establish_responder(
            transport,
            policy,
            HandshakeCredentials::Nn,
            std::time::Instant::now(),
        )
        .await
        .map_err(|_| authentication_error("session-handshake"))?;
        if credential_session {
            // Named-stream registration is local to this endpoint, so the
            // Provider can send on a stream before this side has registered
            // it. Register the Provider session streams on the engine before
            // admission turns it into a driver: once the driver task exists it
            // may route an inbound fragment first, and a fragment for an
            // unregistered stream fails the whole session as an invalid
            // channel.
            preregister_provider_session_streams(&mut responder)
                .map_err(|_| authentication_error("provider-session-stream-open"))?;
        }
        let candidate = acceptor
            .admit(responder, evidence, 1)
            .await
            .map_err(|_| authentication_error("session-admission"))?;
        let session_generation = candidate.route_binding().reconnect_generation();
        let route = candidate.route_binding();
        let authorization_state = self
            .authorization_state
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
            .ok_or_else(|| authentication_error("authorization-state"))?;
        let (ingress, driver) = registrar
            .register_component_service_session(candidate)
            .await
            .map_err(|_| authentication_error("service-registration"))?;
        let (resource_client, service_task) = if credential_session {
            let metadata = ProviderSessionMetadata::from_route(&route)
                .and_then(|metadata| metadata.encode())
                .map_err(|_| authentication_error("provider-session-bootstrap"))?;
            let stream = StreamId::new(PROVIDER_BOOTSTRAP_STREAM_ID)
                .map_err(|_| authentication_error("provider-session-stream"))?;
            driver
                .send_named_stream(stream, metadata)
                .await
                .map_err(|_| authentication_error("provider-session-bootstrap-send"))?;
            driver
                .close_named_stream(stream)
                .await
                .map_err(|_| authentication_error("provider-session-bootstrap-close"))?;
            let delivery_key_handoff = delivery_key_handoff
                .ok_or_else(|| authentication_error("provider-delivery-key-handoff"))?;
            if let Some(backend_lease) = backend_lease.as_ref() {
                if backend_lease
                    .bind_route(&route, None, Some(credentials))
                    .is_err()
                {
                    let _ = registrar.revoke(ingress).await;
                    let _ = driver
                        .close(
                            d2b_contracts_zone_session::v3::component_session::CloseReason::RoleMismatch,
                            d2b_contracts_zone_session::v3::component_session::Remediation::ReplaceGeneration,
                        )
                        .await;
                    return Err(authentication_error("provider-backend-route-bind"));
                }
            } else {
                let _ = registrar.revoke(ingress).await;
                let _ = driver
                    .close(
                        d2b_contracts_zone_session::v3::component_session::CloseReason::RoleMismatch,
                        d2b_contracts_zone_session::v3::component_session::Remediation::ReplaceGeneration,
                    )
                    .await;
                return Err(authentication_error("provider-backend-responder-missing"));
            }
            let key_stream = StreamId::new(PROVIDER_DELIVERY_KEY_STREAM_ID)
                .map_err(|_| authentication_error("provider-delivery-key-stream"))?;
            driver
                .send_named_stream(
                    key_stream,
                    delivery_key_handoff
                        .encode_for_route(&route)
                        .map_err(|_| authentication_error("provider-delivery-key-handoff-encode"))?
                        .to_vec(),
                )
                .await
                .map_err(|_| authentication_error("provider-delivery-key-handoff-send"))?;
            driver
                .close_named_stream(key_stream)
                .await
                .map_err(|_| authentication_error("provider-delivery-key-stream-close"))?;
            receive_provider_ready(&driver)
                .await
                .map_err(|_| authentication_error("provider-session-ready"))?;
            let monitor = driver.clone();
            (
                None,
                tokio::spawn(async move {
                    loop {
                        match monitor.receive_control().await {
                            Ok(d2b_session::SessionEvent::Close(_)) => return Ok(()),
                            Err(_) => {
                                tracing::debug!(
                                    "provider session monitor stopped: control receive failed",
                                );
                                return Ok(());
                            }
                            Ok(_) => {}
                        }
                    }
                }),
            )
        } else {
            let subject = self
                .authorizer
                .issue_authenticated_subject(route.context().clone(), authorization_state)
                .map_err(|_| authentication_error("authenticated-subject"))?;
            let service = Arc::new(
                ResourceBusAdapter::bind_component_session(self.api()?, subject)
                    .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?,
            );
            let resource_client = Some(Arc::new(service.client()));
            let services = Arc::clone(&service).ttrpc_services();
            let service_task = tokio::spawn(d2b_session::serve_ttrpc_services(
                Arc::new(driver.clone()),
                services,
            ));
            (resource_client, service_task)
        };
        tokio::task::yield_now().await;
        if service_task.is_finished() {
            service_task.abort();
            let _ = service_task.await;
            let _ = registrar.revoke(ingress).await;
            return Err(authentication_error("service-task"));
        }
        Ok((
            ingress,
            driver,
            resource_client,
            service_task,
            session_generation,
            route,
            backend_lease,
        ))
    }

    /// The snapshot and input digest one controller-policy compile/install
    /// runs under.
    ///
    /// `Provider` controller grants are compiled from the same committed
    /// policy rows as the Zone projection, so the install must obey the same
    /// monotone-revision rule (`PolicyProjection::next_policy_revision`) the
    /// Zone bus fences every install at.
    fn policy_snapshot_for_install(
        &self,
        resources: &[StoredResource],
    ) -> Result<(PolicySnapshot, [u8; 32]), ResourceRuntimeError> {
        let policy_inputs = policy_inputs_digest(resources);
        let mut snapshot = policy_snapshot_for_rows(resources)?;
        snapshot.policy_revision = self.policy_projection.next_policy_revision(
            snapshot.policy_revision,
            self.policy_projection.installed_policy_inputs() != Some(policy_inputs),
        );
        Ok((snapshot, policy_inputs))
    }

    async fn refresh_controller_policy(
        &self,
        providers: &crate::process_provider_runtime::ProductionProcessProviders,
    ) -> Result<(), ResourceRuntimeError> {
        let plane = self.plane_view()?;
        let provider_subjects = match load_controller_policy_subjects(
            &self.zone,
            plane.as_ref(),
            Some(providers),
            &self.controller_sessions,
        )
        .await
        {
            Ok(subjects) => subjects,
            Err(error) => {
                return Err(error);
            }
        };
        let policy_resources = self.committed_policy_resources().await?;
        let (snapshot, policy_inputs) = self.policy_snapshot_for_install(&policy_resources)?;
        let (policy, state) =
            d2bd_runtime::resource_runtime_support::compile_committed_policy_with_subjects(
                &self.zone,
                snapshot,
                ZoneRevision::new(snapshot.policy_revision),
                &self.bundle_resource_types,
                &policy_resources,
                provider_subjects.iter().cloned(),
            )
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
        self.policy_projection
            .install(policy, state, provider_subjects, policy_inputs)?;
        Ok(())
    }
}

impl ZoneResourceRuntime {

    /// Ensure the per-Zone external-controller session machinery is live and
    /// refresh its fences.
    ///
    /// The generic Process/EphemeralProcess runner this method used to host is
    /// retired: converted `Process` rows are served exclusively by the new
    /// plane's Process driver. External Provider controller sessions are still
    /// established on the old plane until the core-controller conversion, so
    /// their wake registration, establishment loop, and resource fences stay
    /// driven from here.
    pub(crate) async fn reconcile_controller_sessions(
        &self,
        state: Arc<crate::ServerState>,
    ) -> Result<(), ResourceRuntimeError> {
        if !self.readiness.resource_api_ready {
            return Ok(());
        }
        let providers = state
            .provider_runtime
            .process_providers()
            .ok_or(ResourceRuntimeError::ProviderPathUnavailable)?;
        let coordinator = self.controller_session_coordinator();
        let wake_task_slot = Arc::clone(&self.controller_session_reconcile_task);
        let wake = Arc::clone(&self.controller_session_reconcile_wake);
        let wake_shutdown = Arc::clone(&self.controller_session_reconcile_shutdown);
        let wake_coordinator = Arc::downgrade(&coordinator);
        let wake_providers = Arc::downgrade(&providers);
        {
            let _session_guard = self.controller_session_lock.lock().await;
            if self.system_core_rebind_pending.load(Ordering::Acquire) {
                return Err(ResourceRuntimeError::AuthenticationUnavailable);
            }
            *self
                .controller_session_providers
                .lock()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? =
                Some(Arc::clone(&providers));
            providers
                .set_controller_session_waker(
                    self.zone.clone(),
                    Arc::new(move || {
                        let coordinator = wake_coordinator
                            .upgrade()
                            .ok_or_else(|| "controller-session-coordinator-dropped".to_owned())?;
                        let providers = wake_providers
                            .upgrade()
                            .ok_or_else(|| "process-providers-dropped".to_owned())?;
                        schedule_controller_session_reconcile(
                            Arc::clone(&wake_task_slot),
                            Arc::clone(&wake),
                            Arc::clone(&wake_shutdown),
                            coordinator,
                            providers,
                        )
                        .map_err(|error| format!("{error:?}"))
                    }),
                )
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
            coordinator
                .reconcile_controller_sessions_locked(Arc::clone(&providers), false)
                .await?;
        }
        let _guard = self.controller_reconcile_lock.lock().await;
        let resources = self.committed_process_rows().await?;
        coordinator
            .fence_process_resources(&providers, &resources)
            .await?;
        schedule_controller_session_reconcile(
            Arc::clone(&self.controller_session_reconcile_task),
            Arc::clone(&self.controller_session_reconcile_wake),
            Arc::clone(&self.controller_session_reconcile_shutdown),
            coordinator.clone(),
            Arc::clone(&providers),
        )?;
        Ok(())
    }

    /// Return the current daemon-owned AudioBinding projections.
    pub(crate) fn audio_binding_statuses(
        &self,
    ) -> Result<Vec<AudioBindingRuntimeStatus>, ResourceRuntimeError> {
        self.audio_runtime
            .lock()
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?
            .as_ref()
            .map(AudioResourceRuntime::statuses)
            .ok_or(ResourceRuntimeError::CapabilityUnavailable)
    }

    /// Reserve a Host-global claim in the Zone's authority ledger. The ledger
    /// is process-local since U14: the reservation is fenced for this
    /// daemon's lifetime, and a restart re-derives owners through the
    /// drivers' probe/adopt path rather than replaying a durable checkpoint.
    pub async fn reserve_authority(
        &self,
        operation_id: impl Into<String>,
        request: AuthorityRequest,
    ) -> Result<
        AuthorityReservation,
        d2b_core_controller::authority::AuthorityReservationError<
            d2b_core_controller::authority::AuthorityError,
        >,
    > {
        if !self.authority_index.lock().await.is_ready_for_readiness() {
            return Err(
                d2b_core_controller::authority::AuthorityReservationError::Effect(
                    d2b_core_controller::authority::AuthorityError::StartupRehydrationRequired,
                ),
            );
        }
        AuthorityReservation::reserve_durable(
            Arc::clone(&self.authority_index),
            Arc::clone(&self.authority_ledger) as Arc<dyn AuthorityPersistence>,
            operation_id,
            request,
        )
        .await
    }

    /// Reserve an external physical-NIC claim through the same durable
    /// startup-barrier owner as generic Host-global claims.
    ///
    /// The ledger re-proves every claim against the manager rows and the
    /// trusted live external-NIC inventory; the store-era bundle inventory
    /// that used to answer the latter is gone (U14), so until a live
    /// inventory is installed an `ExternalNic` claim is refused rather than
    /// assumed.
    pub async fn reserve_external_nic(
        &self,
        operation_id: impl Into<String>,
        request: ExternalNicClaimRequest,
    ) -> Result<
        ExternalNicReservation,
        d2b_core_controller::authority::AuthorityReservationError<
            d2b_core_controller::authority::AuthorityError,
        >,
    > {
        if !self.authority_index.lock().await.is_ready_for_readiness() {
            return Err(
                d2b_core_controller::authority::AuthorityReservationError::Effect(
                    d2b_core_controller::authority::AuthorityError::StartupRehydrationRequired,
                ),
            );
        }
        ExternalNicReservation::reserve_durable(
            Arc::clone(&self.authority_index),
            Arc::clone(&self.authority_ledger) as Arc<dyn AuthorityPersistence>,
            operation_id,
            request,
        )
        .await
    }

    /// Resolve one recovered authority after the authoritative effect is
    /// observed closed. Persistence must complete before the holder is
    /// removed from the in-memory index.
    pub async fn resolve_recovered_authority_closed(
        &self,
        operation_id: &str,
    ) -> Result<(), d2b_core_controller::authority_persistence::AuthorityPersistenceError> {
        self.authority_recovery
            .resolve_observed_closed(operation_id)
            .await
    }

    /// Mark one recovered operation observed and adopted without releasing
    /// its authority holder.
    pub async fn resolve_recovered_authority_adopted(
        &self,
        operation_id: &str,
    ) -> Result<(), d2b_core_controller::authority_persistence::AuthorityPersistenceError> {
        self.authority_recovery
            .resolve_observed_and_adopted(operation_id)
            .await
    }

    /// Quarantine one recovered operation when observation is ambiguous.
    pub async fn quarantine_recovered_authority(
        &self,
        operation_id: &str,
    ) -> Result<(), d2b_core_controller::authority_persistence::AuthorityPersistenceError> {
        self.authority_recovery.quarantine(operation_id).await
    }

    /// Return the first startup gate that prevents publication.
    pub fn readiness_error(&self) -> Option<ResourceRuntimeError> {
        if !self.policy_installed {
            return Some(ResourceRuntimeError::PolicyUnavailable);
        }
        if !self.readiness.resource_api_ready {
            return Some(ResourceRuntimeError::PolicyUnavailable);
        }
        if !self.controller_endpoint_registered {
            return Some(ResourceRuntimeError::ControllerEndpointUnavailable);
        }
        if !self.readiness.local_session_ready {
            return Some(ResourceRuntimeError::AuthenticationUnavailable);
        }
        if !self.watch_admitted {
            return Some(ResourceRuntimeError::WatchUnavailable);
        }
        if !self.readiness.authority_ready
            || self
                .authority_index
                .try_lock()
                .map(|index| !index.is_ready_for_readiness())
                .unwrap_or(true)
        {
            return Some(ResourceRuntimeError::AuthorityUnavailable);
        }
        if !self.readiness.provider_path_ready {
            return Some(ResourceRuntimeError::ProviderPathUnavailable);
        }
        if matches!(self.interaction_state, InteractionState::Refused) {
            return Some(ResourceRuntimeError::InteractionConfigurationUnavailable);
        }
        if !matches!(self.core_stage().ok(), Some(StartupStage::Ready)) {
            return Some(ResourceRuntimeError::HandlerNotReady);
        }
        if self
            .zone_status
            .try_lock()
            .map(|status| !status.mandatory_handlers_ready())
            .unwrap_or(true)
        {
            return Some(ResourceRuntimeError::HandlerNotReady);
        }
        None
    }

    /// Require a runtime that is safe to publish through the public plane.
    pub fn require_ready(&self) -> Result<(), ResourceRuntimeError> {
        if let Some(error) = self.readiness_error() {
            return Err(error);
        }
        if !matches!(self.core_stage()?, StartupStage::Ready) {
            return Err(ResourceRuntimeError::CoreStartupFailed);
        }
        Ok(())
    }

    /// Refuse an unbound direct read.
    ///
    /// The old helper used a fixed internal provider session. A
    /// caller that does not carry an authenticated session must not reach the
    /// Resource API through this compatibility method.
    pub async fn get(
        &self,
        _target: ResourceRef,
        _operation_id: &str,
    ) -> Result<Value, ResourceRuntimeError> {
        Err(ResourceRuntimeError::IdentityUnbound)
    }

    /// Refuse an unbound direct list.
    pub async fn list(
        &self,
        _resource_type: ResourceTypeName,
        _operation_id: &str,
    ) -> Result<Value, ResourceRuntimeError> {
        Err(ResourceRuntimeError::IdentityUnbound)
    }


    /// Serve a public Resource request through a local authenticated session.
    ///
    /// Admission has already authenticated the peer and assigned its local
    /// daemon role. This method binds that peer credential into a
    /// request-scoped `AuthenticatedSubjectContext` and then uses the same
    /// Resource API client as the registered ComponentSession path. The peer
    /// uid is never read from the JSON envelope and is included in the
    /// transport/transcript binding used by the authorizer.
    pub(crate) async fn dispatch_public_cli_request(
        &self,
        request: &Value,
        peer_uid: u32,
    ) -> Result<Value, ResourceRuntimeError> {
        let requested_zone = request
            .get("zoneRef")
            .and_then(Value::as_str)
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if requested_zone != format!("Zone/{}", self.zone.as_str()) {
            return Err(ResourceRuntimeError::RouteMismatch);
        }
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if !route_service_matches(request.get("service"), method)? {
            return Err(ResourceRuntimeError::RouteMismatch);
        }
        validate_public_request_target(request, method)?;
        if [
            "subject",
            "subjectRef",
            "subjectUid",
            "principal",
            "principalRef",
            "role",
            "uid",
            "user",
            "userRef",
        ]
        .iter()
        .any(|field| request.get(*field).is_some())
        {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        let operation_id = public_operation_id(request, peer_uid, method);
        let resolved_user = self.resolve_public_user(peer_uid, &operation_id).await?;
        let read_only = matches!(method, "Get" | "List");
        if !read_only {
            self.refresh_authorization_policy().await?;
        }
        let context = d2bd_runtime::resource_runtime_support::local_user_subject_context(
            &self.zone,
            &resolved_user,
            &operation_id,
        )?;
        let state = self.policy_projection.installed_state()?;
        // U14: the Zone has exactly one plane. Every public request is served
        // by the manager-backed Resource API service, and the manager plane's
        // authorizer issues the request subject.
        let service = self.manager_api_service().inspect_err(|error| {
            tracing::warn!(
                zone = %self.zone.as_str(),
                error = ?error,
                "public request refused: manager-backed API service is unavailable",
            );
        })?;
        let authorizer = self.manager_plane_authorizer().inspect_err(|error| {
            tracing::warn!(
                zone = %self.zone.as_str(),
                error = ?error,
                "public request refused: manager plane authorizer is unavailable",
            );
        })?;
        let subject = match authorizer.issue_authenticated_subject(context, state) {
            Ok(subject) => subject,
            Err(error) => {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    error = ?error,
                    "public request refused: manager plane subject issuance failed",
                );
                return Err(ResourceRuntimeError::AuthorizationUnavailable);
            }
        };
        let adapter = ResourceBusAdapter::bind_component_session(service, subject)
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?;
        let client = adapter.client();
        self.dispatch_public_resource_call(&client, method, request, &operation_id)
            .await
    }

    /// One public resource call against the zone's manager-backed service.
    async fn dispatch_public_resource_call<S>(
        &self,
        client: &ResourceApiClient<S, UnavailableUpgradeDispatcher>,
        method: &str,
        request: &Value,
        operation_id: &str,
    ) -> Result<Value, ResourceRuntimeError>
    where
        S: d2b_resource_api::ResourceStoreBackend,
    {
        match method {
            "Get" => {
                let resource_ref = request
                    .get("resourceRef")
                    .and_then(Value::as_str)
                    .ok_or(ResourceRuntimeError::RequestInvalid)
                    .and_then(|value| {
                        ResourceRef::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)
                    })?;
                let mut meta = public_request_meta(&operation_id);
                meta.deadline_ms = 30_000;
                let response = client
                    .get(wire::GetRequest {
                        meta: protobuf::MessageField::some(meta),
                        target: protobuf::MessageField::some(wire::ResourceIdentity {
                            zone: self.zone.to_canonical_string(),
                            resource_type: resource_ref.resource_type().to_canonical_string(),
                            name: resource_ref.name().to_canonical_string(),
                            uid: None,
                            generation: None,
                            revision: None,
                            special_fields: protobuf::SpecialFields::new(),
                        }),
                        projection: {
                            let mut projection = wire::Projection::new();
                            projection.kind = protobuf::EnumOrUnknown::new(
                                wire::ProjectionKind::PROJECTION_KIND_FULL,
                            );
                            protobuf::MessageField::some(projection)
                        },
                        special_fields: protobuf::SpecialFields::new(),
                    })
                    .await;
                encode_public_get_response(response)
            }
            "List" => {
                let parsed = parse_list_request(request)?;
                let response = client
                    .list(public_list_request(parsed, &operation_id))
                    .await;
                encode_public_list_response(response)
            }
            "Create" => {
                let request_wire = public_create_request(self, request, &operation_id).await?;
                let response = client.create(request_wire).await;
                encode_public_create_response(response)
            }
            "UpdateSpec" => {
                let request_wire =
                    public_update_spec_request(&client, self, request, &operation_id).await?;
                let response = client.update_spec(request_wire).await;
                encode_public_update_spec_response(response)
            }
            "UpdateStatus" => {
                let request_wire =
                    public_update_status_request(&client, self, request, &operation_id).await?;
                let response = client.update_status(request_wire).await;
                encode_public_update_status_response(response)
            }
            "UpdateFinalizers" => {
                let request_wire = public_update_finalizers_request(self, request, &operation_id)?;
                let response = client.update_finalizers(request_wire).await;
                encode_public_update_finalizers_response(response)
            }
            "Delete" => {
                let target = public_target_ref(request)?;
                let current = public_get_resource(client, self, &target, &operation_id).await?;
                let request_wire =
                    public_delete_request_from_current(self, request, &operation_id, current)?;
                let response = client.delete(request_wire).await;
                encode_public_delete_response(response)
            }
            _ => Err(ResourceRuntimeError::CapabilityUnavailable),
        }
    }

    /// Forward a public Resource request through the authenticated Gateway
    /// Guest ComponentSession.
    ///
    /// The local runtime supplies only committed Zone metadata needed to
    /// encode the public wire request. The Guest session owns authorization,
    /// Provider execution, and the target store; no host Resource API client
    /// is consulted for the forwarded operation.
    pub(crate) async fn dispatch_gateway_resource_request(
        &self,
        session: &d2bd_runtime::guest_component_session::GuestComponentSessionClient,
        request: &Value,
        operation_id: &str,
    ) -> Result<Value, ResourceRuntimeError> {
        if session.identity().zone() != &self.zone {
            return Err(ResourceRuntimeError::RouteMismatch);
        }
        session
            .identity()
            .validate_route(&session.route_binding())
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if !route_service_matches(request.get("service"), method)? {
            return Err(ResourceRuntimeError::RouteMismatch);
        }
        let client = session.resource_service_client();
        match method {
            "Get" => {
                let target = public_target_ref(request)?;
                let mut meta = public_request_meta(operation_id);
                meta.deadline_ms = 30_000;
                let response = client
                    .get(
                        ttrpc::context::Context::default(),
                        &wire::GetRequest {
                            meta: protobuf::MessageField::some(meta),
                            target: protobuf::MessageField::some(public_identity(
                                self,
                                target.resource_type(),
                                target.name().as_str(),
                                None,
                                None,
                                None,
                            )),
                            projection: {
                                let mut projection = wire::Projection::new();
                                projection.kind = protobuf::EnumOrUnknown::new(
                                    wire::ProjectionKind::PROJECTION_KIND_FULL,
                                );
                                protobuf::MessageField::some(projection)
                            },
                            special_fields: protobuf::SpecialFields::new(),
                        },
                    )
                    .await
                    .map_err(|_| ResourceRuntimeError::ProviderPathUnavailable)?;
                d2bd_runtime::resource_runtime_support::encode_public_get_response(response)
            }
            "List" => {
                let parsed = parse_list_request(request)?;
                let response = client
                    .list(
                        ttrpc::context::Context::default(),
                        &public_list_request(parsed, operation_id),
                    )
                    .await
                    .map_err(|_| ResourceRuntimeError::ProviderPathUnavailable)?;
                d2bd_runtime::resource_runtime_support::encode_public_list_response(response)
            }
            "Create" => {
                let request_wire = public_create_request(self, request, operation_id).await?;
                let response = client
                    .create(ttrpc::context::Context::default(), &request_wire)
                    .await
                    .map_err(|_| ResourceRuntimeError::ProviderPathUnavailable)?;
                encode_public_create_response(response)
            }
            "UpdateSpec" => {
                let target = public_target_ref(request)?;
                let current = gateway_get_resource(&client, self, &target, operation_id).await?;
                if current.get("type").and_then(Value::as_str) == Some("error") {
                    return Ok(current);
                }
                let request_wire = public_update_spec_request_from_current(
                    self,
                    request,
                    operation_id,
                    &target,
                    current,
                )?;
                let response = client
                    .update_spec(ttrpc::context::Context::default(), &request_wire)
                    .await
                    .map_err(|_| ResourceRuntimeError::ProviderPathUnavailable)?;
                encode_public_update_spec_response(response)
            }
            "UpdateStatus" => {
                let target = public_target_ref(request)?;
                let current = gateway_get_resource(&client, self, &target, operation_id).await?;
                if current.get("type").and_then(Value::as_str) == Some("error") {
                    return Ok(current);
                }
                let request_wire = public_update_status_request_from_current(
                    self,
                    request,
                    operation_id,
                    &target,
                    current,
                )?;
                let response = client
                    .update_status(ttrpc::context::Context::default(), &request_wire)
                    .await
                    .map_err(|_| ResourceRuntimeError::ProviderPathUnavailable)?;
                encode_public_update_status_response(response)
            }
            "UpdateFinalizers" => {
                let request_wire = public_update_finalizers_request(self, request, operation_id)?;
                let response = client
                    .update_finalizers(ttrpc::context::Context::default(), &request_wire)
                    .await
                    .map_err(|_| ResourceRuntimeError::ProviderPathUnavailable)?;
                encode_public_update_finalizers_response(response)
            }
            "Delete" => {
                let target = public_target_ref(request)?;
                let current =
                    gateway_get_resource(&client, self, &target, operation_id).await?;
                if current.get("type").and_then(Value::as_str) == Some("error") {
                    return Ok(current);
                }
                let request_wire =
                    public_delete_request_from_current(self, request, operation_id, current)?;
                let response = client
                    .delete(ttrpc::context::Context::default(), &request_wire)
                    .await
                    .map_err(|_| ResourceRuntimeError::ProviderPathUnavailable)?;
                encode_public_delete_response(response)
            }
            _ => Err(ResourceRuntimeError::CapabilityUnavailable),
        }
    }

    /// Verify the trusted persisted Device row used by the TPM reconcile
    /// adapter and return Core's sealed legacy-state decision. The VM binding
    /// is read from the authenticated Device record, while the legacy-state
    /// decision comes from the trusted Core bundle resolver; request fields
    /// cannot select either independently.
    #[allow(dead_code)]
    #[allow(dead_code)]
    pub(crate) async fn tpm_device_is_admitted(
        &self,
        device_uid: &ResourceUid,
        device_ref: &ResourceRef,
        vm_id: &str,
        operation_id: &str,
        legacy_intent_anchor: Option<&str>,
    ) -> Result<LegacyTpmMigrationDecision, ResourceRuntimeError> {
        let resource = self
            .committed_resource_stored(device_ref, operation_id)
            .await
            .inspect_err(|error| {
                tracing::warn!(
                    device = %device_ref,
                    error = %error,
                    "legacy TPM device admission read failed",
                );
            })
            .ok();
        let Some(resource) = resource.filter(|resource| {
            resource.uid == *device_uid
                && resource.resource_ref == *device_ref
                && resource.resource_ref.resource_type().as_str() == "Device"
        }) else {
            return Err(ResourceRuntimeError::AuthenticationUnavailable);
        };
        let value = serde_json::from_slice::<Value>(&resource.canonical_json)
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
        let spec = value
            .get("spec")
            .and_then(Value::as_object)
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        if spec.get("providerRef").and_then(Value::as_str)
            != Some(d2b_provider_device_tpm::PROVIDER_REF)
        {
            return Err(ResourceRuntimeError::AuthenticationUnavailable);
        }
        if !Self::tpm_device_targets_vm(&value, vm_id) {
            return Err(ResourceRuntimeError::AuthenticationUnavailable);
        }
        let intent = format!("legacy-swtpm:vm:{vm_id}");
        Ok(Self::tpm_migration_decision(
            vm_id,
            &intent,
            legacy_intent_anchor,
        ))
    }

    /// Load and validate the committed Device record before a security-key
    /// provider constructs its one-use admission. Request fields select a
    /// candidate only; the returned values all originate from the manager row.
    pub(crate) async fn security_key_device_is_admitted(
        &self,
        request: SecurityKeyDeviceAdmissionRequest<'_>,
    ) -> Result<SecurityKeyDeviceAdmission, ResourceRuntimeError> {
        let resource = self
            .committed_resource_stored(request.device_ref, request.operation_id)
            .await
            .inspect_err(|error| {
                tracing::warn!(
                    device = %request.device_ref,
                    error = %error,
                    "security-key device admission read failed",
                );
            })
            .ok();
        let Some(resource) = resource.filter(|resource| {
            resource.uid == *request.device_uid
                && resource.resource_ref == *request.device_ref
                && resource.resource_ref.resource_type().as_str() == "Device"
                && resource.zone == self.zone
        }) else {
            return Err(ResourceRuntimeError::AuthenticationUnavailable);
        };
        let value = serde_json::from_slice::<Value>(&resource.canonical_json)
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
        if !Self::security_key_device_matches(
            &value,
            &self.zone,
            request.request_zone_ref,
            request.holder_ref,
            request.vm_id,
            request.selector_id,
        ) {
            return Err(ResourceRuntimeError::AuthenticationUnavailable);
        }
        let zone_ref = ResourceRef::parse(&format!("Zone/{}", self.zone.as_str()))
            .expect("ZoneId always produces a valid Zone resource reference");
        Ok(SecurityKeyDeviceAdmission {
            zone_ref,
            device_uid: resource.uid,
            holder_ref: request.holder_ref.clone(),
            selector_id: request.selector_id.to_owned(),
        })
    }


    #[allow(dead_code)]
    #[allow(dead_code)]
    fn tpm_device_targets_vm(resource: &Value, vm_id: &str) -> bool {
        resource
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("ownerRef"))
            .and_then(Value::as_str)
            .and_then(|owner| owner.strip_prefix("Guest/"))
            == Some(vm_id)
    }

    #[allow(dead_code)]
    #[allow(dead_code)]
    fn security_key_device_matches(
        resource: &Value,
        zone: &ZoneId,
        request_zone_ref: &ResourceRef,
        holder_ref: &ResourceRef,
        vm_id: &str,
        selector_id: &str,
    ) -> bool {
        let expected_zone_ref = ResourceRef::parse(&format!("Zone/{}", zone.as_str()))
            .expect("ZoneId always produces a valid Zone resource reference");
        if request_zone_ref != &expected_zone_ref
            || holder_ref.resource_type().as_str() != "Guest"
            || holder_ref.name().as_str() != vm_id
        {
            return false;
        }
        let Some(metadata) = resource.get("metadata").and_then(Value::as_object) else {
            return false;
        };
        if metadata.get("zone").and_then(Value::as_str) != Some(zone.as_str())
            || metadata.get("ownerRef").and_then(Value::as_str)
                != Some(holder_ref.to_canonical_string().as_str())
        {
            return false;
        }
        resource
            .get("spec")
            .and_then(Value::as_object)
            .filter(|spec| {
                spec.get("providerRef").and_then(Value::as_str)
                    == Some(d2b_provider_device_security_key::PROVIDER_REF)
            })
            .and_then(|spec| spec.get("inventory"))
            .and_then(Value::as_object)
            .and_then(|inventory| inventory.get("selector"))
            .and_then(Value::as_object)
            .and_then(|selector| selector.get("label"))
            .and_then(Value::as_str)
            == Some(selector_id)
    }

    #[allow(dead_code)]
    #[allow(dead_code)]
    fn tpm_migration_decision(
        vm_id: &str,
        intent: &str,
        legacy_intent_anchor: Option<&str>,
    ) -> LegacyTpmMigrationDecision {
        if let Some(anchor) = legacy_intent_anchor {
            LegacyTpmMigrationDecision::adoption_required(vm_id, intent, anchor)
        } else {
            LegacyTpmMigrationDecision::not_applicable(vm_id, intent)
        }
    }

    /// Close the production Zone runtime's background tasks before it is
    /// discarded.
    pub async fn shutdown(self) -> Result<(), ResourceRuntimeError> {
        let ZoneResourceRuntime {
            bus,
            registrar,
            ingress,
            service_task,
            authority_recovery,
            process_status_client,
            audio_runtime,
            controller_sessions,
            controller_session_reconcile_task,
            controller_session_reconcile_shutdown,
            controller_session_coordinator,
            assignments,
            ..
        } = self;
        controller_session_reconcile_shutdown.store(true, Ordering::Release);
        if let Some(task) = service_task
            .into_inner()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
        {
            task.abort();
            let _ = task.await;
        }
        drop(audio_runtime);
        let controller_session_task = controller_session_reconcile_task
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .take();
        if let Some(task) = controller_session_task {
            task.abort();
            let _ = task.await;
        }
        controller_session_coordinator
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .take();
        let sessions = Arc::try_unwrap(controller_sessions)
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .into_inner()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
        for (_, mut session) in sessions {
            session.cancel_backend_lease();
            if d2b_provider_runtime_cloud_hypervisor::is_provider_ref(
                session.binding.provider_ref(),
            ) {
                assignments
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .revoke_session_for(&session.binding);
            }
            send_controller_assignment_revocations(&session.driver, &session.assignments).await;
            let _ = session
                .driver
                .close(
                    d2b_contracts_zone_session::v3::component_session::CloseReason::RoleMismatch,
                    d2b_contracts_zone_session::v3::component_session::Remediation::ReplaceGeneration,
                )
                .await;
            session.service_task.abort();
            let _ = session.service_task.await;
            let mut ingress = session.ingress;
            let mut registrar = registrar
                .lock()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
            if let Some(registrar) = registrar.as_mut() {
                let _ = registrar.revoke_in_place(&mut ingress).await;
            }
        }
        drop(process_status_client);
        drop(
            ingress
                .into_inner()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
        );
        drop(
            Arc::try_unwrap(registrar)
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
                .into_inner()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
        );
        drop(bus);
        drop(authority_recovery);
        Ok(())
    }
}

fn is_u9_provider_ref(value: &str) -> bool {
    INTERACTION_PROVIDER_REFS.contains(&value)
}

fn contains_u9_provider_ref(value: &Value) -> bool {
    match value {
        Value::String(value) => is_u9_provider_ref(value),
        Value::Array(values) => values.iter().any(contains_u9_provider_ref),
        Value::Object(values) => values.values().any(contains_u9_provider_ref),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn is_u9_resource_type(resource_type: &ResourceTypeName) -> bool {
    let resource_type = resource_type.as_str();
    resource_type.starts_with("display-wayland.")
        || resource_type.starts_with("audio.d2bus.org.")
        || resource_type.starts_with("shell-terminal.d2bus.org.")
}

/// Whether the manager holds any row of the interaction family (U9): a
/// committed interaction `Provider` row, a row of a type the interaction
/// factory serves, or a row referring to one of those Providers.
///
/// U14: the durable store is gone, so the presence scan reads the manager's
/// rows of the closed interaction type set (plus `Provider`, whose rows
/// select the family by reference) instead of paging every type.
async fn interaction_resources_present(
    plane: &dyn ControllerPlaneView,
) -> Result<bool, ResourceRuntimeError> {
    let provider_refs = INTERACTION_PROVIDER_REFS
        .iter()
        .map(|provider| {
            ResourceRef::parse(provider).map_err(|_| ResourceRuntimeError::HandlerNotReady)
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let mut resource_types = vec!["Provider".to_owned()];
    resource_types.extend(INTERACTION_TYPES.iter().map(|resource_type| (*resource_type).to_owned()));
    for resource_type in resource_types {
        for resource in bridge_manager_rows(plane, &resource_type).await? {
            if provider_refs.contains(&resource.resource_ref)
                || is_u9_resource_type(resource.resource_ref.resource_type())
            {
                return Ok(true);
            }
            let value: Value = serde_json::from_slice(&resource.canonical_json)
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
            if contains_u9_provider_ref(&value) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn derive_interaction_state(
    interaction_present: bool,
    configuration: Option<&CommittedInteractionProviderConfiguration>,
    identity: Option<&CommittedInteractionIdentity>,
    refused: bool,
) -> InteractionState {
    if !interaction_present && configuration.is_none() && identity.is_none() && !refused {
        InteractionState::Absent
    } else if !refused && configuration.is_some() && identity.is_some() {
        InteractionState::Ready
    } else {
        InteractionState::Refused
    }
}

async fn load_interaction_provider_configuration(
    plane: &dyn ControllerPlaneView,
    zone: &ZoneId,
) -> Result<Option<CommittedInteractionProviderConfiguration>, ResourceRuntimeError> {
    let clipboard_ref = ResourceRef::parse("Provider/clipboard-wayland")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let notification_ref = ResourceRef::parse("Provider/notification-desktop")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let mut clipboard = None;
    let mut notification = None;
    for resource in bridge_manager_rows(plane, "Provider").await? {
        if resource.resource_ref == clipboard_ref {
            if clipboard.is_some() {
                return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
            }
            clipboard = Some(parse_committed_clipboard_configuration(zone, &resource)?);
        } else if resource.resource_ref == notification_ref {
            if notification.is_some() {
                return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
            }
            notification = Some(parse_committed_notification_configuration(zone, &resource)?);
        }
    }
    if clipboard.is_none() && notification.is_none() {
        Ok(None)
    } else {
        Ok(Some(CommittedInteractionProviderConfiguration {
            clipboard,
            notification,
        }))
    }
}

async fn load_committed_interaction_identity(
    plane: &dyn ControllerPlaneView,
    zone: &ZoneId,
    configuration: Option<&CommittedInteractionProviderConfiguration>,
) -> Result<Option<CommittedInteractionIdentity>, ResourceRuntimeError> {
    let session_resource_type = "display-wayland.d2bus.org.WaylandSession";
    let mut sessions = bridge_manager_rows(plane, session_resource_type).await?;
    if sessions.is_empty() {
        return if configuration.is_none() {
            Ok(None)
        } else {
            Err(ResourceRuntimeError::InteractionConfigurationUnavailable)
        };
    }
    if sessions.len() != 1 {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let session_resource = sessions
        .pop()
        .ok_or(ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let session_spec = committed_wayland_session_spec(zone, &session_resource)
        .inspect_err(|error| {
            tracing::error!(
                zone = %zone.as_str(),
                error = %error,
                "resource runtime committed Wayland session parse failed",
            );
        })?;
    let subject_ref = session_spec.guest_ref().clone();
    let host_execution_ref = session_spec.host_ref().clone();
    let user_ref = session_spec.user_ref().clone();
    let expected_policy_type =
        ResourceTypeName::parse("display-wayland.d2bus.org.WaylandPolicy")
            .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    if session_spec.policy_ref().resource_type() != &expected_policy_type {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let _policy_resource = committed_resource(plane, zone, session_spec.policy_ref())
        .await
        .inspect_err(|error| {
            tracing::error!(
                zone = %zone.as_str(),
                operation = "interaction-policy-lookup",
                error = %error,
                "resource runtime committed Wayland policy lookup failed",
            );
        })?;
    let subject_uid = committed_resource_uid(plane, zone, &subject_ref)
        .await
        .inspect_err(|error| {
            tracing::error!(
                zone = %zone.as_str(),
                operation = "interaction-subject-lookup",
                error = %error,
                "resource runtime committed interaction subject lookup failed",
            );
        })?;
    let _host_uid = committed_resource_uid(plane, zone, &host_execution_ref)
        .await
        .inspect_err(|error| {
            tracing::error!(
                zone = %zone.as_str(),
                operation = "interaction-host-lookup",
                error = %error,
                "resource runtime committed interaction Host lookup failed",
            );
        })?;
    let _user_uid = committed_resource_uid(plane, zone, &user_ref)
        .await
        .inspect_err(|error| {
            tracing::error!(
                zone = %zone.as_str(),
                operation = "interaction-user-lookup",
                error = %error,
                "resource runtime committed interaction User lookup failed",
            );
        })?;

    let display_ref = ResourceRef::parse("Provider/display-wayland")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let display_resource = committed_resource(plane, zone, &display_ref)
        .await
        .inspect_err(|error| {
            tracing::error!(
                zone = %zone.as_str(),
                operation = "display-provider-lookup",
                error = %error,
                "resource runtime committed display Provider lookup failed",
            );
        })?;
    let (_, _, display_provider_generation, _, _) =
        committed_provider_spec(zone, &display_resource, &display_ref)
            .inspect_err(|error| {
                tracing::error!(
                    zone = %zone.as_str(),
                    operation = "display-provider-validation",
                    error = %error,
                    "resource runtime committed display Provider validation failed",
                );
            })?;

    let mut allowed_guest_sources = BTreeMap::from([(subject_ref.clone(), subject_uid.clone())]);
    let mut clipboard_provider_generation = None;
    let mut clipboard_provider_uid = None;
    let mut notification_provider_generation = None;
    let mut notification_provider_uid = None;
    if let Some(configuration) = configuration {
        if !configuration.is_complete() {
            return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
        }
        if let Some(clipboard) = configuration.clipboard() {
            if clipboard.host_execution_ref != host_execution_ref
                || clipboard.host_user_ref != user_ref
                || clipboard.display_wayland_ref != display_ref
            {
                return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
            }
            for guest_ref in &clipboard.guest_sources {
                let uid = committed_resource_uid(plane, zone, guest_ref).await?;
                allowed_guest_sources.insert(guest_ref.clone(), uid);
            }
            clipboard_provider_generation = Some(clipboard.resource_generation);
            clipboard_provider_uid = Some(clipboard.resource_uid().clone());
        }
        if let Some(notification) = configuration.notification() {
            if notification.host_execution_ref != host_execution_ref
                || notification.observer_user_ref() != &user_ref
                || notification.config.display_wayland_ref() != Some(&display_ref)
            {
                return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
            }
            for guest_ref in notification.guest_sources() {
                let uid = committed_resource_uid(plane, zone, guest_ref).await?;
                allowed_guest_sources.insert(guest_ref.clone(), uid);
            }
            notification_provider_generation = Some(notification.resource_generation);
            notification_provider_uid = Some(notification.resource_uid().clone());
        }
    }

    Ok(Some(CommittedInteractionIdentity {
        zone: zone.clone(),
        wayland_session_ref: session_resource.resource_ref,
        wayland_session_uid: session_resource.uid,
        subject_ref,
        subject_uid,
        host_execution_ref,
        user_ref,
        allowed_guest_sources,
        display_provider_generation,
        clipboard_provider_generation,
        clipboard_provider_uid,
        notification_provider_generation,
        notification_provider_uid,
    }))
}

fn committed_wayland_session_spec(
    zone: &ZoneId,
    resource: &StoredResource,
) -> Result<WaylandSessionSpec, ResourceRuntimeError> {
    let expected_type = ResourceTypeName::parse("display-wayland.d2bus.org.WaylandSession")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    if &resource.zone != zone
        || resource.resource_ref.resource_type() != &expected_type
        || resource.generation.get() == 0
        || resource.revision.get() == 0
    {
        tracing::error!(
            zone = %zone.as_str(),
            resource_zone = %resource.zone.as_str(),
            resource_ref = %resource.resource_ref.to_canonical_string(),
            generation = resource.generation.get(),
            revision = resource.revision.get(),
            "committed Wayland session row failed stored-resource identity checks",
        );
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let envelope = ResourceEnvelope::from_json(&resource.canonical_json).map_err(|error| {
        tracing::error!(
            zone = %zone.as_str(),
            error = ?error,
            "committed Wayland session envelope decode failed",
        );
        ResourceRuntimeError::InteractionConfigurationUnavailable
    })?;
    if envelope.resource_type() != &expected_type
        || envelope.metadata().zone() != zone
        || envelope.metadata().uid() != &resource.uid
        || envelope.metadata().generation() != resource.generation
        || envelope.metadata().revision() != resource.revision
        || envelope
            .digest()
            .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?
            != resource.payload_digest
    {
        tracing::error!(
            zone = %zone.as_str(),
            envelope_type = %envelope.resource_type().as_str(),
            envelope_zone = %envelope.metadata().zone().as_str(),
            envelope_uid = %envelope.metadata().uid().as_str(),
            stored_uid = %resource.uid.as_str(),
            envelope_generation = envelope.metadata().generation().get(),
            stored_generation = resource.generation.get(),
            envelope_revision = envelope.metadata().revision().get(),
            stored_revision = resource.revision.get(),
            "committed Wayland session envelope failed integrity checks",
        );
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let spec =
        serde_json::from_slice::<WaylandSessionSpec>(&envelope.spec().base().to_canonical_bytes())
            .map_err(|error| {
                tracing::error!(
                    zone = %zone.as_str(),
                    error = ?error,
                    "committed Wayland session spec decode failed",
                );
                ResourceRuntimeError::InteractionConfigurationUnavailable
            })?;
    Ok(spec)
}

async fn committed_resource_uid(
    plane: &dyn ControllerPlaneView,
    zone: &ZoneId,
    resource_ref: &ResourceRef,
) -> Result<ResourceUid, ResourceRuntimeError> {
    let resource = committed_resource(plane, zone, resource_ref).await?;
    Ok(resource.uid)
}

async fn receive_controller_bootstrap(
    daemon_socket: &SeqpacketSocket,
) -> Result<(SeqpacketSocket, PeerCredentials), ResourceRuntimeError> {
    let policy = controller_bootstrap_attachment_policy();
    let capacity = AncillaryCapacity::from_policy(policy)
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let scopes =
        controller_credit_scopes().map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let burst = tokio::time::timeout(
        CONTROLLER_BOOTSTRAP_TIMEOUT,
        daemon_socket.recv_burst(
            d2b_contracts_zone_session::v3::component_session::LimitProfile::local_default(),
            capacity,
            &scopes,
            2,
        ),
    )
    .await
    .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
    .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    if burst.packets.len() != 1 {
        return Err(ResourceRuntimeError::AuthenticationUnavailable);
    }
    let packet = burst
        .packets
        .into_iter()
        .next()
        .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
    if packet.payload() != d2b_session_unix::CONTROLLER_BOOTSTRAP_PROTOCOL_MARKER {
        return Err(ResourceRuntimeError::AuthenticationUnavailable);
    }
    let (resource_fd, credentials) = packet
        .into_single_file_and_credentials()
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let resource_socket = SeqpacketSocket::from_parent_prearmed(resource_fd)
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    if resource_socket
        .acceptor_peer_credentials()
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
        != credentials
    {
        return Err(ResourceRuntimeError::AuthenticationUnavailable);
    }
    Ok((resource_socket, credentials))
}

/// Registers the Provider session's named streams on the engine before the
/// session driver exists.
///
/// Named-stream registration is local to each endpoint, so the Provider can
/// send on a fixed stream id as soon as its own handshake completes. Once the
/// driver task is spawned it may route an inbound fragment first, and a
/// fragment for an unregistered stream fails the whole session as an invalid
/// channel.
fn preregister_provider_session_streams<T: OwnedTransport>(
    engine: &mut SessionEngine<T>,
) -> Result<(), ResourceRuntimeError> {
    for (stream, credit) in [
        (PROVIDER_BOOTSTRAP_STREAM_ID, PROVIDER_BOOTSTRAP_STREAM_CREDIT),
        (
            PROVIDER_DELIVERY_KEY_STREAM_ID,
            PROVIDER_DELIVERY_KEY_STREAM_CREDIT,
        ),
        (PROVIDER_READY_STREAM_ID, PROVIDER_READY_STREAM_CREDIT),
    ] {
        let stream = StreamId::new(stream)
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
        engine
            .open_named_stream(stream, credit, credit)
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    }
    Ok(())
}

async fn receive_provider_ready(
    driver: &SessionDriverHandle,
) -> Result<(), ResourceRuntimeError> {
    let stream = StreamId::new(PROVIDER_READY_STREAM_ID)
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let mut received = Vec::new();
    loop {
        match driver
            .receive_named_stream_for(stream)
            .await
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
        {
            StreamEvent::Data {
                stream: received_stream,
                bytes,
            } if received_stream == stream => {
                received.extend_from_slice(&bytes);
                if received.len() > PROVIDER_READY_MARKER.len() {
                    return Err(ResourceRuntimeError::AuthenticationUnavailable);
                }
                driver
                    .grant_named_stream_credit(
                        stream,
                        u32::try_from(bytes.len())
                            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
                    )
                    .await
                    .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
            }
            StreamEvent::RemoteClosed {
                stream: closed_stream,
            } if closed_stream == stream => {
                if received == PROVIDER_READY_MARKER {
                    return Ok(());
                }
                return Err(ResourceRuntimeError::AuthenticationUnavailable);
            }
            StreamEvent::Reset { .. } => {
                return Err(ResourceRuntimeError::AuthenticationUnavailable);
            }
            _ => return Err(ResourceRuntimeError::AuthenticationUnavailable),
        }
    }
}

async fn committed_resource(
    plane: &dyn ControllerPlaneView,
    zone: &ZoneId,
    resource_ref: &ResourceRef,
) -> Result<StoredResource, ResourceRuntimeError> {
    if !is_supported_committed_resource_ref(resource_ref) {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let resource = bridge_manager_row(plane, resource_ref)
        .await
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?
        .ok_or(ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    validate_committed_resource(zone, resource_ref, resource)
}

async fn current_committed_resource(
    plane: &dyn ControllerPlaneView,
    zone: &ZoneId,
    resource_ref: &ResourceRef,
    _operation_id: &str,
) -> Result<StoredResource, ResourceRuntimeError> {
    if !is_supported_committed_resource_ref(resource_ref) {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let resource = bridge_manager_row(plane, resource_ref)
        .await
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?
        .ok_or(ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    validate_committed_resource(zone, resource_ref, resource)
}

fn is_supported_committed_resource_ref(resource_ref: &ResourceRef) -> bool {
    matches!(
        resource_ref.resource_type().as_str(),
        "Guest"
            | "Host"
            | "Provider"
            | "User"
            | "display-wayland.d2bus.org.WaylandPolicy"
            | "display-wayland.d2bus.org.WaylandSession"
    )
}

fn validate_committed_resource(
    zone: &ZoneId,
    resource_ref: &ResourceRef,
    resource: StoredResource,
) -> Result<StoredResource, ResourceRuntimeError> {
    if resource.zone != *zone
        || resource.resource_ref != *resource_ref
        || resource.generation.get() == 0
        || resource.revision.get() == 0
    {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    if envelope.resource_type() != resource_ref.resource_type()
        || envelope.metadata().name() != resource_ref.name()
        || envelope.metadata().zone() != zone
        || envelope.metadata().uid() != &resource.uid
        || envelope.metadata().generation() != resource.generation
        || envelope.metadata().revision() != resource.revision
        || envelope
            .digest()
            .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?
            != resource.payload_digest
    {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    Ok(resource)
}

async fn load_controller_policy_subjects(
    zone: &ZoneId,
    plane: &dyn ControllerPlaneView,
    providers: Option<&crate::process_provider_runtime::ProductionProcessProviders>,
    controller_sessions: &Mutex<BTreeMap<ResourceRef, ControllerSession>>,
) -> Result<BTreeSet<BoundSubject>, ResourceRuntimeError> {
    let mut contexts = BTreeMap::new();
    if let Some(providers) = providers {
        for context in providers.controller_bootstrap_contexts(zone) {
            contexts.insert(context.process_ref().clone(), context);
        }
    }
    for session in controller_sessions
        .lock()
        .map_err(|_| ResourceRuntimeError::PolicyUnavailable)?
        .values()
    {
        let context = session.context.clone();
        if let Some(existing) = contexts.get(context.process_ref())
            && existing != &context
        {
            return Err(ResourceRuntimeError::PolicyUnavailable);
        }
        contexts.insert(context.process_ref().clone(), context);
    }
    let provider_refs = contexts
        .values()
        .map(|context| context.provider_owner_ref().clone())
        .collect::<BTreeSet<_>>();
    // `Provider` is a manager row: the committed identity is the manager's.
    let identities = committed_controller_provider_identities(zone, plane, provider_refs)
        .await
        .map_err(|error| {
            tracing::warn!(
                zone = zone.as_str(),
                error = ?error,
                "controller policy subjects: committed Provider identities unavailable",
            );
            ResourceRuntimeError::PolicyUnavailable
        })?;
    let mut subjects = BTreeSet::new();
    for context in contexts.values() {
        let Some((provider_uid, provider_generation)) =
            identities.get(context.provider_owner_ref())
        else {
            tracing::warn!(
                zone = zone.as_str(),
                provider = %context.provider_owner_ref().to_canonical_string(),
                "controller policy subjects: no committed Provider identity for a controller context",
            );
            return Err(ResourceRuntimeError::PolicyUnavailable);
        };
        if provider_uid != context.provider_uid()
            || *provider_generation != context.provider_generation()
        {
            tracing::warn!(
                zone = zone.as_str(),
                provider = %context.provider_owner_ref().to_canonical_string(),
                committed_uid = %provider_uid.as_str(),
                committed_generation = provider_generation.get(),
                context_uid = %context.provider_uid().as_str(),
                context_generation = context.provider_generation().get(),
                "controller policy subjects: controller context identity does not match the committed Provider",
            );
            return Err(ResourceRuntimeError::PolicyUnavailable);
        }
        subjects.insert(BoundSubject {
            subject_ref: context.provider_owner_ref().clone(),
            subject_uid: provider_uid.clone(),
        });
    }
    Ok(subjects)
}

fn controller_assignment_refresh_action<'a>(
    context: &'a crate::process_provider_runtime::ControllerBootstrapContext,
    error: ControllerAssignmentRefreshError,
) -> ControllerAssignmentRefreshAction<'a> {
    match error {
        ControllerAssignmentRefreshError::Retryable => {
            ControllerAssignmentRefreshAction::Retryable { context }
        }
        ControllerAssignmentRefreshError::Failed(error) => {
            ControllerAssignmentRefreshAction::Failed { context, error }
        }
    }
}

async fn reset_controller_assignment_stream(
    driver: &SessionDriverHandle,
    stream: StreamId,
) -> Result<(), ControllerAssignmentRefreshError> {
    driver.reset_named_stream(stream).await.map_err(|_| {
        ControllerAssignmentRefreshError::Failed(ResourceRuntimeError::AuthenticationUnavailable)
    })
}

async fn send_controller_assignment_frame(
    driver: &SessionDriverHandle,
    stream: StreamId,
    encoded: Vec<u8>,
    on_send_failure: impl FnOnce() + Send,
) -> Result<(), ControllerAssignmentRefreshError> {
    if driver.send_named_stream(stream, encoded).await.is_ok() {
        return Ok(());
    }
    on_send_failure();
    reset_controller_assignment_stream(driver, stream).await?;
    Err(ControllerAssignmentRefreshError::Retryable)
}

fn controller_generation_is_stale(
    current_generation: Option<ControllerGeneration>,
    context_generation: ControllerGeneration,
) -> bool {
    current_generation != Some(context_generation)
}

fn assignment_error_is_off_target(error: AssignmentError) -> bool {
    matches!(
        error,
        AssignmentError::InvalidRole
            | AssignmentError::ResourceTypeUnowned
            | AssignmentError::TargetMismatch
            | AssignmentError::TargetKindUnsupported
    )
}

fn controller_session_matches(
    active: &ControllerSessionBinding,
    requested: &ControllerSessionBinding,
    service_task_finished: bool,
) -> bool {
    !service_task_finished && active == requested
}

fn assignment_resource_matches(
    resource_ref: &ResourceRef,
    resource_uid: &ResourceUid,
    resource_generation: ResourceGeneration,
    resource_revision: ZoneRevision,
    resource: &ResourceEnvelope,
) -> bool {
    resource_ref
        == &ResourceRef::new(
            resource.resource_type().clone(),
            resource.metadata().name().clone(),
        )
        && resource_uid == resource.metadata().uid()
        && resource_generation == resource.metadata().generation()
        && resource_revision == resource.metadata().revision()
}

/// Validate one manager-served assignment candidate.
///
/// The manager row carries its own revision and identity, so the checks are
/// the row's own integrity (zone, non-zero identity, envelope matching the
/// row) plus the assignment's own filter: only rows declaring this Provider
/// are assigned. `Ok(None)` is a row that is not this role's assignment.
fn validate_assignment_row(
    stored: &StoredResource,
    zone: &ZoneId,
    provider_ref: &ResourceRef,
) -> Result<Option<ResourceEnvelope>, ControllerAssignmentRefreshError> {
    if &stored.zone != zone || stored.revision.get() == 0 || stored.generation.get() == 0 {
        return Err(ControllerAssignmentRefreshError::Failed(
            ResourceRuntimeError::AuthorizationUnavailable,
        ));
    }
    let envelope = ResourceEnvelope::from_json(&stored.canonical_json).map_err(|_| {
        ControllerAssignmentRefreshError::Failed(ResourceRuntimeError::AuthorizationUnavailable)
    })?;
    if envelope.resource_type() != stored.resource_ref.resource_type()
        || envelope.metadata().name() != stored.resource_ref.name()
        || envelope.metadata().zone() != zone
        || envelope.metadata().uid() != &stored.uid
        || envelope.metadata().generation() != stored.generation
        || envelope.metadata().revision() != stored.revision
        || envelope.digest().map_err(|_| {
            ControllerAssignmentRefreshError::Failed(ResourceRuntimeError::AuthorizationUnavailable)
        })? != stored.payload_digest
    {
        return Err(ControllerAssignmentRefreshError::Failed(
            ResourceRuntimeError::AuthorizationUnavailable,
        ));
    }
    Ok((envelope.spec().provider_ref() == Some(provider_ref)).then_some(envelope))
}

fn admit_assignment_or_skip(
    assignments: &AssignmentRegistry,
    request: AssignmentRequest<'_>,
) -> Result<Option<ResourceClientLease>, ResourceRuntimeError> {
    let mut registry = assignments
        .lock()
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    match registry.admit(request) {
        Ok(lease) => Ok(Some(lease)),
        Err(error) if assignment_error_is_off_target(error) => Ok(None),
        Err(error) => {
            tracing::warn!(
                error = ?error,
                "external Provider controller assignment registry rejected resource",
            );
            Err(ResourceRuntimeError::AuthorizationUnavailable)
        }
    }
}

async fn send_controller_assignment_revocations(
    driver: &SessionDriverHandle,
    assignments: &BTreeMap<ResourceUid, ResourceClientLease>,
) {
    let Ok(stream) = StreamId::new(CONTROLLER_ASSIGNMENT_STREAM_ID) else {
        return;
    };
    for lease in assignments.values() {
        let Ok(bytes) =
            ControllerAssignmentGrant::encode_revocation(lease.provider_ref(), lease.identity())
        else {
            continue;
        };
        if let Err(error) = driver.send_named_stream(stream, bytes).await {
            tracing::warn!(
                error = %error,
                "controller assignment revocation delivery failed",
            );
            let _ = driver.reset_named_stream(stream).await;
            break;
        }
    }
}

fn controller_session_binding(
    context: &crate::process_provider_runtime::ControllerBootstrapContext,
    session_generation: ReconnectGeneration,
) -> Result<ControllerSessionBinding, ResourceRuntimeError> {
    let target_kind = match context.execution_ref().resource_type().as_str() {
        "Host" => PlacementTargetKind::Host,
        "Guest" => PlacementTargetKind::Guest,
        _ => return Err(ResourceRuntimeError::AuthenticationUnavailable),
    };
    let controller_role =
        if d2b_provider_runtime_cloud_hypervisor::is_provider_ref(context.provider_owner_ref()) {
            ResourceRef::parse(d2b_provider_runtime_cloud_hypervisor::CONTROLLER_ROLE_REF)
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
        } else {
            context.process_ref().clone()
        };
    ControllerSessionBinding::new(
        context.process_ref().clone(),
        context.provider_owner_ref().clone(),
        controller_role,
        AssignmentTarget::Execution {
            kind: target_kind,
            reference: context.execution_ref().clone(),
        },
        context.provider_generation(),
        context.controller_generation(),
        session_generation,
    )
    .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)
}

pub(crate) fn controller_session_resource_fences(
    sessions: impl IntoIterator<
        Item = (
            ResourceRef,
            crate::process_provider_runtime::ControllerBootstrapContext,
        ),
    >,
    resources: &[StoredResource],
) -> Vec<(
    ResourceRef,
    crate::process_provider_runtime::ControllerBootstrapContext,
)> {
    sessions
        .into_iter()
        .filter(|(_, context)| {
            !resources
                .iter()
                .find(|resource| resource.resource_ref == *context.process_ref())
                .is_some_and(|resource| controller_resource_matches(context, resource))
        })
        .collect()
}

fn controller_resource_matches(
    context: &crate::process_provider_runtime::ControllerBootstrapContext,
    resource: &StoredResource,
) -> bool {
    if resource.zone != *context.zone()
        || resource.resource_ref != *context.process_ref()
        || resource.uid != *context.process_uid()
        || resource.generation != context.generation()
        || resource.revision.get() == 0
    {
        return false;
    }
    let Ok(envelope) = ResourceEnvelope::from_json(&resource.canonical_json) else {
        return false;
    };
    if envelope.resource_type().as_str() != "Process"
        || envelope.metadata().zone() != &resource.zone
        || envelope.metadata().uid() != &resource.uid
        || envelope.metadata().generation() != resource.generation
        || envelope.metadata().revision() != resource.revision
        || envelope.digest().ok().as_deref() != Some(resource.payload_digest.as_str())
        || envelope.metadata().owner_ref() != Some(context.provider_owner_ref())
        || resource.owner_uid.as_ref() != Some(context.provider_uid())
        || resource
            .owner_generation
            .is_some_and(|generation| generation != context.provider_generation())
        || envelope.spec().provider_ref() != Some(context.process_provider_ref())
    {
        return false;
    }
    let Ok(process) = serde_json::from_slice::<d2b_contracts_resource::v3::process::ProcessSpec>(
        &envelope.spec().base().to_canonical_bytes(),
    ) else {
        return false;
    };
    process.execution().process_class()
        == d2b_contracts_resource::v3::process::ProcessClass::Controller
        && process.execution().execution_ref() == context.execution_ref()
}

fn controller_session_evidence_identity_check(
    resource_matches: bool,
    clearing: bool,
) -> Result<(), ResourceRuntimeError> {
    if !resource_matches && !clearing {
        return Err(ResourceRuntimeError::IdentityUnbound);
    }
    Ok(())
}

/// Whether one manager-served Process row is the row the bootstrap context
/// describes (G5). The manager row persists the authored metadata (`ownerRef`)
/// and the canonical spec envelope; the owner's committed uid/generation are
/// bound onto the ticket by the KTD7 identity seam, so the row-side owner
/// check is the authored reference - the same fact the row was ingested with.
fn controller_plane_resource_matches(
    context: &crate::process_provider_runtime::ControllerBootstrapContext,
    view: &ResourceView,
) -> bool {
    if view.key.zone != context.zone().as_str()
        || view.key.type_name != "Process"
        || view.key.name != context.process_ref().name().as_str()
        || view.generation != context.generation().get()
        || plane_controller_bridge::row_uid(&view.uid).as_ref() != Some(context.process_uid())
    {
        return false;
    }
    let Ok(metadata) = serde_json::from_slice::<Value>(&view.metadata) else {
        return false;
    };
    if metadata.get("ownerRef").and_then(Value::as_str)
        != Some(context.provider_owner_ref().to_canonical_string().as_str())
    {
        return false;
    }
    let Ok(spec) = serde_json::from_slice::<d2b_contracts_resource::v3::ResourceSpec>(&view.spec)
    else {
        return false;
    };
    if spec.provider_ref() != Some(context.process_provider_ref()) {
        return false;
    }
    let Ok(process) = serde_json::from_slice::<ProcessSpec>(&spec.base().to_canonical_bytes())
    else {
        return false;
    };
    process.execution().process_class()
        == d2b_contracts_resource::v3::process::ProcessClass::Controller
        && process.execution().execution_ref() == context.execution_ref()
}

/// U12: the same live controller-session evidence, exposed under the v3 Core
/// driver's effects trait, so the plane's `Provider` observation and drain
/// read the authoritative session (never a durable status copy).
impl crate::core_driver::CoreDriverEffects for ControllerSessionCoordinator {
    fn controller_session_evidence(
        &self,
        process_ref: &ResourceRef,
        process_uid: &ResourceUid,
        generation: ResourceGeneration,
    ) -> Option<Value> {
        <Self as LiveControllerSessionEvidence>::controller_session_evidence(
            self,
            process_ref,
            process_uid,
            generation,
        )
    }
}

impl LiveControllerSessionEvidence for ControllerSessionCoordinator {
    fn controller_session_evidence(
        &self,
        process_ref: &ResourceRef,
        process_uid: &ResourceUid,
        generation: ResourceGeneration,
    ) -> Option<Value> {
        let sessions = self.controller_sessions.lock().ok()?;
        let session = sessions.get(process_ref)?;
        // Liveness and identity are re-read per evaluation; a finished task
        // or a session bound to another row identity/generation is not
        // evidence for this row.
        if session.service_task.is_finished() {
            return None;
        }
        let context = &session.context;
        if context.process_uid() != process_uid || context.generation() != generation {
            return None;
        }
        Some(json!({
            "ready": true,
            "providerRef": context.provider_owner_ref().to_canonical_string(),
            "providerUid": context.provider_uid().as_str(),
            "providerGeneration": context.provider_generation().get(),
            "processRef": context.process_ref().to_canonical_string(),
            "processUid": context.process_uid().as_str(),
            "processGeneration": context.generation().get(),
            "controllerGeneration": context.controller_generation().get(),
            "sessionGeneration": session.binding.session_generation().get(),
            "artifactReady": true,
            "descriptorReady": true,
            "registrationReady": true,
        }))
    }
}

fn committed_provider_spec(
    zone: &ZoneId,
    resource: &StoredResource,
    expected_ref: &ResourceRef,
) -> Result<
    (
        ProviderSpec,
        ResourceUid,
        ResourceGeneration,
        ZoneRevision,
        String,
    ),
    ResourceRuntimeError,
> {
    if &resource.zone != zone
        || &resource.resource_ref != expected_ref
        || resource.generation.get() == 0
        || resource.revision.get() == 0
    {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    if envelope.resource_type().as_str() != "Provider"
        || envelope.metadata().zone() != zone
        || envelope.metadata().uid() != &resource.uid
        || envelope.metadata().generation() != resource.generation
        || envelope.metadata().revision() != resource.revision
        || envelope
            .digest()
            .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?
            != resource.payload_digest
    {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let spec = serde_json::from_slice::<ProviderSpec>(&envelope.spec().base().to_canonical_bytes())
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    // The trusted bundle/resource compiler has already resolved and
    // integrity-pinned the Provider artifact.  Runtime composition is bound
    // to the canonical Provider ResourceRef, not to a package name that may
    // vary between deployments (including hermetic acceptance artifacts).
    if spec.artifact_id().as_str().is_empty() {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    Ok((
        spec,
        resource.uid.clone(),
        resource.generation,
        resource.revision,
        resource.payload_digest.clone(),
    ))
}

fn parse_committed_clipboard_configuration(
    zone: &ZoneId,
    resource: &StoredResource,
) -> Result<CommittedClipboardProviderConfiguration, ResourceRuntimeError> {
    let expected_ref = ResourceRef::parse("Provider/clipboard-wayland")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let (spec, resource_uid, resource_generation, resource_revision, provenance_digest) =
        committed_provider_spec(zone, resource, &expected_ref)?;
    let wire =
        serde_json::from_slice::<ClipboardProviderConfigWire>(&spec.config().to_canonical_bytes())
            .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    if wire.controller_execution_ref != wire.host_execution_ref
        || wire.controller_execution_ref.resource_type().as_str() != "Host"
        || wire.host_execution_ref.resource_type().as_str() != "Host"
        || wire.host_user_ref.resource_type().as_str() != "User"
        || wire.display_wayland_ref.to_canonical_string() != "Provider/display-wayland"
        || wire.policy.cross_zone.enable
        || wire.guest_sources.is_empty()
    {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let mut guest_sources = BTreeSet::new();
    for source in wire.guest_sources {
        if source.guest_ref.resource_type().as_str() != "Guest"
            || !guest_sources.insert(source.guest_ref)
        {
            return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
        }
    }
    let policy = ClipboardPolicy::new_with_fd_write_timeout_seconds(
        wire.policy.allow_host_capture,
        wire.policy.allow_guest_capture,
        wire.policy.require_picker_for_paste,
        wire.policy.suppress_echo,
        false,
        wire.caps.max_history_entries,
        wire.caps.max_item_bytes,
        wire.caps.max_total_bytes,
        wire.caps.max_concurrent_fds,
        wire.caps.max_guest_rate_per_min,
        wire.caps.fd_write_timeout_seconds,
    )
    .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    Ok(CommittedClipboardProviderConfiguration {
        policy,
        audit_capacity: wire.caps.max_history_entries,
        host_execution_ref: wire.host_execution_ref,
        host_user_ref: wire.host_user_ref,
        display_wayland_ref: wire.display_wayland_ref,
        guest_sources,
        resource_uid,
        resource_generation,
        resource_revision,
        provenance_digest,
    })
}

fn parse_committed_notification_configuration(
    zone: &ZoneId,
    resource: &StoredResource,
) -> Result<CommittedNotificationProviderConfiguration, ResourceRuntimeError> {
    let expected_ref = ResourceRef::parse("Provider/notification-desktop")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let (spec, resource_uid, resource_generation, resource_revision, provenance_digest) =
        committed_provider_spec(zone, resource, &expected_ref)?;
    let wire = serde_json::from_slice::<NotificationProviderConfigWire>(
        &spec.config().to_canonical_bytes(),
    )
    .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    if wire.controller_execution_ref != wire.host_execution_ref
        || wire.controller_execution_ref.resource_type().as_str() != "Host"
        || wire.host_execution_ref.resource_type().as_str() != "Host"
        || wire.host_user_ref.resource_type().as_str() != "User"
        || wire.display_wayland_ref.to_canonical_string() != "Provider/display-wayland"
        || wire.guest_sources.is_empty()
    {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let mut sources = Vec::with_capacity(wire.guest_sources.len());
    for source in wire.guest_sources {
        sources.push(
            GuestSourceConfig::new(source.guest_ref, zone.clone(), source.categories)
                .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?,
        );
    }
    let config = NotificationProviderConfig::new(sources)
        .and_then(|config| {
            config.with_host_binding(wire.host_execution_ref.clone(), wire.host_user_ref)
        })
        .and_then(|config| config.with_display_wayland_ref(Some(wire.display_wayland_ref)))
        .and_then(|config| config.with_max_pending_notifications(wire.max_pending_notifications))
        .and_then(|config| config.with_action_nonce_ttl_secs(wire.action_nonce_ttl_secs))
        .and_then(|config| config.with_action_nonce_store_size(wire.action_nonce_store_size))
        .and_then(|config| config.with_acknowledge_timeout_secs(wire.acknowledge_timeout_secs))
        .map(|config| {
            config
                .with_dbus_sink_enabled(wire.dbus_sink_enabled)
                .with_observer_enabled(wire.observer_enabled)
        })
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    Ok(CommittedNotificationProviderConfiguration {
        config,
        host_execution_ref: wire.host_execution_ref,
        resource_uid,
        resource_generation,
        resource_revision,
        provenance_digest,
    })
}

async fn system_core_startup_result(
    plane: &dyn ControllerPlaneView,
    policy: &PolicySnapshot,
) -> Result<SystemCoreReconcileResult, ResourceRuntimeError> {
    let views = plane
        .all_rows()
        .await
        .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
    let resources = views
        .iter()
        .map(|view| {
            d2b_resource_api::manager_backend::manager_row_stored(view)
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let total_resource_count = resources.len().min(u32::MAX as usize) as u32;
    let active_configuration_generation = policy.active_configuration_revision.get();
    let cleanup_pending_count = resources
        .iter()
        .filter(|resource| configuration_cleanup_pending(resource, active_configuration_generation))
        .count()
        .min(u32::MAX as usize) as u32;
    // Startup readiness is established by the already authenticated Core
    // session; the shared runner immediately replaces this provisional
    // projection with persisted Host/User observations.
    Ok(SystemCoreReconcileResult {
        core_phase: ResourcePhase::Ready,
        host_phase: HandlerPhase::Ready,
        user_phase: HandlerPhase::Ready,
        total_resource_count,
        generation_cleanup_pending: cleanup_pending_count > 0,
        cleanup_pending_count,
    })
}



/// Validate the target shape of one public resource request.
///
/// U14: with one plane a request no longer routes, but the declared type must
/// still agree with the reference the handler will actually act on - mutating
/// one reference under another's declared type is refused outright.
fn validate_public_request_target(
    request: &Value,
    method: &str,
) -> Result<(), ResourceRuntimeError> {
    if method == "List" {
        parse_list_request(request)?;
    } else if method == "Create" {
        declared_resource_type(request).ok_or(ResourceRuntimeError::RequestInvalid)?;
    } else {
        let target = public_target_ref(request)?;
        if let Some(declared) = declared_resource_type(request)
            && declared != target.resource_type().as_str()
        {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
    }
    Ok(())
}

/// The resource type a request declares, if any. Only Create declares on this
/// field; every other method derives its target from `resourceRef`.
fn declared_resource_type(request: &Value) -> Option<&str> {
    request
        .get("resourceType")
        .or_else(|| request.get("type"))
        .and_then(Value::as_str)
}

/// The manager plane's store-seal identity: a distinct slot from the
/// guest-target store's, so the manager-plane issuer can never pair with
/// another backend's acceptor that reuses slot 0 (the redb Zone store this
/// slot was originally kept apart from is retired in U14). The uid is inert -
/// it only pairs this Zone's manager-plane issuer with the manager backend's
/// acceptor - and the Zone authority's uid is reused when the runtime has one.
fn manager_plane_seal_identity(
    zone: &ZoneId,
    zone_uid: Option<ResourceUid>,
) -> Result<d2b_contracts_resource::v3::StoreSealIdentity, ResourceRuntimeError> {
    const MANAGER_PLANE_SEAL_SLOT: u32 = 1;
    const MANAGER_PLANE_SEAL_UID: &str = "00000000-0000-4000-8000-000000000001";
    let slot = d2b_contracts_resource::v3::StoreSlot::new(MANAGER_PLANE_SEAL_SLOT)
        .map_err(|_| ResourceRuntimeError::StoreSealUnavailable)?;
    let uid = match zone_uid {
        Some(uid) => uid,
        None => ResourceUid::parse(MANAGER_PLANE_SEAL_UID.to_owned())
            .map_err(|_| ResourceRuntimeError::StoreSealUnavailable)?,
    };
    Ok(d2b_contracts_resource::v3::StoreSealIdentity::new(
        slot,
        zone.clone(),
        uid,
    ))
}

async fn public_create_request(
    runtime: &ZoneResourceRuntime,
    request: &Value,
    operation_id: &str,
) -> Result<wire::CreateRequest, ResourceRuntimeError> {
    let resource_type = request
        .get("resourceType")
        .and_then(Value::as_str)
        .ok_or(ResourceRuntimeError::RequestInvalid)
        .and_then(|value| {
            ResourceTypeName::parse(value.to_owned())
                .map_err(|_| ResourceRuntimeError::RequestInvalid)
        })?;
    let input = request
        .get("resource")
        .or_else(|| request.get("spec"))
        .ok_or(ResourceRuntimeError::RequestInvalid)?;
    let (name, spec) = if is_resource_envelope(input) {
        let name = input
            .pointer("/metadata/name")
            .and_then(Value::as_str)
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        let spec = input
            .get("spec")
            .cloned()
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        (name.to_owned(), spec)
    } else {
        let name = request
            .get("resourceName")
            .and_then(Value::as_str)
            .or_else(|| {
                input
                    .get("metadata")
                    .and_then(|value| value.get("name"))
                    .and_then(Value::as_str)
            })
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        (name.to_owned(), input.clone())
    };
    let payload = public_create_payload(
        runtime,
        &resource_type,
        &name,
        &spec,
        request.get("ownerRef").and_then(Value::as_str),
    )
    .await?;
    let identity = public_identity(runtime, &resource_type, &name, None, None, None);
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_CREATE);
    mutation.target = protobuf::MessageField::some(identity.clone());
    mutation.precondition = protobuf::MessageField::some(create_precondition());
    mutation.resource = protobuf::MessageField::some(public_resource_body(identity, payload)?);
    apply_public_mutation_options(&mut mutation, request)?;
    let mut result = wire::CreateRequest::new();
    result.meta = protobuf::MessageField::some(public_request_meta(operation_id));
    result.mutation = protobuf::MessageField::some(mutation);
    Ok(result)
}

async fn public_update_spec_request<S>(
    client: &ResourceApiClient<S, UnavailableUpgradeDispatcher>,
    runtime: &ZoneResourceRuntime,
    request: &Value,
    operation_id: &str,
) -> Result<wire::UpdateSpecRequest, ResourceRuntimeError>
where
    S: d2b_resource_api::ResourceStoreBackend,
{
    let target = public_target_ref(request)?;
    let current = public_get_resource(client, runtime, &target, operation_id).await?;
    public_update_spec_request_from_current(runtime, request, operation_id, &target, current)
}

fn public_update_spec_request_from_current(
    runtime: &ZoneResourceRuntime,
    request: &Value,
    operation_id: &str,
    target: &ResourceRef,
    current: Value,
) -> Result<wire::UpdateSpecRequest, ResourceRuntimeError> {
    let spec = request
        .get("spec")
        .cloned()
        .ok_or(ResourceRuntimeError::RequestInvalid)?;
    let payload = replace_public_field(&current, "spec", spec)?;
    let current_uid = public_uid(&current)?;
    let current_revision = public_revision(&current)?;
    let expected_revision = public_expected_revision(request)?.unwrap_or(current_revision);
    let identity = public_identity(
        runtime,
        target.resource_type(),
        target.name().as_str(),
        Some(&current_uid),
        Some(public_generation(&current)?),
        Some(expected_revision),
    );
    let mut mutation = public_body_mutation(
        wire::MutationKind::MUTATION_KIND_UPDATE_SPEC,
        identity,
        exact_public_precondition(expected_revision, &current_uid),
        payload,
    )?;
    apply_public_mutation_options(&mut mutation, request)?;
    let mut result = wire::UpdateSpecRequest::new();
    result.meta = protobuf::MessageField::some(public_request_meta(operation_id));
    result.mutation = protobuf::MessageField::some(mutation);
    Ok(result)
}

async fn public_update_status_request<S>(
    client: &ResourceApiClient<S, UnavailableUpgradeDispatcher>,
    runtime: &ZoneResourceRuntime,
    request: &Value,
    operation_id: &str,
) -> Result<wire::UpdateStatusRequest, ResourceRuntimeError>
where
    S: d2b_resource_api::ResourceStoreBackend,
{
    let target = public_target_ref(request)?;
    let current = public_get_resource(client, runtime, &target, operation_id).await?;
    public_update_status_request_from_current(runtime, request, operation_id, &target, current)
}

fn public_update_status_request_from_current(
    runtime: &ZoneResourceRuntime,
    request: &Value,
    operation_id: &str,
    target: &ResourceRef,
    current: Value,
) -> Result<wire::UpdateStatusRequest, ResourceRuntimeError> {
    let status = request
        .get("status")
        .cloned()
        .or_else(|| {
            request
                .get("resource")
                .and_then(|value| value.get("status"))
                .cloned()
        })
        .ok_or(ResourceRuntimeError::RequestInvalid)?;
    let payload = replace_public_field(&current, "status", status)?;
    let current_uid = public_uid(&current)?;
    let current_revision = public_revision(&current)?;
    let expected_revision = public_expected_revision(request)?.unwrap_or(current_revision);
    let identity = public_identity(
        runtime,
        target.resource_type(),
        target.name().as_str(),
        Some(&current_uid),
        Some(public_generation(&current)?),
        Some(expected_revision),
    );
    let mutation = public_body_mutation(
        wire::MutationKind::MUTATION_KIND_UPDATE_STATUS,
        identity,
        exact_public_precondition(expected_revision, &current_uid),
        payload,
    )?;
    let mut result = wire::UpdateStatusRequest::new();
    result.meta = protobuf::MessageField::some(public_request_meta(operation_id));
    result.mutation = protobuf::MessageField::some(mutation);
    Ok(result)
}

fn public_update_finalizers_request(
    runtime: &ZoneResourceRuntime,
    request: &Value,
    operation_id: &str,
) -> Result<wire::UpdateFinalizersRequest, ResourceRuntimeError> {
    let target = public_target_ref(request)?;
    let uid = request
        .get("uid")
        .and_then(Value::as_str)
        .map(|value| ResourceUid::parse(value.to_owned()))
        .transpose()
        .map_err(|_| ResourceRuntimeError::RequestInvalid)?;
    let expected_revision =
        public_expected_revision(request)?.ok_or(ResourceRuntimeError::RequestInvalid)?;
    let uid = uid.ok_or(ResourceRuntimeError::RequestInvalid)?;
    let identity = public_identity(
        runtime,
        target.resource_type(),
        target.name().as_str(),
        Some(&uid),
        None,
        Some(expected_revision),
    );
    let mut mutation = wire::Mutation::new();
    mutation.kind =
        protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_FINALIZERS);
    mutation.target = protobuf::MessageField::some(identity);
    mutation.precondition =
        protobuf::MessageField::some(exact_public_precondition(expected_revision, &uid));
    mutation.add_finalizers = public_string_array(request, "addFinalizers")?;
    mutation.remove_finalizers = public_string_array(request, "removeFinalizers")?;
    if mutation.add_finalizers.is_empty() && mutation.remove_finalizers.is_empty() {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    let mut result = wire::UpdateFinalizersRequest::new();
    result.meta = protobuf::MessageField::some(public_request_meta(operation_id));
    result.mutation = protobuf::MessageField::some(mutation);
    Ok(result)
}

fn public_delete_request_from_current(
    runtime: &ZoneResourceRuntime,
    request: &Value,
    operation_id: &str,
    current: Value,
) -> Result<wire::DeleteRequest, ResourceRuntimeError> {
    let target = public_target_ref(request)?;
    let expected_revision = public_expected_revision(request)?;
    let mut uid = request
        .get("uid")
        .and_then(Value::as_str)
        .map(|value| ResourceUid::parse(value.to_owned()))
        .transpose()
        .map_err(|_| ResourceRuntimeError::RequestInvalid)?;
    if uid.is_none() && expected_revision.is_some() {
        // The precondition binds the exact stable identity, so it must come
        // from the same plane the delete commits against: a converted type
        // rides the manager-backed service, and the legacy store carries a
        // different uid for the same key during the Phase A dual-model.
        uid = Some(public_uid(&current)?);
    }
    let identity = public_identity(
        runtime,
        target.resource_type(),
        target.name().as_str(),
        uid.as_ref(),
        None,
        expected_revision,
    );
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_DELETE);
    mutation.target = protobuf::MessageField::some(identity.clone());
    let precondition = match expected_revision {
        Some(revision) => {
            let uid = uid.ok_or(ResourceRuntimeError::RequestInvalid)?;
            exact_public_precondition(revision, &uid)
        }
        None => {
            let mut precondition = wire::Precondition::new();
            precondition.kind = protobuf::EnumOrUnknown::new(
                wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION,
            );
            precondition.expected_revision = Some(1);
            precondition
        }
    };
    mutation.precondition = protobuf::MessageField::some(precondition);
    apply_public_mutation_options(&mut mutation, request)?;
    let mut result = wire::DeleteRequest::new();
    result.meta = protobuf::MessageField::some(public_request_meta(operation_id));
    result.mutation = protobuf::MessageField::some(mutation);
    Ok(result)
}

fn public_target_ref(request: &Value) -> Result<ResourceRef, ResourceRuntimeError> {
    request
        .get("resourceRef")
        .and_then(Value::as_str)
        .ok_or(ResourceRuntimeError::RequestInvalid)
        .and_then(|value| {
            ResourceRef::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)
        })
}

fn public_expected_revision(request: &Value) -> Result<Option<u64>, ResourceRuntimeError> {
    let Some(value) = request.get("expectedRevision") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        .filter(|value| *value > 0)
        .ok_or(ResourceRuntimeError::RequestInvalid)?;
    Ok(Some(value))
}

fn public_uid(resource: &Value) -> Result<ResourceUid, ResourceRuntimeError> {
    resource
        .pointer("/metadata/uid")
        .and_then(Value::as_str)
        .ok_or(ResourceRuntimeError::ResponseInvalid)
        .and_then(|value| {
            ResourceUid::parse(value.to_owned()).map_err(|_| ResourceRuntimeError::ResponseInvalid)
        })
}

fn public_revision(resource: &Value) -> Result<u64, ResourceRuntimeError> {
    resource
        .pointer("/metadata/revision")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or(ResourceRuntimeError::ResponseInvalid)
}

fn public_generation(resource: &Value) -> Result<u64, ResourceRuntimeError> {
    resource
        .pointer("/metadata/generation")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or(ResourceRuntimeError::ResponseInvalid)
}

async fn public_get_resource<S>(
    client: &ResourceApiClient<S, UnavailableUpgradeDispatcher>,
    runtime: &ZoneResourceRuntime,
    target: &ResourceRef,
    operation_id: &str,
) -> Result<Value, ResourceRuntimeError>
where
    S: d2b_resource_api::ResourceStoreBackend,
{
    let mut meta = public_request_meta(operation_id);
    meta.deadline_ms = 30_000;
    let response = client
        .get(wire::GetRequest {
            meta: protobuf::MessageField::some(meta),
            target: protobuf::MessageField::some(public_identity(
                runtime,
                target.resource_type(),
                target.name().as_str(),
                None,
                None,
                None,
            )),
            projection: {
                let mut projection = wire::Projection::new();
                projection.kind =
                    protobuf::EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
                protobuf::MessageField::some(projection)
            },
            special_fields: protobuf::SpecialFields::new(),
        })
        .await;
    if response.error.is_some() {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    let resource = response
        .resource
        .as_ref()
        .ok_or(ResourceRuntimeError::ResponseInvalid)?;
    encode_public_resource(resource)
}

async fn gateway_get_resource(
    client: &d2b_resource_api::generated::d2b_resource_v3_ttrpc::ResourceServiceClient,
    runtime: &ZoneResourceRuntime,
    target: &ResourceRef,
    operation_id: &str,
) -> Result<Value, ResourceRuntimeError> {
    let mut meta = public_request_meta(operation_id);
    meta.deadline_ms = 30_000;
    let response = client
        .get(
            ttrpc::context::Context::default(),
            &wire::GetRequest {
                meta: protobuf::MessageField::some(meta),
                target: protobuf::MessageField::some(public_identity(
                    runtime,
                    target.resource_type(),
                    target.name().as_str(),
                    None,
                    None,
                    None,
                )),
                projection: {
                    let mut projection = wire::Projection::new();
                    projection.kind =
                        protobuf::EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
                    protobuf::MessageField::some(projection)
                },
                special_fields: protobuf::SpecialFields::new(),
            },
        )
        .await
        .map_err(|_| ResourceRuntimeError::ProviderPathUnavailable)?;
    d2bd_runtime::resource_runtime_support::encode_public_get_response(response)
}

fn public_identity(
    runtime: &ZoneResourceRuntime,
    resource_type: &ResourceTypeName,
    name: &str,
    uid: Option<&ResourceUid>,
    generation: Option<u64>,
    revision: Option<u64>,
) -> wire::ResourceIdentity {
    wire::ResourceIdentity {
        zone: runtime.zone.to_canonical_string(),
        resource_type: resource_type.to_canonical_string(),
        name: name.to_owned(),
        uid: uid.map(|value| value.as_str().to_owned()),
        generation,
        revision,
        special_fields: protobuf::SpecialFields::new(),
    }
}

fn create_precondition() -> wire::Precondition {
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT);
    precondition
}

fn exact_public_precondition(revision: u64, uid: &ResourceUid) -> wire::Precondition {
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(revision);
    precondition.expected_uid = Some(uid.as_str().to_owned());
    precondition
}

fn public_body_mutation(
    kind: wire::MutationKind,
    identity: wire::ResourceIdentity,
    precondition: wire::Precondition,
    payload: Vec<u8>,
) -> Result<wire::Mutation, ResourceRuntimeError> {
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(kind);
    mutation.target = protobuf::MessageField::some(identity.clone());
    mutation.precondition = protobuf::MessageField::some(precondition);
    mutation.resource = protobuf::MessageField::some(public_resource_body(identity, payload)?);
    Ok(mutation)
}

fn public_resource_body(
    identity: wire::ResourceIdentity,
    payload: Vec<u8>,
) -> Result<wire::ResourceEnvelopeBytes, ResourceRuntimeError> {
    let envelope =
        ResourceEnvelope::from_json(&payload).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
    let digest = envelope
        .digest()
        .map_err(|_| ResourceRuntimeError::RequestInvalid)?;
    let mut body = wire::ResourceEnvelopeBytes::new();
    body.identity = protobuf::MessageField::some(identity);
    body.canonical_json = payload;
    body.payload_digest = digest;
    Ok(body)
}

fn apply_public_mutation_options(
    mutation: &mut wire::Mutation,
    request: &Value,
) -> Result<(), ResourceRuntimeError> {
    mutation.wait_for_reconcile = request
        .get("waitForReconcile")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    mutation.reconcile_deadline_ms = request
        .get("reconcileDeadlineMs")
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .unwrap_or(0);
    if !mutation.wait_for_reconcile && mutation.reconcile_deadline_ms != 0 {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    Ok(())
}

fn public_string_array(request: &Value, field: &str) -> Result<Vec<String>, ResourceRuntimeError> {
    let Some(value) = request.get(field) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or(ResourceRuntimeError::RequestInvalid)?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or(ResourceRuntimeError::RequestInvalid)
        })
        .collect()
}

fn is_resource_envelope(value: &Value) -> bool {
    value.get("metadata").is_some()
        && value.get("spec").is_some()
        && (value.get("type").is_some() || value.get("apiVersion").is_some())
}

async fn public_create_payload(
    runtime: &ZoneResourceRuntime,
    resource_type: &ResourceTypeName,
    name: &str,
    spec: &Value,
    owner_ref: Option<&str>,
) -> Result<Vec<u8>, ResourceRuntimeError> {
    let metadata = runtime.committed_policy_snapshot();
    let timestamp = current_status_timestamp();
    let value = json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": resource_type.to_canonical_string(),
        "metadata": {
            "configurationGeneration": metadata.active_configuration_revision.get(),
            "createdAt": timestamp,
            "deletionRequestedAt": null,
            "finalizers": [],
            "generation": 1,
            "managedBy": "api",
            "name": name,
            "ownerRef": owner_ref,
            "revision": 1,
            "updatedAt": timestamp,
            "zone": runtime.zone.as_str()
        },
        "spec": spec,
        "status": {
            "completedAt": null,
            "conditions": [],
            "lastReconciledAt": null,
            "observedGeneration": 0,
            "outcome": null,
            "phase": "Pending",
            "resource": {},
            "startedAt": null,
            "update": {
                "dependencies": {"count": 0, "refs": []},
                "disruption": "None",
                "lastAssessedAt": 0,
                "observedGeneration": 0,
                "operationId": null,
                "owned": {"count": 0, "refs": []},
                "preserveState": true,
                "reasons": [],
                "state": "Unknown",
                "targetGeneration": 1
            }
        }
    });
    if value.get("spec").and_then(Value::as_object).is_none() {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    let bytes = serde_json::to_vec(&value).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
    let canonical = CanonicalJsonValue::parse(&bytes)
        .map_err(|_| ResourceRuntimeError::RequestInvalid)?
        .to_canonical_bytes();
    ResourceEnvelope::from_json(&canonical).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
    Ok(canonical)
}

fn replace_public_field(
    current: &Value,
    field: &str,
    replacement: Value,
) -> Result<Vec<u8>, ResourceRuntimeError> {
    let mut value = current.clone();
    value
        .as_object_mut()
        .and_then(|root| root.get_mut(field))
        .map(|field_value| *field_value = replacement)
        .ok_or(ResourceRuntimeError::RequestInvalid)?;
    let bytes = serde_json::to_vec(&value).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
    let canonical = CanonicalJsonValue::parse(&bytes)
        .map_err(|_| ResourceRuntimeError::RequestInvalid)?
        .to_canonical_bytes();
    ResourceEnvelope::from_json(&canonical).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
    Ok(canonical)
}

fn encode_public_create_response(
    response: wire::CreateResponse,
) -> Result<Value, ResourceRuntimeError> {
    encode_public_mutation_response(
        response.error.as_ref(),
        response.resource.as_ref(),
        None,
        response.revision,
        Some(
            response
                .disposition
                .enum_value()
                .unwrap_or(wire::ReconcileDisposition::RECONCILE_DISPOSITION_UNSPECIFIED),
        ),
        Some(
            response
                .status_persistence
                .enum_value()
                .unwrap_or(wire::StatusPersistence::STATUS_PERSISTENCE_UNSPECIFIED),
        ),
        response.last_persisted_status_revision,
        response.reconcile_projection.as_ref(),
    )
}

fn encode_public_update_spec_response(
    response: wire::UpdateSpecResponse,
) -> Result<Value, ResourceRuntimeError> {
    encode_public_mutation_response(
        response.error.as_ref(),
        response.resource.as_ref(),
        None,
        response.revision,
        Some(
            response
                .disposition
                .enum_value()
                .unwrap_or(wire::ReconcileDisposition::RECONCILE_DISPOSITION_UNSPECIFIED),
        ),
        Some(
            response
                .status_persistence
                .enum_value()
                .unwrap_or(wire::StatusPersistence::STATUS_PERSISTENCE_UNSPECIFIED),
        ),
        response.last_persisted_status_revision,
        response.reconcile_projection.as_ref(),
    )
}

fn encode_public_update_status_response(
    response: wire::UpdateStatusResponse,
) -> Result<Value, ResourceRuntimeError> {
    encode_public_mutation_response(
        response.error.as_ref(),
        response.resource.as_ref(),
        None,
        response.revision,
        None,
        None,
        None,
        None,
    )
}

fn encode_public_update_finalizers_response(
    response: wire::UpdateFinalizersResponse,
) -> Result<Value, ResourceRuntimeError> {
    encode_public_mutation_response(
        response.error.as_ref(),
        response.resource.as_ref(),
        None,
        response.revision,
        None,
        None,
        None,
        None,
    )
}

fn encode_public_delete_response(
    response: wire::DeleteResponse,
) -> Result<Value, ResourceRuntimeError> {
    encode_public_mutation_response(
        response.error.as_ref(),
        None,
        response.resource.as_ref(),
        response.revision,
        Some(
            response
                .disposition
                .enum_value()
                .unwrap_or(wire::ReconcileDisposition::RECONCILE_DISPOSITION_UNSPECIFIED),
        ),
        None,
        None,
        None,
    )
}

fn encode_public_mutation_response(
    error: Option<&wire::ResourceError>,
    resource: Option<&wire::ResourceEnvelopeBytes>,
    identity: Option<&wire::ResourceIdentity>,
    revision: u64,
    disposition: Option<wire::ReconcileDisposition>,
    status_persistence: Option<wire::StatusPersistence>,
    last_persisted_status_revision: Option<u64>,
    reconcile_projection: Option<&wire::ResourceEnvelopeBytes>,
) -> Result<Value, ResourceRuntimeError> {
    if let Some(error) = error {
        tracing::warn!(
            kind = ?error.kind,
            retry_class = ?error.retry_class,
            retry_after_ms = ?error.retry_after_ms,
            reason = %error.reason,
            "public Resource mutation returned an API error",
        );
        return Ok(d2bd_runtime::resource_runtime_support::public_api_error(
            error,
        ));
    }
    let mut body = serde_json::Map::new();
    if let Some(resource) = resource {
        body.insert("resource".to_owned(), encode_public_resource(resource)?);
    }
    if let Some(identity) = identity {
        body.insert(
            "resourceRef".to_owned(),
            Value::String(format!("{}/{}", identity.resource_type, identity.name)),
        );
    }
    body.insert("revision".to_owned(), Value::from(revision));
    if let Some(disposition) = disposition
        .filter(|value| *value != wire::ReconcileDisposition::RECONCILE_DISPOSITION_UNSPECIFIED)
    {
        body.insert(
            "disposition".to_owned(),
            Value::String(
                match disposition {
                    wire::ReconcileDisposition::RECONCILE_DISPOSITION_CONVERGED => "Converged",
                    wire::ReconcileDisposition::RECONCILE_DISPOSITION_PROGRESSING => "Progressing",
                    wire::ReconcileDisposition::RECONCILE_DISPOSITION_BLOCKED => "Blocked",
                    wire::ReconcileDisposition::RECONCILE_DISPOSITION_UPGRADE_REQUIRED => {
                        "UpgradeRequired"
                    }
                    wire::ReconcileDisposition::RECONCILE_DISPOSITION_FAILED => "Failed",
                    wire::ReconcileDisposition::RECONCILE_DISPOSITION_UNSPECIFIED => "Unspecified",
                }
                .to_owned(),
            ),
        );
    }
    if let Some(status_persistence) = status_persistence
        .filter(|value| *value != wire::StatusPersistence::STATUS_PERSISTENCE_UNSPECIFIED)
    {
        body.insert(
            "statusPersistence".to_owned(),
            Value::String(
                match status_persistence {
                    wire::StatusPersistence::STATUS_PERSISTENCE_PENDING => "pending",
                    wire::StatusPersistence::STATUS_PERSISTENCE_COMMITTED => "committed",
                    wire::StatusPersistence::STATUS_PERSISTENCE_UNSPECIFIED => "unspecified",
                }
                .to_owned(),
            ),
        );
    }
    if let Some(revision) = last_persisted_status_revision {
        body.insert(
            "lastPersistedStatusRevision".to_owned(),
            Value::from(revision),
        );
    }
    if let Some(projection) = reconcile_projection {
        body.insert(
            "reconcileProjection".to_owned(),
            encode_public_resource(projection)?,
        );
    }
    Ok(Value::Object(body))
}

/// Root-supervisor ownership index for all local Network host effects.
///
/// The index is shared by every Zone runtime in one daemon. Callers must
/// observe the host before admission; the index itself only commits a
/// candidate after every CIDR, interface, and route collision check passes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NetworkAdmissionOwnerKey {
    zone_uid: ResourceUid,
    network_uid: ResourceUid,
}

impl NetworkAdmissionOwnerKey {
    fn from_key(key: &NetworkAdmissionKey) -> Self {
        Self {
            zone_uid: key.zone_uid().clone(),
            network_uid: key.network_uid().clone(),
        }
    }
}

#[derive(Default)]
pub struct HostNetworkAdmissionIndex {
    entries: BTreeMap<NetworkAdmissionOwnerKey, NetworkAdmissionIntent>,
    retired: BTreeMap<NetworkAdmissionOwnerKey, BTreeSet<NetworkAdmissionKey>>,
    released_floors: BTreeMap<NetworkAdmissionOwnerKey, (u64, u64)>,
}

fn route_conflicts(desired: &RouteTuple, occupied: &RouteTuple) -> bool {
    if desired.table() != occupied.table() {
        return false;
    }
    if desired.destination() == occupied.destination() {
        return true;
    }
    let Some(desired_cidr) =
        d2b_contracts_resource::v3::network::Ipv4Cidr::parse(desired.destination().to_owned()).ok()
    else {
        return false;
    };
    let Some(occupied_cidr) =
        d2b_contracts_resource::v3::network::Ipv4Cidr::parse(occupied.destination().to_owned())
            .ok()
    else {
        return false;
    };
    d2b_contracts_resource::v3::network::cidr_overlaps(&desired_cidr, &occupied_cidr)
}

impl core::fmt::Debug for HostNetworkAdmissionIndex {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("HostNetworkAdmissionIndex")
            .field("entry_count", &self.entries.len())
            .finish()
    }
}

impl HostNetworkAdmissionIndex {
    /// Admit one Network atomically against the observed host and siblings.
    pub fn admit(
        &mut self,
        intent: NetworkAdmissionIntent,
        occupancy: &HostNetworkOccupancy,
    ) -> Result<NetworkAdmissionProof, NetworkEffectError> {
        let key = intent.key().clone();
        let owner = NetworkAdmissionOwnerKey::from_key(&key);
        if self
            .retired
            .get(&owner)
            .is_some_and(|retired| retired.contains(&key))
        {
            return Err(NetworkEffectError::NetworkAdmissionMismatch);
        }
        if !self.entries.contains_key(&owner)
            && self.released_floors.get(&owner).is_some_and(|floor| {
                key.network_generation().get() < floor.0
                    || key.attachment_generation().get() < floor.1
            })
        {
            return Err(NetworkEffectError::NetworkAdmissionMismatch);
        }
        let existing = self.entries.get(&owner).cloned();
        if let Some(current) = existing.as_ref() {
            if current.key() == &key {
                if current != &intent {
                    return Err(NetworkEffectError::NetworkAdmissionMismatch);
                }
                return Ok(current.proof());
            }
            if current.key() != &key && !is_current_or_newer_admission(current.key(), &key) {
                return Err(NetworkEffectError::NetworkAdmissionMismatch);
            }
        }
        let owner_intents = existing
            .as_ref()
            .into_iter()
            .chain(std::iter::once(&intent))
            .collect::<Vec<_>>();

        if self.entries.iter().any(|(candidate, existing)| {
            if candidate == &owner {
                return false;
            }
            intent.cidrs().iter().any(|cidr| {
                existing
                    .cidrs()
                    .iter()
                    .any(|peer| d2b_contracts_resource::v3::network::cidr_overlaps(cidr, peer))
            })
        }) || intent.cidrs().iter().any(|cidr| {
            occupancy.cidrs().iter().any(|peer| {
                d2b_contracts_resource::v3::network::cidr_overlaps(cidr, peer)
                    && !cidr_is_self_owned(&owner_intents, occupancy, peer)
            })
        }) {
            return Err(NetworkEffectError::CidrConflict);
        }

        if intent.interface_names().iter().any(|ifname| {
            occupancy.interface_names().iter().any(|occupied| {
                occupied == ifname && !interface_is_self_owned(&owner_intents, occupancy, occupied)
            }) || self.entries.iter().any(|(candidate, existing)| {
                if candidate == &owner {
                    return false;
                }
                existing
                    .interface_names()
                    .iter()
                    .any(|candidate| candidate == ifname)
            })
        }) {
            return Err(NetworkEffectError::NetworkInterfaceCollision);
        }
        if let Some((parent, mode, sharing)) = intent.external_nic() {
            let mut claims = Vec::new();
            for (candidate, existing) in &self.entries {
                if candidate == &owner {
                    continue;
                }
                let Some((existing_parent, existing_mode, existing_sharing)) =
                    existing.external_nic()
                else {
                    continue;
                };
                if existing_parent != parent {
                    continue;
                }
                claims.push(ExternalNicClaim::new(
                    existing.key().zone_uid().clone(),
                    existing_mode,
                    existing_sharing,
                ));
            }
            claims.push(ExternalNicClaim::new(key.zone_uid().clone(), mode, sharing));
            match admit_external_nic_claims(&claims, 64) {
                Ok(()) => {}
                Err(ExternalNicAdmissionError::ExternalPhysicalNicCrossZoneL2) => {
                    return Err(NetworkEffectError::CrossZoneL2);
                }
                Err(ExternalNicAdmissionError::ExternalPhysicalNicConflict) => {
                    return Err(NetworkEffectError::NetworkAdmissionConflict);
                }
            }
        }

        if intent.routes().iter().any(|route| {
            occupancy.routes().iter().any(|occupied| {
                route_conflicts(route, occupied)
                    && !route_is_self_owned(&owner_intents, occupancy, occupied)
            })
        }) || intent.routes().iter().any(|route| {
            self.entries.iter().any(|(candidate, existing)| {
                if candidate == &owner {
                    return false;
                }
                existing
                    .routes()
                    .iter()
                    .any(|candidate| route_conflicts(route, candidate))
            })
        }) {
            return Err(NetworkEffectError::NetworkRouteCollision);
        }

        let proof = intent.proof();
        if let Some(current) = existing {
            self.retired
                .entry(owner.clone())
                .or_default()
                .insert(current.key().clone());
        }
        self.entries.insert(owner, intent);
        Ok(proof)
    }

    /// Release only the exact admitted identity tuple after finalizer
    /// completion has been confirmed by the Network resource owner.
    pub fn release_after_finalizer(
        &mut self,
        key: &NetworkAdmissionKey,
        finalizer_complete: bool,
    ) -> bool {
        if !finalizer_complete {
            return false;
        }
        let owner = NetworkAdmissionOwnerKey::from_key(key);
        if !self
            .entries
            .get(&owner)
            .is_some_and(|intent| intent.key() == key)
        {
            return false;
        }
        self.entries.remove(&owner);
        self.retired.entry(owner).or_default().insert(key.clone());
        let floor = self
            .released_floors
            .entry(NetworkAdmissionOwnerKey::from_key(key))
            .or_insert((0, 0));
        floor.0 = floor.0.max(key.network_generation().get());
        floor.1 = floor.1.max(key.attachment_generation().get());
        true
    }

    /// Return the live proof for one Zone/Network owner.
    pub fn proof_for(
        &self,
        zone_uid: &ResourceUid,
        network_uid: &ResourceUid,
    ) -> Option<NetworkAdmissionProof> {
        self.entries
            .get(&NetworkAdmissionOwnerKey {
                zone_uid: zone_uid.clone(),
                network_uid: network_uid.clone(),
            })
            .map(NetworkAdmissionIntent::proof)
    }

    /// Release the current owner only after its finalizer has completed.
    pub fn release_owner_after_finalizer(
        &mut self,
        zone_uid: &ResourceUid,
        network_uid: &ResourceUid,
        finalizer_complete: bool,
    ) -> bool {
        let owner = NetworkAdmissionOwnerKey {
            zone_uid: zone_uid.clone(),
            network_uid: network_uid.clone(),
        };
        let Some(key) = self.entries.get(&owner).map(|intent| intent.key().clone()) else {
            return false;
        };
        self.release_after_finalizer(&key, finalizer_complete)
    }

    /// Return the number of admitted Network projections.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no Network projection is currently admitted.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn is_current_or_newer_admission(
    current: &NetworkAdmissionKey,
    candidate: &NetworkAdmissionKey,
) -> bool {
    candidate.network_generation().get() >= current.network_generation().get()
        && candidate.attachment_generation().get() >= current.attachment_generation().get()
}

fn interface_is_self_owned(
    owner_intents: &[&NetworkAdmissionIntent],
    occupancy: &HostNetworkOccupancy,
    ifname: &d2b_contracts_resource::v3::IfName,
) -> bool {
    let actual_markers = occupancy.interface_ownership_markers(ifname);
    !actual_markers.is_empty()
        && actual_markers.iter().all(|actual_marker| {
            owner_intents.iter().any(|intent| {
                intent
                    .interface_ownership_marker(ifname)
                    .is_some_and(|expected| {
                        network_marker_matches(expected, actual_marker, intent.key())
                    })
            })
        })
}

fn cidr_is_self_owned(
    owner_intents: &[&NetworkAdmissionIntent],
    occupancy: &HostNetworkOccupancy,
    cidr: &d2b_contracts_resource::v3::network::Ipv4Cidr,
) -> bool {
    let actual_markers = occupancy.cidr_ownership_markers(cidr);
    !actual_markers.is_empty()
        && actual_markers.iter().all(|actual_marker| {
            owner_intents.iter().any(|intent| {
                if !intent
                    .cidrs()
                    .iter()
                    .any(|owned| d2b_contracts_resource::v3::network::cidr_overlaps(owned, cidr))
                    && !intent.routes().iter().any(|route| {
                        d2b_contracts_resource::v3::network::Ipv4Cidr::parse(
                            route.destination().to_owned(),
                        )
                        .ok()
                        .is_some_and(|route_cidr| {
                            d2b_contracts_resource::v3::network::cidr_overlaps(&route_cidr, cidr)
                        })
                    })
                {
                    return false;
                }
                network_marker_matches(intent.ownership_marker(), actual_marker, intent.key())
                    || intent.interface_names().iter().any(|ifname| {
                        intent
                            .interface_ownership_marker(ifname)
                            .is_some_and(|expected| {
                                network_marker_matches(expected, actual_marker, intent.key())
                            })
                    })
                    || intent.routes().iter().any(|route| {
                        d2b_contracts_resource::v3::network::Ipv4Cidr::parse(
                            route.destination().to_owned(),
                        )
                        .ok()
                        .is_some_and(|route_cidr| {
                            d2b_contracts_resource::v3::network::cidr_overlaps(&route_cidr, cidr)
                                && intent
                                    .route_ownership_marker(route)
                                    .is_some_and(|expected| {
                                        network_marker_matches(
                                            expected,
                                            actual_marker,
                                            intent.key(),
                                        )
                                    })
                        })
                    })
            })
        })
}

fn route_is_self_owned(
    owner_intents: &[&NetworkAdmissionIntent],
    occupancy: &HostNetworkOccupancy,
    route: &RouteTuple,
) -> bool {
    let actual_markers = occupancy.route_ownership_markers(route);
    !actual_markers.is_empty()
        && actual_markers.iter().all(|actual_marker| {
            owner_intents.iter().any(|intent| {
                intent.routes().contains(route)
                    && intent
                        .route_ownership_marker(route)
                        .is_some_and(|expected| {
                            network_marker_matches(expected, actual_marker, intent.key())
                        })
            })
        })
}

fn network_marker_matches(expected: &str, actual: &str, key: &NetworkAdmissionKey) -> bool {
    let Some((expected_key, expected_object)) = parse_network_marker(expected) else {
        return false;
    };
    let Some((actual_key, actual_object)) = parse_network_marker(actual) else {
        return false;
    };
    expected_key == actual_key
        && expected_object == actual_object
        && expected_key.zone_uid() == key.zone_uid()
        && expected_key.network_uid() == key.network_uid()
}

fn parse_network_marker(marker: &str) -> Option<(NetworkAdmissionKey, String)> {
    let marker = marker
        .strip_prefix("d2b managed: ")
        .unwrap_or(marker)
        .trim();
    let (object, rest) = marker.split_once(":zone:")?;
    let object = object.strip_prefix("network:")?.to_owned();
    let (zone, rest) = rest.split_once(":network:")?;
    let (network, rest) = rest.split_once(":generation:")?;
    let (generation, rest) = rest.split_once(":attachment:")?;
    let (attachment, bundle) = rest.split_once(":bundle:")?;
    let zone_uid = ResourceUid::parse(zone.to_owned()).ok()?;
    let network_uid = ResourceUid::parse(network.to_owned()).ok()?;
    let network_generation = ResourceGeneration::new(generation.parse().ok()?).ok()?;
    let attachment_generation = ResourceGeneration::new(attachment.parse().ok()?).ok()?;
    let bundle_generation = ResourceBundleGenerationId::parse(bundle.to_owned()).ok()?;
    Some((
        NetworkAdmissionKey::new(
            zone_uid,
            network_uid,
            network_generation,
            attachment_generation,
            bundle_generation,
        ),
        object,
    ))
}

/// Owner re-proof for one Zone's authority ledger.
///
/// The manager rows are the only authority (U14), so a claim whose owner does
/// not resolve to the manager's current row is refused rather than assumed. A
/// manager read failure is an error, never absence. The runtime is held
/// weakly because the ledger is one of the runtime's own fields.
struct ManagerAuthorityOwnerProvenance {
    runtime: std::sync::Weak<ZoneResourceRuntime>,
}

impl AuthorityOwnerProvenance for ManagerAuthorityOwnerProvenance {
    fn owner_identity<'a>(
        &'a self,
        owner_ref: &'a ResourceRef,
    ) -> AuthorityFuture<'a, Option<(ResourceUid, ResourceGeneration)>> {
        Box::pin(async move {
            let runtime = self
                .runtime
                .upgrade()
                .ok_or(AuthorityPersistenceError::StoreUnavailable)?;
            runtime
                .committed_manager_identity(owner_ref)
                .await
                .map_err(|_| AuthorityPersistenceError::StoreUnavailable)
        })
    }
}

/// All Zone runtimes owned by one daemon.
#[derive(Default)]
pub struct ResourcePlane {
    zones: BTreeMap<ZoneId, Arc<ZoneResourceRuntime>>,
    network_admission_index: Arc<tokio::sync::Mutex<HostNetworkAdmissionIndex>>,
    topology_root: Option<ZoneId>,
    gateway_zone_links: BTreeMap<ZoneId, Arc<crate::ZoneLinkGatewayComposition>>,
    gateway_zone_link_refused: BTreeSet<ZoneId>,
}

impl core::fmt::Debug for ResourcePlane {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ResourcePlane")
            .field("zone_count", &self.zones.len())
            .finish()
    }
}

/// Maximum number of Zone runtimes owned by one daemon (U14: the value the
/// retired store-runtime module carried).
const MAX_ZONE_RUNTIMES: usize = 64;

impl ResourcePlane {
    /// Create an empty daemon-owned plane.
    pub fn new() -> Self {
        Self {
            zones: BTreeMap::new(),
            network_admission_index: Arc::new(tokio::sync::Mutex::new(
                HostNetworkAdmissionIndex::default(),
            )),
            topology_root: None,
            gateway_zone_links: BTreeMap::new(),
            gateway_zone_link_refused: BTreeSet::new(),
        }
    }

    /// Borrow the one root-owned Host-global Network admission index.
    pub fn network_admission_index(&self) -> Arc<tokio::sync::Mutex<HostNetworkAdmissionIndex>> {
        Arc::clone(&self.network_admission_index)
    }

    /// Bind the sealed topology root selected during Zone publication.
    pub(crate) fn set_topology_root(&mut self, root: ZoneId) {
        self.topology_root = Some(root);
    }

    /// Borrow the sealed topology root.
    pub(crate) fn topology_root(&self) -> Option<&ZoneId> {
        self.topology_root.as_ref()
    }

    /// Insert a freshly opened Zone runtime.
    pub fn insert(
        &mut self,
        runtime: ZoneResourceRuntime,
    ) -> Result<Arc<ZoneResourceRuntime>, ResourceRuntimeError> {
        if self.zones.len() >= MAX_ZONE_RUNTIMES {
            return Err(ResourceRuntimeError::CoreStartupFailed);
        }
        let zone = runtime.zone().clone();
        if self.zones.contains_key(&zone) {
            return Err(ResourceRuntimeError::DuplicateZone);
        }
        let runtime = Arc::new(runtime);
        // The authority ledger's owner re-proof reads the manager rows this
        // daemon publishes; the runtime cannot hold a strong handle to itself.
        runtime
            .authority_ledger
            .install_owner_provenance(Arc::new(ManagerAuthorityOwnerProvenance {
                runtime: Arc::downgrade(&runtime),
            }));
        self.zones.insert(zone, Arc::clone(&runtime));
        Ok(runtime)
    }

    /// Resolve a Zone only from the authoritative plane index.
    pub fn zone(&self, zone: &ZoneId) -> Result<Arc<ZoneResourceRuntime>, ResourceRuntimeError> {
        self.zones
            .get(zone)
            .cloned()
            .ok_or(ResourceRuntimeError::PlaneUnavailable)
    }

    /// Return the number of ready Zone runtimes.
    pub fn ready_zone_count(&self) -> usize {
        self.zones
            .values()
            .filter(|runtime| runtime.require_ready().is_ok())
            .count()
    }

    /// Return whether a request still owns any Zone runtime.
    ///
    /// The plane itself owns one strong reference to every runtime. Any
    /// additional reference is an in-flight request owner and must keep the
    /// store open.
    pub fn has_live_request_owners(&self) -> bool {
        self.zones
            .values()
            .any(|runtime| Arc::strong_count(runtime) > 1)
    }

    /// Return the authoritative Zone identities currently owned by the plane.
    pub fn zone_ids(&self) -> Vec<ZoneId> {
        self.zones.keys().cloned().collect()
    }

    /// Install the one child-local Gateway Guest composition for a Zone.
    pub(crate) fn insert_gateway_zone_link(
        &mut self,
        composition: crate::ZoneLinkGatewayComposition,
    ) -> Result<(), ResourceRuntimeError> {
        let zone = composition.zone().clone();
        if self.gateway_zone_links.contains_key(&zone) {
            return Err(ResourceRuntimeError::DuplicateZone);
        }
        self.gateway_zone_link_refused.remove(&zone);
        self.gateway_zone_links.insert(zone, Arc::new(composition));
        Ok(())
    }

    /// Mark a committed gateway-backed Zone as refused so public dispatch
    /// cannot fall back to its host-local Resource API.
    pub(crate) fn refuse_gateway_zone_link(&mut self, zone: ZoneId) {
        self.gateway_zone_link_refused.insert(zone);
    }

    /// Return whether a committed gateway-backed Zone failed composition.
    pub(crate) fn gateway_zone_link_is_refused(&self, zone: &ZoneId) -> bool {
        self.gateway_zone_link_refused.contains(zone)
    }

    /// Borrow a Zone's Gateway Guest route composition, when one is installed.
    pub(crate) fn gateway_zone_link(
        &self,
        zone: &ZoneId,
    ) -> Option<Arc<crate::ZoneLinkGatewayComposition>> {
        self.gateway_zone_links.get(zone).cloned()
    }

    /// Drain runtimes and close every production backend.
    ///
    /// The map remains owned by the caller when a live request owner is
    /// observed, so a refused shutdown cannot drop the last backend owner and
    /// leave its clean-shutdown marker dirty.
    pub async fn shutdown(&mut self) -> Result<(), ResourceRuntimeError> {
        if self.has_live_request_owners() {
            return Err(ResourceRuntimeError::LiveRequestOwners);
        }
        let runtimes = std::mem::take(&mut self.zones);
        for (_, runtime) in runtimes {
            let runtime = match Arc::try_unwrap(runtime) {
                Ok(runtime) => runtime,
                Err(runtime) => {
                    self.zones.insert(runtime.zone().clone(), runtime);
                    return Err(ResourceRuntimeError::LiveRequestOwners);
                }
            };
            runtime.shutdown().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! U14 retired the durable-store runtime, so the store-backed scenarios
    //! this module carried are gone with it; U15 rebuilds the manager-only
    //! invariant matrix for the ZoneResourceRuntime.
    use super::*;
    use std::collections::VecDeque;

    use std::os::fd::AsRawFd;
    use d2b_contracts_zone_session::v3::component_session::LimitProfile;
    use d2b_core::{
        bundle::{Bundle, BundleGeneration},
        bundle_resolver::BundleResolver,
        manifest_v04::ManifestV04,
        processes::ProcessesJson,
    };
    use d2b_session_unix::{CreditPool, CreditScopeSet, OutboundPacket, prearmed_seqpacket_pair};

    /// Regression (host integration 2026-09-11): a reconnect advances the
    /// live accepted session generation by design (the Guest admits only a
    /// strictly newer one), which must not move the Guest's incarnation
    /// fence. The fence's session member is the enrolled identity generation
    /// the live session was admitted under, so a Guest that legitimately
    /// reconnected stays `Ready` instead of reporting `Pending` with
    /// `runtimeReady=false` forever.
    #[test]
    fn reconnect_keeps_the_incarnation_fence_on_the_enrolled_session_generation() {
        use d2b_contracts_resource::v3::identity::{SchemaFingerprint, SessionPurpose};
        let identity = d2bd_runtime::guest_mode::GuestIdentity::new(
            ResourceRef::parse("Guest/acceptance-guest").expect("Guest ref"),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("Guest UID"),
            ZoneId::parse("work").expect("Zone"),
            d2bd_runtime::guest_mode::BootIdentity::from_kernel_boot_id("guest-fence-test")
                .expect("boot identity"),
            SessionPurpose::parse(d2bd_runtime::guest_mode::GUEST_COMPONENT_SESSION_PURPOSE)
                .expect("purpose"),
            SchemaFingerprint::parse(
                "sha256:0000000000000000000000000000000000000000000000000000000000000001",
            )
            .expect("schema"),
            ReconnectGeneration::new(1).expect("enrolled generation"),
            1,
            1,
            1,
        )
        .expect("Guest identity");
        // The second acceptance of the same enrollment: the Guest admitted
        // the reconnect under the same enrolled identity generation.
        let binding = guest_session_evidence_binding(
            &identity,
            2,
            "sha256:0000000000000000000000000000000000000000000000000000000000000002",
            1,
        )
        .expect("evidence binding");
        assert_eq!(
            binding.session_generation(),
            2,
            "the live accepted generation stays the freshness marker",
        );
        assert_eq!(
            binding.reconnect_generation(),
            1,
            "the incarnation fence keeps the enrolled identity generation",
        );
        let evidence = GuestSessionEvidence::current_bound(
            ResourceRef::parse("Guest/acceptance-guest").expect("Guest ref"),
            "sha256:0000000000000000000000000000000000000000000000000000000000000003".to_owned(),
            ["resource-commit".to_owned(), "resource-watch".to_owned()],
            true,
            true,
            true,
            binding,
        )
        .expect("evidence");
        assert!(
            guest_incarnation_generations(1, 1, 1, Some(&evidence)).is_exact(),
            "a reconnect must not break the Guest incarnation fence",
        );
    }

    #[test]
    fn trusted_provider_catalog_includes_declared_binding() {
        let resource_types = trusted_provider_resource_types().expect("trusted declarations");
        // VolumeBinding is admitted through the standard catalog only (KTD8):
        // it is a standard, unqualified type and never a trusted qualified
        // extension, and the removed Export type must not re-enter either.
        let binding = ResourceTypeName::parse(
            d2b_contracts_resource::v3::VOLUME_BINDING_RESOURCE_TYPE,
        )
        .unwrap();
        assert!(!resource_types.contains(&binding));
        assert!(!resource_types
            .iter()
            .any(|resource_type| resource_type.as_str() == "virtiofs.d2bus.org.Export"));
        let standard = d2b_resource_api::authz::ApiCatalog::standard();
        assert!(
            d2b_resource_api::authz::PolicyRule::new(
                &standard,
                [binding],
                [d2b_resource_api::authz::ResourceVerb::Get],
                [],
                [],
                [],
                [],
                [],
            )
            .is_ok(),
            "VolumeBinding must be installed in the standard catalog"
        );
        assert!(d2b_resource_api::authz::PolicyRule::new(
            &standard,
            [ResourceTypeName::parse("virtiofs.d2bus.org.Export").unwrap()],
            [d2b_resource_api::authz::ResourceVerb::Get],
            [],
            [],
            [],
            [],
            [],
        )
        .is_err());
    }

    #[test]
    fn unknown_qualified_bundle_resource_types_fail_closed() {
        let trusted =
            ResourceTypeName::parse(d2b_contracts_resource::v3::VOLUME_BINDING_RESOURCE_TYPE)
                .unwrap();
        assert!(trusted_catalog_resource_types([trusted]).is_ok());
        let unknown = ResourceTypeName::parse("untrusted.d2bus.org.Type").unwrap();
        assert_eq!(
            trusted_catalog_resource_types([unknown]),
            Err(ResourceRuntimeError::AuthorizationUnavailable)
        );
    }

    #[test]
    fn cloud_hypervisor_process_update_applies_requested_lifecycle() {
        let current = json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Process",
            "metadata": {},
            "spec": {
                "desiredLifecycle": "stopped",
                "providerRef": "Provider/system-minijail"
            },
            "status": {}
        });
        let body = d2b_provider_runtime_cloud_hypervisor::ChildCreateBody::Process(
            d2b_provider_runtime_cloud_hypervisor::ProcessCreateBody::new(
                ResourceRef::parse("Host/host-system").unwrap(),
            )
            .unwrap(),
        );

        let updated =
            merge_cloud_hypervisor_child_spec(&current, &body, Some(DesiredLifecycle::Running))
                .unwrap();

        assert_eq!(updated["desiredLifecycle"], "running");
        assert_eq!(updated["providerRef"], "Provider/system-minijail");
    }

    fn test_controller_session_providers(
    ) -> Arc<crate::process_provider_runtime::ProductionProcessProviders> {
        let host = serde_json::from_str(include_str!(
            "../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .unwrap();
        let manifest = ManifestV04::from_slice(
            include_str!("../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .unwrap();
        let resolver = BundleResolver::from_artifacts_with_zone_resource_bundles(
            Bundle {
                bundle_version: 11,
                schema_version: "v2".to_owned(),
                public_manifest_path: "vms.json".to_owned(),
                host_path: "host.json".to_owned(),
                processes_path: "processes.json".to_owned(),
                privileges_path: "privileges.json".to_owned(),
                storage_path: None,
                sync_path: None,
                allocator_path: None,
                realm_controllers_path: None,
                realm_identity_path: None,
                realm_workloads_launcher_v2_path: None,
                unsafe_local_workloads_path: None,
                closures: Vec::new(),
                minijail_profiles: Vec::new(),
                managed_keys: Default::default(),
                generation: BundleGeneration {
                    generator: "test".to_owned(),
                    source_revision: None,
                    generated_at: None,
                },
                bundle_hash: Some("sha256:bundle".to_owned()),
                artifact_hashes: None,
            },
            host,
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            manifest,
            BTreeMap::new(),
        );
        Arc::new(
            crate::process_provider_runtime::ProductionProcessProviders::new_for_mode(
                resolver,
                std::path::PathBuf::from("/nonexistent/d2b-broker.sock"),
                BrokerCallerRole::AdminUid { uid: 0 },
                DaemonMode::Host,
            ),
        )
    }

    #[test]
    fn readable_pending_controller_bootstrap_wakes_coordinator_to_active() {
        let zone = ZoneId::parse("work").unwrap();
        let providers = test_controller_session_providers();
        let process_ref = ResourceRef::parse("Process/provider-controller").unwrap();
        let provider_owner_ref =
            ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap();
        let process_provider_ref = ResourceRef::parse("Provider/system-minijail").unwrap();
        let (daemon_endpoint, peer_endpoint) = prearmed_seqpacket_pair().unwrap();
        nix::sys::socket::send(
            peer_endpoint.as_raw_fd(),
            b"bootstrap-ready",
            nix::sys::socket::MsgFlags::empty(),
        )
        .unwrap();
        let wake_count = Arc::new(AtomicUsize::new(0));
        let callback_providers = Arc::downgrade(&providers);
        let callback_zone = zone.clone();
        let callback_process_ref = process_ref.clone();
        let callback_wake_count = Arc::clone(&wake_count);
        providers
            .set_controller_session_waker(
                zone.clone(),
                Arc::new(move || {
                    let providers = callback_providers
                        .upgrade()
                        .expect("providers remain while the wake is delivered");
                    assert!(providers
                        .controller_bootstrap_ready(&callback_zone, &callback_process_ref));
                    let context = providers
                        .controller_bootstrap_contexts(&callback_zone)
                        .into_iter()
                        .find(|context| context.process_ref() == &callback_process_ref)
                        .expect("readable Pending endpoint context");
                    let endpoint = providers
                        .begin_controller_bootstrap_if_matches(&callback_zone, &context)
                        .expect("readable Pending endpoint must be claimed");
                    let context = endpoint.context().clone();
                    assert!(providers.activate_controller_bootstrap(&context));
                    callback_wake_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }),
            )
            .unwrap();
        providers
            .attach_pending_controller_provider_context_for_test(
                daemon_endpoint,
                zone.clone(),
                process_ref.clone(),
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                ResourceGeneration::new(1).unwrap(),
                process_provider_ref,
                provider_owner_ref,
                ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ResourceRef::parse("Host/host-system").unwrap(),
                ControllerGeneration::new(1).unwrap(),
            )
            .unwrap();

        assert_eq!(wake_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            providers
                .controller_bootstrap_contexts(&zone)
                .into_iter()
                .map(|context| context.process_ref().clone())
                .collect::<Vec<_>>(),
            vec![process_ref.clone()]
        );
        assert!(!providers.controller_bootstrap_ready(&zone, &process_ref));
    }





    #[test]
    fn generation_publication_marker_binds_one_complete_set_across_restart() {
        let zones = BTreeSet::from([
            ZoneId::parse("local-root").unwrap(),
            ZoneId::parse("work").unwrap(),
        ]);
        let generations = BTreeMap::from([
            (
                ZoneId::parse("local-root").unwrap(),
                ResourceBundleGenerationId::parse("sha256:".to_owned() + &"a".repeat(64)).unwrap(),
            ),
            (
                ZoneId::parse("work").unwrap(),
                ResourceBundleGenerationId::parse("sha256:".to_owned() + &"b".repeat(64)).unwrap(),
            ),
        ]);
        let set_generation =
            complete_generation_set_digest(&zones, &generations).expect("complete generation");
        let binding_digest = "sha256:".to_owned() + &"c".repeat(64);
        let payload =
            generation_publication_payload(&set_generation, &binding_digest, &generations)
                .expect("publication payload");
        assert!(generation_publication_payload_matches(
            &payload,
            &set_generation,
            &binding_digest,
            &generations,
        ));

        let mut recovered: Value = serde_json::from_slice(&payload).expect("payload JSON");
        recovered["state"] = Value::String("effect-confirmed".to_owned());
        let recovered = serde_json::to_vec(&recovered).expect("recovered payload");
        assert!(generation_publication_payload_matches(
            &recovered,
            &set_generation,
            &binding_digest,
            &generations,
        ));

        let mut mixed = generations.clone();
        mixed.insert(
            ZoneId::parse("work").unwrap(),
            ResourceBundleGenerationId::parse("sha256:".to_owned() + &"d".repeat(64)).unwrap(),
        );
        assert!(!generation_publication_payload_matches(
            &recovered,
            &set_generation,
            &binding_digest,
            &mixed,
        ));
    }



    #[test]
    fn controller_session_admission_requires_the_exact_owner_binding() {
        let target = AssignmentTarget::Execution {
            kind: PlacementTargetKind::Host,
            reference: ResourceRef::parse("Host/host-system").unwrap(),
        };
        let first = ControllerSessionBinding::new(
            ResourceRef::parse("Process/controller-first").unwrap(),
            ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
            ResourceRef::parse(d2b_provider_runtime_cloud_hypervisor::CONTROLLER_ROLE_REF).unwrap(),
            target.clone(),
            ResourceGeneration::new(2).unwrap(),
            ControllerGeneration::new(3).unwrap(),
            ReconnectGeneration::new(1).unwrap(),
        )
        .unwrap();
        let second = ControllerSessionBinding::new(
            ResourceRef::parse("Process/controller-second").unwrap(),
            ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap(),
            ResourceRef::parse(d2b_provider_runtime_cloud_hypervisor::CONTROLLER_ROLE_REF).unwrap(),
            target,
            ResourceGeneration::new(2).unwrap(),
            ControllerGeneration::new(3).unwrap(),
            ReconnectGeneration::new(1).unwrap(),
        )
        .unwrap();

        assert!(controller_session_matches(&first, &first, false));
        assert!(!controller_session_matches(&first, &second, false));
        assert!(!controller_session_matches(&first, &first, true));
    }

    #[test]
    fn stale_controller_session_clear_errors_are_safe_for_teardown_only() {
        assert!(ControllerSessionCoordinator::controller_session_clear_error_is_stale_identity(
            &ResourceRuntimeError::IdentityUnbound
        ));
        assert!(
            ControllerSessionCoordinator::controller_session_clear_error_is_stale_identity(
                &ResourceRuntimeError::ResourceStatusUpdateFailed(ResourceErrorKind::ResourceNotFound)
            )
        );
        assert!(
            !ControllerSessionCoordinator::controller_session_clear_error_is_stale_identity(
                &ResourceRuntimeError::AuthenticationUnavailable
            )
        );
    }

    #[test]
    fn stale_owner_identity_still_allows_fenced_controller_session_clear() {
        assert_eq!(
            controller_session_evidence_identity_check(false, true),
            Ok(())
        );
        assert_eq!(
            controller_session_evidence_identity_check(false, false),
            Err(ResourceRuntimeError::IdentityUnbound)
        );
        assert_eq!(
            controller_session_evidence_identity_check(true, false),
            Ok(())
        );
    }

    #[test]
    fn controller_session_evidence_conflicts_are_scoped_to_one_context() {
        for kind in [
            ResourceErrorKind::ResourceConflict,
            ResourceErrorKind::RevisionExpired,
        ] {
            assert!(
                ControllerSessionCoordinator::controller_session_evidence_error_is_context_local(
                    &ResourceRuntimeError::ResourceStatusUpdateFailed(kind)
                )
            );
        }
        assert!(
            !ControllerSessionCoordinator::controller_session_evidence_error_is_context_local(
                &ResourceRuntimeError::ResourceStatusUpdateFailed(
                    ResourceErrorKind::ResourceStatusOwnerMismatch
                )
            )
        );
        for kind in [
            ResourceErrorKind::Backpressure,
            ResourceErrorKind::Timeout,
            ResourceErrorKind::Cancelled,
            ResourceErrorKind::ResourcePlaneUnavailable,
        ] {
            assert!(
                ControllerSessionCoordinator::controller_session_evidence_error_is_context_local(
                    &ResourceRuntimeError::ResourceGetFailed(kind)
                )
            );
        }
        assert!(
            !ControllerSessionCoordinator::controller_session_evidence_error_is_context_local(
                &ResourceRuntimeError::AuthenticationUnavailable
            )
        );
    }


    #[test]
    fn controller_provider_identity_failures_preserve_global_store_and_auth_fences() {
        assert!(
            ControllerSessionCoordinator::controller_provider_identity_error_is_global(
                &ResourceRuntimeError::StoreReadFailed
            )
        );
        assert!(
            ControllerSessionCoordinator::controller_provider_identity_error_is_global(
                &ResourceRuntimeError::AuthenticationUnavailable
            )
        );
        assert!(
            !ControllerSessionCoordinator::controller_provider_identity_error_is_global(
                &ResourceRuntimeError::InteractionConfigurationUnavailable
            )
        );
    }

    #[test]
    fn stale_controller_session_clear_isolated_from_unrelated_contexts() {
        assert_eq!(
            ControllerSessionCoordinator::isolate_controller_session_clear_error(Err(
                ResourceRuntimeError::IdentityUnbound
            )),
            Ok(())
        );
        assert_eq!(
            ControllerSessionCoordinator::isolate_controller_session_clear_error(Err(
                ResourceRuntimeError::ResourceStatusUpdateFailed(
                    ResourceErrorKind::ResourceNotFound
                )
            )),
            Ok(())
        );
        for kind in [
            ResourceErrorKind::ResourceConflict,
            ResourceErrorKind::RevisionExpired,
            ResourceErrorKind::Backpressure,
            ResourceErrorKind::Timeout,
            ResourceErrorKind::Cancelled,
            ResourceErrorKind::ResourcePlaneUnavailable,
        ] {
            assert_eq!(
                ControllerSessionCoordinator::isolate_controller_session_clear_error(Err(
                    ResourceRuntimeError::ResourceStatusUpdateFailed(kind)
                )),
                Ok(())
            );
        }
        assert_eq!(
            ControllerSessionCoordinator::isolate_controller_session_clear_error(Err(
                ResourceRuntimeError::AuthenticationUnavailable
            )),
            Err(ResourceRuntimeError::AuthenticationUnavailable)
        );
    }











    /// Regression (vmCheck guest preflight): the endpoint-publication stage
    /// must distinguish a child's retry in progress from a child's terminal
    /// state. The `ch-api` Endpoint actor's bounded realize effect waits for
    /// the VMM evidence and fails retryably while the VMM is still coming up;
    /// treating that `Failed` phase as terminal failed the whole Guest effect
    /// (`CapabilityUnavailable`) and left the Guest Failed.
    #[test]
    fn endpoint_publication_gate_defers_a_retrying_child() {
        assert_eq!(
            child_publication_gate(ResourcePhase::Ready, false),
            ChildPublicationGate::Ready
        );
        for phase in [
            ResourcePhase::Pending,
            ResourcePhase::Unknown,
            ResourcePhase::Degraded,
        ] {
            assert_eq!(
                child_publication_gate(phase, false),
                ChildPublicationGate::Pending,
                "{phase:?} is a child still coming up",
            );
        }
        assert_eq!(
            child_publication_gate(ResourcePhase::Failed, true),
            ChildPublicationGate::Pending,
            "a retryable failure is the child's own retry in progress",
        );
        assert_eq!(
            child_publication_gate(ResourcePhase::Failed, false),
            ChildPublicationGate::Terminal,
            "a terminal child failure still refuses",
        );
        for phase in [ResourcePhase::Deleted, ResourcePhase::Succeeded] {
            assert_eq!(
                child_publication_gate(phase, false),
                ChildPublicationGate::Terminal,
                "{phase:?} is not a child this stage may publish",
            );
        }
    }

    /// The retryable/terminal distinction is read from the manager view's
    /// stamp of the failed actor's closed classification (the converted
    /// plane carries no durable status).
    #[test]
    fn retryable_classification_is_read_from_the_manager_status_projection() {
        let retrying = json!({
            "status": {
                "phase": "Failed",
                "resource": {
                    "driverFailure": { "operation": "Reconcile", "retryable": true },
                },
            },
        });
        assert!(row_status_failure_is_retryable(&retrying));
        let terminal = json!({
            "status": {
                "phase": "Failed",
                "resource": {
                    "driverFailure": { "operation": "Validate", "retryable": false },
                },
            },
        });
        assert!(!row_status_failure_is_retryable(&terminal));
        let ready = json!({ "status": { "phase": "Ready", "resource": {} } });
        assert!(!row_status_failure_is_retryable(&ready));
    }

    /// U12: the provider controller's custody gate. A manager-served Guest
    /// row carries no authored finalizer (the daemon's ensure/clear requests
    /// are idempotent no-ops on the converted plane; the manager's
    /// deleting-row hold replaces it) and must still admit the controller,
    /// while a durable row keeps the exact authored signal.
    #[test]
    fn manager_served_guest_reports_the_controller_finalizer_present() {
        assert!(guest_controller_finalizer_present(
            StoredRowOrigin::Manager,
            std::iter::empty(),
        ));
        assert!(guest_controller_finalizer_present(
            StoredRowOrigin::Manager,
            ["other.d2bus.org/finalizer"].into_iter(),
        ));
        assert!(!guest_controller_finalizer_present(
            StoredRowOrigin::Durable,
            std::iter::empty(),
        ));
        assert!(guest_controller_finalizer_present(
            StoredRowOrigin::Durable,
            [d2b_provider_runtime_cloud_hypervisor::GUEST_CONTROLLER_FINALIZER].into_iter(),
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn controller_bootstrap_receiver_accepts_one_authenticated_endpoint() {
        let (sender_fd, receiver_fd) = prearmed_seqpacket_pair().unwrap();
        let sender = SeqpacketSocket::from_parent_prearmed(sender_fd).unwrap();
        let receiver = SeqpacketSocket::from_parent_prearmed(receiver_fd).unwrap();
        let (resource_fd, _resource_peer) = prearmed_seqpacket_pair().unwrap();
        let policy = controller_bootstrap_attachment_policy();
        let capacity = AncillaryCapacity::from_policy(policy).unwrap();
        let scopes = CreditScopeSet::new(
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
        );
        let packet = OutboundPacket::with_current_credentials(
            d2b_session_unix::CONTROLLER_BOOTSTRAP_PROTOCOL_MARKER.to_vec(),
            vec![Arc::new(resource_fd)],
            LimitProfile::local_default(),
            capacity,
            &scopes,
        )
        .unwrap();
        let mut queue = VecDeque::from([packet]);
        assert_eq!(
            sender
                .send_burst(&mut queue, capacity, 2)
                .await
                .unwrap()
                .sent
                .len(),
            1
        );
        let (resource_socket, credentials) = receive_controller_bootstrap(&receiver)
            .await
            .expect("authenticated bootstrap endpoint");
        assert_eq!(
            resource_socket.acceptor_peer_credentials().unwrap(),
            credentials
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn controller_bootstrap_receiver_rejects_extra_packets() {
        let (sender_fd, receiver_fd) = prearmed_seqpacket_pair().unwrap();
        let sender = SeqpacketSocket::from_parent_prearmed(sender_fd).unwrap();
        let receiver = SeqpacketSocket::from_parent_prearmed(receiver_fd).unwrap();
        let policy = controller_bootstrap_attachment_policy();
        let capacity = AncillaryCapacity::from_policy(policy).unwrap();
        let scopes = CreditScopeSet::new(
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
        );
        let (resource_a, _resource_a_peer) = prearmed_seqpacket_pair().unwrap();
        let (resource_b, _resource_b_peer) = prearmed_seqpacket_pair().unwrap();
        let mut queue = VecDeque::from([
            OutboundPacket::with_current_credentials(
                d2b_session_unix::CONTROLLER_BOOTSTRAP_PROTOCOL_MARKER.to_vec(),
                vec![Arc::new(resource_a)],
                LimitProfile::local_default(),
                capacity,
                &scopes,
            )
            .unwrap(),
            OutboundPacket::with_current_credentials(
                d2b_session_unix::CONTROLLER_BOOTSTRAP_PROTOCOL_MARKER.to_vec(),
                vec![Arc::new(resource_b)],
                LimitProfile::local_default(),
                capacity,
                &scopes,
            )
            .unwrap(),
        ]);
        assert_eq!(
            sender
                .send_burst(&mut queue, capacity, 2)
                .await
                .unwrap()
                .sent
                .len(),
            2
        );
        assert!(matches!(
            receive_controller_bootstrap(&receiver).await,
            Err(ResourceRuntimeError::AuthenticationUnavailable)
        ));
    }

    struct PolicyEvidenceAdmission;

    impl d2b_session::SessionRegistrationCapability<()> for PolicyEvidenceAdmission {
        type Error = std::convert::Infallible;

        fn consume(self, _registrar: &()) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    /// A registrar-free acceptor over one policy. `SessionAcceptor` validates
    /// the transport evidence before either authority callback runs, so the
    /// stubbed callbacks only need to be well-formed.
    fn policy_evidence_acceptor(
        policy: &d2b_contracts_zone_session::v3::component_session::EndpointPolicy,
    ) -> d2b_session::SessionAcceptor<PolicyEvidenceAdmission> {
        use d2b_contracts_resource::v3::identity::SessionBinding;
        use d2b_contracts_zone_session::v3::component_session::{
            AuthorizationLease, SessionErrorCode,
        };
        use d2b_session::{SessionAuthenticationBinding, SessionError};

        let zone = ZoneId::parse("work").unwrap();
        let subject_ref = ResourceRef::parse("Host/policy-evidence").unwrap();
        let subject_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let zone_ref = ResourceRef::parse("Zone/work").unwrap();
        d2b_session::SessionAcceptor::from_verified_adapter(
            policy.clone(),
            zone,
            move |_evidence: TransportEvidence,
                  binding: &SessionAuthenticationBinding,
                  _expected_zone: &ZoneId,
                  now_tick: u64| {
                let subject = AuthenticatedSubjectContext::new(
                    subject_ref.clone(),
                    subject_uid.clone(),
                    zone_ref.clone(),
                    binding.evidence_class(),
                    binding.purpose().clone(),
                    binding.service().clone(),
                    SessionBinding::new(
                        binding.schema_fingerprint().clone(),
                        binding.transport_binding().clone(),
                        binding.reconnect_generation(),
                        binding.transcript_hash().clone(),
                    ),
                );
                let lease = AuthorizationLease::new(1, now_tick.saturating_add(10))
                    .map_err(SessionError::from)?;
                Ok((subject, lease))
            },
            move |_subject: &AuthenticatedSubjectContext,
                  _request: &d2b_session::SessionAuthorizationRequest,
                  previous: AuthorizationLease,
                  now_tick: u64| {
                if previous.policy_revision() != 1 {
                    return Err(SessionError::new(SessionErrorCode::PolicyDenied));
                }
                AuthorizationLease::new(1, now_tick.saturating_add(10))
                    .map_err(SessionError::from)
            },
            PolicyEvidenceAdmission,
        )
        .expect("a policy acceptor")
    }

    /// Regression (security review finding 2): the SessionAcceptor evidence
    /// is derived from the policy the acceptor admits. The credential
    /// Provider lane binds `[0x34; 32]`, so the constant `[0x22; 32]` the
    /// daemon used before admitted no credential session at all, and the
    /// controller lane keeps admitting exactly its own policy binding.
    #[tokio::test(flavor = "current_thread")]
    async fn credential_provider_evidence_comes_from_the_admitted_policy() {
        use d2b_contracts_resource::v3::identity::BindingDigest;
        use d2b_contracts_zone_session::v3::component_session::SessionErrorCode;
        use d2b_session_unix::UnixSeqpacketTransport;

        async fn engines(
            policy: &d2b_contracts_zone_session::v3::component_session::EndpointPolicy,
        ) -> (
            SessionEngine<UnixSeqpacketTransport>,
            SessionEngine<UnixSeqpacketTransport>,
        ) {
            let (initiator_fd, responder_fd) = prearmed_seqpacket_pair().unwrap();
            let initiator_socket = SeqpacketSocket::from_parent_prearmed(initiator_fd).unwrap();
            let responder_socket = SeqpacketSocket::from_parent_prearmed(responder_fd).unwrap();
            let (initiator, responder) = tokio::join!(
                SessionEngine::establish_initiator(
                    unix_transport(initiator_socket, policy).unwrap(),
                    policy.clone(),
                    HandshakeCredentials::Nn,
                    std::time::Instant::now(),
                ),
                SessionEngine::establish_responder(
                    unix_transport(responder_socket, policy).unwrap(),
                    policy.clone(),
                    HandshakeCredentials::Nn,
                    std::time::Instant::now(),
                ),
            );
            (initiator.unwrap(), responder.unwrap())
        }

        // Both lanes admit the binding of the policy they accept on.
        for policy in [
            credential_provider_endpoint_policy(),
            controller_resource_endpoint_policy(),
        ] {
            let (_initiator, responder) = engines(&policy).await;
            let evidence = TransportEvidence::new(
                EvidenceClass::UnixPeer,
                crate::interaction_composition::policy_channel_binding_digest(&policy)
                    .expect("a policy channel binding"),
            );
            let admitted = policy_evidence_acceptor(&policy)
                .admit(responder, evidence, 1)
                .await
                .map(|_| ())
                .map_err(|error| error.code());
            assert_eq!(
                admitted,
                Ok(()),
                "the daemon must admit the binding of the policy it accepted on",
            );
        }

        // The credential Provider lane binds `[0x34; 32]`, so the `[0x22; 32]`
        // constant the daemon used before admits no credential session at all.
        let credential = credential_provider_endpoint_policy();
        let (initiator, _responder) = engines(&credential).await;
        let legacy = TransportEvidence::new(
            EvidenceClass::UnixPeer,
            BindingDigest::parse(format!("sha256:{}", "22".repeat(32)))
                .expect("a binding digest"),
        );
        let refused = policy_evidence_acceptor(&credential)
            .admit(initiator, legacy, 1)
            .await
            .map(|_| ())
            .map_err(|error| error.code());
        assert_eq!(
            refused,
            Err(SessionErrorCode::ChannelBindingMismatch),
            "the constant that admitted no credential session is still refused",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn provider_ready_survives_a_fragment_that_precedes_the_session_driver() {
        let (initiator_fd, responder_fd) = prearmed_seqpacket_pair().unwrap();
        let initiator_socket = SeqpacketSocket::from_parent_prearmed(initiator_fd).unwrap();
        let responder_socket = SeqpacketSocket::from_parent_prearmed(responder_fd).unwrap();
        let policy = credential_provider_endpoint_policy();
        let (initiator, responder) = tokio::join!(
            SessionEngine::establish_initiator(
                unix_transport(initiator_socket, &policy).unwrap(),
                policy.clone(),
                HandshakeCredentials::Nn,
                std::time::Instant::now(),
            ),
            SessionEngine::establish_responder(
                unix_transport(responder_socket, &policy).unwrap(),
                policy.clone(),
                HandshakeCredentials::Nn,
                std::time::Instant::now(),
            ),
        );
        let mut responder = responder.unwrap();
        // The Provider writes its readiness receipt as soon as its own
        // handshake completes, which can precede the daemon-side driver. The
        // streams must already be registered on the engine when that fragment
        // is the first record the driver routes.
        preregister_provider_session_streams(&mut responder).unwrap();
        let initiator = initiator.unwrap().into_driver();
        let ready = StreamId::new(PROVIDER_READY_STREAM_ID).unwrap();
        initiator
            .open_named_stream(
                ready,
                PROVIDER_READY_STREAM_CREDIT,
                PROVIDER_READY_STREAM_CREDIT,
            )
            .await
            .unwrap();
        initiator
            .send_named_stream(ready, PROVIDER_READY_MARKER.to_vec())
            .await
            .unwrap();
        initiator.close_named_stream(ready).await.unwrap();
        let driver = responder.into_driver();
        let ready_result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            receive_provider_ready(&driver),
        )
        .await
        .expect("an early provider fragment must not stall the session");
        assert!(
            ready_result.is_ok(),
            "an early provider fragment must be routed rather than fail the session as an invalid channel: {ready_result:?}",
        );
    }

    #[test]
    fn tpm_device_binding_requires_the_authenticated_guest_owner() {
        let matching = json!({ "metadata": { "ownerRef": "Guest/vm-a" } });
        let mismatched = json!({ "metadata": { "ownerRef": "Guest/vm-b" } });
        let absent = json!({ "metadata": {} });

        assert!(ZoneResourceRuntime::tpm_device_targets_vm(
            &matching, "vm-a"
        ));
        assert!(!ZoneResourceRuntime::tpm_device_targets_vm(
            &mismatched,
            "vm-a"
        ));
        assert!(!ZoneResourceRuntime::tpm_device_targets_vm(&absent, "vm-a"));
    }

    #[test]
    fn security_key_device_binding_requires_stored_zone_owner_and_selector() {
        let matching = json!({
            "metadata": { "ownerRef": "Guest/vm-a", "zone": "work" },
            "spec": {
                "providerRef": "Provider/device-security-key",
                "inventory": { "selector": { "label": "key-primary" } }
            }
        });
        let zone = ZoneId::parse("work".to_owned()).unwrap();
        let zone_ref = ResourceRef::parse("Zone/work").unwrap();
        let holder_ref = ResourceRef::parse("Guest/vm-a").unwrap();

        assert!(ZoneResourceRuntime::security_key_device_matches(
            &matching,
            &zone,
            &zone_ref,
            &holder_ref,
            "vm-a",
            "key-primary",
        ));
        assert!(!ZoneResourceRuntime::security_key_device_matches(
            &matching,
            &zone,
            &ResourceRef::parse("Zone/home").unwrap(),
            &holder_ref,
            "vm-a",
            "key-primary",
        ));
        assert!(!ZoneResourceRuntime::security_key_device_matches(
            &matching,
            &zone,
            &zone_ref,
            &ResourceRef::parse("Guest/vm-b").unwrap(),
            "vm-a",
            "key-primary",
        ));
        assert!(!ZoneResourceRuntime::security_key_device_matches(
            &matching,
            &zone,
            &zone_ref,
            &holder_ref,
            "vm-a",
            "key-secondary",
        ));
    }

    #[test]
    fn trusted_bundle_inventory_selects_fresh_or_legacy_tpm_path() {
        let fresh =
            ZoneResourceRuntime::tpm_migration_decision("vm-a", "legacy-swtpm:vm:vm-a", None);
        assert!(!fresh.requires_migration());
        assert!(fresh.validates_binding("vm-a", "legacy-swtpm:vm:vm-a"));

        let legacy = ZoneResourceRuntime::tpm_migration_decision(
            "vm-a",
            "legacy-swtpm:vm:vm-a",
            Some("legacy-swtpm:vm:vm-a"),
        );
        assert!(legacy.requires_migration());
        assert!(legacy.validates_binding("vm-a", "legacy-swtpm:vm:vm-a"));
        assert!(!legacy.validates_binding("vm-b", "legacy-swtpm:vm:vm-a"));
    }

    fn network_admission_intent(
        zone: &str,
        network: &str,
        lan: &str,
        uplink: &str,
    ) -> NetworkAdmissionIntent {
        let zone_uid = ResourceUid::parse(zone).unwrap();
        let network_uid = ResourceUid::parse(network).unwrap();
        let spec = d2b_contracts_resource::v3::network::NetworkSpec::minimal(
            d2b_contracts_resource::v3::network::Ipv4Cidr::parse(lan).unwrap(),
            d2b_contracts_resource::v3::network::Ipv4Cidr::parse(uplink).unwrap(),
            d2b_contracts_resource::v3::execution_policy::BoundedToken::parse("net-vm-base")
                .unwrap(),
        )
        .unwrap();
        NetworkAdmissionIntent::new(
            NetworkAdmissionKey::new(
                zone_uid,
                network_uid,
                ResourceGeneration::new(1).unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ResourceBundleGenerationId::parse(
                    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                )
                .unwrap(),
            ),
            spec,
            Vec::new(),
        )
        .unwrap()
    }

    fn external_network_admission_intent(
        zone: &str,
        network: &str,
        lan: &str,
        uplink: &str,
        sharing: d2b_contracts_resource::v3::network::SharingPolicy,
    ) -> NetworkAdmissionIntent {
        let zone_uid = ResourceUid::parse(zone).unwrap();
        let network_uid = ResourceUid::parse(network).unwrap();
        let external = d2b_contracts_resource::v3::network::ExternalAttachmentSpec::new(
            d2b_contracts_resource::v3::network::ExternalAttachmentMode::Macvtap,
            d2b_contracts_resource::v3::IfName::parse("eno1").unwrap(),
            d2b_contracts_resource::v3::network::MacvtapMode::Bridge,
            sharing,
            None,
            d2b_contracts_resource::v3::network::ExternalIpv4Spec::default(),
            d2b_contracts_resource::v3::network::EgressSpec::default(),
            Vec::new(),
        )
        .unwrap();
        let spec = d2b_contracts_resource::v3::network::NetworkSpec::new(
            d2b_contracts_resource::v3::network::Ipv4Cidr::parse(lan).unwrap(),
            d2b_contracts_resource::v3::network::Ipv4Cidr::parse(uplink).unwrap(),
            None,
            false,
            d2b_contracts_resource::v3::network::IsolationSpec::default(),
            d2b_contracts_resource::v3::network::RoutingSpec::default(),
            d2b_contracts_resource::v3::network::DhcpSpec::default(),
            d2b_contracts_resource::v3::network::DnsSpec::default(),
            Some(external),
            d2b_contracts_resource::v3::network::MdnsSpec::default(),
            None,
            d2b_contracts_resource::v3::execution_policy::BoundedToken::parse("net-vm-base")
                .unwrap(),
            Vec::new(),
        )
        .unwrap();
        NetworkAdmissionIntent::new(
            NetworkAdmissionKey::new(
                zone_uid,
                network_uid,
                ResourceGeneration::new(1).unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ResourceBundleGenerationId::parse(
                    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                )
                .unwrap(),
            ),
            spec,
            Vec::new(),
        )
        .unwrap()
    }

    fn newer_network_admission_intent(current: &NetworkAdmissionIntent) -> NetworkAdmissionIntent {
        let key = NetworkAdmissionKey::new(
            current.key().zone_uid().clone(),
            current.key().network_uid().clone(),
            ResourceGeneration::new(current.key().network_generation().get() + 1).unwrap(),
            ResourceGeneration::new(current.key().attachment_generation().get() + 1).unwrap(),
            ResourceBundleGenerationId::parse(
                "sha256:abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd",
            )
            .unwrap(),
        );
        let spec = d2b_contracts_resource::v3::network::NetworkSpec::minimal(
            d2b_contracts_resource::v3::network::Ipv4Cidr::parse("10.20.0.0/24").unwrap(),
            d2b_contracts_resource::v3::network::Ipv4Cidr::parse("192.0.2.0/30").unwrap(),
            d2b_contracts_resource::v3::execution_policy::BoundedToken::parse("net-vm-base")
                .unwrap(),
        )
        .unwrap();
        NetworkAdmissionIntent::new(key, spec, Vec::new()).unwrap()
    }

    fn self_owned_occupancy(intent: &NetworkAdmissionIntent) -> HostNetworkOccupancy {
        let interface_markers = intent
            .interface_names()
            .iter()
            .filter_map(|ifname| {
                intent
                    .interface_ownership_marker(ifname)
                    .map(|marker| (ifname.clone(), marker.to_owned()))
            })
            .collect::<Vec<_>>();
        let route_markers = intent
            .routes()
            .iter()
            .filter_map(|route| {
                intent
                    .route_ownership_marker(route)
                    .map(|marker| (route.clone(), marker.to_owned()))
            })
            .collect::<Vec<_>>();
        let cidr_markers = intent
            .cidrs()
            .iter()
            .map(|cidr| (cidr.clone(), intent.ownership_marker().to_owned()))
            .collect::<Vec<_>>();
        HostNetworkOccupancy::from_route_tuples(
            intent.interface_names().to_vec(),
            intent.route_names().to_vec(),
            intent.routes().to_vec(),
            intent.cidrs().to_vec(),
        )
        .with_interface_ownership(interface_markers)
        .with_route_ownership(route_markers)
        .with_cidr_ownership(cidr_markers)
    }

    fn self_owned_kernel_occupancy(intent: &NetworkAdmissionIntent) -> HostNetworkOccupancy {
        let interface_markers = intent
            .interface_names()
            .iter()
            .filter_map(|ifname| {
                intent
                    .interface_ownership_marker(ifname)
                    .map(|marker| (ifname.clone(), marker.to_owned()))
            })
            .collect::<Vec<_>>();
        let route_markers = intent
            .routes()
            .iter()
            .filter_map(|route| {
                intent
                    .route_ownership_marker(route)
                    .map(|marker| (route.clone(), marker.to_owned()))
            })
            .collect::<Vec<_>>();
        let cidrs = vec![
            d2b_contracts_resource::v3::network::Ipv4Cidr::parse("10.20.0.1/24").unwrap(),
            d2b_contracts_resource::v3::network::Ipv4Cidr::parse("192.0.2.1/30").unwrap(),
        ];
        let cidr_markers = cidrs
            .iter()
            .map(|cidr| (cidr.clone(), intent.ownership_marker().to_owned()))
            .collect::<Vec<_>>();
        HostNetworkOccupancy::from_route_tuples(
            intent.interface_names().to_vec(),
            intent.route_names().to_vec(),
            intent.routes().to_vec(),
            cidrs,
        )
        .with_interface_ownership(interface_markers)
        .with_route_ownership(route_markers)
        .with_cidr_ownership(cidr_markers)
    }

    #[test]
    fn host_network_admission_rejects_overlapping_sibling_cidrs_atomically() {
        let mut index = HostNetworkAdmissionIndex::default();
        let first = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let second = network_admission_intent(
            "323e4567-e89b-42d3-a456-426614174002",
            "423e4567-e89b-42d3-a456-426614174003",
            "10.20.0.0/24",
            "198.51.100.0/30",
        );
        let occupancy = HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new());
        index.admit(first, &occupancy).unwrap();
        assert_eq!(
            index.admit(second, &occupancy),
            Err(NetworkEffectError::CidrConflict)
        );
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn host_network_admission_names_same_named_networks_by_uid() {
        let first = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let second = network_admission_intent(
            "323e4567-e89b-42d3-a456-426614174002",
            "423e4567-e89b-42d3-a456-426614174003",
            "10.30.0.0/24",
            "198.51.100.0/30",
        );
        assert_ne!(first.interface_names(), second.interface_names());
        assert_ne!(first.route_names(), second.route_names());
    }

    #[test]
    fn host_network_admission_counts_foreign_and_uidless_occupancy() {
        let mut index = HostNetworkAdmissionIndex::default();
        let intent = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let occupied = HostNetworkOccupancy::from_parts(
            vec![intent.interface_names()[0].clone()],
            vec![intent.route_names()[0].clone()],
            Vec::new(),
        );
        assert_eq!(
            index.admit(intent, &occupied),
            Err(NetworkEffectError::NetworkInterfaceCollision)
        );
        assert!(index.is_empty());
    }

    #[test]
    fn host_network_admission_counts_foreign_cidr_occupancy() {
        let mut index = HostNetworkAdmissionIndex::default();
        let intent = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let occupied = HostNetworkOccupancy::from_parts(
            Vec::new(),
            Vec::new(),
            vec![d2b_contracts_resource::v3::network::Ipv4Cidr::parse("10.20.1.0/23").unwrap()],
        );
        assert_eq!(
            index.admit(intent, &occupied),
            Err(NetworkEffectError::CidrConflict)
        );
        assert!(index.is_empty());
    }

    #[test]
    fn host_network_admission_counts_actual_route_tuple_occupancy() {
        let mut index = HostNetworkAdmissionIndex::default();
        let intent = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let occupied = HostNetworkOccupancy::from_route_tuples(
            Vec::new(),
            Vec::new(),
            vec![RouteTuple::new(
                "10.0.0.0/8",
                Some("192.0.2.1".to_owned()),
                Some(intent.routes()[0].device().unwrap_or("-").to_owned()),
                "254",
            )],
            Vec::new(),
        );
        assert_eq!(
            index.admit(intent, &occupied),
            Err(NetworkEffectError::NetworkRouteCollision)
        );
        assert!(index.is_empty());
    }

    #[test]
    fn host_network_admission_ignores_synthetic_route_ids_without_observed_tuple() {
        let mut index = HostNetworkAdmissionIndex::default();
        let intent = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let occupancy = HostNetworkOccupancy::from_parts(
            Vec::new(),
            vec![intent.route_names()[0].clone()],
            Vec::new(),
        );
        assert!(
            occupancy.routes().is_empty(),
            "a synthetic route name is not an observed kernel route tuple"
        );
        assert!(index.admit(intent, &occupancy).is_ok());
    }

    #[test]
    fn host_network_admission_scopes_route_collisions_by_actual_table() {
        let mut index = HostNetworkAdmissionIndex::default();
        let intent = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let occupied = HostNetworkOccupancy::from_route_tuples(
            Vec::new(),
            Vec::new(),
            vec![RouteTuple::new(
                "10.0.0.0/8",
                Some("192.0.2.1".to_owned()),
                Some("foreign0".to_owned()),
                "100",
            )],
            Vec::new(),
        );
        assert!(index.admit(intent, &occupied).is_ok());
    }

    #[test]
    fn host_network_admission_rejects_stale_network_generation() {
        let mut index = HostNetworkAdmissionIndex::default();
        let stale = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let current = newer_network_admission_intent(&stale);
        let occupancy = HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new());
        index.admit(current, &occupancy).unwrap();
        assert_eq!(
            index.admit(stale, &occupancy),
            Err(NetworkEffectError::NetworkAdmissionMismatch)
        );
    }

    #[test]
    fn host_network_admission_replaces_owner_and_ignores_self_owned_occupancy() {
        let mut index = HostNetworkAdmissionIndex::default();
        let first = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let occupancy = self_owned_kernel_occupancy(&first);
        let newer = newer_network_admission_intent(&first);

        index.admit(first, &occupancy).unwrap();
        let proof = index.admit(newer.clone(), &occupancy).unwrap();

        assert_eq!(proof.key(), newer.key());
        assert_eq!(index.len(), 1);
        assert_eq!(
            index
                .proof_for(newer.key().zone_uid(), newer.key().network_uid())
                .unwrap()
                .key(),
            newer.key()
        );
        assert_eq!(
            index.admit(newer.clone(), &occupancy).unwrap().key(),
            newer.key()
        );
        assert_eq!(
            index.admit(newer.clone(), &occupancy).unwrap().key(),
            newer.key()
        );
    }

    #[test]
    fn host_network_admission_rejects_stale_replacement_and_sibling_overlap_atomically() {
        let mut index = HostNetworkAdmissionIndex::default();
        let first = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let newer = newer_network_admission_intent(&first);
        let sibling = network_admission_intent(
            "323e4567-e89b-42d3-a456-426614174002",
            "423e4567-e89b-42d3-a456-426614174003",
            "10.20.0.0/24",
            "198.51.100.0/30",
        );
        let occupancy = self_owned_occupancy(&first);

        index.admit(first.clone(), &occupancy).unwrap();
        index.admit(newer.clone(), &occupancy).unwrap();
        assert_eq!(
            index.admit(first, &occupancy),
            Err(NetworkEffectError::NetworkAdmissionMismatch)
        );
        assert_eq!(
            index.admit(
                sibling,
                &HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new()),
            ),
            Err(NetworkEffectError::CidrConflict)
        );
        assert_eq!(index.len(), 1);
        assert_eq!(
            index
                .proof_for(newer.key().zone_uid(), newer.key().network_uid())
                .unwrap()
                .key(),
            newer.key()
        );
    }

    #[tokio::test]
    async fn host_network_admission_serializes_replacement_and_sibling_conflicts() {
        let first = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let newer = newer_network_admission_intent(&first);
        let sibling = network_admission_intent(
            "323e4567-e89b-42d3-a456-426614174002",
            "423e4567-e89b-42d3-a456-426614174003",
            "10.20.0.0/24",
            "198.51.100.0/30",
        );
        let mut initial = HostNetworkAdmissionIndex::default();
        initial
            .admit(
                first.clone(),
                &HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new()),
            )
            .unwrap();
        let index = Arc::new(tokio::sync::Mutex::new(initial));
        let replacement_index = Arc::clone(&index);
        let sibling_index = Arc::clone(&index);
        let replacement_occupancy = self_owned_kernel_occupancy(&first);
        let sibling_occupancy =
            HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new());
        let (replacement, sibling) = tokio::join!(
            async move {
                replacement_index
                    .lock()
                    .await
                    .admit(newer, &replacement_occupancy)
            },
            async move {
                sibling_index
                    .lock()
                    .await
                    .admit(sibling, &sibling_occupancy)
            },
        );
        assert!(replacement.is_ok());
        assert_eq!(sibling, Err(NetworkEffectError::CidrConflict));
        assert_eq!(index.lock().await.len(), 1);
    }

    #[test]
    fn host_network_admission_releases_only_after_confirmed_finalizer_completion() {
        let mut index = HostNetworkAdmissionIndex::default();
        let first = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let newer = newer_network_admission_intent(&first);
        let successor = newer_network_admission_intent(&newer);
        let occupancy = self_owned_occupancy(&first);
        index.admit(first.clone(), &occupancy).unwrap();

        assert!(!index.release_after_finalizer(first.key(), false));
        assert_eq!(index.len(), 1);
        assert!(index.admit(newer.clone(), &occupancy).is_ok());
        assert_eq!(index.len(), 1);
        assert!(!index.release_after_finalizer(first.key(), true));
        assert!(index.release_after_finalizer(newer.key(), true));
        assert!(index.is_empty());
        assert_eq!(
            index.admit(
                first,
                &HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new()),
            ),
            Err(NetworkEffectError::NetworkAdmissionMismatch)
        );
        assert!(
            index
                .admit(
                    successor,
                    &HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new()),
                )
                .is_ok()
        );
    }

    #[test]
    fn host_network_admission_rejects_unmarked_or_mismatched_identical_occupancy() {
        let first = network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
        );
        let newer = newer_network_admission_intent(&first);
        let unmarked = HostNetworkOccupancy::from_route_tuples(
            first.interface_names().to_vec(),
            first.route_names().to_vec(),
            first.routes().to_vec(),
            first.cidrs().to_vec(),
        );
        let mut index = HostNetworkAdmissionIndex::default();
        index
            .admit(
                first.clone(),
                &HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new()),
            )
            .unwrap();
        assert_eq!(
            index.admit(newer.clone(), &unmarked),
            Err(NetworkEffectError::CidrConflict)
        );

        let mut mismatched = self_owned_occupancy(&first);
        mismatched = mismatched.with_interface_ownership(vec![(
            first.interface_names()[0].clone(),
            "d2b managed: network:bridge:lan:zone:123e4567-e89b-42d3-a456-426614174000:network:223e4567-e89b-42d3-a456-426614174001:generation:99:attachment:99:bundle:sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        )]);
        let mut index = HostNetworkAdmissionIndex::default();
        index
            .admit(
                first.clone(),
                &HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new()),
            )
            .unwrap();
        assert_eq!(
            index.admit(newer.clone(), &mismatched),
            Err(NetworkEffectError::NetworkInterfaceCollision)
        );

        let mut mismatched_route = self_owned_occupancy(&first);
        mismatched_route = mismatched_route.with_route_ownership(vec![(
            first.routes()[0].clone(),
            "d2b managed: foreign".to_owned(),
        )]);
        let mut index = HostNetworkAdmissionIndex::default();
        index
            .admit(
                first,
                &HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new()),
            )
            .unwrap();
        let route_candidate = newer_network_admission_intent(&newer);
        assert_eq!(
            index.admit(route_candidate, &mismatched_route),
            Err(NetworkEffectError::NetworkRouteCollision)
        );
    }

    #[test]
    fn host_network_admission_rejects_cross_zone_external_bridge_multiplex() {
        let mut index = HostNetworkAdmissionIndex::default();
        let first = external_network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
            d2b_contracts_resource::v3::network::SharingPolicy::Multiplexed,
        );
        let second = external_network_admission_intent(
            "323e4567-e89b-42d3-a456-426614174002",
            "423e4567-e89b-42d3-a456-426614174003",
            "10.30.0.0/24",
            "198.51.100.0/30",
            d2b_contracts_resource::v3::network::SharingPolicy::Multiplexed,
        );
        let occupancy = HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new());
        index.admit(first, &occupancy).unwrap();
        assert_eq!(
            index.admit(second, &occupancy),
            Err(NetworkEffectError::CrossZoneL2)
        );
    }

    #[test]
    fn host_network_admission_rejects_same_zone_exclusive_external_reuse() {
        let mut index = HostNetworkAdmissionIndex::default();
        let first = external_network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "223e4567-e89b-42d3-a456-426614174001",
            "10.20.0.0/24",
            "192.0.2.0/30",
            d2b_contracts_resource::v3::network::SharingPolicy::Exclusive,
        );
        let second = external_network_admission_intent(
            "123e4567-e89b-42d3-a456-426614174000",
            "423e4567-e89b-42d3-a456-426614174003",
            "10.30.0.0/24",
            "198.51.100.0/30",
            d2b_contracts_resource::v3::network::SharingPolicy::Exclusive,
        );
        let occupancy = HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new());
        index.admit(first, &occupancy).unwrap();
        assert_eq!(
            index.admit(second, &occupancy),
            Err(NetworkEffectError::NetworkAdmissionConflict)
        );
    }



    #[test]
    fn gate_keeps_only_bindings_admitted_by_their_volume() {
        use d2b_provider_volume_local::testing::fixtures;
        let volume = fixtures::store_view_volume();
        let volume_ref = ResourceRef::parse("Volume/store-view-work-vm")
            .expect("volume ref");
        let intents = d2b_provider_volume_local::desired_binding_intents(
            volume_ref.clone(),
            &volume,
            false,
        )
        .expect("admitted intents");
        let intent = intents
            .first()
            .expect("store-view fixture declares one attachment");
        let admitted = d2b_contracts_resource::v3::volume_binding::VolumeBindingSpec::new(
            intent.volume_ref().clone(),
            intent.execution_ref().clone(),
            intent.view().as_str(),
            intent.access(),
            intent.mount_path(),
        )
        .expect("admitted spec");
        assert!(ZoneResourceRuntime::binding_admitted_by_volume_spec(
            &admitted,
            intent.name().as_str(),
            &volume
        ));
        let forged = d2b_contracts_resource::v3::volume_binding::VolumeBindingSpec::new(
            intent.volume_ref().clone(),
            ResourceRef::parse("Guest/victim-vm").expect("guest ref"),
            intent.view().as_str(),
            intent.access(),
            intent.mount_path(),
        )
        .expect("forged spec");
        assert!(!ZoneResourceRuntime::binding_admitted_by_volume_spec(
            &forged,
            "VolumeBinding/forged",
            &volume
        ));
    }

    const POLICY_INPUT_GUEST_UID: &str = "30e0427b-496f-4190-ba7c-879ccdec7964";
    const POLICY_INPUT_ROLE_UID: &str = "8a1a6f5e-3d2c-4a5f-9a10-9f0a1c2b3d4e";

    /// One committed policy-input row as the manager serves it: a complete,
    /// decodable resource envelope (the reader bridge renders exactly this
    /// shape), so the digest sees the same row facts the compile does.
    fn policy_input_row(
        resource_type: &str,
        name: &str,
        uid: &str,
        generation: u64,
        phase: ResourcePhase,
    ) -> StoredResource {
        use d2b_contracts_resource::v3::{
            CanonicalJsonObject, ManagedBy, ObservedGeneration, PresentationMetadata,
            ResourceCurrencySet, ResourceMetadata, ResourceName, ResourceSpec, ResourceStatus,
            ResourceUpdateStatus, UpdateDisruption, UpdateState,
        };
        let generation = ResourceGeneration::new(generation).expect("generation");
        let timestamp = Timestamp::parse("2026-09-12T00:00:00.000Z").expect("timestamp");
        let envelope = ResourceEnvelope::new(
            ResourceTypeName::parse(resource_type.to_owned()).expect("type"),
            ResourceMetadata::new(
                ResourceName::parse(name.to_owned()).expect("name"),
                ZoneId::parse("work").expect("zone"),
                ResourceUid::parse(uid.to_owned()).expect("uid"),
                generation,
                ZoneRevision::new(generation.get()),
                None,
                Vec::new(),
                None,
                timestamp.clone(),
                timestamp,
                ManagedBy::Api,
                None,
                None,
                None,
                PresentationMetadata::default(),
            )
            .expect("metadata"),
            ResourceSpec::empty(),
            ResourceStatus::new(
                ObservedGeneration::new(0),
                phase,
                Vec::new(),
                None,
                None,
                None,
                None,
                ResourceUpdateStatus::new(
                    UpdateState::Unknown,
                    Vec::new(),
                    ObservedGeneration::new(0),
                    ResourceGeneration::new(1).expect("generation one"),
                    UpdateDisruption::None,
                    true,
                    None,
                    None,
                    ResourceCurrencySet::new(0, Vec::new()).expect("currency"),
                    ResourceCurrencySet::new(0, Vec::new()).expect("currency"),
                )
                .expect("update status"),
                CanonicalJsonObject::empty(),
                None,
            )
            .expect("status"),
        )
        .expect("envelope");
        StoredResource {
            resource_ref: ResourceRef::parse(&format!("{resource_type}/{name}")).expect("ref"),
            zone: ZoneId::parse("work").expect("zone"),
            uid: ResourceUid::parse(uid.to_owned()).expect("uid"),
            owner_uid: None,
            owner_generation: None,
            generation,
            revision: ZoneRevision::new(generation.get()),
            canonical_json: envelope.canonical_bytes().expect("canonical bytes"),
            payload_digest: envelope.digest().expect("envelope digest"),
        }
    }

    fn installed_policy_projection(installed_revision: Option<u64>) -> PolicyProjection {
        let zone = ZoneId::parse("work").expect("zone");
        let snapshot = initial_policy_snapshot().expect("snapshot");
        let (_, state) = runtime_policy(
            &zone,
            &snapshot,
            ZoneRevision::new(installed_revision.unwrap_or(1)),
            &[],
        )
        .expect("bootstrap policy");
        PolicyProjection {
            authorizer: Arc::new(runtime_authorizer(&[]).expect("authorizer")),
            manager_authorizer: None,
            bus: Arc::new(Mutex::new(None)),
            authorization_state: Arc::new(Mutex::new(installed_revision.map(|_| state))),
            policy_refresh: Arc::new(Mutex::new(())),
            policy_loaded: Arc::new(Mutex::new(installed_revision.is_some())),
            installed_controller_subjects: Arc::new(Mutex::new(BTreeSet::new())),
            installed_policy_inputs: Arc::new(Mutex::new(None)),
        }
    }

    /// U14: the committed policy rows are the whole policy input set, and the
    /// derived revision is not a change signal for them. Removing the row
    /// that carried the highest generation - the Guest row when its teardown
    /// completes - lowers the maximum, and a tombstoned row keeps both its
    /// generation and its uid; the digest is what still tells a refresh the
    /// inputs moved.
    #[test]
    fn policy_input_digest_moves_when_a_row_removal_lowers_the_derived_revision() {
        let role = policy_input_row(
            "Role",
            "operator",
            POLICY_INPUT_ROLE_UID,
            1,
            ResourcePhase::Ready,
        );
        let guest = policy_input_row(
            "Guest",
            "acceptance-guest",
            POLICY_INPUT_GUEST_UID,
            3,
            ResourcePhase::Ready,
        );
        let with_guest = vec![role.clone(), guest.clone()];
        let without_guest = vec![role.clone()];
        assert!(
            ResourceEnvelope::from_json(&guest.canonical_json).is_ok(),
            "the fixture rows must be the decodable envelopes the manager serves",
        );

        assert_eq!(
            policy_snapshot_for_rows(&with_guest)
                .expect("snapshot")
                .policy_revision,
            3,
        );
        assert_eq!(
            policy_snapshot_for_rows(&without_guest)
                .expect("snapshot")
                .policy_revision,
            1,
            "removing the highest-generation row lowers the derived revision",
        );
        assert_ne!(
            policy_inputs_digest(&with_guest),
            policy_inputs_digest(&without_guest),
            "the removed row must still be visible to the refresh",
        );

        // Read order is a property of the manager call, not of the inputs.
        assert_eq!(
            policy_inputs_digest(&with_guest),
            policy_inputs_digest(&[guest, role.clone()]),
        );
        // A tombstone keeps generation and uid but changes the evidence the
        // compile reads, so it must move the digest too.
        let tombstoned = policy_input_row(
            "Role",
            "operator",
            POLICY_INPUT_ROLE_UID,
            1,
            ResourcePhase::Deleted,
        );
        assert_eq!(
            policy_input_content(&tombstoned).1,
            b"deleted".as_slice(),
            "a deleted row is the tombstone class the compile drops grants for",
        );
        assert_ne!(
            policy_inputs_digest(&without_guest),
            policy_inputs_digest(&[tombstoned]),
        );
    }

    /// The Zone bus fences every policy install at the installed revision
    /// (`BusAuthorizer::replace_policy`), so a refresh following a row
    /// removal must not hand its compile the regressed derived revision: it
    /// advances past the installed one instead - the retired durable store's
    /// commit counter advanced on every commit - and holds it when the inputs
    /// did not move.
    #[test]
    fn policy_revision_never_regresses_against_the_installed_projection() {
        let projection = installed_policy_projection(Some(2));
        assert_eq!(
            projection.next_policy_revision(1, true),
            3,
            "a changed input set advances past the installed revision",
        );
        assert_eq!(
            projection.next_policy_revision(7, true),
            7,
            "a row generation that did advance keeps its own revision",
        );
        assert_eq!(
            projection.next_policy_revision(1, false),
            2,
            "an unchanged input set holds the installed revision",
        );
        let unloaded = installed_policy_projection(None);
        assert_eq!(
            unloaded.next_policy_revision(1, true),
            1,
            "with no installed projection the rows dictate the revision",
        );
    }

    /// Regression (P2): the manager leg is the manager's complete answer, so
    /// folding it over the durable leg must not be truncated by the durable
    /// page bound. The pre-fix merge refused the union as `Truncated` as soon
    /// as an unseen manager row crossed 256 - a Zone holding 85 guests' worth
    /// of converted children failed every child relist - and the manager's
    /// rendering must keep winning where both planes hold a reference.
    #[test]
    fn manager_rows_merge_over_the_durable_page_bound() {
        let merge_row =
            |index: u64, revision: u64| merge_test_row(&format!("Process/owned-{index}"), revision);
        let mut merged = (0..256u64)
            .map(|index| merge_row(index, 1))
            .collect::<Vec<_>>();
        let manager_rows = (0..300u64)
            .map(|index| merge_row(index, 2))
            .collect::<Vec<_>>();

        merge_manager_rows(&mut merged, manager_rows);

        assert_eq!(
            merged.len(),
            300,
            "every manager row must survive the merge, past the 256-row page cap",
        );
        assert!(
            merged
                .iter()
                .all(|row| row.revision == ZoneRevision::new(2)),
            "the manager rendering must win where both planes hold the reference",
        );
    }

    /// The child relist's owner fence: the request's `guest_ref` scopes both
    /// legs, but the manager leg is scoped to this session's own owner, so a
    /// session for Guest A asking for Guest B used to be answered with A's
    /// manager rows while B's manager-only rows were dropped. The request's
    /// owner must be the plane owner; a session with no published plane keeps
    /// the durable leg's own `guest_ref` filter.
    #[test]
    fn child_relist_fences_the_request_owner_against_the_plane_owner() {
        let owner = ResourceRef::parse("Guest/acceptance-guest").expect("owner ref");
        let other = ResourceRef::parse("Guest/other-guest").expect("other ref");

        assert_eq!(fence_relist_owner(Some(&owner), &owner), Ok(()));
        assert_eq!(
            fence_relist_owner(Some(&owner), &other),
            Err(CloudHypervisorResourceApiError::Conflict),
            "a session for another Guest never answers a foreign relist"
        );
        assert_eq!(
            fence_relist_owner(None, &other),
            Ok(()),
            "no published plane: the durable leg's own owner filter stands"
        );
    }

    /// One stored row for the manager/durable merge: identity and revision
    /// are all the merge reads.
    fn merge_test_row(resource_ref: &str, revision: u64) -> StoredResource {
        StoredResource {
            resource_ref: ResourceRef::parse(resource_ref).expect("ref"),
            zone: ZoneId::parse("work").expect("zone"),
            uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174099").expect("uid"),
            owner_uid: None,
            owner_generation: None,
            generation: ResourceGeneration::new(1).expect("generation"),
            revision: ZoneRevision::new(revision),
            canonical_json: Vec::new(),
            payload_digest: String::new(),
        }
    }
}
