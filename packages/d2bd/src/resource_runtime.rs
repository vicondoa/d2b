//! Production Zone resource-plane ownership for `d2bd`.
//!
//! A Zone runtime is opened only from the broker's opaque
//! [`d2b_contracts::broker_wire::OpenZoneStoreRequest`]. The broker owns path
//! resolution and returns one
//! close-on-exec database descriptor; this module consumes that descriptor
//! into the production redb backend and never opens a caller-supplied path.
//! The runtime owns the API, core-process readiness, and restart lifecycle as
//! one Zone-scoped value.

use std::{
    collections::BTreeMap,
    fs::File,
    os::fd::OwnedFd,
    sync::{Arc, Mutex},
};

#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

use crate::authority_persistence::RedbAuthorityPersistence;
use d2b_audit::{AuditSink, DurabilityEvidence};
#[cfg(test)]
use d2b_contracts::v3::{
    DEFAULT_LIST_PAGE_SIZE, MAX_FILTER_VALUES, MAX_LIST_FILTERS, MAX_LIST_PAGE_SIZE,
    MAX_LIST_RESOURCE_TYPES, MAX_PAGE_CURSOR_BYTES, MAX_RESPONSE_CANONICAL_BYTES, ResourceName,
    ZoneRevision,
};
use d2b_contracts::{
    broker_wire::{OpenZoneStoreResponse, ZoneStoreDisposition},
    v3::{
        ConfigurationGeneration, ControllerGeneration, ResourceError, ResourceErrorKind,
        ResourceErrorReason, ResourcePhase, ResourceRef, ResourceTypeName, ResourceUid, RetryClass,
        Timestamp, ZoneId, ZoneStatusResource,
    },
};
use d2b_core_controller::authority::{
    AuthorityRequest, AuthorityReservation, ExternalNicClaimRequest, ExternalNicRecoveryInventory,
    ExternalNicReservation, HostGlobalAuthorityIndex, TrustedExternalNicInventory,
};
use d2b_core_controller::authority_persistence::AuthorityRecoveryCoordinator;
use d2b_core_controller::main::{
    CoreProcess, RecoverySnapshot, RuntimeReadiness as CoreRuntimeReadiness, StartupError,
    StartupStage,
};
use d2b_core_controller::zone_status::{SystemCoreStatusEmitter, ZoneStatusInput};
use d2b_resource_api::{
    RedbBackend, ResourceService,
    authz::{ApiCatalog, NativeAuthorizer},
};
use d2b_resource_store::{PolicySnapshot, StoreSlot};
#[cfg(test)]
use d2b_resource_store::{StoreFilter, StoreListResult, StoreProjection};
use d2b_resource_store_redb::{
    BrokerEvidenceIndex, RedbResourceStore, StoreIdentity, StoreRuntimeMetadata,
    write_provisioning_marker,
};
#[cfg(test)]
use serde_json::json;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// Maximum number of Zone runtimes owned by one daemon.
pub const MAX_ZONE_RUNTIMES: usize = 64;

/// Stable production runtime refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceRuntimeError {
    /// The broker response did not describe the requested opaque store.
    BrokerResponseMismatch,
    /// The broker response did not carry exactly one descriptor.
    BrokerFdCountMismatch,
    /// The response disposition was not in the closed contract.
    BrokerDispositionInvalid,
    /// The Zone store id was not the canonical per-Zone id.
    ZoneStoreIdInvalid,
    /// The descriptor was not accepted by the production backend.
    StoreOpenFailed,
    /// The native API authorizer or store seal could not be constructed.
    AuthorizationUnavailable,
    /// The API could not consume its one store admission binding.
    ResourceApiBindFailed,
    /// The runtime could not issue the store-instance seal acceptor.
    StoreSealUnavailable,
    /// The fixed core process could not reach readiness.
    CoreStartupFailed,
    /// The Zone is already owned by this plane.
    DuplicateZone,
    /// The runtime has no ready resource plane.
    PlaneUnavailable,
    /// A CLI request did not match the authoritative Zone route.
    RouteMismatch,
    /// The CLI request is outside the bounded read-only adapter surface.
    RequestInvalid,
    /// The underlying store refused a read.
    StoreReadFailed,
    /// The installed native policy is not available for this Zone.
    PolicyUnavailable,
    /// The production ResourceService endpoint is not registered.
    ControllerEndpointUnavailable,
    /// The authenticated Zone session is not available.
    AuthenticationUnavailable,
    /// The store watch/recovery admission is not available.
    WatchUnavailable,
    /// Durable Host-global authority proofs have not crossed the startup
    /// barrier.
    AuthorityUnavailable,
    /// The fixed core handlers have not converged.
    HandlerNotReady,
    /// The provider path has not completed startup.
    ProviderPathUnavailable,
    /// A shutdown was refused because request owners are still live.
    LiveRequestOwners,
    /// No authenticated subject was bound to the request.
    IdentityUnbound,
    /// The requested operation is not exposed by the registered service.
    CapabilityUnavailable,
}

