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
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Read},
    os::fd::OwnedFd,
    os::unix::fs::FileTypeExt,
    path::Path,
    sync::{Arc, Mutex},
};

use std::time::{SystemTime, UNIX_EPOCH};

use crate::activation_resource_runtime::{
    ActivationResourceRuntime, ActivationResourceRuntimeError, activation_watch_request,
    list_activation_snapshot, run_activation_watch,
};
use crate::audio_resource_runtime::{
    AudioBindingRuntimeStatus, AudioResourceRuntime, AudioResourceRuntimeError,
    audio_binding_status_value, audio_watch_request, list_audio_snapshot, remove_audio_finalizer,
    run_audio_watch,
};
use crate::authority_persistence::RedbAuthorityPersistence;
use crate::process_resource_runtime::{
    ProcessResourceRuntime, ProcessResourceRuntimeError, list_process_snapshot,
    process_watch_request, run_process_watch,
};
use crate::resource_operator_activation::{
    Wave6AcceptanceReport, Wave6Dependencies, Wave6ProviderBoundary, Wave6ReconcileResult,
    select_wave6_resources,
};
use d2b_audit::{AuditSink, DurabilityEvidence};
use d2b_bus::{BusAuthorizer, BusConfig, BusIngress, ZoneBus, ZoneRegistrar};
use d2b_contracts::v3::identity::STANDARD_RESOURCE_TYPES;
use d2b_contracts::v3::provider::ProviderSpec;
use d2b_contracts::v3::{
    DEFAULT_LIST_PAGE_SIZE, MAX_FILTER_VALUES, MAX_LIST_FILTERS, MAX_LIST_PAGE_SIZE,
    MAX_LIST_RESOURCE_TYPES, MAX_PAGE_CURSOR_BYTES, MAX_RESPONSE_CANONICAL_BYTES,
};
use d2b_contracts::v3::{
    host::{HOST_PROVIDER_REF, HostSpec},
    user::UserSpec,
};
use d2b_contracts::{
    broker_wire::{OpenZoneStoreResponse, ZoneStoreDisposition},
    resource_proto as wire,
    v3::{
        AuthenticatedSubjectContext, BindingDigest, CanonicalJsonValue, ConfigurationGeneration,
        ControllerGeneration, EvidenceClass, Locality as IdentityLocality, ReconnectGeneration,
        ResourceBundle, ResourceEnvelope, ResourceError, ResourceErrorKind, ResourceErrorReason,
        ResourceGeneration, ResourceName, ResourcePhase, ResourceRef, ResourceTypeName,
        ResourceUid, RetryClass, SchemaFingerprint, ServiceName, SessionBinding, SessionPurpose,
        Timestamp, TranscriptHash, TransportBinding, ZoneId, ZoneRevision, ZoneStatusResource,
        component_session::{
            AttachmentPolicy, EndpointPolicy, EndpointPurpose, EndpointRole,
            IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, PurposeClass,
            ServicePackage, TransportBinding as ComponentTransportBinding, TransportClass,
        },
    },
};
use d2b_core_controller::authority::{
    AuthorityRequest, AuthorityReservation, ExternalNicClaimRequest, ExternalNicRecoveryInventory,
    ExternalNicReservation, HostGlobalAuthorityIndex, TrustedExternalNicInventory,
};
use d2b_core_controller::authority_persistence::AuthorityRecoveryCoordinator;
use d2b_core_controller::controllers::{
    CoreHandlerKind, HandlerOutcome, HandlerPhase, HandlerStatus,
};
use d2b_core_controller::main::{
    CoreProcess, RecoverySnapshot, RuntimeReadiness as CoreRuntimeReadiness, StartupError,
    StartupStage,
};
use d2b_core_controller::migration::LegacyTpmMigrationDecision;
use d2b_core_controller::zone_status::{
    SystemCoreStatusEmitter, ZoneRuntimeMetadata, ZoneStatusInput,
};
use d2b_provider_clipboard_wayland::Policy as ClipboardPolicy;
use d2b_provider_display_wayland::WaylandSessionSpec;
use d2b_provider_notification_desktop::{Category, GuestSourceConfig, NotificationProviderConfig};
use d2b_provider_system_core::{
    HostCapabilityClass, HostObservationReport, HostProbeEffectPort, HostProbeMetadata,
    HostReconciler, MinijailPlatformGate, UserBinding, UserDiscoveryEffectPort, UserIdentityDigest,
    UserObservation, UserReconciler,
};
use d2b_resource_api::{
    RedbBackend, ResourceApiClient, ResourceBusAdapter, ResourceService, ResourceStoreBackend,
    authz::{
        ApiCatalog, AuthorizationState, BindingScope, BootstrapPhase, BoundSubject, CompiledRole,
        CompiledRoleBinding, NativeAuthorizer, PolicyRule, PolicySet, RelayGrantAuthority,
        ResourceVerb, SessionVerb,
    },
    service::UnavailableUpgradeDispatcher,
};
use d2b_resource_store::StoreFilter;
#[cfg(test)]
use d2b_resource_store::StoreListResult;
use d2b_resource_store::{
    PolicySnapshot, StoreGetRequest, StoreListRequest, StoreOperationContext, StoreProjection,
    StoreSlot, StoredResource,
};
use d2b_resource_store_redb::{
    BrokerEvidenceIndex, LogicalBackup, RedbResourceStore, StoreIdentity, StoreRuntimeMetadata,
    write_provisioning_marker,
};
use d2b_session::{
    HandshakeCredentials, SessionEngine, SessionServerError, TransportEvidence,
    serve_ttrpc_services,
};
use d2b_session_unix::{
    CreditPool, CreditScopeSet, DescriptorPolicyResolver, PeerIdentityPolicy, SeqpacketSocket,
    UnixSeqpacketTransport, UnixSessionError, VerifiedUnixPeer, prearmed_seqpacket_pair,
};
use nix::unistd::{Group, Uid, User};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// Maximum number of Zone runtimes owned by one daemon.
pub const MAX_ZONE_RUNTIMES: usize = 64;

const OPERATOR_SUBJECT_REF: &str = "User/d2bd-operator";
const OPERATOR_SUBJECT_UID: &str = "22222222-2222-4222-8222-222222222222";

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
    /// The Resource API returned a malformed canonical response.
    ResponseInvalid,
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
    /// Committed interaction Provider configuration is absent or invalid.
    InteractionConfigurationUnavailable,
    /// A shutdown was refused because request owners are still live.
    LiveRequestOwners,
    /// No authenticated subject was bound to the request.
    IdentityUnbound,
    /// The requested operation is not exposed by the registered service.
    CapabilityUnavailable,
    /// The public Resource API returned a typed error while loading a
    /// resource for provider reconciliation.
    ResourceGetFailed(ResourceErrorKind),
    /// The Wave 6 operator acceptance boundary did not converge.
    Wave6AcceptanceFailed,
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
            Self::ResponseInvalid => "resource-runtime-response-invalid",
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
            Self::InteractionConfigurationUnavailable => {
                "resource-runtime-interaction-configuration-unavailable"
            }
            Self::LiveRequestOwners => "resource-runtime-live-request-owners",
            Self::IdentityUnbound => "resource-runtime-identity-unbound",
            Self::CapabilityUnavailable => "resource-runtime-capability-unavailable",
            Self::ResourceGetFailed(_) => "resource-runtime-resource-get-failed",
            Self::Wave6AcceptanceFailed => "resource-runtime-wave6-acceptance-failed",
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

#[derive(Clone)]
pub(crate) struct CommittedInteractionProviderConfiguration {
    clipboard: Option<CommittedClipboardProviderConfiguration>,
    notification: Option<CommittedNotificationProviderConfiguration>,
}

#[derive(Clone)]
pub(crate) struct CommittedInteractionIdentity {
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

    pub(crate) const fn clipboard_provider_generation(&self) -> Option<ResourceGeneration> {
        self.clipboard_provider_generation
    }

    pub(crate) fn clipboard_provider_uid(&self) -> Option<&ResourceUid> {
        self.clipboard_provider_uid.as_ref()
    }

