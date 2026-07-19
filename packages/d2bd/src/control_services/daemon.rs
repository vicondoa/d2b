//! Authenticated `d2b.daemon.v2` service composition.

#[path = "../daemon_guest_proxy.rs"]
mod guest_proxy;

use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    os::fd::OwnedFd,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    public_wire,
    v2_component_session::{
        AttachmentPolicy, AttachmentPolicyKind, EndpointPolicy, EndpointPurpose, EndpointRole,
        IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, PurposeClass, RequestId,
        ServicePackage, TransportBinding, TransportClass,
    },
    v2_services::{
        StrictWireMessage, admit_metadata,
        common::{self, ServiceRequest, ServiceResponse},
        daemon,
        daemon_ttrpc::{DaemonService, create_daemon_service},
        guest_ttrpc::create_guest_service,
        public_daemon_schema_fingerprint, terminal, validate_terminal_open_response_for_request,
    },
};
use d2b_session::{
    Cancellation, ComponentSessionDriver, HandshakeCredentials, OwnedTransport, SessionEngine,
    TransportDescriptor, TransportError, TransportPacket,
};
use d2b_session_unix::{
    AncillaryCapacity, CreditPool, CreditScopeSet, OutboundPacket, SeqpacketSocket,
    UnixSessionError,
};
use futures::stream;
use protobuf::{EnumOrUnknown, MessageField};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::{
    ServerState,
    admission::{PeerIdentity, PeerRole},
    daemon_terminal::{
        CancelTerminalResult, PreparedTerminal, TerminalBinding, TerminalFailure,
        TerminalSessionManager, new_terminal_resource_handle, terminal_open_failure_response,
        terminal_open_success_response,
    },
    typed_error::TypedError,
};

const MAX_DAEMON_REQUEST_LIFETIME_MS: u64 = 15 * 60 * 1_000;
#[allow(dead_code)]
pub(super) fn owns(
    service: &d2b_contracts::v2_services::ServiceSpec,
    _: &d2b_contracts::v2_services::MethodSpec,
) -> bool {
    service.package == "d2b.daemon.v2"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaemonPeerRole {
    Launcher,
    Admin,
    HostShutdown,
}

impl From<PeerRole> for DaemonPeerRole {
    fn from(value: PeerRole) -> Self {
        match value {
            PeerRole::Launcher => Self::Launcher,
            PeerRole::Admin => Self::Admin,
            PeerRole::HostShutdown => Self::HostShutdown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaemonAdapter {
    Realm,
    Guest,
    Provider,
    Broker,
    Allocator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaemonMethod {
    Resolve,
    ListRealms,
    ListWorkloads,
    Inspect,
    Apply,
    Start,
    Stop,
    Restart,
    Exec,
    Shell,
    OpenConsole,
    ExportAudit,
}

impl DaemonMethod {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Resolve => "Resolve",
            Self::ListRealms => "ListRealms",
            Self::ListWorkloads => "ListWorkloads",
            Self::Inspect => "Inspect",
            Self::Apply => "Apply",
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
            Self::Exec => "Exec",
            Self::Shell => "Shell",
            Self::OpenConsole => "OpenConsole",
            Self::ExportAudit => "ExportAudit",
        }
    }

    pub const fn adapter(self) -> DaemonAdapter {
        match self {
            Self::Resolve | Self::ListRealms | Self::ListWorkloads => DaemonAdapter::Realm,
            Self::Inspect | Self::Start | Self::Stop | Self::Restart => DaemonAdapter::Provider,
            Self::Apply => DaemonAdapter::Allocator,
            Self::Exec | Self::Shell | Self::OpenConsole => DaemonAdapter::Guest,
            Self::ExportAudit => DaemonAdapter::Broker,
        }
    }

    const fn mutating(self) -> bool {
        matches!(
            self,
            Self::Apply
                | Self::Start
                | Self::Stop
                | Self::Restart
                | Self::Exec
                | Self::Shell
                | Self::OpenConsole
        )
    }
}

impl DaemonPeerRole {
    pub const fn permits(self, method: DaemonMethod) -> bool {
        match self {
            Self::Admin => true,
            Self::Launcher => matches!(
                method,
                DaemonMethod::Resolve
                    | DaemonMethod::ListRealms
                    | DaemonMethod::ListWorkloads
                    | DaemonMethod::Inspect
                    | DaemonMethod::Exec
                    | DaemonMethod::OpenConsole
            ),
            Self::HostShutdown => matches!(method, DaemonMethod::Stop),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonServiceFailure {
    InvalidRequest,
    PermissionDenied,
    DeadlineExceeded,
    Cancelled,
    ResourceExhausted,
    Backend,
}

impl fmt::Display for DaemonServiceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "daemon-request-invalid",
            Self::PermissionDenied => "daemon-admission-denied",
            Self::DeadlineExceeded => "daemon-deadline-exceeded",
            Self::Cancelled => "daemon-request-cancelled",
            Self::ResourceExhausted => "daemon-resource-exhausted",
            Self::Backend => "daemon-operation-failed",
        })
    }
}

impl std::error::Error for DaemonServiceFailure {}

#[derive(Clone)]
pub struct DaemonCallContext {
    pub peer_role: DaemonPeerRole,
    pub peer_uid: u32,
    pub request_id: RequestId,
    pub session_generation: u64,
    pub remaining: Duration,
    pub cancellation: Cancellation,
}

impl fmt::Debug for DaemonCallContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonCallContext")
            .field("peer_role", &self.peer_role)
            .field("peer_uid", &self.peer_uid)
            .field("request_id", &"<redacted>")
            .field("session_generation", &"<redacted>")
            .field("remaining", &self.remaining)
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish()
    }
}

#[async_trait]
pub trait DaemonOperationHandler: Send + Sync {
    async fn handle_service(
        &self,
        adapter: DaemonAdapter,
        method: DaemonMethod,
        request: ServiceRequest,
        context: &DaemonCallContext,
    ) -> Result<ServiceResponse, DaemonServiceFailure>;

    async fn list_realms(
        &self,
        request: ServiceRequest,
        context: &DaemonCallContext,
    ) -> Result<daemon::ListRealmsResponse, DaemonServiceFailure>;

    async fn list_workloads(
        &self,
        request: ServiceRequest,
        context: &DaemonCallContext,
    ) -> Result<daemon::ListWorkloadsResponse, DaemonServiceFailure>;

    async fn inspect(
        &self,
        request: ServiceRequest,
        context: &DaemonCallContext,
    ) -> Result<daemon::InspectResponse, DaemonServiceFailure>;

    async fn prepare_terminal(
        &self,
        method: DaemonMethod,
        request: &terminal::TerminalOpenRequest,
        context: &DaemonCallContext,
    ) -> Result<Arc<dyn PreparedTerminal>, TerminalFailure>;
}

pub(crate) struct ProductionDaemonOperationHandler {
    state: Arc<ServerState>,
    peer: PeerIdentity,
}

impl ProductionDaemonOperationHandler {
    pub(crate) fn new(state: Arc<ServerState>, peer: PeerIdentity) -> Self {
        Self { state, peer }
    }
}

#[async_trait]
impl DaemonOperationHandler for ProductionDaemonOperationHandler {
    async fn handle_service(
        &self,
        adapter: DaemonAdapter,
        method: DaemonMethod,
        request: ServiceRequest,
        context: &DaemonCallContext,
    ) -> Result<ServiceResponse, DaemonServiceFailure> {
        if context.cancellation.is_cancelled() {
            return Err(DaemonServiceFailure::Cancelled);
        }
        let state = Arc::clone(&self.state);
        let peer = self.peer.clone();
        let response = tokio::task::spawn_blocking(move || {
            dispatch_production(&state, &peer, adapter, method, &request)
        })
        .await
        .map_err(|_| DaemonServiceFailure::Backend)??;
        if context.cancellation.is_cancelled() {
            return Err(DaemonServiceFailure::Cancelled);
        }
        Ok(response)
    }

    async fn list_realms(
        &self,
        request: ServiceRequest,
        context: &DaemonCallContext,
    ) -> Result<daemon::ListRealmsResponse, DaemonServiceFailure> {
        if context.cancellation.is_cancelled() {
            return Err(DaemonServiceFailure::Cancelled);
        }
        build_list_realms_response(&self.state, &request, context.session_generation)
    }

    async fn list_workloads(
        &self,
        request: ServiceRequest,
        context: &DaemonCallContext,
    ) -> Result<daemon::ListWorkloadsResponse, DaemonServiceFailure> {
        if context.cancellation.is_cancelled() {
            return Err(DaemonServiceFailure::Cancelled);
        }
        let state = Arc::clone(&self.state);
        let peer_uid = self.peer.uid;
        tokio::task::spawn_blocking(move || {
            build_list_workloads_response(&state, &request, peer_uid)
        })
        .await
        .map_err(|_| DaemonServiceFailure::Backend)?
    }

    async fn inspect(
        &self,
        request: ServiceRequest,
        context: &DaemonCallContext,
    ) -> Result<daemon::InspectResponse, DaemonServiceFailure> {
        if context.cancellation.is_cancelled() {
            return Err(DaemonServiceFailure::Cancelled);
        }
        let state = Arc::clone(&self.state);
        let peer_uid = self.peer.uid;
        tokio::task::spawn_blocking(move || build_inspect_response(&state, &request, peer_uid))
            .await
            .map_err(|_| DaemonServiceFailure::Backend)?
    }

    async fn prepare_terminal(
        &self,
        method: DaemonMethod,
        request: &terminal::TerminalOpenRequest,
        context: &DaemonCallContext,
    ) -> Result<Arc<dyn PreparedTerminal>, TerminalFailure> {
        crate::terminal_owners::prepare(
            Arc::clone(&self.state),
            self.peer.clone(),
            terminal_kind(method).ok_or(TerminalFailure::Protocol)?,
            request,
            context.session_generation,
        )
        .await
    }
}

fn dispatch_production(
    state: &ServerState,
    peer: &PeerIdentity,
    adapter: DaemonAdapter,
    method: DaemonMethod,
    request: &ServiceRequest,
) -> Result<ServiceResponse, DaemonServiceFailure> {
    let value = match method {
        DaemonMethod::Start => {
            crate::daemon_provider_start(state, peer, lifecycle_request(request)?)
        }
        DaemonMethod::Stop => crate::daemon_provider_stop(state, peer, lifecycle_request(request)?),
        DaemonMethod::Restart => {
            crate::daemon_provider_restart(state, peer, lifecycle_request(request)?)
        }
        DaemonMethod::Apply => match request.desired_state.enum_value_or_default() {
            common::DesiredState::DESIRED_STATE_RUNNING => {
                crate::daemon_provider_start(state, peer, lifecycle_request(request)?)
            }
            common::DesiredState::DESIRED_STATE_STOPPED
            | common::DesiredState::DESIRED_STATE_ABSENT => {
                crate::daemon_provider_stop(state, peer, lifecycle_request(request)?)
            }
            _ => return Err(DaemonServiceFailure::InvalidRequest),
        },
        DaemonMethod::Resolve => {
            return projection_response(method, adapter, request, None);
        }
        DaemonMethod::ExportAudit => {
            return projection_response(method, adapter, request, None);
        }
        DaemonMethod::ListRealms
        | DaemonMethod::ListWorkloads
        | DaemonMethod::Inspect
        | DaemonMethod::Exec
        | DaemonMethod::Shell
        | DaemonMethod::OpenConsole => return Err(DaemonServiceFailure::InvalidRequest),
    }
    .map_err(map_typed_error)?;
    let encoded = serde_json::to_vec(&value).map_err(|_| DaemonServiceFailure::Backend)?;
    let digest: [u8; 32] = Sha256::digest(encoded).into();
    let response = ServiceResponse {
        outcome: common::Outcome::OUTCOME_SUCCEEDED.into(),
        operation_id: request.operation_id.clone(),
        resource_handle: request.resource_id.clone(),
        stream_id: request.stream_id.clone(),
        result_digest: digest.to_vec(),
        ..Default::default()
    };
    response
        .validate_wire(false)
        .map_err(|_| DaemonServiceFailure::Backend)?;
    Ok(response)
}