impl ResourceRuntimeError {
    /// Stable, identity-free error label.
    pub const fn code(self) -> &'static str {
        match self {
            Self::BrokerResponseMismatch => "resource-runtime-broker-response-mismatch",
            Self::BrokerFdCountMismatch => "resource-runtime-broker-fd-count-mismatch",
            Self::BrokerDispositionInvalid => "resource-runtime-broker-disposition-invalid",
            Self::ZoneStoreIdInvalid => "resource-runtime-zone-store-id-invalid",
            Self::StoreOpenFailed => "resource-runtime-store-open-failed",
            Self::AuthorizationUnavailable => "resource-runtime-authorization-unavailable",
            Self::ResourceApiBindFailed => "resource-runtime-api-bind-failed",
            Self::StoreSealUnavailable => "resource-runtime-store-seal-unavailable",
            Self::CoreStartupFailed => "resource-runtime-core-startup-failed",
            Self::DuplicateZone => "resource-runtime-duplicate-zone",
            Self::PlaneUnavailable => "resource-runtime-plane-unavailable",
            Self::RouteMismatch => "resource-runtime-route-mismatch",
            Self::RequestInvalid => "resource-runtime-request-invalid",
            Self::StoreReadFailed => "resource-runtime-store-read-failed",
            Self::PolicyUnavailable => "resource-runtime-policy-unavailable",
            Self::ControllerEndpointUnavailable => {
                "resource-runtime-controller-endpoint-unavailable"
            }
            Self::AuthenticationUnavailable => "resource-runtime-authentication-unavailable",
            Self::WatchUnavailable => "resource-runtime-watch-unavailable",
            Self::AuthorityUnavailable => "resource-runtime-authority-unavailable",
            Self::HandlerNotReady => "resource-runtime-handler-not-ready",
            Self::ProviderPathUnavailable => "resource-runtime-provider-path-unavailable",
            Self::LiveRequestOwners => "resource-runtime-live-request-owners",
            Self::IdentityUnbound => "resource-runtime-identity-unbound",
            Self::CapabilityUnavailable => "resource-runtime-capability-unavailable",
        }
    }
}

impl core::fmt::Display for ResourceRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ResourceRuntimeError {}

/// Broker client result required by [`ZoneResourceRuntime::open`].
pub struct OpenedZoneStore {
    /// Opaque broker response metadata.
    pub response: OpenZoneStoreResponse,
    /// The one owned database descriptor received from the broker.
    pub database_fd: OwnedFd,
    /// Host/bundle-owned trusted external-NIC inventory, when available.
    pub external_inventory: Option<Arc<dyn ExternalNicRecoveryInventory>>,
}

impl core::fmt::Debug for OpenedZoneStore {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OpenedZoneStore")
            .field("response", &self.response)
            .field("has_external_inventory", &self.external_inventory.is_some())
            .finish()
    }
}

/// Readiness projection for one Zone runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneRuntimeReadiness {
    pub store_ready: bool,
    pub resource_api_ready: bool,
    pub local_session_ready: bool,
    pub provider_path_ready: bool,
    pub authority_ready: bool,
    pub core_stage: StartupStage,
}

impl ZoneRuntimeReadiness {
    /// Return true only after every externally visible startup gate is ready.
    pub const fn is_ready(self) -> bool {
        self.store_ready
            && self.resource_api_ready
            && self.local_session_ready
            && self.provider_path_ready
            && self.authority_ready
            && matches!(self.core_stage, StartupStage::Ready)
    }
}

/// A production Resource API and core-controller runtime for one Zone.
pub struct ZoneResourceRuntime {
    zone: ZoneId,
    store_id: String,
    store: Arc<RedbResourceStore>,
    store_metadata: StoreRuntimeMetadata,
    backend: Arc<RedbBackend>,
    api: Arc<ResourceService<RedbBackend>>,
    core: Mutex<CoreProcess>,
    readiness: ZoneRuntimeReadiness,
    policy_installed: bool,
    controller_endpoint_registered: bool,
    watch_admitted: bool,
    authority_index: Arc<tokio::sync::Mutex<HostGlobalAuthorityIndex>>,
    authority_persistence: Arc<RedbAuthorityPersistence>,
    authority_recovery: Arc<AuthorityRecoveryCoordinator>,
    zone_status: Mutex<ZoneStatusResource>,
}

impl core::fmt::Debug for ZoneResourceRuntime {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ZoneResourceRuntime")
            .field("zone", &self.zone)
            .field("store_id", &"<opaque>")
            .field("current_revision", &self.store_metadata.current_revision)
            .field("readiness", &self.readiness)
            .finish()
    }
}

impl ZoneResourceRuntime {
    /// Open one Zone from a broker-owned descriptor.
    pub async fn open(zone: ZoneId, opened: OpenedZoneStore) -> Result<Self, ResourceRuntimeError> {
        Self::open_internal(
            zone,
            opened,
            None,
            Arc::new(BrokerEvidenceIndex::default()),
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
        Self::open_internal(zone, opened, Some(audit_sink), broker_evidence, None).await
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
        )
        .await
    }