    pub(crate) const fn notification_provider_generation(&self) -> Option<ResourceGeneration> {
        self.notification_provider_generation
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
    authorizer: Arc<NativeAuthorizer>,
    authorization_state: Option<AuthorizationState>,
    #[allow(dead_code)]
    bus: Option<ZoneBus>,
    registrar: Mutex<Option<ZoneRegistrar>>,
    ingress: Mutex<Option<BusIngress>>,
    service_task: Mutex<Option<tokio::task::JoinHandle<Result<(), SessionServerError>>>>,
    process_status_client:
        Option<Arc<ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>>>,
    core: Mutex<CoreProcess>,
    readiness: ZoneRuntimeReadiness,
    policy_installed: bool,
    controller_endpoint_registered: bool,
    watch_admitted: bool,
    authority_index: Arc<tokio::sync::Mutex<HostGlobalAuthorityIndex>>,
    authority_persistence: Arc<RedbAuthorityPersistence>,
    authority_recovery: Arc<AuthorityRecoveryCoordinator>,
    device_tpm_controller: crate::tpm_effect_port::DeviceTpmControllerRegistration,
    zone_status: Mutex<ZoneStatusResource>,
    audio_runtime: Arc<Mutex<Option<AudioResourceRuntime>>>,
    audio_watch_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    process_runtime: Arc<Mutex<Option<ProcessResourceRuntime>>>,
    process_watch_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    activation_runtime: Arc<Mutex<Option<ActivationResourceRuntime>>>,
    activation_watch_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    interaction_provider_configuration: Option<CommittedInteractionProviderConfiguration>,
    interaction_identity: Option<CommittedInteractionIdentity>,
    interaction_provider_configuration_refused: bool,
}

/// Store-derived admission evidence for one security-key Device effect.
///
/// This contains only the exact values validated against the authoritative
/// resource record. It is consumed by the Device effect adapter before it can
/// request a broker-opened descriptor.
pub(crate) struct SecurityKeyDeviceAdmission {
    pub(crate) zone_ref: ResourceRef,
    pub(crate) device_uid: ResourceUid,
    pub(crate) holder_ref: ResourceRef,
    pub(crate) selector_id: String,
}

/// Request fields that select the Device admission record to validate.
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
            false,
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
        )
        .await
    }

    /// Open a newly provisioned production Zone with the immutable bootstrap
    /// policy and system-core Host authority required for first readiness.
    ///
    /// Existing stores are never promoted by this path; their durable policy
    /// snapshot remains the authority for restart and migration decisions.
    pub(crate) async fn open_production_with_audit_and_evidence_and_telemetry(
        zone: ZoneId,
        opened: OpenedZoneStore,
        audit_sink: Arc<AuditSink>,
        broker_evidence: Arc<BrokerEvidenceIndex>,
        telemetry_path: impl Into<std::path::PathBuf>,
        desired_bundle: ResourceBundle,
    ) -> Result<Self, ResourceRuntimeError> {
        Self::open_internal(
            zone,
            opened,
            Some(audit_sink),
            broker_evidence,
            Some(telemetry_path.into()),
            true,
            Some(desired_bundle),
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
            bootstrap_provisioned_store,
            desired_bundle,
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
        let store_identity = store_identity(&zone, &opened.response.store_identity)?;
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
        let authorizer = Arc::new(runtime_authorizer(&bundle_resource_types)?);
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
        let store_metadata = store
            .runtime_metadata()
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
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
        let backend = Arc::new(RedbBackend::from_arc(Arc::clone(&store)));
        let api = Arc::new(
            ResourceService::new(Arc::clone(&backend), Arc::clone(&authorizer))
                .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?,
        );
        let mut interaction_provider_configuration = None;
        let mut interaction_provider_configuration_refused = false;
        let mut interaction_identity = None;

        let mut core = CoreProcess::new();
        let mut bus = None;
        let mut registrar = None;
        let mut ingress = None;
        let mut service_task = None;
        let mut process_status_client = None;
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
            let _ = core.connect_runtime(CoreRuntimeReadiness {
                store_ready: true,
                resource_api_ready: false,
                local_bus_ready: false,
                controller_endpoint_registered: false,
                authenticated_system_core_session: false,
            });
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
            let (zone_ingress, zone_service_task, status_client) = register_system_core_session(
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
            if let Some(bundle) = desired_bundle.as_ref() {
                materialize_zone_resource_bundle(&zone, bundle, &store, &status_client)
                    .await
                    .inspect_err(|error| {
                        tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime Zone bundle materialization failed");
                    })?;
            } else if bootstrap_provisioned_store
                && disposition == ZoneStoreDisposition::Provisioned
            {
                ensure_bootstrap_host_resource(&zone, &store, &status_client).await?;
            }
            let store_metadata = store
                .runtime_metadata()
                .await
                .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
            (
                interaction_provider_configuration,
                interaction_provider_configuration_refused,
            ) = match load_interaction_provider_configuration(
                &zone,
                &store,
                store_metadata.current_revision,
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
                store_metadata.current_revision,
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
            let system_core =
                reconcile_system_core_resources(&zone, &store, Arc::clone(&status_client))
                    .await
                    .inspect_err(|error| {
                        tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime system-core reconciliation failed");
                    })?;
            tracing::debug!(
                zone = %zone.as_str(),
                host_phase = ?system_core.host_phase,
                user_phase = ?system_core.user_phase,
                total_resources = system_core.total_resource_count,
                "system-core bootstrap reconciliation completed",
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
                .map_err(|error| {
                    let error = map_startup_error(error);
                    tracing::error!(zone = %zone.as_str(), error = ?error, "resource runtime core startup failed");
                    error
                })?;
                mark_core_handlers(
                    &mut core,
                    aggregate_handler_phase,
                    store_metadata.current_revision.get(),
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
                                &store_metadata,
                                system_core.total_resource_count,
                                system_core.generation_cleanup_pending,
                                system_core.cleanup_pending_count,
                                Some(current_status_timestamp()),
                            )),
                    )
                    .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
                Some(state),
            )
        };
        let store_metadata = store
            .runtime_metadata()
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        Ok(Self {
            zone,
            store_id: expected_store_id,
            store,
            store_metadata,
            backend,
            api,
            authorizer,
            authorization_state,
            bus,
            registrar: Mutex::new(registrar),
            ingress: Mutex::new(ingress),
            service_task: Mutex::new(service_task),
            process_status_client,
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
            authority_index,
            authority_persistence,
            authority_recovery,
            device_tpm_controller: crate::tpm_effect_port::register_device_tpm_controller(),
            zone_status: Mutex::new(zone_status),
            audio_runtime: Arc::new(Mutex::new(None)),
            audio_watch_task: Mutex::new(None),
            process_runtime: Arc::new(Mutex::new(None)),
            process_watch_task: Mutex::new(None),
            activation_runtime: Arc::new(Mutex::new(None)),
            activation_watch_task: Mutex::new(None),
            interaction_provider_configuration,
            interaction_identity,
            interaction_provider_configuration_refused,
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

    /// Return the policy revision committed in the opened resource store.
    ///
    /// Interaction Providers bind this snapshot instead of carrying a
    /// route-derived policy placeholder.
    pub const fn committed_policy_snapshot(&self) -> PolicySnapshot {
        self.store_metadata.policy_snapshot
    }

    /// Return the durable resource revision used to fence interaction
    /// evidence against a later store commit.
    pub const fn current_revision(&self) -> ZoneRevision {
        self.store_metadata.current_revision
    }

    /// Bind a Resource API client to an authenticated local operator subject.
    ///
    /// The caller supplies claims produced by the authenticated
    /// ComponentSession boundary. This performs the final live-policy
    /// admission and returns the same checked Resource API client used by the
    /// registered system-core session; it never turns a Unix peer or request
    /// payload into a subject.
    pub fn bind_operator_resource_client(
        &self,
        context: d2b_contracts::v3::AuthenticatedSubjectContext,
    ) -> Result<
        Arc<ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>>,
        ResourceRuntimeError,
    > {
        let state = self
            .authorization_state
            .clone()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        let subject = self
            .authorizer
            .issue_authenticated_subject(context, state)
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
        let adapter = ResourceBusAdapter::bind_component_session(Arc::clone(&self.api), subject)
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?;
        Ok(Arc::new(adapter.client()))
    }

    /// Persist a provider reconcile phase through the authenticated Resource
    /// API so restart admission can rely on durable observed generation.
    pub(crate) async fn persist_public_reconcile_phase(
        &self,
        peer_uid: u32,
        resource_ref: &ResourceRef,
        resource_uid: &ResourceUid,
        operation_id: &str,
        phase: &str,
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
        if current_phase == Some(phase) {
            return Ok(());
        }
        let status = json!({ "phase": phase });
        let context = local_operator_subject_context(&self.zone, peer_uid, operation_id)?;
        let client = self.bind_operator_resource_client(context)?;
        persist_resource_status(&client, &resource, &status).await
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
        client: &ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>,
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

    /// Return the stable operator subject identity admitted by the local
    /// Resource API policy.
    pub fn operator_subject_identity() -> (ResourceRef, ResourceUid) {
        (
            ResourceRef::parse(OPERATOR_SUBJECT_REF).expect("stable operator subject ref"),
            ResourceUid::parse(OPERATOR_SUBJECT_UID).expect("stable operator subject uid"),
        )
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

    pub(crate) const fn interaction_provider_configuration_refused(&self) -> bool {
        self.interaction_provider_configuration_refused
    }

    /// Return the current core-controller stage.
    pub fn core_stage(&self) -> Result<StartupStage, ResourceRuntimeError> {
        self.core
            .lock()
            .map(|core| core.stage())
            .map_err(|_| ResourceRuntimeError::CoreStartupFailed)
    }

    /// Whether the production Device TPM reconcile entry point is registered.
    pub(crate) const fn device_tpm_controller_registered(&self) -> bool {
        self.device_tpm_controller.is_registered()
    }

    /// Borrow the registered Device TPM reconcile entry point.
    pub(crate) const fn device_tpm_controller(
        &self,
    ) -> crate::tpm_effect_port::DeviceTpmControllerRegistration {
        self.device_tpm_controller
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

    /// Relist and reconcile the durable PipeWire resources owned by this
    /// Zone. The registry is initialized once and survives ordinary
    /// watch/reconcile cycles; a daemon restart reconstructs it from store
    /// rows before any public readiness is published.
    pub(crate) async fn reconcile_audio_resources(
        &self,
        state: Arc<crate::ServerState>,
    ) -> Result<(), ResourceRuntimeError> {
        if !self.readiness.resource_api_ready {
            return Ok(());
        }
        let snapshot = list_audio_snapshot(&self.store, &self.zone)
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        let binding_resources = snapshot.bindings.clone();
        let statuses;
        let pending_finalizers;
        {
            let mut runtime = self
                .audio_runtime
                .lock()
                .map_err(|_| ResourceRuntimeError::CapabilityUnavailable)?;
            let registry =
                runtime.get_or_insert_with(|| AudioResourceRuntime::new(self.zone.clone(), state));
            registry
                .reconcile(snapshot)
                .map_err(map_audio_runtime_error)?;
            statuses = registry.statuses();
            pending_finalizers = registry.take_pending_finalizers();
        }
        for resource in pending_finalizers {
            remove_audio_finalizer(
                self.process_status_client
                    .as_ref()
                    .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?,
                &resource,
            )
            .await
            .map_err(map_audio_runtime_error)?;
        }
        for status in statuses {
            let Some(resource) = binding_resources
                .iter()
                .find(|resource| resource.resource_ref == status.resource)
            else {
                return Err(ResourceRuntimeError::StoreReadFailed);
            };
            persist_resource_status(
                self.process_status_client
                    .as_ref()
                    .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?,
                resource,
                &audio_binding_status_value(status.status),
            )
            .await?;
        }
        let start_watch = {
            let mut watch_task = self
                .audio_watch_task
                .lock()
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            watch_needs_restart(&mut watch_task)
        };
        if start_watch {
            let watch = d2b_resource_api::watch::WatchService::new(Arc::clone(&self.store))
                .open(audio_watch_request(&self.zone))
                .await
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            let store = Arc::clone(&self.store);
            let zone = self.zone.clone();
            let registry = Arc::clone(&self.audio_runtime);
            let status_client = self
                .process_status_client
                .as_ref()
                .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?
                .clone();
            let task = tokio::spawn(run_audio_watch(watch, store, zone, registry, status_client));
            let mut watch_task = self
                .audio_watch_task
                .lock()
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            if watch_task.is_none() {
                *watch_task = Some(task);
            } else {
                task.abort();
            }
        }
        Ok(())
    }

    /// Relist and reconcile generic Process and EphemeralProcess resources
    /// owned by this Zone. Their lifecycle effects remain inside the fixed
    /// daemon-composed process Provider supervisors.
    pub(crate) async fn reconcile_process_resources(
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
        let snapshot = list_process_snapshot(&self.store, &self.zone)
            .await
            .map_err(map_process_runtime_error)?;
        let runtime = match self.process_runtime.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => return Err(ResourceRuntimeError::CapabilityUnavailable),
        };
        let mut runtime =
            runtime.unwrap_or_else(|| ProcessResourceRuntime::new(self.zone.clone(), providers));
        if let Some(controller_generation) =
            self.store_metadata.policy_snapshot.controller_generation
        {
            runtime.set_controller_generation(controller_generation);
        }
        if let Some(status_client) = &self.process_status_client {
            runtime.set_status_client(Arc::clone(status_client));
        }
        let result = runtime.reconcile(snapshot).await;
        match self.process_runtime.lock() {
            Ok(mut guard) => *guard = Some(runtime),
            Err(_) => return Err(ResourceRuntimeError::CapabilityUnavailable),
        }
        result.map_err(map_process_runtime_error)?;

        let start_watch = {
            let mut watch_task = self
                .process_watch_task
                .lock()
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            watch_needs_restart(&mut watch_task)
        };
        if start_watch {
            let watch = d2b_resource_api::watch::WatchService::new(Arc::clone(&self.store))
                .open(process_watch_request(&self.zone))
                .await
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            let store = Arc::clone(&self.store);
            let zone = self.zone.clone();
            let registry = Arc::clone(&self.process_runtime);
            let task = tokio::spawn(run_process_watch(watch, store, zone, registry));
            let mut watch_task = self
                .process_watch_task
                .lock()
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            if watch_task.is_none() {
                *watch_task = Some(task);
            } else {
                task.abort();
            }
        }
        Ok(())
    }

    /// Relist and reconcile durable NixOS generation resources owned by this
    /// Zone. Activation effects remain behind the typed broker or the
    /// authenticated guest-control bridge.
    pub(crate) async fn reconcile_activation_resources(
        &self,
        state: Arc<crate::ServerState>,
    ) -> Result<(), ResourceRuntimeError> {
        if !self.readiness.resource_api_ready {
            return Ok(());
        }
        let snapshot = list_activation_snapshot(&self.store, &self.zone)
            .await
            .map_err(map_activation_runtime_error)?;
        let runtime = match self.activation_runtime.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => return Err(ResourceRuntimeError::CapabilityUnavailable),
        };
        let mut runtime =
            runtime.unwrap_or_else(|| ActivationResourceRuntime::new(self.zone.clone()));
        if let Some(status_client) = &self.process_status_client {
            runtime.set_status_client(Arc::clone(status_client));
        }
        let result = runtime.reconcile(Arc::clone(&state), snapshot).await;
        match self.activation_runtime.lock() {
            Ok(mut guard) => *guard = Some(runtime),
            Err(_) => return Err(ResourceRuntimeError::CapabilityUnavailable),
        }
        result.map_err(map_activation_runtime_error)?;
        let start_watch = {
            let mut watch_task = self
                .activation_watch_task
                .lock()
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            watch_needs_restart(&mut watch_task)
        };
        if start_watch {
            let watch = d2b_resource_api::watch::WatchService::new(Arc::clone(&self.store))
                .open(activation_watch_request(&self.zone))
                .await
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            let store = Arc::clone(&self.store);
            let zone = self.zone.clone();
            let state = state.clone();
            let registry = Arc::clone(&self.activation_runtime);
            let task = tokio::spawn(run_activation_watch(watch, store, zone, state, registry));
            let mut watch_task = self
                .activation_watch_task
                .lock()
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            if watch_task.is_none() {
                *watch_task = Some(task);
            } else {
                task.abort();
            }
        }
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
    pub async fn dispatch_cli_request(
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
    pub async fn dispatch_public_cli_request(
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
        let operation_id = public_operation_id(request, peer_uid, method);
        let context = local_operator_subject_context(&self.zone, peer_uid, &operation_id)?;
        let client = self.bind_operator_resource_client(context)?;
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
            _ => Err(ResourceRuntimeError::CapabilityUnavailable),
        }
    }

    /// Verify the trusted persisted Device row used by the TPM reconcile
    /// adapter and return Core's sealed legacy-state decision. The VM binding
    /// is read from the authenticated Device record, while the legacy-state
    /// decision comes from the trusted Core bundle resolver; request fields
    /// cannot select either independently.
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

    /// Publish terminal broker evidence into the live store join index.
    pub fn ingest_broker_evidence(
        &self,
        evidence: DurabilityEvidence,
    ) -> Result<(), ResourceRuntimeError> {
        self.store
            .ingest_broker_evidence(evidence)
            .map_err(|_| ResourceRuntimeError::StoreOpenFailed)
    }

    fn tpm_device_targets_vm(resource: &Value, vm_id: &str) -> bool {
        resource
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("ownerRef"))
            .and_then(Value::as_str)
            .and_then(|owner| owner.strip_prefix("Guest/"))
            == Some(vm_id)
    }

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
            audio_watch_task,
            audio_runtime,
            process_watch_task,
            process_runtime,
            activation_watch_task,
            activation_runtime,
            ..
        } = self;
        if let Some(task) = service_task
            .into_inner()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
        {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = audio_watch_task
            .into_inner()
            .map_err(|_| ResourceRuntimeError::WatchUnavailable)?
        {
            task.abort();
            let _ = task.await;
        }
        drop(audio_runtime);
        if let Some(task) = process_watch_task
            .into_inner()
            .map_err(|_| ResourceRuntimeError::WatchUnavailable)?
        {
            task.abort();
            let _ = task.await;
        }
        drop(process_runtime);
        if let Some(task) = activation_watch_task
            .into_inner()
            .map_err(|_| ResourceRuntimeError::WatchUnavailable)?
        {
            task.abort();
            let _ = task.await;
        }
        drop(activation_runtime);
        drop(process_status_client);
        drop(
            ingress
                .into_inner()
                .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
        );
        drop(
            registrar
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedListRequest {
    resource_types: Vec<ResourceTypeName>,
    resource_names: Vec<ResourceName>,
    filters: Vec<StoreFilter>,
    page_size: u32,
    cursor: Option<String>,
    projection: StoreProjection,
}

fn parse_list_request(request: &Value) -> Result<ParsedListRequest, ResourceRuntimeError> {
    const LIST_FIELDS: &[&str] = &[
        "type",
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

fn runtime_policy(
    zone: &ZoneId,
    snapshot: &PolicySnapshot,
    current_revision: ZoneRevision,
    bundle_resource_types: &[ResourceTypeName],
) -> Result<(PolicySet, AuthorizationState), ResourceRuntimeError> {
    if snapshot.policy_revision == 0
        || snapshot.api_catalog_revision == 0
        || snapshot.active_configuration_revision.get() == 0
    {
        return Err(ResourceRuntimeError::PolicyUnavailable);
    }
    let catalog = ApiCatalog::with_extensions(
        bundle_resource_types
            .iter()
            .filter(|resource_type| resource_type.as_str().contains(".d2bus.org."))
            .cloned(),
    )
    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let mut resource_types = STANDARD_RESOURCE_TYPES
        .iter()
        .map(|name| ResourceTypeName::parse(*name))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    for resource_type in bundle_resource_types {
        if !resource_types.contains(resource_type) {
            resource_types.push(resource_type.clone());
        }
    }
    let resource_verbs = [
        ResourceVerb::Get,
        ResourceVerb::List,
        ResourceVerb::Watch,
        ResourceVerb::Create,
        ResourceVerb::UpdateSpec,
        ResourceVerb::UpdateStatus,
        ResourceVerb::UpdateMetadata,
        ResourceVerb::UpdateFinalizers,
        ResourceVerb::Delete,
    ];
    let session_verbs = [
        SessionVerb::Connect,
        SessionVerb::Invoke,
        SessionVerb::OpenStream,
        SessionVerb::Cancel,
    ];
    let mut rules = Vec::new();
    for chunk in resource_types.chunks(16) {
        rules.push(
            PolicyRule::new(
                &catalog,
                chunk.iter().cloned(),
                resource_verbs,
                session_verbs,
                [],
                [],
                [zone.clone()],
                [],
            )
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
        );
    }
    let role_ref = ResourceRef::parse("Role/system-core-runtime")
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let role = CompiledRole::new(role_ref.clone(), rules)
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let binding_scope = BindingScope {
        zones: [zone.clone()].into_iter().collect(),
        ..BindingScope::default()
    };
    let binding = CompiledRoleBinding::new(
        role_ref.clone(),
        [
            BoundSubject {
                subject_ref: ResourceRef::parse("Provider/system-core")
                    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
                subject_uid: ResourceUid::parse("11111111-1111-4111-8111-111111111111")
                    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
            },
            BoundSubject {
                subject_ref: ResourceRef::parse(OPERATOR_SUBJECT_REF)
                    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
                subject_uid: ResourceUid::parse(OPERATOR_SUBJECT_UID)
                    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
            },
        ],
        binding_scope,
        RelayGrantAuthority::None,
    )
    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let policy = PolicySet::new(
        &catalog,
        snapshot.policy_revision,
        vec![role],
        vec![binding],
    )
    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let state = AuthorizationState {
        snapshot: *snapshot,
        zone_policy_revision: current_revision,
        bootstrap_phase: BootstrapPhase::Disabled,
        now_tick: 1,
    };
    Ok((policy, state))
}

fn system_core_endpoint_policy() -> EndpointPolicy {
    EndpointPolicy {
        purpose: EndpointPurpose::ResourceService,
        purpose_class: PurposeClass::Local,
        initiator_role: EndpointRole::ZoneController,
        responder_role: EndpointRole::Component,
        service: ServicePackage::ResourceV3,
        schema_fingerprint: [0x11; 32],
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: ComponentTransportBinding {
            transport: TransportClass::InheritedSocketpair,
            locality: Locality::HostLocal,
            channel_binding: [0x22; 32],
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: 1,
        attachment_policy: AttachmentPolicy {
            kind: d2b_contracts::v3::component_session::AttachmentPolicyKind::PacketAtomic,
            max_per_packet: 1,
            max_per_request: 1,
            max_per_operation: 1,
            max_per_session: 1,
            credentials_allowed: false,
        },
    }
}

fn unix_transport(
    socket: SeqpacketSocket,
    policy: &EndpointPolicy,
) -> Result<UnixSeqpacketTransport, ResourceRuntimeError> {
    let expected_peer = socket
        .acceptor_peer_credentials()
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let credits = CreditScopeSet::new(
        CreditPool::new(64).map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
        CreditPool::new(64).map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
        CreditPool::new(64).map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
        CreditPool::new(64).map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
        CreditPool::new(64).map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
        CreditPool::new(64).map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
    );
    let resolver: DescriptorPolicyResolver =
        std::sync::Arc::new(|_| Err(UnixSessionError::DescriptorMismatch));
    UnixSeqpacketTransport::new(
        socket,
        policy.transport_binding.locality,
        policy.limits,
        policy.attachment_policy,
        credits,
        resolver,
        PeerIdentityPolicy::inherited_socketpair(expected_peer),
    )
    .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)
}

async fn register_system_core_session(
    registrar: &mut ZoneRegistrar,
    api: Arc<ResourceService<RedbBackend>>,
    authorizer: Arc<NativeAuthorizer>,
    authz_state: AuthorizationState,
) -> Result<
    (
        BusIngress,
        tokio::task::JoinHandle<Result<(), SessionServerError>>,
        Arc<ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>>,
    ),
    ResourceRuntimeError,
> {
    let policy = system_core_endpoint_policy();
    let (initiator_fd, responder_fd) =
        prearmed_seqpacket_pair().map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let initiator_socket = SeqpacketSocket::from_parent_prearmed(initiator_fd)
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let responder_socket = SeqpacketSocket::from_parent_prearmed(responder_fd)
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let verified_peer = VerifiedUnixPeer::verify_inherited_seqpacket(&initiator_socket)
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let initiator = unix_transport(initiator_socket, &policy)?;
    let responder = unix_transport(responder_socket, &policy)?;
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
    let initiator = initiator.map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let responder = responder.map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let acceptor = registrar
        .component_session_acceptor(policy.clone(), verified_peer)
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let candidate = acceptor
        .admit(
            initiator,
            TransportEvidence::new(
                d2b_contracts::v3::EvidenceClass::UnixPeer,
                BindingDigest::parse(format!("sha256:{}", "22".repeat(32)))
                    .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
            ),
            1,
        )
        .await
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let controller_generation = authz_state
        .snapshot
        .controller_generation
        .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
    let subject = authorizer
        .issue_authenticated_subject(
            candidate
                .route_binding()
                .context()
                .clone()
                .with_controller_generation(controller_generation),
            authz_state,
        )
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let service = Arc::new(
        ResourceBusAdapter::bind_component_session(api, subject)
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?,
    );
    let status_client = Arc::new(service.client());
    let services = Arc::clone(&service).ttrpc_services();
    let ingress = registrar
        .register_component_session(candidate)
        .await
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let service_task = tokio::spawn(serve_ttrpc_services(
        Arc::new(responder.into_driver()),
        services,
    ));
    Ok((ingress, service_task, status_client))
}

#[derive(Debug, Clone, Copy)]
struct SystemCoreReconcileResult {
    core_phase: ResourcePhase,
    host_phase: HandlerPhase,
    user_phase: HandlerPhase,
    total_resource_count: u32,
    generation_cleanup_pending: bool,
    cleanup_pending_count: u32,
}

fn watch_needs_restart(slot: &mut Option<tokio::task::JoinHandle<()>>) -> bool {
    if slot.as_ref().is_some_and(|task| task.is_finished()) {
        *slot = None;
    }
    slot.is_none()
}

fn zone_runtime_metadata(
    store_metadata: &StoreRuntimeMetadata,
    total_resource_count: u32,
    generation_cleanup_pending: bool,
    cleanup_pending_count: u32,
    last_reconciled_at: Option<Timestamp>,
) -> ZoneRuntimeMetadata {
    ZoneRuntimeMetadata {
        api_catalog_revision: store_metadata.policy_snapshot.api_catalog_revision,
        policy_revision: store_metadata.policy_snapshot.policy_revision,
        configuration_revision: store_metadata
            .policy_snapshot
            .active_configuration_revision
            .get(),
        installed_provider_count: 0,
        ready_provider_count: 0,
        total_resource_count,
        active_configuration_generation: store_metadata
            .policy_snapshot
            .active_configuration_revision
            .get(),
        generation_cleanup_pending,
        cleanup_pending_count,
        last_reconciled_at,
    }
}

fn current_status_timestamp() -> Timestamp {
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
    Timestamp::parse(format!(
        "{year:04}-{month:02}-{day_of_month:02}T{hour:02}:{minute:02}:{second:02}.{:03}Z",
        millis % 1_000
    ))
    .expect("system timestamp formatter emits canonical UTC")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month = (5 * doy + 2) / 153;
    let day = doy - (153 * month + 2) / 5 + 1;
    let month = month + if month < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

fn handler_phase_to_zone_phase(phase: HandlerPhase) -> d2b_contracts::v3::ZoneHandlerPhase {
    match phase {
        HandlerPhase::Ready => d2b_contracts::v3::ZoneHandlerPhase::Ready,
        HandlerPhase::Degraded => d2b_contracts::v3::ZoneHandlerPhase::Degraded,
        HandlerPhase::Failed => d2b_contracts::v3::ZoneHandlerPhase::Failed,
        HandlerPhase::Unknown => d2b_contracts::v3::ZoneHandlerPhase::Unknown,
        HandlerPhase::Pending | HandlerPhase::Recovering => {
            d2b_contracts::v3::ZoneHandlerPhase::Pending
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct SystemCoreUserDiscovery;

#[derive(Debug, Clone, Copy)]
struct SystemCoreHostProbe {
    user_uid: u32,
}

impl SystemCoreHostProbe {
    fn current() -> Self {
        Self {
            user_uid: Uid::current().as_raw(),
        }
    }

    fn kernel_release() -> Result<String, d2b_provider_system_core::SystemCoreError> {
        read_bounded("/proc/sys/kernel/osrelease", 64)
            .map(|release| release.trim().to_owned())
            .map_err(|_| d2b_provider_system_core::SystemCoreError::HostProbeFailed)
    }

    fn os_name() -> Result<String, d2b_provider_system_core::SystemCoreError> {
        let release = read_bounded("/etc/os-release", 16 * 1024)
            .map_err(|_| d2b_provider_system_core::SystemCoreError::HostProbeFailed)?;
        Ok(release
            .lines()
            .find_map(|line| line.strip_prefix("NAME="))
            .map(|name| name.trim_matches('"').to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "unknown".to_owned()))
    }

    fn runtime_path(&self, name: &str) -> std::path::PathBuf {
        Path::new("/run/user")
            .join(self.user_uid.to_string())
            .join(name)
    }

    fn has_render_node() -> bool {
        fs::read_dir("/dev/dri")
            .map(|entries| {
                entries.flatten().any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("renderD"))
                })
            })
            .unwrap_or(false)
    }

    fn has_primary_drm_node() -> bool {
        fs::read_dir("/dev/dri")
            .map(|entries| {
                entries.flatten().any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("card"))
                })
            })
            .unwrap_or(false)
    }

    fn active_process_count() -> Result<u32, d2b_provider_system_core::SystemCoreError> {
        let mut count = 0_u32;
        for entry in fs::read_dir("/proc")
            .map_err(|_| d2b_provider_system_core::SystemCoreError::HostProbeFailed)?
        {
            let entry =
                entry.map_err(|_| d2b_provider_system_core::SystemCoreError::HostProbeFailed)?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.bytes().all(|byte| byte.is_ascii_digit()))
            {
                count = count.saturating_add(1);
            }
        }
        Ok(count)
    }
}

impl HostProbeEffectPort for SystemCoreHostProbe {
    async fn probe(
        &self,
        capability: HostCapabilityClass,
    ) -> Result<bool, d2b_provider_system_core::SystemCoreError> {
        let available = match capability {
            HostCapabilityClass::Kvm => Path::new("/dev/kvm").is_file(),
            HostCapabilityClass::Pidfd => {
                let gate = crate::process_provider_runtime::detect_minijail_platform_gate();
                gate.kernel_major > 5 || (gate.kernel_major == 5 && gate.kernel_minor >= 3)
            }
            HostCapabilityClass::CgroupV2 => {
                Path::new("/sys/fs/cgroup/cgroup.controllers").is_file()
            }
            HostCapabilityClass::UserNamespace => Path::new("/proc/self/ns/user").exists(),
            HostCapabilityClass::Virtiofs => Path::new("/dev/fuse").is_file(),
            HostCapabilityClass::AudioPipewire => is_socket(&self.runtime_path("pipewire-0")),
            HostCapabilityClass::Wayland => is_socket(&self.runtime_path("wayland-0")),
            HostCapabilityClass::GpuRender => Self::has_render_node(),
            HostCapabilityClass::GpuDrm => Self::has_primary_drm_node(),
            HostCapabilityClass::Tpm2 => {
                Path::new("/dev/tpmrm0").is_file() || Path::new("/dev/tpm0").is_file()
            }
            HostCapabilityClass::Usbip => {
                Path::new("/sys/module/usbip_core").exists()
                    || Path::new("/sys/module/usbip_host").exists()
            }
        };
        Ok(available)
    }

    async fn platform(
        &self,
    ) -> Result<MinijailPlatformGate, d2b_provider_system_core::SystemCoreError> {
        let gate = crate::process_provider_runtime::detect_minijail_platform_gate();
        Ok(MinijailPlatformGate::new(
            gate.kernel_major,
            gate.kernel_minor,
            gate.cgroup_kill_writable,
        ))
    }

    async fn metadata(
        &self,
    ) -> Result<HostProbeMetadata, d2b_provider_system_core::SystemCoreError> {
        Ok(HostProbeMetadata {
            kernel_release: Self::kernel_release()?,
            os_name: Self::os_name()?,
            user_manager_available: self.runtime_path("systemd").is_dir(),
            active_process_count: Self::active_process_count()?,
        })
    }
}

impl UserDiscoveryEffectPort for SystemCoreUserDiscovery {
    async fn discover(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<
        Option<d2b_provider_system_core::DiscoveredUser>,
        d2b_provider_system_core::SystemCoreError,
    > {
        discover_local_user(user_ref, spec).await
    }
}

async fn discover_local_user(
    user_ref: &ResourceRef,
    spec: &UserSpec,
) -> Result<
    Option<d2b_provider_system_core::DiscoveredUser>,
    d2b_provider_system_core::SystemCoreError,
> {
    let username = spec.os_username().as_str();
    let user = User::from_name(username)
        .map_err(|_| d2b_provider_system_core::SystemCoreError::DiscoveryUnavailable)?;
    let Some(user) = user else {
        return Ok(None);
    };

    let mut digest = Sha256::new();
    digest.update(b"d2b-system-core-user-v1");
    digest.update(user_ref.name().as_str().as_bytes());
    digest.update([0]);
    digest.update(username.as_bytes());
    digest.update([0]);
    digest.update(user.uid.as_raw().to_le_bytes());
    digest.update(user.gid.as_raw().to_le_bytes());

    let mut verified = std::collections::BTreeSet::from([UserBinding::NssRecord]);
    if Group::from_gid(user.gid)
        .map_err(|_| d2b_provider_system_core::SystemCoreError::DiscoveryUnavailable)?
        .is_some()
    {
        verified.insert(UserBinding::PrimaryGroup);
    }

    let mut groups_verified = true;
    for group in spec.groups() {
        let Some(group_record) = Group::from_name(group.as_str())
            .map_err(|_| d2b_provider_system_core::SystemCoreError::DiscoveryUnavailable)?
        else {
            groups_verified = false;
            continue;
        };
        digest.update([0]);
        digest.update(group.as_str().as_bytes());
        if !group_record.mem.iter().any(|member| member == username) {
            groups_verified = false;
        }
    }
    if groups_verified && !spec.groups().is_empty() {
        verified.insert(UserBinding::GroupMemberships);
    }

    Ok(Some(d2b_provider_system_core::DiscoveredUser {
        identity: UserIdentityDigest::from_bytes(digest.finalize().into()),
        observed: UserObservation::from_verified(verified),
    }))
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
    let page = store
        .list(StoreListRequest {
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
        })
        .await
        .map_err(|error| {
            tracing::error!(zone = %zone.as_str(), error = ?error, "bootstrap Host list failed");
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
    let page = store
        .list(StoreListRequest {
            operation,
            zone: zone.clone(),
            resource_types: vec![session_resource_type],
            resource_names: Vec::new(),
            filters: Vec::new(),
            page_size: 2,
            cursor: None,
            projection: StoreProjection::Full,
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

async fn committed_resource(
    zone: &ZoneId,
    store: &RedbResourceStore,
    current_revision: ZoneRevision,
    resource_ref: &ResourceRef,
) -> Result<StoredResource, ResourceRuntimeError> {
    if !matches!(
        resource_ref.resource_type().as_str(),
        "Guest" | "Host" | "Provider" | "User" | "display-wayland.d2bus.org.WaylandPolicy"
    ) {
        return Err(ResourceRuntimeError::InteractionConfigurationUnavailable);
    }
    let operation_id = format!(
        "interaction-identity:{}",
        resource_ref.to_canonical_string()
    );
    let resource = store
        .get(StoreGetRequest {
            operation: StoreOperationContext {
                operation_id: operation_id.clone(),
                idempotency_key: None,
                correlation_id: operation_id,
                trace_id: None,
                deadline_ms: 10_000,
            },
            zone: zone.clone(),
            target: resource_ref.clone(),
            expected_uid: None,
            projection: StoreProjection::Full,
        })
        .await
        .map_err(|_| ResourceRuntimeError::InteractionConfigurationUnavailable)?;
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
    if wire.host_execution_ref.resource_type().as_str() != "Host"
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
    if wire.host_execution_ref.resource_type().as_str() != "Host"
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

async fn reconcile_system_core_resources(
    zone: &ZoneId,
    store: &RedbResourceStore,
    status_client: Arc<ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>>,
) -> Result<SystemCoreReconcileResult, ResourceRuntimeError> {
    let host_type =
        ResourceTypeName::parse("Host").map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let user_type =
        ResourceTypeName::parse("User").map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let operation = |suffix: &str| StoreOperationContext {
        operation_id: format!("system-core-reconcile:{suffix}"),
        idempotency_key: None,
        correlation_id: format!("system-core-reconcile:{suffix}"),
        trace_id: None,
        deadline_ms: 10_000,
    };
    let mut resources = Vec::new();
    let mut cursor = None;
    loop {
        let page = store
            .list(StoreListRequest {
                operation: operation("all"),
                zone: zone.clone(),
                resource_types: Vec::new(),
                resource_names: Vec::new(),
                filters: Vec::new(),
                page_size: 128,
                cursor,
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        resources.extend(page.resources);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    let hosts = resources
        .iter()
        .filter(|resource| resource.resource_ref.resource_type() == &host_type)
        .cloned()
        .collect::<Vec<_>>();
    let users = resources
        .iter()
        .filter(|resource| resource.resource_ref.resource_type() == &user_type)
        .cloned()
        .collect::<Vec<_>>();
    let total_resource_count = resources.len().min(u32::MAX as usize) as u32;
    let active_configuration_generation = store
        .runtime_metadata()
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

    let mut host_phase = host_phase_for_resource_count(hosts.len());
    for resource in hosts {
        let envelope = d2b_contracts::v3::ResourceEnvelope::from_json(&resource.canonical_json)
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        let spec: HostSpec = serde_json::from_slice(&envelope.spec().base().to_canonical_bytes())
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        let host_ref = ResourceRef::new(
            envelope.resource_type().clone(),
            envelope.metadata().name().clone(),
        );
        let provider_ref = envelope
            .spec()
            .provider_ref()
            .ok_or(ResourceRuntimeError::HandlerNotReady)?;
        if provider_ref.to_canonical_string() != HOST_PROVIDER_REF {
            return Err(ResourceRuntimeError::HandlerNotReady);
        }
        let report = match HostReconciler::new()
            .reconcile_with_probe(
                &host_ref,
                provider_ref,
                &spec,
                &SystemCoreHostProbe::current(),
                &BTreeSet::new(),
                false,
            )
            .await
        {
            Ok(report) => report,
            Err(_) => {
                let mut status = HostReconciler::new()
                    .reconcile(&host_ref, provider_ref, &spec)
                    .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
                status.phase = ResourcePhase::Degraded;
                HostObservationReport {
                    status,
                    capabilities: Vec::new(),
                    kernel_release: "unknown".to_owned(),
                    os_name: "unknown".to_owned(),
                    user_manager_available: false,
                    active_process_count: 0,
                    minijail_ready: false,
                }
            }
        };
        if report.status.phase != ResourcePhase::Ready {
            host_phase = HandlerPhase::Degraded;
        }
        let status = host_status_value(&report)?;
        persist_resource_status(&status_client, &resource, &status).await?;
    }

    let user_reconciler = UserReconciler::new(SystemCoreUserDiscovery);
    let mut user_phase = HandlerPhase::Ready;
    for resource in users {
        let envelope = d2b_contracts::v3::ResourceEnvelope::from_json(&resource.canonical_json)
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        let spec: UserSpec = serde_json::from_slice(&envelope.spec().base().to_canonical_bytes())
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        let user_ref = ResourceRef::new(
            envelope.resource_type().clone(),
            envelope.metadata().name().clone(),
        );
        let status = user_reconciler
            .reconcile(&user_ref, &spec)
            .await
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        persist_resource_status(
            &status_client,
            &resource,
            &serde_json::to_value(&status).map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
        )
        .await?;
        if status.phase != ResourcePhase::Ready {
            // A User that is absent or drifted is a live, durable condition,
            // not a store failure. Keep the handler current but degraded.
            user_phase = HandlerPhase::Degraded;
        }
    }
    let core_phase =
        if matches!(host_phase, HandlerPhase::Ready) && matches!(user_phase, HandlerPhase::Ready) {
            ResourcePhase::Ready
        } else {
            ResourcePhase::Degraded
        };
    Ok(SystemCoreReconcileResult {
        core_phase,
        host_phase,
        user_phase,
        total_resource_count,
        generation_cleanup_pending: cleanup_pending_count != 0,
        cleanup_pending_count,
    })
}

fn configuration_cleanup_pending(
    resource: &StoredResource,
    active_configuration_generation: u64,
) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&resource.canonical_json) else {
        return false;
    };
    let Some(metadata) = value.get("metadata").and_then(serde_json::Value::as_object) else {
        return false;
    };
    metadata
        .get("managedBy")
        .and_then(serde_json::Value::as_str)
        == Some("configuration")
        && metadata
            .get("configurationGeneration")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|generation| generation < active_configuration_generation)
        && metadata
            .get("deletionRequestedAt")
            .is_some_and(|value| !value.is_null())
}

fn host_status_value(
    report: &HostObservationReport,
) -> Result<serde_json::Value, ResourceRuntimeError> {
    let mut status =
        serde_json::to_value(&report.status).map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let object = status
        .as_object_mut()
        .ok_or(ResourceRuntimeError::HandlerNotReady)?;
    object.insert(
        "capabilities".to_owned(),
        serde_json::to_value(&report.capabilities)
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
    );
    object.insert(
        "kernelRelease".to_owned(),
        serde_json::Value::String(report.kernel_release.clone()),
    );
    object.insert(
        "osName".to_owned(),
        serde_json::Value::String(report.os_name.clone()),
    );
    object.insert(
        "userManagerAvailable".to_owned(),
        serde_json::Value::Bool(report.user_manager_available),
    );
    object.insert(
        "activeProcessCount".to_owned(),
        serde_json::Value::Number(report.active_process_count.into()),
    );
    object.insert(
        "minijailReady".to_owned(),
        serde_json::Value::Bool(report.minijail_ready),
    );
    Ok(status)
}

pub(crate) async fn persist_resource_status(
    client: &ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>,
    resource: &StoredResource,
    status: &serde_json::Value,
) -> Result<(), ResourceRuntimeError> {
    let mut value = CanonicalJsonValue::parse(&resource.canonical_json)
        .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let root = match &mut value {
        CanonicalJsonValue::Object(root) => root,
        _ => return Err(ResourceRuntimeError::HandlerNotReady),
    };
    let status_bytes =
        serde_json::to_vec(status).map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let desired_status = CanonicalJsonValue::parse(&status_bytes)
        .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let CanonicalJsonValue::Object(resource_status) = desired_status else {
        return Err(ResourceRuntimeError::HandlerNotReady);
    };
    let phase = resource_status
        .get("phase")
        .cloned()
        .ok_or(ResourceRuntimeError::HandlerNotReady)?;
    let Some(CanonicalJsonValue::Object(status)) = root.get_mut("status") else {
        return Err(ResourceRuntimeError::HandlerNotReady);
    };
    let now = current_status_timestamp().as_str().to_owned();
    status.insert("phase".to_owned(), phase.clone());
    status.insert(
        "observedGeneration".to_owned(),
        CanonicalJsonValue::Integer(resource.generation.get() as i64),
    );
    status.insert(
        "lastReconciledAt".to_owned(),
        CanonicalJsonValue::String(now.clone()),
    );
    if matches!(
        phase,
        CanonicalJsonValue::String(ref phase) if phase == "Ready"
    ) && status
        .get("startedAt")
        .is_none_or(|value| matches!(value, CanonicalJsonValue::Null))
    {
        status.insert(
            "startedAt".to_owned(),
            CanonicalJsonValue::String(now.clone()),
        );
    }
    status.insert(
        "resource".to_owned(),
        CanonicalJsonValue::Object(resource_status),
    );
    let Some(CanonicalJsonValue::Object(update)) = status.get_mut("update") else {
        return Err(ResourceRuntimeError::HandlerNotReady);
    };
    update.insert(
        "observedGeneration".to_owned(),
        CanonicalJsonValue::Integer(resource.generation.get() as i64),
    );
    update.insert("lastAssessedAt".to_owned(), CanonicalJsonValue::String(now));
    let canonical = value.to_canonical_bytes();
    let envelope = ResourceEnvelope::from_json(&canonical)
        .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let digest = envelope
        .digest()
        .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = resource.zone.to_canonical_string();
    identity.resource_type = resource.resource_ref.resource_type().to_canonical_string();
    identity.name = resource.resource_ref.name().to_canonical_string();
    identity.uid = Some(resource.uid.as_str().to_owned());
    identity.generation = Some(resource.generation.get());
    identity.revision = Some(resource.revision.get());

    let mut resource_bytes = wire::ResourceEnvelopeBytes::new();
    resource_bytes.identity = protobuf::MessageField::some(identity.clone());
    resource_bytes.canonical_json = canonical;
    resource_bytes.payload_digest = digest;

    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(resource.revision.get());
    precondition.expected_uid = Some(resource.uid.as_str().to_owned());

    let operation = format!(
        "system-core-status-{}-{}",
        resource
            .resource_ref
            .to_canonical_string()
            .replace('/', "-"),
        resource.revision.get()
    );
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_STATUS);
    mutation.target = protobuf::MessageField::some(identity);
    mutation.precondition = protobuf::MessageField::some(precondition);
    mutation.resource = protobuf::MessageField::some(resource_bytes);

    let mut meta = wire::RequestMeta::new();
    meta.operation_id = operation.clone();
    meta.idempotency_key = operation.clone();
    meta.correlation_id = operation.clone();
    meta.trace_id = operation;
    meta.deadline_ms = 10_000;

    let mut request = wire::UpdateStatusRequest::new();
    request.meta = protobuf::MessageField::some(meta);
    request.mutation = protobuf::MessageField::some(mutation);
    let response = client.update_status(request).await;
    if let Some(error) = response.error.as_ref() {
        tracing::warn!(
            error_kind = ?error.kind,
            reason = %error.reason,
            "public Resource status update was refused"
        );
        return Err(ResourceRuntimeError::StoreReadFailed);
    }
    if response.resource.is_none() {
        return Err(ResourceRuntimeError::StoreReadFailed);
    }
    Ok(())
}

fn read_bounded(path: impl AsRef<Path>, limit: usize) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::with_capacity(limit.min(4096));
    file.by_ref()
        .take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bounded host probe exceeded limit",
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "host probe was not utf-8"))
}

fn is_socket(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false)
}

fn host_phase_for_resource_count(count: usize) -> HandlerPhase {
    if count == 0 {
        // A Zone without a Host resource has no host authority to bootstrap
        // dependent Providers. Keep the runtime visible, but do not publish
        // a falsely ready system-core handler.
        HandlerPhase::Degraded
    } else {
        HandlerPhase::Ready
    }
}

fn map_audio_runtime_error(error: AudioResourceRuntimeError) -> ResourceRuntimeError {
    match error {
        AudioResourceRuntimeError::Controller(_) => ResourceRuntimeError::CapabilityUnavailable,
        AudioResourceRuntimeError::InvalidResource
        | AudioResourceRuntimeError::InvalidRelationship => {
            ResourceRuntimeError::CapabilityUnavailable
        }
    }
}

fn map_process_runtime_error(error: ProcessResourceRuntimeError) -> ResourceRuntimeError {
    match error {
        ProcessResourceRuntimeError::Store => ResourceRuntimeError::StoreReadFailed,
        ProcessResourceRuntimeError::UnsupportedProvider
        | ProcessResourceRuntimeError::TemplateUnavailable
        | ProcessResourceRuntimeError::IdentityAmbiguous
        | ProcessResourceRuntimeError::ProviderEffect
        | ProcessResourceRuntimeError::InvalidResource => {
            ResourceRuntimeError::CapabilityUnavailable
        }
    }
}

fn map_activation_runtime_error(error: ActivationResourceRuntimeError) -> ResourceRuntimeError {
    match error {
        ActivationResourceRuntimeError::Store => ResourceRuntimeError::StoreReadFailed,
        ActivationResourceRuntimeError::InvalidResource
        | ActivationResourceRuntimeError::Policy => ResourceRuntimeError::CapabilityUnavailable,
    }
}

fn mark_core_handlers(
    core: &mut CoreProcess,
    phase: HandlerPhase,
    revision: u64,
) -> Result<(), ResourceRuntimeError> {
    let revision = revision.max(1);
    let status_for = |phase| HandlerStatus {
        phase,
        outcome: match phase {
            HandlerPhase::Ready => HandlerOutcome::Converged,
            HandlerPhase::Degraded => HandlerOutcome::Failed,
            HandlerPhase::Pending | HandlerPhase::Recovering => HandlerOutcome::Recovering,
            HandlerPhase::Failed => HandlerOutcome::Failed,
            HandlerPhase::Unknown => HandlerOutcome::Ambiguous,
        },
        observed_generation: revision,
        queued: 0,
        running: 0,
        last_watch_revision: revision,
        checkpoint_revision: revision,
        last_reconciled_tick: revision,
        retry_after_tick: None,
    };
    for kind in CoreHandlerKind::ALL {
        core.handlers_mut()
            .update(
                kind,
                status_for(if kind == CoreHandlerKind::Watches {
                    HandlerPhase::Ready
                } else {
                    phase
                }),
            )
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    }
    Ok(())
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

fn public_operation_id(request: &Value, peer_uid: u32, method: &str) -> String {
    request
        .get("operationId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            let resource_type = request
                .get("resourceType")
                .and_then(Value::as_str)
                .unwrap_or("resource");
            format!("public-{peer_uid}-{method}-{resource_type}")
        })
}

fn compatibility_error_envelope(error: ResourceRuntimeError) -> Value {
    let (kind, retry_class, reason) = match error {
        ResourceRuntimeError::AuthenticationUnavailable
        | ResourceRuntimeError::PolicyUnavailable
        | ResourceRuntimeError::IdentityUnbound => (
            ResourceErrorKind::AuthorizationDenied,
            RetryClass::Reauthorize,
            "authenticated local Zone session or policy is unavailable",
        ),
        ResourceRuntimeError::ControllerEndpointUnavailable
        | ResourceRuntimeError::WatchUnavailable
        | ResourceRuntimeError::AuthorityUnavailable
        | ResourceRuntimeError::HandlerNotReady
        | ResourceRuntimeError::ProviderPathUnavailable
        | ResourceRuntimeError::PlaneUnavailable
        | ResourceRuntimeError::CoreStartupFailed => (
            ResourceErrorKind::ResourcePlaneUnavailable,
            RetryClass::AfterDelay,
            "Zone resource runtime is not ready",
        ),
        ResourceRuntimeError::CapabilityUnavailable => (
            ResourceErrorKind::UnsupportedCapability,
            RetryClass::Never,
            "the requested resource operation is not registered",
        ),
        _ => (
            ResourceErrorKind::InternalIntegrityFailure,
            RetryClass::Never,
            "the public resource request was refused",
        ),
    };
    resource_error_envelope(
        &ResourceError::new(
            kind,
            None,
            None,
            retry_class,
            ResourceErrorReason::parse(reason).expect("fixed compatibility error reason"),
        )
        .expect("fixed compatibility error"),
    )
}

fn public_request_meta(operation_id: &str) -> wire::RequestMeta {
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = operation_id.to_owned();
    meta.idempotency_key = operation_id.to_owned();
    meta.correlation_id = operation_id.to_owned();
    meta.trace_id = operation_id.to_owned();
    meta.deadline_ms = 30_000;
    meta
}

fn local_operator_subject_context(
    zone: &ZoneId,
    peer_uid: u32,
    operation_id: &str,
) -> Result<AuthenticatedSubjectContext, ResourceRuntimeError> {
    let (subject_ref, subject_uid) = ZoneResourceRuntime::operator_subject_identity();
    let zone_ref = ResourceRef::parse(&format!("Zone/{}", zone.as_str()))
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;
    let schema_fingerprint = SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64)))
        .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;

    let mut transport_digest = Sha256::new();
    transport_digest.update(b"d2bd-public-resource-transport\0");
    transport_digest.update(peer_uid.to_le_bytes());
    transport_digest.update(zone.as_str().as_bytes());
    let transport_digest =
        BindingDigest::parse(format!("sha256:{:x}", transport_digest.finalize()))
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?;

    let mut transcript_digest = Sha256::new();
    transcript_digest.update(b"d2bd-public-resource-transcript\0");
    transcript_digest.update(peer_uid.to_le_bytes());
    transcript_digest.update(zone.as_str().as_bytes());
    transcript_digest.update(operation_id.as_bytes());
    let transcript_digest = TranscriptHash::from_bytes(transcript_digest.finalize().into());

    let session = SessionBinding::new(
        schema_fingerprint,
        TransportBinding::new(IdentityLocality::Local, transport_digest),
        ReconnectGeneration::new(1).map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
        transcript_digest,
    );
    Ok(AuthenticatedSubjectContext::new(
        subject_ref,
        subject_uid,
        zone_ref,
        EvidenceClass::UnixPeer,
        SessionPurpose::parse("zone-bus")
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
        ServiceName::parse("d2b.resource.v3")
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?,
        session,
    ))
}

fn public_list_request(parsed: ParsedListRequest, operation_id: &str) -> wire::ListRequest {
    let mut request = wire::ListRequest::new();
    request.meta = protobuf::MessageField::some(public_request_meta(operation_id));
    request.resource_types = parsed
        .resource_types
        .into_iter()
        .map(|resource_type| resource_type.to_canonical_string())
        .collect();
    request.filters = parsed
        .filters
        .into_iter()
        .map(|filter| {
            let mut wire_filter = wire::ListFilter::new();
            wire_filter.field = filter.field;
            wire_filter.values = filter.values;
            wire_filter
        })
        .collect();
    if !parsed.resource_names.is_empty() {
        let mut name_filter = wire::ListFilter::new();
        name_filter.field = "metadata.name".to_owned();
        name_filter.values = parsed
            .resource_names
            .into_iter()
            .map(|name| name.to_canonical_string())
            .collect();
        request.filters.push(name_filter);
    }
    request.page_size = parsed.page_size;
    if let Some(cursor) = parsed.cursor {
        let mut page_cursor = wire::PageCursor::new();
        page_cursor.value = cursor;
        request.cursor = protobuf::MessageField::some(page_cursor);
    }
    let mut projection = wire::Projection::new();
    projection.kind = protobuf::EnumOrUnknown::new(match parsed.projection {
        StoreProjection::Full => wire::ProjectionKind::PROJECTION_KIND_FULL,
        StoreProjection::BaseOnly => wire::ProjectionKind::PROJECTION_KIND_BASE_ONLY,
        StoreProjection::MetadataOnly => wire::ProjectionKind::PROJECTION_KIND_METADATA_ONLY,
    });
    request.projection = protobuf::MessageField::some(projection);
    request
}

fn encode_public_resource(
    resource: &wire::ResourceEnvelopeBytes,
) -> Result<Value, ResourceRuntimeError> {
    if resource.canonical_json.len() > MAX_RESPONSE_CANONICAL_BYTES {
        return Err(ResourceRuntimeError::ResponseInvalid);
    }
    let value: Value = serde_json::from_slice(&resource.canonical_json)
        .map_err(|_| ResourceRuntimeError::ResponseInvalid)?;
    if !value.is_object() {
        return Err(ResourceRuntimeError::ResponseInvalid);
    }
    Ok(value)
}

fn public_api_error(error: &wire::ResourceError) -> Value {
    let kind = resource_error_kind_from_wire(error.kind.enum_value().ok());
    let retry_class = retry_class_from_wire(error.retry_class.enum_value().ok());
    let current_revision = matches!(
        kind,
        ResourceErrorKind::ResourceConflict
            | ResourceErrorKind::AuthorizationDenied
            | ResourceErrorKind::RevisionExpired
    )
    .then(|| error.current_revision.map(ZoneRevision::new))
    .flatten();
    let retry_after_ms = error
        .retry_after_ms
        .filter(|delay| (1..=d2b_contracts::v3::MAX_RESOURCE_ERROR_RETRY_AFTER_MS).contains(delay));
    let retry_class = if retry_after_ms.is_some() {
        RetryClass::AfterDelay
    } else if retry_class == RetryClass::AfterDelay {
        RetryClass::Never
    } else {
        retry_class
    };
    let reason = ResourceErrorReason::parse("resource API returned a typed error")
        .expect("fixed public resource error reason");
    let error = ResourceError::new(kind, current_revision, retry_after_ms, retry_class, reason)
        .expect("fixed public resource error");
    resource_error_envelope(&error)
}

fn resource_error_kind_from_wire(kind: Option<wire::ResourceErrorKind>) -> ResourceErrorKind {
    match kind {
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_NOT_FOUND) => {
            ResourceErrorKind::ResourceNotFound
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_ALREADY_EXISTS) => {
            ResourceErrorKind::ResourceAlreadyExists
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_CONFLICT) => {
            ResourceErrorKind::ResourceConflict
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_SCHEMA_INVALID) => {
            ResourceErrorKind::ResourceSchemaInvalid
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_REF_INVALID) => {
            ResourceErrorKind::ResourceRefInvalid
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_OWNER_CYCLE) => {
            ResourceErrorKind::ResourceOwnerCycle
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_OWNER_DEPTH) => {
            ResourceErrorKind::ResourceOwnerDepth
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_FINALIZER_DENIED) => {
            ResourceErrorKind::ResourceFinalizerDenied
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_PROVIDER_UNAVAILABLE) => {
            ResourceErrorKind::ResourceProviderUnavailable
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_CONTROLLER_MISMATCH) => {
            ResourceErrorKind::ResourceControllerMismatch
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_STATUS_OWNER_MISMATCH) => {
            ResourceErrorKind::ResourceStatusOwnerMismatch
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_STATUS_OVERSIZE) => {
            ResourceErrorKind::StatusOversize
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_STATUS_PROVIDER_SCHEMA_INVALID) => {
            ResourceErrorKind::StatusProviderSchemaInvalid
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_STATUS_PROVIDER_OVERLAP) => {
            ResourceErrorKind::StatusProviderOverlap
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_SPEC_PROVIDER_SCHEMA_INVALID) => {
            ResourceErrorKind::SpecProviderSchemaInvalid
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_SPEC_PROVIDER_SHADOW) => {
            ResourceErrorKind::SpecProviderShadow
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_UNSUPPORTED_CAPABILITY) => {
            ResourceErrorKind::UnsupportedCapability
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_EXPEDITED_NOT_AUTHORIZED) => {
            ResourceErrorKind::ExpeditedNotAuthorized
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_EXPEDITED_QUOTA_EXCEEDED) => {
            ResourceErrorKind::ExpeditedQuotaExceeded
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_EXPEDITED_RECONCILE_PENDING) => {
            ResourceErrorKind::ExpeditedReconcilePending
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_UPGRADE_REQUIRED) => {
            ResourceErrorKind::UpgradeRequired
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_ENDPOINT_RESOLVE_DENIED) => {
            ResourceErrorKind::EndpointResolveDenied
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RELAY_DENIED) => {
            ResourceErrorKind::RelayDenied
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_ROLE_RELAY_GRANT_RESTRICTED) => {
            ResourceErrorKind::RoleRelayGrantRestricted
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_AUTHORIZATION_DENIED) => {
            ResourceErrorKind::AuthorizationDenied
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_REVISION_EXPIRED) => {
            ResourceErrorKind::RevisionExpired
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_BACKPRESSURE) => {
            ResourceErrorKind::Backpressure
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_TIMEOUT) => ResourceErrorKind::Timeout,
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_CANCELLED) => {
            ResourceErrorKind::Cancelled
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_PLANE_UNAVAILABLE) => {
            ResourceErrorKind::ResourcePlaneUnavailable
        }
        Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_INTERNAL_INTEGRITY_FAILURE)
        | Some(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_UNSPECIFIED)
        | None => ResourceErrorKind::InternalIntegrityFailure,
    }
}