fn projection_response(
    method: DaemonMethod,
    adapter: DaemonAdapter,
    request: &ServiceRequest,
    stream_id: Option<String>,
) -> Result<ServiceResponse, DaemonServiceFailure> {
    let mut digest = Sha256::new();
    digest.update(b"d2b.daemon.v2\0");
    digest.update(method.name().as_bytes());
    digest.update([adapter as u8]);
    digest.update(request.resource_id.as_bytes());
    digest.update(request.operation_id.as_bytes());
    let response = ServiceResponse {
        outcome: common::Outcome::OUTCOME_ACCEPTED.into(),
        operation_id: request.operation_id.clone(),
        resource_handle: request.resource_id.clone(),
        stream_id: stream_id.unwrap_or_default(),
        result_digest: digest.finalize().to_vec(),
        ..Default::default()
    };
    response
        .validate_wire(false)
        .map_err(|_| DaemonServiceFailure::Backend)?;
    Ok(response)
}

fn lifecycle_request(
    request: &ServiceRequest,
) -> Result<public_wire::VmLifecycleRequest, DaemonServiceFailure> {
    if request.resource_id.is_empty() {
        return Err(DaemonServiceFailure::InvalidRequest);
    }
    Ok(public_wire::VmLifecycleRequest {
        vm: request.resource_id.clone(),
        flags: public_wire::MutationFlags {
            apply: true,
            ..Default::default()
        },
        force: false,
        no_wait_api: false,
    })
}

fn terminal_kind(method: DaemonMethod) -> Option<terminal::TerminalKind> {
    match method {
        DaemonMethod::Exec => Some(terminal::TerminalKind::TERMINAL_KIND_EXEC),
        DaemonMethod::Shell => Some(terminal::TerminalKind::TERMINAL_KIND_SHELL),
        DaemonMethod::OpenConsole => Some(terminal::TerminalKind::TERMINAL_KIND_CONSOLE),
        _ => None,
    }
}

fn peer_role_label(role: DaemonPeerRole) -> &'static str {
    match role {
        DaemonPeerRole::Launcher => "launcher",
        DaemonPeerRole::Admin => "admin",
        DaemonPeerRole::HostShutdown => "host-shutdown",
    }
}

fn build_list_realms_response(
    state: &ServerState,
    request: &ServiceRequest,
    generation: u64,
) -> Result<daemon::ListRealmsResponse, DaemonServiceFailure> {
    use d2b_contracts::v2_identity::{RealmId, RealmPath};
    use d2b_core::realm_controller_config::RealmControllerPlacement;

    let local_path = RealmPath::parse("local-root").map_err(|_| DaemonServiceFailure::Backend)?;
    let mut realms = vec![daemon::RealmProjection {
        realm_id: RealmId::derive(&local_path).as_str().to_owned(),
        realm_path: local_path.as_str().to_owned(),
        realm_label: "local-root".to_owned(),
        mode: EnumOrUnknown::new(daemon::RealmMode::REALM_MODE_HOST_LOCAL),
        state: EnumOrUnknown::new(daemon::RealmState::REALM_STATE_READY),
        cross_realm_policy: EnumOrUnknown::new(
            daemon::CrossRealmPolicy::CROSS_REALM_POLICY_DEFAULT_DENY,
        ),
        credential_boundary: EnumOrUnknown::new(
            daemon::CredentialBoundary::CREDENTIAL_BOUNDARY_HOST_LOCAL,
        ),
        generation,
        ..Default::default()
    }];
    if let Some(loaded) =
        crate::load_realm_controllers_config(&state.config.realm_controllers_config_path)
            .map_err(map_typed_error)?
    {
        for controller in loaded.config.controllers {
            let path = RealmPath::parse(controller.realm_path.as_str().to_owned())
                .map_err(|_| DaemonServiceFailure::Backend)?;
            let realm_id = RealmId::derive(&path);
            let host_local = controller.placement == RealmControllerPlacement::HostLocal;
            let gateway_name = d2b_contracts::v2_identity::WorkloadName::parse("gateway")
                .map_err(|_| DaemonServiceFailure::Backend)?;
            let gateway_id =
                d2b_contracts::v2_identity::WorkloadId::derive(&realm_id, &gateway_name);
            realms.push(daemon::RealmProjection {
                realm_id: realm_id.as_str().to_owned(),
                realm_path: path.as_str().to_owned(),
                realm_label: controller
                    .realm_path
                    .as_str()
                    .split('.')
                    .next()
                    .unwrap_or("realm")
                    .to_owned(),
                mode: EnumOrUnknown::new(if host_local {
                    daemon::RealmMode::REALM_MODE_HOST_LOCAL
                } else {
                    daemon::RealmMode::REALM_MODE_GATEWAY_BACKED
                }),
                state: EnumOrUnknown::new(if host_local {
                    daemon::RealmState::REALM_STATE_READY
                } else {
                    daemon::RealmState::REALM_STATE_UNAVAILABLE
                }),
                cross_realm_policy: EnumOrUnknown::new(
                    daemon::CrossRealmPolicy::CROSS_REALM_POLICY_DEFAULT_DENY,
                ),
                credential_boundary: EnumOrUnknown::new(if host_local {
                    daemon::CredentialBoundary::CREDENTIAL_BOUNDARY_HOST_LOCAL
                } else {
                    daemon::CredentialBoundary::CREDENTIAL_BOUNDARY_GATEWAY_GUEST
                }),
                gateway_workload_id: if host_local {
                    String::new()
                } else {
                    gateway_id.as_str().to_owned()
                },
                gateway_target: if host_local {
                    String::new()
                } else {
                    format!("gateway.{}.d2b", path.as_str())
                },
                generation,
                ..Default::default()
            });
        }
    }
    realms.sort_by(|left, right| left.realm_path.cmp(&right.realm_path));
    let (realms, page) = paginate(
        realms,
        request,
        "r",
        d2b_contracts::v2_services::MAX_DAEMON_REALMS,
    )?;
    let response = daemon::ListRealmsResponse {
        outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED),
        realms,
        page: MessageField::some(page),
        ..Default::default()
    };
    response
        .validate_wire(false)
        .map_err(|_| DaemonServiceFailure::Backend)?;
    Ok(response)
}

fn build_list_workloads_response(
    state: &ServerState,
    request: &ServiceRequest,
    peer_uid: u32,
) -> Result<daemon::ListWorkloadsResponse, DaemonServiceFailure> {
    let (workloads, _) = catalog_workload_projections(state, request, peer_uid, false)?;
    let (workloads, page) = paginate(
        workloads,
        request,
        "w",
        d2b_contracts::v2_services::MAX_DAEMON_WORKLOADS,
    )?;
    let response = daemon::ListWorkloadsResponse {
        outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED),
        workloads,
        page: MessageField::some(page),
        ..Default::default()
    };
    response
        .validate_wire(false)
        .map_err(|_| DaemonServiceFailure::Backend)?;
    Ok(response)
}

fn build_inspect_response(
    state: &ServerState,
    request: &ServiceRequest,
    peer_uid: u32,
) -> Result<daemon::InspectResponse, DaemonServiceFailure> {
    let (workloads, read_model) = catalog_workload_projections(state, request, peer_uid, true)?;
    let (workloads, page) = paginate(
        workloads,
        request,
        "i",
        d2b_contracts::v2_services::MAX_DAEMON_WORKLOADS,
    )?;
    let response = daemon::InspectResponse {
        outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED),
        workloads,
        page: MessageField::some(page),
        read_model,
        ..Default::default()
    };
    response
        .validate_wire(false)
        .map_err(|_| DaemonServiceFailure::Backend)?;
    Ok(response)
}