    async fn open_internal(
        zone: ZoneId,
        opened: OpenedZoneStore,
        audit_sink: Option<Arc<AuditSink>>,
        broker_evidence: Arc<BrokerEvidenceIndex>,
        telemetry_path: Option<std::path::PathBuf>,
    ) -> Result<Self, ResourceRuntimeError> {
        #[cfg(test)]
        let audit_sink = audit_sink.or_else(|| {
            let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join(format!(
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
        )
        .await
    }

    async fn open_with_external_inventory_and_audit(
        zone: ZoneId,
        opened: OpenedZoneStore,
        external_inventory: Arc<dyn ExternalNicRecoveryInventory>,
        audit_sink: Option<Arc<AuditSink>>,
        broker_evidence: Arc<BrokerEvidenceIndex>,
        telemetry_path: Option<std::path::PathBuf>,
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

        let store_identity = store_identity(&zone, &opened.response.store_identity)?;
        let authorizer = Arc::new(runtime_authorizer()?);
        let acceptor = authorizer
            .take_store_seal(store_identity.seal_identity())
            .map_err(|_| ResourceRuntimeError::StoreSealUnavailable)?;
        let disposition = opened.response.disposition;
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
        let store_metadata = store
            .runtime_metadata()
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        let backend = Arc::new(RedbBackend::from_arc(Arc::clone(&store)));
        let api = Arc::new(
            ResourceService::new(Arc::clone(&backend), Arc::clone(&authorizer))
                .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?,
        );

        let mut core = CoreProcess::new();
        {
            let recovered_authority = authority_index.lock().await;
            let _ = drive_core_startup(
                &mut core,
                CoreRuntimeReadiness {
                    store_ready: true,
                    resource_api_ready: false,
                    local_bus_ready: false,
                    controller_endpoint_registered: false,
                    authenticated_system_core_session: false,
                },
                RecoverySnapshot {
                    startup_epoch: 0,
                    checkpoint_revision: 0,
                    active_configuration_revision: store_metadata
                        .policy_snapshot
                        .active_configuration_revision
                        .get(),
                    provider_lease_count: 0,
                    controller_lease_count: 0,
                    ambiguous_operation_count: 0,
                    watch_admitted: false,
                },
                &recovered_authority,
            );
        }
        let stage = core.stage();
        let zone_status = SystemCoreStatusEmitter::new()
            .emit(ZoneStatusInput::new(ResourcePhase::Pending, Vec::new()))
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        Ok(Self {
            zone,
            store_id: expected_store_id,
            store,
            store_metadata,
            backend,
            api,
            core: Mutex::new(core),
            readiness: ZoneRuntimeReadiness {
                store_ready: true,
                resource_api_ready: false,
                local_session_ready: false,
                provider_path_ready: false,
                authority_ready: true,
                core_stage: stage,
            },
            policy_installed: false,
            controller_endpoint_registered: false,
            watch_admitted: false,
            authority_index,
            authority_persistence,
            authority_recovery,
            zone_status: Mutex::new(zone_status),
        })
    }

    /// Borrow the authoritative Zone identity.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the opaque store id used for the broker request.
    pub fn store_id(&self) -> &str {
        &self.store_id
    }

    /// Return the startup readiness projection.
    pub const fn readiness(&self) -> ZoneRuntimeReadiness {
        self.readiness
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

    /// Mark the trusted Provider path after the daemon has configured it.
    ///
    /// Provider configuration is loaded outside this Zone store boundary, so
    /// `open` cannot claim this bit from the descriptor alone.
    pub(crate) fn set_provider_path_ready(&mut self, ready: bool) {
        self.readiness.provider_path_ready = ready;
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
    /// This compatibility entry point deliberately has no authenticated
    /// session argument. Resource operations therefore return a typed
    /// authorization error instead of borrowing a daemon-owned Provider
    /// identity.
    pub async fn dispatch_cli_request(
        &self,
        request: &Value,
    ) -> Result<Value, ResourceRuntimeError> {
        self.dispatch_public_cli_request(request).await
    }

    /// Refuse the public compatibility route until an authenticated
    /// ComponentSession is registered by the ZoneRegistrar.
    pub async fn dispatch_public_cli_request(
        &self,
        request: &Value,
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
        Ok(resource_error_envelope(&readiness_resource_error(
            ResourceRuntimeError::AuthenticationUnavailable,
        )))
    }

    /// Publish terminal broker evidence into the live store join index.
    pub fn ingest_broker_evidence(
        &self,
        evidence: DurabilityEvidence,
    ) -> Result<(), ResourceRuntimeError> {
        self.store
            .ingest_broker_evidence(evidence)
            .map_err(|_| ResourceRuntimeError::StoreOpenFailed)
    }

    /// Close the production redb workers before the runtime is discarded.
    pub async fn shutdown(self) -> Result<(), ResourceRuntimeError> {
        let ZoneResourceRuntime {
            store,
            backend,
            api,
            authority_persistence,
            authority_recovery,
            ..
        } = self;
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

fn route_service_matches(
    service: Option<&Value>,
    method: &str,
) -> Result<bool, ResourceRuntimeError> {
    let Some(service) = service else {
        return Ok(true);
    };
    let service = service
        .as_str()
        .ok_or(ResourceRuntimeError::RequestInvalid)?;
    Ok(match method {
        "ZoneList" | "ZoneStatus" => service == "d2b.zone.v3",
        _ => service == "d2b.resource.v3",
    })
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedListRequest {
    resource_types: Vec<ResourceTypeName>,
    resource_names: Vec<ResourceName>,
    filters: Vec<StoreFilter>,
    page_size: u32,
    cursor: Option<String>,
    projection: StoreProjection,
}

#[cfg(test)]
fn parse_list_request(request: &Value) -> Result<ParsedListRequest, ResourceRuntimeError> {
    const LIST_FIELDS: &[&str] = &[
        "service",
        "method",
        "zoneRef",
        "schemaVersion",
        "sessionVerb",
        "operationId",
        "resourceType",
        "resourceTypes",
        "resourceNames",
        "filters",
        "limit",
        "pageSize",
        "pageToken",
        "cursor",
        "pageCursor",
        "executionRef",
        "domain",
        "phase",
        "labelSelector",
        "updates",
        "revisionCursor",
        "sinceRevision",
        "afterRevision",
        "revision",
        "projection",
    ];
    if request.as_object().is_none_or(|object| {
        object
            .keys()
            .any(|key| !LIST_FIELDS.contains(&key.as_str()))
    }) {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    let singular_type = request
        .get("resourceType")
        .and_then(Value::as_str)
        .map(|value| {
            ResourceTypeName::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)
        })
        .transpose()?;
    let resource_types = match request.get("resourceTypes") {
        None | Some(Value::Null) => singular_type
            .clone()
            .map(|resource_type| vec![resource_type])
            .ok_or(ResourceRuntimeError::RequestInvalid)?,
        Some(value) => {
            let values = value
                .as_array()
                .ok_or(ResourceRuntimeError::RequestInvalid)?;
            if values.is_empty() || values.len() > MAX_LIST_RESOURCE_TYPES {
                return Err(ResourceRuntimeError::RequestInvalid);
            }
            let parsed = values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .ok_or(ResourceRuntimeError::RequestInvalid)
                        .and_then(|value| {
                            ResourceTypeName::parse(value)
                                .map_err(|_| ResourceRuntimeError::RequestInvalid)
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(singular_type) = singular_type
                && (parsed.len() != 1 || parsed[0] != singular_type)
            {
                return Err(ResourceRuntimeError::RequestInvalid);
            }
            parsed
        }
    };

    let mut resource_names = parse_resource_names(request)?;
    let mut filters = parse_typed_filters(request)?;
    for filter in &filters {
        if filter.field == "metadata.name" {
            resource_names.extend(
                filter
                    .values
                    .iter()
                    .map(|value| {
                        ResourceName::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
    }
    if let Some(selector) = request.get("labelSelector")
        && !selector.is_null()
    {
        let selector = selector
            .as_str()
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if !selector.is_empty() {
            let (field, value) = selector
                .split_once('=')
                .ok_or(ResourceRuntimeError::RequestInvalid)?;
            if filters.len() >= MAX_LIST_FILTERS {
                return Err(ResourceRuntimeError::RequestInvalid);
            }
            let filter = typed_filter(field, vec![value.to_owned()])?;
            if field == "metadata.name" {
                resource_names.extend(
                    filter
                        .values
                        .iter()
                        .map(|value| {
                            ResourceName::parse(value)
                                .map_err(|_| ResourceRuntimeError::RequestInvalid)
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            filters.push(filter);
        }
    }
    if let Some(domain) = request.get("domain").and_then(Value::as_str)
        && !domain.is_empty()
        && !matches!(domain, "system" | "user")
    {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    if let Some(execution_ref) = request.get("executionRef")
        && !execution_ref.is_null()
    {
        let execution_ref = execution_ref
            .as_str()
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if !execution_ref.is_empty() {
            ResourceRef::parse(execution_ref).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
    }
    optional_capability_string(request, "domain", 16)?;
    optional_capability_string(request, "phase", 128)?;
    if let Some(schema_version) = request.get("schemaVersion")
        && !schema_version.is_null()
        && schema_version.as_u64() != Some(1)
    {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    if let Some(session_verb) = request.get("sessionVerb")
        && !session_verb.is_null()
        && session_verb.as_str() != Some("Invoke")
    {
        return Err(ResourceRuntimeError::CapabilityUnavailable);
    }
    if request.get("updates").is_some_and(|value| !value.is_null()) {
        let updates = request
            .get("updates")
            .and_then(Value::as_bool)
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if updates {
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
    }

    let cursor = aliased_cursor(request)?;
    let page_size = aliased_page_size(request)?;
    let projection = parse_projection(request.get("projection"))?;
    for field in [
        "revisionCursor",
        "sinceRevision",
        "afterRevision",
        "revision",
    ] {
        if let Some(value) = request.get(field) {
            if value.is_null() {
                continue;
            }
            let value = value.as_u64().ok_or(ResourceRuntimeError::RequestInvalid)?;
            if value != 0 {
                return Err(ResourceRuntimeError::CapabilityUnavailable);
            }
        }
    }

    Ok(ParsedListRequest {
        resource_types,
        resource_names,
        filters,
        page_size,
        cursor,
        projection,
    })
}

#[cfg(test)]
fn parse_resource_names(request: &Value) -> Result<Vec<ResourceName>, ResourceRuntimeError> {
    let Some(value) = request.get("resourceNames") else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let values = value
        .as_array()
        .ok_or(ResourceRuntimeError::RequestInvalid)?;
    if values.len() > MAX_FILTER_VALUES {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or(ResourceRuntimeError::RequestInvalid)
                .and_then(|value| {
                    ResourceName::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)
                })
        })
        .collect()
}

#[cfg(test)]
fn parse_typed_filters(request: &Value) -> Result<Vec<StoreFilter>, ResourceRuntimeError> {
    let Some(value) = request.get("filters") else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let values = value
        .as_array()
        .ok_or(ResourceRuntimeError::RequestInvalid)?;
    if values.len() > MAX_LIST_FILTERS {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    values
        .iter()
        .map(|value| {
            let object = value
                .as_object()
                .ok_or(ResourceRuntimeError::RequestInvalid)?;
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "field" | "values"))
            {
                return Err(ResourceRuntimeError::RequestInvalid);
            }
            let field = object
                .get("field")
                .and_then(Value::as_str)
                .ok_or(ResourceRuntimeError::RequestInvalid)?;
            let values = object
                .get("values")
                .and_then(Value::as_array)
                .ok_or(ResourceRuntimeError::RequestInvalid)?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .ok_or(ResourceRuntimeError::RequestInvalid)
                })
                .collect::<Result<Vec<_>, _>>()?;
            typed_filter(field, values)
        })
        .collect()
}

#[cfg(test)]
fn typed_filter(field: &str, values: Vec<String>) -> Result<StoreFilter, ResourceRuntimeError> {
    if field.is_empty()
        || field.len() > 64
        || !field
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        || values.is_empty()
        || values.len() > MAX_FILTER_VALUES
        || values
            .iter()
            .any(|value| value.is_empty() || value.len() > 256)
    {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    if !matches!(field, "metadata.name" | "type") {
        return Err(ResourceRuntimeError::CapabilityUnavailable);
    }
    if field == "metadata.name" {
        for value in &values {
            ResourceName::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
        }
    } else {
        for value in &values {
            ResourceTypeName::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
        }
    }
    Ok(StoreFilter {
        field: field.to_owned(),
        values,
    })
}

#[cfg(test)]
fn optional_capability_string(
    request: &Value,
    field: &str,
    max_bytes: usize,
) -> Result<(), ResourceRuntimeError> {
    let Some(value) = request.get(field) else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    let value = value.as_str().ok_or(ResourceRuntimeError::RequestInvalid)?;
    if value.len() > max_bytes {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    if !value.is_empty() {
        return Err(ResourceRuntimeError::CapabilityUnavailable);
    }
    Ok(())
}

#[cfg(test)]
fn aliased_cursor(request: &Value) -> Result<Option<String>, ResourceRuntimeError> {
    let mut values = Vec::new();
    for field in ["cursor", "pageCursor", "pageToken"] {
        let Some(value) = request.get(field) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let value = value.as_str().ok_or(ResourceRuntimeError::RequestInvalid)?;
        if value.len() > MAX_PAGE_CURSOR_BYTES {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        values.push(value.to_owned());
    }
    if values.windows(2).any(|pair| pair[0] != pair[1]) {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    Ok(values.into_iter().next().filter(|value| !value.is_empty()))
}

#[cfg(test)]
fn aliased_page_size(request: &Value) -> Result<u32, ResourceRuntimeError> {
    let mut values = Vec::new();
    for field in ["pageSize", "limit"] {
        let Some(value) = request.get(field) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let value = value.as_u64().ok_or(ResourceRuntimeError::RequestInvalid)?;
        if value == 0 || value > u64::from(MAX_LIST_PAGE_SIZE) {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        values.push(u32::try_from(value).map_err(|_| ResourceRuntimeError::RequestInvalid)?);
    }
    if values.windows(2).any(|pair| pair[0] != pair[1]) {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    Ok(values.first().copied().unwrap_or(DEFAULT_LIST_PAGE_SIZE))
}

#[cfg(test)]
fn parse_projection(value: Option<&Value>) -> Result<StoreProjection, ResourceRuntimeError> {
    let Some(value) = value else {
        return Ok(StoreProjection::Full);
    };
    if value.is_null() {
        return Ok(StoreProjection::Full);
    }
    let kind = match value {
        Value::String(value) => value.as_str(),
        Value::Object(object) => {
            if object.len() != 1 {
                return Err(ResourceRuntimeError::RequestInvalid);
            }
            object
                .get("kind")
                .and_then(Value::as_str)
                .ok_or(ResourceRuntimeError::RequestInvalid)?
        }
        _ => return Err(ResourceRuntimeError::RequestInvalid),
    };
    match kind {
        "full" | "FULL" | "projection-kind-full" => Ok(StoreProjection::Full),
        "baseOnly" | "base-only" | "BASE_ONLY" => Ok(StoreProjection::BaseOnly),
        "metadataOnly" | "metadata-only" | "METADATA_ONLY" => Ok(StoreProjection::MetadataOnly),
        _ => Err(ResourceRuntimeError::RequestInvalid),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn drive_core_startup(
    core: &mut CoreProcess,
    readiness: CoreRuntimeReadiness,
    recovery: RecoverySnapshot,
    authority_index: &HostGlobalAuthorityIndex,
) -> Result<StartupStage, ResourceRuntimeError> {
    core.start_production(readiness, recovery, authority_index)
        .map_err(map_startup_error)?;
    core.publish_readiness().map_err(map_startup_error)
}

fn map_startup_error(error: StartupError) -> ResourceRuntimeError {
    match error {
        StartupError::ControllerEndpointUnavailable => {
            ResourceRuntimeError::ControllerEndpointUnavailable
        }
        StartupError::AuthenticationUnavailable => ResourceRuntimeError::AuthenticationUnavailable,
        StartupError::WatchAdmissionUnavailable => ResourceRuntimeError::WatchUnavailable,
        StartupError::AuthorityRehydrationUnavailable => ResourceRuntimeError::AuthorityUnavailable,
        StartupError::MandatoryHandlerNotReady => ResourceRuntimeError::HandlerNotReady,
        StartupError::RuntimeNotReady | StartupError::InvalidRecoverySnapshot => {
            ResourceRuntimeError::CoreStartupFailed
        }
    }
}

fn readiness_resource_error(error: ResourceRuntimeError) -> ResourceError {
    let (kind, retry_class, reason) = match error {
        ResourceRuntimeError::PolicyUnavailable => (
            ResourceErrorKind::AuthorizationDenied,
            RetryClass::Reauthorize,
            "installed resource policy is unavailable",
        ),
        ResourceRuntimeError::AuthenticationUnavailable => (
            ResourceErrorKind::AuthorizationDenied,
            RetryClass::Reauthorize,
            "ZoneRegistrar ComponentSession route is unavailable",
        ),
        ResourceRuntimeError::IdentityUnbound => (
            ResourceErrorKind::AuthorizationDenied,
            RetryClass::Reauthorize,
            "authenticated resource subject is unavailable",
        ),
        ResourceRuntimeError::ControllerEndpointUnavailable => (
            ResourceErrorKind::ResourcePlaneUnavailable,
            RetryClass::AfterDelay,
            "registered Resource API endpoint is unavailable",
        ),
        ResourceRuntimeError::WatchUnavailable
        | ResourceRuntimeError::AuthorityUnavailable
        | ResourceRuntimeError::HandlerNotReady => (
            ResourceErrorKind::ResourcePlaneUnavailable,
            RetryClass::AfterDelay,
            "zone resource runtime is still converging",
        ),
        _ => (
            ResourceErrorKind::ResourcePlaneUnavailable,
            RetryClass::AfterDelay,
            "zone resource runtime is unavailable",
        ),
    };
    ResourceError::new(
        kind,
        None,
        None,
        retry_class,
        ResourceErrorReason::parse(reason).expect("fixed readiness error reason"),
    )
    .expect("fixed readiness error fields")
}

#[cfg(test)]
fn resource_result_error(reason: &'static str) -> ResourceError {
    ResourceError::terminal(ResourceErrorKind::InternalIntegrityFailure, reason)
}

#[cfg(test)]
fn decode_resource_result(bytes: &[u8]) -> Result<Value, ResourceError> {
    if bytes.len() > MAX_RESPONSE_CANONICAL_BYTES {
        return Err(resource_result_error(
            "resource result exceeds its byte bound",
        ));
    }
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| resource_result_error("resource result is malformed"))?;
    if !value.is_object() {
        return Err(resource_result_error("resource result is not an object"));
    }
    Ok(value)
}

#[cfg(test)]
fn encode_list_result(result: StoreListResult) -> Result<Value, ResourceError> {
    let resources = result
        .resources
        .iter()
        .map(|resource| decode_resource_result(&resource.canonical_json))
        .collect::<Result<Vec<_>, _>>()?;
    if result
        .next_cursor
        .as_ref()
        .is_some_and(|cursor| cursor.len() > MAX_PAGE_CURSOR_BYTES)
    {
        return Err(resource_result_error(
            "resource result cursor exceeds its byte bound",
        ));
    }
    let mut response = Map::new();
    response.insert("resources".to_owned(), Value::Array(resources));
    response.insert(
        "snapshotRevision".to_owned(),
        Value::Number(result.snapshot_revision.get().into()),
    );
    response.insert("truncated".to_owned(), Value::Bool(result.truncated));
    if let Some(cursor) = result.next_cursor {
        response.insert("nextCursor".to_owned(), Value::String(cursor));
    }
    let value = Value::Object(response);
    let encoded = serde_json::to_vec(&value)
        .map_err(|_| resource_result_error("resource result could not be encoded"))?;
    if encoded.len() > MAX_RESPONSE_CANONICAL_BYTES {
        return Err(resource_result_error(
            "resource list result exceeds its byte bound",
        ));
    }
    Ok(value)
}

fn resource_error_envelope(error: &ResourceError) -> Value {
    let mut body = Map::new();
    body.insert(
        "kind".to_owned(),
        Value::String(error.kind().as_str().to_owned()),
    );
    body.insert(
        "errorClass".to_owned(),
        Value::String(error.kind().as_str().to_owned()),
    );
    body.insert(
        "retryClass".to_owned(),
        Value::String(retry_class_name(error.retry_class()).to_owned()),
    );
    body.insert(
        "message".to_owned(),
        Value::String(error.reason().as_str().to_owned()),
    );
    body.insert(
        "remediation".to_owned(),
        Value::String(resource_error_remediation(error.kind()).to_owned()),
    );
    if let Some(revision) = error.current_revision() {
        body.insert(
            "currentRevision".to_owned(),
            Value::Number(revision.get().into()),
        );
    }
    if let Some(retry_after_ms) = error.retry_after_ms() {
        body.insert(
            "retryAfterMs".to_owned(),
            Value::Number(retry_after_ms.into()),
        );
    }
    let mut envelope = Map::new();
    envelope.insert("type".to_owned(), Value::String("error".to_owned()));
    envelope.insert("error".to_owned(), Value::Object(body));
    Value::Object(envelope)
}

const fn retry_class_name(retry_class: RetryClass) -> &'static str {
    match retry_class {
        RetryClass::Never => "never",
        RetryClass::Immediate => "immediate",
        RetryClass::AfterDelay => "after-delay",
        RetryClass::Reauthorize => "reauthorize",
    }
}

const fn resource_error_remediation(kind: ResourceErrorKind) -> &'static str {
    match kind {
        ResourceErrorKind::AuthorizationDenied => {
            "authenticate an exact local Zone session and install its matching policy before retrying"
        }
        ResourceErrorKind::UnsupportedCapability => {
            "use a method exposed by the registered Zone service"
        }
        ResourceErrorKind::ResourcePlaneUnavailable => {
            "wait for Zone runtime readiness and retry after the authoritative plane is published"
        }
        ResourceErrorKind::InternalIntegrityFailure => "repair the resource result before retrying",
        _ => "follow the typed resource error retry policy",
    }
}

/// All Zone runtimes owned by one daemon.
#[derive(Default)]
pub struct ResourcePlane {
    zones: BTreeMap<ZoneId, Arc<ZoneResourceRuntime>>,
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
    pub const fn new() -> Self {
        Self {
            zones: BTreeMap::new(),
        }
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

    /// Ingest one terminal broker result into every Zone's shared live index.
    pub fn ingest_broker_evidence(
        &self,
        evidence: DurabilityEvidence,
    ) -> Result<(), ResourceRuntimeError> {
        for runtime in self.zones.values() {
            runtime.ingest_broker_evidence(evidence.clone())?;
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

fn runtime_authorizer() -> Result<NativeAuthorizer, ResourceRuntimeError> {
    NativeAuthorizer::new(ApiCatalog::standard(), None)
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)
}

fn store_identity(
    zone: &ZoneId,
    store_identity: &str,
) -> Result<StoreIdentity, ResourceRuntimeError> {
    let store_uuid = stable_uid("store", store_identity);
    let zone_uid = stable_uid("zone", zone.as_str());
    let created_at = Timestamp::parse("1970-01-01T00:00:00.000Z")
        .map_err(|_| ResourceRuntimeError::StoreOpenFailed)?;
    let revisions = PolicySnapshot {
        policy_revision: 0,
        api_catalog_revision: 1,
        active_configuration_revision: ConfigurationGeneration::new(1)
            .map_err(|_| ResourceRuntimeError::StoreOpenFailed)?,
        controller_generation: Some(
            ControllerGeneration::new(1).map_err(|_| ResourceRuntimeError::StoreOpenFailed)?,
        ),
    };
    Ok(StoreIdentity::new(
        StoreSlot::new(0).map_err(|_| ResourceRuntimeError::StoreOpenFailed)?,
        store_uuid,
        zone.clone(),
        zone_uid,
        created_at,
        revisions,
    ))
}

fn stable_uid(domain: &str, value: &str) -> ResourceUid {
    let mut digest = Sha256::new();
    digest.update(domain.as_bytes());
    digest.update([0]);
    digest.update(value.as_bytes());
    let mut bytes: [u8; 16] = digest.finalize()[..16]
        .try_into()
        .expect("fixed digest slice");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let rendered = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    );
    ResourceUid::parse(rendered).expect("stable UUID is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::OpenOptions, os::fd::AsRawFd, sync::Arc};

    use d2b_resource_store::mutation_seal::mutation_seal_pair;
    use d2b_resource_store_redb::write_provisioning_marker;

    fn test_audit_sink(directory: &std::path::Path, name: &str) -> Arc<AuditSink> {
        Arc::new(AuditSink::open(directory.join(name)).unwrap())
    }

    #[test]
    fn stable_identity_is_repeatable_and_uuid_v4_shaped() {
        let first = stable_uid("store", "sha256:aaa");
        assert_eq!(first, stable_uid("store", "sha256:aaa"));
        assert_ne!(first, stable_uid("store", "sha256:bbb"));
    }

    #[test]
    fn core_progression_reaches_handler_gate_before_readiness_check() {
        let mut core = CoreProcess::new();
        let authority = HostGlobalAuthorityIndex::new_for_tests_ready();
        let result = drive_core_startup(
            &mut core,
            CoreRuntimeReadiness {
                store_ready: true,
                resource_api_ready: true,
                local_bus_ready: true,
                controller_endpoint_registered: true,
                authenticated_system_core_session: true,
            },
            RecoverySnapshot {
                startup_epoch: 0,
                checkpoint_revision: 0,
                active_configuration_revision: 1,
                provider_lease_count: 0,
                controller_lease_count: 0,
                ambiguous_operation_count: 0,
                watch_admitted: true,
            },
            &authority,
        );
        assert_eq!(result, Err(ResourceRuntimeError::HandlerNotReady));
        assert_eq!(core.stage(), StartupStage::ReconcilingSystemCore);
    }

    #[test]
    fn broker_response_requires_one_canonical_zone_store() {
        let response = OpenZoneStoreResponse {
            zone_store_id: d2b_contracts::v3::storage::ZoneStoreId::parse("zone-store-work")
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

    #[test]
    fn list_preserves_typed_pagination_and_filters() {
        let request = json!({
            "resourceType": "Guest",
            "limit": 10,
            "pageToken": "opaque-cursor",
            "filters": [{
                "field": "metadata.name",
                "values": ["corp-vm"],
            }],
        });
        let parsed = parse_list_request(&request).unwrap();
        assert_eq!(parsed.page_size, 10);
        assert_eq!(parsed.cursor.as_deref(), Some("opaque-cursor"));
        assert_eq!(parsed.resource_types[0].as_str(), "Guest");
        assert_eq!(parsed.resource_names[0].as_str(), "corp-vm");
        assert_eq!(parsed.filters[0].field, "metadata.name");
    }

    #[test]
    fn list_refuses_query_fields_without_a_store_semantic() {
        let request = json!({
            "resourceType": "Guest",
            "executionRef": "Host/host-system",
        });
        assert_eq!(
            parse_list_request(&request),
            Err(ResourceRuntimeError::CapabilityUnavailable)
        );
    }

    #[test]
    fn list_rejects_conflicting_legacy_and_typed_pagination_aliases() {
        let request = json!({
            "resourceType": "Guest",
            "limit": 10,
            "pageSize": 20,
            "pageToken": "opaque-cursor",
            "cursor": "different-cursor",
        });
        assert_eq!(
            parse_list_request(&request),
            Err(ResourceRuntimeError::RequestInvalid)
        );
    }

    #[test]
    fn malformed_resource_results_fail_closed() {
        assert_eq!(
            decode_resource_result(br#"{"unterminated":"value""#)
                .unwrap_err()
                .kind(),
            ResourceErrorKind::InternalIntegrityFailure
        );
        assert_eq!(
            decode_resource_result(&vec![b' '; MAX_RESPONSE_CANONICAL_BYTES + 1])
                .unwrap_err()
                .kind(),
            ResourceErrorKind::InternalIntegrityFailure
        );
    }

    #[test]
    fn list_result_retains_the_store_cursor() {
        let result = encode_list_result(StoreListResult {
            resources: Vec::new(),
            snapshot_revision: ZoneRevision::new(7),
            next_cursor: Some("opaque-cursor".to_owned()),
            truncated: true,
        })
        .unwrap();
        assert_eq!(result["snapshotRevision"], 7);
        assert_eq!(result["nextCursor"], "opaque-cursor");
        assert!(result.get("nextPageToken").is_none());
        assert_eq!(result["truncated"], true);
    }

    #[test]
    fn resource_error_envelope_retains_kind_and_retry_metadata() {
        let error = ResourceError::new(
            ResourceErrorKind::ResourceConflict,
            Some(ZoneRevision::new(11)),
            Some(250),
            RetryClass::AfterDelay,
            d2b_contracts::v3::ResourceErrorReason::parse("revision-changed").unwrap(),
        )
        .unwrap();
        let envelope = resource_error_envelope(&error);
        assert_eq!(envelope["error"]["kind"], "resource-conflict");
        assert_eq!(envelope["error"]["currentRevision"], 11);
        assert_eq!(envelope["error"]["retryAfterMs"], 250);
        assert_eq!(envelope["error"]["retryClass"], "after-delay");
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
                    zone_store_id: d2b_contracts::v3::storage::ZoneStoreId::parse(
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
                    zone_store_id: d2b_contracts::v3::storage::ZoneStoreId::parse(
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
                    zone_store_id: d2b_contracts::v3::storage::ZoneStoreId::parse(
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
    async fn public_reads_remain_fail_closed_after_restart_revisions_rehydrate() {
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
                    zone_store_id: d2b_contracts::v3::storage::ZoneStoreId::parse(
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

        let peer_route = runtime
            .dispatch_public_cli_request(&json!({
                "method": "List",
                "zoneRef": "Zone/work",
                "resourceType": "Host",
            }))
            .await
            .unwrap();
        assert_eq!(
            peer_route["error"]["kind"], "authorization-denied",
            "local lifecycle admission must not mint a Resource API subject"
        );
        assert_eq!(
            peer_route["error"]["message"],
            "ZoneRegistrar ComponentSession route is unavailable"
        );
        let direct_delete = runtime
            .dispatch_public_cli_request(&json!({
                "method": "Delete",
                "zoneRef": "Zone/work",
                "resourceRef": "Host/host-system",
            }))
            .await
            .unwrap();
        assert_eq!(direct_delete["error"]["kind"], "authorization-denied");
        runtime.shutdown().await.unwrap();
    }
}