fn retry_class_from_wire(retry_class: Option<wire::RetryClass>) -> RetryClass {
    match retry_class {
        Some(wire::RetryClass::RETRY_CLASS_IMMEDIATE) => RetryClass::Immediate,
        Some(wire::RetryClass::RETRY_CLASS_AFTER_DELAY) => RetryClass::AfterDelay,
        Some(wire::RetryClass::RETRY_CLASS_REAUTHORIZE) => RetryClass::Reauthorize,
        Some(wire::RetryClass::RETRY_CLASS_NEVER)
        | Some(wire::RetryClass::RETRY_CLASS_UNSPECIFIED)
        | None => RetryClass::Never,
    }
}

fn encode_public_get_response(response: wire::GetResponse) -> Result<Value, ResourceRuntimeError> {
    if let Some(error) = response.error.as_ref() {
        tracing::warn!(
            kind = ?error.kind,
            retry_class = ?error.retry_class,
            retry_after_ms = ?error.retry_after_ms,
            reason = %error.reason,
            "public Resource Get returned an API error"
        );
        return Ok(public_api_error(error));
    }
    let resource = response
        .resource
        .as_ref()
        .ok_or(ResourceRuntimeError::ResponseInvalid)?;
    encode_public_resource(resource)
}

fn encode_public_list_response(
    response: wire::ListResponse,
) -> Result<Value, ResourceRuntimeError> {
    if let Some(error) = response.error.as_ref() {
        return Ok(public_api_error(error));
    }
    let resources = response
        .resources
        .iter()
        .map(encode_public_resource)
        .collect::<Result<Vec<_>, _>>()?;
    let mut body = Map::new();
    body.insert("resources".to_owned(), Value::Array(resources));
    body.insert(
        "snapshotRevision".to_owned(),
        Value::Number(response.snapshot_revision.into()),
    );
    body.insert("truncated".to_owned(), Value::Bool(response.truncated));
    if let Some(cursor) = response.next_cursor.as_ref() {
        body.insert("nextCursor".to_owned(), Value::String(cursor.value.clone()));
    }
    Ok(Value::Object(body))
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

fn runtime_authorizer(
    bundle_resource_types: &[ResourceTypeName],
) -> Result<NativeAuthorizer, ResourceRuntimeError> {
    let catalog = ApiCatalog::with_extensions(
        bundle_resource_types
            .iter()
            .filter(|resource_type| resource_type.as_str().contains(".d2bus.org."))
            .cloned(),
    )
    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    NativeAuthorizer::new(catalog, None).map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)
}