fn catalog_workload_projections(
    state: &ServerState,
    request: &ServiceRequest,
    peer_uid: u32,
    inspect: bool,
) -> Result<(Vec<daemon::WorkloadProjection>, String), DaemonServiceFailure> {
    let resolver = crate::load_bundle_resolver(state).map_err(map_typed_error)?;
    let catalog = crate::workload_dispatch::WorkloadCatalog::from_resolver(&resolver)
        .map_err(|_| DaemonServiceFailure::Backend)?;
    let generation = state.pidfd_table.generation().saturating_add(1);
    let mut workloads = catalog
        .entries()
        .filter(|entry| catalog_entry_matches(&entry.metadata.identity, &request.resource_id))
        .map(|entry| {
            let legacy = entry
                .metadata
                .identity
                .legacy_vm_name
                .as_ref()
                .and_then(|vm| resolver.manifest.vms.get(vm.as_str()));
            let (workload_state, availability) =
                crate::workload_runtime_status(state, peer_uid, entry);
            let environment =
                catalog_environment(&resolver, &entry.metadata.identity).or_else(|| {
                    legacy
                        .and_then(|legacy| legacy.env.as_deref())
                        .map(str::to_owned)
                });
            let declared_roles = catalog_declared_roles(&resolver, &entry.metadata.identity);
            project_catalog_workload(
                entry,
                legacy,
                workload_state,
                availability,
                generation,
                inspect,
                environment.as_deref(),
                declared_roles,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    workloads.sort_by(|left, right| {
        left.identity
            .as_ref()
            .map(|identity| identity.canonical_target.as_str())
            .cmp(
                &right
                    .identity
                    .as_ref()
                    .map(|identity| identity.canonical_target.as_str()),
            )
    });
    Ok((
        workloads,
        format!("realm-workload-catalog-v2;generation={generation};freshness=direct"),
    ))
}

fn catalog_entry_matches(
    identity: &d2b_core::workload_identity::WorkloadIdentity,
    resource_id: &str,
) -> bool {
    resource_id.is_empty()
        || resource_id == identity.canonical_target.to_canonical()
        || resource_id
            == format!(
                "{}.{}.local-root.d2b",
                identity.workload_id.as_str(),
                identity.realm_path.target_form()
            )
        || resource_id == identity.workload_id.as_str()
        || identity
            .legacy_vm_name
            .as_ref()
            .is_some_and(|vm| vm.as_str() == resource_id)
}

#[allow(clippy::too_many_arguments)]
fn project_catalog_workload(
    entry: &crate::workload_dispatch::CatalogEntry,
    legacy: Option<&d2b_core::manifest_v04::VmEntry>,
    workload_state: d2b_realm_core::WorkloadState,
    availability: public_wire::WorkloadAvailability,
    generation: u64,
    inspect: bool,
    environment: Option<&str>,
    declared_roles: Vec<String>,
) -> Result<daemon::WorkloadProjection, DaemonServiceFailure> {
    let identity = &entry.metadata.identity;
    let lifecycle = catalog_lifecycle(workload_state, availability);
    let runtime = public_wire::RuntimeSummary {
        detail: workload_state_label(workload_state).to_owned(),
        kind: Some(catalog_runtime_kind(entry.metadata.provider_kind).to_owned()),
        operation_capabilities: Default::default(),
        services: Vec::new(),
    };
    let (supported, unsupported) = catalog_runtime_capabilities(entry, legacy);
    let services = catalog_service_states(workload_state, entry.metadata.provider_kind, legacy);
    let autostart = legacy.map(|legacy| public_wire::VmAutostartPosture {
        mode: match legacy.runtime.autostart_policy {
            d2b_core::runtime::RuntimeAutostartPolicy::HostBootEligible => "enabled",
            d2b_core::runtime::RuntimeAutostartPolicy::Disabled => "disabled",
            d2b_core::runtime::RuntimeAutostartPolicy::ManualOnly
            | d2b_core::runtime::RuntimeAutostartPolicy::Unknown => "manual-only",
        }
        .to_owned(),
        reason: "typed-runtime-policy".to_owned(),
    });
    let bridge_checks = if inspect {
        legacy_bridge_checks(legacy, workload_state)
    } else {
        Vec::new()
    };
    project_workload(
        identity
            .legacy_vm_name
            .as_ref()
            .map(|vm| vm.as_str())
            .unwrap_or(identity.workload_id.as_str()),
        identity.workload_id.as_str(),
        environment,
        legacy.is_some_and(|legacy| legacy.graphics),
        legacy.is_some_and(|legacy| legacy.tpm),
        legacy.is_some_and(|legacy| legacy.usbip_yubikey),
        legacy.and_then(|legacy| legacy.static_ip.as_deref()),
        legacy.is_some_and(|legacy| legacy.is_net_vm),
        legacy.is_some_and(|legacy| legacy.ssh_user.is_some()),
        &lifecycle,
        &runtime,
        &supported,
        &unsupported,
        &services,
        autostart.as_ref(),
        None,
        None,
        &bridge_checks,
        Some(identity),
        generation,
    )
    .map(|mut projection| {
        projection.declared_roles = declared_roles;
        projection
    })
}

fn catalog_lifecycle(
    state: d2b_realm_core::WorkloadState,
    availability: public_wire::WorkloadAvailability,
) -> public_wire::VmLifecycle {
    let degraded = state == d2b_realm_core::WorkloadState::Failed
        || availability != public_wire::WorkloadAvailability::Ready;
    public_wire::VmLifecycle {
        degraded,
        degraded_reasons: if degraded {
            vec![public_wire::VmLifecycleDegradedReason {
                reason: workload_availability_label(availability).to_owned(),
                remediation: "inspect-workload".to_owned(),
            }]
        } else {
            Vec::new()
        },
        pending_restart: false,
        state: match state {
            d2b_realm_core::WorkloadState::Stopped => public_wire::VmLifecycleState::Stopped,
            d2b_realm_core::WorkloadState::Starting => public_wire::VmLifecycleState::Starting,
            d2b_realm_core::WorkloadState::Running => public_wire::VmLifecycleState::Running,
            d2b_realm_core::WorkloadState::Stopping => public_wire::VmLifecycleState::Stopping,
            d2b_realm_core::WorkloadState::Failed => public_wire::VmLifecycleState::Failed,
        },
    }
}

fn workload_state_label(state: d2b_realm_core::WorkloadState) -> &'static str {
    match state {
        d2b_realm_core::WorkloadState::Stopped => "stopped",
        d2b_realm_core::WorkloadState::Starting => "starting",
        d2b_realm_core::WorkloadState::Running => "running",
        d2b_realm_core::WorkloadState::Stopping => "stopping",
        d2b_realm_core::WorkloadState::Failed => "failed",
    }
}

fn workload_availability_label(availability: public_wire::WorkloadAvailability) -> &'static str {
    match availability {
        public_wire::WorkloadAvailability::Ready => "ready",
        public_wire::WorkloadAvailability::HelperUnavailable => "helper-unavailable",
        public_wire::WorkloadAvailability::HelperStale => "helper-stale",
        public_wire::WorkloadAvailability::UserManagerUnavailable => "user-manager-unavailable",
        public_wire::WorkloadAvailability::GraphicalSessionInactive => "graphical-session-inactive",
        public_wire::WorkloadAvailability::WaylandUnavailable => "wayland-unavailable",
        public_wire::WorkloadAvailability::ProxyUnavailable => "proxy-unavailable",
        public_wire::WorkloadAvailability::Degraded => "provider-degraded",
    }
}

fn catalog_runtime_kind(provider: d2b_realm_core::WorkloadProviderKind) -> &'static str {
    match provider {
        d2b_realm_core::WorkloadProviderKind::LocalVm => "nixos",
        d2b_realm_core::WorkloadProviderKind::QemuMedia => "qemu-media",
        d2b_realm_core::WorkloadProviderKind::ProviderManaged => "remote",
        d2b_realm_core::WorkloadProviderKind::UnsafeLocal => "unsafe-local",
    }
}

