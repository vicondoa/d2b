//! Production Zone resource-plane ownership for `d2bd`.
//!
//! A Zone runtime is opened only from the broker's opaque
//! [`d2b_contracts_broker::broker_wire::OpenZoneStoreRequest`]. The broker owns path
//! resolution and returns one
//! close-on-exec database descriptor; this module consumes that descriptor
//! into the production redb backend and never opens a caller-supplied path.
//! The runtime owns the API, core-process readiness, and restart lifecycle as
//! one Zone-scoped value.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(test)]
use std::sync::atomic::AtomicUsize;
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use crate::ServerState;
use crate::audio_resource_runtime::{AudioBindingRuntimeStatus, AudioResourceRuntime};
use crate::credential_driver::{
    AgentReadyFuture, CredentialDependencyFacts, CredentialDriverEffects,
    ProductionCredentialDriverEffects,
};
use crate::credential_resource_runtime::{
    CredentialSession, CredentialSessionRegistry, ComponentCredentialSession,
    is_credential_provider_ref,
};
use crate::process_resource_runtime::{ProcessResourceRuntimeError, list_process_resources};
use async_trait::async_trait;
use d2b_audit::{AuditSink, DurabilityEvidence};
use d2b_bus::{
    BusAuthorizer, BusConfig, BusIngress, CommittedControllerProcessSubjectInput,
    CommittedInteractionSubjectInstall, CommittedInteractionSubjectIssuer, ZoneBus, ZoneRegistrar,
};
#[cfg(test)]
use d2b_contracts_broker::broker_wire::OpenZoneStoreResponse;
use d2b_contracts_broker::broker_wire::ZoneStoreDisposition;
#[cfg(test)]
use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_contracts_provider::v3::provider::ProviderSpec;
use d2b_contracts_resource::resource_proto as wire;
#[cfg(test)]
use d2b_contracts_resource::v3::ConfigurationGeneration;
use d2b_contracts_resource::v3::identity::{
    AuthenticatedSubjectContext, BindingDigest, EvidenceClass, ReconnectGeneration,
};
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ControllerGeneration, DesiredLifecycle,
    PlacementTargetKind, ResourceBundleGenerationId, ResourceEnvelope, ResourceGeneration,
    ResourceErrorKind, ResourceName, ResourcePhase, ResourceRef, ResourceTypeName, ResourceUid,
    ZoneId, ZoneRevision,
    process::ProcessSpec,
    volume::VolumeSpec,
};
use d2b_contracts_resource::v3::guest::GuestSpec;
use d2b_contracts_zone_session::v3::{ZoneStatusResource, resource_bundle::ResourceBundle};
use d2b_core_controller::authority::{
    AuthorityRequest, AuthorityReservation, ExternalNicClaimRequest, ExternalNicRecoveryInventory,
    ExternalNicReservation, HostGlobalAuthorityIndex, TrustedExternalNicInventory,
};
use d2b_core_controller::authority_persistence::AuthorityRecoveryCoordinator;
use d2b_core_controller::controller_assignment::{
    AssignmentError, AssignmentIdentity, AssignmentPhase, AssignmentRequest, AssignmentTarget,
    CONTROLLER_ASSIGNMENT_STREAM_CREDIT, CONTROLLER_ASSIGNMENT_STREAM_ID,
    ControllerAssignmentGrant, ControllerRoleContract, ControllerSessionBinding,
    ResourceClientLease,
};
use d2b_core_controller::controllers::HandlerPhase;
use d2b_core_controller::{SelectorField, SourceError};
#[cfg(test)]
use d2b_core_controller::ControllerDescriptor;
#[cfg(test)]
use d2b_core_controller::{DependencySnapshot, ResourceKey, ResourceSnapshot};
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
#[cfg(test)]
use d2b_provider_runtime_azure_container_apps as aca_runtime;
#[cfg(test)]
use d2b_provider_runtime_azure_virtual_machine as azure_vm_runtime;
#[cfg(test)]
use d2b_provider_runtime_qemu_media as qemu_media_runtime;
use d2b_resource_api::{
    RedbBackend, ResourceApiClient, ResourceBusAdapter, ResourceService, ResourceStoreBackend,
    authz::{AuthorizationState, BoundSubject, NativeAuthorizer, PolicySet},
    registered::{AssignmentFenceResolver, RedbRegisteredControllerApi},
    service::UnavailableUpgradeDispatcher,
};
use d2b_resource_store::{
    ExpectedRevision, PolicySnapshot, ResourceAssignmentFence, ResourceAssignmentScope,
    ResourceMutationKind, StoreError, StoreErrorKind, StoreGetRequest, StoreListRequest,
    StoreListResult, StoreMutation, StoreOperationContext, StoreProjection, StoredResource,
};
use d2b_resource_store_redb::{
    AuthorityOperationState, BrokerEvidenceIndex, LogicalBackup, RedbResourceStore,
    StoreRuntimeMetadata, write_provisioning_marker,
};
use d2b_session::{
    ComponentSessionDriver, HandshakeCredentials, SessionDriverHandle, SessionEngine,
    SessionServerError, StreamEvent, StreamId, TransportEvidence,
};
use d2b_session_unix::{
    AncillaryCapacity, CONTROLLER_BOOTSTRAP_TIMEOUT, PeerCredentials, SeqpacketSocket,
    VerifiedUnixPeer, controller_bootstrap_attachment_policy, controller_credit_scopes,
    controller_resource_endpoint_policy, credential_provider_endpoint_policy,
};
use d2bd_runtime::authority_persistence::RedbAuthorityPersistence;
pub use d2bd_runtime::resource_api::ResourceRuntimeError;
use d2bd_runtime::resource_api::{parse_list_request, route_service_matches};
use d2bd_runtime::resource_operator_activation::{
    Wave6AcceptanceReport, Wave6Dependencies, Wave6ProviderBoundary, Wave6ReconcileResult,
    select_wave6_resources,
};
#[cfg(test)]
use d2bd_runtime::resource_runtime_support::compatibility_error_envelope;
use d2bd_runtime::resource_runtime_support::{
    AssignmentRegistry, PolicySubjectFingerprint, SystemCoreReconcileResult, ZoneStoreBackend,
    configuration_cleanup_pending, current_status_timestamp, encode_public_get_response,
    encode_public_list_response, encode_public_resource, ensure_bootstrap_host_resource,
    ensure_bootstrap_zone_resource, handler_phase_to_zone_phase,
    initial_policy_snapshot, map_startup_error, materialize_zone_resource_bundle,
    new_assignment_registry, public_list_request, public_operation_id, public_request_meta,
    persist_resource_controller_session_evidence, refreshed_policy_subject_fingerprints,
    register_system_core_session, retry_transient_store_list, retry_transient_store_read,
    runtime_authorizer, runtime_policy, store_identity,
    store_identity_for_authority, unix_transport, validate_zone_resource_bundle,
    validate_zone_self_resource, zone_runtime_metadata,
};
use d2bd_runtime::guest_component_session::COMPONENT_SESSION_RETRY_BACKOFF;
pub use d2bd_runtime::resource_runtime_support::{
    ZoneRuntimeReadiness, bounded_operation_id, persist_resource_status_with_projection,
};
use d2bd_runtime::resource_store_runtime::{MAX_ZONE_RUNTIMES, OpenedZoneStore};
use d2bd_runtime::target_runtime::{DaemonMode, ProviderDeployment};
use d2bd_runtime::zone_authority::{
    ZONE_GENERATION_PUBLICATION_OPERATION_PREFIX, ZoneAuthorityIdentity,
    complete_generation_set_digest,
};
#[cfg(test)]
use nix::unistd::{Uid, User};
use protobuf::{EnumOrUnknown, MessageField};
use serde::Deserialize;
use serde_json::{Value, json};

mod volume_effect_adapter;
mod guest_provider_runtime;
mod plane_controller_bridge;
mod shared_provider_runtime;
pub(crate) mod interaction_effects;
use plane_controller_bridge::{
    ControllerPlaneView, LiveControllerSessionEvidence, ManagerControllerPlaneView,
    PublishedPlaneControllerView,
};
use d2b_resource_runtime::manager::ResourceView;
pub use guest_provider_runtime::{
    compose_shared_guest_runner_descriptors, SharedGuestRunnerRegistration,
    U6_SHARED_PROVIDER_RUNNERS,
};
pub use volume_effect_adapter::{
    AnchoredVolumeEffectAdapter, FdRootResolver, ResolvedVolumeRoot, VolumeRootResolver,
};
pub use shared_provider_runtime::compose_shared_provider_runner_descriptors;
pub(crate) use interaction_effects::ProductionInteractionDriverEffects;
pub(crate) use shared_provider_runtime::{
    DaemonSharedProviderEffects, GuestRuntimeReconciler, SharedProviderEffectExecutor,
    SharedProviderResourceKind,
};
#[cfg(test)]
pub(crate) use shared_provider_runtime::SharedProviderResourceReconciler;
use crate::interaction_driver::INTERACTION_PROVIDER_REFS;
use shared_provider_runtime::UnavailableSharedProviderEffects;
#[cfg(test)]
use shared_provider_runtime::{
    FrameworkAcaControl, FrameworkAcaLease, FrameworkAcaState, FrameworkAzureCredential,
    FrameworkAzureEffect, FrameworkAzureState, FrameworkQemuEffect, GuestRuntimeController,
};

#[cfg(test)]
#[allow(unused_imports)]
use shared_provider_runtime::finalizer_candidate;

/// Bounded attempts to recompile the manager-plane authorization projection
/// after the Zone's plane is published. Each attempt costs a policy recompile
/// and a system-core session rebind, so the budget stays small; a public
/// request that arrives before the manager's subject rows settle retries on a
const CORE_CONTROLLER_PROCESS_REF: &str = "Process/d2b-core-controller";
const CORE_CONTROLLER_PROVIDER_REF: &str = "Provider/system-core";
const CORE_CONTROLLER_HOST_REF: &str = "Host/host-system";
/// Bounded attempts when a policy-input change races the authorization
/// policy projection refresh. The projection compiles the committed policy
/// rows against the policy snapshot; a policy change landing under the
/// refresh must not surface as a failed public mutation.
const POLICY_REFRESH_ATTEMPTS: u32 = 8;
const POLICY_REFRESH_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(50);

fn trusted_provider_resource_types() -> Result<Vec<ResourceTypeName>, ResourceRuntimeError> {
    let mut resource_types = BTreeSet::new();
    for resource_type in U6_SHARED_PROVIDER_RUNNERS
        .iter()
        .map(|registration| registration.resource_type)
        .chain(crate::interaction_driver::INTERACTION_TYPES)
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
    // Qualified API extensions come only from trusted U6 runner and U9 driver declarations.
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

/// One Provider-owned ResourceType registration served by the shared Runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedProviderRunnerRegistration {
    /// Static controller process identity.
    pub controller_ref: &'static str,
    /// Provider identity selected by the Resource spec.
    pub provider_ref: &'static str,
    /// ResourceType owned by this runner.
    pub resource_type: &'static str,
    /// Exact finalizer installed by the owner.
    pub finalizer: &'static str,
    /// Descriptor repair/resync interval in runner ticks.
    pub repair_interval_ticks: u64,
    /// Whether watched configuration is dependency-only.
    pub watched_configuration_is_dependency: bool,
}

/// Compat assignment-epoch value written into every newly constructed
/// assignment fence and stored AssignmentRecord. Epochs no longer take part
/// in any decision (succession is read off provider/controller/session
/// generations plus resource revisions, and reconnect reconciliation adopts
/// every fence not strictly newer); the constant only keeps the retained
/// schema and stored-record validation (a nonzero epoch) intact.
pub(super) const ASSIGNMENT_EPOCH: u64 = 1;

#[derive(Clone)]
pub(super) struct CoreAssignmentAuthority {
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    session_generation: ReconnectGeneration,
    controller_role: ResourceRef,
    target: ResourceRef,
}

/// Whether a stored assignment fence conflicts with the current authority.
///
/// Succession is read off the monotonic authority axes. A stored fence that
/// is strictly newer on any axis (provider, controller, or reconnect session
/// generation) means a concurrent or future writer is active: conflict. A
/// stored fence that is older-or-equal on every axis is a predecessor from a
/// superseded authority: the reconnecting successor adopts it and re-fences
/// on write, instead of dying and wedging the runner forever. Role/target
/// drift at fully equal axes is still a conflict: the same writer identity
/// must not silently change its binding mid-session.
pub(super) fn assignment_fence_conflict(
    stored: &ResourceAssignmentFence,
    uid: &ResourceUid,
    authority: &CoreAssignmentAuthority,
) -> bool {
    let strictly_newer = stored.provider_generation > authority.provider_generation
        || stored.controller_generation > authority.controller_generation
        || stored.session_generation > authority.session_generation;
    let same_axes = stored.provider_generation == authority.provider_generation
        && stored.controller_generation == authority.controller_generation
        && stored.session_generation == authority.session_generation;
    let conflict = stored.resource_uid != *uid
        || strictly_newer
        || (same_axes
            && (stored.controller_role != authority.controller_role
                || stored.target != authority.target));
    // A conflict kills the reconciling runner; log both sides so the
    // succession that produced it is diagnosable from the journal alone.
    if conflict {
        tracing::warn!(
            stored_provider_generation = stored.provider_generation.get(),
            stored_controller_generation = stored.controller_generation.get(),
            stored_session_generation = stored.session_generation.get(),
            authority_provider_generation = authority.provider_generation.get(),
            authority_controller_generation = authority.controller_generation.get(),
            authority_session_generation = authority.session_generation.get(),
            same_role = stored.controller_role == authority.controller_role,
            same_target = stored.target == authority.target,
            "assignment fence conflict",
        );
    }
    conflict
}
/// Read the durable facts the Credential driver gates reconcile and delete
/// on: the Provider row and the declared execution target, each `Ready` at
/// its current generation (the old dependency snapshots' predicate).
/// The manager rows of one type through a plane-view seam, rendered through
/// the same projection the manager-backed API serves (U12 reader bridge,
/// mirroring G5).
///
/// Converted types live in the manager, not in the pre-v3 store, so the
/// readers that still take a store handle merge these in. `None` (no
/// published plane), an unconverted type, or a manager without the rows
/// leaves the caller on the durable store path; a manager RPC failure is an
/// error - never reported as absence.
async fn bridge_manager_rows(
    plane: Option<&dyn ControllerPlaneView>,
    resource_type: &str,
) -> Result<Vec<StoredResource>, ResourceRuntimeError> {
    let Some(plane) = plane else {
        return Ok(Vec::new());
    };
    if crate::resource_plane_v3::route_resource_type(resource_type)
        != crate::resource_plane_v3::PlaneRoute::NewPlane
    {
        return Ok(Vec::new());
    }
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

/// Merge the manager-served rows of one type into a store-shaped reader
/// result, keyed by resource reference: the manager row wins where both
/// planes hold the same reference (the manager is the execution authority;
/// the store copy is the pre-v3 mirror U14 deletes).
async fn bridge_merge_rows(
    plane: Option<&dyn ControllerPlaneView>,
    resource_type: &str,
    rows: &mut Vec<StoredResource>,
) -> Result<(), ResourceRuntimeError> {
    for row in bridge_manager_rows(plane, resource_type).await? {
        match rows
            .iter_mut()
            .find(|existing| existing.resource_ref == row.resource_ref)
        {
            Some(existing) => *existing = row,
            None => rows.push(row),
        }
    }
    Ok(())
}

/// The multi-type form of [`bridge_merge_rows`].
async fn bridge_merge_rows_for_types(
    plane: Option<&dyn ControllerPlaneView>,
    resource_types: &[&str],
    rows: &mut Vec<StoredResource>,
) -> Result<(), ResourceRuntimeError> {
    for resource_type in resource_types {
        bridge_merge_rows(plane, resource_type, rows).await?;
    }
    Ok(())
}

/// The committed policy inputs for one Zone, durable rows merged with the
/// manager-served ones (U12 bridge; the policy loader itself is redb-only).
///
/// Role/RoleBinding/Zone/Provider and the subject rows are converted types,
/// so the manager is their execution authority; without the merge a
/// RoleBinding whose Role row has moved to the manager returns
/// `AuthorizationUnavailable` and its subjects are dropped - i.e. the Zone's
/// committed policy would empty out. A miss on both planes stays the loud,
/// fail-closed compile failure it is today.
async fn committed_policy_resources_bridged(
    zone: &ZoneId,
    store: &RedbResourceStore,
    plane: Option<&dyn ControllerPlaneView>,
    operation_id: &str,
) -> Result<Vec<StoredResource>, ResourceRuntimeError> {
    let mut resources = d2bd_runtime::resource_runtime_support::load_committed_policy_resources(
        store, zone, operation_id,
    )
    .await?;
    bridge_merge_rows_for_types(
        plane,
        &d2bd_runtime::resource_runtime_support::COMMITTED_POLICY_RESOURCE_TYPES,
        &mut resources,
    )
    .await?;
    Ok(resources)
}

/// The committed identities of the requested `Provider` refs, manager rows
/// first and the durable store only for what the manager does not serve (U12
/// bridge).
///
/// The controller session/policy paths compare a bootstrap context against
/// the committed Provider identity, and `Provider` is a manager row now: the
/// per-ref durable read would fail closed on every ref whose row moved, so
/// the manager is consulted first and the store keeps the pre-v3 rows U14
/// deletes.
async fn committed_controller_provider_identities_bridged(
    zone: &ZoneId,
    store: &RedbResourceStore,
    plane: Option<&dyn ControllerPlaneView>,
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
        let revision = row.revision;
        let expected_ref = row.resource_ref.clone();
        let (_, uid, generation, _, _) =
            committed_provider_spec(zone, revision, &row, &expected_ref)?;
        identities.insert(expected_ref, (uid, generation));
    }
    let remaining = provider_refs
        .iter()
        .filter(|provider_ref| !identities.contains_key(*provider_ref))
        .cloned()
        .collect::<BTreeSet<_>>();
    if !remaining.is_empty() {
        identities.extend(
            load_committed_controller_provider_identities(zone, store, remaining).await?,
        );
    }
    Ok(identities)
}

async fn credential_dependency_facts(
    store: &RedbResourceStore,
    zone: &ZoneId,
    provider_ref: &ResourceRef,
    execution_ref: &ResourceRef,
) -> Option<CredentialDependencyFacts> {
    let provider = credential_dependency_row(store, zone, provider_ref, "credential-provider").await?;
    let execution =
        credential_dependency_row(store, zone, execution_ref, "credential-execution").await?;
    Some(CredentialDependencyFacts {
        provider_uid: provider.uid.as_str().to_owned(),
        provider_generation: provider.generation.get(),
        provider_ready: credential_row_ready(&provider),
        execution_ready: credential_row_ready(&execution),
    })
}

async fn credential_dependency_row(
    store: &RedbResourceStore,
    zone: &ZoneId,
    target: &ResourceRef,
    operation: &str,
) -> Option<StoredResource> {
    let operation_id = format!("{operation}-{}", target.name().as_str());
    let request = StoreGetRequest {
        operation: StoreOperationContext {
            operation_id: operation_id.clone(),
            idempotency_key: None,
            correlation_id: operation_id.clone(),
            trace_id: None,
            deadline_ms: 10_000,
        },
        zone: zone.clone(),
        target: target.clone(),
        expected_uid: None,
        projection: StoreProjection::Full,
    };
    retry_transient_store_read(zone, &operation_id, || store.get(request.clone()))
        .await
        .ok()
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

    #[cfg(test)]
    pub(crate) fn allows_guest_source(&self, source: &ResourceRef) -> bool {
        self.guest_sources.contains(source)
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
    resource_client: Option<Arc<ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>>>,
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
    store: Arc<RedbResourceStore>,
    /// G5 bridge: the zone plane's manager view for converted rows. Set by
    /// [`ZoneResourceRuntime::attach_v3_planes`] once the composition
    /// publishes the plane; absent keeps every controller-session read on
    /// the durable store path.
    plane_view: Arc<Mutex<Option<Arc<dyn ControllerPlaneView>>>>,
    assigned_process_api: Arc<Mutex<Option<Arc<RedbRegisteredControllerApi>>>>,
    api: Arc<ResourceService<ZoneStoreBackend>>,
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
    #[cfg(test)]
    reconcile_attempts: Arc<AtomicUsize>,
    #[cfg(test)]
    admission_test_seam: Arc<Mutex<Option<ControllerSessionAdmissionTestSeam>>>,
    #[cfg(test)]
    controller_session_evidence_test_errors: Arc<Mutex<Vec<ResourceRuntimeError>>>,
}

type CloudHypervisorResourceClient = ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>;

#[cfg(test)]
#[derive(Clone)]
struct ControllerSessionAdmissionTestSeam {
    after_snapshot:
        Arc<dyn Fn(&crate::process_provider_runtime::ProductionProcessProviders) + Send + Sync>,
    after_policy_install: Arc<dyn Fn(&BTreeSet<BoundSubject>) + Send + Sync>,
    admit_without_transport: bool,
}

#[cfg(test)]
struct ControllerSessionAdmissionTestUnwindGuard {
    first_policy_release: Arc<AtomicBool>,
    second_snapshot_release: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    wake: Arc<tokio::sync::Notify>,
}

#[cfg(test)]
impl Drop for ControllerSessionAdmissionTestUnwindGuard {
    fn drop(&mut self) {
        self.first_policy_release.store(true, Ordering::Release);
        self.second_snapshot_release
            .store(true, Ordering::Release);
        self.shutdown.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }
}

#[derive(Clone)]
struct PolicyProjection {
    authorizer: Arc<NativeAuthorizer>,
    /// The manager plane's authorizer: same catalog and policy as the
    /// primary, its own mutation seal for the manager-backed API service
    /// (U8/U9 F1 wiring). `None` for runtimes that never serve the v3 plane.
    manager_authorizer: Option<Arc<NativeAuthorizer>>,
    bus: Option<Arc<ZoneBus>>,
    authorization_state: Arc<Mutex<Option<AuthorizationState>>>,
    policy_refresh: Arc<Mutex<()>>,
    policy_loaded: Arc<Mutex<bool>>,
    installed_controller_subjects: Arc<Mutex<BTreeSet<BoundSubject>>>,
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
        let install_result = if let Some(bus) = &self.bus {
            bus.replace_policy(policy, state.clone())
                .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)
        } else {
            self.authorizer
                .replace_policy(policy, &state)
                .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)
        };
        if let Err(error) = install_result {
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
        if let Some(bus) = &self.bus {
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
    assigned_mutation_api: Arc<RedbRegisteredControllerApi>,
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
    let binding = GuestSessionEvidenceBinding::new(
        identity.guest_uid().to_canonical_string(),
        descriptor.descriptor().descriptor_digest().as_str(),
        identity.schema_fingerprint().as_str(),
        identity.provider_generation(),
        identity.controller_generation(),
        session.generation(),
        route.reconnect_generation().get(),
        target.endpoint_generation().get(),
        1,
    )
    .inspect_err(|_| {
        tracing::debug!(
            guest = %guest_ref.to_canonical_string(),
            "guest session evidence binding construction failed",
        );
    })
    .ok()?;
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

impl std::fmt::Debug for CloudHypervisorResourceSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CloudHypervisorResourceSession(<redacted>)")
    }
}

fn cloud_hypervisor_assigned_mutation_error(
    error: SourceError,
) -> CloudHypervisorResourceApiError {
    match error {
        SourceError::Conflict(_) | SourceError::Integrity => {
            CloudHypervisorResourceApiError::Conflict
        }
        SourceError::Unavailable
        | SourceError::Backpressure
        | SourceError::Cancelled
        | SourceError::Timeout => CloudHypervisorResourceApiError::Transport,
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

    async fn get_stored(
        &self,
        target: &ResourceRef,
        operation: &str,
    ) -> Result<StoredResource, CloudHypervisorResourceApiError> {
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
        Ok(resource)
    }

    async fn list_stored(
        &self,
        resource_types: &[&str],
        owner_uid: Option<&ResourceUid>,
        operation: &str,
    ) -> Result<Vec<StoredResource>, CloudHypervisorResourceApiError> {
        let mut request = wire::ListRequest::new();
        request.meta = MessageField::some(public_request_meta(operation));
        request.resource_types = resource_types
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
        loop {
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

    async fn authenticated_guest_session(
        &self,
        guest_ref: &ResourceRef,
        guest_uid: &ResourceUid,
    ) -> Result<
        Arc<d2bd_runtime::guest_component_session::GuestComponentSessionClient>,
        CloudHypervisorResourceApiError,
    > {
        self.guest_for_fenced_operation(guest_ref, guest_uid, "cloud-hypervisor-guest-session")
            .await?;
        let key = self.session_key(guest_ref, guest_uid)?;
        let session = self
            .guest_sessions
            .lock()
            .await
            .get(&key)
            .cloned()
            .ok_or(CloudHypervisorResourceApiError::Authentication)?;
        if session.identity().zone() != &self.zone
            || session.identity().guest_ref() != guest_ref
            || session.identity().guest_uid() != guest_uid
        {
            return Err(CloudHypervisorResourceApiError::Conflict);
        }
        Ok(session)
    }

    async fn close_guest_session(
        &self,
        guest_ref: &ResourceRef,
        guest_uid: &ResourceUid,
    ) -> Result<(), CloudHypervisorResourceApiError> {
        let _ = self
            .authenticated_guest_session(guest_ref, guest_uid)
            .await?;
        let key = self.session_key(guest_ref, guest_uid)?;
        let mut sessions = self.guest_sessions.lock().await;
        let removed = if sessions
            .get(&key)
            .is_some_and(|session| session.identity().guest_uid() == guest_uid)
        {
            sessions.remove(&key);
            true
        } else {
            false
        };
        drop(sessions);
        if removed {
            self.closed_guest_sessions.lock().await.insert(key);
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
        let session = self
            .authenticated_guest_session(guest_ref, guest_uid)
            .await?;
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
            GuestGenerationSet {
                provider: self.descriptor.descriptor().provider_generation().get(),
                descriptor: self.descriptor.descriptor().provider_generation().get(),
                controller: self.controller_generation.get(),
                child: guest.generation.get(),
                session: self
                    .session_evidence
                    .as_ref()
                    .and_then(GuestSessionEvidence::session_generation)
                    .unwrap_or(0),
            },
            deleting,
        )
        .map_err(|_| {
            tracing::warn!("Cloud Hypervisor Guest snapshot failed: construction");
            CloudHypervisorResourceApiError::InvalidResponse
        })?
        .with_controller_finalizer_present(envelope.metadata().finalizers().iter().any(
            |finalizer| {
                finalizer.as_str()
                    == d2b_provider_runtime_cloud_hypervisor::GUEST_CONTROLLER_FINALIZER
            },
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
                let guest = self
                    .get_stored(&guest_ref, "cloud-hypervisor-get-guest")
                    .await?;
                Ok(CloudHypervisorResourceResponse::Guest(
                    self.snapshot_from_stored(&guest)?,
                ))
            }
            CloudHypervisorResourceRequest::RelistOwnedChildren {
                guest_ref,
                expected_refs,
            } => {
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
                let operation_id = format!(
                    "cloud-hypervisor-commit-children-{}-{}",
                    batch.owner_uid().as_str(),
                    batch.owner_revision().get(),
                );
                let mut mutations = Vec::with_capacity(batch.mutations().len());
                for mutation in batch.mutations() {
                    let canonical = batch
                        .canonical_payload(mutation.target())
                        .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                    mutations.push(StoreMutation {
                        kind: ResourceMutationKind::Create,
                        zone: batch.zone().clone(),
                        target: mutation.target().clone(),
                        expected: ExpectedRevision::CreateAbsent,
                        expected_uid: None,
                        owner: Some(batch.owner_ref().clone()),
                        canonical_resource: Some(canonical),
                        add_finalizers: Vec::new(),
                        remove_finalizers: Vec::new(),
                        wait_for_reconcile: false,
                        reconcile_deadline_ms: None,
                        configuration_generation: None,
                        assignment: None,
                    });
                }
                let stored = self
                    .assigned_mutation_api
                    .commit_assigned_child_mutations(&owner, mutations, &operation_id)
                    .await
                    .map_err(cloud_hypervisor_assigned_mutation_error)?;
                let mut committed = Vec::with_capacity(stored.len());
                for resource in stored {
                    committed.push(
                        d2b_provider_runtime_cloud_hypervisor::CommittedChild::new(
                            resource.resource_ref,
                            batch.owner_ref().clone(),
                            resource.zone,
                            resource.uid,
                            resource.revision,
                        )
                        .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?,
                    );
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
                if owner_ref.resource_type().as_str() != "Guest"
                    || current.owner_uid.is_none()
                {
                    return Err(CloudHypervisorResourceApiError::Conflict);
                }
                let owner = self
                    .get_stored(&owner_ref, "cloud-hypervisor-update-child-owner")
                    .await?;
                if current.owner_uid.as_ref() != Some(&owner.uid) {
                    return Err(CloudHypervisorResourceApiError::Conflict);
                }
                let stored = self
                    .assigned_mutation_api
                    .commit_assigned_child_mutations(
                        &owner,
                        vec![StoreMutation {
                            kind: ResourceMutationKind::UpdateSpec,
                            zone: current.zone.clone(),
                            target: update.target().clone(),
                            expected: ExpectedRevision::Exact(update.expected_revision()),
                            expected_uid: Some(update.expected_uid().clone()),
                            owner: Some(owner_ref),
                            canonical_resource: Some(payload),
                            add_finalizers: Vec::new(),
                            remove_finalizers: Vec::new(),
                            wait_for_reconcile: false,
                            reconcile_deadline_ms: None,
                            configuration_generation: None,
                            assignment: None,
                        }],
                        &operation_id,
                    )
                    .await
                    .map_err(cloud_hypervisor_assigned_mutation_error)?
                    .into_iter()
                    .next()
                    .ok_or(CloudHypervisorResourceApiError::InvalidResponse)?;
                Ok(CloudHypervisorResourceResponse::Updated(
                    d2b_provider_runtime_cloud_hypervisor::CommittedChild::new(
                        stored.resource_ref,
                        update.target().clone(),
                        stored.zone,
                        stored.uid,
                        stored.revision,
                    )
                    .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?,
                ))
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
                self.assigned_mutation_api
                    .persist_assigned_status(&current, payload, &operation_id)
                    .await
                    .map_err(cloud_hypervisor_assigned_mutation_error)?;
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
                let operation_id = format!(
                    "cloud-hypervisor-delete-child-{}-{}",
                    child.uid().as_str(),
                    child.revision().get(),
                );
                self.assigned_mutation_api
                    .commit_assigned_child_mutations(
                        &owner,
                        vec![StoreMutation {
                            kind: ResourceMutationKind::Delete,
                            zone: current.zone.clone(),
                            target: child.target().clone(),
                            expected: ExpectedRevision::Exact(child.revision()),
                            expected_uid: Some(child.uid().clone()),
                            owner: Some(guest_ref),
                            canonical_resource: None,
                            add_finalizers: Vec::new(),
                            remove_finalizers: Vec::new(),
                            wait_for_reconcile: false,
                            reconcile_deadline_ms: None,
                            configuration_generation: None,
                            assignment: None,
                        }],
                        &operation_id,
                    )
                    .await
                    .map_err(cloud_hypervisor_assigned_mutation_error)?;
                Ok(CloudHypervisorResourceResponse::LifecycleApplied)
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
                let operation_id = format!(
                    "cloud-hypervisor-clear-finalizer-{}-{}",
                    guest_uid.as_str(),
                    guest_revision.get(),
                );
                self.assigned_mutation_api
                    .persist_assigned_finalizers(
                        &current,
                        Vec::new(),
                        vec![
                            d2b_provider_runtime_cloud_hypervisor::GUEST_CONTROLLER_FINALIZER
                                .to_owned(),
                        ],
                        &operation_id,
                    )
                    .await
                    .map_err(cloud_hypervisor_assigned_mutation_error)?;
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
                let envelope = ResourceEnvelope::from_json(&current.canonical_json)
                    .map_err(|_| CloudHypervisorResourceApiError::InvalidResponse)?;
                if envelope.metadata().finalizers().iter().any(|finalizer| {
                    finalizer.as_str()
                        == d2b_provider_runtime_cloud_hypervisor::GUEST_CONTROLLER_FINALIZER
                }) {
                    return Ok(CloudHypervisorResourceResponse::LifecycleApplied);
                }
                let operation_id = format!(
                    "cloud-hypervisor-ensure-finalizer-{}-{}",
                    guest_uid.as_str(),
                    guest_revision.get(),
                );
                self.assigned_mutation_api
                    .persist_assigned_finalizers(
                        &current,
                        vec![
                            d2b_provider_runtime_cloud_hypervisor::GUEST_CONTROLLER_FINALIZER
                                .to_owned(),
                        ],
                        Vec::new(),
                        &operation_id,
                    )
                    .await
                    .map_err(cloud_hypervisor_assigned_mutation_error)?;
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
    bootstrap_provisioned_store: bool,
    store_id: String,
    store: Arc<RedbResourceStore>,
    store_metadata: StoreRuntimeMetadata,
    backend: Arc<ZoneStoreBackend>,
    api: Arc<ResourceService<ZoneStoreBackend>>,
    authorizer: Arc<NativeAuthorizer>,
    authorization_state: Arc<Mutex<Option<AuthorizationState>>>,
    policy_projection: Arc<PolicyProjection>,
    bundle_resource_types: Vec<ResourceTypeName>,
    /// The published per-zone v3 planes (F1 wiring): the manager-backed API
    /// service resolves its manager client and watch hub from here. The
    /// inner lock is the composition's published plane table.
    v3_planes: Mutex<
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
    /// The manager-backed Resource API service for this Zone's converted
    /// types: built once, after the Zone's v3 plane has been published.
    v3_api:
        Mutex<Option<Arc<ResourceService<d2b_resource_api::manager_backend::ManagerBackend>>>>,
    manager_authorizer: Arc<NativeAuthorizer>,
    policy_subject_fingerprints:
        Mutex<BTreeMap<(ResourceRef, ResourceRef), PolicySubjectFingerprint>>,
    bus: Option<Arc<ZoneBus>>,
    registrar: Arc<Mutex<Option<ZoneRegistrar>>>,
    ingress: Mutex<Option<BusIngress>>,
    service_task: Mutex<Option<tokio::task::JoinHandle<Result<(), SessionServerError>>>>,
    process_status_client:
        Arc<Mutex<Option<Arc<ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>>>>>,
    core_controller_subject: Mutex<Option<AuthenticatedSubjectContext>>,
    system_core_rebind_pending: AtomicBool,
    u6_runner_tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    u6_runner_lock: Arc<tokio::sync::Mutex<()>>,
    u6_state: Mutex<Option<Arc<crate::ServerState>>>,
    u6_required: AtomicBool,
    credential_sessions: CredentialSessionRegistry,
    core: Mutex<CoreProcess>,
    readiness: ZoneRuntimeReadiness,
    policy_installed: bool,
    controller_endpoint_registered: bool,
    watch_admitted: bool,
    assignments: AssignmentRegistry,
    authority_index: Arc<tokio::sync::Mutex<HostGlobalAuthorityIndex>>,
    authority_persistence: Arc<RedbAuthorityPersistence>,
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
    shared_provider_effects: Arc<dyn SharedProviderEffectExecutor>,
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
            .field("store_id", &"<opaque>")
            .field("current_revision", &self.store_metadata.current_revision)
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

impl ZoneResourceRuntime {
    /// Install the daemon-owned typed effect executor used by U8 Provider
    /// runners. The binding is replaced only during trusted composition.
    pub(crate) fn set_shared_provider_effects(
        &mut self,
        effects: Arc<dyn SharedProviderEffectExecutor>,
    ) {
        self.shared_provider_effects = effects;
    }

    /// Open one Zone from a broker-owned descriptor.
    pub async fn open(zone: ZoneId, opened: OpenedZoneStore) -> Result<Self, ResourceRuntimeError> {
        Self::open_internal(
            zone,
            opened,
            None,
            Arc::new(BrokerEvidenceIndex::default()),
            None,
            false,
            None,
            None,
        )
        .await
    }

    /// Open one Zone with the production-owned durable audit sink.
    pub async fn open_with_audit(
        zone: ZoneId,
        opened: OpenedZoneStore,
        audit_sink: Arc<AuditSink>,
    ) -> Result<Self, ResourceRuntimeError> {
        Self::open_internal(
            zone,
            opened,
            Some(audit_sink),
            Arc::new(BrokerEvidenceIndex::default()),
            None,
            false,
            None,
            None,
        )
        .await
    }

    /// Open one Zone with durable audit and broker reconciliation evidence.
    pub async fn open_with_audit_and_evidence(
        zone: ZoneId,
        opened: OpenedZoneStore,
        audit_sink: Arc<AuditSink>,
        broker_evidence: Arc<BrokerEvidenceIndex>,
    ) -> Result<Self, ResourceRuntimeError> {
        Self::open_internal(
            zone,
            opened,
            Some(audit_sink),
            broker_evidence,
            None,
            false,
            None,
            None,
        )
        .await
    }

    /// Open one Zone with explicit audit, broker-evidence, and telemetry
    /// ownership.
    pub async fn open_with_audit_and_evidence_and_telemetry(
        zone: ZoneId,
        opened: OpenedZoneStore,
        audit_sink: Arc<AuditSink>,
        broker_evidence: Arc<BrokerEvidenceIndex>,
        telemetry_path: impl Into<std::path::PathBuf>,
    ) -> Result<Self, ResourceRuntimeError> {
        Self::open_internal(
            zone,
            opened,
            Some(audit_sink),
            broker_evidence,
            Some(telemetry_path.into()),
            false,
            None,
            None,
        )
        .await
    }

    /// Open a production Zone with a bundle-bound immutable identity.
    pub(crate) async fn open_production_with_identity(
        zone: ZoneId,
        opened: OpenedZoneStore,
        audit_sink: Arc<AuditSink>,
        broker_evidence: Arc<BrokerEvidenceIndex>,
        telemetry_path: impl Into<std::path::PathBuf>,
        desired_bundle: ResourceBundle,
        authority_identity: ZoneAuthorityIdentity,
    ) -> Result<Self, ResourceRuntimeError> {
        Self::open_internal(
            zone,
            opened,
            Some(audit_sink),
            broker_evidence,
            Some(telemetry_path.into()),
            true,
            Some(desired_bundle),
            Some(authority_identity),
        )
        .await
    }

    async fn open_internal(
        zone: ZoneId,
        opened: OpenedZoneStore,
        audit_sink: Option<Arc<AuditSink>>,
        broker_evidence: Arc<BrokerEvidenceIndex>,
        telemetry_path: Option<std::path::PathBuf>,
        bootstrap_provisioned_store: bool,
        desired_bundle: Option<ResourceBundle>,
        authority_identity: Option<ZoneAuthorityIdentity>,
    ) -> Result<Self, ResourceRuntimeError> {
        #[cfg(test)]
        let audit_sink = audit_sink.or_else(|| {
            let base = std::env::var_os("TEST_TMPDIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::var_os("CARGO_MANIFEST_DIR")
                        .map(std::path::PathBuf::from)
                        .or_else(|| std::env::current_dir().ok())
                        .expect("resolve resource runtime scratch root")
                        .join("target")
                        .join("tmp")
                });
            let path = base.join(format!(
                "d2bd-resource-audit-{}-{}-{}",
                zone.as_str(),
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or_default()
            ));
            AuditSink::open(path).ok().map(Arc::new)
        });
        #[cfg(not(test))]
        let audit_sink = audit_sink;
        let external_inventory = opened.external_inventory.clone().unwrap_or_else(|| {
            Arc::new(TrustedExternalNicInventory::default())
                as Arc<dyn ExternalNicRecoveryInventory>
        });
        Self::open_with_external_inventory_and_audit(
            zone,
            opened,
            external_inventory,
            audit_sink,
            broker_evidence,
            telemetry_path,
            bootstrap_provisioned_store,
            desired_bundle,
            authority_identity,
        )
        .await
    }

    /// Open one Zone with the host/bundle-owned physical-NIC inventory port.
    pub async fn open_with_external_inventory(
        zone: ZoneId,
        opened: OpenedZoneStore,
        external_inventory: Arc<dyn ExternalNicRecoveryInventory>,
    ) -> Result<Self, ResourceRuntimeError> {
        Self::open_with_external_inventory_and_audit(
            zone,
            opened,
            external_inventory,
            None,
            Arc::new(BrokerEvidenceIndex::default()),
            None,
            false,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn open_with_external_inventory_and_audit(
        zone: ZoneId,
        opened: OpenedZoneStore,
        external_inventory: Arc<dyn ExternalNicRecoveryInventory>,
        audit_sink: Option<Arc<AuditSink>>,
        broker_evidence: Arc<BrokerEvidenceIndex>,
        telemetry_path: Option<std::path::PathBuf>,
        bootstrap_provisioned_store: bool,
        desired_bundle: Option<ResourceBundle>,
        authority_identity: Option<ZoneAuthorityIdentity>,
    ) -> Result<Self, ResourceRuntimeError> {
        let expected_store_id = format!("zone-store-{}", zone.as_str());
        if opened.response.zone_store_id.as_str() != expected_store_id {
            return Err(ResourceRuntimeError::BrokerResponseMismatch);
        }
        if opened.response.fd_index != 0 {
            return Err(ResourceRuntimeError::BrokerFdCountMismatch);
        }
        if !matches!(
            opened.response.disposition,
            ZoneStoreDisposition::Provisioned | ZoneStoreDisposition::Opened
        ) {
            return Err(ResourceRuntimeError::BrokerDispositionInvalid);
        }

        let disposition = opened.response.disposition;
        let store_identity = if let Some(authority) = authority_identity.as_ref() {
            let bundle = desired_bundle
                .as_ref()
                .ok_or(ResourceRuntimeError::IdentityUnbound)?;
            if bundle.zone_uid() != Some(authority.zone_uid())
                || bundle.integrity().content_hash != authority.bundle_generation().as_str()
            {
                return Err(ResourceRuntimeError::HandlerNotReady);
            }
            if !d2b_contracts_resource::v3::is_canonical_digest(&opened.response.store_identity) {
                return Err(ResourceRuntimeError::BrokerResponseMismatch);
            }
            store_identity_for_authority(&zone, authority)?
        } else {
            store_identity(&zone, &opened.response.store_identity)?
        };
        let store_identity =
            if bootstrap_provisioned_store && disposition == ZoneStoreDisposition::Provisioned {
                store_identity.with_revisions(initial_policy_snapshot()?)
            } else {
                store_identity
            };
        let bundle_resource_types = desired_bundle
            .as_ref()
            .map(|bundle| {
                bundle
                    .resources
                    .iter()
                    .map(|resource| resource.resource_type().clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let bundle_resource_types = trusted_catalog_resource_types(bundle_resource_types)?;
        let authorizer = Arc::new(runtime_authorizer(&bundle_resource_types)?);
        // The manager plane's own authorizer: same catalog, its own mutation
        // seal (an authorizer hands out exactly one), so the manager-backed
        // API service can serve the Zone's converted types alongside the
        // redb service (U8/U9 F1 wiring).
        let manager_authorizer = Arc::new(runtime_authorizer(&bundle_resource_types)?);
        let assignments = new_assignment_registry();
        let acceptor = authorizer
            .take_store_seal(store_identity.seal_identity())
            .map_err(|_| ResourceRuntimeError::StoreSealUnavailable)?;
        let file = File::from(opened.database_fd);
        let store = match disposition {
            ZoneStoreDisposition::Provisioned => {
                let mut marker =
                    tempfile::tempfile().map_err(|_| ResourceRuntimeError::StoreOpenFailed)?;
                write_provisioning_marker(&mut marker, &store_identity)
                    .map_err(|_| ResourceRuntimeError::StoreOpenFailed)?;
                match audit_sink {
                    Some(sink) => {
                        match telemetry_path.as_ref() {
                            Some(path) => {
                                RedbResourceStore::provision_owned_with_audit_and_evidence_and_telemetry(
                                    file,
                                    marker,
                                    store_identity,
                                    acceptor,
                                    sink,
                                    broker_evidence,
                                    path,
                                )
                                .await
                            }
                            None => {
                                RedbResourceStore::provision_owned_with_audit_and_evidence(
                                    file,
                                    marker,
                                    store_identity,
                                    acceptor,
                                    sink,
                                    broker_evidence,
                                )
                                .await
                            }
                        }
                    }
                    None => {
                        RedbResourceStore::provision_owned(file, marker, store_identity, acceptor)
                            .await
                    }
                }
            }
            ZoneStoreDisposition::Opened => match audit_sink {
                Some(sink) => {
                    match telemetry_path.as_ref() {
                        Some(path) => {
                            RedbResourceStore::open_owned_with_audit_and_evidence_and_telemetry(
                                file,
                                store_identity,
                                acceptor,
                                sink,
                                broker_evidence,
                                path,
                            )
                            .await
                        }
                        None => {
                            RedbResourceStore::open_owned_with_audit_and_evidence(
                                file,
                                store_identity,
                                acceptor,
                                sink,
                                broker_evidence,
                            )
                            .await
                        }
                    }
                }
                None => RedbResourceStore::open_owned(file, store_identity, acceptor).await,
            },
        }
        .map_err(|_| ResourceRuntimeError::StoreOpenFailed)?;
        let store = Arc::new(store);
        let authority_persistence = Arc::new(
            RedbAuthorityPersistence::new(Arc::clone(&store))
                .with_external_inventory(external_inventory),
        );
        let authority_recovery = Arc::new(
            AuthorityRecoveryCoordinator::recover_with_provenance(
                authority_persistence.clone(),
                authority_persistence.as_ref(),
            )
            .await
            .map_err(|_| ResourceRuntimeError::AuthorityUnavailable)?,
        );
        let authority_index = authority_recovery.index();
        let store_metadata = retry_transient_store_read(
            &zone,
            "runtime-open-metadata",
            || store.runtime_metadata(),
        )
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        if let Some(authority) = authority_identity.as_ref() {
            if store_metadata.zone_uid != *authority.zone_uid()
                || store_metadata.store_uid != *authority.store_uid()
                || store_metadata.store_epoch != authority.store_epoch()
            {
                return Err(ResourceRuntimeError::HandlerNotReady);
            }
            if disposition == ZoneStoreDisposition::Opened
                && store_metadata.policy_snapshot.policy_revision != 0
            {
                validate_zone_self_resource(
                    &store,
                    &zone,
                    authority.zone_uid(),
                    authority.store_uid(),
                )
                .await?;
            }
        }
        tracing::error!(
            zone = %zone.as_str(),
            disposition = ?disposition,
            policy_revision = store_metadata.policy_snapshot.policy_revision,
            api_catalog_revision = store_metadata.policy_snapshot.api_catalog_revision,
            active_configuration_revision = %store_metadata
                .policy_snapshot
                .active_configuration_revision
                .get(),
            desired_resource_count = desired_bundle
                .as_ref()
                .map(|bundle| bundle.resources.len())
                .unwrap_or_default(),
            "resource runtime opened Zone store"
        );
        if desired_bundle
            .as_ref()
            .is_some_and(|bundle| bundle.zone != zone)
        {
            return Err(ResourceRuntimeError::HandlerNotReady);
        }
        // The per-type partition (KTD4) is enforced by routing, not refusal:
        // the public surface dispatches converted types to the manager-backed
        // service (see `dispatch_public_cli_request`), so the redb service
        // only ever sees unconverted types from outside. Legacy in-daemon
        // writers (the framework runners and controller sessions that Phase B
        // deletes) keep their redb path until their providers convert.
        let backend = Arc::new(RedbBackend::from_arc(Arc::clone(&store)));
        let api = Arc::new(
            ResourceService::new_with_zone_uid(
                Arc::clone(&backend),
                Arc::clone(&authorizer),
                authority_identity
                    .as_ref()
                    .map(|authority| authority.zone_uid().clone()),
            )
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?,
        );
        let mut interaction_provider_configuration = None;
        let mut interaction_provider_configuration_refused;
        let mut interaction_identity = None;

        let mut core = CoreProcess::new();
        let mut bus = None;
        let mut registrar = None;
        let mut ingress = None;
        let mut service_task = None;
        let mut process_status_client = None;
        let mut core_controller_subject = None;
        let defer_activation = authority_identity.is_some();
        let mut final_store_metadata = store_metadata.clone();
        let mut interaction_state = InteractionState::Absent;
        let (
            resource_api_ready,
            local_session_ready,
            policy_installed,
            controller_endpoint_registered,
            watch_admitted,
            stage,
            zone_status,
            authorization_state,
        ) = if store_metadata.policy_snapshot.policy_revision == 0 {
            if let Err(error) = core.connect_runtime(CoreRuntimeReadiness {
                store_ready: true,
                resource_api_ready: false,
                local_bus_ready: false,
                controller_endpoint_registered: false,
                authenticated_system_core_session: false,
            }) {
                tracing::warn!(
                    error = ?error,
                    "core runtime readiness connect failed during zone bootstrap",
                );
            }
            (
                false,
                false,
                false,
                false,
                false,
                core.stage(),
                SystemCoreStatusEmitter::new()
                    .emit(
                        ZoneStatusInput::new(ResourcePhase::Pending, Vec::new())
                            .with_runtime_metadata(zone_runtime_metadata(
                                &store_metadata,
                                0,
                                false,
                                0,
                                Some(current_status_timestamp()),
                            )),
                    )
                    .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
                None,
            )
        } else {
            let (policy, state) = runtime_policy(
                &zone,
                &store_metadata.policy_snapshot,
                store_metadata.current_revision,
                &bundle_resource_types,
            )
            .inspect_err(|error| {
                tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime policy setup failed");
            })?;
            authorizer
                .replace_policy(policy.clone(), &state)
                .map_err(|error| {
                    tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime policy installation failed");
                    ResourceRuntimeError::AuthorizationUnavailable
                })?;
            let bus_authorizer = BusAuthorizer::from_shared(Arc::clone(&authorizer), state.clone())
                .map(|authorizer| authorizer.with_assignment_registry(Arc::clone(&assignments)))
                .map_err(|error| {
                    tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime bus authorizer setup failed");
                    ResourceRuntimeError::AuthorizationUnavailable
                })?;
            let (zone_bus, mut zone_registrar) =
                ZoneBus::new(zone.clone(), bus_authorizer, BusConfig::default())
                    .map_err(|error| {
                        tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime Zone bus setup failed");
                        ResourceRuntimeError::AuthenticationUnavailable
                    })?;
            let (zone_ingress, zone_service_task, status_client, subject_context) =
                register_system_core_session(
                    &mut zone_registrar,
                    Arc::clone(&api),
                    Arc::clone(&authorizer),
                    state.clone(),
                )
                .await
                .inspect_err(|error| {
                    tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime system-core session registration failed");
                })?;
            process_status_client = Some(Arc::clone(&status_client));
            core_controller_subject = Some(subject_context);
            if defer_activation {
                bus = Some(zone_bus);
                registrar = Some(zone_registrar);
                ingress = Some(zone_ingress);
                service_task = Some(zone_service_task);
                (
                    true,
                    true,
                    true,
                    true,
                    true,
                    core.stage(),
                    SystemCoreStatusEmitter::new()
                        .emit(
                            ZoneStatusInput::new(ResourcePhase::Pending, Vec::new())
                                .with_runtime_metadata(zone_runtime_metadata(
                                    &store_metadata,
                                    0,
                                    false,
                                    0,
                                    Some(current_status_timestamp()),
                                )),
                        )
                        .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
                    Some(state),
                )
            } else {
                if bootstrap_provisioned_store
                    && disposition == ZoneStoreDisposition::Provisioned
                    && desired_bundle.is_none()
                {
                    ensure_bootstrap_host_resource(&zone, &store, &status_client).await?;
                }
                if let Some(bundle) = desired_bundle.as_ref() {
                    // Phase A partition (U10): converted types (Process,
                    // Volume, VolumeBinding, Endpoint) are served only by the
                    // v3 resource plane and never materialize into redb; the
                    // old path sees only unconverted rows.
                    let old_bundle = crate::resource_plane_v3::old_plane_bundle(bundle)
                        .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
                    materialize_zone_resource_bundle(&zone, &old_bundle, &store, &status_client)
                        .await
                        .inspect_err(|error| {
                            tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime Zone bundle materialization failed");
                        })?;
                }
                let current_store_metadata = retry_transient_store_read(
                    &zone,
                    "runtime-open-metadata-after-materialization",
                    || store.runtime_metadata(),
                )
                .await
                    .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
                final_store_metadata = current_store_metadata.clone();
                (
                    interaction_provider_configuration,
                    interaction_provider_configuration_refused,
                ) = match load_interaction_provider_configuration(
                    &zone,
                    &store,
                    current_store_metadata.current_revision,
                )
                .await
                {
                    Ok(None) => (None, false),
                    Ok(Some(configuration)) if configuration.is_complete() => {
                        (Some(configuration), false)
                    }
                    Ok(Some(_)) => {
                        tracing::error!(
                            zone = %zone.as_str(),
                            "resource runtime committed interaction Provider configuration is incomplete",
                        );
                        (None, true)
                    }
                    Err(error) => {
                        tracing::error!(
                            zone = %zone.as_str(),
                            error = %error,
                            "resource runtime committed interaction Provider configuration load failed",
                        );
                        (None, true)
                    }
                };
                interaction_identity = match load_committed_interaction_identity(
                    &zone,
                    &store,
                    current_store_metadata.current_revision,
                    interaction_provider_configuration.as_ref(),
                )
                .await
                {
                    Ok(identity) => identity,
                    Err(error) => {
                        tracing::error!(
                            zone = %zone.as_str(),
                            error = %error,
                            "resource runtime committed interaction identity load failed",
                        );
                        interaction_provider_configuration_refused = true;
                        None
                    }
                };
                let interaction_present =
                    interaction_resources_present(&zone, &store).await?;
                interaction_state = derive_interaction_state(
                    interaction_present,
                    interaction_provider_configuration.as_ref(),
                    interaction_identity.as_ref(),
                    interaction_provider_configuration_refused,
                );
                let system_core = system_core_startup_result(&zone, &store)
                    .await
                    .inspect_err(|error| {
                        tracing::error!(
                            zone = %zone.as_str(),
                            error = ?error,
                            "resource runtime system-core startup summary failed"
                        );
                    })?;
                tracing::warn!(
                    zone = %zone.as_str(),
                    host_phase = ?system_core.host_phase,
                    user_phase = ?system_core.user_phase,
                    total_resources = system_core.total_resource_count,
                    "system-core shared runner startup summary completed",
                );
                let aggregate_handler_phase = if system_core.host_phase == HandlerPhase::Ready
                    && system_core.user_phase == HandlerPhase::Ready
                {
                    HandlerPhase::Ready
                } else {
                    HandlerPhase::Degraded
                };
                tracing::error!(
                    zone = %zone.as_str(),
                    host_phase = ?system_core.host_phase,
                    user_phase = ?system_core.user_phase,
                    core_phase = ?system_core.core_phase,
                    total_resource_count = system_core.total_resource_count,
                    "resource runtime system-core reconciliation result"
                );
                let stage = {
                    let recovered_authority = authority_index.lock().await;
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
                        checkpoint_revision: current_store_metadata.current_revision.get(),
                        active_configuration_revision: current_store_metadata
                            .policy_snapshot
                            .active_configuration_revision
                            .get(),
                        provider_lease_count: 0,
                        controller_lease_count: 0,
                        ambiguous_operation_count: 0,
                        watch_admitted: true,
                    },
                    &recovered_authority,
                )
                .map_err(|error| {
                    let error = map_startup_error(error);
                    tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime core startup failed");
                    error
                })?;
                    d2bd_runtime::resource_runtime_support::mark_core_handlers(
                    &mut core,
                    aggregate_handler_phase,
                    current_store_metadata.current_revision.get(),
                )
                .inspect_err(|error| {
                    tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime handler marking failed");
                })?;
                    core.publish_readiness().map_err(|error| {
                    let error = map_startup_error(error);
                    tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime readiness publication failed");
                    error
                })?
                };
                bus = Some(zone_bus);
                registrar = Some(zone_registrar);
                ingress = Some(zone_ingress);
                service_task = Some(zone_service_task);
                (
                    true,
                    true,
                    true,
                    true,
                    true,
                    stage,
                    SystemCoreStatusEmitter::new()
                        .emit(
                            ZoneStatusInput::new(system_core.core_phase, Vec::new())
                                .with_system_core_phases(
                                    handler_phase_to_zone_phase(system_core.host_phase),
                                    handler_phase_to_zone_phase(system_core.user_phase),
                                )
                                .with_runtime_metadata(zone_runtime_metadata(
                                    &current_store_metadata,
                                    system_core.total_resource_count,
                                    system_core.generation_cleanup_pending,
                                    system_core.cleanup_pending_count,
                                    Some(current_status_timestamp()),
                                )),
                        )
                        .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
                    Some(state),
                )
            }
        };
        let store_metadata = final_store_metadata;
        let defer_core_start = authority_identity.is_some();
        let bus = bus.map(Arc::new);
        let authorization_state = Arc::new(Mutex::new(authorization_state));
        let policy_loaded = authorization_state
            .lock()
            .ok()
            .is_some_and(|state| state.is_some());
        let policy_projection = Arc::new(PolicyProjection {
            authorizer: Arc::clone(&authorizer),
            manager_authorizer: Some(Arc::clone(&manager_authorizer)),
            bus: bus.clone(),
            authorization_state: Arc::clone(&authorization_state),
            policy_refresh: Arc::new(Mutex::new(())),
            policy_loaded: Arc::new(Mutex::new(policy_loaded)),
            installed_controller_subjects: Arc::new(Mutex::new(BTreeSet::new())),
        });
        let runtime = Self {
            zone,
            authority_identity,
            bootstrap_provisioned_store: bootstrap_provisioned_store
                && disposition == ZoneStoreDisposition::Provisioned,
            store_id: expected_store_id,
            store,
            store_metadata,
            backend,
            api,
            authorizer,
            authorization_state,
            policy_projection,
            bundle_resource_types,
            v3_planes: Mutex::new(None),
            v3_api: Mutex::new(None),
            manager_authorizer,
            policy_subject_fingerprints: Mutex::new(BTreeMap::new()),
            bus,
            registrar: Arc::new(Mutex::new(registrar)),
            ingress: Mutex::new(ingress),
            service_task: Mutex::new(service_task),
            process_status_client: Arc::new(Mutex::new(process_status_client)),
            core_controller_subject: Mutex::new(core_controller_subject),
            system_core_rebind_pending: AtomicBool::new(false),
            u6_runner_tasks: Mutex::new(Vec::new()),
            u6_runner_lock: Arc::new(tokio::sync::Mutex::new(())),
            u6_state: Mutex::new(None),
            u6_required: AtomicBool::new(false),
            credential_sessions: CredentialSessionRegistry::default(),
            core: Mutex::new(core),
            readiness: ZoneRuntimeReadiness {
                store_ready: true,
                resource_api_ready,
                local_session_ready,
                provider_path_ready: false,
                authority_ready: true,
                core_stage: stage,
            },
            policy_installed,
            controller_endpoint_registered,
            watch_admitted,
            assignments,
            authority_index,
            authority_persistence,
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
            shared_provider_effects: Arc::new(UnavailableSharedProviderEffects),
            interaction_provider_configuration,
            interaction_identity,
            interaction_state,
        };
        let coordinator = Arc::new(runtime.build_controller_session_coordinator()?);
        *runtime
            .controller_session_coordinator
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? = Some(coordinator);
        // U12: the Core runner family is deleted. The nine Core-family
        // ResourceTypes are driven by the v3 plane's Core resource driver,
        // and the system-core session registered above is the surviving
        // Core-owned startup surface. `defer_core_start` (authority-identity
        // zones defer to activate_published_bundle) still gates it.
        let _ = defer_core_start;
        Ok(runtime)
    }

    /// Materialize the verified bundle after the complete local generation
    /// has passed its read-only validation barrier.
    pub(crate) async fn materialize_desired_bundle(
        &self,
        bundle: &ResourceBundle,
    ) -> Result<(), ResourceRuntimeError> {
        let client = self.status_client()?;
        materialize_zone_resource_bundle(&self.zone, bundle, &self.store, &client).await
    }

    /// Validate the desired bundle without advancing this Zone store.
    pub(crate) async fn validate_desired_bundle(
        &self,
        bundle: &ResourceBundle,
    ) -> Result<(), ResourceRuntimeError> {
        validate_zone_resource_bundle(&self.zone, bundle, &self.store).await
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
        let binding_digest = self.store.authority_binding_digest(set_generation.as_str());
        let payload = generation_publication_payload(set_generation, &binding_digest, generations)?;
        let operations = self
            .store
            .authority_operations()
            .await
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
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
        self.store
            .prepare_authority_operation(operation_id, payload, set_generation.as_str())
            .await
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
        let binding_digest = self.store.authority_binding_digest(set_generation.as_str());
        let operation = self
            .store
            .authority_operations()
            .await
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?
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
            .store
            .resume_authority_operation(operation_id, &binding_digest)
            .await
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

    /// Materialize a validated bundle and provision bootstrap rows only after
    /// the complete local generation set has been durably prepared.
    pub(crate) async fn prepare_published_bundle(
        &self,
        bundle: &ResourceBundle,
    ) -> Result<(), ResourceRuntimeError> {
        self.materialize_desired_bundle(bundle)
            .await
            .inspect_err(|error| {
                tracing::error!(
                    zone = %self.zone.as_str(),
                    error = ?error,
                    "resource runtime desired bundle materialization failed"
                );
            })?;
        if self.bootstrap_provisioned_store {
            let client = self.status_client()?;
            if let Some(authority) = self.authority_identity.as_ref() {
                ensure_bootstrap_zone_resource(
                    &self.zone,
                    authority.zone_uid(),
                    &self.store,
                    &client,
                )
                .await
                .inspect_err(|error| {
                    tracing::error!(
                        zone = %self.zone.as_str(),
                        error = ?error,
                        "resource runtime Zone self bootstrap failed"
                    );
                })?;
            }
            if !bundle
                .resources
                .iter()
                .any(|resource| resource.resource_type().as_str() == "Host")
            {
                ensure_bootstrap_host_resource(&self.zone, &self.store, &client)
                    .await
                    .inspect_err(|error| {
                        tracing::error!(
                            zone = %self.zone.as_str(),
                            error = ?error,
                            "resource runtime Host bootstrap failed"
                        );
                    })?;
            }
        }
        Ok(())
    }

    /// Finish deferred startup after the complete bundle is visible in the
    /// store. The shared Core runners own all Host/User observation and status
    /// writes after this point.
    pub(crate) async fn activate_published_bundle(&mut self) -> Result<(), ResourceRuntimeError> {
        self.refresh_authorization_policy().await?;
        let store_metadata = retry_transient_store_read(
            &self.zone,
            "runtime-activate-metadata",
            || self.store.runtime_metadata(),
        )
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;

        let (interaction_provider_configuration, mut interaction_provider_configuration_refused) =
            match load_interaction_provider_configuration(
                &self.zone,
                &self.store,
                store_metadata.current_revision,
            )
            .await
            {
            Ok(None) => (None, false),
            Ok(Some(configuration)) if configuration.is_complete() => (Some(configuration), false),
            Ok(Some(_)) => {
                tracing::error!(
                    zone = %self.zone.as_str(),
                    "resource runtime committed interaction Provider configuration is incomplete",
                );
                (None, true)
            }
            Err(error) => {
                tracing::error!(
                    zone = %self.zone.as_str(),
                    error = %error,
                    "resource runtime committed interaction Provider configuration load failed",
                );
                (None, true)
            }
        };
        self.interaction_provider_configuration = interaction_provider_configuration;
        self.interaction_identity = match load_committed_interaction_identity(
            &self.zone,
            &self.store,
            store_metadata.current_revision,
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
        let interaction_present =
            interaction_resources_present(&self.zone, &self.store).await?;
        self.interaction_state = derive_interaction_state(
            interaction_present,
            self.interaction_provider_configuration.as_ref(),
            self.interaction_identity.as_ref(),
            interaction_provider_configuration_refused,
        );
        let system_core =
            system_core_startup_result(&self.zone, &self.store)
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
        let store_metadata = retry_transient_store_read(
            &self.zone,
            "runtime-activate-metadata-after-core-runners",
            || self.store.runtime_metadata(),
        )
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
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
                    checkpoint_revision: store_metadata.current_revision.get(),
                    active_configuration_revision: store_metadata
                        .policy_snapshot
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
                store_metadata.current_revision.get(),
            )?;
        };
        self.store_metadata = store_metadata;
        self.zone_status = Mutex::new(
            SystemCoreStatusEmitter::new()
                .emit(
                    ZoneStatusInput::new(system_core.core_phase, Vec::new())
                        .with_system_core_phases(
                            handler_phase_to_zone_phase(system_core.host_phase),
                            handler_phase_to_zone_phase(system_core.user_phase),
                        )
                        .with_runtime_metadata(zone_runtime_metadata(
                            &self.store_metadata,
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
            store_ready: true,
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

    /// Borrow the content-addressed bundle generation bound at production
    /// startup.
    pub(crate) fn authority_bundle_generation(&self) -> Option<&ResourceBundleGenerationId> {
        self.authority_identity
            .as_ref()
            .map(ZoneAuthorityIdentity::bundle_generation)
    }

    /// Borrow the opaque store id used for the broker request.
    pub fn store_id(&self) -> &str {
        &self.store_id
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

    /// Return the policy revision committed in the opened resource store.
    ///
    /// Interaction Providers bind this snapshot instead of carrying a
    /// route-derived policy placeholder.
    pub const fn committed_policy_snapshot(&self) -> PolicySnapshot {
        self.store_metadata.policy_snapshot
    }

    /// Return the durable resource revision used to fence interaction
    /// evidence against a later store commit.
    pub fn current_revision(&self) -> ZoneRevision {
        self.authorization_state
            .lock()
            .ok()
            .and_then(|state| state.as_ref().map(|state| state.zone_policy_revision))
            .unwrap_or(self.store_metadata.current_revision)
    }

    /// Verify the committed policy inputs the refresh compiled against.
    ///
    /// A mutation is admitted against the policy snapshot (the store enforces
    /// the same fence at commit, `transaction.rs`); the Zone's resource
    /// revision advancing under the load - derived children, process churn -
    /// never changes the authorization inputs and must not gate the mutation.
    /// Only a policy-input change supersedes the loaded rows.
    fn verify_policy_snapshot(
        zone: &ZoneId,
        loaded: &StoreRuntimeMetadata,
        verify: &StoreRuntimeMetadata,
    ) -> Result<(), ResourceRuntimeError> {
        if verify.policy_snapshot == loaded.policy_snapshot {
            return Ok(());
        }
        tracing::warn!(
            zone = zone.as_str(),
            before_policy_revision = loaded.policy_snapshot.policy_revision,
            after_policy_revision = verify.policy_snapshot.policy_revision,
            "authorization policy refresh: policy snapshot changed under the refresh",
        );
        Err(ResourceRuntimeError::PolicyUnavailable)
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
        let metadata = retry_transient_store_read(
            &self.zone,
            "authorization-policy-refresh-metadata",
            || self.store.runtime_metadata(),
        )
        .await
            .map_err(|error| {
                tracing::warn!(
                    zone = self.zone.as_str(),
                    error = ?error,
                    "authorization policy refresh: Store metadata read failed",
                );
                ResourceRuntimeError::PolicyUnavailable
            })?;
        let current = self
            .policy_projection
            .installed_state()
            .ok();
        let policy_loaded = current.is_some();
        if metadata.policy_snapshot.policy_revision == 0 {
            tracing::warn!(
                zone = self.zone.as_str(),
                "authorization policy refresh: committed policy revision is unset",
            );
            return Err(ResourceRuntimeError::PolicyUnavailable);
        }
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
            && !self.system_core_rebind_pending.load(Ordering::Acquire)
            && current.as_ref().is_some_and(|state| {
                state.snapshot == metadata.policy_snapshot
                    && state.zone_policy_revision == metadata.current_revision
            })
            && installed_controller_subjects == controller_subjects
        {
            return Ok(());
        }
        let resources = self
            .committed_policy_resources(&format!(
                "authorization-policy-refresh:{}",
                metadata.current_revision.get()
            ))
            .await?;
        let current_metadata = retry_transient_store_read(
            &self.zone,
            "authorization-policy-refresh-verify",
            || self.store.runtime_metadata(),
        )
        .await
            .map_err(|error| {
                tracing::warn!(
                    zone = self.zone.as_str(),
                    error = ?error,
                    "authorization policy refresh: Store metadata verify read failed",
                );
                ResourceRuntimeError::PolicyUnavailable
            })?;
        // A mutation is admitted against the policy snapshot (the store
        // enforces the same fence at commit, `transaction.rs`); the Zone's
        // resource revision advancing under the load - derived children,
        // process churn - does not change the authorization inputs and must
        // never gate the mutation. Only a policy-input change supersedes the
        // loaded rows.
        Self::verify_policy_snapshot(&self.zone, &metadata, &current_metadata)?;
        let previous = self
            .policy_subject_fingerprints
            .lock()
            .map_err(|_| ResourceRuntimeError::IdentityUnbound)?
            .clone();
        let fingerprints = refreshed_policy_subject_fingerprints(&resources, &previous)?;
        let (policy, state) =
            d2bd_runtime::resource_runtime_support::compile_committed_policy_with_subjects(
                &self.zone,
                metadata.policy_snapshot,
                metadata.current_revision,
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
        let _u6_runner_guard = if rebind_core {
            Some(self.u6_runner_lock.lock().await)
        } else {
            None
        };

        if rebind_core {
            self.stop_u6_controller_runners_locked().await?;
        }
        self.install_policy_projection(policy, state.clone(), controller_subjects)?;
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
            if let Some(providers) = self
                .controller_session_providers
                .lock()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
                .clone()
            {
                self.rebuild_assigned_process_api_locked(&providers).await?;
            }
            let state = self
                .u6_state
                .lock()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
                .clone();
            if let Some(state) = state {
                self.start_u6_controller_runners_locked(state).await?;
            }
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
        load_controller_policy_subjects(
            &self.zone,
            &self.store,
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
    ) -> Result<(), ResourceRuntimeError> {
        self.policy_projection
            .install(policy, state, controller_subjects)
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
        if let Some(coordinator) = self
            .controller_session_coordinator
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
        {
            coordinator.clear_assigned_process_api()?;
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
                Arc::clone(&self.api),
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
    /// `User` is a converted type: the manager is its status authority and a
    /// converted row's status is in-memory only (R11), so a durable-store read
    /// can never observe it `Ready`. Read the manager-rendered rows when this
    /// Zone's plane serves them and fall back to the store otherwise (an
    /// unconverted or legacy Zone).
    async fn resolve_public_user(
        &self,
        peer_uid: u32,
        operation_id: &str,
    ) -> Result<d2bd_runtime::resource_runtime_support::ResolvedZoneUser, ResourceRuntimeError>
    {
        let mut rows = d2bd_runtime::resource_runtime_support::load_zone_user_rows(
            &self.store,
            &self.zone,
            &format!("{operation_id}:user"),
        )
        .await?;
        if let Ok(plane) = self.v3_plane() {
            let client = plane.client().clone();
            for row in rows.iter_mut() {
                let key = d2b_resource_runtime::identity::ResourceKey::new(
                    self.zone.as_str(),
                    "User",
                    row.resource_ref.name().as_str(),
                );
                let Ok(Some(view)) = client.get(key).await else {
                    continue;
                };
                overlay_manager_row_status(row, &view);
            }
        }
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
        self.api
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
        let client = self
            .process_resource_client()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        client
            .admit_guest_lifecycle(target, operation_id.to_owned())
            .await
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)
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
        let guest = self
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "guest-lifecycle-identity".to_owned(),
                    idempotency_key: None,
                    correlation_id: "guest-lifecycle-identity".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target: target.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
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
        let provider = self
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "guest-lifecycle-provider-identity".to_owned(),
                    idempotency_key: None,
                    correlation_id: "guest-lifecycle-provider-identity".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target: provider_ref,
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
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
            self.store_metadata.zone_uid.clone(),
            guest.uid,
            guest.generation,
            provider.generation,
        ))
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
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "guest-provider-route".to_owned(),
                    idempotency_key: None,
                    correlation_id: "guest-provider-route".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target: target.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
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

    /// Bind a Resource API client to a sealed Resource API session subject.
    ///
    /// The wrapper is issued only after ComponentSession or root-listener
    /// authentication and native policy evaluation. Callers cannot construct
    /// it from a request payload.
    pub(crate) fn bind_operator_resource_client(
        &self,
        subject: d2b_resource_api::AuthenticatedSubjectContext,
    ) -> Result<
        Arc<ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>>,
        ResourceRuntimeError,
    > {
        let adapter = ResourceBusAdapter::bind_component_session(Arc::clone(&self.api), subject)
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?;
        Ok(Arc::new(adapter.client()))
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
        // G5: the controller-session path (and the Provider's dependency
        // observation) read manager-served controller rows through the same
        // published table, so the session coordinator gets the zone plane's
        // manager view seam here, before activation. This runs before the
        // composition fills the table (it is published after the per-zone
        // loop), so the seam resolves the zone's plane per read.
        self.controller_session_coordinator()
            .attach_plane_view(Arc::new(PublishedPlaneControllerView::new(
                Arc::clone(&planes),
                self.zone.clone(),
            )));
    }

    /// The manager-backed Resource API service for this Zone's converted
    /// types (U8/U9 F1 wiring): the manager client and watch hub come from
    /// the published v3 plane, the policy from the Zone's manager-plane
    /// authorizer. Built once, on first use.
    /// The Zone's durable store handle (read-only seam for the converted
    /// shared-provider effects).
    pub(crate) fn store(&self) -> &Arc<RedbResourceStore> {
        &self.store
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
        let plane = self.manager_plane_view();
        bridge_manager_rows(plane.as_deref(), resource_type).await
    }

    /// The plane-view seam over the published per-zone plane, `None` until
    /// the composition publishes it (U12 bridge: the same `Ok(empty)`-keeps-
    /// the-old-path contract G5 uses).
    fn manager_plane_view(&self) -> Option<Arc<dyn ControllerPlaneView>> {
        let plane = self.v3_plane().ok()?;
        Some(Arc::new(ManagerControllerPlaneView::new(
            plane.client().clone(),
            self.zone.clone(),
        )))
    }

    /// Merge the manager-served rows of every requested type into a
    /// store-shaped reader result (the multi-type form of
    /// [`Self::manager_stored_rows`]).
    pub(crate) async fn merge_manager_rows_for_types(
        &self,
        resource_types: &[&str],
        rows: &mut Vec<StoredResource>,
    ) -> Result<(), ResourceRuntimeError> {
        let plane = self.manager_plane_view();
        bridge_merge_rows_for_types(plane.as_deref(), resource_types, rows).await
    }

    /// The committed policy inputs for this Zone, durable rows merged with
    /// the manager-served ones (U12 bridge; the policy loader itself is
    /// redb-only).
    pub(crate) async fn committed_policy_resources(
        &self,
        operation_id: &str,
    ) -> Result<Vec<StoredResource>, ResourceRuntimeError> {
        let plane = self.manager_plane_view();
        committed_policy_resources_bridged(&self.zone, &self.store, plane.as_deref(), operation_id)
            .await
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

    #[cfg(feature = "test-support")]
    pub fn bind_operator_resource_client_for_test(
        &self,
        context: d2b_contracts_resource::v3::identity::AuthenticatedSubjectContext,
    ) -> Result<
        Arc<ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>>,
        ResourceRuntimeError,
    > {
        let state = self
            .authorization_state
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        let subject = self
            .authorizer
            .issue_authenticated_subject(context, state)
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
        self.bind_operator_resource_client(subject)
    }

    /// Borrow the daemon-owned Resource API client used by the target-local
    /// process reconciler. The client is present only after the Zone's
    /// authenticated system-core session has been enrolled.
    pub(crate) fn process_resource_client(
        &self,
    ) -> Option<Arc<ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>>> {
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
        Arc<ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>>,
        ResourceRuntimeError,
    > {
        self.process_status_client
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)
    }

    async fn cloud_hypervisor_assigned_mutation_api(
        &self,
        provider_ref: &ResourceRef,
    ) -> Result<Arc<RedbRegisteredControllerApi>, ResourceRuntimeError> {
        let provider = retry_transient_store_read(
            &self.zone,
            "cloud-hypervisor-assignment-provider",
            || {
                self.store.get(StoreGetRequest {
                    operation: StoreOperationContext {
                        operation_id: "cloud-hypervisor-assignment-provider".to_owned(),
                        idempotency_key: None,
                        correlation_id: "cloud-hypervisor-assignment-provider".to_owned(),
                        trace_id: None,
                        deadline_ms: 10_000,
                    },
                    zone: self.zone.clone(),
                    target: provider_ref.clone(),
                    expected_uid: None,
                    projection: StoreProjection::MetadataOnly,
                })
            },
        )
        .await
        .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        if provider.zone != self.zone
            || provider.resource_ref != *provider_ref
            || provider.generation.get() == 0
        {
            return Err(ResourceRuntimeError::HandlerNotReady);
        }
        let provider_ref_text = provider_ref.to_canonical_string();
        let registration = U6_SHARED_PROVIDER_RUNNERS
            .iter()
            .copied()
            .find(|registration| registration.provider_ref == provider_ref_text)
            .ok_or(ResourceRuntimeError::CapabilityUnavailable)?;
        let subject_context = self
            .core_controller_subject
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        let authorization_state = self
            .authorization_state
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        let controller_generation = self
            .store_metadata
            .policy_snapshot
            .controller_generation
            .ok_or(ResourceRuntimeError::HandlerNotReady)?;
        let session_generation = subject_context.reconnect_generation();
        let provider_generations = BTreeMap::from([(provider_ref.clone(), provider.generation)]);
        let descriptor = compose_shared_guest_runner_descriptors(
            [registration],
            self.zone.clone(),
            controller_generation,
            &provider_generations,
            session_generation,
        )?
        .into_iter()
        .next()
        .map(|(_, descriptor)| descriptor)
        .ok_or(ResourceRuntimeError::HandlerNotReady)?;
        let controller_ref = ResourceRef::parse(registration.controller_ref)
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        let (assignments, authority) = self
            .u12_controller_assignments(
                &descriptor,
                controller_ref,
                provider.generation,
                controller_generation,
                session_generation,
            )
            .await?;
        let subject = self
            .authorizer
            .issue_authenticated_subject(subject_context, authorization_state.clone())
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
        let api = self
            .api
            .registered_controller_api(subject, authorization_state, assignments)
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?;
        let allowed_types = descriptor.resource_types().cloned().collect::<BTreeSet<_>>();
        Ok(Arc::new(api.with_assignment_fence_resolver(
            shared_provider_assignment_fence_resolver(
                Arc::clone(&self.store),
                allowed_types,
                authority,
            ),
        )))
    }

    fn process_controller_api(
        &self,
        mode: DaemonMode,
        authority: Arc<CoreAssignmentAuthority>,
        authorization_state: AuthorizationState,
    ) -> Result<RedbRegisteredControllerApi, ResourceRuntimeError> {
        let subject_context = self
            .core_controller_subject
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        let subject = self
            .authorizer
            .issue_authenticated_subject(subject_context, authorization_state.clone())
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
        Ok(self
            .api
            .registered_controller_api(subject, authorization_state, Vec::new())
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?
            .with_assignment_fence_resolver(process_assignment_fence_resolver(
                Arc::clone(&self.store),
                mode,
                authority,
            )))
    }

    // Callers hold controller_session_lock so the authority and API publish
    // cannot race controller-session evidence writes.
    async fn rebuild_assigned_process_api_locked(
        &self,
        providers: &crate::process_provider_runtime::ProductionProcessProviders,
    ) -> Result<(Arc<CoreAssignmentAuthority>, AuthorizationState), ResourceRuntimeError> {
        let core_authority = self.core_assignment_fences().await?.4;
        let authorization_state = self
            .authorization_state
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        let process_api = Arc::new(self.process_controller_api(
            providers.mode(),
            Arc::clone(&core_authority),
            authorization_state.clone(),
        )?);
        self.controller_session_coordinator()
            .set_assigned_process_api(process_api)?;
        Ok((core_authority, authorization_state))
    }

    async fn core_assignment_fences(
        &self,
    ) -> Result<
        (
            Vec<(ResourceRef, ResourceAssignmentFence)>,
            ResourceGeneration,
            ControllerGeneration,
            ReconnectGeneration,
            Arc<CoreAssignmentAuthority>,
        ),
        ResourceRuntimeError,
    > {
        let subject = self
            .core_controller_subject
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        let metadata = retry_transient_store_read(
            &self.zone,
            "core-controller-assignment-metadata",
            || self.store.runtime_metadata(),
        )
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        let controller_generation = metadata
            .policy_snapshot
            .controller_generation
            .ok_or(ResourceRuntimeError::HandlerNotReady)?;
        let resource_types = crate::core_driver::CORE_RESOURCE_TYPES
            .iter()
            .map(|resource_type| {
                ResourceTypeName::parse((*resource_type).to_owned())
                    .map_err(|_| ResourceRuntimeError::HandlerNotReady)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut resources = Vec::new();
        let mut cursor = None;
        loop {
            let request = StoreListRequest {
                    operation: StoreOperationContext {
                        operation_id: "core-controller-assignment-relist".to_owned(),
                        idempotency_key: None,
                        correlation_id: "core-controller-assignment-relist".to_owned(),
                        trace_id: None,
                        deadline_ms: 10_000,
                    },
                    zone: self.zone.clone(),
                    resource_types: resource_types.clone(),
                    resource_names: Vec::new(),
                    filters: Vec::new(),
                    page_size: 256,
                    cursor: cursor.clone(),
                    projection: StoreProjection::MetadataOnly,
                };
            let page = retry_transient_store_list(
                &self.zone,
                "core-controller-assignment-relist",
                || self.store.list(request.clone()),
            )
            .await
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
            resources.extend(page.resources);
            if resources.len() > d2b_core_controller::controller_assignment::MAX_ASSIGNMENTS {
                return Err(ResourceRuntimeError::AuthorizationUnavailable);
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        // U12 bridge: the nine Core-family types are manager rows now; a
        // converted row the pre-v3 store no longer holds must still fence
        // (the manager row wins where both hold the reference).
        self.merge_manager_rows_for_types(&crate::core_driver::CORE_RESOURCE_TYPES, &mut resources)
            .await?;
        let provider_ref = ResourceRef::parse(CORE_CONTROLLER_PROVIDER_REF)
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        let provider_generation = resources
            .iter()
            .find(|resource| resource.resource_ref == provider_ref)
            .map(|resource| resource.generation)
            .ok_or(ResourceRuntimeError::HandlerNotReady)?;
        let controller_ref = ResourceRef::parse(CORE_CONTROLLER_PROCESS_REF)
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        let target = ResourceRef::parse(&format!("Zone/{}", self.zone.as_str()))
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        let session_generation = subject.reconnect_generation();
        let authority = Arc::new(CoreAssignmentAuthority {
            provider_generation,
            controller_generation,
            session_generation,
            controller_role: controller_ref.clone(),
            target: target.clone(),
        });
        let assignments = resources
            .into_iter()
            .map(|resource| {
                let fence = ResourceAssignmentFence {
                    resource_uid: resource.uid.clone(),
                    resource_revision: resource.revision,
                    provider_generation,
                    controller_generation,
                    controller_role: controller_ref.clone(),
                    target: target.clone(),
                    session_generation,
                    epoch: ASSIGNMENT_EPOCH,
                    scope: ResourceAssignmentScope::Primary,
                };
                (resource.resource_ref, fence)
            })
            .collect();
        Ok((
            assignments,
            provider_generation,
            controller_generation,
            session_generation,
            authority,
        ))
    }

    async fn provider_resources_present(
        &self,
        provider_ref: &str,
        resource_types: &[&str],
    ) -> Result<bool, ResourceRuntimeError> {
        for resource_type in resource_types {
            if self
                .committed_resources_of_type(resource_type)
                .await?
                .iter()
                .any(|resource| {
                    resource
                        .pointer("/spec/providerRef")
                        .and_then(Value::as_str)
                        == Some(provider_ref)
                })
            {
                return Ok(true);
            }
        }
        Ok(self
            .committed_resources_of_type("Process")
            .await?
            .iter()
            .any(|resource| {
                resource
                    .pointer("/spec/providerRef")
                    .and_then(Value::as_str)
                    == Some(provider_ref)
                    || resource
                        .pointer("/metadata/ownerRef")
                        .and_then(Value::as_str)
                        == Some(provider_ref)
            }))
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
        let facts_store = Arc::clone(&self.store);
        let facts_zone = self.zone.clone();
        Arc::new(ProductionCredentialDriverEffects::new(
            Arc::new(move |provider_ref: &ResourceRef, execution_ref: &ResourceRef| {
                let store = Arc::clone(&facts_store);
                let zone = facts_zone.clone();
                let provider_ref = provider_ref.clone();
                let execution_ref = execution_ref.clone();
                Box::pin(async move {
                    credential_dependency_facts(&store, &zone, &provider_ref, &execution_ref).await
                })
            }),
            Arc::new(|_credential_ref: &ResourceRef| Box::pin(async { None })),
            agent_ready,
            self.credential_sessions.clone(),
        ))
    }

    async fn stop_u6_controller_runners_locked(&self) -> Result<(), ResourceRuntimeError> {
        let tasks = {
            let mut tasks = self
                .u6_runner_tasks
                .lock()
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            std::mem::take(&mut *tasks)
        };
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
        self.u6_required.store(false, Ordering::Release);
        Ok(())
    }

    /// Attach the selected Guest runtime Providers to the production shared
    /// Runner. Provider selection remains an exact `Guest.spec.providerRef`
    /// admission rule.
    pub(crate) async fn start_u6_controller_runners(
        &self,
        state: Arc<crate::ServerState>,
    ) -> Result<(), ResourceRuntimeError> {
        let _runner_guard = self.u6_runner_lock.lock().await;
        let result = self
            .start_u6_controller_runners_locked(Arc::clone(&state))
            .await;
        if result.is_ok() {
            match self.u6_state.lock() {
                Ok(mut current) => *current = Some(state),
                Err(_) => {
                    tracing::warn!(
                        controller = "guest",
                        "U6 controller runner state lock poisoned; stopping runners",
                    );
                    self.stop_u6_controller_runners_locked().await?;
                    return Err(ResourceRuntimeError::AuthenticationUnavailable);
                }
            }
        }
        result
    }

    async fn start_u6_controller_runners_locked(
        &self,
        state: Arc<crate::ServerState>,
    ) -> Result<(), ResourceRuntimeError> {
        if !self.readiness.resource_api_ready {
            return Ok(());
        }
        {
            let tasks = self
                .u6_runner_tasks
                .lock()
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            if tasks.iter().any(|task| !task.is_finished()) {
                return Ok(());
            }
        }
        let stale = {
            let mut tasks = self
                .u6_runner_tasks
                .lock()
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            std::mem::take(&mut *tasks)
        };
        for task in stale {
            if let Err(error) = task.await {
                tracing::warn!(
                    controller = "guest",
                    error = ?error,
                    "stale U6 controller runner task died",
                );
            }
        }
        let required = guest_provider_runtime::start(self, state).await?;
        self.u6_required.store(required, Ordering::Release);
        Ok(())
    }

    async fn u12_controller_assignments(
        &self,
        descriptor: &d2b_core_controller::ControllerDescriptor,
        controller_ref: ResourceRef,
        provider_generation: ResourceGeneration,
        controller_generation: ControllerGeneration,
        session_generation: ReconnectGeneration,
    ) -> Result<
        (
            Vec<(ResourceRef, ResourceAssignmentFence)>,
            Arc<CoreAssignmentAuthority>,
        ),
        ResourceRuntimeError,
    > {
        let resource_types = descriptor.resource_types().cloned().collect::<Vec<_>>();
        let provider_selector = descriptor
            .watch_selectors()
            .iter()
            .find(|selector| selector.field() == SelectorField::Spec)
            .and_then(|selector| selector.exact_value())
            .map(str::to_owned);
        let assignment_projection = if provider_selector.is_some() {
            StoreProjection::BaseOnly
        } else {
            StoreProjection::MetadataOnly
        };
        let mut resources = Vec::new();
        let mut cursor = None;
        loop {
            let request = StoreListRequest {
                    operation: StoreOperationContext {
                        operation_id: "u12-controller-assignment-relist".to_owned(),
                        idempotency_key: None,
                        correlation_id: "u12-controller-assignment-relist".to_owned(),
                        trace_id: None,
                        deadline_ms: 10_000,
                    },
                    zone: self.zone.clone(),
                    resource_types: resource_types.clone(),
                    resource_names: Vec::new(),
                    filters: Vec::new(),
                    page_size: 256,
                    cursor: cursor.clone(),
                    projection: assignment_projection,
                };
            let page = retry_transient_store_list(
                &self.zone,
                "u12-controller-assignment-relist",
                || self.store.list(request.clone()),
            )
            .await
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
            resources.extend(page.resources);
            if resources.len() > d2b_core_controller::controller_assignment::MAX_ASSIGNMENTS {
                return Err(ResourceRuntimeError::AuthorizationUnavailable);
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        // U12 bridge: the descriptor's types are manager rows now (the nine
        // Core-family types), so the assignment scan merges what the store no
        // longer holds.
        let bridge_types = resource_types
            .iter()
            .map(ResourceTypeName::as_str)
            .collect::<Vec<_>>();
        self.merge_manager_rows_for_types(&bridge_types, &mut resources)
            .await?;
        let target = ResourceRef::parse(&format!("Zone/{}", self.zone.as_str()))
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        let resources = resources
            .into_iter()
            .filter(|resource| {
                let Some(expected_provider) = provider_selector.as_deref() else {
                    return true;
                };
                serde_json::from_slice::<Value>(&resource.canonical_json)
                    .ok()
                    .and_then(|value| {
                        value
                            .pointer("/spec/providerRef")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .is_some_and(|provider| provider == expected_provider)
            })
            .collect::<Vec<_>>();
        let authority = Arc::new(CoreAssignmentAuthority {
            provider_generation,
            controller_generation,
            session_generation,
            controller_role: controller_ref.clone(),
            target: target.clone(),
        });
        let assignments = resources
            .into_iter()
            .filter(|resource| {
                resource
                    .resource_ref
                    .resource_type()
                    .to_canonical_string()
                    != "Provider"
            })
            .map(|resource| {
                (
                    resource.resource_ref,
                    ResourceAssignmentFence {
                        resource_uid: resource.uid,
                        resource_revision: resource.revision,
                        provider_generation,
                        controller_generation,
                        controller_role: controller_ref.clone(),
                        target: target.clone(),
                        session_generation,
                        epoch: ASSIGNMENT_EPOCH,
                        scope: ResourceAssignmentScope::Primary,
                    },
                )
            })
            .collect();
        Ok((assignments, authority))
    }

    /// Persist a provider phase together with its typed durable projection.
    ///
    /// Provider readiness must be observed from this committed projection on
    /// the next reconcile pass; an in-memory effect port is not an authority
    /// for restart or dependent-resource admission.
    pub(crate) async fn persist_public_reconcile_status(
        &self,
        resource_ref: &ResourceRef,
        resource_uid: &ResourceUid,
        operation_id: &str,
        phase: &str,
        resource_projection: Option<&Value>,
    ) -> Result<(), ResourceRuntimeError> {
        let resource = self
            .backend
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: operation_id.to_owned(),
                    idempotency_key: None,
                    correlation_id: operation_id.to_owned(),
                    trace_id: None,
                    deadline_ms: 30_000,
                },
                zone: self.zone.clone(),
                target: resource_ref.clone(),
                expected_uid: Some(resource_uid.clone()),
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        let current = serde_json::from_slice::<Value>(&resource.canonical_json)
            .map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
        let current_phase = current
            .get("status")
            .and_then(|status| status.get("phase"))
            .and_then(Value::as_str);
        let current_observed_generation = current
            .get("status")
            .and_then(|status| status.get("observedGeneration"))
            .and_then(Value::as_u64);
        if current_phase == Some(phase)
            && current_observed_generation == Some(resource.generation.get())
            && resource_projection.is_none()
        {
            return Ok(());
        }
        let status = json!({ "phase": phase });
        let client = self
            .status_client()
            .map_err(|_| ResourceRuntimeError::ControllerEndpointUnavailable)?;
        let projection = resource_projection.or_else(|| {
            current
                .get("status")
                .and_then(|status| status.get("resource"))
        });
        persist_resource_status_with_projection(&client, &resource, &status, projection).await
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
        client: &ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>,
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
        let (resource, snapshot_revision) = current_committed_resource(
            &self.zone,
            &self.store,
            identity.wayland_session_ref(),
            "interaction-wayland-session-current",
        )
        .await?;
        let spec = committed_wayland_session_spec(
            &self.zone,
            snapshot_revision,
            &resource,
        )?;
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
        let resource_type = ResourceTypeName::parse(resource_type.to_owned())
            .map_err(|_| ResourceRuntimeError::RequestInvalid)?;
        let mut cursor = None;
        let mut out: Vec<Value> = Vec::new();
        loop {
            let request = StoreListRequest {
                    operation: StoreOperationContext {
                        operation_id: "network-admission-scan".to_owned(),
                        idempotency_key: None,
                        correlation_id: "network-admission-scan".to_owned(),
                        trace_id: None,
                        deadline_ms: 10_000,
                    },
                    zone: self.zone.clone(),
                    resource_types: vec![resource_type.clone()],
                    resource_names: Vec::new(),
                    filters: Vec::new(),
                    page_size: 512,
                    cursor: cursor.clone(),
                    projection: StoreProjection::Full,
                };
            let page = retry_transient_store_list(
                &self.zone,
                "network-admission-scan",
                || self.store.list(request.clone()),
            )
            .await
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
            for resource in page.resources {
                out.push(
                    serde_json::from_slice(&resource.canonical_json)
                        .map_err(|_| ResourceRuntimeError::StoreReadFailed)?,
                );
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        // U12/G5 reader bridge: a converted type's rows live in the manager
        // (the store keeps the pre-v3 mirror U14 deletes), so the manager row
        // is authoritative where both hold the same reference. The merge is a
        // no-op for unconverted types and when no plane is published.
        let manager_rows = self.manager_stored_rows(resource_type.as_str()).await?;
        for row in manager_rows {
            let Ok(value) = serde_json::from_slice::<Value>(&row.canonical_json) else {
                continue;
            };
            match out.iter_mut().find(|existing| {
                existing.get("type").and_then(Value::as_str)
                    == Some(row.resource_ref.resource_type().as_str())
                    && existing
                        .get("metadata")
                        .and_then(|metadata| metadata.get("name"))
                        .and_then(Value::as_str)
                        == Some(row.resource_ref.name().as_str())
            }) {
                Some(existing) => *existing = value,
                None => out.push(value),
            }
        }
        Ok(out)
    }

    pub(crate) async fn committed_resource_value(
        &self,
        target: &ResourceRef,
        operation_id: &str,
    ) -> Result<Value, ResourceRuntimeError> {
        // U12/G5 reader bridge: the manager is the authority for a converted
        // type. A row the manager does not hold (or an unpublished plane)
        // keeps the caller on the durable store path.
        if let Some(row) = self
            .manager_stored_rows(target.resource_type().as_str())
            .await?
            .into_iter()
            .find(|row| row.resource_ref == *target)
        {
            return serde_json::from_slice::<Value>(&row.canonical_json)
                .map_err(|_| ResourceRuntimeError::StoreReadFailed);
        }
        let request = StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: operation_id.to_owned(),
                    idempotency_key: None,
                    correlation_id: operation_id.to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target: target.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            };
        let resource = retry_transient_store_read(&self.zone, operation_id, || {
            self.store.get(request.clone())
        })
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        if resource.zone != self.zone || resource.resource_ref != *target {
            return Err(ResourceRuntimeError::StoreReadFailed);
        }
        serde_json::from_slice(&resource.canonical_json)
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)
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
            &self.store_metadata,
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
        self.reconcile_cloud_hypervisor_guests_inner(state, None)
            .await
            .map(|_| ())
    }

    /// Reconcile one Cloud Hypervisor Guest selected by the shared Runner.
    ///
    /// The legacy relist helper remains available to explicit lifecycle
    /// commands, but the shared Runner always supplies one exact Guest key.
    pub(crate) async fn reconcile_cloud_hypervisor_guest(
        &self,
        state: Arc<crate::ServerState>,
        guest_ref: &ResourceRef,
    ) -> Result<CloudHypervisorReconcileOutcome, ResourceRuntimeError> {
        self.reconcile_cloud_hypervisor_guests_inner(state, Some(guest_ref))
            .await
    }

    async fn reconcile_cloud_hypervisor_guests_inner(
        &self,
        state: Arc<crate::ServerState>,
        selected_guest: Option<&ResourceRef>,
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
                &self.store_metadata.zone_uid,
                &guest_ref,
                &guest_uid,
                guest_generation,
                provider_assignment_generation,
                self.store_metadata.policy_snapshot.policy_revision,
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
            let assigned_mutation_api = self
                .cloud_hypervisor_assigned_mutation_api(&provider_ref)
                .await
                .inspect_err(|error| {
                    tracing::warn!(
                        error = ?error,
                        "Cloud Hypervisor reconcile stage failed: assignment-api",
                    );
                })?;
            let session = CloudHypervisorResourceSession {
                client: Arc::clone(&client),
                assigned_mutation_api,
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
                zone_uid: self.store_metadata.zone_uid.clone(),
                policy_revision: self.store_metadata.policy_snapshot.policy_revision,
                provider_ref,
                execution_ref,
                descriptor: descriptor.clone(),
                controller_generation: self
                    .store_metadata
                    .policy_snapshot
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
            self.reconcile_cloud_hypervisor_setup_volume(&state, &guest_ref)
                .await
                .inspect_err(|error| {
                    tracing::warn!(
                        zone = %self.zone.as_str(),
                        guest = %guest_ref.name().as_str(),
                        stage = "setup-volume",
                        error = ?error,
                        "Cloud Hypervisor reconcile stage failed",
                    );
                })?;
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
        let process = self
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "cloud-hypervisor-endpoint-process".to_owned(),
                    idempotency_key: None,
                    correlation_id: "cloud-hypervisor-endpoint-process".to_owned(),
                    trace_id: None,
                    deadline_ms: 30_000,
                },
                zone: self.zone.clone(),
                target: process_ref,
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        let process_envelope = ResourceEnvelope::from_json(&process.canonical_json)
            .map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
        if process_envelope.metadata().owner_ref() != Some(guest_ref)
        {
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
        let process_phase = process_envelope.status().phase();
        if process_phase != ResourcePhase::Ready {
            tracing::debug!(
                zone = %self.zone.as_str(),
                guest = %guest_ref.name().as_str(),
                phase = ?process_phase,
                "Cloud Hypervisor endpoint publication deferred until VMM Process is Ready",
            );
            return if matches!(
                process_phase,
                ResourcePhase::Pending | ResourcePhase::Unknown | ResourcePhase::Degraded
            ) {
                Ok(CloudHypervisorEndpointOutcome::Pending)
            } else {
                Err(ResourceRuntimeError::CapabilityUnavailable)
            };
        }
        let provider_ref = ResourceRef::parse("Provider/runtime-cloud-hypervisor")
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        for role in [ChildRole::ChApiEndpoint, ChildRole::GuestControlEndpoint] {
            let endpoint_ref = deterministic_child_ref(guest_ref, role)
                .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
            let endpoint = self
                .store
                .get(StoreGetRequest {
                    operation: StoreOperationContext {
                        operation_id: "cloud-hypervisor-endpoint".to_owned(),
                        idempotency_key: None,
                        correlation_id: "cloud-hypervisor-endpoint".to_owned(),
                        trace_id: None,
                        deadline_ms: 30_000,
                    },
                    zone: self.zone.clone(),
                    target: endpoint_ref.clone(),
                    expected_uid: None,
                    projection: StoreProjection::Full,
                })
                .await
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
            let envelope = ResourceEnvelope::from_json(&endpoint.canonical_json)
                .map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
            if envelope.metadata().owner_ref() != Some(guest_ref)
                || envelope.spec().provider_ref() != Some(&provider_ref)
            {
                return Err(ResourceRuntimeError::CapabilityUnavailable);
            }
            let operation_id = format!(
                "cloud-hypervisor-endpoint-ready-{}-{}",
                endpoint.uid.as_str(),
                endpoint.revision.get(),
            );
            let projection = json!({
                "endpointGeneration": endpoint.generation.get(),
            });
            self.persist_public_reconcile_status(
                &endpoint_ref,
                &endpoint.uid,
                &operation_id,
                "Ready",
                Some(&projection),
            )
            .await?;
        }
        Ok(CloudHypervisorEndpointOutcome::Ready)
    }

    async fn reconcile_cloud_hypervisor_setup_volume(
        &self,
        state: &crate::ServerState,
        guest_ref: &ResourceRef,
    ) -> Result<(), ResourceRuntimeError> {
        let volume_ref =
            deterministic_child_ref(guest_ref, ChildRole::SystemVolume).map_err(|_| {
                tracing::warn!("Cloud Hypervisor setup Volume ref derivation failed");
                ResourceRuntimeError::CapabilityUnavailable
            })?;
        let volume = self
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "cloud-hypervisor-setup-volume".to_owned(),
                    idempotency_key: None,
                    correlation_id: "cloud-hypervisor-setup-volume".to_owned(),
                    trace_id: None,
                    deadline_ms: 30_000,
                },
                zone: self.zone.clone(),
                target: volume_ref.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| {
                tracing::warn!("Cloud Hypervisor setup Volume read failed");
                ResourceRuntimeError::StoreReadFailed
            })?;
        let envelope = ResourceEnvelope::from_json(&volume.canonical_json).map_err(|_| {
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
        Ok(())
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
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "cloud-hypervisor-controller-deployment".to_owned(),
                    idempotency_key: None,
                    correlation_id: "cloud-hypervisor-controller-deployment".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target: provider_ref.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        let manifest = d2b_provider_runtime_cloud_hypervisor::provider_manifest()
            .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        let controller_generation = self
            .store_metadata
            .policy_snapshot
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
            let resource = match self
                .store
                .get(StoreGetRequest {
                    operation: StoreOperationContext {
                        operation_id: "cloud-hypervisor-lifecycle-dependencies".to_owned(),
                        idempotency_key: None,
                        correlation_id: "cloud-hypervisor-lifecycle-dependencies".to_owned(),
                        trace_id: None,
                        deadline_ms: 10_000,
                    },
                    zone: self.zone.clone(),
                    target: resource_ref.clone(),
                    expected_uid: None,
                    projection: StoreProjection::Full,
                })
                .await
            {
                Ok(resource) => resource,
                Err(error) if error.kind() == StoreErrorKind::ResourceNotFound => {
                    return Ok(false);
                }
                Err(error) => {
                    tracing::debug!(
                        error = %error,
                        resource = %resource_ref.to_canonical_string(),
                        "Cloud Hypervisor dependency read failed",
                    );
                    return Err(ResourceRuntimeError::StoreReadFailed);
                }
            };
            let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
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
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "cloud-hypervisor-guest-ready".to_owned(),
                    idempotency_key: None,
                    correlation_id: "cloud-hypervisor-guest-ready".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target: guest_ref.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
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
        let client = self.status_client()?;
        let current = self
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "cloud-hypervisor-vmm-lifecycle".to_owned(),
                    idempotency_key: None,
                    correlation_id: "cloud-hypervisor-vmm-lifecycle".to_owned(),
                    trace_id: None,
                    deadline_ms: 30_000,
                },
                zone: self.zone.clone(),
                target: process_ref.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
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
        let process = self
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "cloud-hypervisor-vmm-state".to_owned(),
                    idempotency_key: None,
                    correlation_id: "cloud-hypervisor-vmm-state".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target: process_ref,
                expected_uid: None,
                projection: StoreProjection::Full,
            })
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
            self.store_metadata
                .policy_snapshot
                .controller_generation
                .unwrap_or_else(|| ControllerGeneration::new(1).expect("generation one")),
            Some(guest_ref.clone()),
        )
        .with_lifecycle_identity(
            Some(self.store_metadata.zone_uid.clone()),
            Some(self.store_metadata.policy_snapshot.policy_revision),
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
        let resource_type =
            ResourceTypeName::parse("Guest").map_err(|_| ResourceRuntimeError::RequestInvalid)?;
        let mut request = StoreListRequest {
            operation: StoreOperationContext {
                operation_id: "cloud-hypervisor-guest-relist".to_owned(),
                idempotency_key: None,
                correlation_id: "cloud-hypervisor-guest-relist".to_owned(),
                trace_id: None,
                deadline_ms: 10_000,
            },
            zone: self.zone.clone(),
            resource_types: vec![resource_type],
            resource_names: Vec::new(),
            filters: Vec::new(),
            page_size: 256,
            cursor: None,
            projection: StoreProjection::Full,
        };
        let mut guests = Vec::new();
        loop {
            let page = self
                .store
                .list(request.clone())
                .await
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
            for resource in page.resources {
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
            request.cursor = page.next_cursor;
            if request.cursor.is_none() {
                break;
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
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "cloud-hypervisor-binding-admission".to_owned(),
                    idempotency_key: None,
                    correlation_id: "cloud-hypervisor-binding-admission".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target: spec.volume_ref().clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
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
        let guest = self
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "cloud-hypervisor-guest-inputs".to_owned(),
                    idempotency_key: None,
                    correlation_id: "cloud-hypervisor-guest-inputs".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target: guest_ref.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        let envelope = ResourceEnvelope::from_json(&guest.canonical_json)
            .map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
        let provider_ref = envelope
            .spec()
            .provider_ref()
            .cloned()
            .filter(|reference| d2b_provider_runtime_cloud_hypervisor::is_provider_ref(reference))
            .ok_or(ResourceRuntimeError::CapabilityUnavailable)?;
        let (provider, snapshot_revision) = current_committed_resource(
            &self.zone,
            &self.store,
            &provider_ref,
            "cloud-hypervisor-guest-inputs",
        )
        .await?;
        let guest_spec =
            serde_json::from_slice::<GuestSpec>(&envelope.spec().base().to_canonical_bytes())
                .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
        let (provider_spec, _, _, _, _) = committed_provider_spec(
            &self.zone,
            snapshot_revision,
            &provider,
            &provider_ref,
        )?;
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
        let mut cursor = None;
        loop {
            let request = StoreListRequest {
                    operation: StoreOperationContext {
                        operation_id: "cloud-hypervisor-binding-inputs".to_owned(),
                        idempotency_key: None,
                        correlation_id: "cloud-hypervisor-binding-inputs".to_owned(),
                        trace_id: None,
                        deadline_ms: 10_000,
                    },
                    zone: self.zone.clone(),
                    resource_types: vec![ResourceTypeName::parse(
                        d2b_contracts_resource::v3::VOLUME_BINDING_RESOURCE_TYPE,
                    )
                    .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?],
                    resource_names: Vec::new(),
                    filters: Vec::new(),
                    page_size: 256,
                    cursor: cursor.clone(),
                    projection: StoreProjection::Full,
                };
            let page = self
                .store
                .list(request)
                .await
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
            binding_refs.extend(
                self.admitted_guest_binding_refs(&page.resources, guest_ref)
                    .await,
            );
            if page.next_cursor.is_none() {
                break;
            }
            cursor = page.next_cursor;
        }
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
            store: Arc::clone(&self.store),
            plane_view: Arc::new(Mutex::new(None)),
            assigned_process_api: Arc::new(Mutex::new(None)),
            api: Arc::clone(&self.api),
            authorizer: Arc::clone(&self.authorizer),
            authorization_state: self.authorization_state.clone(),
            policy_projection: Arc::clone(&self.policy_projection),
            registrar: Arc::clone(&self.registrar),
            assignments: Arc::clone(&self.assignments),
            controller_sessions: Arc::clone(&self.controller_sessions),
            pending_controller_session_clears: Arc::new(Mutex::new(BTreeMap::new())),
            credential_sessions: self.credential_sessions.clone(),
            controller_session_lock: Arc::clone(&self.controller_session_lock),
            #[cfg(test)]
            reconcile_attempts: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            admission_test_seam: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            controller_session_evidence_test_errors: Arc::new(Mutex::new(Vec::new())),
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
    fn set_assigned_process_api(
        &self,
        api: Arc<RedbRegisteredControllerApi>,
    ) -> Result<(), ResourceRuntimeError> {
        *self
            .assigned_process_api
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? = Some(api);
        Ok(())
    }

    fn clear_assigned_process_api(&self) -> Result<(), ResourceRuntimeError> {
        *self
            .assigned_process_api
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? = None;
        Ok(())
    }

    #[cfg(test)]
    fn set_controller_session_admission_test_seam(
        &self,
        seam: ControllerSessionAdmissionTestSeam,
    ) {
        *self
            .admission_test_seam
            .lock()
            .expect("controller-session test seam lock") = Some(seam);
    }

    #[cfg(test)]
    fn set_controller_session_evidence_test_errors(
        &self,
        errors: Vec<ResourceRuntimeError>,
    ) {
        *self
            .controller_session_evidence_test_errors
            .lock()
            .expect("controller-session evidence test seam lock") = errors;
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

    fn controller_session_evidence_read_error(kind: StoreErrorKind) -> ResourceRuntimeError {
        match kind {
            StoreErrorKind::ResourceConflict => {
                ResourceRuntimeError::ResourceGetFailed(ResourceErrorKind::ResourceConflict)
            }
            StoreErrorKind::RevisionExpired => {
                ResourceRuntimeError::ResourceGetFailed(ResourceErrorKind::RevisionExpired)
            }
            StoreErrorKind::Backpressure | StoreErrorKind::StoreBackpressure => {
                ResourceRuntimeError::ResourceGetFailed(ResourceErrorKind::Backpressure)
            }
            StoreErrorKind::Timeout => {
                ResourceRuntimeError::ResourceGetFailed(ResourceErrorKind::Timeout)
            }
            StoreErrorKind::Cancelled => {
                ResourceRuntimeError::ResourceGetFailed(ResourceErrorKind::Cancelled)
            }
            StoreErrorKind::ResourcePlaneUnavailable => {
                ResourceRuntimeError::ResourceGetFailed(ResourceErrorKind::ResourcePlaneUnavailable)
            }
            _ => ResourceRuntimeError::StoreReadFailed,
        }
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

    /// The committed policy inputs for this Zone with the manager rows merged
    /// (U12 bridge; see `committed_policy_resources_bridged`).
    pub(crate) async fn committed_policy_resources(
        &self,
        operation_id: &str,
    ) -> Result<Vec<StoredResource>, ResourceRuntimeError> {
        let plane = self.plane_handle();
        committed_policy_resources_bridged(&self.zone, &self.store, plane.as_deref(), operation_id)
            .await
    }

    /// The committed `Provider` identities for the requested refs, manager
    /// rows first (U12 bridge).
    pub(crate) async fn committed_controller_provider_identities(
        &self,
        provider_refs: BTreeSet<ResourceRef>,
    ) -> Result<BTreeMap<ResourceRef, (ResourceUid, ResourceGeneration)>, ResourceRuntimeError> {
        let plane = self.plane_handle();
        committed_controller_provider_identities_bridged(
            &self.zone,
            &self.store,
            plane.as_deref(),
            provider_refs,
        )
        .await
    }
    /// The manager's row for one controller Process key, when the new plane
    /// serves it. `Ok(None)` keeps the caller on the durable store path (an
    /// unconverted or legacy row); a manager failure is reported, never
    /// folded into absence.
    async fn controller_plane_row(
        &self,
        process_ref: &ResourceRef,
    ) -> Result<Option<ResourceView>, ResourceRuntimeError> {
        let Some(plane) = self.plane_view.lock().ok().and_then(|slot| slot.clone()) else {
            return Ok(None);
        };
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
        #[cfg(test)]
        if session_generation.is_none()
            && let Some(error) = self
                .controller_session_evidence_test_errors
                .lock()
                .ok()
                .and_then(|mut errors| errors.pop())
        {
            return Err(error);
        }
        if let Some(view) = self.controller_plane_row(context.process_ref()).await? {
            // Manager-served row (KTD4): its status is never durable
            // (R11/AE6), so there is nothing to write or clear here - the
            // evidence is the live admitted session, which the Provider's
            // dependency observation reads on every pass. The identity check
            // keeps the old semantics: evidence for a row this context does
            // not describe still refuses.
            controller_session_evidence_identity_check(
                controller_plane_resource_matches(context, &view),
                session_generation.is_none(),
            )?;
            tracing::debug!(
                process = %context.process_ref(),
                "controller-session evidence for a manager-served row is the live session",
            );
            return Ok(());
        }
        let process = match self
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "controller-session-evidence-read".to_owned(),
                    idempotency_key: None,
                    correlation_id: "controller-session-evidence-read".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target: context.process_ref().clone(),
                expected_uid: Some(context.process_uid().clone()),
                projection: StoreProjection::Full,
            })
            .await
        {
            Ok(process) => process,
            Err(error) if error.kind() == StoreErrorKind::ResourceNotFound => {
                if session_generation.is_none() {
                    return Ok(());
                }
                return Err(ResourceRuntimeError::IdentityUnbound);
            }
            Err(error) => {
                return Err(Self::controller_session_evidence_read_error(error.kind()));
            }
        };
        if let Err(error) = controller_session_evidence_identity_check(
            controller_resource_matches(context, &process),
            session_generation.is_none(),
        ) {
            return Err(error);
        }
        let controller_session = session_generation.map(|session_generation| {
            json!({
                "ready": true,
                "providerRef": context.provider_owner_ref().to_canonical_string(),
                "providerUid": context.provider_uid().as_str(),
                "providerGeneration": context.provider_generation().get(),
                "processRef": context.process_ref().to_canonical_string(),
                "processUid": context.process_uid().as_str(),
                "processGeneration": context.generation().get(),
                "controllerGeneration": context.controller_generation().get(),
                "sessionGeneration": session_generation.get(),
                "artifactReady": true,
                "descriptorReady": true,
                "registrationReady": true,
            })
        });
        let api = self
            .assigned_process_api
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        persist_resource_controller_session_evidence(
            &api,
            &process,
            controller_session.as_ref(),
        )
        .await
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
        let store_metadata = retry_transient_store_read(
            &self.zone,
            "controller-session-fence-metadata",
            || self.store.runtime_metadata(),
        )
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        let current_controller_generation = store_metadata.policy_snapshot.controller_generation;
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
        #[cfg(test)]
        self.reconcile_attempts.fetch_add(1, Ordering::SeqCst);
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
        let store_metadata = retry_transient_store_read(
            &self.zone,
            "controller-session-reconcile-metadata",
            || self.store.runtime_metadata(),
        )
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
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
        #[cfg(test)]
        if let Some(seam) = self
            .admission_test_seam
            .lock()
            .ok()
            .and_then(|seam| seam.clone())
        {
            (seam.after_snapshot)(providers.as_ref());
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
        let policy_resources = self
            .committed_policy_resources("controller-session-policy")
            .await?;
        let (policy, state) =
            match d2bd_runtime::resource_runtime_support::compile_committed_policy_with_subjects(
                &self.zone,
                store_metadata.policy_snapshot,
                store_metadata.current_revision,
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
        #[cfg(test)]
        let installed_subjects = provider_subjects.clone();
        if let Err(error) = self
            .policy_projection
            .install(policy, state, provider_subjects)
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
        #[cfg(test)]
        if let Some(seam) = self
            .admission_test_seam
            .lock()
            .ok()
            .and_then(|seam| seam.clone())
        {
            (seam.after_policy_install)(&installed_subjects);
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
            #[cfg(test)]
            if let Some(_seam) = self
                .admission_test_seam
                .lock()
                .ok()
                .and_then(|seam| seam.clone())
                .filter(|seam| seam.admit_without_transport)
            {
                let current = match self
                    .controller_context_is_current(&providers, &context)
                    .await
                {
                    Ok(current) => current,
                    Err(ResourceRuntimeError::IdentityUnbound) => false,
                    Err(error) => return Err(error),
                };
                let expected_subject = BoundSubject {
                    subject_ref: context.provider_owner_ref().clone(),
                    subject_uid: context.provider_uid().clone(),
                };
                let policy_has_subject = self
                    .policy_projection
                    .installed_controller_subjects()
                    .is_ok_and(|subjects| subjects.contains(&expected_subject));
                *self
                    .registrar
                    .lock()
                    .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)? =
                    Some(registrar);
                assert!(
                    policy_has_subject,
                    "controller admission must use a policy compiled with its Provider subject"
                );
                if current {
                    assert!(
                        providers.activate_controller_bootstrap(&context),
                        "matching controller bootstrap must activate exactly once"
                    );
                } else {
                    providers.fail_controller_bootstrap(&context);
                }
                continue;
            }
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
        let metadata = retry_transient_store_read(
            &self.zone,
            "controller-session-context-metadata",
            || self.store.runtime_metadata(),
        )
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        if controller_generation_is_stale(
            metadata.policy_snapshot.controller_generation,
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
        // G5: a controller Process row minted in the new plane (KTD4) is not
        // in the durable store; the zone manager is its authority.
        if let Some(view) = self.controller_plane_row(context.process_ref()).await? {
            return Ok(controller_plane_resource_matches(context, &view));
        }
        let resources =
            crate::process_resource_runtime::list_process_resources(&self.store, &self.zone)
                .await
                .map_err(map_process_runtime_error)?;
        Ok(resources
            .iter()
            .find(|resource| resource.resource_ref == *context.process_ref())
            .is_some_and(|resource| controller_resource_matches(context, resource)))
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
        let mut cursor = None;
        let mut snapshot_revision = None;
        let mut resources = Vec::new();
        let mut resource_uids = BTreeSet::new();
        loop {
            let request = StoreListRequest {
                    operation: StoreOperationContext {
                        operation_id: "controller-assignment-list".to_owned(),
                        idempotency_key: None,
                        correlation_id: "controller-assignment-list".to_owned(),
                        trace_id: None,
                        deadline_ms: 10_000,
                    },
                    zone: self.zone.clone(),
                    resource_types: role.resource_types().iter().cloned().collect(),
                    resource_names: Vec::new(),
                    filters: Vec::new(),
                    page_size: 128,
                    cursor: cursor.clone(),
                    projection: StoreProjection::Full,
                };
            let page = retry_transient_store_list(
                &self.zone,
                "controller-assignment-list",
                || self.store.list(request.clone()),
            )
            .await
                .map_err(|error| {
                    if matches!(
                        error.kind(),
                        StoreErrorKind::RevisionExpired
                            | StoreErrorKind::Backpressure
                            | StoreErrorKind::Timeout
                            | StoreErrorKind::Cancelled
                            | StoreErrorKind::ResourcePlaneUnavailable
                            | StoreErrorKind::StoreBackpressure
                    ) {
                        ControllerAssignmentRefreshError::Retryable
                    } else {
                        ControllerAssignmentRefreshError::Failed(
                            ResourceRuntimeError::StoreReadFailed,
                        )
                    }
                })?;
            let page_resources =
                validate_assignment_list_page(&page, &self.zone, provider_ref, snapshot_revision)?;
            snapshot_revision.get_or_insert(page.snapshot_revision);
            if resources.len().saturating_add(page_resources.len())
                > d2b_core_controller::controller_assignment::MAX_ASSIGNMENTS
            {
                return Err(ControllerAssignmentRefreshError::Failed(
                    ResourceRuntimeError::AuthorizationUnavailable,
                ));
            }
            for envelope in page_resources {
                if !resource_uids.insert(envelope.metadata().uid().clone()) {
                    return Err(ControllerAssignmentRefreshError::Failed(
                        ResourceRuntimeError::AuthorizationUnavailable,
                    ));
                }
                resources.push(envelope);
            }
            cursor = page.next_cursor.clone();
            if cursor.is_none() {
                break;
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
            Option<Arc<ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>>>,
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

        let store_metadata = retry_transient_store_read(
            &self.zone,
            "controller-process-bootstrap-metadata",
            || self.store.runtime_metadata(),
        )
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        let provider_resource = committed_resource(
            &self.zone,
            &self.store,
            store_metadata.current_revision,
            context.provider_owner_ref(),
        )
        .await
        .map_err(|_| authentication_error("provider-resource-load"))?;
        let (_, provider_uid, provider_generation, _, _) = committed_provider_spec(
            &self.zone,
            store_metadata.current_revision,
            &provider_resource,
            context.provider_owner_ref(),
        )
        .map_err(|_| authentication_error("provider-resource-identity"))?;
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
        let transport = unix_transport(resource_socket, &policy)?;
        let responder = SessionEngine::establish_responder(
            transport,
            policy,
            HandshakeCredentials::Nn,
            std::time::Instant::now(),
        )
        .await
        .map_err(|_| authentication_error("session-handshake"))?;
        let candidate = acceptor
            .admit(
                responder,
                TransportEvidence::new(
                    EvidenceClass::UnixPeer,
                    BindingDigest::parse(format!("sha256:{}", "22".repeat(32)))
                        .map_err(|_| authentication_error("binding-digest"))?,
                ),
                1,
            )
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
                .open_named_stream(
                    stream,
                    PROVIDER_BOOTSTRAP_STREAM_CREDIT,
                    PROVIDER_BOOTSTRAP_STREAM_CREDIT,
                )
                .await
                .map_err(|_| authentication_error("provider-session-stream-open"))?;
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
                .open_named_stream(
                    key_stream,
                    PROVIDER_DELIVERY_KEY_STREAM_CREDIT,
                    PROVIDER_DELIVERY_KEY_STREAM_CREDIT,
                )
                .await
                .map_err(|_| authentication_error("provider-delivery-key-stream-open"))?;
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
            let ready_stream = StreamId::new(PROVIDER_READY_STREAM_ID)
                .map_err(|_| authentication_error("provider-session-ready-stream"))?;
            driver
                .open_named_stream(
                    ready_stream,
                    PROVIDER_READY_STREAM_CREDIT,
                    PROVIDER_READY_STREAM_CREDIT,
                )
                .await
                .map_err(|_| authentication_error("provider-session-ready-open"))?;
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
                ResourceBusAdapter::bind_component_session(Arc::clone(&self.api), subject)
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

    async fn refresh_controller_policy(
        &self,
        providers: &crate::process_provider_runtime::ProductionProcessProviders,
    ) -> Result<(), ResourceRuntimeError> {
        let store_metadata = retry_transient_store_read(
            &self.zone,
            "controller-policy-refresh-metadata",
            || self.store.runtime_metadata(),
        )
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        let provider_subjects = match load_controller_policy_subjects(
            &self.zone,
            &self.store,
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
        let policy_resources = self
            .committed_policy_resources("controller-policy-refresh")
            .await?;
        let (policy, state) =
            d2bd_runtime::resource_runtime_support::compile_committed_policy_with_subjects(
                &self.zone,
                store_metadata.policy_snapshot,
                store_metadata.current_revision,
                &self.bundle_resource_types,
                &policy_resources,
                provider_subjects.iter().cloned(),
            )
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
        self.policy_projection
            .install(policy, state, provider_subjects)?;
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
            self.rebuild_assigned_process_api_locked(&providers).await?;
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
        let resources = list_process_resources(&self.store, &self.zone)
            .await
            .map_err(map_process_runtime_error)?;
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

    /// Reserve a Host-global claim through the Zone's durable redb owner.
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
            self.authority_persistence.clone(),
            operation_id,
            request,
        )
        .await
    }

    /// Reserve an external physical-NIC claim through the same durable
    /// startup-barrier owner as generic Host-global claims.
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
            self.authority_persistence.clone(),
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
        if !self.readiness.store_ready {
            return Some(ResourceRuntimeError::StoreOpenFailed);
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

    /// Serve the existing CLI request envelope.
    ///
    /// This in-process compatibility entry point represents a trusted local
    /// caller with uid zero. The public socket uses
    /// [`Self::dispatch_public_cli_request`] with the authenticated
    /// `SO_PEERCRED` uid instead.
    #[cfg(test)]
    pub(crate) async fn dispatch_cli_request(
        &self,
        request: &Value,
    ) -> Result<Value, ResourceRuntimeError> {
        match self.dispatch_public_cli_request(request, 0).await {
            Ok(value) => Ok(value),
            Err(error) => Ok(compatibility_error_envelope(error)),
        }
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
        match public_request_route(request, method)? {
            crate::resource_plane_v3::PlaneRoute::NewPlane => {
                let service = self.manager_api_service().inspect_err(|error| {
                    tracing::warn!(
                        zone = %self.zone.as_str(),
                        error = ?error,
                        "public request refused: manager-backed API service is unavailable",
                    );
                })?;
                if self.policy_projection.installed_state().is_err() {
                    tracing::warn!(
                        zone = %self.zone.as_str(),
                        "public request refused: no authorization policy is installed",
                    );
                }
                let authorizer = self.manager_plane_authorizer().inspect_err(|error| {
                    tracing::warn!(
                        zone = %self.zone.as_str(),
                        error = ?error,
                        "public request refused: manager plane authorizer is unavailable",
                    );
                })?;
                let subject = match authorizer.issue_authenticated_subject(context.clone(), state.clone()) {
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
            crate::resource_plane_v3::PlaneRoute::OldPlane => {
                let subject = self
                    .authorizer
                    .issue_authenticated_subject(context, state)
                    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
                let client = self.bind_operator_resource_client(subject)?;
                self.dispatch_public_resource_call(&client, method, request, &operation_id)
                    .await
            }
        }
    }

    /// One public resource call against the plane's sealed client: converted
    /// types ride the manager-backed service, everything else the redb one.
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
            .backend
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: operation_id.to_owned(),
                    idempotency_key: None,
                    correlation_id: operation_id.to_owned(),
                    trace_id: None,
                    deadline_ms: 30_000,
                },
                zone: self.zone.clone(),
                target: device_ref.clone(),
                expected_uid: Some(device_uid.clone()),
                projection: StoreProjection::Full,
            })
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
        if legacy_intent_anchor.is_some() {
            // A live legacy TPM adoption is the first irreversible provider
            // effect. Refuse the admission if the owning store cannot produce
            // its logical recovery image first.
            self.backup_before_live_adoption().await?;
        }
        Ok(Self::tpm_migration_decision(
            vm_id,
            &intent,
            legacy_intent_anchor,
        ))
    }

    /// Capture the owning Zone store before a live Provider adoption or
    /// durable schema advance. The caller must retain or publish the image
    /// through the storage owner's recovery path before applying the effect.
    pub async fn backup_before_live_adoption(&self) -> Result<LogicalBackup, ResourceRuntimeError> {
        self.store
            .logical_backup()
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)
    }

    /// Load and validate the persisted Device record before a security-key
    /// provider constructs its one-use admission. Request fields select a
    /// candidate only; the returned values all originate from the store.
    #[allow(dead_code)]
    #[allow(dead_code)]
    pub(crate) async fn security_key_device_is_admitted(
        &self,
        request: SecurityKeyDeviceAdmissionRequest<'_>,
    ) -> Result<SecurityKeyDeviceAdmission, ResourceRuntimeError> {
        let resource = self
            .backend
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: request.operation_id.to_owned(),
                    idempotency_key: None,
                    correlation_id: request.operation_id.to_owned(),
                    trace_id: None,
                    deadline_ms: 30_000,
                },
                zone: self.zone.clone(),
                target: request.device_ref.clone(),
                expected_uid: Some(request.device_uid.clone()),
                projection: StoreProjection::Full,
            })
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

    /// Record broker evidence for synchronous non-resource broker dispatches.
    pub fn record_broker_evidence(
        &self,
        evidence: DurabilityEvidence,
    ) -> Result<(), ResourceRuntimeError> {
        self.store
            .broker_evidence_index()
            .insert(evidence)
            .map_err(|_| ResourceRuntimeError::StoreOpenFailed)
    }

    /// Publish terminal broker evidence and drain the live store outbox.
    pub async fn ingest_broker_evidence(
        &self,
        operation_id: &str,
        evidence: DurabilityEvidence,
    ) -> Result<(), ResourceRuntimeError> {
        self.store
            .ingest_broker_evidence(operation_id, evidence)
            .await
            .map_err(|_| ResourceRuntimeError::StoreOpenFailed)
    }

    /// Return every pending trusted-deferred activation outbox for this Zone.
    pub(crate) async fn pending_trusted_activation_operation_ids(
        &self,
    ) -> Result<Vec<String>, ResourceRuntimeError> {
        self.store
            .pending_deferred_activation_operation_ids()
            .await
            .map_err(|_| ResourceRuntimeError::StoreOpenFailed)
    }

    /// Refuse publication while any trusted-deferred activation outbox remains.
    pub(crate) async fn require_trusted_activation_outboxes_drained(
        &self,
    ) -> Result<(), ResourceRuntimeError> {
        match self
            .store
            .require_no_pending_deferred_activation_outboxes()
            .await
        {
            Ok(()) => Ok(()),
            Err(error) if error.reason_code() == "audit-deferred-evidence-pending" => {
                Err(ResourceRuntimeError::HandlerNotReady)
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "trusted-deferred activation outbox drain check failed",
                );
                Err(ResourceRuntimeError::StoreOpenFailed)
            }
        }
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

    /// Close the production redb workers before the runtime is discarded.
    pub async fn shutdown(self) -> Result<(), ResourceRuntimeError> {
        let ZoneResourceRuntime {
            store,
            backend,
            api,
            bus,
            registrar,
            ingress,
            service_task,
            authority_persistence,
            authority_recovery,
            process_status_client,
            u6_runner_tasks,
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
        let u6_runner_tasks = u6_runner_tasks
            .into_inner()
            .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
        for task in u6_runner_tasks {
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
        drop(api);
        drop(backend);
        drop(authority_persistence);
        drop(authority_recovery);
        let store = Arc::try_unwrap(store).map_err(|_| ResourceRuntimeError::CoreStartupFailed)?;
        store
            .shutdown()
            .await
            .map_err(|_| ResourceRuntimeError::StoreOpenFailed)
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

async fn interaction_resources_present(
    zone: &ZoneId,
    store: &RedbResourceStore,
) -> Result<bool, ResourceRuntimeError> {
    let provider_type =
        ResourceTypeName::parse("Provider").map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let provider_names = INTERACTION_PROVIDER_REFS
        .iter()
        .map(|provider| {
            provider
                .strip_prefix("Provider/")
                .and_then(|name| ResourceName::parse(name).ok())
                .ok_or(ResourceRuntimeError::HandlerNotReady)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut cursor = None;
    loop {
        let request = StoreListRequest {
                operation: StoreOperationContext {
                    operation_id: "interaction-presence-providers".to_owned(),
                    idempotency_key: None,
                    correlation_id: "interaction-presence-providers".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: zone.clone(),
                resource_types: vec![provider_type.clone()],
                resource_names: provider_names.clone(),
                filters: Vec::new(),
                page_size: 16,
                cursor: cursor.clone(),
                projection: StoreProjection::Full,
            };
        let page = retry_transient_store_list(zone, "interaction-presence-providers", || {
            store.list(request.clone())
        })
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        if !page.resources.is_empty() {
            return Ok(true);
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    let mut cursor = None;
    loop {
        let request = StoreListRequest {
                operation: StoreOperationContext {
                    operation_id: "interaction-presence-resources".to_owned(),
                    idempotency_key: None,
                    correlation_id: "interaction-presence-resources".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: zone.clone(),
                resource_types: Vec::new(),
                resource_names: Vec::new(),
                filters: Vec::new(),
                page_size: 512,
                cursor: cursor.clone(),
                projection: StoreProjection::Full,
            };
        let page = retry_transient_store_list(zone, "interaction-presence-resources", || {
            store.list(request.clone())
        })
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        for resource in page.resources {
            if is_u9_resource_type(&resource.resource_ref.resource_type()) {
                return Ok(true);
            }
            let value: Value = serde_json::from_slice(&resource.canonical_json)
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
            if contains_u9_provider_ref(&value) {
                return Ok(true);
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
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
    zone: &ZoneId,
    store: &RedbResourceStore,
    current_revision: ZoneRevision,
) -> Result<Option<CommittedInteractionProviderConfiguration>, ResourceRuntimeError> {
    let provider_type =
        ResourceTypeName::parse("Provider").map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let operation = StoreOperationContext {
        operation_id: "interaction-provider-config".to_owned(),
        idempotency_key: None,
        correlation_id: "interaction-provider-config".to_owned(),
        trace_id: None,
        deadline_ms: 10_000,
    };
    let clipboard_ref = ResourceRef::parse("Provider/clipboard-wayland")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let notification_ref = ResourceRef::parse("Provider/notification-desktop")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let request = StoreListRequest {
            operation,
            zone: zone.clone(),
            resource_types: vec![provider_type],
            resource_names: vec![
                ResourceName::parse("clipboard-wayland")
                    .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?,
                ResourceName::parse("notification-desktop")
                    .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?,
            ],
            filters: Vec::new(),
            page_size: 2,
            cursor: None,
            projection: StoreProjection::Full,
        };
    let page = retry_transient_store_list(zone, "interaction-provider-config", || {
        store.list(request.clone())
    })
    .await
        .map_err(|error| {
            tracing::error!(
                zone = %zone.as_str(),
                operation = "interaction-provider-config",
                store_error_kind = ?error.kind(),
                reason_code = error.reason_code(),
                "interaction Provider configuration list failed",
            );
            ResourceRuntimeError::StoreReadFailed
        })?;
    if page.next_cursor.is_some() {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let mut clipboard = None;
    let mut notification = None;
    for resource in page.resources {
        if resource.resource_ref == clipboard_ref {
            if clipboard.is_some() {
                return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
            }
            clipboard = Some(parse_committed_clipboard_configuration(
                zone,
                current_revision,
                &resource,
            )?);
        } else if resource.resource_ref == notification_ref {
            if notification.is_some() {
                return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
            }
            notification = Some(parse_committed_notification_configuration(
                zone,
                current_revision,
                &resource,
            )?);
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
    zone: &ZoneId,
    store: &RedbResourceStore,
    current_revision: ZoneRevision,
    configuration: Option<&CommittedInteractionProviderConfiguration>,
) -> Result<Option<CommittedInteractionIdentity>, ResourceRuntimeError> {
    let session_resource_type = ResourceTypeName::parse("display-wayland.d2bus.org.WaylandSession")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let operation = StoreOperationContext {
        operation_id: "interaction-wayland-session".to_owned(),
        idempotency_key: None,
        correlation_id: "interaction-wayland-session".to_owned(),
        trace_id: None,
        deadline_ms: 10_000,
    };
    let request = StoreListRequest {
            operation,
            zone: zone.clone(),
            resource_types: vec![session_resource_type],
            resource_names: Vec::new(),
            filters: Vec::new(),
            page_size: 2,
            cursor: None,
            projection: StoreProjection::Full,
        };
    let page = retry_transient_store_list(zone, "interaction-wayland-session", || {
        store.list(request.clone())
    })
    .await
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    if page.next_cursor.is_some() {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    if page.resources.is_empty() {
        return if configuration.is_none() {
            Ok(None)
        } else {
            Err(ResourceRuntimeError::InteractionConfigurationUnavailable)
        };
    }
    if page.resources.len() != 1 {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let session_resource = page
        .resources
        .into_iter()
        .next()
        .ok_or(ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let session_spec = committed_wayland_session_spec(zone, current_revision, &session_resource)
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
    let _policy_resource =
        committed_resource(zone, store, current_revision, session_spec.policy_ref())
            .await
            .inspect_err(|error| {
                tracing::error!(
                    zone = %zone.as_str(),
                    operation = "interaction-policy-lookup",
                    error = %error,
                    "resource runtime committed Wayland policy lookup failed",
                );
            })?;
    let subject_uid = committed_resource_uid(zone, store, current_revision, &subject_ref)
        .await
        .inspect_err(|error| {
            tracing::error!(
                zone = %zone.as_str(),
                operation = "interaction-subject-lookup",
                error = %error,
                "resource runtime committed interaction subject lookup failed",
            );
        })?;
    let _host_uid = committed_resource_uid(zone, store, current_revision, &host_execution_ref)
        .await
        .inspect_err(|error| {
            tracing::error!(
                zone = %zone.as_str(),
                operation = "interaction-host-lookup",
                error = %error,
                "resource runtime committed interaction Host lookup failed",
            );
        })?;
    let _user_uid = committed_resource_uid(zone, store, current_revision, &user_ref)
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
    let display_resource = committed_resource(zone, store, current_revision, &display_ref)
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
        committed_provider_spec(zone, current_revision, &display_resource, &display_ref)
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
                let uid = committed_resource_uid(zone, store, current_revision, guest_ref).await?;
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
                let uid = committed_resource_uid(zone, store, current_revision, guest_ref).await?;
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
    current_revision: ZoneRevision,
    resource: &StoredResource,
) -> Result<WaylandSessionSpec, ResourceRuntimeError> {
    let expected_type = ResourceTypeName::parse("display-wayland.d2bus.org.WaylandSession")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    if &resource.zone != zone
        || resource.resource_ref.resource_type() != &expected_type
        || resource.generation.get() == 0
        || resource.revision.get() == 0
        || resource.revision > current_revision
    {
        tracing::error!(
            zone = %zone.as_str(),
            resource_zone = %resource.zone.as_str(),
            resource_ref = %resource.resource_ref.to_canonical_string(),
            generation = resource.generation.get(),
            revision = resource.revision.get(),
            current_revision = current_revision.get(),
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
    zone: &ZoneId,
    store: &RedbResourceStore,
    current_revision: ZoneRevision,
    resource_ref: &ResourceRef,
) -> Result<ResourceUid, ResourceRuntimeError> {
    let resource = committed_resource(zone, store, current_revision, resource_ref).await?;
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
    zone: &ZoneId,
    store: &RedbResourceStore,
    current_revision: ZoneRevision,
    resource_ref: &ResourceRef,
) -> Result<StoredResource, ResourceRuntimeError> {
    if !is_supported_committed_resource_ref(resource_ref) {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let operation_id = format!(
        "interaction-identity:{}",
        resource_ref.to_canonical_string()
    );
    let request = StoreGetRequest {
            operation: StoreOperationContext {
                operation_id: operation_id.clone(),
                idempotency_key: None,
                correlation_id: operation_id.clone(),
                trace_id: None,
                deadline_ms: 10_000,
            },
            zone: zone.clone(),
            target: resource_ref.clone(),
            expected_uid: None,
            projection: StoreProjection::Full,
        };
    let resource = retry_transient_store_read(zone, &operation_id, || {
        store.get(request.clone())
    })
    .await
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    validate_committed_resource(zone, current_revision, resource_ref, resource)
}

async fn current_committed_resource(
    zone: &ZoneId,
    store: &RedbResourceStore,
    resource_ref: &ResourceRef,
    operation_id: &str,
) -> Result<(StoredResource, ZoneRevision), ResourceRuntimeError> {
    if !is_supported_committed_resource_ref(resource_ref) {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let request = StoreListRequest {
            operation: StoreOperationContext {
                operation_id: operation_id.to_owned(),
                idempotency_key: None,
                correlation_id: operation_id.to_owned(),
                trace_id: None,
                deadline_ms: 10_000,
            },
            zone: zone.clone(),
            resource_types: vec![resource_ref.resource_type().clone()],
            resource_names: vec![resource_ref.name().clone()],
            filters: Vec::new(),
            page_size: 1,
            cursor: None,
            projection: StoreProjection::Full,
        };
    let snapshot = retry_transient_store_list(zone, operation_id, || {
        store.list(request.clone())
    })
    .await
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    if snapshot.truncated || snapshot.next_cursor.is_some() || snapshot.resources.len() != 1 {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let snapshot_revision = snapshot.snapshot_revision;
    let resource = snapshot
        .resources
        .into_iter()
        .next()
        .ok_or(ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let resource = validate_committed_resource(zone, snapshot_revision, resource_ref, resource)?;
    Ok((resource, snapshot_revision))
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
    current_revision: ZoneRevision,
    resource_ref: &ResourceRef,
    resource: StoredResource,
) -> Result<StoredResource, ResourceRuntimeError> {
    if resource.zone != *zone
        || resource.resource_ref != *resource_ref
        || resource.generation.get() == 0
        || resource.revision.get() == 0
        || resource.revision > current_revision
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

async fn load_committed_controller_provider_identities(
    zone: &ZoneId,
    store: &RedbResourceStore,
    provider_refs: BTreeSet<ResourceRef>,
) -> Result<BTreeMap<ResourceRef, (ResourceUid, ResourceGeneration)>, ResourceRuntimeError> {
    if provider_refs.is_empty() {
        return Ok(BTreeMap::new());
    }
    let provider_type = ResourceTypeName::parse("Provider")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let resource_names = provider_refs
        .iter()
        .map(|provider_ref| {
            if provider_ref.resource_type() != &provider_type {
                return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
            }
            Ok(provider_ref.name().clone())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let page_size = u32::try_from(provider_refs.len())
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let request = StoreListRequest {
            operation: StoreOperationContext {
                operation_id: "controller-provider-identity-snapshot".to_owned(),
                idempotency_key: None,
                correlation_id: "controller-provider-identity-snapshot".to_owned(),
                trace_id: None,
                deadline_ms: 10_000,
            },
            zone: zone.clone(),
            resource_types: vec![provider_type],
            resource_names,
            filters: Vec::new(),
            page_size,
            cursor: None,
            projection: StoreProjection::Full,
        };
    let snapshot = retry_transient_store_list(
        zone,
        "controller-provider-identity-snapshot",
        || store.list(request.clone()),
    )
    .await
        .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
    if snapshot.truncated || snapshot.next_cursor.is_some() {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }

    let mut identities = BTreeMap::new();
    for resource in snapshot.resources {
        let provider_ref = resource.resource_ref.clone();
        if !provider_refs.contains(&provider_ref) || identities.contains_key(&provider_ref) {
            return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
        }
        let (_, uid, generation, _, _) =
            committed_provider_spec(zone, snapshot.snapshot_revision, &resource, &provider_ref)?;
        identities.insert(provider_ref, (uid, generation));
    }
    if identities.len() != provider_refs.len() {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    Ok(identities)
}

async fn load_controller_policy_subjects(
    zone: &ZoneId,
    store: &RedbResourceStore,
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
    let identities = load_committed_controller_provider_identities(
        zone,
        store,
        provider_refs,
    )
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

impl ZoneResourceRuntime {
    /// The committed `Provider` identities the v3 plane publishes into its
    /// registry (KTD7). Unconverted `Provider` rows live only in this plane's
    /// redb store, so the new plane resolves them here - the same durable
    /// rows the controller policy and session fences compare against.
    pub(crate) async fn committed_provider_identities(
        &self,
        provider_refs: BTreeSet<ResourceRef>,
    ) -> Result<BTreeMap<ResourceRef, (ResourceUid, ResourceGeneration)>, ResourceRuntimeError> {
        load_committed_controller_provider_identities(&self.zone, &self.store, provider_refs).await
    }
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

fn validate_assignment_list_page(
    page: &StoreListResult,
    zone: &ZoneId,
    provider_ref: &ResourceRef,
    expected_snapshot: Option<ZoneRevision>,
) -> Result<Vec<ResourceEnvelope>, ControllerAssignmentRefreshError> {
    if expected_snapshot.is_some_and(|snapshot| snapshot != page.snapshot_revision) {
        return Err(ControllerAssignmentRefreshError::Retryable);
    }
    let mut resources = Vec::new();
    for stored in &page.resources {
        if &stored.zone != zone
            || stored.revision.get() == 0
            || stored.revision > page.snapshot_revision
        {
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
                ControllerAssignmentRefreshError::Failed(
                    ResourceRuntimeError::AuthorizationUnavailable,
                )
            })? != stored.payload_digest
        {
            return Err(ControllerAssignmentRefreshError::Failed(
                ResourceRuntimeError::AuthorizationUnavailable,
            ));
        }
        if envelope.spec().provider_ref() == Some(provider_ref) {
            resources.push(envelope);
        }
    }
    Ok(resources)
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

/// Overlay the manager's live status onto one durable `User` row.
///
/// A converted `User` row carries its status only in the manager (R11). The
/// bundle-publication path still materializes every bundle row into the
/// durable store with the create payload's `Pending` status, and nothing ever
/// updates it, so identity resolution - which reads the durable envelope -
/// must be handed the manager's live status for the row it is about to judge.
///
/// Identity is deliberately left alone: the durable row owns uid, generation
/// and revision (KTD2/KTD8), and the compiled authorization policy's subjects
/// are compiled from them. Only the phase and `observedGeneration` are
/// replaced, and the stamped `observedGeneration` is the durable row's own
/// generation: the resolved-user check compares the status against that field,
/// while the manager's generation numbers a different view of the same row and
/// is never substituted for it.
fn overlay_manager_row_status(row: &mut StoredResource, view: &ResourceView) {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&row.canonical_json) else {
        return;
    };
    let Some(root) = value.as_object_mut() else {
        return;
    };
    let phase = match view.observed_status() {
        Some(d2b_resource_runtime::resource::ResourceStatus::Ready) => "Ready",
        Some(d2b_resource_runtime::resource::ResourceStatus::Failed(_)) => "Failed",
        // `ResourcePhase`'s closed vocabulary has no `Deleting`; a row whose
        // deletion is requested reads as `Deleted` here, exactly as the
        // shared-provider and interaction projections read it, so the
        // overlaid envelope still decodes under the strict contract (the
        // deletion mark itself stays in `metadata.deletionRequestedAt`).
        Some(d2b_resource_runtime::resource::ResourceStatus::Deleting) => "Deleted",
        _ => "Pending",
    };
    let status = root
        .entry("status".to_owned())
        .or_insert_with(|| serde_json::json!({}));
    if let Some(status) = status.as_object_mut() {
        status.insert(
            "observedGeneration".to_owned(),
            serde_json::json!(row.generation.get()),
        );
        status.insert("phase".to_owned(), serde_json::json!(phase));
    }
    let Ok(bytes) = serde_json::to_vec(&value) else {
        return;
    };
    let Ok(canonical) = d2b_contracts_resource::v3::CanonicalJsonValue::parse(&bytes) else {
        return;
    };
    row.canonical_json = canonical.to_canonical_bytes();
    // The row's identity and payload are validated against its digest by
    // every reader that fences on the stored row (the policy loader refuses a
    // row whose digest does not match), so the digest follows the edit.
    row.payload_digest = d2b_contracts_resource::v3::resource_schema::canonical_digest(
        d2b_contracts_resource::v3::resource_schema::RESOURCE_ENVELOPE_DOMAIN_TAG,
        &row.canonical_json,
    );
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
    current_revision: ZoneRevision,
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
        || resource.revision > current_revision
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
    current_revision: ZoneRevision,
    resource: &StoredResource,
) -> Result<CommittedClipboardProviderConfiguration, ResourceRuntimeError> {
    let expected_ref = ResourceRef::parse("Provider/clipboard-wayland")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let (spec, resource_uid, resource_generation, resource_revision, provenance_digest) =
        committed_provider_spec(zone, current_revision, resource, &expected_ref)?;
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
    current_revision: ZoneRevision,
    resource: &StoredResource,
) -> Result<CommittedNotificationProviderConfiguration, ResourceRuntimeError> {
    let expected_ref = ResourceRef::parse("Provider/notification-desktop")
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
    let (spec, resource_uid, resource_generation, resource_revision, provenance_digest) =
        committed_provider_spec(zone, current_revision, resource, &expected_ref)?;
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
    zone: &ZoneId,
    store: &RedbResourceStore,
) -> Result<SystemCoreReconcileResult, ResourceRuntimeError> {
    let mut resources = Vec::new();
    let mut cursor = None;
    loop {
        let request = StoreListRequest {
                operation: StoreOperationContext {
                    operation_id: "system-core-startup-summary".to_owned(),
                    idempotency_key: None,
                    correlation_id: "system-core-startup-summary".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: zone.clone(),
                resource_types: Vec::new(),
                resource_names: Vec::new(),
                filters: Vec::new(),
                page_size: 128,
                cursor: cursor.clone(),
                projection: StoreProjection::Full,
            };
        let page = retry_transient_store_list(zone, "system-core-startup-summary", || {
            store.list(request.clone())
        })
        .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        resources.extend(page.resources);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    let total_resource_count = resources.len().min(u32::MAX as usize) as u32;
    let active_configuration_generation = retry_transient_store_read(
        zone,
        "system-core-startup-metadata",
        || store.runtime_metadata(),
    )
    .await
        .map_err(|_| ResourceRuntimeError::StoreReadFailed)?
        .policy_snapshot
        .active_configuration_revision
        .get();
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

fn map_process_runtime_error(error: ProcessResourceRuntimeError) -> ResourceRuntimeError {
    match error {
        ProcessResourceRuntimeError::Store => ResourceRuntimeError::StoreReadFailed,
        ProcessResourceRuntimeError::UnsupportedProvider
        | ProcessResourceRuntimeError::TemplateUnavailable
        | ProcessResourceRuntimeError::IdentityAmbiguous
        | ProcessResourceRuntimeError::ProviderEffect
        | ProcessResourceRuntimeError::ControllerBootstrapUnavailable
        | ProcessResourceRuntimeError::ProviderIdentityUnavailable
        | ProcessResourceRuntimeError::OwnerIdentityUnavailable
        | ProcessResourceRuntimeError::InvalidResource => {
            ResourceRuntimeError::CapabilityUnavailable
        }
    }
}

fn assignment_fence_store_error(error: &StoreError, fallback_revision: ZoneRevision) -> SourceError {
    match error.kind() {
        StoreErrorKind::Backpressure | StoreErrorKind::StoreBackpressure => {
            SourceError::Backpressure
        }
        StoreErrorKind::Timeout => SourceError::Timeout,
        StoreErrorKind::ResourceConflict => {
            SourceError::Conflict(error.current_revision().unwrap_or(fallback_revision))
        }
        _ => SourceError::Unavailable,
    }
}

pub(super) fn shared_provider_assignment_fence_resolver(
    store: Arc<RedbResourceStore>,
    allowed_types: BTreeSet<ResourceTypeName>,
    authority: Arc<CoreAssignmentAuthority>,
) -> AssignmentFenceResolver {
    Arc::new(move |target, uid, revision| {
        let store = Arc::clone(&store);
        let authority = Arc::clone(&authority);
        let allowed_types = allowed_types.clone();
        Box::pin(async move {
            if !allowed_types.contains(target.resource_type()) {
                tracing::warn!(
                    target = %target.to_canonical_string(),
                    allowed = ?allowed_types,
                    "assignment fence rejected non-owned resource type",
                );
                return Err(SourceError::Integrity);
            }
            if let Some(stored) = store
                .assignment_fence(store.identity().zone().clone(), target.clone())
                .await
                .map_err(|error| match error.kind() {
                    StoreErrorKind::Backpressure | StoreErrorKind::StoreBackpressure => {
                        SourceError::Backpressure
                    }
                    StoreErrorKind::Timeout => SourceError::Timeout,
                    _ => SourceError::Unavailable,
                })?
            {
                if assignment_fence_conflict(&stored, &uid, &authority) {
                    return Err(SourceError::Integrity);
                }
            }
            Ok(ResourceAssignmentFence {
                resource_uid: uid,
                resource_revision: revision,
                provider_generation: authority.provider_generation,
                controller_generation: authority.controller_generation,
                controller_role: authority.controller_role.clone(),
                target: authority.target.clone(),
                session_generation: authority.session_generation,
                epoch: ASSIGNMENT_EPOCH,
                scope: ResourceAssignmentScope::Primary,
            })
        })
    })
}

fn process_assignment_fence_resolver(
    store: Arc<RedbResourceStore>,
    mode: DaemonMode,
    authority: Arc<CoreAssignmentAuthority>,
) -> AssignmentFenceResolver {
    fn integrity(target: &ResourceRef, reason: &'static str) -> SourceError {
        tracing::warn!(
            resource = %target.to_canonical_string(),
            reason,
            "Process assignment source rejected resource identity",
        );
        SourceError::Integrity
    }

    Arc::new(move |target, uid, revision| {
        let store = Arc::clone(&store);
        let authority = Arc::clone(&authority);
        Box::pin(async move {
            let resource = store
                .get(StoreGetRequest {
                    operation: StoreOperationContext {
                        operation_id: "process-assignment-fence".to_owned(),
                        idempotency_key: None,
                        correlation_id: "process-assignment-fence".to_owned(),
                        trace_id: None,
                        deadline_ms: 10_000,
                    },
                    zone: store.identity().zone().clone(),
                    target: target.clone(),
                    expected_uid: Some(uid.clone()),
                    projection: StoreProjection::Full,
                })
                .await
                .map_err(|error| assignment_fence_store_error(&error, revision))?;
            if resource.revision != revision {
                return Err(SourceError::Conflict(resource.revision));
            }
            let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
                .map_err(|_| integrity(&target, "resource-envelope-invalid"))?;
            let provider_ref = envelope
                .spec()
                .provider_ref()
                .cloned()
                .ok_or_else(|| integrity(&target, "process-provider-ref-missing"))?;
            if !matches!(
                provider_ref.name().as_str(),
                "system-minijail" | "system-systemd"
            ) {
                return Err(integrity(&target, "process-provider-ref-unsupported"));
            }
            let execution_ref = envelope
                .spec()
                .base()
                .get("executionRef")
                .and_then(|value| match value {
                    CanonicalJsonValue::String(value) => ResourceRef::parse(value).ok(),
                    _ => None,
                })
                .ok_or_else(|| integrity(&target, "process-execution-ref-missing"))?;
            let expected_target = match mode {
                DaemonMode::Host => "Host",
                DaemonMode::Guest => "Guest",
            };
            if execution_ref.resource_type().as_str() != expected_target {
                return Err(integrity(&target, "process-execution-ref-wrong-domain"));
            }
            let _target_provider = match store
                .get(StoreGetRequest {
                    operation: StoreOperationContext {
                        operation_id: "process-assignment-provider".to_owned(),
                        idempotency_key: None,
                        correlation_id: "process-assignment-provider".to_owned(),
                        trace_id: None,
                        deadline_ms: 10_000,
                    },
                    zone: store.identity().zone().clone(),
                    target: provider_ref.clone(),
                    expected_uid: None,
                    projection: StoreProjection::MetadataOnly,
                })
                .await
            {
                Ok(provider) if provider.generation.get() > 0 => provider,
                Ok(_) => {
                    return Err(integrity(&target, "target-provider-generation-invalid"));
                }
                Err(error) if error.kind() == StoreErrorKind::ResourceNotFound => {
                    let current_revision = store
                        .runtime_metadata()
                        .await
                        .map_err(|error| assignment_fence_store_error(&error, revision))?
                        .current_revision;
                    return Err(SourceError::Conflict(current_revision));
                }
                Err(error) if error.kind() == StoreErrorKind::ResourceConflict => {
                    return Err(SourceError::Conflict(
                        error.current_revision().unwrap_or(revision),
                    ));
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        StoreErrorKind::Backpressure | StoreErrorKind::StoreBackpressure
                    ) =>
                {
                    return Err(SourceError::Backpressure);
                }
                Err(error) if error.kind() == StoreErrorKind::Timeout => {
                    return Err(SourceError::Timeout);
                }
                Err(_) => return Err(SourceError::Integrity),
            };
            Ok(ResourceAssignmentFence {
                resource_uid: uid,
                resource_revision: revision,
                provider_generation: authority.provider_generation,
                controller_generation: authority.controller_generation,
                controller_role: authority.controller_role.clone(),
                target: execution_ref,
                session_generation: authority.session_generation,
                epoch: ASSIGNMENT_EPOCH,
                scope: ResourceAssignmentScope::Primary,
            })
        })
    })
}

/// Which plane serves one public resource request.
///
/// Converted types (the Phase A set) are served by the manager-backed
/// service, everything else by the redb service, and a request that spans
/// both planes is refused rather than split across two sealed stores. The
/// plane is chosen from the reference the handler will actually act on -
/// `resourceRef` for every targeted method, `resourceType` for Create, the
/// parsed type set for List - never from a declared type field the handler
/// ignores. A declared type that disagrees with the target is refused
/// outright: routing on one reference and mutating another would move a
/// converted type onto the legacy plane.
fn public_request_route(
    request: &Value,
    method: &str,
) -> Result<crate::resource_plane_v3::PlaneRoute, ResourceRuntimeError> {
    let types: Vec<String> = if method == "List" {
        parse_list_request(request)?
            .resource_types
            .iter()
            .map(|resource_type| resource_type.as_str().to_owned())
            .collect()
    } else if method == "Create" {
        vec![declared_resource_type(request)
            .ok_or(ResourceRuntimeError::RequestInvalid)?
            .to_owned()]
    } else {
        let target = public_target_ref(request)?;
        if let Some(declared) = declared_resource_type(request)
            && declared != target.resource_type().as_str()
        {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        vec![target.resource_type().as_str().to_owned()]
    };
    let mut new_plane = false;
    let mut old_plane = false;
    for resource_type in &types {
        match crate::resource_plane_v3::route_resource_type(resource_type) {
            crate::resource_plane_v3::PlaneRoute::NewPlane => new_plane = true,
            crate::resource_plane_v3::PlaneRoute::OldPlane => old_plane = true,
        }
    }
    match (new_plane, old_plane) {
        (true, false) => Ok(crate::resource_plane_v3::PlaneRoute::NewPlane),
        (false, _) => Ok(crate::resource_plane_v3::PlaneRoute::OldPlane),
        (true, true) => Err(ResourceRuntimeError::CapabilityUnavailable),
    }
}

/// The resource type a request declares, if any. Only Create routes on this
/// field; every other method derives its target from `resourceRef`.
fn declared_resource_type(request: &Value) -> Option<&str> {
    request
        .get("resourceType")
        .or_else(|| request.get("type"))
        .and_then(Value::as_str)
}

/// The manager plane's store-seal identity: a deliberately distinct slot from
/// the redb Zone store's, so one authorizer never pairs with both planes. The
/// uid is inert - it only pairs this Zone's manager-plane issuer with the
/// manager backend's acceptor - and the Zone authority's uid is reused when
/// the runtime has one.
fn manager_plane_seal_identity(
    zone: &ZoneId,
    zone_uid: Option<ResourceUid>,
) -> Result<d2b_resource_store::StoreSealIdentity, ResourceRuntimeError> {
    const MANAGER_PLANE_SEAL_SLOT: u32 = 1;
    const MANAGER_PLANE_SEAL_UID: &str = "00000000-0000-4000-8000-000000000001";
    let slot = d2b_resource_store::StoreSlot::new(MANAGER_PLANE_SEAL_SLOT)
        .map_err(|_| ResourceRuntimeError::StoreSealUnavailable)?;
    let uid = match zone_uid {
        Some(uid) => uid,
        None => ResourceUid::parse(MANAGER_PLANE_SEAL_UID.to_owned())
            .map_err(|_| ResourceRuntimeError::StoreSealUnavailable)?,
    };
    Ok(d2b_resource_store::StoreSealIdentity::new(
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
    let metadata = runtime
        .store
        .runtime_metadata()
        .await
        .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
    let timestamp = current_status_timestamp();
    let value = json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": resource_type.to_canonical_string(),
        "metadata": {
            "configurationGeneration": metadata.policy_snapshot.active_configuration_revision.get(),
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

    /// Record one terminal broker result in every Zone's shared live index.
    pub fn record_broker_evidence(
        &self,
        evidence: DurabilityEvidence,
    ) -> Result<(), ResourceRuntimeError> {
        for runtime in self.zones.values() {
            runtime.record_broker_evidence(evidence.clone())?;
        }
        Ok(())
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
    use super::*;
    use std::{
        collections::VecDeque,
        fs::OpenOptions,
        os::fd::{AsRawFd, OwnedFd},
        sync::Arc,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use d2b_contracts_resource::v3::{
        CanonicalJsonObject, Timestamp,
        storage::{ZoneStoreIdentity, ZoneStoreStorageRow},
    };
    use d2b_contracts_zone_session::v3::component_session::LimitProfile;
    use d2b_contracts_zone_session::v3::resource_bundle::{BundleResource, BundleResourceMetadata};
    use d2b_core::{
        bundle::{Bundle, BundleGeneration},
        bundle_resolver::BundleResolver,
        manifest_v04::ManifestV04,
        processes::ProcessesJson,
    };
    use d2b_resource_store::mutation_seal::mutation_seal_pair;
    use d2b_resource_store_redb::write_provisioning_marker;
    use d2b_session_unix::{CreditPool, CreditScopeSet, OutboundPacket, prearmed_seqpacket_pair};

    fn test_authority(
        provider_generation: u64,
        controller_generation: u64,
        session_generation: u64,
    ) -> CoreAssignmentAuthority {
        CoreAssignmentAuthority {
            provider_generation: ResourceGeneration::new(provider_generation).unwrap(),
            controller_generation: ControllerGeneration::new(controller_generation).unwrap(),
            session_generation: ReconnectGeneration::new(session_generation).unwrap(),
            controller_role: ResourceRef::parse("Process/d2b-core-controller").unwrap(),
            target: ResourceRef::parse("Zone/work").unwrap(),
        }
    }

    fn test_fence(
        provider_generation: u64,
        controller_generation: u64,
        session_generation: u64,
    ) -> ResourceAssignmentFence {
        ResourceAssignmentFence {
            resource_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            resource_revision: ZoneRevision::new(1),
            provider_generation: ResourceGeneration::new(provider_generation).unwrap(),
            controller_generation: ControllerGeneration::new(controller_generation).unwrap(),
            controller_role: ResourceRef::parse("Process/d2b-core-controller").unwrap(),
            target: ResourceRef::parse("Zone/work").unwrap(),
            session_generation: ReconnectGeneration::new(session_generation).unwrap(),
            epoch: ASSIGNMENT_EPOCH,
            scope: ResourceAssignmentScope::Primary,
        }
    }

    #[test]
    fn fence_conflict_adopts_pure_predecessors_and_rejects_the_rest() {
        let uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let authority = test_authority(3, 5, 7);
        // Identical: no conflict.
        assert!(!assignment_fence_conflict(&test_fence(3, 5, 7), &uid, &authority));
        // Pure predecessor on every axis: adopt (reconnect reconciliation).
        assert!(!assignment_fence_conflict(&test_fence(2, 4, 6), &uid, &authority));
        assert!(!assignment_fence_conflict(&test_fence(3, 5, 6), &uid, &authority));
        // Role/target drift on a strictly older fence is adopted by the successor.
        let mut drifted = test_fence(2, 4, 6);
        drifted.controller_role = ResourceRef::parse("Process/other").unwrap();
        assert!(!assignment_fence_conflict(&drifted, &uid, &authority));
        // Strictly newer on any authority axis: conflict.
        assert!(assignment_fence_conflict(&test_fence(4, 5, 7), &uid, &authority));
        assert!(assignment_fence_conflict(&test_fence(3, 6, 7), &uid, &authority));
        assert!(assignment_fence_conflict(&test_fence(3, 5, 8), &uid, &authority));
        // Mixed drift is not pure staleness: conflict.
        assert!(assignment_fence_conflict(&test_fence(2, 6, 6), &uid, &authority));
        // Role drift at fully equal axes: conflict.
        let mut foreign = test_fence(3, 5, 7);
        foreign.controller_role = ResourceRef::parse("Process/other").unwrap();
        assert!(assignment_fence_conflict(&foreign, &uid, &authority));
        // Foreign uid always conflicts.
        let other_uid =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
        assert!(assignment_fence_conflict(&test_fence(3, 5, 7), &other_uid, &authority));
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
    fn guest_dependency_reenters_only_after_provider_status_is_ready() {
        let guest = serde_json::json!({
            "spec": { "providerRef": "Provider/runtime" }
        });
        let provider_ref = ResourceRef::parse("Provider/runtime").unwrap();
        let provider_uid =
            ResourceUid::parse("11111111-1111-4111-8111-111111111111").unwrap();
        let pending = DependencySnapshot::new(ResourceSnapshot::new(
            ResourceKey::new(
                ZoneId::parse("work").unwrap(),
                provider_ref.clone(),
                provider_uid.clone(),
            ),
            ZoneRevision::new(4),
            ResourceGeneration::new(2).unwrap(),
            serde_json::to_vec(&serde_json::json!({
                "status": { "phase": "Pending" }
            }))
            .unwrap(),
            false,
        ));
        assert!(!DaemonSharedProviderEffects::related_guest_dependency(
            &guest, &pending
        )
        .unwrap());
        let ready = DependencySnapshot::new(ResourceSnapshot::new(
            ResourceKey::new(
                ZoneId::parse("work").unwrap(),
                provider_ref,
                provider_uid,
            ),
            ZoneRevision::new(5),
            ResourceGeneration::new(2).unwrap(),
            serde_json::to_vec(&serde_json::json!({
                "status": {
                    "phase": "Ready",
                    "observedGeneration": 2
                }
            }))
            .unwrap(),
            false,
        ));
        assert!(DaemonSharedProviderEffects::related_guest_dependency(
            &guest, &ready
        )
        .unwrap());
    }

    fn shared_provider_test_descriptor_for(
        registration: SharedProviderRunnerRegistration,
    ) -> (
        SharedProviderRunnerRegistration,
        ControllerDescriptor,
    ) {
        let provider_ref = ResourceRef::parse(registration.provider_ref).unwrap();
        let generations = BTreeMap::from([(provider_ref, ResourceGeneration::new(7).unwrap())]);
        compose_shared_provider_runner_descriptors(
            [registration],
            ZoneId::parse("work").unwrap(),
            ControllerGeneration::new(3).unwrap(),
            &generations,
            ReconnectGeneration::new(5).unwrap(),
        )
        .unwrap()
        .pop()
        .unwrap()
    }

    fn shared_provider_test_resource_for(
        registration: SharedProviderRunnerRegistration,
        finalizers: &[&str],
        deleting: bool,
    ) -> ResourceSnapshot {
        let zone = ZoneId::parse("work").unwrap();
        let resource_ref =
            ResourceRef::parse(&format!("{}/work", registration.resource_type)).unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let finalizers = finalizers
            .iter()
            .map(|value| Value::String((*value).to_owned()))
            .collect::<Vec<_>>();
        let spec = if registration.resource_type.starts_with("display-wayland.") {
            json!({})
        } else {
            json!({"providerRef": registration.provider_ref})
        };
        let body = json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": registration.resource_type,
            "metadata": {
                "name": "work",
                "zone": "work",
                "uid": uid.as_str(),
                "generation": 1,
                "revision": 1,
                "finalizers": finalizers,
            },
            "spec": spec,
            "status": {
                "phase": "Pending",
                "observedGeneration": 0,
            },
        });
        ResourceSnapshot::new(
            ResourceKey::new(zone, resource_ref, uid),
            ZoneRevision::new(1),
            ResourceGeneration::new(1).unwrap(),
            serde_json::to_vec(&body).unwrap(),
            deleting,
        )
    }

    #[test]
    fn production_provider_composition_closes_exactly_27_typed_rows() {
        const EXPECTED_PROVIDER_IDS: [&str; 27] = [
            "system-core",
            "system-systemd",
            "system-minijail",
            "runtime-cloud-hypervisor",
            "runtime-qemu-media",
            "runtime-azure-container-apps",
            "runtime-azure-virtual-machine",
            "volume-local",
            "volume-virtiofs",
            "network-local",
            "device-tpm",
            "device-usbip",
            "device-security-key",
            "device-gpu",
            "display-wayland",
            "audio-pipewire",
            "clipboard-wayland",
            "notification-desktop",
            "shell-terminal",
            "credential-secret-service",
            "credential-entra",
            "credential-managed-identity",
            "transport-unix",
            "transport-vsock",
            "transport-azure-relay",
            "observability-otel",
            "activation-nixos",
        ];
        let expected = EXPECTED_PROVIDER_IDS.into_iter().collect::<BTreeSet<_>>();
        assert_eq!(expected.len(), 27);

        let mut resource_owners = BTreeSet::from(["system-systemd", "system-minijail"]);
        let mut registration_keys = BTreeSet::<(String, String, String)>::new();
        let mut check_shared_registration = |registration: SharedProviderRunnerRegistration| {
            assert!(
                registration_keys.insert((
                    registration.provider_ref.to_owned(),
                    registration.resource_type.to_owned(),
                    registration.controller_ref.to_owned(),
                )),
                "duplicate Provider/resource/controller registration"
            );
            assert!(expected.contains(
                registration
                    .provider_ref
                    .strip_prefix("Provider/")
                    .expect("Provider registration reference")
            ));
            assert!(registration.watched_configuration_is_dependency);
            assert!((30_000..=300_000).contains(&registration.repair_interval_ticks));
            resource_owners.insert(
                registration
                    .provider_ref
                    .strip_prefix("Provider/")
                    .expect("Provider registration reference"),
            );
            let (_, descriptor) = shared_provider_test_descriptor_for(registration);
            assert_eq!(
                descriptor.identity().provider_ref().to_canonical_string(),
                registration.provider_ref
            );
            assert_eq!(
                descriptor.identity().controller_ref().to_canonical_string(),
                registration.controller_ref
            );
            assert_eq!(
                descriptor.resource_types().next().expect("one resource type").as_str(),
                registration.resource_type
            );
            assert_eq!(
                descriptor.execution().resync().observe_interval_ticks(),
                Some(registration.repair_interval_ticks)
            );
            let provider_selector = descriptor
                .watch_selectors()
                .iter()
                .find(|selector| selector.field() == SelectorField::Spec)
                .and_then(|selector| selector.exact_value());
            if registration.resource_type.starts_with("display-wayland.") {
                assert!(provider_selector.is_none());
            } else {
                assert_eq!(provider_selector, Some(registration.provider_ref));
            }
            if registration.finalizer.is_empty() {
                assert!(descriptor.finalizers().is_empty());
            } else {
                assert_eq!(descriptor.finalizers(), &[registration.finalizer.to_owned()]);
            }
            for finalizer in descriptor
                .finalizers()
                .iter()
                .filter(|finalizer| !finalizer.is_empty())
            {
                let parsed = d2b_contracts_resource::v3::FinalizerId::parse(finalizer.clone())
                    .expect("production finalizer must satisfy the Resource API contract");
                assert_eq!(parsed.as_str(), finalizer);
            }
            match registration.provider_ref {
                "Provider/volume-local" => {
                    assert_eq!(registration.finalizer, "volume-local.d2bus.org/layout");
                }
                "Provider/volume-virtiofs" => {
                    assert_eq!(
                        registration.finalizer,
                        d2b_provider_volume_virtiofs::VOLUME_BINDING_FINALIZER
                    );
                }
                _ => {}
            }
        };

        for registration in U6_SHARED_PROVIDER_RUNNERS {
            check_shared_registration(registration);
        }

        // U12: the interaction/shell family is composed by the v3 plane's
        // registered InteractionDriverFactory rather than shared Runner rows;
        // every family ResourceType must route to the new plane.
        for resource_type in crate::interaction_driver::INTERACTION_TYPES {
            assert_eq!(
                crate::resource_plane_v3::route_resource_type(resource_type),
                crate::resource_plane_v3::PlaneRoute::NewPlane,
                "the converted interaction type {resource_type} must route to the new plane",
            );
        }

        // U12: the U8 shared host-provider family (Network; Device: tpm,
        // usbip, security-key, gpu) is composed by the v3 plane's
        // SharedProviderDriverFactory rather than shared Runner rows; every
        // family ResourceType must route to the new plane.
        for resource_type in crate::shared_provider_driver::SHARED_PROVIDER_TYPES {
            assert_eq!(
                crate::resource_plane_v3::route_resource_type(resource_type),
                crate::resource_plane_v3::PlaneRoute::NewPlane,
                "the converted shared-provider type {resource_type} must route to the new plane",
            );
        }

        // U12: the storage Providers are composed by the v3 resource plane's
        // registered Volume/VolumeBinding driver factories rather than a
        // shared Runner row; both converted types must route to the new
        // plane and keep owning an old-plane Runner row's identity.
        for (resource_type, provider) in [
            (crate::volume_driver::VOLUME_TYPE_NAME, "volume-local"),
            (crate::binding_driver::BINDING_TYPE_NAME, "volume-virtiofs"),
        ] {
            assert_eq!(
                crate::resource_plane_v3::route_resource_type(resource_type),
                crate::resource_plane_v3::PlaneRoute::NewPlane,
                "the converted volume type {resource_type} must route to the new plane",
            );
            resource_owners.insert(provider);
        }

        // U12: the activation Provider converted to the v3 plane, so it is
        // composed by the plane's registered driver factory rather than a
        // shared Runner row; its type must route to the new plane.
        assert_eq!(
            crate::resource_plane_v3::route_resource_type(
                crate::activation_driver::ACTIVATION_TYPE_NAME,
            ),
            crate::resource_plane_v3::PlaneRoute::NewPlane,
            "the converted activation type must route to the new plane",
        );
        resource_owners.insert("activation-nixos");

        // U12: the system-core Host/User family is composed by the v3 plane's
        // registered SystemCoreDriverFactory rather than the shared Core
        // Runner; both converted types must route to the new plane.
        for resource_type in ["Host", "User"] {
            assert_eq!(
                crate::resource_plane_v3::route_resource_type(resource_type),
                crate::resource_plane_v3::PlaneRoute::NewPlane,
                "the converted system-core type {resource_type} must route to the new plane",
            );
        }

        assert_eq!(
            resource_owners,
            BTreeSet::from([
                "system-systemd",
                "system-minijail",
                "runtime-cloud-hypervisor",
                "runtime-qemu-media",
                "runtime-azure-container-apps",
                "runtime-azure-virtual-machine",
                "volume-local",
                "volume-virtiofs",
                "activation-nixos",
            ])
        );

        // The Credential Providers are typed rows but no longer own an
        // old-plane controller runner: their Credential resources are served
        // by the v3 `Credential` driver.
        let driver_owned = [
            "credential-secret-service",
            "credential-entra",
            "credential-managed-identity",
        ];
        // U12: the telemetry Provider's types are served by the v3 resource
        // plane, so it no longer carries an old-plane controller runner; the
        // U8 shared host-provider family and the system-core Host/User family
        // joined it on the same plane.
        let new_plane_only = [
            "observability-otel",
            "network-local",
            "device-tpm",
            "device-usbip",
            "device-security-key",
            "device-gpu",
            "system-core",
            "display-wayland",
            "audio-pipewire",
            "shell-terminal",
        ];
        let session_only = ["clipboard-wayland", "notification-desktop"];
        let transport_only = ["transport-unix", "transport-vsock", "transport-azure-relay"];
        assert!(driver_owned
            .into_iter()
            .chain(session_only)
            .chain(transport_only)
            .chain(new_plane_only)
            .all(|provider| expected.contains(provider) && !resource_owners.contains(provider)));
        let mut composed = resource_owners;
        composed.extend(driver_owned);
        composed.extend(session_only);
        composed.extend(transport_only);
        composed.extend(new_plane_only);
        assert_eq!(composed, expected);
    }

    #[test]
    fn every_u6_guest_runtime_descriptor_is_provider_ref_scoped() {
        for registration in U6_SHARED_PROVIDER_RUNNERS {
            let (_, descriptor) = shared_provider_test_descriptor_for(registration);
            assert_eq!(
                descriptor
                    .resource_types()
                    .map(|resource_type| resource_type.as_str())
                    .collect::<Vec<_>>(),
                vec!["Guest"]
            );
            assert!(descriptor
                .watch_selectors()
                .iter()
                .any(|selector| selector.exact_value() == Some(registration.provider_ref)));
            assert!(descriptor.dependency_selectors().iter().any(|selector| {
                selector.resource_type().as_str() == "Process"
            }));
            assert!(registration.watched_configuration_is_dependency);
        }
    }

    #[test]
    fn u6_guest_runtime_kinds_are_closed_to_the_four_provider_rows() {
        assert_eq!(
            SharedProviderResourceKind::from_registration(U6_SHARED_PROVIDER_RUNNERS[0]).unwrap(),
            SharedProviderResourceKind::CloudHypervisorGuest
        );
        assert_eq!(
            SharedProviderResourceKind::from_registration(U6_SHARED_PROVIDER_RUNNERS[1]).unwrap(),
            SharedProviderResourceKind::QemuMediaGuest
        );
        assert_eq!(
            SharedProviderResourceKind::from_registration(U6_SHARED_PROVIDER_RUNNERS[2]).unwrap(),
            SharedProviderResourceKind::AzureContainerAppsGuest
        );
        assert_eq!(
            SharedProviderResourceKind::from_registration(U6_SHARED_PROVIDER_RUNNERS[3]).unwrap(),
            SharedProviderResourceKind::AzureVirtualMachineGuest
        );
    }

    #[test]
    fn cloud_hypervisor_assigned_mutation_errors_keep_retry_classes() {
        assert_eq!(
            cloud_hypervisor_assigned_mutation_error(SourceError::Conflict(ZoneRevision::new(2))),
            CloudHypervisorResourceApiError::Conflict
        );
        assert_eq!(
            cloud_hypervisor_assigned_mutation_error(SourceError::Integrity),
            CloudHypervisorResourceApiError::Conflict
        );
        assert_eq!(
            cloud_hypervisor_assigned_mutation_error(SourceError::Backpressure),
            CloudHypervisorResourceApiError::Transport
        );
        assert_eq!(
            cloud_hypervisor_assigned_mutation_error(SourceError::Timeout),
            CloudHypervisorResourceApiError::Transport
        );
    }

    #[test]
    fn u6_guest_runner_enrolls_its_exact_finalizer_before_effects() {
        let registration = U6_SHARED_PROVIDER_RUNNERS[1];
        let (_, descriptor) = shared_provider_test_descriptor_for(registration);
        let reconciler = SharedProviderResourceReconciler::new(
            descriptor,
            SharedProviderResourceKind::QemuMediaGuest,
            Arc::new(UnavailableSharedProviderEffects),
        );
        let result = reconciler
            .first_pass_for_test(&shared_provider_test_resource_for(
                registration,
                &[],
                false,
            ))
            .expect("finalizer enrollment result");
        let mutation = result
            .mutation_batch()
            .expect("first pass must mutate only finalizers")
            .mutations()
            .first()
            .expect("finalizer mutation");
        assert_eq!(
            mutation.kind(),
            d2b_core_controller::MutationIntentKind::UpdateFinalizers
        );
        let payload = mutation
            .canonical_resource()
            .expect("full finalizer candidate");
        let value: Value = serde_json::from_slice(payload).expect("candidate JSON");
        assert_eq!(
            value["metadata"]["finalizers"],
            serde_json::json!([registration.finalizer])
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

    fn test_audit_sink(directory: &std::path::Path, name: &str) -> Arc<AuditSink> {
        Arc::new(AuditSink::open(directory.join(name)).unwrap())
    }

    struct PublicationStoreFixture {
        _directory: tempfile::TempDir,
        database_path: std::path::PathBuf,
        response_identity: String,
        zone: ZoneId,
        identity: d2b_resource_store_redb::StoreIdentity,
    }

    impl PublicationStoreFixture {
        async fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let database_path = directory.path().join("store.redb");
            let zone = ZoneId::parse("work").unwrap();
            let response_identity = "sha256:".to_owned() + &"1".repeat(64);
            let identity = store_identity(&zone, &response_identity).unwrap();
            let database = OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(&database_path)
                .unwrap();
            let mut marker = OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(directory.path().join(".d2b-store-marker"))
                .unwrap();
            write_provisioning_marker(&mut marker, &identity).unwrap();
            RedbResourceStore::provision_owned(
                database,
                marker,
                identity.clone(),
                mutation_seal_pair(identity.seal_identity()).1,
            )
            .await
            .unwrap()
            .shutdown()
            .await
            .unwrap();
            Self {
                _directory: directory,
                database_path,
                response_identity,
                zone,
                identity,
            }
        }

        async fn open(&self, bundle: &ResourceBundle) -> ZoneResourceRuntime {
            let database = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.database_path)
                .unwrap();
            let mut runtime = ZoneResourceRuntime::open(
                self.zone.clone(),
                OpenedZoneStore {
                    response: OpenZoneStoreResponse {
                        zone_store_id: d2b_contracts_resource::v3::storage::ZoneStoreId::parse(
                            "zone-store-work",
                        )
                        .unwrap(),
                        store_identity: self.response_identity.clone(),
                        disposition: ZoneStoreDisposition::Opened,
                        fd_index: 0,
                    },
                    database_fd: database.into(),
                    external_inventory: None,
                },
            )
            .await
            .unwrap();
            let storage = publication_storage_row(&self.zone, &self.identity);
            runtime.authority_identity = Some(
                ZoneAuthorityIdentity::from_bundle_and_storage(&self.zone, bundle, &storage)
                    .unwrap(),
            );
            runtime
        }

        fn generation_set(
            &self,
            bundle: &ResourceBundle,
        ) -> (
            ResourceBundleGenerationId,
            BTreeMap<ZoneId, ResourceBundleGenerationId>,
        ) {
            let generation =
                ResourceBundleGenerationId::parse(bundle.integrity().content_hash.clone()).unwrap();
            let generations = BTreeMap::from([(self.zone.clone(), generation)]);
            let set_generation =
                complete_generation_set_digest(&BTreeSet::from([self.zone.clone()]), &generations)
                    .unwrap();
            (set_generation, generations)
        }
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

    fn attach_ready_controller(
        providers: &Arc<crate::process_provider_runtime::ProductionProcessProviders>,
        zone: &ZoneId,
        process_ref: &str,
        process_uid: &str,
        process_generation: u64,
        provider_ref: &str,
        provider_uid: &str,
        provider_generation: u64,
        controller_generation: ControllerGeneration,
    ) -> OwnedFd {
        let (daemon_endpoint, peer_endpoint) = prearmed_seqpacket_pair().unwrap();
        nix::sys::socket::send(
            peer_endpoint.as_raw_fd(),
            b"controller-ready",
            nix::sys::socket::MsgFlags::empty(),
        )
        .unwrap();
        providers
            .attach_pending_controller_provider_context_for_test(
                daemon_endpoint,
                zone.clone(),
                ResourceRef::parse(process_ref).unwrap(),
                ResourceUid::parse(process_uid).unwrap(),
                ResourceGeneration::new(process_generation).unwrap(),
                ResourceRef::parse("Provider/system-minijail").unwrap(),
                ResourceRef::parse(provider_ref).unwrap(),
                ResourceUid::parse(provider_uid).unwrap(),
                ResourceGeneration::new(provider_generation).unwrap(),
                ResourceRef::parse("Host/host-system").unwrap(),
                controller_generation,
            )
            .unwrap();
        peer_endpoint
    }

    async fn insert_test_controller_session(
        coordinator: &mut ControllerSessionCoordinator,
        context: &crate::process_provider_runtime::ControllerBootstrapContext,
    ) {
        let policy = controller_resource_endpoint_policy();
        let catalog = d2b_resource_api::authz::ApiCatalog::standard();
        let role_ref = ResourceRef::parse("Role/test-controller").unwrap();
        let role = d2b_resource_api::authz::CompiledRole::new(
            role_ref.clone(),
            vec![
                d2b_resource_api::authz::PolicyRule::new(
                    &catalog,
                    [],
                    [],
                    [d2b_resource_api::authz::SessionVerb::Connect],
                    [],
                    [],
                    [context.zone().clone()],
                    [],
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let metadata = coordinator.store.runtime_metadata().await.unwrap();
        let state = AuthorizationState {
            snapshot: metadata.policy_snapshot,
            zone_policy_revision: metadata.current_revision,
            bootstrap_phase: d2b_resource_api::authz::BootstrapPhase::Disabled,
            now_tick: 1,
        };
        let binding = d2b_resource_api::authz::CompiledRoleBinding::new(
            role_ref,
            [BoundSubject {
                subject_ref: context.provider_owner_ref().clone(),
                subject_uid: context.provider_uid().clone(),
            }],
            d2b_resource_api::authz::BindingScope {
                zones: [context.zone().clone()].into_iter().collect(),
                ..d2b_resource_api::authz::BindingScope::default()
            },
            d2b_resource_api::authz::RelayGrantAuthority::None,
        )
        .unwrap();
        let policy_set =
            PolicySet::new(&catalog, state.snapshot.policy_revision, vec![role], vec![binding])
                .unwrap();
        let native = NativeAuthorizer::new(catalog, Some(policy_set)).unwrap();
        let bus_authorizer = BusAuthorizer::new(native, state).unwrap();
        let (_bus, registrar) =
            ZoneBus::new(context.zone().clone(), bus_authorizer, BusConfig::default()).unwrap();
        coordinator.registrar = Arc::new(Mutex::new(Some(registrar)));

        let (initiator_fd, responder_fd) = prearmed_seqpacket_pair().unwrap();
        let initiator_socket = SeqpacketSocket::from_parent_prearmed(initiator_fd).unwrap();
        let responder_socket = SeqpacketSocket::from_parent_prearmed(responder_fd).unwrap();
        let verified_peer =
            VerifiedUnixPeer::verify_inherited_seqpacket(&initiator_socket).unwrap();
        let mut registrar = coordinator.registrar.lock().unwrap().take().unwrap();
        registrar
            .install_committed_controller_process_subject(
                &verified_peer,
                CommittedControllerProcessSubjectInput {
                    provider_ref: context.provider_owner_ref().clone(),
                    provider_uid: context.provider_uid().clone(),
                    process_ref: context.process_ref().clone(),
                    zone_ref: ResourceRef::parse("Zone/work").unwrap(),
                    execution_ref: context.execution_ref().clone(),
                    provider_generation: context.provider_generation(),
                    controller_generation: context.controller_generation(),
                },
            )
            .unwrap();
        let initiator = unix_transport(initiator_socket, &policy).unwrap();
        let responder = unix_transport(responder_socket, &policy).unwrap();
        let (initiator, responder) = tokio::join!(
            SessionEngine::establish_initiator(
                initiator,
                policy.clone(),
                HandshakeCredentials::Nn,
                std::time::Instant::now(),
            ),
            SessionEngine::establish_responder(
                responder,
                policy.clone(),
                HandshakeCredentials::Nn,
                std::time::Instant::now(),
            ),
        );
        let acceptor = registrar
            .component_session_acceptor(policy, verified_peer)
            .unwrap();
        let candidate = acceptor
            .admit(
                initiator.unwrap(),
                TransportEvidence::new(
                    EvidenceClass::UnixPeer,
                    BindingDigest::parse(format!("sha256:{}", "22".repeat(32))).unwrap(),
                ),
                1,
            )
            .await
            .unwrap();
        let (ingress, driver) = registrar
            .register_component_service_session(candidate)
            .await
            .unwrap();
        *coordinator.registrar.lock().unwrap() = Some(registrar);

        let session_generation = ReconnectGeneration::new(1).unwrap();
        let binding = controller_session_binding(context, session_generation).unwrap();
        let service_task = tokio::spawn(async { Ok::<(), SessionServerError>(()) });
        coordinator
            .controller_sessions
            .lock()
            .unwrap()
            .insert(
                context.process_ref().clone(),
                ControllerSession {
                    context: context.clone(),
                    binding,
                    ingress,
                    driver,
                    _backend_lease: None,
                    resource_client: None,
                    service_task,
                    assignments: BTreeMap::new(),
                    assignment_stream_open: false,
                    assignments_revoked: false,
                    transport_closed: false,
                    ingress_revoked: false,
                },
            );
        drop(responder);
    }

    async fn read_test_resource(
        runtime: &ZoneResourceRuntime,
        target: ResourceRef,
        operation_id: &str,
    ) -> StoredResource {
        runtime
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: operation_id.to_owned(),
                    idempotency_key: None,
                    correlation_id: operation_id.to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: runtime.zone.clone(),
                target,
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .unwrap()
    }

    fn publication_storage_row(
        zone: &ZoneId,
        identity: &d2b_resource_store_redb::StoreIdentity,
    ) -> ZoneStoreStorageRow {
        let storage_identity = ZoneStoreIdentity::new(
            identity.zone_uid().clone(),
            identity.store_uid().clone(),
            identity.store_epoch(),
        )
        .unwrap();
        serde_json::from_value(json!({
            "identity": storage_identity,
            "zoneStoreId": format!("zone-store-{}", zone.as_str()),
            "storageOwnerPrincipal": "d2b-zonert",
            "parentDirectoryId": format!("zone-store-parent-{}", zone.as_str()),
            "ownership": {
                "owner": "d2b-zonert", "group": "d2b-zonert",
                "mode": "0640", "linkCount": 1
            },
            "auxiliaryDirectories": {
                "audit": {
                    "directoryId": format!("zone-store-audit-{}", zone.as_str()),
                    "owner": "d2bd", "group": "d2bd",
                    "mode": "0700", "repairOwner": "privileged-broker"
                },
                "telemetry": {
                    "directoryId": format!("zone-store-telemetry-{}", zone.as_str()),
                    "owner": "d2bd", "group": "d2bd",
                    "mode": "0700", "repairOwner": "privileged-broker"
                }
            },
            "filesystem": "regular-file-anchored-fd-relative-no-follow",
            "locking": "ofd-close-on-exec",
            "marker": {
                "identityMarkerId": format!("zone-store-marker-{}", zone.as_str())
            },
            "replacementDetection": "fail-closed-on-missing-replaced-or-identity-mismatch",
            "fsync": "database-and-parent-directory",
            "publication": {
                "descriptor": "owned-descriptor-close-on-exec-verified-before-concurrency",
                "replacement": "atomic-rename-retain-prior-quarantine-ambiguity"
            }
        }))
        .unwrap()
    }

    fn publication_bundle(zone: &ZoneId, zone_uid: &ResourceUid, value: &str) -> ResourceBundle {
        let resource = BundleResource::new(
            ResourceTypeName::parse("Host").unwrap(),
            BundleResourceMetadata::new(
                ResourceName::parse(format!("generation-{value}")).unwrap(),
                zone.clone(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(format!(r#"{{"value":"{value}"}}"#).as_bytes()).unwrap(),
        )
        .unwrap();
        ResourceBundle::new(
            zone.clone(),
            vec![resource],
            "sha256:".to_owned() + &"f".repeat(64),
            BTreeMap::new(),
            BTreeMap::new(),
            Timestamp::parse("2026-08-26T00:00:00.000Z").unwrap(),
        )
        .unwrap()
        .with_zone_uid(zone_uid.clone())
    }

    async fn publish_generation(
        runtime: &ZoneResourceRuntime,
        set_generation: &ResourceBundleGenerationId,
        generations: &BTreeMap<ZoneId, ResourceBundleGenerationId>,
    ) {
        runtime
            .prepare_generation_publication(set_generation, generations)
            .await
            .unwrap();
        runtime
            .commit_generation_publication(set_generation, generations)
            .await
            .unwrap();
    }

    async fn publication_state(
        runtime: &ZoneResourceRuntime,
        set_generation: &ResourceBundleGenerationId,
    ) -> AuthorityOperationState {
        runtime
            .store
            .authority_operations()
            .await
            .unwrap()
            .into_iter()
            .find(|operation| {
                operation.operation_id == generation_publication_operation_id(set_generation)
            })
            .unwrap()
            .state
    }

    #[tokio::test]
    async fn system_core_refresh_waits_for_controller_session_guard() {
        let fixture = PublicationStoreFixture::new().await;
        let bundle =
            publication_bundle(&fixture.zone, fixture.identity.zone_uid(), "session-guard");
        let runtime = fixture.open(&bundle).await;
        let state = AuthorizationState {
            snapshot: runtime.store_metadata.policy_snapshot,
            zone_policy_revision: runtime.store_metadata.current_revision,
            bootstrap_phase: d2b_resource_api::authz::BootstrapPhase::Disabled,
            now_tick: 0,
        };

        let guard = runtime.controller_session_lock.lock().await;
        {
            let refresh = runtime.refresh_system_core_session(state);
            tokio::pin!(refresh);
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(20), &mut refresh)
                    .await
                    .is_err(),
                "system-core refresh must not bypass controller-session establishment"
            );
            drop(guard);
            assert_eq!(refresh.await, Ok(()));
        }
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn public_policy_refresh_preserves_installed_controller_subjects() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let zone = runtime.zone.clone();
        let provider_ref = ResourceRef::parse("Provider/system-minijail").unwrap();
        materialize_test_bundle(
            &runtime,
            vec![bundle_resource(
                "Provider",
                "system-minijail",
                &zone,
                r#"{"artifactId":"system-minijail","config":{}}"#,
            )],
        )
        .await;
        let providers = test_controller_session_providers();
        *runtime.controller_session_providers.lock().unwrap() = Some(Arc::clone(&providers));
        let provider = runtime
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "controller-policy-provider-read".to_owned(),
                    idempotency_key: None,
                    correlation_id: "controller-policy-provider-read".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: zone.clone(),
                target: provider_ref.clone(),
                expected_uid: None,
                projection: StoreProjection::MetadataOnly,
            })
            .await
            .unwrap();
        let metadata = runtime.store.runtime_metadata().await.unwrap();
        let controller_generation = metadata
            .policy_snapshot
            .controller_generation
            .expect("test policy has a controller generation");
        providers
            .attach_controller_provider_context_for_test(
                zone.clone(),
                ResourceRef::parse("Process/system-minijail-controller").unwrap(),
                ResourceUid::parse("323e4567-e89b-42d3-a456-426614174000").unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ResourceRef::parse("Provider/system-minijail").unwrap(),
                provider_ref.clone(),
                provider.uid.clone(),
                provider.generation,
                ResourceRef::parse("Host/host-system").unwrap(),
                controller_generation,
            )
            .unwrap();
        assert!(
            runtime
                .policy_projection
                .installed_controller_subjects()
                .unwrap()
                .is_empty()
        );

        // The Provider materialization advanced the store revision, so this
        // refresh must recompile instead of returning from its installed-state
        // fast path.
        runtime.refresh_authorization_policy().await.unwrap();

        let controller_subject = BoundSubject {
            subject_ref: provider_ref,
            subject_uid: provider.uid,
        };
        let state = runtime.policy_projection.installed_state().unwrap();
        let authorization_state = runtime
            .authorization_state
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        assert_eq!(state.snapshot, authorization_state.snapshot);
        assert_eq!(
            state.zone_policy_revision,
            authorization_state.zone_policy_revision
        );
        assert_eq!(
            runtime
                .policy_projection
                .installed_controller_subjects()
                .unwrap(),
            BTreeSet::from([controller_subject.clone()])
        );
        let context = || {
            AuthenticatedSubjectContext::new(
                controller_subject.subject_ref.clone(),
                controller_subject.subject_uid.clone(),
                ResourceRef::parse("Zone/work").unwrap(),
                EvidenceClass::UnixPeer,
                d2b_contracts_resource::v3::identity::SessionPurpose::parse("resource-api")
                    .unwrap(),
                d2b_contracts_resource::v3::identity::ServiceName::parse("d2b.resource.v3")
                    .unwrap(),
                d2b_contracts_resource::v3::identity::SessionBinding::new(
                    d2b_contracts_resource::v3::SchemaFingerprint::parse(format!(
                        "sha256:{}",
                        "1".repeat(64)
                    ))
                    .unwrap(),
                    d2b_contracts_resource::v3::identity::TransportBinding::new(
                        d2b_contracts_resource::v3::identity::Locality::Local,
                        BindingDigest::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
                    ),
                    ReconnectGeneration::new(1).unwrap(),
                    d2b_contracts_resource::v3::identity::TranscriptHash::from_bytes([3; 32]),
                ),
            )
        };
        assert!(
            runtime
                .authorizer
                .issue_authenticated_subject(context(), state.clone())
                .is_ok(),
            "policy refresh must retain the installed Provider controller grant"
        );
        let bus_authorizer = runtime
            .bus
            .as_ref()
            .expect("production runtime has a ZoneBus")
            .native_authorizer();
        assert!(
            bus_authorizer
                .issue_authenticated_subject(context(), state)
                .is_ok(),
            "ZoneBus must retain the installed Provider controller grant"
        );
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn policy_projection_preflight_failure_preserves_last_good_projection() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let before = runtime.policy_projection.installed_state().unwrap();
        let resources = d2bd_runtime::resource_runtime_support::load_committed_policy_resources(
            &runtime.store,
            &runtime.zone,
            "policy-preflight-regression",
        )
        .await
        .unwrap();
        let metadata = runtime.store.runtime_metadata().await.unwrap();
        let (policy, mut rejected_state) =
            d2bd_runtime::resource_runtime_support::compile_committed_policy_with_subjects(
                &runtime.zone,
                metadata.policy_snapshot,
                metadata.current_revision,
                &runtime.bundle_resource_types,
                &resources,
                std::iter::empty(),
            )
            .unwrap();
        rejected_state.snapshot.policy_revision =
            rejected_state.snapshot.policy_revision.saturating_add(1);

        assert_eq!(
            runtime
                .policy_projection
                .install(policy, rejected_state, BTreeSet::new()),
            Err(ResourceRuntimeError::AuthorizationUnavailable)
        );
        assert_eq!(runtime.policy_projection.installed_state().unwrap(), before);
        assert!(*runtime.policy_projection.policy_loaded.lock().unwrap());
        assert_eq!(
            runtime
                .authorization_state
                .lock()
                .unwrap()
                .as_ref(),
            Some(&before)
        );
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn public_get_and_list_use_installed_policy_after_rebind_failure() {
        let (_directory, runtime, broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let zone = runtime.zone.clone();
        let process_provider_ref = ResourceRef::parse("Provider/system-minijail").unwrap();
        let process_ref = ResourceRef::parse("Process/system-minijail-controller").unwrap();
        let uid = Uid::current();
        let username = User::from_uid(uid)
            .unwrap()
            .expect("test peer uid has an NSS user")
            .name;
        let user_ref = ResourceRef::parse(&format!("User/{username}")).unwrap();
        let role_ref = ResourceRef::parse("Role/public-read").unwrap();
        let role = d2b_contracts_zone_session::v3::role::RoleSpec::new(vec![
            d2b_contracts_zone_session::v3::role::RoleRule::new(
                vec![
                    ResourceTypeName::parse("Guest").unwrap(),
                    ResourceTypeName::parse("EphemeralProcess").unwrap(),
                ],
                vec![
                    d2b_contracts_zone_session::v3::role::RoleResourceVerb::Get,
                    d2b_contracts_zone_session::v3::role::RoleResourceVerb::List,
                ],
                Vec::new(),
                Vec::new(),
                vec![zone.clone()],
                Vec::new(),
                vec![d2b_contracts_zone_session::v3::role::RoleSessionVerb::Connect],
            )
            .unwrap(),
        ])
        .unwrap();
        let binding =
            d2b_contracts_zone_session::v3::role_binding::RoleBindingSpec::new(
                role_ref,
                vec![user_ref.clone()],
                None,
                None,
            )
            .unwrap();
        materialize_test_bundle(
            &runtime,
            vec![
                bundle_resource(
                    "Provider",
                    "system-minijail",
                    &zone,
                    r#"{"artifactId":"system-minijail","config":{}}"#,
                ),
                BundleResource::new(
                    ResourceTypeName::parse("Process").unwrap(),
                    BundleResourceMetadata::new(
                        process_ref.name().clone(),
                        zone.clone(),
                        Some(process_provider_ref.clone()),
                        BTreeMap::new(),
                        BTreeMap::new(),
                    ),
                    CanonicalJsonObject::parse(
                        br#"{"executionRef":"Host/host-system","processClass":"controller","providerRef":"Provider/system-minijail","template":"test-controller"}"#,
                    )
                    .unwrap(),
                )
                .unwrap(),
                bundle_resource(
                    "User",
                    &username,
                    &zone,
                    &serde_json::to_string(&json!({
                        "displayName": username,
                        "groups": [],
                        "osUsername": username,
                    }))
                    .unwrap(),
                ),
                bundle_resource(
                    "Role",
                    "public-read",
                    &zone,
                    &serde_json::to_string(&role).unwrap(),
                ),
                bundle_resource(
                    "RoleBinding",
                    "public-read",
                    &zone,
                    &serde_json::to_string(&binding).unwrap(),
                ),
            ],
        )
        .await;
        mark_test_resource_ready(&runtime, &user_ref, &broker_evidence).await;
        let process_provider = read_test_resource(
            &runtime,
            process_provider_ref.clone(),
            "rebind-process-provider",
        )
        .await;
        let process = read_test_resource(&runtime, process_ref.clone(), "rebind-process")
            .await;
        let providers = test_controller_session_providers();
        *runtime.controller_session_providers.lock().unwrap() = Some(Arc::clone(&providers));
        let controller_generation = runtime
            .store
            .runtime_metadata()
            .await
            .unwrap()
            .policy_snapshot
            .controller_generation
            .expect("test policy has a controller generation");
        providers
            .attach_controller_provider_context_for_test(
                zone.clone(),
                process_ref.clone(),
                process.uid.clone(),
                process.generation,
                process_provider_ref.clone(),
                process_provider_ref.clone(),
                process_provider.uid.clone(),
                process_provider.generation,
                ResourceRef::parse("Host/host-system").unwrap(),
                controller_generation,
            )
            .unwrap();
        let coordinator = runtime.controller_session_coordinator();
        let authority = runtime.core_assignment_fences().await.unwrap().4;
        let authorization_state = runtime
            .authorization_state
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        coordinator
            .set_assigned_process_api(Arc::new(
                runtime
                    .process_controller_api(
                        DaemonMode::Host,
                        authority,
                        authorization_state.clone(),
                    )
                    .unwrap(),
            ))
            .unwrap();
        runtime.refresh_authorization_policy().await.unwrap();
        let before_rebind_authority = runtime.core_assignment_fences().await.unwrap().4;
        let before_rebind_authorization_state = runtime
            .authorization_state
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        let before_rebind_process =
            read_test_resource(&runtime, process_ref.clone(), "rebind-process-before-fence").await;
        let before_rebind_api = Arc::new(
            runtime
                .process_controller_api(
                    DaemonMode::Host,
                    Arc::clone(&before_rebind_authority),
                    before_rebind_authorization_state,
                )
                .unwrap(),
        );
        let before_rebind_fence = process_assignment_fence_resolver(
            Arc::clone(&runtime.store),
            DaemonMode::Host,
            Arc::clone(&before_rebind_authority),
        )(
            process_ref.clone(),
            before_rebind_process.uid.clone(),
            before_rebind_process.revision,
        )
        .await
        .expect("pre-rebind Process assignment fence");
        assert_eq!(
            before_rebind_fence,
            ResourceAssignmentFence {
                resource_uid: before_rebind_process.uid.clone(),
                resource_revision: before_rebind_process.revision,
                provider_generation: before_rebind_authority.provider_generation,
                controller_generation: before_rebind_authority.controller_generation,
                controller_role: before_rebind_authority.controller_role.clone(),
                target: ResourceRef::parse("Host/host-system").unwrap(),
                session_generation: before_rebind_authority.session_generation,
                epoch: ASSIGNMENT_EPOCH,
                scope: ResourceAssignmentScope::Primary,
            }
        );
        let before_rebind_revision = runtime
            .store
            .runtime_metadata()
            .await
            .unwrap()
            .current_revision;
        let registrar = runtime
            .registrar
            .lock()
            .unwrap()
            .take()
            .expect("system-core registrar");
        let service_task = runtime
            .service_task
            .lock()
            .unwrap()
            .take()
            .expect("system-core session task");
        service_task.abort();
        materialize_test_bundle(
            &runtime,
            vec![bundle_resource(
                "Role",
                "rebind-trigger",
                &zone,
                &serde_json::to_string(&role).unwrap(),
            )],
        )
        .await;
        assert!(
            runtime.store.runtime_metadata().await.unwrap().current_revision
                > before_rebind_revision,
            "rebind trigger must advance the committed revision"
        );
        assert_eq!(
            runtime.refresh_authorization_policy().await,
            Err(ResourceRuntimeError::AuthenticationUnavailable)
        );
        assert!(
            runtime.policy_projection.installed_state().is_ok(),
            "a failed session rebind must preserve the complete installed policy"
        );
        let installed_after_failure = runtime.policy_projection.installed_state().unwrap();
        let metadata_after_failure = runtime.store.runtime_metadata().await.unwrap();
        assert_eq!(
            installed_after_failure.zone_policy_revision,
            metadata_after_failure.current_revision,
            "the retryable failure must occur after the complete projection is installed"
        );
        assert!(
            coordinator.assigned_process_api.lock().unwrap().is_none(),
            "failed system-core rebind must invalidate the assigned Process API"
        );
        let context = providers
            .controller_bootstrap_contexts(&zone)
            .into_iter()
            .find(|context| context.process_ref() == &process_ref)
            .expect("rebind Process controller context");
        assert_eq!(
            coordinator
                .persist_controller_session_evidence(
                    &context,
                    Some(ReconnectGeneration::new(1).unwrap()),
                )
                .await,
            Err(ResourceRuntimeError::AuthenticationUnavailable),
            "pending evidence must not write through an invalidated Process API"
        );

        let guard = runtime.controller_session_lock.lock().await;
        for request in [
            json!({
                "method": "Get",
                "zoneRef": "Zone/work",
                "resourceRef": "Guest/rebind-guest",
            }),
            json!({
                "method": "List",
                "zoneRef": "Zone/work",
                "resourceType": "EphemeralProcess",
            }),
        ] {
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                runtime.dispatch_public_cli_request(&request, uid.as_raw()),
            )
            .await
            .expect("public read must not wait for controller-session policy refresh")
            .expect("public read should use the installed authorization projection");
        }
        drop(guard);
        let _ = service_task.await;
        *runtime.registrar.lock().unwrap() = Some(registrar);
        runtime
            .refresh_authorization_policy()
            .await
            .expect("the next refresh must retry system-core rebind recovery");
        assert!(
            runtime.service_task.lock().unwrap().is_some(),
            "recovery must re-enroll the fenced system-core session"
        );
        assert!(
            coordinator.assigned_process_api.lock().unwrap().is_some(),
            "successful system-core rebind must restore the assigned Process API"
        );
        let after_rebind_authority = runtime.core_assignment_fences().await.unwrap().4;
        let after_rebind_process =
            read_test_resource(&runtime, process_ref.clone(), "rebind-process-after-fence").await;
        let rebuilt_process_fence = process_assignment_fence_resolver(
            Arc::clone(&runtime.store),
            DaemonMode::Host,
            Arc::clone(&after_rebind_authority),
        )(
            process_ref.clone(),
            after_rebind_process.uid.clone(),
            after_rebind_process.revision,
        )
        .await
        .expect("rebuilt Process assignment fence");
        assert_eq!(
            rebuilt_process_fence,
            before_rebind_fence.clone(),
            "rebind must preserve Process identity and fence material",
        );
        assert_eq!(
            after_rebind_authority.provider_generation,
            before_rebind_authority.provider_generation
        );
        assert_eq!(
            after_rebind_authority.controller_generation,
            before_rebind_authority.controller_generation
        );
        assert_eq!(
            after_rebind_authority.controller_role,
            before_rebind_authority.controller_role
        );
        assert_eq!(
            after_rebind_authority.target,
            before_rebind_authority.target
        );
        assert_eq!(
            after_rebind_authority.session_generation,
            before_rebind_authority.session_generation
        );
        coordinator
            .persist_controller_session_evidence(
                &context,
                Some(ReconnectGeneration::new(1).unwrap()),
            )
            .await
            .expect("rebind must rebuild the assigned Process API before evidence writes");
        let current_process =
            read_test_resource(&runtime, process_ref.clone(), "rebind-process-current-fence")
                .await;
        let current_process_fence = process_assignment_fence_resolver(
            Arc::clone(&runtime.store),
            DaemonMode::Host,
            Arc::clone(&after_rebind_authority),
        )(
            process_ref.clone(),
            current_process.uid.clone(),
            current_process.revision,
        )
        .await
        .expect("current Process assignment fence");
        let durable_current_fence = runtime
            .store
            .assignment_fence(zone.clone(), process_ref.clone())
            .await
            .unwrap()
            .expect("rebuilt Process assignment fence must persist");
        assert_eq!(durable_current_fence, current_process_fence);
        assert!(
            persist_resource_controller_session_evidence(
                &before_rebind_api,
                &current_process,
                Some(&json!({"ready": false, "stale": true})),
            )
            .await
            .is_ok(),
            "same-identity pre-rebind evidence re-fences idempotently without an epoch"
        );
        drop(before_rebind_api);
        drop(coordinator);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn controller_session_reconcile_wake_survives_guard_contention() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let coordinator = runtime.controller_session_coordinator();
        let providers = test_controller_session_providers();
        let task_slot = Arc::clone(&runtime.controller_session_reconcile_task);
        let wake = Arc::clone(&runtime.controller_session_reconcile_wake);
        let shutdown = Arc::clone(&runtime.controller_session_reconcile_shutdown);
        *runtime.authorization_state.lock().unwrap() = None;

        let guard = runtime.controller_session_lock.lock().await;
        schedule_controller_session_reconcile(
            task_slot,
            wake,
            Arc::clone(&shutdown),
            Arc::clone(&coordinator),
            providers,
        )
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if coordinator.reconcile_attempts.load(Ordering::SeqCst) == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("controller-session worker should attempt reconciliation");
        assert!(runtime.authorization_state.lock().unwrap().is_none());
        schedule_controller_session_reconcile(
            Arc::clone(&runtime.controller_session_reconcile_task),
            Arc::clone(&runtime.controller_session_reconcile_wake),
            Arc::clone(&shutdown),
            Arc::clone(&coordinator),
            test_controller_session_providers(),
        )
        .unwrap();

        drop(guard);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if runtime.authorization_state.lock().unwrap().is_some() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("guarded controller-session wake should reconcile after release");
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if coordinator.reconcile_attempts.load(Ordering::SeqCst) == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("wake arriving during reconciliation should trigger one later pass");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(
            coordinator.reconcile_attempts.load(Ordering::SeqCst),
            2,
            "one wake arriving during reconciliation must not create a hot loop"
        );

        shutdown.store(true, Ordering::Release);
        drop(coordinator);
        runtime.shutdown().await.unwrap();
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn controller_session_admission_freezes_pending_snapshot_across_passes() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let zone = ZoneId::parse("work").unwrap();
        materialize_test_bundle(&runtime, Vec::new()).await;
        let first_provider_ref =
            ResourceRef::parse("Provider/runtime-cloud-hypervisor").expect("first provider");
        let second_provider_ref =
            ResourceRef::parse("Provider/runtime-qemu-media").expect("second provider");
        let first_process_ref =
            ResourceRef::parse("Process/provider-controller-first").expect("first process");
        let second_process_ref =
            ResourceRef::parse("Process/provider-controller-second").expect("second process");
        let controller_process = |name: &str, owner_ref: &ResourceRef, template: &str| {
            BundleResource::new(
                ResourceTypeName::parse("Process").unwrap(),
                BundleResourceMetadata::new(
                    ResourceName::parse(name).unwrap(),
                    zone.clone(),
                    Some(owner_ref.clone()),
                    BTreeMap::new(),
                    BTreeMap::new(),
                ),
                CanonicalJsonObject::parse(
                    format!(
                        r#"{{"executionRef":"Host/host-system","processClass":"controller","providerRef":"Provider/system-minijail","template":"{template}"}}"#
                    )
                    .as_bytes(),
                )
                .unwrap(),
            )
            .unwrap()
        };
        materialize_test_bundle(
            &runtime,
            vec![
                bundle_resource(
                    "Provider",
                    "runtime-cloud-hypervisor",
                    &zone,
                    r#"{"artifactId":"runtime-cloud-hypervisor","config":{}}"#,
                ),
                bundle_resource(
                    "Provider",
                    "runtime-qemu-media",
                    &zone,
                    r#"{"artifactId":"runtime-qemu-media","config":{}}"#,
                ),
                controller_process(
                    first_process_ref.name().as_str(),
                    &first_provider_ref,
                    "acceptance-controller",
                ),
                controller_process(
                    second_process_ref.name().as_str(),
                    &second_provider_ref,
                    "acceptance-controller",
                ),
            ],
        )
        .await;
        materialize_test_bundle(
            &runtime,
            vec![
                bundle_resource(
                    "Provider",
                    "runtime-qemu-media",
                    &zone,
                    r#"{"artifactId":"runtime-qemu-media","config":{"replacement":true}}"#,
                ),
                controller_process(
                    second_process_ref.name().as_str(),
                    &second_provider_ref,
                    "acceptance-controller-replacement",
                ),
            ],
        )
        .await;
        let first_provider =
            read_test_resource(&runtime, first_provider_ref.clone(), "admission-first-provider")
                .await;
        let second_provider = read_test_resource(
            &runtime,
            second_provider_ref.clone(),
            "admission-second-provider",
        )
        .await;
        let first_process =
            read_test_resource(&runtime, first_process_ref.clone(), "admission-first-process")
                .await;
        let second_process = read_test_resource(
            &runtime,
            second_process_ref.clone(),
            "admission-second-process",
        )
        .await;
        assert_eq!(second_provider.generation.get(), 2);
        assert_eq!(second_process.generation.get(), 2);
        let controller_generation = runtime
            .store
            .runtime_metadata()
            .await
            .unwrap()
            .policy_snapshot
            .controller_generation
            .expect("test policy has a controller generation");
        let providers = test_controller_session_providers();
        let coordinator = runtime.controller_session_coordinator();
        let authority = runtime.core_assignment_fences().await.unwrap().4;
        let authorization_state = runtime
            .authorization_state
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        coordinator
            .set_assigned_process_api(Arc::new(
                runtime
                    .process_controller_api(DaemonMode::Host, authority, authorization_state)
                    .unwrap(),
            ))
            .unwrap();
        let wake = Arc::clone(&runtime.controller_session_reconcile_wake);
        let wake_for_waker = Arc::clone(&wake);
        let shutdown = Arc::clone(&runtime.controller_session_reconcile_shutdown);
        let task_slot_for_waker = Arc::clone(&runtime.controller_session_reconcile_task);
        let shutdown_for_waker = Arc::clone(&shutdown);
        let coordinator_for_waker = Arc::downgrade(&coordinator);
        let providers_for_waker = Arc::downgrade(&providers);
        let wake_count = Arc::new(AtomicUsize::new(0));
        let callback_wake_count = Arc::clone(&wake_count);
        providers
            .set_controller_session_waker(
                zone.clone(),
                Arc::new(move || {
                    callback_wake_count.fetch_add(1, Ordering::SeqCst);
                    let coordinator = coordinator_for_waker
                        .upgrade()
                        .ok_or_else(|| "controller-session-coordinator-dropped".to_owned())?;
                    let providers = providers_for_waker
                        .upgrade()
                        .ok_or_else(|| "process-providers-dropped".to_owned())?;
                    schedule_controller_session_reconcile(
                        Arc::clone(&task_slot_for_waker),
                        Arc::clone(&wake_for_waker),
                        Arc::clone(&shutdown_for_waker),
                        coordinator,
                        providers,
                    )
                    .map_err(|error| format!("{error:?}"))
                }),
            )
            .unwrap();
        let peers = Arc::new(Mutex::new(Vec::<OwnedFd>::new()));
        let hook_ran = Arc::new(AtomicBool::new(false));
        let stale_context = Arc::new(Mutex::new(None));
        let peers_for_hook = Arc::clone(&peers);
        let hook_ran_for_hook = Arc::clone(&hook_ran);
        let stale_context_for_hook = Arc::clone(&stale_context);
        let providers_for_hook = Arc::clone(&providers);
        let second_snapshot_seen = Arc::new(AtomicBool::new(false));
        let second_snapshot_seen_for_hook = Arc::clone(&second_snapshot_seen);
        let second_snapshot_release = Arc::new(AtomicBool::new(false));
        let second_snapshot_release_for_hook = Arc::clone(&second_snapshot_release);
        let zone_for_hook = zone.clone();
        let replacement_process_uid = second_process.uid.clone();
        let replacement_process_generation = second_process.generation.get();
        let replacement_provider_uid = second_provider.uid.clone();
        let replacement_provider_generation = second_provider.generation.get();
        let controller_generation_for_hook = controller_generation;
        let second_process_ref_for_hook = second_process_ref.clone();
        let second_provider_ref_for_hook = second_provider_ref.clone();
        let after_snapshot = Arc::new(
            move |_providers: &crate::process_provider_runtime::ProductionProcessProviders| {
                if hook_ran_for_hook.swap(true, Ordering::AcqRel) {
                    second_snapshot_seen_for_hook.store(true, Ordering::Release);
                    while !second_snapshot_release_for_hook.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                    return;
                }
                let late_peer = attach_ready_controller(
                    &providers_for_hook,
                    &zone_for_hook,
                    second_process_ref_for_hook.to_canonical_string().as_str(),
                    "323e4567-e89b-42d3-a456-426614174002",
                    1,
                    second_provider_ref_for_hook.to_canonical_string().as_str(),
                    "423e4567-e89b-42d3-a456-426614174003",
                    1,
                    controller_generation_for_hook,
                );
                let late_context = providers_for_hook
                    .controller_bootstrap_contexts(&zone_for_hook)
                    .into_iter()
                    .find(|context| context.process_ref() == &second_process_ref_for_hook)
                    .expect("late Pending controller context");
                *stale_context_for_hook.lock().unwrap() = Some(late_context);
                let replacement_peer = attach_ready_controller(
                    &providers_for_hook,
                    &zone_for_hook,
                    second_process_ref_for_hook.to_canonical_string().as_str(),
                    replacement_process_uid.as_str(),
                    replacement_process_generation,
                    second_provider_ref_for_hook.to_canonical_string().as_str(),
                    replacement_provider_uid.as_str(),
                    replacement_provider_generation,
                    controller_generation_for_hook,
                );
                peers_for_hook
                    .lock()
                    .unwrap()
                    .extend([late_peer, replacement_peer]);
            },
        );
        let policy_snapshots = Arc::new(Mutex::new(Vec::<BTreeSet<BoundSubject>>::new()));
        let policy_snapshots_for_hook = Arc::clone(&policy_snapshots);
        let first_policy_seen = Arc::new(AtomicBool::new(false));
        let first_policy_seen_for_hook = Arc::clone(&first_policy_seen);
        let first_policy_release = Arc::new(AtomicBool::new(false));
        let first_policy_release_for_hook = Arc::clone(&first_policy_release);
        let _admission_unwind_guard = ControllerSessionAdmissionTestUnwindGuard {
            first_policy_release: Arc::clone(&first_policy_release),
            second_snapshot_release: Arc::clone(&second_snapshot_release),
            shutdown: Arc::clone(&shutdown),
            wake: Arc::clone(&wake),
        };
        coordinator.set_controller_session_admission_test_seam(
            ControllerSessionAdmissionTestSeam {
                after_snapshot,
                after_policy_install: Arc::new(move |subjects| {
                    policy_snapshots_for_hook
                        .lock()
                        .unwrap()
                        .push(subjects.clone());
                    if !first_policy_seen_for_hook.swap(true, Ordering::AcqRel) {
                        while !first_policy_release_for_hook.load(Ordering::Acquire) {
                            std::thread::yield_now();
                        }
                    }
                }),
                admit_without_transport: true,
            },
        );
        peers.lock().unwrap().push(attach_ready_controller(
            &providers,
            &zone,
            first_process_ref.to_canonical_string().as_str(),
            first_process.uid.as_str(),
            first_process.generation.get(),
            first_provider_ref.to_canonical_string().as_str(),
            first_provider.uid.as_str(),
            first_provider.generation.get(),
            controller_generation,
        ));

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if policy_snapshots.lock().unwrap().len() == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the first scheduled pass must install its frozen policy");
        let stale_context = stale_context
            .lock()
            .unwrap()
            .clone()
            .expect("test seam captured the replaced context");
        let replacement_context = providers
            .controller_bootstrap_contexts(&zone)
            .into_iter()
            .find(|context| context.process_ref() == &second_process_ref)
            .expect("replacement Pending controller context");
        let first_subject = BoundSubject {
            subject_ref: first_provider_ref.clone(),
            subject_uid: first_provider.uid.clone(),
        };
        let second_subject = BoundSubject {
            subject_ref: second_provider_ref.clone(),
            subject_uid: second_provider.uid.clone(),
        };
        assert_eq!(
            policy_snapshots.lock().unwrap().as_slice(),
            [BTreeSet::from([first_subject.clone()])]
        );
        first_policy_release.store(true, Ordering::Release);

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if second_snapshot_seen.load(Ordering::Acquire)
                    && !providers.controller_bootstrap_ready(&zone, &first_process_ref)
                    && providers.controller_bootstrap_ready(&zone, replacement_context.process_ref())
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the queued wake must reach a second frozen admission pass");
        assert_ne!(
            stale_context.process_uid(),
            replacement_context.process_uid(),
            "replacement must fence the stale Process UID"
        );
        assert_ne!(
            stale_context.generation(),
            replacement_context.generation(),
            "replacement must fence the stale Process generation"
        );
        assert_ne!(
            stale_context.provider_uid(),
            replacement_context.provider_uid(),
            "replacement must fence the stale Provider UID"
        );
        assert_ne!(
            stale_context.provider_generation(),
            replacement_context.provider_generation(),
            "replacement must fence the stale Provider generation"
        );
        assert!(
            providers
                .begin_controller_bootstrap_if_matches(&zone, &stale_context)
                .is_none(),
            "a replaced Pending marker must not be claimed by a stale snapshot"
        );
        assert!(
            providers.controller_bootstrap_ready(&zone, replacement_context.process_ref()),
            "the replacement must remain Pending after pass one"
        );
        assert!(
            !providers.controller_bootstrap_ready(&zone, &first_process_ref),
            "the original context must be Active after pass one"
        );
        second_snapshot_release.store(true, Ordering::Release);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if policy_snapshots.lock().unwrap().len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the second pass must install the replacement Provider subject");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(
            coordinator.reconcile_attempts.load(Ordering::SeqCst),
            2,
            "the non-lossy wake must not create a duplicate pass"
        );
        assert_eq!(
            policy_snapshots.lock().unwrap().as_slice(),
            [
                BTreeSet::from([first_subject.clone()]),
                BTreeSet::from([first_subject.clone(), second_subject.clone()]),
            ]
        );
        for context in providers.controller_bootstrap_contexts(&zone) {
            assert!(!providers.controller_bootstrap_ready(&zone, context.process_ref()));
            assert!(
                providers
                    .begin_controller_bootstrap_if_matches(&zone, &context)
                    .is_none(),
                "an active controller must not be admitted twice"
            );
        }
        assert_eq!(wake_count.load(Ordering::SeqCst), 3);

        shutdown.store(true, Ordering::Release);
        wake.notify_one();
        drop(peers);
        drop(coordinator);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn controller_session_reconcile_failures_use_bounded_backoff() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let coordinator = runtime.controller_session_coordinator();
        let providers = test_controller_session_providers();
        let task_slot = Arc::clone(&runtime.controller_session_reconcile_task);
        let wake = Arc::clone(&runtime.controller_session_reconcile_wake);
        let shutdown = Arc::clone(&runtime.controller_session_reconcile_shutdown);
        let sessions = Arc::clone(&runtime.controller_sessions);
        std::thread::spawn(move || {
            let _guard = sessions.lock().unwrap();
            panic!("poison controller-session test lock");
        })
        .join()
        .expect_err("test thread must poison the controller-session lock");

        schedule_controller_session_reconcile(
            task_slot,
            wake,
            Arc::clone(&shutdown),
            Arc::clone(&coordinator),
            providers,
        )
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if coordinator.reconcile_attempts.load(Ordering::SeqCst) == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("failed reconciliation should be attempted");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            coordinator.reconcile_attempts.load(Ordering::SeqCst),
            1,
            "a failed reconciliation must not self-notify into a hot loop"
        );
        tokio::time::timeout(
            std::time::Duration::from_millis(500),
            async {
                loop {
                    if coordinator.reconcile_attempts.load(Ordering::SeqCst) >= 2 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            },
        )
        .await
        .expect("failed reconciliation should retry after bounded backoff");

        runtime.controller_sessions.clear_poison();
        shutdown.store(true, Ordering::Release);
        drop(coordinator);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn system_core_refresh_rejects_partial_session_state() {
        let fixture = PublicationStoreFixture::new().await;
        let bundle =
            publication_bundle(&fixture.zone, fixture.identity.zone_uid(), "session-partial");
        let runtime = fixture.open(&bundle).await;
        let state = AuthorizationState {
            snapshot: runtime.store_metadata.policy_snapshot,
            zone_policy_revision: runtime.store_metadata.current_revision,
            bootstrap_phase: d2b_resource_api::authz::BootstrapPhase::Disabled,
            now_tick: 0,
        };
        *runtime.service_task.lock().unwrap() = Some(tokio::spawn(async {
            std::future::pending::<()>().await;
            Ok(())
        }));

        assert_eq!(
            runtime.refresh_system_core_session(state).await,
            Err(ResourceRuntimeError::AuthenticationUnavailable)
        );
        assert!(runtime.service_task.lock().unwrap().is_some());
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn system_core_refresh_rejects_empty_existing_session_state() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let state = runtime.policy_projection.installed_state().unwrap();
        let registrar = runtime
            .registrar
            .lock()
            .unwrap()
            .take()
            .expect("system-core registrar");
        *runtime.ingress.lock().unwrap() = None;
        let service_task = runtime
            .service_task
            .lock()
            .unwrap()
            .take()
            .expect("system-core session task");
        service_task.abort();

        assert_eq!(
            runtime.refresh_system_core_session(state).await,
            Err(ResourceRuntimeError::AuthenticationUnavailable)
        );
        *runtime.registrar.lock().unwrap() = Some(registrar);
        let _ = service_task.await;
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn generation_publication_retires_a_before_admitting_b() {
        let fixture = PublicationStoreFixture::new().await;
        let bundle_a = publication_bundle(&fixture.zone, fixture.identity.zone_uid(), "a");
        let bundle_b = publication_bundle(&fixture.zone, fixture.identity.zone_uid(), "b");

        let runtime_a = fixture.open(&bundle_a).await;
        let (set_a, generations_a) = fixture.generation_set(&bundle_a);
        publish_generation(&runtime_a, &set_a, &generations_a).await;
        assert_eq!(
            publication_state(&runtime_a, &set_a).await,
            AuthorityOperationState::Released
        );
        runtime_a.shutdown().await.unwrap();

        let runtime_b = fixture.open(&bundle_b).await;
        let (set_b, generations_b) = fixture.generation_set(&bundle_b);
        publish_generation(&runtime_b, &set_b, &generations_b).await;
        assert_eq!(
            publication_state(&runtime_b, &set_b).await,
            AuthorityOperationState::Released
        );
        runtime_b.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn generation_publication_restart_recovers_confirmed_a_idempotently() {
        let fixture = PublicationStoreFixture::new().await;
        let bundle_a = publication_bundle(&fixture.zone, fixture.identity.zone_uid(), "a");
        let runtime_a = fixture.open(&bundle_a).await;
        let (set_a, generations_a) = fixture.generation_set(&bundle_a);
        runtime_a
            .prepare_generation_publication(&set_a, &generations_a)
            .await
            .unwrap();
        let operation_id = generation_publication_operation_id(&set_a);
        let binding_digest = runtime_a.store.authority_binding_digest(set_a.as_str());
        let capability = runtime_a
            .store
            .resume_authority_operation(operation_id, &binding_digest)
            .await
            .unwrap();
        capability
            .record_effect(AuthorityOperationState::EffectConfirmed)
            .await
            .unwrap();
        drop(capability);
        runtime_a.shutdown().await.unwrap();

        let runtime_restart = fixture.open(&bundle_a).await;
        publish_generation(&runtime_restart, &set_a, &generations_a).await;
        assert_eq!(
            publication_state(&runtime_restart, &set_a).await,
            AuthorityOperationState::Released
        );
        runtime_restart.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn generation_publication_pending_and_retryable_a_fence_b() {
        for retryable in [false, true] {
            let fixture = PublicationStoreFixture::new().await;
            let bundle_a = publication_bundle(
                &fixture.zone,
                fixture.identity.zone_uid(),
                if retryable { "retryable" } else { "pending" },
            );
            let bundle_b = publication_bundle(&fixture.zone, fixture.identity.zone_uid(), "b");
            let runtime_a = fixture.open(&bundle_a).await;
            let (set_a, generations_a) = fixture.generation_set(&bundle_a);
            runtime_a
                .prepare_generation_publication(&set_a, &generations_a)
                .await
                .unwrap();
            if retryable {
                let binding_digest = runtime_a.store.authority_binding_digest(set_a.as_str());
                let capability = runtime_a
                    .store
                    .resume_authority_operation(
                        generation_publication_operation_id(&set_a),
                        &binding_digest,
                    )
                    .await
                    .unwrap();
                capability
                    .record_effect(AuthorityOperationState::EffectRetryable)
                    .await
                    .unwrap();
            }
            runtime_a.shutdown().await.unwrap();

            let runtime_b = fixture.open(&bundle_b).await;
            let (set_b, generations_b) = fixture.generation_set(&bundle_b);
            assert!(
                runtime_b
                    .prepare_generation_publication(&set_b, &generations_b)
                    .await
                    .is_err()
            );
            runtime_b.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn generation_publication_unclosed_or_unreleased_a_does_not_admit_b() {
        // A failed close leaves EffectConfirmed; a failed release leaves Closing.
        for (close_recorded, expected_state) in [
            (false, AuthorityOperationState::EffectConfirmed),
            (true, AuthorityOperationState::Closing),
        ] {
            let fixture = PublicationStoreFixture::new().await;
            let bundle_a = publication_bundle(
                &fixture.zone,
                fixture.identity.zone_uid(),
                if close_recorded {
                    "closing"
                } else {
                    "confirmed"
                },
            );
            let bundle_b = publication_bundle(&fixture.zone, fixture.identity.zone_uid(), "b");
            let runtime_a = fixture.open(&bundle_a).await;
            let (set_a, generations_a) = fixture.generation_set(&bundle_a);
            runtime_a
                .prepare_generation_publication(&set_a, &generations_a)
                .await
                .unwrap();
            let binding_digest = runtime_a.store.authority_binding_digest(set_a.as_str());
            let capability = runtime_a
                .store
                .resume_authority_operation(
                    generation_publication_operation_id(&set_a),
                    &binding_digest,
                )
                .await
                .unwrap();
            capability
                .record_effect(AuthorityOperationState::EffectConfirmed)
                .await
                .unwrap();
            if close_recorded {
                capability.record_close().await.unwrap();
            }
            drop(capability);
            assert_eq!(publication_state(&runtime_a, &set_a).await, expected_state);
            runtime_a.shutdown().await.unwrap();

            let runtime_b = fixture.open(&bundle_b).await;
            let (set_b, generations_b) = fixture.generation_set(&bundle_b);
            assert!(
                runtime_b
                    .prepare_generation_publication(&set_b, &generations_b)
                    .await
                    .is_err()
            );
            runtime_b.shutdown().await.unwrap();
        }
    }

    fn committed_provider_resource(name: &str, artifact_id: &str, config: Value) -> StoredResource {
        let zone = ZoneId::parse("work").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let envelope = json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Provider",
            "metadata": {
                "name": name,
                "zone": zone.as_str(),
                "uid": uid.as_str(),
                "generation": 1,
                "revision": 1,
                "ownerRef": null,
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "2026-07-22T00:00:00.000Z",
                "updatedAt": "2026-07-22T00:00:00.000Z",
                "managedBy": "configuration",
                "configurationGeneration": 1,
            },
            "spec": {
                "artifactId": artifact_id,
                "config": config,
            },
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
                    "observedGeneration": 0,
                    "lastAssessedAt": null,
                    "observedGeneration": 0,
                    "operationId": null,
                    "owned": {"count": 0, "refs": []},
                    "preserveState": true,
                    "reasons": [],
                    "state": "Unknown",
                    "targetGeneration": 1,
                },
            },
        });
        let canonical_json = d2b_contracts_resource::v3::canonical_json_bytes(&envelope).unwrap();
        let parsed = ResourceEnvelope::from_json(&canonical_json).unwrap();
        StoredResource {
            resource_ref: ResourceRef::parse(&format!("Provider/{name}")).unwrap(),
            zone,
            uid,
            owner_uid: None,
            owner_generation: None,
            generation: ResourceGeneration::new(1).unwrap(),
            revision: ZoneRevision::new(1),
            canonical_json,
            payload_digest: parsed.digest().unwrap(),
        }
    }

    fn clipboard_provider_config() -> Value {
        json!({
            "controllerExecutionRef": "Host/host-system",
            "hostExecutionRef": "Host/host-system",
            "hostUserRef": "User/alice",
            "displayWaylandRef": "Provider/display-wayland",
            "guestSources": [{"guestRef": "Guest/workstation"}],
        })
    }

    fn notification_provider_config() -> Value {
        json!({
            "controllerExecutionRef": "Host/host-system",
            "hostExecutionRef": "Host/host-system",
            "hostUserRef": "User/alice",
            "displayWaylandRef": "Provider/display-wayland",
            "guestSources": [{
                "guestRef": "Guest/workstation",
                "categories": ["system.info"],
            }],
        })
    }

    #[test]
    fn committed_interaction_provider_configuration_requires_integrity_bound_typed_rows() {
        let zone = ZoneId::parse("work").unwrap();
        let clipboard = committed_provider_resource(
            "clipboard-wayland",
            "clipboard-wayland",
            clipboard_provider_config(),
        );
        let notification = committed_provider_resource(
            "notification-desktop",
            "notification-desktop",
            notification_provider_config(),
        );
        let clipboard =
            parse_committed_clipboard_configuration(&zone, ZoneRevision::new(1), &clipboard)
                .expect("clipboard configuration is accepted");
        let notification =
            parse_committed_notification_configuration(&zone, ZoneRevision::new(1), &notification)
                .expect("notification configuration is accepted");
        let configuration = CommittedInteractionProviderConfiguration {
            clipboard: Some(clipboard),
            notification: Some(notification),
        };

        assert!(configuration.is_complete());
        assert!(
            CommittedInteractionProviderConfiguration {
                clipboard: configuration.clipboard().cloned(),
                notification: None,
            }
            .is_complete()
        );
        assert!(
            CommittedInteractionProviderConfiguration {
                clipboard: None,
                notification: configuration.notification().cloned(),
            }
            .is_complete()
        );
        assert!(
            configuration
                .clipboard()
                .unwrap()
                .allows_guest_source(&ResourceRef::parse("Guest/workstation").unwrap())
        );
        assert_eq!(
            configuration
                .notification()
                .unwrap()
                .config()
                .max_pending_notifications(),
            64
        );
        assert_eq!(
            configuration.notification().unwrap().observer_user_ref(),
            &ResourceRef::parse("User/alice").unwrap()
        );

        let mut mismatched = clipboard_provider_config();
        mismatched["controllerExecutionRef"] = json!("Host/other");
        let mismatched =
            committed_provider_resource("clipboard-wayland", "clipboard-wayland", mismatched);
        assert!(matches!(
            parse_committed_clipboard_configuration(&zone, ZoneRevision::new(1), &mismatched,),
            Err(ResourceRuntimeError::InteractionConfigurationUnavailable)
        ));
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
    fn committed_interaction_provider_configuration_rejects_tampered_or_invalid_rows() {
        let zone = ZoneId::parse("work").unwrap();
        let mut tampered = committed_provider_resource(
            "clipboard-wayland",
            "clipboard-wayland",
            clipboard_provider_config(),
        );
        tampered.payload_digest = "sha256:tampered".to_owned();
        assert!(matches!(
            parse_committed_clipboard_configuration(&zone, ZoneRevision::new(1), &tampered),
            Err(ResourceRuntimeError::InteractionConfigurationUnavailable)
        ));

        let invalid_guest_source = committed_provider_resource(
            "notification-desktop",
            "notification-desktop",
            json!({
                "controllerExecutionRef": "Host/host-system",
                "hostExecutionRef": "Host/host-system",
                "hostUserRef": "User/alice",
                "displayWaylandRef": "Provider/display-wayland",
                "guestSources": [{
                    "guestRef": "Host/host-system",
                    "categories": ["system.info"],
                }],
            }),
        );
        assert!(matches!(
            parse_committed_notification_configuration(
                &zone,
                ZoneRevision::new(1),
                &invalid_guest_source,
            ),
            Err(ResourceRuntimeError::InteractionConfigurationUnavailable)
        ));
    }

    #[test]
    fn controller_provider_identity_projection_uses_authoritative_uid_generation_and_revision() {
        let zone = ZoneId::parse("work").unwrap();
        let expected_ref = ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap();
        let resource = committed_provider_resource(
            "runtime-cloud-hypervisor",
            "runtime-cloud-hypervisor",
            json!({}),
        );
        let (_, uid, generation, revision, digest) =
            committed_provider_spec(&zone, ZoneRevision::new(1), &resource, &expected_ref)
                .expect("committed Provider identity");
        assert_eq!(uid, resource.uid);
        assert_eq!(generation, resource.generation);
        assert_eq!(revision, resource.revision);
        assert_eq!(digest, resource.payload_digest);

        let mut future = resource.clone();
        future.revision = ZoneRevision::new(2);
        assert!(matches!(
            committed_provider_spec(&zone, ZoneRevision::new(1), &future, &expected_ref),
            Err(ResourceRuntimeError::InteractionConfigurationUnavailable)
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
    fn transient_controller_session_get_errors_do_not_abort_sibling_sessions() {
        for kind in [
            StoreErrorKind::ResourceConflict,
            StoreErrorKind::RevisionExpired,
            StoreErrorKind::Backpressure,
            StoreErrorKind::StoreBackpressure,
            StoreErrorKind::Timeout,
            StoreErrorKind::Cancelled,
            StoreErrorKind::ResourcePlaneUnavailable,
        ] {
            let error = ControllerSessionCoordinator::controller_session_evidence_read_error(kind);
            assert!(
                ControllerSessionCoordinator::controller_session_evidence_error_is_context_local(
                    &error
                ),
                "target-local {kind:?} must requeue only its own Provider session"
            );
        }
        assert!(
            !ControllerSessionCoordinator::controller_session_evidence_error_is_context_local(
                &ResourceRuntimeError::StoreReadFailed
            ),
            "global store integrity failures must still propagate"
        );
        assert_eq!(
            ControllerSessionCoordinator::controller_session_evidence_read_error(
                StoreErrorKind::StoreIntegrityFailure
            ),
            ResourceRuntimeError::StoreReadFailed
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

    #[tokio::test(flavor = "current_thread")]
    async fn post_teardown_controller_session_clear_does_not_fence_siblings() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let zone = runtime.zone.clone();
        let provider_ref = ResourceRef::parse("Provider/system-core").unwrap();
        let process_ref = ResourceRef::parse("Process/d2b-core-controller").unwrap();
        materialize_test_bundle(&runtime, Vec::new()).await;
        let process = BundleResource::new(
            ResourceTypeName::parse("Process").unwrap(),
            BundleResourceMetadata::new(
                ResourceName::parse("d2b-core-controller").unwrap(),
                zone.clone(),
                Some(provider_ref.clone()),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(
                br#"{"executionRef":"Host/host-system","processClass":"controller","providerRef":"Provider/system-minijail","template":"test-controller"}"#,
            )
            .unwrap(),
        )
        .unwrap();
        materialize_test_bundle(&runtime, vec![process]).await;
        let provider =
            read_test_resource(&runtime, provider_ref.clone(), "sibling-fence-provider").await;
        let process =
            read_test_resource(&runtime, process_ref.clone(), "sibling-fence-process").await;
        let controller_generation = runtime
            .store
            .runtime_metadata()
            .await
            .unwrap()
            .policy_snapshot
            .controller_generation
            .expect("test policy has a controller generation");
        let providers = test_controller_session_providers();
        providers
            .attach_controller_provider_context_for_test(
                zone.clone(),
                process_ref.clone(),
                process.uid.clone(),
                process.generation,
                ResourceRef::parse("Provider/system-minijail").unwrap(),
                provider_ref.clone(),
                provider.uid.clone(),
                provider.generation,
                ResourceRef::parse("Host/host-system").unwrap(),
                controller_generation,
            )
            .unwrap();
        let first_context = providers
            .controller_bootstrap_contexts(&zone)
            .into_iter()
            .find(|context| context.process_ref() == &process_ref)
            .expect("fenced controller context");
        providers
            .set_controller_session_waker(zone.clone(), Arc::new(|| Ok(())))
            .unwrap();
        let sibling_process_ref = ResourceRef::parse("Process/sibling-controller").unwrap();
        let sibling_peer = attach_ready_controller(
            &providers,
            &zone,
            sibling_process_ref.to_canonical_string().as_str(),
            "523e4567-e89b-42d3-a456-426614174004",
            1,
            provider_ref.to_canonical_string().as_str(),
            provider.uid.as_str(),
            provider.generation.get(),
            controller_generation,
        );
        let sibling_context = providers
            .controller_bootstrap_contexts(&zone)
            .into_iter()
            .find(|context| context.process_ref() == &sibling_process_ref)
            .expect("eligible sibling controller context");
        let mut coordinator = runtime.build_controller_session_coordinator().unwrap();
        let authority = runtime.core_assignment_fences().await.unwrap().4;
        let authorization_state = runtime
            .authorization_state
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        coordinator
            .set_assigned_process_api(Arc::new(
                runtime
                    .process_controller_api(DaemonMode::Host, authority, authorization_state)
                    .unwrap(),
            ))
            .unwrap();
        insert_test_controller_session(&mut coordinator, &first_context).await;
        providers.fail_controller_bootstrap(&first_context);
        coordinator.set_controller_session_evidence_test_errors(vec![
            ResourceRuntimeError::ResourceGetFailed(ResourceErrorKind::Timeout),
            ResourceRuntimeError::ResourceStatusUpdateFailed(ResourceErrorKind::ResourceConflict),
        ]);

        assert_eq!(coordinator.fence(&providers).await, Ok(()));
        assert!(
            !coordinator
                .controller_sessions
                .lock()
                .unwrap()
                .contains_key(&process_ref),
            "post-teardown evidence failure must not reinsert the dead session"
        );
        assert!(
            coordinator
                .pending_controller_session_clears
                .lock()
                .unwrap()
                .contains_key(&process_ref),
            "durable evidence clear must remain queued for retry"
        );
        assert!(
            providers.controller_bootstrap_ready(&zone, sibling_context.process_ref()),
            "a context-local clear failure must not abort an eligible sibling"
        );

        assert_eq!(coordinator.fence(&providers).await, Ok(()));
        assert!(
            !coordinator
                .pending_controller_session_clears
                .lock()
                .unwrap()
                .contains_key(&process_ref),
            "the next fence must retry and retire the queued evidence clear"
        );
        let cleared = read_test_resource(
            &runtime,
            process_ref.clone(),
            "sibling-fence-cleared-evidence",
        )
        .await;
        assert!(
            serde_json::from_slice::<serde_json::Value>(&cleared.canonical_json)
                .unwrap()
                .pointer("/status/resource/controllerSession")
                .is_none(),
            "retry must clear durable evidence without restoring transport authority"
        );
        assert!(providers.controller_bootstrap_ready(
            &zone,
            sibling_context.process_ref()
        ));

        drop(sibling_peer);
        drop(coordinator);
        runtime.shutdown().await.unwrap();
    }

    /// One manager-served `Process` row for the G5 bridge tests.
    struct ControllerPlaneRowFixture {
        view: Option<ResourceView>,
    }

    #[async_trait]
    impl ControllerPlaneView for ControllerPlaneRowFixture {
        async fn process_view(
            &self,
            _process_ref: &ResourceRef,
        ) -> Result<Option<ResourceView>, d2b_resource_runtime::error::ResourceError> {
            Ok(self.view.clone())
        }
    }

    fn plane_uid_bytes(uid: &ResourceUid) -> [u8; 16] {
        let hex = uid.as_str().replace('-', "");
        let mut bytes = [0u8; 16];
        for (index, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
        }
        bytes
    }

    /// The manager view of the controller row one bootstrap context describes.
    fn manager_process_view(
        context: &crate::process_provider_runtime::ControllerBootstrapContext,
    ) -> ResourceView {
        ResourceView {
            key: d2b_resource_runtime::identity::ResourceKey::new(
                context.zone().as_str(),
                "Process",
                context.process_ref().name().as_str(),
            ),
            uid: plane_uid_bytes(context.process_uid()),
            generation: context.generation().get(),
            deleting: false,
            provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
            spec: serde_json::to_vec(&json!({
                "providerRef": context.process_provider_ref().to_canonical_string(),
                "processClass": "controller",
                "template": "test-controller",
                "executionRef": context.execution_ref().to_canonical_string(),
            }))
            .unwrap(),
            metadata: serde_json::to_vec(&json!({
                "ownerRef": context.provider_owner_ref().to_canonical_string(),
                "labels": {},
                "annotations": {},
            }))
            .unwrap(),
            owner_key: None,
            status: Some(d2b_resource_runtime::resource::ResourceStatus::Ready),
            status_generation: Some(context.generation().get()),
        }
    }

    /// A `Provider/system-core`-owned controller context whose Process row is
    /// only ever minted in the manager (never materialized in the store).
    async fn manager_served_controller_fixture(
        runtime: &ZoneResourceRuntime,
    ) -> (
        Arc<crate::process_provider_runtime::ProductionProcessProviders>,
        ResourceRef,
        crate::process_provider_runtime::ControllerBootstrapContext,
    ) {
        let zone = runtime.zone.clone();
        let provider_ref = ResourceRef::parse("Provider/system-core").unwrap();
        let process_ref = ResourceRef::parse("Process/manager-controller").unwrap();
        materialize_test_bundle(runtime, Vec::new()).await;
        let provider = read_test_resource(runtime, provider_ref.clone(), "g5-provider").await;
        let controller_generation = runtime
            .store
            .runtime_metadata()
            .await
            .unwrap()
            .policy_snapshot
            .controller_generation
            .expect("test policy has a controller generation");
        let providers = test_controller_session_providers();
        providers
            .attach_controller_provider_context_for_test(
                zone.clone(),
                process_ref.clone(),
                ResourceUid::parse("11111111-1111-4111-8111-111111111111").unwrap(),
                ResourceGeneration::new(3).unwrap(),
                ResourceRef::parse("Provider/system-minijail").unwrap(),
                provider_ref.clone(),
                provider.uid.clone(),
                provider.generation,
                ResourceRef::parse("Host/host-system").unwrap(),
                controller_generation,
            )
            .unwrap();
        let context = providers
            .controller_bootstrap_contexts(&zone)
            .into_iter()
            .find(|context| context.process_ref() == &process_ref)
            .expect("manager-served controller context");
        (providers, process_ref, context)
    }

    /// G5: a controller Process row minted in the new plane is visible to the
    /// session-evidence path through the manager's view.
    #[tokio::test(flavor = "current_thread")]
    async fn manager_served_controller_row_is_current_and_evidence_is_live() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let (providers, process_ref, context) =
            manager_served_controller_fixture(&runtime).await;
        let coordinator = runtime.controller_session_coordinator();

        assert!(
            !coordinator
                .controller_context_is_current(&providers, &context)
                .await
                .unwrap(),
            "the durable store does not serve the manager row"
        );

        coordinator.attach_plane_view(Arc::new(ControllerPlaneRowFixture {
            view: Some(manager_process_view(&context)),
        }));
        assert!(
            coordinator
                .controller_context_is_current(&providers, &context)
                .await
                .unwrap(),
            "the manager's row is the authority for a converted Process row"
        );
        coordinator
            .persist_controller_session_evidence(
                &context,
                Some(ReconnectGeneration::new(1).unwrap()),
            )
            .await
            .expect("manager-served evidence is the live session, not a durable write");

        // A manager row for another identity is not this context's row.
        let mut mismatched = manager_process_view(&context);
        mismatched.generation = context.generation().get() + 1;
        coordinator.attach_plane_view(Arc::new(ControllerPlaneRowFixture {
            view: Some(mismatched),
        }));
        assert!(
            !coordinator
                .controller_context_is_current(&providers, &context)
                .await
                .unwrap()
        );
        assert_eq!(
            coordinator
                .persist_controller_session_evidence(
                    &context,
                    Some(ReconnectGeneration::new(1).unwrap()),
                )
                .await,
            Err(ResourceRuntimeError::IdentityUnbound),
            "evidence for a row this context does not describe still refuses"
        );

        assert_eq!(context.process_ref(), &process_ref);

        drop(coordinator);
        runtime.shutdown().await.unwrap();
    }

    /// G5: a row still served by the durable plane keeps the old path, with
    /// or without a manager that holds no such row.
    #[tokio::test(flavor = "current_thread")]
    async fn durable_controller_row_still_resolves_without_a_manager_row() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let zone = runtime.zone.clone();
        let provider_ref = ResourceRef::parse("Provider/system-core").unwrap();
        let process_ref = ResourceRef::parse("Process/durable-controller").unwrap();
        materialize_test_bundle(&runtime, Vec::new()).await;
        let process = BundleResource::new(
            ResourceTypeName::parse("Process").unwrap(),
            BundleResourceMetadata::new(
                ResourceName::parse("durable-controller").unwrap(),
                zone.clone(),
                Some(provider_ref.clone()),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(
                br#"{"executionRef":"Host/host-system","processClass":"controller","providerRef":"Provider/system-minijail","template":"test-controller"}"#,
            )
            .unwrap(),
        )
        .unwrap();
        materialize_test_bundle(&runtime, vec![process]).await;
        let provider = read_test_resource(&runtime, provider_ref.clone(), "g5-durable-provider").await;
        let process = read_test_resource(&runtime, process_ref.clone(), "g5-durable-process").await;
        let controller_generation = runtime
            .store
            .runtime_metadata()
            .await
            .unwrap()
            .policy_snapshot
            .controller_generation
            .expect("test policy has a controller generation");
        let providers = test_controller_session_providers();
        providers
            .attach_controller_provider_context_for_test(
                zone.clone(),
                process_ref.clone(),
                process.uid.clone(),
                process.generation,
                ResourceRef::parse("Provider/system-minijail").unwrap(),
                provider_ref.clone(),
                provider.uid.clone(),
                provider.generation,
                ResourceRef::parse("Host/host-system").unwrap(),
                controller_generation,
            )
            .unwrap();
        let context = providers
            .controller_bootstrap_contexts(&zone)
            .into_iter()
            .find(|context| context.process_ref() == &process_ref)
            .expect("durable controller context");
        let coordinator = runtime.controller_session_coordinator();

        assert!(
            coordinator
                .controller_context_is_current(&providers, &context)
                .await
                .unwrap(),
            "the durable row still resolves through the old path"
        );
        coordinator.attach_plane_view(Arc::new(ControllerPlaneRowFixture { view: None }));
        assert!(
            coordinator
                .controller_context_is_current(&providers, &context)
                .await
                .unwrap(),
            "a manager that does not serve the row keeps the durable path authoritative"
        );

        drop(coordinator);
        runtime.shutdown().await.unwrap();
    }

    /// G5: the session fence does not retire a live session whose row the
    /// durable list cannot see but the manager does.
    #[tokio::test(flavor = "current_thread")]
    async fn session_fence_consults_the_manager_for_manager_served_rows() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let (providers, process_ref, context) =
            manager_served_controller_fixture(&runtime).await;
        providers
            .set_controller_session_waker(runtime.zone.clone(), Arc::new(|| Ok(())))
            .unwrap();
        let mut coordinator = runtime.build_controller_session_coordinator().unwrap();

        insert_test_controller_session(&mut coordinator, &context).await;
        coordinator
            .fence_process_resources(&providers, &[])
            .await
            .unwrap();
        assert!(
            !coordinator
                .controller_sessions
                .lock()
                .unwrap()
                .contains_key(&process_ref),
            "without a manager row the durable fence still retires the session"
        );

        insert_test_controller_session(&mut coordinator, &context).await;
        coordinator.attach_plane_view(Arc::new(ControllerPlaneRowFixture {
            view: Some(manager_process_view(&context)),
        }));
        coordinator
            .fence_process_resources(&providers, &[])
            .await
            .unwrap();
        assert!(
            coordinator
                .controller_sessions
                .lock()
                .unwrap()
                .contains_key(&process_ref),
            "the manager's row keeps the session out of the durable fence"
        );

        drop(coordinator);
        runtime.shutdown().await.unwrap();
    }

    /// G5: the synthesized session evidence is the live session, re-read per
    /// evaluation, and every uncertainty answers `None`.
    #[tokio::test(flavor = "current_thread")]
    async fn live_session_evidence_is_generation_and_liveness_bound() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let (_providers, _process_ref, context) =
            manager_served_controller_fixture(&runtime).await;
        let mut coordinator = runtime.build_controller_session_coordinator().unwrap();
        insert_test_controller_session(&mut coordinator, &context).await;
        {
            let mut sessions = coordinator.controller_sessions.lock().unwrap();
            let session = sessions.get_mut(context.process_ref()).unwrap();
            session.service_task = tokio::spawn(async {
                std::future::pending::<()>().await;
                Ok::<(), SessionServerError>(())
            });
        }

        let evidence = LiveControllerSessionEvidence::controller_session_evidence(
            &coordinator,
            context.process_ref(),
            context.process_uid(),
            context.generation(),
        )
        .expect("a live admitted session is evidence");
        assert_eq!(evidence["ready"], json!(true));
        assert_eq!(evidence["sessionGeneration"], json!(1));
        assert_eq!(
            evidence["processUid"].as_str(),
            Some(context.process_uid().as_str())
        );

        assert!(
            LiveControllerSessionEvidence::controller_session_evidence(
                &coordinator,
                context.process_ref(),
                context.process_uid(),
                ResourceGeneration::new(context.generation().get() + 1).unwrap(),
            )
            .is_none(),
            "another generation is not this row's evidence"
        );
        assert!(
            LiveControllerSessionEvidence::controller_session_evidence(
                &coordinator,
                context.process_ref(),
                &ResourceUid::parse("99999999-9999-4999-8999-999999999999").unwrap(),
                context.generation(),
            )
            .is_none(),
            "another row identity is not this row's evidence"
        );

        {
            let mut sessions = coordinator.controller_sessions.lock().unwrap();
            let session = sessions.get_mut(context.process_ref()).unwrap();
            session.service_task = tokio::spawn(async { Ok::<(), SessionServerError>(()) });
        }
        tokio::task::yield_now().await;
        assert!(
            LiveControllerSessionEvidence::controller_session_evidence(
                &coordinator,
                context.process_ref(),
                context.process_uid(),
                context.generation(),
            )
            .is_none(),
            "a finished service task is not live evidence"
        );

        drop(coordinator);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_controller_session_clear_retains_live_authority() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let zone = runtime.zone.clone();
        let provider_ref = ResourceRef::parse("Provider/system-core").unwrap();
        let process_ref = ResourceRef::parse("Process/d2b-core-controller").unwrap();
        materialize_test_bundle(&runtime, Vec::new()).await;
        let process = BundleResource::new(
            ResourceTypeName::parse("Process").unwrap(),
            BundleResourceMetadata::new(
                ResourceName::parse("d2b-core-controller").unwrap(),
                zone.clone(),
                Some(provider_ref.clone()),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(
                br#"{"executionRef":"Host/host-system","processClass":"controller","providerRef":"Provider/system-minijail","template":"test-controller"}"#,
            )
            .unwrap(),
        )
        .unwrap();
        materialize_test_bundle(&runtime, vec![process]).await;
        let provider = read_test_resource(
            &runtime,
            provider_ref.clone(),
            "failed-session-clear-provider",
        )
        .await;
        let process = read_test_resource(
            &runtime,
            process_ref.clone(),
            "failed-session-clear-process",
        )
        .await;
        let controller_generation = runtime
            .store
            .runtime_metadata()
            .await
            .unwrap()
            .policy_snapshot
            .controller_generation
            .expect("test policy has a controller generation");
        let providers = test_controller_session_providers();
        providers
            .attach_controller_provider_context_for_test(
                zone.clone(),
                process_ref.clone(),
                process.uid.clone(),
                process.generation,
                ResourceRef::parse("Provider/system-minijail").unwrap(),
                provider_ref.clone(),
                provider.uid.clone(),
                provider.generation,
                ResourceRef::parse("Host/host-system").unwrap(),
                controller_generation,
            )
            .unwrap();
        let context = providers
            .controller_bootstrap_contexts(&zone)
            .into_iter()
            .find(|context| context.process_ref() == &process_ref)
            .expect("test controller context");
        assert!(controller_resource_matches(&context, &process));
        let mut legacy_process = process.clone();
        legacy_process.owner_generation = None;
        assert!(
            controller_resource_matches(&context, &legacy_process),
            "legacy Process rows without owner generation must still accept session evidence"
        );
        let mut recreated_owner = process.clone();
        recreated_owner.owner_uid =
            Some(ResourceUid::parse("44444444-4444-4444-8444-444444444444").unwrap());
        assert!(
            !controller_resource_matches(&context, &recreated_owner),
            "a Process row recycled under the Provider name must not satisfy the new Provider UID"
        );
        assert_eq!(
            controller_session_resource_fences(
                vec![(process_ref.clone(), context.clone())],
                &[recreated_owner.clone()],
            )
            .len(),
            1,
            "session fencing must reject a same-name Process under an older Provider UID",
        );
        let mut recreated_generation = process.clone();
        recreated_generation.owner_generation = Some(
            ResourceGeneration::new(context.provider_generation().get().saturating_add(1))
                .unwrap(),
        );
        assert!(
            !controller_resource_matches(&context, &recreated_generation),
            "a Process row from an older Provider generation must not satisfy the new owner"
        );

        let mut coordinator = runtime.build_controller_session_coordinator().unwrap();
        let authority = runtime.core_assignment_fences().await.unwrap().4;
        let authorization_state = runtime
            .authorization_state
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        let assigned_process_api = Arc::new(
            runtime
                .process_controller_api(DaemonMode::Host, authority, authorization_state)
                .unwrap(),
        );
        coordinator
            .set_assigned_process_api(Arc::clone(&assigned_process_api))
            .unwrap();
        persist_resource_controller_session_evidence(
            &assigned_process_api,
            &legacy_process,
            Some(&json!({ "ready": true })),
        )
        .await
        .expect("persist legacy controller-session evidence");
        let legacy_persisted = read_test_resource(
            &runtime,
            process_ref.clone(),
            "legacy-controller-session-evidence",
        )
        .await;
        assert!(
            serde_json::from_slice::<serde_json::Value>(&legacy_persisted.canonical_json)
                .unwrap()
                .pointer("/status/resource/controllerSession")
                .is_some(),
            "legacy Process rows must persist controller-session evidence"
        );
        coordinator
            .persist_controller_session_evidence(
                &context,
                Some(ReconnectGeneration::new(1).unwrap()),
            )
            .await
            .expect("seed durable controller-session evidence");
        *coordinator
            .assigned_process_api
            .lock()
            .unwrap() = None;
        let policy = controller_resource_endpoint_policy();
        let catalog = d2b_resource_api::authz::ApiCatalog::standard();
        let role_ref = ResourceRef::parse("Role/test-controller").unwrap();
        let role = d2b_resource_api::authz::CompiledRole::new(
            role_ref.clone(),
            vec![
                d2b_resource_api::authz::PolicyRule::new(
                    &catalog,
                    [],
                    [],
                    [d2b_resource_api::authz::SessionVerb::Connect],
                    [],
                    [],
                    [zone.clone()],
                    [],
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let metadata = runtime.store.runtime_metadata().await.unwrap();
        let state = AuthorizationState {
            snapshot: metadata.policy_snapshot,
            zone_policy_revision: metadata.current_revision,
            bootstrap_phase: d2b_resource_api::authz::BootstrapPhase::Disabled,
            now_tick: 1,
        };
        let binding = d2b_resource_api::authz::CompiledRoleBinding::new(
            role_ref,
            [BoundSubject {
                subject_ref: context.provider_owner_ref().clone(),
                subject_uid: context.provider_uid().clone(),
            }],
            d2b_resource_api::authz::BindingScope {
                zones: [zone.clone()].into_iter().collect(),
                ..d2b_resource_api::authz::BindingScope::default()
            },
            d2b_resource_api::authz::RelayGrantAuthority::None,
        )
        .unwrap();
        let policy_set =
            PolicySet::new(&catalog, state.snapshot.policy_revision, vec![role], vec![binding])
                .unwrap();
        let native = NativeAuthorizer::new(catalog, Some(policy_set)).unwrap();
        let bus_authorizer = BusAuthorizer::new(native, state).unwrap();
        let (_bus, registrar) =
            ZoneBus::new(zone.clone(), bus_authorizer, BusConfig::default()).unwrap();
        coordinator.registrar = Arc::new(Mutex::new(Some(registrar)));
        let (initiator_fd, responder_fd) = prearmed_seqpacket_pair().unwrap();
        let initiator_socket = SeqpacketSocket::from_parent_prearmed(initiator_fd).unwrap();
        let responder_socket = SeqpacketSocket::from_parent_prearmed(responder_fd).unwrap();
        let verified_peer =
            VerifiedUnixPeer::verify_inherited_seqpacket(&initiator_socket).unwrap();
        let registrar = coordinator.registrar.clone();
        let mut registrar = registrar
            .lock()
            .unwrap()
            .take()
            .expect("test registrar");
        registrar
            .install_committed_controller_process_subject(
                &verified_peer,
                CommittedControllerProcessSubjectInput {
                    provider_ref: context.provider_owner_ref().clone(),
                    provider_uid: context.provider_uid().clone(),
                    process_ref: context.process_ref().clone(),
                    zone_ref: ResourceRef::parse("Zone/work").unwrap(),
                    execution_ref: context.execution_ref().clone(),
                    provider_generation: context.provider_generation(),
                    controller_generation: context.controller_generation(),
                },
            )
            .unwrap();
        let initiator = unix_transport(initiator_socket, &policy).unwrap();
        let responder = unix_transport(responder_socket, &policy).unwrap();
        let (initiator, responder) = tokio::join!(
            SessionEngine::establish_initiator(
                initiator,
                policy.clone(),
                HandshakeCredentials::Nn,
                std::time::Instant::now(),
            ),
            SessionEngine::establish_responder(
                responder,
                policy.clone(),
                HandshakeCredentials::Nn,
                std::time::Instant::now(),
            ),
        );
        let acceptor = registrar
            .component_session_acceptor(policy, verified_peer)
            .unwrap();
        let candidate = acceptor
            .admit(
                initiator.unwrap(),
                TransportEvidence::new(
                    EvidenceClass::UnixPeer,
                    BindingDigest::parse(format!("sha256:{}", "22".repeat(32))).unwrap(),
                ),
                1,
            )
            .await
            .unwrap();
        let (ingress, driver) = registrar
            .register_component_service_session(candidate)
            .await
            .unwrap();
        coordinator
            .registrar
            .lock()
            .unwrap()
            .replace(registrar);
        let binding =
            controller_session_binding(&context, ReconnectGeneration::new(1).unwrap()).unwrap();
        let service_task = tokio::spawn(async { Ok::<(), SessionServerError>(()) });
        let session = ControllerSession {
            context: context.clone(),
            binding,
            ingress,
            driver,
            _backend_lease: None,
            resource_client: None,
            service_task,
            assignments: BTreeMap::new(),
            assignment_stream_open: false,
            assignments_revoked: false,
            transport_closed: false,
            ingress_revoked: false,
        };
        coordinator
            .controller_sessions
            .lock()
            .unwrap()
            .insert(process_ref.clone(), session);
        let registrar_for_retry = coordinator.registrar.lock().unwrap().take();
        let result = coordinator
            .remove_controller_session(&process_ref, Some(&context))
            .await;
        assert_eq!(
            result,
            Err(ResourceRuntimeError::AuthenticationUnavailable)
        );
        assert!(
            coordinator
                .controller_sessions
                .lock()
                .unwrap()
                .contains_key(&process_ref),
                "failed ingress teardown must retain the live session authority"
        );
        let retained = read_test_resource(
                &runtime,
                process_ref.clone(),
                "failed-ingress-teardown-retains-evidence",
        )
        .await;
        let retained_value: serde_json::Value =
                serde_json::from_slice(&retained.canonical_json).unwrap();
        assert!(
                retained_value
                    .pointer("/status/resource/controllerSession")
                    .is_some(),
                "failed ingress teardown must not clear durable session evidence"
        );
        *coordinator.registrar.lock().unwrap() = registrar_for_retry;
        let result = coordinator
                .remove_controller_session(&process_ref, Some(&context))
                .await;
        assert_eq!(
                result,
                Err(ResourceRuntimeError::AuthenticationUnavailable)
        );
        assert!(
                coordinator
                    .controller_sessions
                    .lock()
                    .unwrap()
                    .contains_key(&process_ref),
                "failed durable clear must retain the already-torn-down session for retry"
        );
        *coordinator.assigned_process_api.lock().unwrap() = Some(assigned_process_api);
        assert_eq!(
                coordinator
                    .remove_controller_session(&process_ref, Some(&context))
                    .await,
                Ok(())
        );
        assert!(
                !coordinator
                    .controller_sessions
                    .lock()
                    .unwrap()
                    .contains_key(&process_ref),
                "successful retry must perform the final session-map removal"
        );
        drop(responder);
        drop(coordinator);
        let _ = runtime.shutdown().await;
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

    #[test]
    fn broker_response_requires_one_canonical_zone_store() {
        let response = OpenZoneStoreResponse {
            zone_store_id: d2b_contracts_resource::v3::storage::ZoneStoreId::parse(
                "zone-store-work",
            )
            .unwrap(),
            store_identity: "sha256:".to_owned() + &"a".repeat(64),
            disposition: ZoneStoreDisposition::Opened,
            fd_index: 0,
        };
        assert_eq!(response.fd_index, 0);
        assert!(response.store_identity.starts_with("sha256:"));
    }

    #[test]
    fn opened_fd_is_owned_by_the_runtime_boundary() {
        let (left, right) = nix::sys::socket::socketpair(
            nix::sys::socket::AddressFamily::Unix,
            nix::sys::socket::SockType::SeqPacket,
            None,
            nix::sys::socket::SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        assert!(left.as_raw_fd() >= 0);
        drop(right);
        drop(left);
    }

    #[tokio::test]
    async fn production_runtime_opens_and_re_adopts_the_broker_owned_store() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("store.redb");
        let marker_path = directory.path().join(".d2b-store-marker");
        let zone = ZoneId::parse("work").unwrap();
        let marker_identity = "sha256:".to_owned() + &"b".repeat(64);
        let identity = store_identity(&zone, &marker_identity).unwrap();

        let database = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&database_path)
            .unwrap();
        let mut marker = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&marker_path)
            .unwrap();
        write_provisioning_marker(&mut marker, &identity).unwrap();
        let (_, acceptor) = mutation_seal_pair(identity.seal_identity());
        let provisioned = RedbResourceStore::provision_owned_with_audit(
            database,
            marker,
            identity,
            acceptor,
            test_audit_sink(directory.path(), "audit-provision"),
        )
        .await
        .unwrap();
        provisioned.shutdown().await.unwrap();

        let database = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&database_path)
            .unwrap();
        let fd = database.as_raw_fd();
        assert!(
            rustix::io::fcntl_getfd(&database)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        let runtime = ZoneResourceRuntime::open(
            zone.clone(),
            OpenedZoneStore {
                response: OpenZoneStoreResponse {
                    zone_store_id: d2b_contracts_resource::v3::storage::ZoneStoreId::parse(
                        "zone-store-work",
                    )
                    .unwrap(),
                    store_identity: marker_identity.clone(),
                    disposition: ZoneStoreDisposition::Opened,
                    fd_index: 0,
                },
                database_fd: database.into(),
                external_inventory: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(runtime.zone(), &zone);
        assert!(runtime.readiness().store_ready);
        assert!(!runtime.readiness().resource_api_ready);
        assert!(!runtime.readiness().local_session_ready);
        assert!(!runtime.readiness().provider_path_ready);
        assert_eq!(
            runtime.core_stage().unwrap(),
            StartupStage::WaitingForResourceApi
        );
        assert_eq!(
            runtime.readiness_error(),
            Some(ResourceRuntimeError::PolicyUnavailable)
        );
        let zone_status = runtime
            .dispatch_cli_request(&json!({
                "method": "ZoneStatus",
                "zoneRef": "Zone/work",
            }))
            .await
            .unwrap();
        assert_eq!(zone_status["type"], "error");
        assert_eq!(zone_status["error"]["kind"], "authorization-denied");
        let list = runtime
            .dispatch_cli_request(&json!({
                "method": "List",
                "zoneRef": "Zone/work",
                "resourceType": "Guest",
            }))
            .await
            .unwrap();
        assert_eq!(list["type"], "error");
        assert_eq!(list["error"]["kind"], "authorization-denied");
        assert_eq!(list["error"]["retryClass"], "reauthorize");
        let watch = runtime
            .dispatch_cli_request(&json!({
                "method": "Watch",
                "zoneRef": "Zone/work",
                "resourceType": "Guest",
            }))
            .await
            .unwrap();
        assert_eq!(watch["error"]["kind"], "authorization-denied");
        let status = runtime
            .dispatch_cli_request(&json!({
                "method": "Status",
                "zoneRef": "Zone/work",
                "resourceRef": "Guest/corp-vm",
            }))
            .await
            .unwrap();
        assert_eq!(status["error"]["kind"], "authorization-denied");
        runtime.shutdown().await.unwrap();
        assert!(fd >= 0);
    }

    #[tokio::test]
    async fn production_runtime_provisions_a_broker_provisioned_store() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("store.redb");
        let database = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&database_path)
            .unwrap();
        let zone = ZoneId::parse("work").unwrap();
        let marker_identity = "sha256:".to_owned() + &"c".repeat(64);
        let runtime = ZoneResourceRuntime::open(
            zone,
            OpenedZoneStore {
                response: OpenZoneStoreResponse {
                    zone_store_id: d2b_contracts_resource::v3::storage::ZoneStoreId::parse(
                        "zone-store-work",
                    )
                    .unwrap(),
                    store_identity: marker_identity,
                    disposition: ZoneStoreDisposition::Provisioned,
                    fd_index: 0,
                },
                database_fd: database.into(),
                external_inventory: None,
            },
        )
        .await
        .unwrap();
        assert!(runtime.readiness().store_ready);
        assert!(!runtime.readiness().resource_api_ready);
        let mut plane = ResourcePlane::new();
        let owner = plane.insert(runtime).unwrap();
        assert_eq!(plane.ready_zone_count(), 0);
        assert!(plane.has_live_request_owners());
        assert_eq!(
            plane.shutdown().await,
            Err(ResourceRuntimeError::LiveRequestOwners)
        );
        assert!(plane.has_live_request_owners());
        drop(owner);
        assert!(!plane.has_live_request_owners());
        plane.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn production_runtime_rejects_immutable_store_identity_mismatch() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("store.redb");
        let marker_path = directory.path().join(".d2b-store-marker");
        let zone = ZoneId::parse("work").unwrap();
        let stored_identity = "sha256:".to_owned() + &"e".repeat(64);
        let identity = store_identity(&zone, &stored_identity).unwrap();
        let database = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&database_path)
            .unwrap();
        let mut marker = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&marker_path)
            .unwrap();
        write_provisioning_marker(&mut marker, &identity).unwrap();
        let provisioned = RedbResourceStore::provision_owned_with_audit(
            database,
            marker,
            identity,
            mutation_seal_pair(
                store_identity(&zone, &stored_identity)
                    .unwrap()
                    .seal_identity(),
            )
            .1,
            test_audit_sink(directory.path(), "audit-mismatch"),
        )
        .await
        .unwrap();
        provisioned.shutdown().await.unwrap();

        let database = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&database_path)
            .unwrap();
        let result = ZoneResourceRuntime::open(
            zone,
            OpenedZoneStore {
                response: OpenZoneStoreResponse {
                    zone_store_id: d2b_contracts_resource::v3::storage::ZoneStoreId::parse(
                        "zone-store-work",
                    )
                    .unwrap(),
                    store_identity: "sha256:".to_owned() + &"f".repeat(64),
                    disposition: ZoneStoreDisposition::Opened,
                    fd_index: 0,
                },
                database_fd: database.into(),
                external_inventory: None,
            },
        )
        .await;
        assert!(matches!(result, Err(ResourceRuntimeError::StoreOpenFailed)));
    }

    #[tokio::test]
    async fn public_reads_use_authenticated_session_after_restart_revisions_rehydrate() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("store.redb");
        let zone = ZoneId::parse("work").unwrap();
        let marker_identity = "sha256:".to_owned() + &"d".repeat(64);
        let revisions = PolicySnapshot {
            policy_revision: 7,
            api_catalog_revision: 8,
            active_configuration_revision: ConfigurationGeneration::new(9).unwrap(),
            controller_generation: Some(ControllerGeneration::new(10).unwrap()),
        };
        let identity = store_identity(&zone, &marker_identity)
            .unwrap()
            .with_revisions(revisions);

        let database = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&database_path)
            .unwrap();
        let mut marker = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(directory.path().join(".d2b-store-marker"))
            .unwrap();
        write_provisioning_marker(&mut marker, &identity).unwrap();
        let provisioned = RedbResourceStore::provision_owned_with_audit(
            database,
            marker,
            identity,
            mutation_seal_pair(
                store_identity(&zone, &marker_identity)
                    .unwrap()
                    .with_revisions(revisions)
                    .seal_identity(),
            )
            .1,
            test_audit_sink(directory.path(), "audit-rehydrate"),
        )
        .await
        .unwrap();
        provisioned.shutdown().await.unwrap();

        let database = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&database_path)
            .unwrap();
        let runtime = ZoneResourceRuntime::open(
            zone.clone(),
            OpenedZoneStore {
                response: OpenZoneStoreResponse {
                    zone_store_id: d2b_contracts_resource::v3::storage::ZoneStoreId::parse(
                        "zone-store-work",
                    )
                    .unwrap(),
                    store_identity: marker_identity,
                    disposition: ZoneStoreDisposition::Opened,
                    fd_index: 0,
                },
                database_fd: database.into(),
                external_inventory: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(runtime.store_metadata.policy_snapshot, revisions);

        let forged_claim = runtime
            .dispatch_public_cli_request(
                &json!({
                    "method": "List",
                    "zoneRef": "Zone/work",
                    "resourceType": "Host",
                    "subjectRef": "User/alice",
                }),
                1000,
            )
            .await
            .unwrap_err();
        assert_eq!(forged_claim, ResourceRuntimeError::RequestInvalid);

        let peer_route = runtime
            .dispatch_public_cli_request(
                &json!({
                    "method": "List",
                    "zoneRef": "Zone/work",
                    "resourceType": "Host",
                }),
                1000,
            )
            .await
            .unwrap_err();
        assert_eq!(peer_route.code(), "resource-runtime-identity-unbound");
        runtime.shutdown().await.unwrap();
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
    fn qemu_controller_contract_invokes_controller_and_finalizes() {
        let guest_ref = ResourceRef::parse("Guest/qemu").unwrap();
        let config = qemu_media_runtime::ProviderConfig::new(
            "Host/host-system",
            "qemu-system-x86-64",
            "Provider/network-local",
            "Provider/volume-local",
            None,
        )
        .unwrap();
        let process = qemu_media_runtime::build_process_spec(
            config.controller_execution_ref.clone(),
            ResourceRef::parse("Volume/qemu-runtime").unwrap(),
            Some(ResourceRef::parse("Device/host-kvm").unwrap()),
            [],
        )
        .unwrap();
        let mut controller = qemu_media_runtime::QemuMediaController::new(
            config,
            qemu_media_runtime::GuestProviderSpecSettings::default(),
            process,
            guest_ref.clone(),
        )
        .unwrap();
        let mut effect = FrameworkQemuEffect::new(guest_ref.clone());
        let dependencies = qemu_media_runtime::QemuMediaDependencies::ready(
            qemu_media_runtime::DeviceObservation {
                device_ref: ResourceRef::parse("Device/host-kvm").unwrap(),
                phase: qemu_media_runtime::DevicePhase::Ready,
                owner_ref: None,
                platform: qemu_media_runtime::PlatformClass::X86_64Linux,
                authority_key: [1; 32],
                process_identity: Some("qemu-media-runner".to_owned()),
                media_contract: "qemu-media/v1".to_owned(),
            },
        );
        assert_eq!(
            controller.reconcile(&dependencies, &mut effect).unwrap(),
            qemu_media_runtime::QemuMediaReconcileOutcome::Ready
        );
        assert_eq!(controller.phase(), qemu_media_runtime::QemuMediaPhase::PausedAtBoot);
        controller.finalize(&mut effect).unwrap();
        assert!(!controller.finalizer_installed());
    }

    #[test]
    fn qemu_guest_child_graph_contains_one_runtime_volume_and_process() {
        let owner = ResourceRef::parse("Guest/qemu").unwrap();
        let guest = json!({
            "spec": {
                "deviceAttachments": [{"deviceRef": "Device/host-kvm"}],
                "networkAttachments": [],
                "provider": {
                    "settings": serde_json::to_value(
                        qemu_media_runtime::GuestProviderSpecSettings::default()
                    )
                    .unwrap()
                }
            }
        });
        let provider_config = serde_json::to_value(
            qemu_media_runtime::ProviderConfig::new(
                "Host/host-system",
                "qemu-system-x86-64",
                "Provider/network-local",
                "Provider/volume-local",
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let provider = json!({
            "spec": {
                "config": provider_config
            }
        });
        let children = DaemonSharedProviderEffects::qemu_guest_children(
            &guest,
            &provider,
            &owner,
            &ZoneId::parse("work").unwrap(),
        )
        .unwrap();
        assert_eq!(
            children
                .iter()
                .map(|child| child.target().resource_type().as_str())
                .collect::<Vec<_>>(),
            vec!["Volume", "Process"]
        );
        assert_eq!(
            children[1].dependencies(),
            &BTreeSet::from([ResourceRef::parse("Volume/qemu-runtime").unwrap()])
        );
    }

    #[tokio::test]
    async fn aca_controller_contract_invokes_controller_and_finalizes() {
        let profile = aca_runtime::AcaSandboxProfile::new(
            aca_runtime::AcaProfileId::parse("default").unwrap(),
            aca_runtime::AcaDiskImageSource::ConfiguredDisk {
                binding_id: aca_runtime::AcaConfiguredDiskId::parse("image-1").unwrap(),
            },
            aca_runtime::AcaCpuMillis::new(500).unwrap(),
            aca_runtime::AcaMemoryMib::new(2_048).unwrap(),
            300,
            None,
        )
        .unwrap();
        let defaults = aca_runtime::AcaRuntimeConfig::new(
            profile,
            aca_runtime::AcaReadinessPolicy::new(3, 10).unwrap(),
            1_000,
            4,
        )
        .unwrap();
        let config = aca_runtime::AcaProviderConfig::new(
            ResourceRef::parse("Guest/gateway").unwrap(),
            aca_runtime::OpaqueAzureRef::parse("tenant").unwrap(),
            aca_runtime::OpaqueAzureRef::parse("client").unwrap(),
            aca_runtime::OpaqueAzureRef::parse("subscription").unwrap(),
            ResourceRef::parse("Credential/control").unwrap(),
            None,
            aca_runtime::AcaConfiguredImageId::parse("environment").unwrap(),
            aca_runtime::AcaConfiguredImageId::parse("resource-group").unwrap(),
            None,
            aca_runtime::AcaProfileId::parse("relay").unwrap(),
            defaults,
        )
        .unwrap();
        let controller = aca_runtime::AzureContainerAppsRuntimeProvider::new(
            config,
            Arc::new(FrameworkAcaControl {
                state: Arc::new(tokio::sync::Mutex::new(FrameworkAcaState::new(1))),
            }),
            Arc::new(FrameworkAcaLease),
        )
        .unwrap()
        .controller(aca_runtime::AcaResourceBinding {
            guest_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            provider_generation: 1,
            config_fingerprint: [2; 32],
        });
        let mut controller = GuestRuntimeController::Aca { controller };
        let GuestRuntimeController::Aca { controller } = &mut controller else {
            unreachable!();
        };
        let operation = aca_runtime::AcaOperationId::parse("u6-aca-test").unwrap();
        assert_eq!(
            controller.reconcile(operation.clone(), 30_000).await.unwrap(),
            aca_runtime::AcaReconcileOutcome::Progressing { after_ms: 10 }
        );
        assert_eq!(
            controller.reconcile(operation, 30_000).await.unwrap(),
            aca_runtime::AcaReconcileOutcome::Converged
        );
        assert_eq!(controller.phase(), aca_runtime::AcaPhase::Ready);
        controller
            .finalize(
                aca_runtime::AcaOperationId::parse("u6-aca-delete").unwrap(),
                30_000,
            )
            .await
            .unwrap();
        assert!(!controller.finalizer_installed());
    }

    #[tokio::test]
    async fn azure_vm_controller_contract_invokes_controller_and_finalizes() {
        let opaque = |value: &str| d2b_contracts::OpaqueAzureRef::parse(value).unwrap();
        let config = azure_vm_runtime::AzureVmConfig {
            tenant_id: None,
            client_id: None,
            arm_credential_ref: ResourceRef::parse("Credential/arm").unwrap(),
            controller_execution_ref: ResourceRef::parse("Guest/gateway").unwrap(),
            network_ref: None,
        };
        let settings = azure_vm_runtime::AzureVmGuestSettings {
            subscription_id: opaque("subscription"),
            resource_group: opaque("resource-group"),
            region: opaque("eastus"),
            vm_size: opaque("standard"),
            image_ref: opaque("image"),
            disk_sku: azure_vm_runtime::DiskSku::PremiumLrs,
            os_disk_size_gb: None,
            admin_user: "azureuser".to_owned(),
            vnet_subscription_id: None,
            vnet_resource_group: None,
            vnet_name: opaque("vnet"),
            subnet_name: opaque("subnet"),
            assign_public_ip: false,
            data_disks: Vec::new(),
            bootstrap_psk_delivery: azure_vm_runtime::BootstrapPskDelivery::VmExtension,
            bootstrap_deadline_ms: 60_000,
            child_zone_hosting: false,
            azure_tags: Vec::new(),
        };
        let effect = Arc::new(FrameworkAzureEffect {
            state: Arc::new(tokio::sync::Mutex::new(FrameworkAzureState::new(&settings))),
        });
        let mut controller = azure_vm_runtime::AzureVmController::new(
            config,
            settings,
            effect,
            Arc::new(FrameworkAzureCredential),
            None,
        )
        .unwrap()
        .with_bootstrap_service(azure_vm_runtime::BootstrapService::from_state(
            azure_vm_runtime::BootstrapServiceState::Enrolled,
        ));
        assert_eq!(
            controller
                .reconcile("work", "123e4567-e89b-42d3-a456-426614174000", 1)
                .await
                .unwrap(),
            azure_vm_runtime::AzureVmReconcileOutcome::Progressing { after_ms: 1_000 }
        );
        for _ in 0..2 {
            controller
                .reconcile("work", "123e4567-e89b-42d3-a456-426614174000", 1)
                .await
                .unwrap();
            if controller.phase() == azure_vm_runtime::AzureVmPhase::Ready {
                break;
            }
        }
        assert_eq!(controller.phase(), azure_vm_runtime::AzureVmPhase::Ready);
        for _ in 0..8 {
            if let Some(operation) = controller.recovery_state().operation {
                controller.poll_operation(operation).await.unwrap();
            }
            let outcome = controller
                .finalize("work", "123e4567-e89b-42d3-a456-426614174000", 1)
                .await
                .unwrap();
            let _ = outcome;
            if !controller.finalizer_installed() {
                break;
            }
        }
        assert!(!controller.finalizer_installed());
    }

    #[tokio::test]
    async fn production_guest_test_store_opens_with_core_session() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("store.redb");
        let database = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&database_path)
            .unwrap();
        let zone = ZoneId::parse("work").unwrap();
        let store_identity = "sha256:".to_owned() + &"d".repeat(64);
        let runtime = ZoneResourceRuntime::open_internal(
            zone.clone(),
            OpenedZoneStore {
                response: OpenZoneStoreResponse {
                    zone_store_id: d2b_contracts_resource::v3::storage::ZoneStoreId::parse(
                        "zone-store-work",
                    )
                    .unwrap(),
                    store_identity,
                    disposition: ZoneStoreDisposition::Provisioned,
                    fd_index: 0,
                },
                database_fd: database.into(),
                external_inventory: None,
            },
            None,
            Arc::new(BrokerEvidenceIndex::default()),
            None,
            true,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(runtime.readiness().resource_api_ready);
        assert!(runtime.core_controller_subject.lock().unwrap().is_some());
        assert!(runtime.process_status_client.lock().unwrap().is_some());
        let provider = BundleResource::new(
            ResourceTypeName::parse("Provider").unwrap(),
            BundleResourceMetadata::new(
                ResourceName::parse("runtime-qemu-media").unwrap(),
                zone.clone(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(
                br#"{"artifactId":"runtime-qemu-media","config":{"controllerExecutionRef":"Host/host-system","networkProviderRef":"Provider/network-local","volumeProviderRef":"Provider/volume-local"}}"#,
            )
            .unwrap(),
        )
        .unwrap();
        let guest_spec_json = br#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"deviceAttachments":[],"networkAttachments":[],"providerRef":"Provider/runtime-qemu-media","systemArtifactId":null,"volumeAttachmentDefaults":[]}"#;
        serde_json::from_slice::<d2b_contracts_resource::v3::ResourceSpec>(guest_spec_json)
            .unwrap_or_else(|error| panic!("Guest spec: {error}"));
        let guest = BundleResource::new(
            ResourceTypeName::parse("Guest").unwrap(),
            BundleResourceMetadata::new(
                ResourceName::parse("qemu").unwrap(),
                zone.clone(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(guest_spec_json).unwrap(),
        )
        .unwrap();
        let bundle = ResourceBundle::new(
            zone.clone(),
            vec![provider],
            "sha256:".to_owned() + &"e".repeat(64),
            BTreeMap::new(),
            BTreeMap::new(),
            Timestamp::parse("1970-01-01T00:00:00.000Z").unwrap(),
        )
        .unwrap()
        .with_zone_uid(runtime.store.identity().zone_uid().clone());
        runtime
            .materialize_desired_bundle(&bundle)
            .await
            .unwrap_or_else(|error| panic!("test bundle materialization failed: {error:?}"));
        assert!(
            runtime
                .committed_resource_value(
                    &ResourceRef::parse("Provider/runtime-qemu-media").unwrap(),
                    "u6-test-provider-read",
                )
                .await
                .is_ok()
        );
        let guest_bundle = ResourceBundle::new(
            zone.clone(),
            vec![guest],
            "sha256:".to_owned() + &"f".repeat(64),
            BTreeMap::new(),
            BTreeMap::new(),
            Timestamp::parse("1970-01-01T00:00:00.000Z").unwrap(),
        )
        .unwrap()
        .with_zone_uid(runtime.store.identity().zone_uid().clone());
        runtime.materialize_desired_bundle(&guest_bundle).await.unwrap();
        assert!(
            runtime
                .committed_resource_value(
                    &ResourceRef::parse("Guest/qemu").unwrap(),
                    "u6-test-guest-read",
                )
                .await
                .is_ok()
        );
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn production_materialization_supplies_fixed_process_provider_rows() {
        let (_directory, runtime, _broker_evidence) = open_production_guest_runtime_for_test().await;
        let provider_refs = [
            "Provider/system-core",
            "Provider/system-minijail",
            "Provider/system-systemd",
        ]
        .into_iter()
        .map(|provider| ResourceRef::parse(provider).unwrap())
        .collect::<BTreeSet<_>>();
        assert!(
            load_committed_controller_provider_identities(
                &runtime.zone,
                &runtime.store,
                provider_refs.clone(),
            )
            .await
            .is_err(),
            "missing fixed Provider rows must fail closed"
        );

        materialize_test_bundle(&runtime, Vec::new()).await;
        let identities = load_committed_controller_provider_identities(
            &runtime.zone,
            &runtime.store,
            provider_refs,
        )
        .await
        .expect("materialized fixed Provider identities");
        assert_eq!(identities.len(), 3);

        materialize_test_bundle(
            &runtime,
            vec![bundle_resource(
                "Process",
                "bootstrap-process",
                &runtime.zone,
                r#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"reaction"}"#,
            )],
        )
        .await;
        let process = runtime
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "process-assignment-read-after-bootstrap".to_owned(),
                    idempotency_key: None,
                    correlation_id: "process-assignment-read-after-bootstrap".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: runtime.zone.clone(),
                target: ResourceRef::parse("Process/bootstrap-process").unwrap(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .unwrap();
        let authority = Arc::new(CoreAssignmentAuthority {
            provider_generation: ResourceGeneration::new(17).unwrap(),
            controller_generation: ControllerGeneration::new(23).unwrap(),
            session_generation: ReconnectGeneration::new(19).unwrap(),
            controller_role: ResourceRef::parse("Process/d2b-core-controller").unwrap(),
            target: ResourceRef::parse("Zone/work").unwrap(),
        });
        let resolver = process_assignment_fence_resolver(
            Arc::clone(&runtime.store),
            DaemonMode::Host,
            Arc::clone(&authority),
        );
        let fence = resolver(
            process.resource_ref.clone(),
            process.uid.clone(),
            process.revision,
        )
        .await
        .expect("fixed Provider appearance must requeue assignment admission");
        assert_eq!(fence.resource_uid, process.uid);
        assert_eq!(fence.provider_generation, authority.provider_generation);
        assert_eq!(fence.controller_generation, authority.controller_generation);
        assert_eq!(fence.session_generation, authority.session_generation);
        assert_eq!(fence.controller_role, authority.controller_role);
        assert_eq!(
            fence.target,
            ResourceRef::parse("Host/host-system").unwrap()
        );
        assert_eq!(fence.epoch, ASSIGNMENT_EPOCH);
        drop(resolver);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn controller_provider_identity_projection_uses_one_current_store_snapshot() {
        let (_directory, runtime, broker_evidence) = open_production_guest_runtime_for_test().await;
        let provider_ref = ResourceRef::parse("Provider/system-minijail").unwrap();
        materialize_test_bundle(
            &runtime,
            vec![bundle_resource(
                "Provider",
                "system-minijail",
                &runtime.zone,
                r#"{"artifactId":"system-minijail","config":{}}"#,
            )],
        )
        .await;
        let stale_revision = runtime.store.runtime_metadata().await.unwrap().current_revision;

        mark_test_resource_phase(&runtime, &provider_ref, &broker_evidence, "Ready").await;
        let current = runtime
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "provider-identity-current-row".to_owned(),
                    idempotency_key: None,
                    correlation_id: "provider-identity-current-row".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: runtime.zone.clone(),
                target: provider_ref.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .unwrap();
        assert!(current.revision > stale_revision);

        let identities = load_committed_controller_provider_identities(
            &runtime.zone,
            &runtime.store,
            BTreeSet::from([provider_ref.clone()]),
        )
        .await
        .expect("current Provider status revision must not invalidate its identity");
        assert_eq!(
            identities.get(&provider_ref),
            Some(&(current.uid.clone(), current.generation))
        );
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn live_cloud_hypervisor_inputs_use_current_provider_snapshot() {
        let (_directory, mut runtime, broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let zone = runtime.zone.clone();
        let provider_ref = ResourceRef::parse("Provider/runtime-cloud-hypervisor").unwrap();
        let guest_ref = ResourceRef::parse("Guest/cloud-hypervisor").unwrap();
        materialize_test_bundle(
            &runtime,
            vec![
                bundle_resource(
                    "Provider",
                    "runtime-cloud-hypervisor",
                    &zone,
                    r#"{"artifactId":"runtime-cloud-hypervisor","config":{"controllerExecutionRef":"Host/host-system","defaultVcpus":2,"defaultMemoryMb":512,"defaultMachineType":"q35","watchdog":true,"adoptionWindowMs":30000,"healthCheckIntervalMs":30000,"healthCheckTimeoutMs":5000,"healthCheckFailureThreshold":3,"startupDeadlineMs":120000}}"#,
                ),
                bundle_resource(
                    "Guest",
                    "cloud-hypervisor",
                    &zone,
                    r#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"deviceAttachments":[],"networkAttachments":[],"providerRef":"Provider/runtime-cloud-hypervisor","systemArtifactId":null,"volumeAttachmentDefaults":[]}"#,
                ),
            ],
        )
        .await;
        runtime.store_metadata = runtime.store.runtime_metadata().await.unwrap();
        let activation_revision = runtime.store_metadata.current_revision;
        let before = read_test_resource(
            &runtime,
            provider_ref.clone(),
            "cloud-hypervisor-inputs-before-status",
        )
        .await;
        let (_, _, expected_config, _) = runtime
            .cloud_hypervisor_inputs(&guest_ref)
            .await
            .expect("activation snapshot must admit the unchanged Provider config");

        mark_test_resource_phase(&runtime, &provider_ref, &broker_evidence, "Ready").await;
        let after = read_test_resource(
            &runtime,
            provider_ref,
            "cloud-hypervisor-inputs-after-status",
        )
        .await;
        assert!(after.revision > activation_revision);
        assert_eq!(after.uid, before.uid);
        assert_eq!(after.generation, before.generation);

        let (_, _, config, _) = runtime
            .cloud_hypervisor_inputs(&guest_ref)
            .await
            .expect("Provider status revision must not invalidate unchanged CH config");
        assert_eq!(config, expected_config);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn live_wayland_session_lookup_uses_current_store_snapshot() {
        let (_directory, mut runtime, broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let zone = runtime.zone.clone();
        let guest_ref = ResourceRef::parse("Guest/workstation").unwrap();
        let host_ref = ResourceRef::parse("Host/host-system").unwrap();
        let user_ref = ResourceRef::parse("User/alice").unwrap();
        let session_ref =
            ResourceRef::parse("display-wayland.d2bus.org.WaylandSession/display-wayland")
                .unwrap();
        let session_spec = d2b_provider_display_wayland::WaylandSessionSpec::new(
            guest_ref.clone(),
            host_ref.clone(),
            user_ref.clone(),
            ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/display-wayland").unwrap(),
            d2b_provider_display_wayland::DisplayIdentity::new(
                "display",
                "#112233",
                "#223344",
                "#334455",
            )
            .unwrap(),
            true,
        )
        .unwrap();
        materialize_test_bundle(
            &runtime,
            vec![bundle_resource(
                "display-wayland.d2bus.org.WaylandSession",
                "display-wayland",
                &zone,
                &serde_json::to_string(&session_spec).unwrap(),
            )],
        )
        .await;
        let stored =
            read_test_resource(&runtime, session_ref.clone(), "wayland-session-before-status")
                .await;
        let mut identity = CommittedInteractionIdentity::for_test(
            zone.clone(),
            guest_ref,
            ResourceUid::parse("44444444-4444-4444-8444-444444444444").unwrap(),
            host_ref,
            user_ref,
            BTreeMap::new(),
            ResourceGeneration::new(1).unwrap(),
            None,
            None,
            None,
            None,
        );
        identity.wayland_session_uid = stored.uid.clone();
        runtime.interaction_identity = Some(identity);
        runtime.readiness.resource_api_ready = true;
        runtime.store_metadata = runtime.store.runtime_metadata().await.unwrap();
        let activation_revision = runtime.store_metadata.current_revision;
        let (_, _, expected_spec) = runtime
            .committed_wayland_session_for_vm("workstation")
            .await
            .expect("activation snapshot must admit the unchanged Wayland session")
            .expect("Wayland session identity is present");

        mark_test_resource_phase(&runtime, &session_ref, &broker_evidence, "Ready").await;
        let current =
            read_test_resource(&runtime, session_ref, "wayland-session-after-status").await;
        assert!(current.revision > activation_revision);
        assert_eq!(current.uid, stored.uid);
        assert_eq!(current.generation, stored.generation);

        let (_, uid, spec) = runtime
            .committed_wayland_session_for_vm("workstation")
            .await
            .expect("Wayland status revision must not invalidate unchanged session")
            .expect("Wayland session identity remains present");
        assert_eq!(uid, current.uid);
        assert_eq!(spec, expected_spec);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn generated_controller_process_assignment_uses_core_authority_identity() {
        let (_directory, runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let zone = runtime.zone.clone();
        materialize_test_bundle(
            &runtime,
            vec![bundle_resource(
                "Provider",
                "network-local",
                &zone,
                r#"{"artifactId":"acceptance-provider","config":{}}"#,
            )],
        )
        .await;
        let owner = ResourceRef::parse("Provider/network-local").unwrap();
        let process = BundleResource::new(
            ResourceTypeName::parse("Process").unwrap(),
            BundleResourceMetadata::new(
                ResourceName::parse("controller-e1e6335ae78671758ea381fce5d44fdd").unwrap(),
                zone.clone(),
                Some(owner),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(
                br#"{"executionRef":"Host/host-system","processClass":"controller","providerRef":"Provider/system-minijail","template":"acceptance-controller"}"#,
            )
            .unwrap(),
        )
        .unwrap();
        materialize_test_bundle(&runtime, vec![process]).await;
        let process_ref =
            ResourceRef::parse("Process/controller-e1e6335ae78671758ea381fce5d44fdd").unwrap();
        let stored = runtime
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "generated-controller-process-read".to_owned(),
                    idempotency_key: None,
                    correlation_id: "generated-controller-process-read".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: zone.clone(),
                target: process_ref.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .unwrap();
        let authority = Arc::new(CoreAssignmentAuthority {
            provider_generation: ResourceGeneration::new(17).unwrap(),
            controller_generation: ControllerGeneration::new(23).unwrap(),
            session_generation: ReconnectGeneration::new(19).unwrap(),
            controller_role: ResourceRef::parse("Process/d2b-core-controller").unwrap(),
            target: ResourceRef::parse("Zone/work").unwrap(),
        });
        let system_minijail = runtime
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "generated-controller-process-provider-read".to_owned(),
                    idempotency_key: None,
                    correlation_id: "generated-controller-process-provider-read".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: zone.clone(),
                target: ResourceRef::parse("Provider/system-minijail").unwrap(),
                expected_uid: None,
                projection: StoreProjection::MetadataOnly,
            })
            .await
            .unwrap();
        assert_ne!(
            authority.provider_generation,
            system_minijail.generation,
            "test must distinguish Core Provider from Process effect Provider",
        );
        let resolver = process_assignment_fence_resolver(
            Arc::clone(&runtime.store),
            DaemonMode::Host,
            Arc::clone(&authority),
        );
        let fence = resolver(
            process_ref.clone(),
            stored.uid.clone(),
            stored.revision,
        )
        .await
        .expect("generated controller Process assignment");
        assert_eq!(fence.provider_generation, authority.provider_generation);
        assert_eq!(fence.controller_generation, authority.controller_generation);
        assert_eq!(fence.session_generation, authority.session_generation);
        assert_eq!(fence.controller_role, authority.controller_role);
        assert_eq!(
            fence.target,
            ResourceRef::parse("Host/host-system").unwrap()
        );
        assert_eq!(fence.epoch, ASSIGNMENT_EPOCH);

        let wrong_process = bundle_resource(
            "Process",
            "wrong-controller-provider",
            &zone,
            r#"{"executionRef":"Host/host-system","processClass":"controller","providerRef":"Provider/not-a-process-provider","template":"acceptance-controller"}"#,
        );
        materialize_test_bundle(&runtime, vec![wrong_process]).await;
        let wrong_ref = ResourceRef::parse("Process/wrong-controller-provider").unwrap();
        let wrong = runtime
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "wrong-controller-provider-read".to_owned(),
                    idempotency_key: None,
                    correlation_id: "wrong-controller-provider-read".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: zone.clone(),
                target: wrong_ref.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .unwrap();
        assert!(matches!(
            resolver(wrong_ref, wrong.uid, wrong.revision).await,
            Err(SourceError::Integrity)
        ));

        let missing_target = ResourceRef::parse("Process/missing-controller").unwrap();
        assert!(
            resolver(
                missing_target,
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                ZoneRevision::new(1),
            )
            .await
            .is_err(),
            "missing Process target must fail closed"
        );
        let _ = runtime.shutdown().await;
    }

    #[tokio::test]
    async fn sparse_seven_row_bundle_starts_only_present_shared_provider_runners() {
        let (_directory, mut runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let zone = runtime.zone.clone();
        let resources = vec![
            bundle_resource(
                "Provider",
                "network-local",
                &zone,
                r#"{"artifactId":"acceptance-provider","config":{}}"#,
            ),
            bundle_resource(
                "Provider",
                "runtime-qemu-media",
                &zone,
                r#"{"artifactId":"runtime-qemu-media","config":{"controllerExecutionRef":"Host/host-system","networkProviderRef":"Provider/network-local","volumeProviderRef":"Provider/volume-local"}}"#,
            ),
            bundle_resource(
                "User",
                "alice",
                &zone,
                r#"{"displayName":"Alice","groups":[],"osUsername":"alice"}"#,
            ),
            bundle_resource(
                "User",
                "d2bd",
                &zone,
                r#"{"displayName":"d2bd","groups":[],"osUsername":"d2bd"}"#,
            ),
            bundle_resource(
                "Device",
                "host-kvm",
                &zone,
                r#"{"deviceClass":"emulated","arbitration":"exclusive","maxConcurrentClaims":1,"inventory":{}}"#,
            ),
            bundle_resource(
                "Guest",
                "qemu",
                &zone,
                r#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"deviceAttachments":[{"deviceRef":"Device/host-kvm","exclusive":false}],"networkAttachments":[],"providerRef":"Provider/runtime-qemu-media","systemArtifactId":null,"volumeAttachmentDefaults":[]}"#,
            ),
            bundle_resource(
                "Provider",
                "acceptance-extra",
                &zone,
                r#"{"artifactId":"acceptance-provider","config":{}}"#,
            ),
        ];
        assert_eq!(resources.len(), 7);
        materialize_test_bundle(&runtime, resources).await;
        assert_eq!(
            runtime
                .committed_resources_of_type("Provider")
                .await
                .unwrap()
                .len(),
            6
        );
        runtime.readiness.resource_api_ready = true;
        runtime.set_provider_path_ready(true);
        runtime
            .require_ready()
            .expect("sparse bundle must reach Host readiness");
        assert_eq!(runtime.interaction_state, InteractionState::Absent);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn present_interaction_provider_without_identity_refuses_readiness() {
        let (_directory, mut runtime, _broker_evidence) =
            open_production_guest_runtime_for_test().await;
        let zone = runtime.zone.clone();
        materialize_test_bundle(
            &runtime,
            vec![bundle_resource(
                "Provider",
                "display-wayland",
                &zone,
                r#"{"artifactId":"display-wayland","config":{}}"#,
            )],
        )
        .await;
        assert!(
            interaction_resources_present(&zone, &runtime.store)
                .await
                .expect("interaction presence scan")
        );
        runtime.readiness.resource_api_ready = true;
        // A present interaction Provider whose committed identity never
        // resolved leaves the interaction composition refused: readiness is
        // withheld rather than reported for a half-composed family.
        runtime.interaction_state = InteractionState::Refused;
        runtime.set_provider_path_ready(true);
        assert_eq!(
            runtime.require_ready(),
            Err(ResourceRuntimeError::InteractionConfigurationUnavailable)
        );
        runtime.shutdown().await.unwrap();
    }

    pub(crate) async fn open_production_guest_runtime_for_test() -> (
        tempfile::TempDir,
        ZoneResourceRuntime,
        Arc<BrokerEvidenceIndex>,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("store.redb");
        let database = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&database_path)
            .unwrap();
        let zone = ZoneId::parse("work").unwrap();
        let broker_evidence = Arc::new(BrokerEvidenceIndex::default());
        let runtime = ZoneResourceRuntime::open_internal(
            zone,
            OpenedZoneStore {
                response: OpenZoneStoreResponse {
                    zone_store_id: d2b_contracts_resource::v3::storage::ZoneStoreId::parse(
                        "zone-store-work",
                    )
                    .unwrap(),
                    store_identity: "sha256:".to_owned() + &"a".repeat(64),
                    disposition: ZoneStoreDisposition::Provisioned,
                    fd_index: 0,
                },
                database_fd: database.into(),
                external_inventory: None,
            },
            None,
            Arc::clone(&broker_evidence),
            None,
            true,
            None,
            None,
        )
        .await
        .unwrap();
        (directory, runtime, broker_evidence)
    }

    pub(crate) fn bundle_resource(
        resource_type: &str,
        name: &str,
        zone: &ZoneId,
        spec: &str,
    ) -> BundleResource {
        bundle_resource_with_annotations(
            resource_type,
            name,
            zone,
            spec,
            BTreeMap::new(),
        )
    }

    fn bundle_resource_with_annotations(
        resource_type: &str,
        name: &str,
        zone: &ZoneId,
        spec: &str,
        annotations: BTreeMap<String, String>,
    ) -> BundleResource {
        BundleResource::new(
            ResourceTypeName::parse(resource_type).unwrap(),
            BundleResourceMetadata::new(
                ResourceName::parse(name).unwrap(),
                zone.clone(),
                None,
                BTreeMap::new(),
                annotations,
            ),
            CanonicalJsonObject::parse(spec.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn credential_spec(gateway: &str) -> String {
        let scope = d2b_contracts_provider::v3::credential::CredentialScope::new(
            Some(ResourceRef::parse(gateway).unwrap()),
            None,
            None,
        )
        .unwrap();
        let spec = d2b_contracts_provider::v3::credential::CredentialSpec::new(
            scope,
            d2b_contracts_provider::v3::credential::AudienceToken::parse(
                "azure-resource-manager",
            )
            .unwrap(),
            None,
            vec![
                d2b_contracts_provider::v3::credential::CredentialOperation::AcquireToken,
            ],
            d2b_contracts_provider::v3::credential::RotationSpec::default(),
            d2b_contracts_provider::v3::credential::ExpirySpec::default(),
            d2b_contracts_provider::v3::credential::RevocationSpec::default(),
            None,
            None,
        )
        .unwrap();
        serde_json::to_string(&spec).unwrap()
    }

    pub(crate) async fn materialize_test_bundle(
        runtime: &ZoneResourceRuntime,
        resources: Vec<BundleResource>,
    ) {
        let zone = runtime.zone.clone();
        let bundle = ResourceBundle::new(
            zone,
            resources,
            "sha256:".to_owned() + &"b".repeat(64),
            BTreeMap::new(),
            BTreeMap::new(),
            Timestamp::parse("1970-01-01T00:00:00.000Z").unwrap(),
        )
        .unwrap()
        .with_zone_uid(runtime.store.identity().zone_uid().clone());
        bundle
            .verify()
            .unwrap_or_else(|error| panic!("test bundle verification failed: {error:?}"));
        runtime
            .materialize_desired_bundle(&bundle)
            .await
            .unwrap_or_else(|error| panic!("test bundle materialization failed: {error:?}"));
    }

    async fn start_production_guest_runner_fixture(
        resources: Vec<BundleResource>,
    ) -> (
        tempfile::TempDir,
        Arc<ServerState>,
        Arc<ResourcePlane>,
        Arc<ZoneResourceRuntime>,
        Arc<BrokerEvidenceIndex>,
    ) {
        let (directory, state, plane, runtime, broker_evidence) =
            prepare_production_guest_runner_fixture(resources).await;
        runtime
            .start_u6_controller_runners(Arc::clone(&state))
            .await
            .unwrap();
        (directory, state, plane, runtime, broker_evidence)
    }

    async fn prepare_production_guest_runner_fixture(
        resources: Vec<BundleResource>,
    ) -> (
        tempfile::TempDir,
        Arc<ServerState>,
        Arc<ResourcePlane>,
        Arc<ZoneResourceRuntime>,
        Arc<BrokerEvidenceIndex>,
    ) {
        let (directory, runtime, broker_evidence) =
            open_production_guest_runtime_for_test().await;
        materialize_test_bundle(&runtime, resources).await;
        let state = Arc::new(crate::detached_exec_routing_tests::test_state(
            Default::default(),
        ));
        let zone = runtime.zone.clone();
        let mut plane = ResourcePlane::new();
        plane.insert(runtime).unwrap();
        let plane = crate::install_test_resource_plane(&state, plane);
        let runtime = plane.zone(&zone).unwrap();
        (directory, state, plane, runtime, broker_evidence)
    }

    async fn mark_test_resource_ready(
        runtime: &ZoneResourceRuntime,
        target: &ResourceRef,
        broker_evidence: &BrokerEvidenceIndex,
    ) {
        mark_test_resource_phase(runtime, target, broker_evidence, "Ready").await;
    }

    async fn mark_test_resource_phase(
        runtime: &ZoneResourceRuntime,
        target: &ResourceRef,
        broker_evidence: &BrokerEvidenceIndex,
        phase: &str,
    ) {
        let current = runtime
            .committed_resource_value(target, "u6-test-ready-read")
            .await
            .unwrap();
        let mut status = current.get("status").cloned().unwrap();
        status["phase"] = Value::String(phase.to_owned());
        status["observedGeneration"] = current["metadata"]["generation"].clone();
        let client = runtime.status_client().unwrap();
        let operation = bounded_operation_id(&format!(
            "u6-test-ready:{}:{}",
            target.to_canonical_string(),
            current["metadata"]["revision"]
        ));
        if matches!(
            target.resource_type().as_str(),
            "Provider" | "Credential"
        ) {
            broker_evidence
                .insert(DurabilityEvidence {
                    key: d2b_audit::operation::ZoneOperationKey::derive(
                        runtime.zone.as_str(),
                        &operation,
                    )
                    .unwrap(),
                    outcome: d2b_audit::DurabilityOutcome::Success,
                    effect_durable: true,
                })
                .unwrap();
        }
        let request = public_update_status_request_from_current(
            runtime,
            &json!({
                "status": status,
                "expectedRevision": current["metadata"]["revision"],
            }),
            &operation,
            target,
            current,
        )
        .unwrap();
        let response = client.update_status(request).await;
        if let Some(error) = response.error.as_ref() {
            panic!(
                "test status update rejected: kind={:?} reason={}",
                error.kind, error.reason
            );
        }
    }

    async fn assert_guest_assignment_fence(
        runtime: &ZoneResourceRuntime,
        guest_ref: &ResourceRef,
        provider_ref: &ResourceRef,
        controller_ref: &ResourceRef,
    ) {
        let guest = runtime
            .committed_resource_value(guest_ref, "u6-test-fence-read")
            .await
            .unwrap();
        assert_eq!(
            guest["spec"]["providerRef"],
            provider_ref.to_canonical_string()
        );
        let provider = runtime
            .committed_resource_value(provider_ref, "u6-test-provider-fence-read")
            .await
            .unwrap();
        let fence = runtime
            .store
            .assignment_fence(runtime.zone.clone(), guest_ref.clone())
            .await
            .unwrap()
            .expect("Guest assignment fence");
        assert_eq!(
            fence.resource_uid,
            ResourceUid::parse(guest["metadata"]["uid"].as_str().unwrap()).unwrap()
        );
        assert_eq!(
            fence.resource_revision,
            ZoneRevision::new(guest["metadata"]["revision"].as_u64().unwrap())
        );
        assert_eq!(
            fence.provider_generation,
            ResourceGeneration::new(provider["metadata"]["generation"].as_u64().unwrap()).unwrap()
        );
        assert_eq!(
            fence.controller_generation,
            runtime
                .store
                .runtime_metadata()
                .await
                .unwrap()
                .policy_snapshot
                .controller_generation
                .unwrap()
        );
        assert_eq!(fence.controller_role, controller_ref.clone());
        assert_eq!(
            fence.target,
            ResourceRef::parse(&format!("Zone/{}", runtime.zone.as_str())).unwrap()
        );
        assert_eq!(
            fence.session_generation,
            runtime
                .core_controller_subject
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .reconnect_generation()
        );
        assert_eq!(fence.epoch, ASSIGNMENT_EPOCH);
        assert!(matches!(fence.scope, ResourceAssignmentScope::Primary));
    }

    async fn wait_for_test_resource(
        runtime: &ZoneResourceRuntime,
        target: &ResourceRef,
        predicate: impl Fn(&Value) -> bool,
    ) -> Value {
        for _ in 0..3_000 {
            if let Ok(value) = runtime
                .committed_resource_value(target, "u6-test-wait")
                .await
                && predicate(&value)
            {
                return value;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {}", target.to_canonical_string());
    }

    async fn wait_for_test_resource_gone(runtime: &ZoneResourceRuntime, target: &ResourceRef) {
        for _ in 0..3_000 {
            if runtime
                .committed_resource_value(target, "u6-test-wait-gone")
                .await
                .is_err()
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {}", target.to_canonical_string());
    }

    async fn request_test_delete(runtime: &ZoneResourceRuntime, target: &ResourceRef) {
        for attempt in 0..100 {
            let current = runtime
                .committed_resource_value(target, "u6-test-delete-read")
                .await
                .unwrap();
            let client = runtime.status_client().unwrap();
            let operation = format!("u6-test-delete-{attempt}");
            let request = public_delete_request_from_current(
                runtime,
                &json!({
                    "resourceRef": target.to_canonical_string(),
                    "uid": current["metadata"]["uid"],
                    "expectedRevision": current["metadata"]["revision"],
                }),
                &operation,
                current,
            )
            .unwrap();
            let response = client.delete(request).await;
            let Some(error) = response.error.as_ref() else {
                return;
            };
            if error.reason.as_str() == "resource-revision-changed" {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                continue;
            }
            panic!(
                "test delete rejected: kind={:?} reason={}",
                error.kind, error.reason
            );
        }
        panic!("test delete did not become admitted");
    }

    async fn add_test_child_finalizer(runtime: &ZoneResourceRuntime, target: &ResourceRef) {
        let current = runtime
            .committed_resource_value(target, "u6-test-child-finalizer-read")
            .await
            .unwrap();
        let request = public_update_finalizers_request(
            runtime,
            &json!({
                "resourceRef": target.to_canonical_string(),
                "uid": current["metadata"]["uid"],
                "expectedRevision": current["metadata"]["revision"],
                "addFinalizers": ["test.d2bus.org/hold"],
                "removeFinalizers": [],
            }),
            "u6-test-child-finalizer",
        )
        .unwrap();
        let response = runtime.status_client().unwrap().update_finalizers(request).await;
        if let Some(error) = response.error.as_ref() {
            panic!(
                "test child finalizer update rejected: kind={:?} reason={}",
                error.kind, error.reason
            );
        }
    }

    async fn create_test_child(
        runtime: &ZoneResourceRuntime,
        owner: &ResourceRef,
        target: &ResourceRef,
    ) {
        let process = qemu_media_runtime::build_process_spec(
            ResourceRef::parse("Host/host-system").unwrap(),
            ResourceRef::parse("Volume/u6-test-runtime").unwrap(),
            None,
            [],
        )
        .unwrap();
        let mut process_spec = serde_json::to_value(process).unwrap();
        process_spec
            .as_object_mut()
            .unwrap()
            .insert(
                "providerRef".to_owned(),
                Value::String("Provider/system-minijail".to_owned()),
            );
        let canonical = DaemonSharedProviderEffects::guest_child_resource(
            target,
            owner,
            &runtime.zone,
            process_spec,
        )
        .unwrap();
        let identity = public_identity(
            runtime,
            target.resource_type(),
            target.name().as_str(),
            None,
            None,
            None,
        );
        let mut mutation = wire::Mutation::new();
        mutation.kind =
            protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_CREATE);
        mutation.target = protobuf::MessageField::some(identity.clone());
        mutation.precondition = protobuf::MessageField::some(create_precondition());
        mutation.resource = protobuf::MessageField::some(
            ch_resource_body(&runtime.zone, target, None, &canonical).unwrap(),
        );
        mutation.owner = protobuf::MessageField::some(public_identity(
            runtime,
            owner.resource_type(),
            owner.name().as_str(),
            None,
            None,
            None,
        ));
        let mut request = wire::CreateRequest::new();
        request.meta = protobuf::MessageField::some(public_request_meta(
            &bounded_operation_id(&format!(
                "u6-test-child-create:{}",
                target.to_canonical_string()
            )),
        ));
        request.mutation = protobuf::MessageField::some(mutation);
        let response = runtime.status_client().unwrap().create(request).await;
        assert!(response.error.is_none(), "{:?}", response.error);
    }

    async fn clear_test_child_finalizers(
        runtime: &ZoneResourceRuntime,
        target: &ResourceRef,
    ) {
        let current = runtime
            .committed_resource_value(target, "u6-test-child-finalizer-clear-read")
            .await
            .unwrap();
        let request = public_update_finalizers_request(
            runtime,
            &json!({
                "resourceRef": target.to_canonical_string(),
                "uid": current["metadata"]["uid"],
                "expectedRevision": current["metadata"]["revision"],
                "addFinalizers": [],
                "removeFinalizers": ["test.d2bus.org/hold"],
            }),
            "u6-test-child-finalizer-clear",
        )
        .unwrap();
        let response = runtime.status_client().unwrap().update_finalizers(request).await;
        assert!(response.error.is_none(), "{:?}", response.error);
    }

    async fn close_production_guest_runtime_fixture(
        state: Arc<ServerState>,
        plane: Arc<ResourcePlane>,
    ) {
        state
            .resource_plane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let mut plane = Arc::try_unwrap(plane).expect("test plane has one owner");
        // Runner tasks outlive the test body (detached respawn loops) and
        // their final store requests may still own a runtime reference when
        // teardown starts. Drain briefly instead of panicking on the first
        // live owner: the runners never hold the runtime beyond one request.
        for _ in 0..250 {
            match plane.shutdown().await {
                Ok(()) => return,
                Err(ResourceRuntimeError::LiveRequestOwners) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Err(error) => panic!("fixture plane shutdown failed: {error:?}"),
            }
        }
        panic!("fixture plane still had live request owners after the drain window");
    }

    #[tokio::test]
    async fn qemu_framework_runner_invokes_controller_and_finalizes() {
        let (_directory, runtime, broker_evidence) = open_production_guest_runtime_for_test().await;
        let zone = runtime.zone.clone();
        materialize_test_bundle(
            &runtime,
            vec![
                bundle_resource(
                    "Provider",
                    "runtime-qemu-media",
                    &zone,
                    r#"{"artifactId":"runtime-qemu-media","config":{"controllerExecutionRef":"Host/host-system","networkProviderRef":"Provider/network-local","volumeProviderRef":"Provider/volume-local"}}"#,
                ),
                bundle_resource(
                    "Device",
                    "host-kvm",
                    &zone,
                    r#"{"deviceClass":"emulated","arbitration":"exclusive","maxConcurrentClaims":1,"inventory":{}}"#,
                ),
                bundle_resource(
                    "Guest",
                    "qemu-delete",
                    &zone,
                    r#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"deviceAttachments":[{"deviceRef":"Device/host-kvm","exclusive":false}],"networkAttachments":[],"providerRef":"Provider/runtime-qemu-media","systemArtifactId":null,"volumeAttachmentDefaults":[]}"#,
                ),
                bundle_resource(
                    "Guest",
                    "qemu-ready",
                    &zone,
                    r#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"deviceAttachments":[{"deviceRef":"Device/host-kvm","exclusive":false}],"networkAttachments":[],"providerRef":"Provider/runtime-qemu-media","systemArtifactId":null,"volumeAttachmentDefaults":[]}"#,
                ),
            ],
        )
        .await;
        let delete_guest_ref = ResourceRef::parse("Guest/qemu-delete").unwrap();
        let ready_guest_ref = ResourceRef::parse("Guest/qemu-ready").unwrap();
        let device_ref = ResourceRef::parse("Device/host-kvm").unwrap();
        let provider_ref = ResourceRef::parse("Provider/runtime-qemu-media").unwrap();

        let state = Arc::new(crate::detached_exec_routing_tests::test_state(
            Default::default(),
        ));
        let mut plane = ResourcePlane::new();
        plane.insert(runtime).unwrap();
        let plane = crate::install_test_resource_plane(&state, plane);
        let runtime = plane.zone(&zone).unwrap();
        runtime
            .start_u6_controller_runners(Arc::clone(&state))
            .await
            .unwrap();
        assert!(
            !runtime.u6_runner_tasks.lock().unwrap().is_empty(),
            "U6 runner did not start"
        );

        let deleting_guest = wait_for_test_resource(&runtime, &delete_guest_ref, |value| {
            value["metadata"]["finalizers"]
                .as_array()
                .is_some_and(|finalizers| !finalizers.is_empty())
        })
        .await;
        assert_eq!(
            deleting_guest["metadata"]["finalizers"],
            serde_json::json!([
                qemu_media_runtime::FINALIZER
            ])
        );
        let ready_guest = wait_for_test_resource(&runtime, &ready_guest_ref, |value| {
            value["metadata"]["finalizers"]
                .as_array()
                .is_some_and(|finalizers| !finalizers.is_empty())
        })
        .await;
        assert_eq!(ready_guest["status"]["phase"], "Pending");
        assert_eq!(deleting_guest["status"]["phase"], "Pending");
        assert!(
            runtime
                .committed_resource_value(
                    &ResourceRef::parse("Volume/qemu-delete-runtime").unwrap(),
                    "u6-test-first-finalizer-only",
                )
                .await
                .is_err()
        );
        assert!(
            runtime
                .committed_resource_value(
                    &ResourceRef::parse("Process/qemu-delete-qemu").unwrap(),
                    "u6-test-first-finalizer-only",
                )
                .await
                .is_err()
        );
        assert!(
            runtime
                .committed_resource_value(
                    &ResourceRef::parse("Volume/qemu-ready-runtime").unwrap(),
                    "u6-test-first-finalizer-only",
                )
                .await
                .is_err()
        );
        assert!(
            runtime
                .committed_resource_value(
                    &ResourceRef::parse("Process/qemu-ready-qemu").unwrap(),
                    "u6-test-first-finalizer-only",
                )
                .await
                .is_err()
        );
        assert_guest_assignment_fence(
            &runtime,
            &delete_guest_ref,
            &provider_ref,
            &ResourceRef::parse("Process/runtime-qemu-media-controller").unwrap(),
        )
        .await;
        assert_guest_assignment_fence(
            &runtime,
            &ready_guest_ref,
            &provider_ref,
            &ResourceRef::parse("Process/runtime-qemu-media-controller").unwrap(),
        )
        .await;
        mark_test_resource_ready(&runtime, &device_ref, &broker_evidence).await;
        mark_test_resource_ready(&runtime, &provider_ref, &broker_evidence).await;
        let ready_volume_ref = ResourceRef::parse("Volume/qemu-ready-runtime").unwrap();
        let ready_process_ref = ResourceRef::parse("Process/qemu-ready-qemu").unwrap();
        let ready_volume = wait_for_test_resource(&runtime, &ready_volume_ref, |_| true).await;
        mark_test_resource_ready(&runtime, &ready_volume_ref, &broker_evidence).await;
        let ready_process = wait_for_test_resource(&runtime, &ready_process_ref, |_| true).await;
        assert!(
            ready_process["metadata"]["revision"].as_u64().unwrap()
                > ready_volume["metadata"]["revision"].as_u64().unwrap()
        );
        mark_test_resource_ready(&runtime, &ready_process_ref, &broker_evidence).await;
        let ready_guest = wait_for_test_resource(&runtime, &ready_guest_ref, |value| {
            value["status"]["phase"] == "Ready"
                && value["status"]["observedGeneration"] == value["metadata"]["generation"]
        })
        .await;
        assert_eq!(ready_guest["status"]["phase"], "Ready");
        assert_eq!(ready_guest["status"]["phase"], "Ready");
        assert_eq!(ready_guest["status"]["phase"], "Ready");

        let delete_volume_ref = ResourceRef::parse("Volume/qemu-delete-runtime").unwrap();
        let delete_process_ref = ResourceRef::parse("Process/qemu-delete-qemu").unwrap();
        wait_for_test_resource(&runtime, &delete_volume_ref, |_| true).await;
        wait_for_test_resource(&runtime, &delete_process_ref, |_| true).await;
        add_test_child_finalizer(&runtime, &delete_process_ref).await;
        request_test_delete(&runtime, &delete_guest_ref).await;
        let deleting_guest = wait_for_test_resource(&runtime, &delete_guest_ref, |value| {
            value["metadata"]["deletionRequestedAt"].is_string()
        })
        .await;
        assert_eq!(
            deleting_guest["metadata"]["finalizers"],
            serde_json::json!([qemu_media_runtime::FINALIZER])
        );
        assert_ne!(deleting_guest["status"]["phase"], "Ready");
        let requested_child = wait_for_test_resource(&runtime, &delete_process_ref, |value| {
            value["metadata"]["deletionRequestedAt"].is_string()
        })
        .await;
        assert_eq!(
            requested_child["metadata"]["finalizers"],
            serde_json::json!(["test.d2bus.org/hold"])
        );
        let child_revision = requested_child["metadata"]["revision"].clone();
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        let still_requested = runtime
        .committed_resource_value(&delete_process_ref, "u6-test-no-second-delete")
            .await
            .unwrap();
        assert_eq!(still_requested["metadata"]["revision"], child_revision);
        let owner_while_child_held = runtime
        .committed_resource_value(&delete_guest_ref, "u6-test-owner-finalizer-retained")
        .await
        .unwrap();
        assert_eq!(
        owner_while_child_held["metadata"]["finalizers"],
        serde_json::json!([qemu_media_runtime::FINALIZER])
        );
        clear_test_child_finalizers(&runtime, &delete_process_ref).await;
        wait_for_test_resource_gone(&runtime, &delete_process_ref).await;
        wait_for_test_resource_gone(&runtime, &delete_volume_ref).await;
        wait_for_test_resource_gone(&runtime, &delete_guest_ref).await;
        drop(runtime);
        close_production_guest_runtime_fixture(state, plane).await;
    }

    #[tokio::test]
    async fn aca_framework_runner_invokes_controller_and_finalizes() {
        let zone = ZoneId::parse("work").unwrap();
        let credential = credential_spec("Guest/gateway");
        let (_directory, state, plane, runtime, broker_evidence) =
            start_production_guest_runner_fixture(vec![
                bundle_resource(
                    "Provider",
                    "runtime-azure-container-apps",
                    &zone,
                    r#"{"artifactId":"runtime-azure-container-apps","config":{"gatewayExecutionRef":"Guest/gateway","tenantId":"tenant","clientId":"client","subscriptionId":"subscription","controlCredentialRef":"Credential/aca-control","pullCredentialRef":null,"environmentId":"environment","resourceGroupId":"resource-group","networkRef":null,"sandboxTransportAlias":"relay","defaults":{"profile":{"profileId":"default","diskImage":{"configuredDisk":{"binding_id":"image-1"}},"cpu":500,"memory":2048,"autoSuspendSecs":300,"sandboxIdentityBindingId":null},"readiness":{"attempts":3,"intervalMs":10},"planTtlMs":1000,"completedOperationCapacity":4}}}"#,
                ),
                bundle_resource("Credential", "aca-control", &zone, &credential),
                bundle_resource(
                    "Guest",
                    "gateway",
                    &zone,
                    r#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"deviceAttachments":[],"networkAttachments":[],"systemArtifactId":null,"volumeAttachmentDefaults":[]}"#,
                ),
                bundle_resource(
                    "Guest",
                    "aca-delete",
                    &zone,
                    r#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"executionRef":"Guest/gateway","deviceAttachments":[],"networkAttachments":[],"providerRef":"Provider/runtime-azure-container-apps","systemArtifactId":null,"volumeAttachmentDefaults":[]}"#,
                ),
                bundle_resource(
                    "Guest",
                    "aca-ready",
                    &zone,
                    r#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"executionRef":"Guest/gateway","deviceAttachments":[],"networkAttachments":[],"providerRef":"Provider/runtime-azure-container-apps","systemArtifactId":null,"volumeAttachmentDefaults":[]}"#,
                ),
            ])
            .await;
        let provider_ref = ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap();
        let credential_ref = ResourceRef::parse("Credential/aca-control").unwrap();
        let gateway_ref = ResourceRef::parse("Guest/gateway").unwrap();

        let delete_guest_ref = ResourceRef::parse("Guest/aca-delete").unwrap();
        let ready_guest_ref = ResourceRef::parse("Guest/aca-ready").unwrap();
        let deleting_guest = wait_for_test_resource(&runtime, &delete_guest_ref, |value| {
            value["metadata"]["finalizers"]
                .as_array()
                .is_some_and(|finalizers| !finalizers.is_empty())
        })
        .await;
        let ready_guest = wait_for_test_resource(&runtime, &ready_guest_ref, |value| {
            value["metadata"]["finalizers"]
                .as_array()
                .is_some_and(|finalizers| !finalizers.is_empty())
        })
        .await;
        assert_eq!(
            deleting_guest["metadata"]["finalizers"],
            serde_json::json!([aca_runtime::FINALIZER])
        );
        assert_eq!(
            ready_guest["metadata"]["finalizers"],
            serde_json::json!([aca_runtime::FINALIZER])
        );
        assert_eq!(deleting_guest["status"]["phase"], "Pending");
        assert_eq!(ready_guest["status"]["phase"], "Pending");
        assert!(
            runtime
                .committed_resource_value(
                    &ResourceRef::parse("Endpoint/aca-delete-sandbox-agent").unwrap(),
                    "u6-test-first-finalizer-only",
                )
                .await
                .is_err()
        );
        assert!(
            runtime
                .committed_resource_value(
                    &ResourceRef::parse("Endpoint/aca-ready-sandbox-agent").unwrap(),
                    "u6-test-first-finalizer-only",
                )
                .await
                .is_err()
        );
        assert_guest_assignment_fence(
            &runtime,
            &delete_guest_ref,
            &provider_ref,
            &ResourceRef::parse("Process/aca-controller").unwrap(),
        )
        .await;
        assert_guest_assignment_fence(
            &runtime,
            &ready_guest_ref,
            &provider_ref,
            &ResourceRef::parse("Process/aca-controller").unwrap(),
        )
        .await;
        mark_test_resource_ready(&runtime, &provider_ref, &broker_evidence).await;
        mark_test_resource_ready(&runtime, &credential_ref, &broker_evidence).await;
        mark_test_resource_ready(&runtime, &gateway_ref, &broker_evidence).await;
        let ready_endpoint_ref = ResourceRef::parse("Endpoint/aca-ready-sandbox-agent").unwrap();
        wait_for_test_resource(&runtime, &ready_endpoint_ref, |_| true).await;
        mark_test_resource_ready(&runtime, &ready_endpoint_ref, &broker_evidence).await;
        let ready_guest = wait_for_test_resource(&runtime, &ready_guest_ref, |value| {
            value["status"]["phase"] == "Ready"
                && value["status"]["observedGeneration"] == value["metadata"]["generation"]
        })
        .await;
        assert_eq!(ready_guest["status"]["phase"], "Ready");

        let delete_endpoint_ref = ResourceRef::parse("Endpoint/aca-delete-sandbox-agent").unwrap();
        wait_for_test_resource(&runtime, &delete_endpoint_ref, |_| true).await;
        add_test_child_finalizer(&runtime, &delete_endpoint_ref).await;
        request_test_delete(&runtime, &delete_guest_ref).await;
        let deleting_guest = wait_for_test_resource(&runtime, &delete_guest_ref, |value| {
            value["metadata"]["deletionRequestedAt"].is_string()
        })
        .await;
        assert_eq!(
            deleting_guest["metadata"]["finalizers"],
            serde_json::json!([aca_runtime::FINALIZER])
        );
        assert_ne!(deleting_guest["status"]["phase"], "Ready");
        let requested_child = wait_for_test_resource(&runtime, &delete_endpoint_ref, |value| {
            value["metadata"]["deletionRequestedAt"].is_string()
        })
        .await;
        assert_eq!(
            requested_child["metadata"]["finalizers"],
            serde_json::json!(["test.d2bus.org/hold"])
        );
        let child_revision = requested_child["metadata"]["revision"].clone();
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        let still_requested = runtime
            .committed_resource_value(&delete_endpoint_ref, "u6-test-no-second-delete")
            .await
            .unwrap();
        assert_eq!(still_requested["metadata"]["revision"], child_revision);
        let owner_while_child_held = runtime
            .committed_resource_value(&delete_guest_ref, "u6-test-owner-finalizer-retained")
            .await
            .unwrap();
        assert_eq!(
            owner_while_child_held["metadata"]["finalizers"],
            serde_json::json!([aca_runtime::FINALIZER])
        );
        clear_test_child_finalizers(&runtime, &delete_endpoint_ref).await;
        wait_for_test_resource_gone(&runtime, &delete_endpoint_ref).await;
        wait_for_test_resource_gone(&runtime, &delete_guest_ref).await;
        drop(runtime);
        close_production_guest_runtime_fixture(state, plane).await;
    }

    #[tokio::test]
    async fn azure_vm_framework_runner_invokes_controller_and_finalizes() {
        let zone = ZoneId::parse("work").unwrap();
        let credential = credential_spec("Guest/gateway");
        let provider_config = azure_vm_runtime::AzureVmConfig {
            tenant_id: Some(d2b_contracts::OpaqueAzureRef::parse("tenant").unwrap()),
            client_id: None,
            arm_credential_ref: ResourceRef::parse("Credential/azure-arm").unwrap(),
            controller_execution_ref: ResourceRef::parse("Guest/gateway").unwrap(),
            network_ref: None,
        };
        let provider_spec = format!(
            r#"{{"artifactId":"runtime-azure-virtual-machine","config":{}}}"#,
            serde_json::to_string(&provider_config).unwrap()
        );
        let guest_spec = r#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"deviceAttachments":[],"networkAttachments":[],"providerRef":"Provider/runtime-azure-virtual-machine","systemArtifactId":null,"volumeAttachmentDefaults":[],"executionRef":"Guest/hold"}"#;
        let ready_guest_spec = guest_spec.replace("Guest/hold", "Guest/gateway");
        let (_directory, state, plane, runtime, broker_evidence) =
            prepare_production_guest_runner_fixture(vec![
                bundle_resource(
                    "Provider",
                    "runtime-azure-virtual-machine",
                    &zone,
                    &provider_spec,
                ),
                bundle_resource("Credential", "azure-arm", &zone, &credential),
                bundle_resource(
                    "Guest",
                    "gateway",
                    &zone,
                    r#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"deviceAttachments":[],"networkAttachments":[],"systemArtifactId":null,"volumeAttachmentDefaults":[]}"#,
                ),
                bundle_resource(
                    "Guest",
                    "hold",
                    &zone,
                    r#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"deviceAttachments":[],"networkAttachments":[],"systemArtifactId":null,"volumeAttachmentDefaults":[]}"#,
                ),
                bundle_resource_with_annotations(
                    "Guest",
                    "azure-vm-delete",
                    &zone,
                    guest_spec,
                    BTreeMap::from([(
                        "d2b.test/azure-vm-settings".to_owned(),
                        "framework".to_owned(),
                    )]),
                ),
                bundle_resource_with_annotations(
                    "Guest",
                    "azure-vm-ready",
                    &zone,
                    &ready_guest_spec,
                    BTreeMap::from([(
                        "d2b.test/azure-vm-settings".to_owned(),
                        "framework".to_owned(),
                    )]),
                ),
            ])
            .await;
        let provider_ref = ResourceRef::parse("Provider/runtime-azure-virtual-machine").unwrap();
        let credential_ref = ResourceRef::parse("Credential/azure-arm").unwrap();
        let gateway_ref = ResourceRef::parse("Guest/gateway").unwrap();
        mark_test_resource_ready(&runtime, &provider_ref, &broker_evidence).await;
        mark_test_resource_ready(&runtime, &credential_ref, &broker_evidence).await;
        mark_test_resource_ready(&runtime, &gateway_ref, &broker_evidence).await;
        wait_for_test_resource(&runtime, &provider_ref, |value| {
            value["status"]["phase"] == "Ready"
        })
        .await;
        wait_for_test_resource(&runtime, &credential_ref, |value| {
            value["status"]["phase"] == "Ready"
        })
        .await;
        wait_for_test_resource(&runtime, &gateway_ref, |value| {
            value["status"]["phase"] == "Ready"
        })
        .await;
        runtime
            .start_u6_controller_runners(Arc::clone(&state))
            .await
            .unwrap();

        let delete_guest_ref = ResourceRef::parse("Guest/azure-vm-delete").unwrap();
        let ready_guest_ref = ResourceRef::parse("Guest/azure-vm-ready").unwrap();
        let deleting_guest = wait_for_test_resource(&runtime, &delete_guest_ref, |value| {
            value["metadata"]["finalizers"]
                .as_array()
                .is_some_and(|finalizers| !finalizers.is_empty())
        })
        .await;
        let ready_guest = wait_for_test_resource(&runtime, &ready_guest_ref, |value| {
            value["metadata"]["finalizers"]
                .as_array()
                .is_some_and(|finalizers| !finalizers.is_empty())
        })
        .await;
        assert_eq!(
            deleting_guest["metadata"]["finalizers"],
            serde_json::json!([azure_vm_runtime::FINALIZER])
        );
        assert_eq!(
            ready_guest["metadata"]["finalizers"],
            serde_json::json!([azure_vm_runtime::FINALIZER])
        );
        assert_eq!(deleting_guest["status"]["phase"], "Pending");
        assert_eq!(ready_guest["status"]["phase"], "Pending");
        assert_guest_assignment_fence(
            &runtime,
            &delete_guest_ref,
            &provider_ref,
            &ResourceRef::parse("Process/azure-vm-controller-process").unwrap(),
        )
        .await;
        assert_guest_assignment_fence(
            &runtime,
            &ready_guest_ref,
            &provider_ref,
            &ResourceRef::parse("Process/azure-vm-controller-process").unwrap(),
        )
        .await;
        let child_ref = ResourceRef::parse("Process/azure-vm-child").unwrap();
        create_test_child(&runtime, &delete_guest_ref, &child_ref).await;
        add_test_child_finalizer(&runtime, &child_ref).await;
        request_test_delete(&runtime, &delete_guest_ref).await;
        let ready_guest = wait_for_test_resource(&runtime, &ready_guest_ref, |value| {
            value["status"]["phase"] == "Ready"
                && value["status"]["observedGeneration"] == value["metadata"]["generation"]
        })
        .await;
        assert_eq!(ready_guest["status"]["phase"], "Ready");
        let deleting_guest = wait_for_test_resource(&runtime, &delete_guest_ref, |value| {
            value["metadata"]["deletionRequestedAt"].is_string()
        })
        .await;
        assert_eq!(
            deleting_guest["metadata"]["finalizers"],
            serde_json::json!([azure_vm_runtime::FINALIZER])
        );
        assert_ne!(deleting_guest["status"]["phase"], "Ready");
        let requested_child = wait_for_test_resource(&runtime, &child_ref, |value| {
            value["metadata"]["deletionRequestedAt"].is_string()
        })
        .await;
        assert_eq!(
            requested_child["metadata"]["finalizers"],
            serde_json::json!(["test.d2bus.org/hold"])
        );
        let child_revision = requested_child["metadata"]["revision"].clone();
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        let still_requested = runtime
            .committed_resource_value(&child_ref, "u6-test-no-second-delete")
            .await
            .unwrap();
        assert_eq!(still_requested["metadata"]["revision"], child_revision);
        let owner_while_child_held = runtime
            .committed_resource_value(&delete_guest_ref, "u6-test-owner-finalizer-retained")
            .await
            .unwrap();
        assert_eq!(
            owner_while_child_held["metadata"]["finalizers"],
            serde_json::json!([azure_vm_runtime::FINALIZER])
        );
        clear_test_child_finalizers(&runtime, &child_ref).await;
        wait_for_test_resource_gone(&runtime, &child_ref).await;
        wait_for_test_resource_gone(&runtime, &delete_guest_ref).await;
        drop(runtime);
        close_production_guest_runtime_fixture(state, plane).await;
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

    /// The refresh fence must still fail closed when a policy input moves
    /// under it: the narrowed comparison only ignores Zone resource-revision
    /// churn, so every policy-snapshot component still yields
    /// `PolicyUnavailable` for the stale/foreign case it exists to reject.
    #[test]
    fn policy_refresh_rejects_a_policy_snapshot_that_moved_under_it() {
        let zone = ZoneId::parse("work").unwrap();
        let base = |policy_revision: u64| StoreRuntimeMetadata {
            store_uid: ResourceUid::parse("11111111-1111-4111-8111-111111111111")
                .expect("store uid"),
            zone_uid: ResourceUid::parse("22222222-2222-4222-8222-222222222222")
                .expect("zone uid"),
            store_epoch: 1,
            current_revision: ZoneRevision::new(7),
            compaction_floor: ZoneRevision::new(2),
            policy_snapshot: PolicySnapshot {
                policy_revision,
                api_catalog_revision: 8,
                active_configuration_revision: ConfigurationGeneration::new(9)
                    .expect("configuration generation"),
                controller_generation: Some(
                    ControllerGeneration::new(10).expect("controller generation"),
                ),
            },
        };
        let loaded = base(7);
        assert_eq!(
            ZoneResourceRuntime::verify_policy_snapshot(&zone, &loaded, &loaded),
            Ok(())
        );
        // The Zone resource revision advancing under the load is not a policy
        // input: derived children and process churn must never gate the
        // mutation.
        let revision_moved = StoreRuntimeMetadata {
            current_revision: ZoneRevision::new(8),
            compaction_floor: ZoneRevision::new(3),
            ..loaded.clone()
        };
        assert_eq!(
            ZoneResourceRuntime::verify_policy_snapshot(&zone, &loaded, &revision_moved),
            Ok(())
        );
        // Every policy-input component still supersedes the loaded rows and
        // fails the mutation closed.
        let policy_moved = base(8);
        let api_catalog_moved = StoreRuntimeMetadata {
            policy_snapshot: PolicySnapshot {
                api_catalog_revision: 9,
                ..loaded.policy_snapshot
            },
            ..loaded.clone()
        };
        let configuration_moved = StoreRuntimeMetadata {
            policy_snapshot: PolicySnapshot {
                active_configuration_revision: ConfigurationGeneration::new(10)
                    .expect("configuration generation"),
                ..loaded.policy_snapshot
            },
            ..loaded.clone()
        };
        let controller_moved = StoreRuntimeMetadata {
            policy_snapshot: PolicySnapshot {
                controller_generation: Some(
                    ControllerGeneration::new(11).expect("controller generation"),
                ),
                ..loaded.policy_snapshot
            },
            ..loaded.clone()
        };
        for (label, moved) in [
            ("policy", policy_moved),
            ("api-catalog", api_catalog_moved),
            ("configuration", configuration_moved),
            ("controller", controller_moved),
        ] {
            assert_eq!(
                ZoneResourceRuntime::verify_policy_snapshot(&zone, &loaded, &moved),
                Err(ResourceRuntimeError::PolicyUnavailable),
                "{label} revision moving under the refresh must fail closed",
            );
        }
    }
}