fn initial_policy_snapshot() -> Result<PolicySnapshot, ResourceRuntimeError> {
    Ok(PolicySnapshot {
        policy_revision: 1,
        api_catalog_revision: 1,
        active_configuration_revision: ConfigurationGeneration::new(1)
            .map_err(|_| ResourceRuntimeError::StoreOpenFailed)?,
        controller_generation: Some(
            ControllerGeneration::new(1).map_err(|_| ResourceRuntimeError::StoreOpenFailed)?,
        ),
    })
}

async fn ensure_bootstrap_host_resource(
    zone: &ZoneId,
    store: &RedbResourceStore,
    client: &ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>,
) -> Result<(), ResourceRuntimeError> {
    let host_type =
        ResourceTypeName::parse("Host").map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let page = store
        .list(StoreListRequest {
            operation: StoreOperationContext {
                operation_id: "system-core-bootstrap-list-host".to_owned(),
                idempotency_key: None,
                correlation_id: "system-core-bootstrap-list-host".to_owned(),
                trace_id: None,
                deadline_ms: 10_000,
            },
            zone: zone.clone(),
            resource_types: vec![host_type],
            resource_names: Vec::new(),
            filters: Vec::new(),
            page_size: 2,
            cursor: None,
            projection: StoreProjection::MetadataOnly,
        })
        .await
        .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
    if !page.resources.is_empty() {
        return Ok(());
    }

    let payload = CanonicalJsonValue::parse(
        &serde_json::to_vec(&json!({
            "apiVersion": "resources.d2bus.org/v3",
            "metadata": {
                "configurationGeneration": 1,
                "createdAt": "1970-01-01T00:00:00.000Z",
                "deletionRequestedAt": null,
                "finalizers": [],
                "generation": 1,
                "managedBy": "configuration",
                "name": "host-system",
                "ownerRef": null,
                "revision": 1,
                "updatedAt": "1970-01-01T00:00:00.000Z",
                "zone": zone.as_str()
            },
            "spec": {
                "providerRef": HOST_PROVIDER_REF,
                "updatePolicy": {
                    "disruptive": "manual",
                    "nonDisruptive": "automatic"
                }
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
                    "lastAssessedAt": null,
                    "observedGeneration": 0,
                    "operationId": null,
                    "owned": {"count": 0, "refs": []},
                    "preserveState": true,
                    "reasons": [],
                    "state": "Unknown",
                    "targetGeneration": 1
                }
            },
            "type": "Host"
        }))
        .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
    )
    .map_err(|_| ResourceRuntimeError::HandlerNotReady)?
    .to_canonical_bytes();
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = zone.as_str().to_owned();
    identity.resource_type = "Host".to_owned();
    identity.name = "host-system".to_owned();
    let mut body = wire::ResourceEnvelopeBytes::new();
    body.identity = protobuf::MessageField::some(identity.clone());
    body.payload_digest = d2b_contracts::v3::canonical_digest(
        d2b_contracts::v3::RESOURCE_ENVELOPE_DOMAIN_TAG,
        &payload,
    );
    body.canonical_json = payload;
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT);
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_CREATE);
    mutation.target = protobuf::MessageField::some(identity);
    mutation.precondition = protobuf::MessageField::some(precondition);
    mutation.resource = protobuf::MessageField::some(body);
    let mut request = wire::CreateRequest::new();
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = "system-core-bootstrap-host".to_owned();
    meta.correlation_id = meta.operation_id.clone();
    meta.idempotency_key = meta.operation_id.clone();
    request.meta = protobuf::MessageField::some(meta);
    request.mutation = protobuf::MessageField::some(mutation);
    let response = client.create(request).await;
    if let Some(error) = response.error.as_ref() {
        tracing::error!(
            zone = %zone.as_str(),
            error_kind = ?error.kind,
            reason = %error.reason.as_str(),
            retry_class = ?error.retry_class,
            "bootstrap Host create failed",
        );
        return Err(ResourceRuntimeError::HandlerNotReady);
    }
    Ok(())
}