fn catalog_runtime_capabilities(
    entry: &crate::workload_dispatch::CatalogEntry,
    legacy: Option<&d2b_core::manifest_v04::VmEntry>,
) -> (Vec<String>, Vec<String>) {
    let mut supported = entry
        .metadata
        .capabilities
        .iter()
        .filter_map(|capability| match capability {
            d2b_realm_core::Capability::Lifecycle => Some("lifecycle"),
            d2b_realm_core::Capability::Exec => Some("exec"),
            d2b_realm_core::Capability::PersistentShell => Some("shell"),
            d2b_realm_core::Capability::Vsock => Some("guest-control"),
            d2b_realm_core::Capability::Virtiofs => Some("store-sync"),
            d2b_realm_core::Capability::WindowForwarding
            | d2b_realm_core::Capability::DisplayStreaming => Some("display"),
            d2b_realm_core::Capability::Usb | d2b_realm_core::Capability::Hotplug => {
                Some("usb-hotplug")
            }
            _ => None,
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if let Some(legacy) = legacy {
        for (enabled, capability) in [
            (legacy.runtime.capabilities.lifecycle, "lifecycle"),
            (legacy.runtime.capabilities.display, "display"),
            (legacy.runtime.capabilities.usb_hotplug, "usb-hotplug"),
            (legacy.runtime.capabilities.config_sync, "config-sync"),
            (legacy.runtime.capabilities.exec, "exec"),
            (legacy.runtime.capabilities.guest_control, "guest-control"),
            (
                legacy.runtime.capabilities.in_guest_observability,
                "in-guest-observability",
            ),
            (legacy.runtime.capabilities.keys, "keys"),
            (legacy.runtime.capabilities.store_sync, "store-sync"),
        ] {
            if enabled {
                supported.push(capability.to_owned());
            }
        }
    }
    supported.sort();
    supported.dedup();
    let known = [
        "lifecycle",
        "display",
        "usb-hotplug",
        "config-sync",
        "exec",
        "guest-control",
        "in-guest-observability",
        "keys",
        "shell",
        "store-sync",
    ];
    let unsupported = known
        .into_iter()
        .filter(|capability| !supported.iter().any(|value| value == capability))
        .map(str::to_owned)
        .collect();
    (supported, unsupported)
}

fn catalog_service_states(
    state: d2b_realm_core::WorkloadState,
    provider: d2b_realm_core::WorkloadProviderKind,
    legacy: Option<&d2b_core::manifest_v04::VmEntry>,
) -> public_wire::PublicVmServices {
    let running = state == d2b_realm_core::WorkloadState::Running;
    let state = if running { "active" } else { "inactive" };
    let vm_runtime = matches!(
        provider,
        d2b_realm_core::WorkloadProviderKind::LocalVm
            | d2b_realm_core::WorkloadProviderKind::QemuMedia
    );
    public_wire::PublicVmServices {
        gpu: legacy
            .is_some_and(|legacy| legacy.graphics)
            .then(|| state.to_owned()),
        microvm: if vm_runtime { state } else { "unsupported" }.to_owned(),
        d2b: "active".to_owned(),
        qemu_media: (provider == d2b_realm_core::WorkloadProviderKind::QemuMedia)
            .then(|| state.to_owned()),
        snd: legacy
            .is_some_and(|legacy| legacy.audio)
            .then(|| state.to_owned()),
        swtpm: legacy
            .is_some_and(|legacy| legacy.tpm)
            .then(|| state.to_owned()),
        video: None,
        virtiofsd: if vm_runtime { state } else { "unsupported" }.to_owned(),
    }
}

fn catalog_environment(
    resolver: &d2b_core::bundle_resolver::BundleResolver,
    identity: &d2b_core::workload_identity::WorkloadIdentity,
) -> Option<String> {
    resolver
        .realm_controllers
        .as_ref()?
        .controllers
        .iter()
        .filter_map(|controller| controller.local_runtime.as_ref())
        .flat_map(|runtime| &runtime.workloads)
        .find(|workload| workload.identity.as_ref() == Some(identity))
        .map(|workload| workload.env.as_str().to_owned())
}

fn catalog_declared_roles(
    resolver: &d2b_core::bundle_resolver::BundleResolver,
    identity: &d2b_core::workload_identity::WorkloadIdentity,
) -> Vec<String> {
    resolver
        .processes
        .vms
        .iter()
        .find(|vm| {
            identity
                .legacy_vm_name
                .as_ref()
                .is_some_and(|legacy| legacy.as_str() == vm.vm)
        })
        .map(|vm| {
            vm.nodes
                .iter()
                .filter_map(|node| {
                    serde_json::to_value(&node.role)
                        .ok()?
                        .as_str()
                        .map(str::to_owned)
                })
                .take(d2b_contracts::v2_services::MAX_DAEMON_SERVICES)
                .collect()
        })
        .unwrap_or_default()
}

fn legacy_bridge_checks(
    legacy: Option<&d2b_core::manifest_v04::VmEntry>,
    state: d2b_realm_core::WorkloadState,
) -> Vec<public_wire::BridgeCheck> {
    let Some(legacy) = legacy else {
        return Vec::new();
    };
    let Some(bridge) = legacy
        .bridge
        .as_ref()
        .and_then(|bridge| d2b_core::host::IfName::new(bridge.clone()).ok())
    else {
        return Vec::new();
    };
    vec![public_wire::BridgeCheck {
        bridge,
        present: state == d2b_realm_core::WorkloadState::Running,
        tap: d2b_core::host::IfName::new(legacy.tap.clone()).ok(),
    }]
}

fn paginate<T>(
    values: Vec<T>,
    request: &ServiceRequest,
    cursor_prefix: &str,
    default_limit: usize,
) -> Result<(Vec<T>, daemon::PageInfo), DaemonServiceFailure> {
    let total = values.len();
    let offset = if request.page_cursor.is_empty() {
        0
    } else {
        request
            .page_cursor
            .strip_prefix(cursor_prefix)
            .and_then(|value| value.strip_prefix('-'))
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|offset| *offset <= total)
            .ok_or(DaemonServiceFailure::InvalidRequest)?
    };
    let limit = if request.page_size == 0 {
        default_limit
    } else {
        (request.page_size as usize).min(default_limit)
    };
    let end = offset.saturating_add(limit).min(total);
    let returned = end.saturating_sub(offset);
    let truncated = end < total;
    let next_page_cursor = if truncated {
        format!("{cursor_prefix}-{end}")
    } else {
        String::new()
    };
    let page = daemon::PageInfo {
        truncated,
        next_page_cursor,
        returned_items: u32::try_from(returned)
            .map_err(|_| DaemonServiceFailure::ResourceExhausted)?,
        total_items_known: true,
        total_items: u32::try_from(total).map_err(|_| DaemonServiceFailure::ResourceExhausted)?,
        ..Default::default()
    };
    Ok((
        values.into_iter().skip(offset).take(returned).collect(),
        page,
    ))
}

#[cfg(test)]
fn project_list_entry(
    entry: &public_wire::ListEntry,
    generation: u64,
) -> Result<daemon::WorkloadProjection, DaemonServiceFailure> {
    project_workload(
        entry.vm.as_str(),
        entry.name.as_str(),
        entry.env.as_deref(),
        entry.graphics,
        entry.tpm,
        entry.usbip,
        entry.static_ip.as_deref(),
        entry.is_net_vm,
        entry.ssh_user.is_some(),
        &entry.lifecycle,
        &entry.runtime,
        &entry.runtime_capabilities,
        &entry.unsupported_capabilities,
        &entry.services,
        entry.autostart.as_ref(),
        entry.qemu_media.as_ref(),
        None,
        &[],
        entry.workload_identity.as_ref(),
        generation,
    )
}

#[cfg(test)]
fn project_status_entry(
    entry: &public_wire::VmStatus,
    generation: u64,
) -> Result<daemon::WorkloadProjection, DaemonServiceFailure> {
    project_workload(
        entry.vm.as_str(),
        entry.name.as_str(),
        entry.env.as_deref(),
        entry.graphics,
        entry.tpm,
        entry.usbip,
        entry.static_ip.as_deref(),
        entry.is_net_vm,
        entry.ssh_user.is_some(),
        &entry.lifecycle,
        &entry.runtime,
        &entry.runtime_capabilities,
        &entry.unsupported_capabilities,
        &entry.services,
        entry.autostart.as_ref(),
        entry.qemu_media.as_ref(),
        entry.usb.as_ref(),
        &entry.bridge_checks,
        entry.workload_identity.as_ref(),
        generation,
    )
}

#[allow(clippy::too_many_arguments)]
fn project_workload(
    vm: &str,
    name: &str,
    environment: Option<&str>,
    graphics: bool,
    tpm: bool,
    usbip: bool,
    static_ip: Option<&str>,
    is_net_workload: bool,
    ssh_configured: bool,
    lifecycle: &public_wire::VmLifecycle,
    runtime: &public_wire::RuntimeSummary,
    supported_capabilities: &[String],
    unsupported_capabilities: &[String],
    services: &public_wire::PublicVmServices,
    autostart: Option<&public_wire::VmAutostartPosture>,
    qemu_media: Option<&public_wire::QemuMediaStatus>,
    usb: Option<&public_wire::UsbipVmStatus>,
    bridge_checks: &[public_wire::BridgeCheck],
    identity: Option<&d2b_core::workload_identity::WorkloadIdentity>,
    generation: u64,
) -> Result<daemon::WorkloadProjection, DaemonServiceFailure> {
    let identity = project_workload_identity(vm, name, identity)?;
    let degraded_reasons = if lifecycle.degraded {
        let mut reasons = lifecycle
            .degraded_reasons
            .iter()
            .take(16)
            .map(|reason| daemon::DegradedReason {
                reason: bounded_public_detail(&reason.reason, "runtime-degraded"),
                remediation: bounded_public_detail(&reason.remediation, "inspect-runtime-state"),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        if reasons.is_empty() {
            reasons.push(daemon::DegradedReason {
                reason: "runtime-degraded".to_owned(),
                remediation: "inspect-runtime-state".to_owned(),
                ..Default::default()
            });
        }
        reasons
    } else {
        Vec::new()
    };
    let lifecycle = daemon::WorkloadLifecycleProjection {
        state: EnumOrUnknown::new(project_lifecycle_state(lifecycle.state)),
        degraded: lifecycle.degraded,
        pending_restart: lifecycle.pending_restart,
        degraded_reasons,
        generation: generation.max(1),
        ..Default::default()
    };
    let supported = project_capabilities(supported_capabilities);
    let supported_values = supported
        .iter()
        .map(EnumOrUnknown::value)
        .collect::<std::collections::BTreeSet<_>>();
    let unsupported = project_capabilities(unsupported_capabilities)
        .into_iter()
        .filter(|capability| !supported_values.contains(&capability.value()))
        .collect();
    let runtime = daemon::RuntimeProjection {
        kind: EnumOrUnknown::new(project_runtime_kind(runtime.kind.as_deref())),
        detail: bounded_public_detail(&runtime.detail, "unknown"),
        supported_capabilities: supported,
        unsupported_capabilities: unsupported,
        ..Default::default()
    };
    let projection = daemon::WorkloadProjection {
        identity: MessageField::some(identity),
        name: bounded_workload_name(name, vm)?,
        environment: environment
            .map(|value| bounded_workload_name(value, "default"))
            .transpose()?
            .unwrap_or_default(),
        graphics,
        tpm,
        usbip,
        static_ip: static_ip
            .and_then(|value| value.parse::<std::net::IpAddr>().ok())
            .map(|value| match value {
                std::net::IpAddr::V4(address) => address.octets().to_vec(),
                std::net::IpAddr::V6(address) => address.octets().to_vec(),
            })
            .unwrap_or_default(),
        is_net_workload,
        ssh_configured,
        lifecycle: MessageField::some(lifecycle),
        runtime: MessageField::some(runtime),
        services: project_services(services),
        autostart: autostart.map(project_autostart).into(),
        qemu_media: qemu_media.and_then(project_qemu_media).into(),
        usb: usb.map(project_usb).into(),
        bridge_checks: bridge_checks
            .iter()
            .take(32)
            .map(|bridge| daemon::BridgeProjection {
                bridge: bridge.bridge.as_str().to_owned(),
                present: bridge.present,
                tap: bridge
                    .tap
                    .as_ref()
                    .map(|tap| tap.as_str().to_owned())
                    .unwrap_or_default(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    Ok(projection)
}

fn project_workload_identity(
    vm: &str,
    name: &str,
    identity: Option<&d2b_core::workload_identity::WorkloadIdentity>,
) -> Result<daemon::WorkloadIdentityProjection, DaemonServiceFailure> {
    use d2b_contracts::v2_identity::{RealmId, RealmPath, WorkloadId, WorkloadName};

    let realm_path_text = identity
        .map(|identity| format!("{}.local-root", identity.realm_path.target_form()))
        .unwrap_or_else(|| "local-root".to_owned());
    let realm_path =
        RealmPath::parse(realm_path_text).map_err(|_| DaemonServiceFailure::Backend)?;
    let realm_id = RealmId::derive(&realm_path);
    let workload_name_text = identity
        .map(|identity| identity.workload_id.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(if name.is_empty() { vm } else { name });
    let workload_name = WorkloadName::parse(workload_name_text.to_owned())
        .map_err(|_| DaemonServiceFailure::Backend)?;
    let workload_id = WorkloadId::derive(&realm_id, &workload_name);
    let canonical_target = format!("{}.{}.d2b", workload_name.as_str(), realm_path.as_str());
    Ok(daemon::WorkloadIdentityProjection {
        realm_id: realm_id.as_str().to_owned(),
        workload_id: workload_id.as_str().to_owned(),
        realm_path: realm_path.as_str().to_owned(),
        workload_name: workload_name.as_str().to_owned(),
        canonical_target,
        ..Default::default()
    })
}

fn bounded_workload_name(value: &str, fallback: &str) -> Result<String, DaemonServiceFailure> {
    d2b_contracts::v2_identity::WorkloadName::parse(value.to_owned())
        .map(|name| name.as_str().to_owned())
        .or_else(|_| {
            d2b_contracts::v2_identity::WorkloadName::parse(fallback.to_owned())
                .map(|name| name.as_str().to_owned())
        })
        .map_err(|_| DaemonServiceFailure::Backend)
}

fn bounded_public_detail(value: &str, fallback: &str) -> String {
    if value.is_ascii() && !value.is_empty() && value.len() <= 256 && !value.contains('/') {
        value.to_owned()
    } else {
        fallback.to_owned()
    }
}

fn project_lifecycle_state(state: public_wire::VmLifecycleState) -> daemon::WorkloadLifecycleState {
    match state {
        public_wire::VmLifecycleState::Stopped => {
            daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_STOPPED
        }
        public_wire::VmLifecycleState::Starting => {
            daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_STARTING
        }
        public_wire::VmLifecycleState::Booted => {
            daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_BOOTED
        }
        public_wire::VmLifecycleState::Running => {
            daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_RUNNING
        }
        public_wire::VmLifecycleState::Stopping => {
            daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_STOPPING
        }
        public_wire::VmLifecycleState::Restarting => {
            daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_RESTARTING
        }
        public_wire::VmLifecycleState::Failed => {
            daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_FAILED
        }
        public_wire::VmLifecycleState::Unknown => {
            daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_UNKNOWN
        }
    }
}

fn project_runtime_kind(kind: Option<&str>) -> daemon::RuntimeKind {
    match kind {
        Some("qemu-media") => daemon::RuntimeKind::RUNTIME_KIND_QEMU_MEDIA,
        Some("unsafe-local") => daemon::RuntimeKind::RUNTIME_KIND_UNSAFE_LOCAL,
        Some("aca-sandbox") => daemon::RuntimeKind::RUNTIME_KIND_ACA_SANDBOX,
        Some("remote") => daemon::RuntimeKind::RUNTIME_KIND_REMOTE,
        _ => daemon::RuntimeKind::RUNTIME_KIND_NIXOS,
    }
}

fn project_capabilities(values: &[String]) -> Vec<EnumOrUnknown<daemon::RuntimeCapability>> {
    let mut projected = values
        .iter()
        .filter_map(|value| {
            let capability = match value.as_str() {
                "lifecycle" => daemon::RuntimeCapability::RUNTIME_CAPABILITY_LIFECYCLE,
                "display" => daemon::RuntimeCapability::RUNTIME_CAPABILITY_DISPLAY,
                "usb-hotplug" => daemon::RuntimeCapability::RUNTIME_CAPABILITY_USB_HOTPLUG,
                "config-sync" => daemon::RuntimeCapability::RUNTIME_CAPABILITY_CONFIG_SYNC,
                "exec" => daemon::RuntimeCapability::RUNTIME_CAPABILITY_EXEC,
                "guest-control" => daemon::RuntimeCapability::RUNTIME_CAPABILITY_GUEST_CONTROL,
                "in-guest-observability" => {
                    daemon::RuntimeCapability::RUNTIME_CAPABILITY_IN_GUEST_OBSERVABILITY
                }
                "keys" => daemon::RuntimeCapability::RUNTIME_CAPABILITY_KEYS,
                "shell" => daemon::RuntimeCapability::RUNTIME_CAPABILITY_SHELL,
                "store-sync" => daemon::RuntimeCapability::RUNTIME_CAPABILITY_STORE_SYNC,
                _ => return None,
            };
            Some(EnumOrUnknown::new(capability))
        })
        .collect::<Vec<_>>();
    projected.sort_by_key(EnumOrUnknown::value);
    projected.dedup_by_key(|value| value.value());
    projected.truncate(32);
    projected
}

fn project_services(services: &public_wire::PublicVmServices) -> Vec<daemon::ServiceProjection> {
    let mut projected = vec![
        service_projection(
            daemon::ServiceKind::SERVICE_KIND_DAEMON,
            "daemon",
            &services.d2b,
        ),
        service_projection(
            daemon::ServiceKind::SERVICE_KIND_HYPERVISOR,
            "hypervisor",
            &services.microvm,
        ),
        service_projection(
            daemon::ServiceKind::SERVICE_KIND_VIRTIOFSD,
            "virtiofsd",
            &services.virtiofsd,
        ),
    ];
    for (value, kind, role) in [
        (
            services.gpu.as_deref(),
            daemon::ServiceKind::SERVICE_KIND_GPU,
            "gpu",
        ),
        (
            services.qemu_media.as_deref(),
            daemon::ServiceKind::SERVICE_KIND_QEMU_MEDIA,
            "qemu-media",
        ),
        (
            services.snd.as_deref(),
            daemon::ServiceKind::SERVICE_KIND_AUDIO,
            "audio",
        ),
        (
            services.swtpm.as_deref(),
            daemon::ServiceKind::SERVICE_KIND_SWTPM,
            "swtpm",
        ),
        (
            services.video.as_deref(),
            daemon::ServiceKind::SERVICE_KIND_VIDEO,
            "video",
        ),
    ] {
        if let Some(value) = value {
            projected.push(service_projection(kind, role, value));
        }
    }
    projected
}

fn service_projection(
    kind: daemon::ServiceKind,
    role_id: &str,
    state: &str,
) -> daemon::ServiceProjection {
    daemon::ServiceProjection {
        kind: EnumOrUnknown::new(kind),
        role_id: role_id.to_owned(),
        state: EnumOrUnknown::new(project_service_state(state)),
        ..Default::default()
    }
}

fn project_service_state(value: &str) -> daemon::ServiceState {
    match value.to_ascii_lowercase().as_str() {
        "active" | "running" | "ready" => daemon::ServiceState::SERVICE_STATE_ACTIVE,
        "inactive" | "stopped" => daemon::ServiceState::SERVICE_STATE_INACTIVE,
        "starting" | "activating" => daemon::ServiceState::SERVICE_STATE_STARTING,
        "stopping" | "deactivating" => daemon::ServiceState::SERVICE_STATE_STOPPING,
        "failed" | "error" => daemon::ServiceState::SERVICE_STATE_FAILED,
        "unavailable" | "missing" => daemon::ServiceState::SERVICE_STATE_UNAVAILABLE,
        "unsupported" | "not-declared" => daemon::ServiceState::SERVICE_STATE_UNSUPPORTED,
        _ => daemon::ServiceState::SERVICE_STATE_UNKNOWN,
    }
}

fn project_autostart(value: &public_wire::VmAutostartPosture) -> daemon::AutostartProjection {
    daemon::AutostartProjection {
        mode: EnumOrUnknown::new(match value.mode.as_str() {
            "enabled" | "host-boot-eligible" => daemon::AutostartMode::AUTOSTART_MODE_ENABLED,
            "disabled" => daemon::AutostartMode::AUTOSTART_MODE_DISABLED,
            _ => daemon::AutostartMode::AUTOSTART_MODE_MANUAL_ONLY,
        }),
        reason: bounded_public_detail(&value.reason, "operator-controlled"),
        ..Default::default()
    }
}

fn project_qemu_media(value: &public_wire::QemuMediaStatus) -> Option<daemon::QemuMediaProjection> {
    let media = value
        .media
        .iter()
        .take(32)
        .filter(|entry| {
            !entry.media_ref.is_empty()
                && entry.media_ref.len() <= 64
                && !entry.slot.is_empty()
                && entry.slot.len() <= 64
        })
        .map(|entry| daemon::QemuMediaSourceProjection {
            media_ref: entry.media_ref.clone(),
            slot: entry.slot.clone(),
            source_kind: EnumOrUnknown::new(match entry.source_kind.as_str() {
                "physical-usb" => daemon::QemuMediaSourceKind::QEMU_MEDIA_SOURCE_KIND_PHYSICAL_USB,
                _ => daemon::QemuMediaSourceKind::QEMU_MEDIA_SOURCE_KIND_IMAGE_FILE,
            }),
            format: EnumOrUnknown::new(match entry.format.as_str() {
                "qcow2" => daemon::QemuMediaFormat::QEMU_MEDIA_FORMAT_QCOW2,
                "iso" => daemon::QemuMediaFormat::QEMU_MEDIA_FORMAT_ISO,
                _ => daemon::QemuMediaFormat::QEMU_MEDIA_FORMAT_RAW,
            }),
            read_only: entry.read_only,
            registry: MessageField::some(daemon::QemuMediaRegistryProjection {
                state: EnumOrUnknown::new(project_service_state(&entry.registry.state)),
                remediation: entry
                    .registry
                    .remediation
                    .as_deref()
                    .map(|detail| bounded_public_detail(detail, "inspect-media-registry"))
                    .unwrap_or_default(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();
    Some(daemon::QemuMediaProjection {
        firmware_mode: EnumOrUnknown::new(match value.firmware_mode.as_str() {
            "uefi" => daemon::QemuMediaFirmwareMode::QEMU_MEDIA_FIRMWARE_MODE_UEFI,
            _ => daemon::QemuMediaFirmwareMode::QEMU_MEDIA_FIRMWARE_MODE_NONE,
        }),
        runner_state: EnumOrUnknown::new(project_service_state(&value.runner.state)),
        qmp_readiness: EnumOrUnknown::new(match value.runner.qmp_readiness.as_deref() {
            Some("ready") => daemon::QemuMediaReadiness::QEMU_MEDIA_READINESS_READY,
            Some("pending") => daemon::QemuMediaReadiness::QEMU_MEDIA_READINESS_PENDING,
            _ => daemon::QemuMediaReadiness::QEMU_MEDIA_READINESS_NOT_STARTED,
        }),
        pre_cont_progress: EnumOrUnknown::new(match value.runner.pre_cont_progress.as_str() {
            "running" => daemon::QemuMediaProgress::QEMU_MEDIA_PROGRESS_RUNNING,
            "paused-before-cont" => {
                daemon::QemuMediaProgress::QEMU_MEDIA_PROGRESS_PAUSED_BEFORE_CONT
            }
            "waiting-for-qmp" => daemon::QemuMediaProgress::QEMU_MEDIA_PROGRESS_WAITING_FOR_QMP,
            _ => daemon::QemuMediaProgress::QEMU_MEDIA_PROGRESS_NOT_STARTED,
        }),
        media,
        ..Default::default()
    })
}

fn project_usb(value: &public_wire::UsbipVmStatus) -> daemon::UsbProjection {
    let devices = value
        .entries
        .iter()
        .take(32)
        .filter_map(|entry| {
            let device_id = if !entry.bus_id.is_empty() {
                entry.bus_id.clone()
            } else {
                entry.slot.clone().unwrap_or_default()
            };
            if device_id.is_empty() || device_id.len() > 64 {
                return None;
            }
            let degraded_reasons = if entry.degraded_reasons.is_empty() {
                Vec::new()
            } else {
                vec![daemon::DegradedReason {
                    reason: "usb-device-degraded".to_owned(),
                    remediation: "inspect-usb-state".to_owned(),
                    ..Default::default()
                }]
            };
            Some(daemon::UsbDeviceProjection {
                device_id,
                state: EnumOrUnknown::new(project_usb_state(entry.status)),
                slot: entry.slot.clone().unwrap_or_default(),
                media_ref: entry
                    .media_ref
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                candidate_device_ids: entry
                    .candidate_bus_ids
                    .iter()
                    .filter(|candidate| !candidate.is_empty() && candidate.len() <= 64)
                    .take(32)
                    .cloned()
                    .collect(),
                degraded_reasons,
                ..Default::default()
            })
        })
        .collect();
    daemon::UsbProjection {
        degraded: value.degraded,
        devices,
        ..Default::default()
    }
}

fn project_usb_state(value: public_wire::UsbipProbeStatus) -> daemon::UsbDeviceState {
    match value {
        public_wire::UsbipProbeStatus::Bound | public_wire::UsbipProbeStatus::Enrolled => {
            daemon::UsbDeviceState::USB_DEVICE_STATE_ATTACHED
        }
        public_wire::UsbipProbeStatus::Unbound => daemon::UsbDeviceState::USB_DEVICE_STATE_DETACHED,
        public_wire::UsbipProbeStatus::Enrollable => daemon::UsbDeviceState::USB_DEVICE_STATE_READY,
        public_wire::UsbipProbeStatus::Stale | public_wire::UsbipProbeStatus::Degraded => {
            daemon::UsbDeviceState::USB_DEVICE_STATE_DEGRADED
        }
        public_wire::UsbipProbeStatus::DirectConfig => {
            daemon::UsbDeviceState::USB_DEVICE_STATE_READY
        }
        public_wire::UsbipProbeStatus::Unknown => {
            daemon::UsbDeviceState::USB_DEVICE_STATE_UNAVAILABLE
        }
    }
}

fn map_typed_error(error: TypedError) -> DaemonServiceFailure {
    match error {
        TypedError::AuthzNotAdmin { .. } | TypedError::AuthzNotALauncher { .. } => {
            DaemonServiceFailure::PermissionDenied
        }
        TypedError::DaemonBusy => DaemonServiceFailure::ResourceExhausted,
        _ => DaemonServiceFailure::Backend,
    }
}

pub struct DaemonServiceV2<H> {
    handler: Arc<H>,
    session: Arc<dyn ComponentSessionDriver>,
    peer_role: DaemonPeerRole,
    peer_uid: u32,
    active: Arc<Mutex<BTreeMap<Vec<u8>, Cancellation>>>,
    in_flight: Arc<tokio::sync::Semaphore>,
    terminals: Arc<TerminalSessionManager>,
}

impl<H> DaemonServiceV2<H> {
    pub fn new(
        handler: Arc<H>,
        session: Arc<dyn ComponentSessionDriver>,
        peer_role: DaemonPeerRole,
        peer_uid: u32,
    ) -> Self {
        let terminals = TerminalSessionManager::new(
            Arc::clone(&session),
            LimitProfile::local_default().active_named_streams as usize,
        )
        .expect("established daemon session has a valid terminal limit");
        Self {
            handler,
            session,
            peer_role,
            peer_uid,
            active: Arc::new(Mutex::new(BTreeMap::new())),
            in_flight: Arc::new(tokio::sync::Semaphore::new(64)),
            terminals,
        }
    }

    pub(crate) fn peer_role(&self) -> DaemonPeerRole {
        self.peer_role
    }

    pub(crate) fn peer_uid(&self) -> u32 {
        self.peer_uid
    }

    pub(crate) fn in_flight(&self) -> Arc<tokio::sync::Semaphore> {
        Arc::clone(&self.in_flight)
    }

    pub(crate) fn terminals(&self) -> Arc<TerminalSessionManager> {
        Arc::clone(&self.terminals)
    }

    pub(crate) fn generation(&self) -> u64 {
        self.session.generation()
    }

    pub(crate) fn cancel_request(
        &self,
        request: &common::CancelRequest,
    ) -> ttrpc::Result<common::CancelResponse> {
        request
            .validate_wire(false)
            .map_err(|_| invalid_request())?;
        let outcome = if request.session_generation != self.session.generation() {
            common::CancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH
        } else {
            let unary = self
                .active
                .lock()
                .map_err(|_| response_error())?
                .get(&request.request_id)
                .cloned();
            match unary {
                Some(cancellation) if cancellation.cancel() => {
                    common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
                }
                Some(_) => common::CancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL,
                None => match self
                    .terminals
                    .cancel(request.session_generation, &request.request_id)
                {
                    CancelTerminalResult::Signalled => {
                        common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
                    }
                    CancelTerminalResult::AlreadyTerminal => {
                        common::CancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL
                    }
                    CancelTerminalResult::GenerationMismatch => {
                        common::CancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH
                    }
                    CancelTerminalResult::Unknown => {
                        common::CancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST
                    }
                },
            }
        };
        let response = common::CancelResponse {
            outcome: outcome.into(),
            ..Default::default()
        };
        response
            .validate_wire(false)
            .map_err(|_| response_error())?;
        Ok(response)
    }
}

impl<H: DaemonOperationHandler> DaemonServiceV2<H> {
    async fn dispatch(
        &self,
        ttrpc_context: &ttrpc::r#async::TtrpcContext,
        method: DaemonMethod,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        let _permit = Arc::clone(&self.in_flight)
            .try_acquire_owned()
            .map_err(|_| resource_exhausted())?;
        let admitted = self
            .admit(ttrpc_context, method, request.metadata.as_ref())
            .await?;
        request
            .validate_wire(method.mutating())
            .map_err(|_| invalid_request())?;
        if !request.attachment_indexes.is_empty() {
            return admitted.finish(Err(invalid_request())).await;
        }
        let result = tokio::select! {
            biased;
            () = admitted.context.cancellation.cancelled() => Err(cancelled()),
            response = tokio::time::timeout(
                admitted.context.remaining,
                self.handler.handle_service(method.adapter(), method, request, &admitted.context),
            ) => match response {
                Ok(Ok(response)) => {
                    response.validate_wire(false).map_err(|_| response_error())?;
                    Ok(response)
                }
                Ok(Err(error)) => Err(service_error(error)),
                Err(_) => {
                    let _ = admitted.context.cancellation.cancel();
                    Err(deadline_exceeded())
                }
            }
        };
        admitted.finish(result).await
    }

    async fn dispatch_typed<T, F, Fut>(
        &self,
        ttrpc_context: &ttrpc::r#async::TtrpcContext,
        method: DaemonMethod,
        request: ServiceRequest,
        call: F,
    ) -> ttrpc::Result<T>
    where
        T: StrictWireMessage,
        F: FnOnce(Arc<H>, ServiceRequest, DaemonCallContext) -> Fut,
        Fut: std::future::Future<Output = Result<T, DaemonServiceFailure>>,
    {
        let _permit = Arc::clone(&self.in_flight)
            .try_acquire_owned()
            .map_err(|_| resource_exhausted())?;
        let admitted = self
            .admit(ttrpc_context, method, request.metadata.as_ref())
            .await?;
        request
            .validate_wire(method.mutating())
            .map_err(|_| invalid_request())?;
        if !request.attachment_indexes.is_empty() || !request.stream_id.is_empty() {
            return admitted.finish(Err(invalid_request())).await;
        }
        let handler = Arc::clone(&self.handler);
        let context = admitted.context.clone();
        let result = tokio::select! {
            biased;
            () = admitted.context.cancellation.cancelled() => Err(cancelled()),
            response = tokio::time::timeout(
                admitted.context.remaining,
                call(handler, request, context),
            ) => match response {
                Ok(Ok(response)) => {
                    response.validate_wire(false).map_err(|_| response_error())?;
                    Ok(response)
                }
                Ok(Err(error)) => Err(service_error(error)),
                Err(_) => {
                    let _ = admitted.context.cancellation.cancel();
                    Err(deadline_exceeded())
                }
            }
        };
        admitted.finish(result).await
    }

    async fn dispatch_terminal(
        &self,
        ttrpc_context: &ttrpc::r#async::TtrpcContext,
        method: DaemonMethod,
        request: terminal::TerminalOpenRequest,
    ) -> ttrpc::Result<terminal::TerminalOpenResponse> {
        let _permit = Arc::clone(&self.in_flight)
            .try_acquire_owned()
            .map_err(|_| resource_exhausted())?;
        let admitted = self
            .admit(ttrpc_context, method, request.metadata.as_ref())
            .await?;
        request.validate_wire(true).map_err(|_| invalid_request())?;
        let generation = self.terminals.generation();
        let prepared = tokio::select! {
            biased;
            () = admitted.context.cancellation.cancelled() => {
                return admitted.finish(Err(cancelled())).await;
            }
            result = tokio::time::timeout(
                admitted.context.remaining,
                self.handler.prepare_terminal(method, &request, &admitted.context),
            ) => match result {
                Ok(Ok(prepared)) => prepared,
                Ok(Err(failure)) => {
                    let response = terminal_open_failure_response(&request, generation, failure);
                    validate_terminal_open_response_for_request(&request, &response)
                        .map_err(|_| response_error())?;
                    return admitted.finish(Ok(response)).await;
                }
                Err(_) => {
                    let _ = admitted.context.cancellation.cancel();
                    return admitted.finish(Err(deadline_exceeded())).await;
                }
            }
        };
        let metadata = request.metadata.as_ref().ok_or_else(invalid_request)?;
        let request_id: [u8; 16] = metadata
            .request_id
            .as_slice()
            .try_into()
            .map_err(|_| invalid_request())?;
        let resource_handle = new_terminal_resource_handle().map_err(|_| response_error())?;
        let binding = TerminalBinding {
            session_generation: generation,
            request_id,
            operation_id: request.operation_id.clone(),
            resource_handle: resource_handle.clone(),
            peer_principal: format!(
                "local-{}-{}",
                self.peer_uid,
                peer_role_label(self.peer_role)
            ),
            peer_uid: self.peer_uid,
            kind: terminal_kind(method).ok_or_else(invalid_request)?,
            retained_log: None,
        };
        let response = match self
            .terminals
            .reserve(binding, prepared, admitted.context.cancellation.clone())
            .await
        {
            Ok(stream_id) => {
                terminal_open_success_response(&request, generation, stream_id, resource_handle)
                    .map_err(|_| response_error())?
            }
            Err(failure) => terminal_open_failure_response(&request, generation, failure),
        };
        validate_terminal_open_response_for_request(&request, &response)
            .map_err(|_| response_error())?;
        admitted.finish(Ok(response)).await
    }

    pub(crate) async fn admit(
        &self,
        ttrpc_context: &ttrpc::r#async::TtrpcContext,
        method: DaemonMethod,
        metadata: Option<&common::RequestMetadata>,
    ) -> ttrpc::Result<AdmittedCall> {
        if !self.peer_role.permits(method) {
            return Err(permission_denied());
        }
        let metadata = metadata.ok_or_else(invalid_request)?;
        if metadata.session_generation != self.session.generation() {
            return Err(permission_denied());
        }
        let remaining_nanos = admit_metadata(
            metadata,
            method.mutating(),
            now_unix_ms(),
            MAX_DAEMON_REQUEST_LIFETIME_MS,
            None,
            peer_timeout(ttrpc_context),
        )
        .map_err(|_| invalid_request())?;
        let request_id =
            RequestId::new(metadata.request_id.clone()).map_err(|_| invalid_request())?;
        let cancellation = self
            .session
            .register_inbound_call(request_id.clone())
            .await
            .map_err(|_| invalid_request())?;
        self.active
            .lock()
            .map_err(|_| response_error())?
            .insert(metadata.request_id.clone(), cancellation.clone());
        Ok(AdmittedCall {
            driver: Arc::clone(&self.session),
            request_id: Some(request_id.clone()),
            request_key: metadata.request_id.clone(),
            active: Arc::clone(&self.active),
            context: DaemonCallContext {
                peer_role: self.peer_role,
                peer_uid: self.peer_uid,
                request_id,
                session_generation: self.session.generation(),
                remaining: Duration::from_nanos(remaining_nanos),
                cancellation,
            },
        })
    }
}

#[async_trait]
impl<H: DaemonOperationHandler + 'static> DaemonService for DaemonServiceV2<H> {
    async fn resolve(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, DaemonMethod::Resolve, request).await
    }

    async fn list_realms(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<daemon::ListRealmsResponse> {
        self.dispatch_typed(
            context,
            DaemonMethod::ListRealms,
            request,
            |handler, request, context| async move { handler.list_realms(request, &context).await },
        )
        .await
    }

    async fn list_workloads(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<daemon::ListWorkloadsResponse> {
        self.dispatch_typed(
            context,
            DaemonMethod::ListWorkloads,
            request,
            |handler, request, context| async move {
                handler.list_workloads(request, &context).await
            },
        )
        .await
    }

    async fn inspect(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<daemon::InspectResponse> {
        self.dispatch_typed(
            context,
            DaemonMethod::Inspect,
            request,
            |handler, request, context| async move { handler.inspect(request, &context).await },
        )
        .await
    }

    async fn apply(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, DaemonMethod::Apply, request).await
    }

    async fn start(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, DaemonMethod::Start, request).await
    }

    async fn stop(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, DaemonMethod::Stop, request).await
    }

    async fn restart(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, DaemonMethod::Restart, request).await
    }

    async fn exec(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: terminal::TerminalOpenRequest,
    ) -> ttrpc::Result<terminal::TerminalOpenResponse> {
        self.dispatch_terminal(context, DaemonMethod::Exec, request)
            .await
    }

    async fn shell(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: terminal::TerminalOpenRequest,
    ) -> ttrpc::Result<terminal::TerminalOpenResponse> {
        self.dispatch_terminal(context, DaemonMethod::Shell, request)
            .await
    }

    async fn open_console(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: terminal::TerminalOpenRequest,
    ) -> ttrpc::Result<terminal::TerminalOpenResponse> {
        self.dispatch_terminal(context, DaemonMethod::OpenConsole, request)
            .await
    }

    async fn export_audit(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(context, DaemonMethod::ExportAudit, request)
            .await
    }

    async fn cancel(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: common::CancelRequest,
    ) -> ttrpc::Result<common::CancelResponse> {
        self.cancel_request(&request)
    }
}

pub async fn serve_daemon_session<H>(
    driver: Arc<dyn ComponentSessionDriver>,
    peer_role: DaemonPeerRole,
    peer_uid: u32,
    handler: Arc<H>,
    guest_connector: Arc<dyn crate::guest_terminal::GuestTerminalConnector>,
) -> Result<(), DaemonServiceFailure>
where
    H: DaemonOperationHandler + 'static,
{
    let service = Arc::new(DaemonServiceV2::new(
        handler,
        Arc::clone(&driver),
        peer_role,
        peer_uid,
    ));
    let guest_proxy = guest_proxy::DaemonGuestProxy::new(Arc::clone(&service), guest_connector);
    let terminals = Arc::clone(&service.terminals);
    let (server_transport, bridge_transport) =
        tokio::io::duplex(LimitProfile::local_default().logical_ttrpc_bytes as usize);
    let listener = ttrpc::r#async::transport::Listener::new(stream::once(async move {
        Ok::<_, std::io::Error>(server_transport)
    }));
    let mut server = ttrpc::r#async::Server::new()
        .add_listener(listener)
        .register_service(create_daemon_service(service))
        .register_service(create_guest_service(guest_proxy));
    server
        .start()
        .await
        .map_err(|_| DaemonServiceFailure::Backend)?;
    let (mut reader, mut writer) = tokio::io::split(bridge_transport);
    let receive_driver = Arc::clone(&driver);
    let receive = async move {
        loop {
            let frame = receive_driver
                .receive_ttrpc()
                .await
                .map_err(|_| DaemonServiceFailure::Backend)?;
            writer
                .write_all(&frame)
                .await
                .map_err(|_| DaemonServiceFailure::Backend)?;
            writer
                .flush()
                .await
                .map_err(|_| DaemonServiceFailure::Backend)?;
        }
    };
    let send = async move {
        let logical_limit = LimitProfile::local_default().logical_ttrpc_bytes;
        loop {
            let next = crate::ttrpc_frame::read_ttrpc_frame(&mut reader, logical_limit)
                .await
                .map_err(|_| DaemonServiceFailure::Backend)?;
            let Some((_, frame)) = next else {
                return Ok::<(), DaemonServiceFailure>(());
            };
            driver
                .send_ttrpc(frame)
                .await
                .map_err(|_| DaemonServiceFailure::Backend)?;
        }
    };
    let terminal_router = Arc::clone(&terminals).run_router();
    let result = tokio::select! {
        result = receive => result,
        result = send => result,
        result = terminal_router => result.map_err(|_| DaemonServiceFailure::Backend),
    };
    terminals.shutdown().await;
    server.disconnect().await;
    result
}

pub(crate) async fn serve_accepted_daemon_socket(
    fd: OwnedFd,
    peer: PeerIdentity,
    generation: u64,
    state: Arc<ServerState>,
) -> Result<(), DaemonServiceFailure> {
    let socket = SeqpacketSocket::from_owned(fd).map_err(|_| DaemonServiceFailure::Backend)?;
    let credentials = socket
        .acceptor_peer_credentials()
        .map_err(|_| DaemonServiceFailure::PermissionDenied)?;
    if credentials.uid().as_raw() != peer.uid {
        return Err(DaemonServiceFailure::PermissionDenied);
    }
    let policy = daemon_endpoint_policy(
        generation,
        daemon_channel_binding(credentials.uid().as_raw(), credentials.gid().as_raw()),
    )?;
    let transport = DaemonSeqpacketTransport::new(socket, Locality::HostLocal, policy.limits)?;
    let engine = SessionEngine::establish_responder(
        transport,
        policy,
        HandshakeCredentials::Nn,
        Instant::now(),
    )
    .await
    .map_err(|_| DaemonServiceFailure::PermissionDenied)?;
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
    let role = DaemonPeerRole::from(peer.role);
    let uid = peer.uid;
    let guest_connector = Arc::clone(&state.guest_terminal_connector);
    let handler = Arc::new(ProductionDaemonOperationHandler::new(state, peer));
    serve_daemon_session(driver, role, uid, handler, guest_connector).await
}

pub fn daemon_endpoint_policy(
    generation: u64,
    channel_binding: [u8; 32],
) -> Result<EndpointPolicy, DaemonServiceFailure> {
    Ok(EndpointPolicy {
        purpose: EndpointPurpose::DaemonLocal,
        purpose_class: PurposeClass::Local,
        initiator_role: EndpointRole::CommandClient,
        responder_role: EndpointRole::LocalRootController,
        service: ServicePackage::DaemonV2,
        schema_fingerprint: public_daemon_schema_fingerprint(),
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            channel_binding,
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: generation,
        attachment_policy: AttachmentPolicy {
            kind: AttachmentPolicyKind::Disabled,
            max_per_packet: 0,
            max_per_request: 0,
            max_per_operation: 0,
            max_per_session: 0,
            credentials_allowed: false,
        },
    })
}

pub fn daemon_channel_binding(uid: u32, gid: u32) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b.daemon.v2\0unix-seqpacket\0");
    digest.update(uid.to_be_bytes());
    digest.update(gid.to_be_bytes());
    digest.finalize().into()
}

fn daemon_credit_scopes() -> CreditScopeSet {
    let limit = 1;
    CreditScopeSet::new(
        CreditPool::new(limit).expect("positive daemon packet credit"),
        CreditPool::new(limit).expect("positive daemon request credit"),
        CreditPool::new(limit).expect("positive daemon operation credit"),
        CreditPool::new(limit).expect("positive daemon session credit"),
        CreditPool::new(limit).expect("positive daemon process credit"),
        CreditPool::new(limit).expect("positive daemon host credit"),
    )
}

pub struct DaemonSeqpacketTransport {
    socket: SeqpacketSocket,
    locality: Locality,
    limits: LimitProfile,
    ancillary: AncillaryCapacity,
    credits: CreditScopeSet,
    received: VecDeque<Vec<u8>>,
    closed: bool,
}

impl DaemonSeqpacketTransport {
    pub fn new(
        socket: SeqpacketSocket,
        locality: Locality,
        limits: LimitProfile,
    ) -> Result<Self, DaemonServiceFailure> {
        let ancillary = AncillaryCapacity::from_policy(AttachmentPolicy {
            kind: AttachmentPolicyKind::PacketAtomic,
            max_per_packet: 1,
            max_per_request: 1,
            max_per_operation: 1,
            max_per_session: 1,
            credentials_allowed: false,
        })
        .map_err(|_| DaemonServiceFailure::Backend)?;
        Ok(Self {
            socket,
            locality,
            limits,
            ancillary,
            credits: daemon_credit_scopes(),
            received: VecDeque::new(),
            closed: false,
        })
    }
}

impl fmt::Debug for DaemonSeqpacketTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonSeqpacketTransport")
            .field("locality", &self.locality)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl OwnedTransport for DaemonSeqpacketTransport {
    fn descriptor(&self) -> TransportDescriptor {
        TransportDescriptor {
            class: TransportClass::UnixSeqpacket,
            locality: self.locality,
            packet_atomic: true,
            supports_attachments: false,
        }
    }

    async fn receive(&mut self, protected_limit: usize) -> Result<TransportPacket, TransportError> {
        if self.closed {
            return Err(TransportError::Disconnected);
        }
        if let Some(bytes) = self.received.pop_front() {
            return Ok(TransportPacket::new(bytes));
        }
        let burst = self
            .socket
            .recv_burst(self.limits, self.ancillary, &self.credits, 8)
            .await
            .map_err(map_unix_transport_error)?;
        for packet in burst.packets {
            if packet.control_count() != 0 {
                return Err(TransportError::InvalidAttachment);
            }
            let bytes = packet.payload().to_vec();
            if bytes.is_empty() || bytes.len() > protected_limit {
                return Err(TransportError::LimitExceeded);
            }
            self.received.push_back(bytes);
        }
        self.received
            .pop_front()
            .map(TransportPacket::new)
            .ok_or(TransportError::WouldBlock)
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Disconnected);
        }
        let (bytes, attachments) = packet.into_parts();
        if !attachments.is_empty() {
            return Err(TransportError::InvalidAttachment);
        }
        let outbound = OutboundPacket::new(
            bytes,
            Vec::new(),
            None,
            self.limits,
            self.ancillary,
            &self.credits,
        )
        .map_err(map_unix_transport_error)?;
        let mut queue = VecDeque::from([outbound]);
        while !queue.is_empty() {
            self.socket
                .send_burst(&mut queue, self.ancillary, 1)
                .await
                .map_err(map_unix_transport_error)?;
        }
        Ok(())
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.closed = true;
        self.socket.close().map_err(map_unix_transport_error)
    }
}

fn map_unix_transport_error(error: UnixSessionError) -> TransportError {
    match error {
        UnixSessionError::Closed => TransportError::Disconnected,
        UnixSessionError::MessageTruncated | UnixSessionError::ControlTruncated => {
            TransportError::Truncated
        }
        UnixSessionError::PayloadLimit
        | UnixSessionError::AncillaryCapacity
        | UnixSessionError::CreditExceeded => TransportError::LimitExceeded,
        UnixSessionError::UnknownControl
        | UnixSessionError::ControlMismatch
        | UnixSessionError::CredentialMismatch
        | UnixSessionError::DescriptorMismatch
        | UnixSessionError::DuplicateObject
        | UnixSessionError::MissingCloexec
        | UnixSessionError::PidfdEvidenceUnavailable
        | UnixSessionError::PidfdIdentityMismatch => TransportError::InvalidAttachment,
        _ => TransportError::Other,
    }
}

pub(crate) struct AdmittedCall {
    driver: Arc<dyn ComponentSessionDriver>,
    request_id: Option<RequestId>,
    request_key: Vec<u8>,
    active: Arc<Mutex<BTreeMap<Vec<u8>, Cancellation>>>,
    pub(crate) context: DaemonCallContext,
}

impl AdmittedCall {
    pub(crate) async fn finish<T>(mut self, result: ttrpc::Result<T>) -> ttrpc::Result<T> {
        self.active
            .lock()
            .map_err(|_| response_error())?
            .remove(&self.request_key);
        let request_id = self.request_id.take().ok_or_else(response_error)?;
        let completed = if result.is_ok() {
            self.driver.complete_inbound_call(request_id).await
        } else {
            self.driver.remove_inbound_call(request_id).await
        }
        .map_err(|_| response_error())?;
        if !completed {
            return Err(response_error());
        }
        result
    }
}

impl Drop for AdmittedCall {
    fn drop(&mut self) {
        let Some(request_id) = self.request_id.take() else {
            return;
        };
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.request_key);
        }
        let driver = Arc::clone(&self.driver);
        tokio::spawn(async move {
            let _ = driver.remove_inbound_call(request_id).await;
        });
    }
}

fn peer_timeout(context: &ttrpc::r#async::TtrpcContext) -> Option<u64> {
    u64::try_from(context.timeout_nano)
        .ok()
        .filter(|timeout| *timeout != 0)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn rpc_error(code: ttrpc::Code, message: &'static str) -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(code, message.to_owned()))
}

fn invalid_request() -> ttrpc::Error {
    rpc_error(ttrpc::Code::INVALID_ARGUMENT, "daemon-request-invalid")
}

fn permission_denied() -> ttrpc::Error {
    rpc_error(ttrpc::Code::PERMISSION_DENIED, "daemon-admission-denied")
}

fn cancelled() -> ttrpc::Error {
    rpc_error(ttrpc::Code::CANCELLED, "daemon-request-cancelled")
}

fn deadline_exceeded() -> ttrpc::Error {
    rpc_error(ttrpc::Code::DEADLINE_EXCEEDED, "daemon-deadline-exceeded")
}

fn resource_exhausted() -> ttrpc::Error {
    rpc_error(ttrpc::Code::RESOURCE_EXHAUSTED, "daemon-resource-exhausted")
}

fn response_error() -> ttrpc::Error {
    rpc_error(ttrpc::Code::INTERNAL, "daemon-response-contract-invalid")
}

fn service_error(error: DaemonServiceFailure) -> ttrpc::Error {
    match error {
        DaemonServiceFailure::InvalidRequest => invalid_request(),
        DaemonServiceFailure::PermissionDenied => permission_denied(),
        DaemonServiceFailure::DeadlineExceeded => deadline_exceeded(),
        DaemonServiceFailure::Cancelled => cancelled(),
        DaemonServiceFailure::ResourceExhausted => resource_exhausted(),
        DaemonServiceFailure::Backend => {
            rpc_error(ttrpc::Code::FAILED_PRECONDITION, "daemon-operation-failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v2_services::daemon::WorkloadLifecycleState;
    use d2b_core::{
        realm_workloads_launcher::LauncherWorkloadSummary,
        workload_identity::{WorkloadIdentity, WorkloadTarget},
    };
    use d2b_realm_core::{
        CapabilitySet, DisplayEnvironmentPosture, EnvironmentPosture, ExecutionIdentityPosture,
        IsolationPosture, LauncherIcon, SessionPersistencePosture, WorkloadExecutionPosture,
        WorkloadProviderKind,
        ids::{RealmId, WorkloadId},
        realm::RealmPath,
    };

    fn public_services() -> public_wire::PublicVmServices {
        public_wire::PublicVmServices {
            gpu: None,
            microvm: "active".to_owned(),
            d2b: "active".to_owned(),
            qemu_media: None,
            snd: None,
            swtpm: None,
            video: None,
            virtiofsd: "active".to_owned(),
        }
    }

    fn public_lifecycle() -> public_wire::VmLifecycle {
        public_wire::VmLifecycle {
            degraded: false,
            degraded_reasons: Vec::new(),
            pending_restart: true,
            state: public_wire::VmLifecycleState::Running,
        }
    }

    fn public_runtime() -> public_wire::RuntimeSummary {
        public_wire::RuntimeSummary {
            detail: "running".to_owned(),
            kind: Some("nixos".to_owned()),
            operation_capabilities: Default::default(),
            services: Vec::new(),
        }
    }

    #[test]
    fn nonempty_list_projection_preserves_operator_visible_state_without_paths() {
        let entry = public_wire::ListEntry {
            env: Some("work".to_owned()),
            graphics: true,
            is_net_vm: false,
            lifecycle: public_lifecycle(),
            name: "corp-vm".to_owned(),
            guest_closure_out_path: Some("/nix/store/sensitive-closure".to_owned()),
            autostart: None,
            qemu_media: None,
            runtime: public_runtime(),
            runtime_capabilities: vec!["lifecycle".to_owned(), "exec".to_owned()],
            services: public_services(),
            service_capabilities: Vec::new(),
            ssh_user: Some("alice".to_owned()),
            static_ip: Some("10.40.0.2".to_owned()),
            tpm: true,
            unsupported_capabilities: vec!["display".to_owned()],
            usbip: false,
            vm: "corp-vm".to_owned(),
            workload_identity: None,
        };
        let projected = project_list_entry(&entry, 9).expect("project list entry");
        assert_eq!(projected.name, "corp-vm");
        assert_eq!(projected.environment, "work");
        assert_eq!(projected.static_ip, [10, 40, 0, 2]);
        assert!(projected.graphics);
        assert!(projected.ssh_configured);
        assert_eq!(
            projected
                .lifecycle
                .as_ref()
                .unwrap()
                .state
                .enum_value()
                .unwrap(),
            WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_RUNNING
        );
        assert!(projected.deployment.is_none());
        let rendered = format!("{projected:?}");
        assert!(!rendered.contains("/nix/store"));
        assert_projection_valid(projected);
    }

    #[test]
    fn nonempty_inspect_projection_preserves_bridge_and_runtime_state() {
        let entry = public_wire::VmStatus {
            bridge_checks: vec![public_wire::BridgeCheck {
                bridge: d2b_core::host::IfName::new("d2b-work").unwrap(),
                present: true,
                tap: Some(d2b_core::host::IfName::new("d2bv-corp").unwrap()),
            }],
            env: Some("work".to_owned()),
            graphics: false,
            is_net_vm: false,
            lifecycle: public_lifecycle(),
            name: "corp-vm".to_owned(),
            autostart: None,
            qemu_media: None,
            runtime: public_runtime(),
            runtime_capabilities: vec!["guest-control".to_owned()],
            services: public_services(),
            service_capabilities: Vec::new(),
            ssh_user: None,
            static_ip: None,
            tpm: false,
            unsupported_capabilities: Vec::new(),
            usbip: false,
            usb: None,
            vm: "corp-vm".to_owned(),
            workload_identity: None,
        };
        let projected = project_status_entry(&entry, 12).expect("project status entry");
        assert_eq!(projected.bridge_checks.len(), 1);
        assert_eq!(projected.bridge_checks[0].bridge, "d2b-work");
        assert_eq!(projected.lifecycle.as_ref().unwrap().generation, 12);
        assert_projection_valid(projected);
    }

    #[test]
    fn realm_native_workload_without_legacy_vm_is_projected() {
        let realm_id = RealmId::parse("work").unwrap();
        let identity = WorkloadIdentity::new(
            WorkloadId::parse("browser").unwrap(),
            realm_id.clone(),
            RealmPath::new(vec![realm_id]).unwrap(),
            WorkloadTarget::parse("browser.work.d2b").unwrap(),
        );
        assert!(identity.legacy_vm_name.is_none());
        let entry = crate::workload_dispatch::CatalogEntry {
            metadata: LauncherWorkloadSummary {
                identity,
                provider_kind: WorkloadProviderKind::UnsafeLocal,
                execution_posture: WorkloadExecutionPosture {
                    isolation: IsolationPosture::UnsafeLocal,
                    environment: EnvironmentPosture::SystemdUserManagerAmbient,
                    display_environment: DisplayEnvironmentPosture::WaylandProxyOnly,
                    execution_identity: ExecutionIdentityPosture::AuthenticatedRequesterUid,
                    session_persistence: SessionPersistencePosture::UserManagerLifetime,
                },
                label: "Browser".to_owned(),
                icon: LauncherIcon::default(),
                realm_accent_color: "#336699".to_owned(),
                launcher_enabled: true,
                default_item_id: None,
                capabilities: CapabilitySet::default(),
                items: Vec::new(),
            },
            route: crate::workload_dispatch::WorkloadRoute::UnsafeLocal,
        };
        let projected = project_catalog_workload(
            &entry,
            None,
            d2b_realm_core::WorkloadState::Running,
            public_wire::WorkloadAvailability::Ready,
            11,
            true,
            Some("work"),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(projected.name, "browser");
        assert_eq!(projected.environment, "work");
        assert_eq!(
            projected
                .runtime
                .as_ref()
                .unwrap()
                .kind
                .enum_value()
                .unwrap(),
            daemon::RuntimeKind::RUNTIME_KIND_UNSAFE_LOCAL
        );
        assert_eq!(
            projected.identity.as_ref().unwrap().canonical_target,
            "browser.work.local-root.d2b"
        );
        assert_projection_valid(projected);
    }

    #[test]
    fn pagination_is_explicit_stable_and_rejects_foreign_cursors() {
        let mut request = ServiceRequest {
            page_size: 2,
            ..Default::default()
        };
        let (first, page) = paginate(vec!["a", "b", "c"], &request, "w", 256).expect("first page");
        assert_eq!(first, ["a", "b"]);
        assert!(page.truncated);
        assert_eq!(page.next_page_cursor, "w-2");
        assert_eq!(page.total_items, 3);

        request.page_cursor = page.next_page_cursor;
        let (second, page) =
            paginate(vec!["a", "b", "c"], &request, "w", 256).expect("second page");
        assert_eq!(second, ["c"]);
        assert!(!page.truncated);
        assert!(page.next_page_cursor.is_empty());

        request.page_cursor = "r-1".to_owned();
        assert_eq!(
            paginate(vec!["a"], &request, "w", 256).unwrap_err(),
            DaemonServiceFailure::InvalidRequest
        );
    }

    #[test]
    fn realm_pagination_is_clamped_to_endpoint_cap() {
        let request = ServiceRequest {
            page_size: d2b_contracts::v2_services::MAX_PAGE_SIZE,
            ..Default::default()
        };
        let (realms, page) = paginate(
            (0_u32..100).collect::<Vec<_>>(),
            &request,
            "r",
            d2b_contracts::v2_services::MAX_DAEMON_REALMS,
        )
        .unwrap();
        assert_eq!(realms.len(), d2b_contracts::v2_services::MAX_DAEMON_REALMS);
        assert!(page.truncated);
        assert_eq!(page.next_page_cursor, "r-64");
    }

    fn assert_projection_valid(projected: daemon::WorkloadProjection) {
        let response = daemon::ListWorkloadsResponse {
            outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED),
            workloads: vec![projected],
            page: MessageField::some(daemon::PageInfo {
                returned_items: 1,
                total_items_known: true,
                total_items: 1,
                ..Default::default()
            }),
            ..Default::default()
        };
        response.validate_wire(false).expect("strict projection");
    }
}