/// Materialize the verified Nix Zone bundle through the authenticated
/// system-core Resource API before any production composition reads the
/// store.  The store remains the authority for UIDs, revisions, ownership,
/// and update generation; this function only supplies desired state.
async fn materialize_zone_resource_bundle(
    zone: &ZoneId,
    bundle: &ResourceBundle,
    store: &RedbResourceStore,
    client: &ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>,
) -> Result<(), ResourceRuntimeError> {
    let metadata = store
        .runtime_metadata()
        .await
        .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
    let mut existing = BTreeMap::new();
    let mut cursor = None;
    loop {
        let page = store
            .list(StoreListRequest {
                operation: StoreOperationContext {
                    operation_id: "resource-bundle-materialization-list".to_owned(),
                    idempotency_key: None,
                    correlation_id: "resource-bundle-materialization-list".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: zone.clone(),
                resource_types: Vec::new(),
                resource_names: Vec::new(),
                filters: Vec::new(),
                page_size: 256,
                cursor,
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        for resource in page.resources {
            existing.insert(resource.resource_ref.clone(), resource);
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    let mut pending = bundle.resources.iter().collect::<Vec<_>>();
    let mut ordered = Vec::with_capacity(pending.len());
    let mut admitted_refs = existing.keys().cloned().collect::<BTreeSet<_>>();
    while !pending.is_empty() {
        let Some(index) = pending.iter().position(|resource| {
            resource
                .metadata()
                .owner_ref()
                .is_none_or(|owner| admitted_refs.contains(owner))
        }) else {
            return Err(ResourceRuntimeError::HandlerNotReady);
        };
        let resource = pending.remove(index);
        let resource_ref = ResourceRef::new(
            resource.resource_type().clone(),
            resource.metadata().name().clone(),
        );
        admitted_refs.insert(resource_ref);
        ordered.push(resource);
    }

    let active_configuration_generation =
        metadata.policy_snapshot.active_configuration_revision.get();
    let mut mutations = Vec::new();
    for resource in ordered {
        let resource_ref = ResourceRef::new(
            resource.resource_type().clone(),
            resource.metadata().name().clone(),
        );
        if let Some(current) = existing.get(&resource_ref) {
            let current_envelope = ResourceEnvelope::from_json(&current.canonical_json)
                .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
            if current_envelope.metadata().owner_ref() != resource.metadata().owner_ref() {
                return Err(ResourceRuntimeError::HandlerNotReady);
            }
            let desired_spec = resource.spec().to_canonical_bytes();
            let current_spec = current_envelope
                .spec()
                .canonical_bytes()
                .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
            if desired_spec != current_spec {
                let payload = update_resource_payload(&current.canonical_json, resource)?;
                mutations.push(update_mutation(
                    zone,
                    &resource_ref,
                    current_envelope.metadata().uid(),
                    current_envelope.metadata().revision(),
                    payload,
                )?);
            }
        } else {
            let payload = create_resource_payload(zone, resource, active_configuration_generation)?;
            mutations.push(create_mutation(zone, resource, payload)?);
        }
    }
    if mutations.is_empty() {
        return Ok(());
    }

    let operation_id = resource_bundle_materialization_operation_id(zone, bundle);
    let mut request = wire::CommitBatchRequest::new();
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = operation_id.clone();
    meta.idempotency_key = operation_id.clone();
    meta.correlation_id = operation_id;
    request.meta = protobuf::MessageField::some(meta);
    request.mutations = mutations;
    let response = client.commit_batch(request).await;
    if let Some(error) = response.error.as_ref() {
        tracing::error!(
            zone = %zone.as_str(),
            error_kind = ?error.kind,
            reason = %error.reason.as_str(),
            "authenticated Zone resource bundle materialization failed",
        );
        return Err(ResourceRuntimeError::HandlerNotReady);
    }
    Ok(())
}

pub(crate) fn resource_bundle_materialization_operation_id(
    zone: &ZoneId,
    bundle: &ResourceBundle,
) -> String {
    format!(
        "resource-bundle-materialization:{}:{}",
        zone.as_str(),
        bundle.integrity().content_hash
    )
}

fn create_resource_payload(
    zone: &ZoneId,
    resource: &d2b_contracts::v3::resource_bundle::BundleResource,
    configuration_generation: u64,
) -> Result<Vec<u8>, ResourceRuntimeError> {
    let mut value =
        serde_json::to_value(resource).map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let root = value
        .as_object_mut()
        .ok_or(ResourceRuntimeError::HandlerNotReady)?;
    let metadata = root
        .get_mut("metadata")
        .and_then(Value::as_object_mut)
        .ok_or(ResourceRuntimeError::HandlerNotReady)?;
    metadata.insert(
        "configurationGeneration".to_owned(),
        Value::from(configuration_generation),
    );
    metadata.insert(
        "createdAt".to_owned(),
        Value::String("1970-01-01T00:00:00.000Z".to_owned()),
    );
    metadata.insert("deletionRequestedAt".to_owned(), Value::Null);
    metadata.insert("finalizers".to_owned(), Value::Array(Vec::new()));
    metadata.insert("generation".to_owned(), Value::from(1_u64));
    metadata.insert(
        "managedBy".to_owned(),
        Value::String("configuration".to_owned()),
    );
    metadata.insert("revision".to_owned(), Value::from(1_u64));
    metadata.insert(
        "updatedAt".to_owned(),
        Value::String("1970-01-01T00:00:00.000Z".to_owned()),
    );
    metadata.insert("zone".to_owned(), Value::String(zone.as_str().to_owned()));
    root.insert(
        "status".to_owned(),
        json!({
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
                "lastAssessedAt": null,
                "observedGeneration": 0,
                "operationId": null,
                "owned": {"count": 0, "refs": []},
                "preserveState": true,
                "reasons": [],
                "state": "Unknown",
                "targetGeneration": 1
            }
        }),
    );
    let bytes = serde_json::to_vec(&value).map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    CanonicalJsonValue::parse(&bytes)
        .map(|value| value.to_canonical_bytes())
        .map_err(|_| ResourceRuntimeError::HandlerNotReady)
}

fn update_resource_payload(
    current: &[u8],
    resource: &d2b_contracts::v3::resource_bundle::BundleResource,
) -> Result<Vec<u8>, ResourceRuntimeError> {
    let mut value = serde_json::from_slice::<Value>(current)
        .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let desired_spec =
        serde_json::to_value(resource.spec()).map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    value
        .as_object_mut()
        .and_then(|root| root.get_mut("spec"))
        .map(|spec| *spec = desired_spec)
        .ok_or(ResourceRuntimeError::HandlerNotReady)?;
    let bytes = serde_json::to_vec(&value).map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    CanonicalJsonValue::parse(&bytes)
        .map(|value| value.to_canonical_bytes())
        .map_err(|_| ResourceRuntimeError::HandlerNotReady)
}

fn resource_identity(
    zone: &ZoneId,
    resource_type: &ResourceTypeName,
    name: &ResourceName,
    uid: Option<&ResourceUid>,
) -> wire::ResourceIdentity {
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = zone.as_str().to_owned();
    identity.resource_type = resource_type.as_str().to_owned();
    identity.name = name.as_str().to_owned();
    identity.uid = uid.map(|uid| uid.as_str().to_owned());
    identity
}

fn resource_envelope_body(
    identity: wire::ResourceIdentity,
    payload: Vec<u8>,
    payload_digest: String,
) -> wire::ResourceEnvelopeBytes {
    let mut body = wire::ResourceEnvelopeBytes::new();
    body.identity = protobuf::MessageField::some(identity);
    body.canonical_json = payload;
    body.payload_digest = payload_digest;
    body
}

fn create_mutation(
    zone: &ZoneId,
    resource: &d2b_contracts::v3::resource_bundle::BundleResource,
    payload: Vec<u8>,
) -> Result<wire::Mutation, ResourceRuntimeError> {
    let identity = resource_identity(
        zone,
        resource.resource_type(),
        resource.metadata().name(),
        None,
    );
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT);
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_CREATE);
    mutation.target = protobuf::MessageField::some(identity.clone());
    mutation.precondition = protobuf::MessageField::some(precondition);
    mutation.resource = protobuf::MessageField::some(resource_envelope_body(
        identity,
        payload.clone(),
        d2b_contracts::v3::canonical_digest(
            d2b_contracts::v3::RESOURCE_ENVELOPE_DOMAIN_TAG,
            &payload,
        ),
    ));
    if let Some(owner) = resource.metadata().owner_ref() {
        mutation.owner = protobuf::MessageField::some(resource_identity(
            zone,
            owner.resource_type(),
            owner.name(),
            None,
        ));
    }
    Ok(mutation)
}

fn update_mutation(
    zone: &ZoneId,
    resource_ref: &ResourceRef,
    uid: &ResourceUid,
    revision: ZoneRevision,
    payload: Vec<u8>,
) -> Result<wire::Mutation, ResourceRuntimeError> {
    let identity = resource_identity(
        zone,
        resource_ref.resource_type(),
        resource_ref.name(),
        Some(uid),
    );
    let envelope =
        ResourceEnvelope::from_json(&payload).map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(revision.get());
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_SPEC);
    mutation.target = protobuf::MessageField::some(identity.clone());
    mutation.precondition = protobuf::MessageField::some(precondition);
    mutation.resource = protobuf::MessageField::some(resource_envelope_body(
        identity,
        payload,
        envelope
            .digest()
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?,
    ));
    Ok(mutation)
}

fn store_identity(
    zone: &ZoneId,
    store_identity: &str,
) -> Result<StoreIdentity, ResourceRuntimeError> {
    let store_uuid = stable_uid("store", store_identity);
    let zone_uid = stable_uid("zone", zone.as_str());
    let created_at = Timestamp::parse("1970-01-01T00:00:00.000Z")
        .map_err(|_| ResourceRuntimeError::StoreOpenFailed)?;
    let mut revisions = initial_policy_snapshot()?;
    revisions.policy_revision = 0;
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
        let canonical_json = d2b_contracts::v3::canonical_json_bytes(&envelope).unwrap();
        let parsed = ResourceEnvelope::from_json(&canonical_json).unwrap();
        StoredResource {
            resource_ref: ResourceRef::parse(&format!("Provider/{name}")).unwrap(),
            zone,
            uid,
            generation: ResourceGeneration::new(1).unwrap(),
            revision: ZoneRevision::new(1),
            canonical_json,
            payload_digest: parsed.digest().unwrap(),
        }
    }

    fn clipboard_provider_config() -> Value {
        json!({
            "hostExecutionRef": "Host/host-system",
            "hostUserRef": "User/alice",
            "displayWaylandRef": "Provider/display-wayland",
            "guestSources": [{"guestRef": "Guest/workstation"}],
        })
    }

    fn notification_provider_config() -> Value {
        json!({
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
    fn stable_identity_is_repeatable_and_uuid_v4_shaped() {
        let first = stable_uid("store", "sha256:aaa");
        assert_eq!(first, stable_uid("store", "sha256:aaa"));
        assert_ne!(first, stable_uid("store", "sha256:bbb"));
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
    fn system_core_requires_a_host_but_accepts_multiple_host_resources() {
        assert_eq!(
            host_phase_for_resource_count(0),
            HandlerPhase::Degraded,
            "zero Host resources must not publish a ready handler"
        );
        assert_eq!(host_phase_for_resource_count(1), HandlerPhase::Ready);
        assert_eq!(host_phase_for_resource_count(2), HandlerPhase::Ready);
    }

    #[test]
    fn cleanup_pending_counts_only_deleted_prior_configuration_generations() {
        let mut resource = StoredResource {
            resource_ref: ResourceRef::parse("Host/host-system").unwrap(),
            zone: ZoneId::parse("dev").unwrap(),
            uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            generation: ResourceGeneration::new(1).unwrap(),
            revision: ZoneRevision::new(1),
            canonical_json: br#"{"metadata":{"managedBy":"configuration","configurationGeneration":3,"deletionRequestedAt":"2026-08-15T00:00:00Z"}}"#.to_vec(),
            payload_digest: String::new(),
        };
        assert!(configuration_cleanup_pending(&resource, 4));
        resource.canonical_json =
            br#"{"metadata":{"managedBy":"configuration","configurationGeneration":4,"deletionRequestedAt":"2026-08-15T00:00:00Z"}}"#.to_vec();
        assert!(!configuration_cleanup_pending(&resource, 4));
        resource.canonical_json =
            br#"{"metadata":{"managedBy":"operator","configurationGeneration":3,"deletionRequestedAt":"2026-08-15T00:00:00Z"}}"#.to_vec();
        assert!(!configuration_cleanup_pending(&resource, 4));
    }

    #[tokio::test]
    async fn completed_watch_handles_are_cleared_for_bounded_restart() {
        let completed = tokio::spawn(async {});
        while !completed.is_finished() {
            tokio::task::yield_now().await;
        }
        let mut slot = Some(completed);
        assert!(watch_needs_restart(&mut slot));
        assert!(slot.is_none());

        let running = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        });
        let mut slot = Some(running);
        assert!(!watch_needs_restart(&mut slot));
        slot.take().expect("running watch").abort();
    }

    #[tokio::test]
    async fn production_system_core_probe_returns_bounded_host_observations() {
        let probe = SystemCoreHostProbe::current();
        let metadata = probe
            .metadata()
            .await
            .expect("the local host metadata probe succeeds");
        assert!(!metadata.kernel_release.is_empty());
        assert!(metadata.kernel_release.len() <= 64);
        assert!(metadata.os_name.len() <= 128);
        let platform = probe
            .platform()
            .await
            .expect("the local platform probe succeeds");
        assert!(platform.kernel_major > 0);
        let pidfd = probe
            .probe(HostCapabilityClass::Pidfd)
            .await
            .expect("the pidfd capability probe succeeds");
        assert_eq!(
            pidfd,
            platform.kernel_major > 5 || (platform.kernel_major == 5 && platform.kernel_minor >= 3)
        );
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

    #[test]
    fn public_api_error_preserves_not_found_and_plane_kinds() {
        let mut not_found = wire::ResourceError::new();
        not_found.kind = protobuf::EnumOrUnknown::new(
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_NOT_FOUND,
        );
        not_found.retry_class = protobuf::EnumOrUnknown::new(wire::RetryClass::RETRY_CLASS_NEVER);
        assert_eq!(
            public_api_error(&not_found)["error"]["kind"],
            "resource-not-found"
        );

        let mut unavailable = wire::ResourceError::new();
        unavailable.kind = protobuf::EnumOrUnknown::new(
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_PLANE_UNAVAILABLE,
        );
        unavailable.retry_class =
            protobuf::EnumOrUnknown::new(wire::RetryClass::RETRY_CLASS_AFTER_DELAY);
        assert_eq!(
            public_api_error(&unavailable)["error"]["kind"],
            "resource-plane-unavailable"
        );
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
        assert!(runtime.device_tpm_controller_registered());
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
            .dispatch_public_cli_request(
                &json!({
                    "method": "List",
                    "zoneRef": "Zone/work",
                    "resourceType": "Host",
                }),
                1000,
            )
            .await
            .unwrap();
        assert!(peer_route["resources"].is_array());
        let direct_delete = runtime
            .dispatch_public_cli_request(
                &json!({
                    "method": "Delete",
                    "zoneRef": "Zone/work",
                    "resourceRef": "Host/host-system",
                }),
                1000,
            )
            .await
            .unwrap_err();
        assert_eq!(direct_delete, ResourceRuntimeError::CapabilityUnavailable);
        runtime.shutdown().await.unwrap();
    }
}
