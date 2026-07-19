//! Authenticated `d2b.realm.v2` service over an established ComponentSession.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        EndpointRole, Locality, MAX_LOGICAL_MESSAGE_BYTES, MAX_REQUEST_LIFETIME_MS, PurposeClass,
        RequestId, SessionErrorCode,
    },
    v2_identity::RealmId,
    v2_services::{
        StrictWireMessage, admit_metadata,
        common::{self, CancelOutcome, ObservationState, Outcome},
        realm_ttrpc::{self, RealmService},
    },
};
use d2b_session::{Cancellation, ComponentSessionDriver, SessionError};
use protobuf::{EnumOrUnknown, Message, MessageField};
use sha2::{Digest, Sha256};
use tokio::{sync::Semaphore, task::JoinSet};
use ttrpc::{
    r#async::{Service, TtrpcContext},
    context,
    proto::{MESSAGE_HEADER_LENGTH, MESSAGE_TYPE_REQUEST, MessageHeader},
};

pub const REALM_SERVICE_NAME: &str = "d2b.realm.v2.RealmService";
pub const DEFAULT_MAX_REALM_BINDINGS: usize = 256;
pub const DEFAULT_MAX_SHORTCUTS: usize = 256;
pub const DEFAULT_MAX_MUTATION_RECORDS: usize = 1_024;
pub const DEFAULT_AUDIT_CAPACITY: usize = 1_024;

const MAX_CONFIGURED_BOUND: usize = 4_096;
const MAX_DISPATCH_IN_FLIGHT: usize = 64;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialCustody {
    /// Host-local sessions retain only public identity pins and digests.
    None,
    /// Realm credentials are held by the gateway guest that terminated the
    /// authenticated session. No credential bytes enter this service.
    GatewayGuest,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RealmSessionAuthority {
    realm: RealmId,
    peer_role: EndpointRole,
    locality: Locality,
    purpose: PurposeClass,
    custody: CredentialCustody,
}

impl RealmSessionAuthority {
    pub fn local_controller(
        realm: RealmId,
        peer_role: EndpointRole,
    ) -> Result<Self, RealmServiceError> {
        Self::new(
            realm,
            peer_role,
            Locality::HostLocal,
            PurposeClass::Local,
            CredentialCustody::None,
        )
    }

    pub fn gateway_peer(
        realm: RealmId,
        peer_role: EndpointRole,
        purpose: PurposeClass,
    ) -> Result<Self, RealmServiceError> {
        Self::new(
            realm,
            peer_role,
            Locality::Remote,
            purpose,
            CredentialCustody::GatewayGuest,
        )
    }

    pub fn new(
        realm: RealmId,
        peer_role: EndpointRole,
        locality: Locality,
        purpose: PurposeClass,
        custody: CredentialCustody,
    ) -> Result<Self, RealmServiceError> {
        let role_allowed = matches!(
            peer_role,
            EndpointRole::LocalRootController
                | EndpointRole::RealmController
                | EndpointRole::RemotePeer
        );
        let remote_is_gateway = locality == Locality::Remote
            && custody == CredentialCustody::GatewayGuest
            && matches!(
                peer_role,
                EndpointRole::RealmController | EndpointRole::RemotePeer
            )
            && !matches!(purpose, PurposeClass::Local);
        let local_is_credential_free = locality != Locality::Remote
            && custody == CredentialCustody::None
            && peer_role != EndpointRole::RemotePeer
            && purpose == PurposeClass::Local;
        if !role_allowed || (!remote_is_gateway && !local_is_credential_free) {
            return Err(RealmServiceError::InvalidAuthority);
        }
        Ok(Self {
            realm,
            peer_role,
            locality,
            purpose,
            custody,
        })
    }

    pub fn realm(&self) -> &RealmId {
        &self.realm
    }

    pub fn peer_role(&self) -> EndpointRole {
        self.peer_role
    }

    pub fn locality(&self) -> Locality {
        self.locality
    }

    pub fn purpose(&self) -> PurposeClass {
        self.purpose
    }

    pub fn credential_custody(&self) -> CredentialCustody {
        self.custody
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealmServiceLimits {
    pub max_bindings: usize,
    pub max_shortcuts: usize,
    pub max_mutation_records: usize,
    pub audit_capacity: usize,
}

impl Default for RealmServiceLimits {
    fn default() -> Self {
        Self {
            max_bindings: DEFAULT_MAX_REALM_BINDINGS,
            max_shortcuts: DEFAULT_MAX_SHORTCUTS,
            max_mutation_records: DEFAULT_MAX_MUTATION_RECORDS,
            audit_capacity: DEFAULT_AUDIT_CAPACITY,
        }
    }
}

impl RealmServiceLimits {
    fn validate(self) -> Result<Self, RealmServiceError> {
        if [
            self.max_bindings,
            self.max_shortcuts,
            self.max_mutation_records,
            self.audit_capacity,
        ]
        .into_iter()
        .any(|bound| bound == 0 || bound > MAX_CONFIGURED_BOUND)
        {
            return Err(RealmServiceError::InvalidLimits);
        }
        Ok(self)
    }
}

pub trait RealmClock: Send + Sync {
    fn now_unix_ms(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemRealmClock;

impl RealmClock for SystemRealmClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealmServiceError {
    InvalidAuthority,
    InvalidGeneration,
    InvalidLimits,
    SessionClosed,
    ProtocolViolation,
}

impl fmt::Display for RealmServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidAuthority => "realm session authority is invalid",
            Self::InvalidGeneration => "realm session generation is invalid",
            Self::InvalidLimits => "realm service bounds are invalid",
            Self::SessionClosed => "realm component session closed",
            Self::ProtocolViolation => "realm component session protocol violation",
        })
    }
}

impl std::error::Error for RealmServiceError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealmMethod {
    Bootstrap,
    Enroll,
    ResolveRoute,
    AuthorizeShortcut,
    RevokeShortcut,
    ReportShortcutClose,
    Inspect,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealmAuditOutcome {
    Completed,
    Denied,
    Cancelled,
    Overloaded,
    ProtocolViolation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealmAuditEvent {
    pub method: RealmMethod,
    pub outcome: RealmAuditOutcome,
}

#[derive(Debug)]
struct BoundedAudit {
    capacity: usize,
    events: Mutex<VecDeque<RealmAuditEvent>>,
}

impl BoundedAudit {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            events: Mutex::new(VecDeque::with_capacity(capacity)),
        }
    }

    fn record(&self, event: RealmAuditEvent) {
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if events.len() == self.capacity {
            events.pop_front();
        }
        events.push_back(event);
    }

    fn snapshot(&self) -> Vec<RealmAuditEvent> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .copied()
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BootstrapBinding {
    envelope_digest: [u8; 32],
    operation_id: String,
    controller_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EnrollmentBinding {
    bootstrap_digest: [u8; 32],
    enrollment_digest: [u8; 32],
    operation_id: String,
    controller_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortcutState {
    Authorized,
    Revoked,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShortcutBinding {
    route_handle: String,
    route_digest: [u8; 32],
    operation_id: String,
    controller_generation: u64,
    policy_epoch: u64,
    expires_at_unix_ms: u64,
    state: ShortcutState,
}

#[derive(Debug, Clone, PartialEq)]
struct MutationRecord {
    fingerprint: [u8; 32],
    response: common::ServiceResponse,
}

#[derive(Debug, Default)]
struct RealmState {
    bootstraps: BTreeMap<String, BootstrapBinding>,
    enrollments: BTreeMap<String, EnrollmentBinding>,
    shortcuts: BTreeMap<String, ShortcutBinding>,
    mutations: BTreeMap<Vec<u8>, MutationRecord>,
}

pub struct RealmServiceServer {
    authority: RealmSessionAuthority,
    driver: Arc<dyn ComponentSessionDriver>,
    clock: Arc<dyn RealmClock>,
    limits: RealmServiceLimits,
    policy_epoch: u64,
    state: Mutex<RealmState>,
    active_calls: Mutex<HashMap<Vec<u8>, Cancellation>>,
    audit: BoundedAudit,
}

impl fmt::Debug for RealmServiceServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RealmServiceServer")
            .field("realm", &"<bound>")
            .field("peer_role", &self.authority.peer_role)
            .finish_non_exhaustive()
    }
}

impl RealmServiceServer {
    pub fn new(
        authority: RealmSessionAuthority,
        driver: Arc<dyn ComponentSessionDriver>,
        policy_epoch: u64,
    ) -> Result<Arc<Self>, RealmServiceError> {
        Self::new_with(
            authority,
            driver,
            policy_epoch,
            RealmServiceLimits::default(),
            Arc::new(SystemRealmClock),
        )
    }

    pub fn new_with(
        authority: RealmSessionAuthority,
        driver: Arc<dyn ComponentSessionDriver>,
        policy_epoch: u64,
        limits: RealmServiceLimits,
        clock: Arc<dyn RealmClock>,
    ) -> Result<Arc<Self>, RealmServiceError> {
        if driver.generation() == 0 || policy_epoch == 0 {
            return Err(RealmServiceError::InvalidGeneration);
        }
        let limits = limits.validate()?;
        Ok(Arc::new(Self {
            authority,
            driver,
            clock,
            limits,
            policy_epoch,
            state: Mutex::new(RealmState::default()),
            active_calls: Mutex::new(HashMap::new()),
            audit: BoundedAudit::new(limits.audit_capacity),
        }))
    }

    pub fn generated_services(self: &Arc<Self>) -> HashMap<String, Service> {
        realm_ttrpc::create_realm_service(self.clone())
    }

    pub fn audit_snapshot(&self) -> Vec<RealmAuditEvent> {
        self.audit.snapshot()
    }

    pub fn binding_count(&self) -> usize {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.bootstraps.len() + state.enrollments.len()
    }

    pub fn shortcut_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .shortcuts
            .len()
    }

    async fn call(
        &self,
        method: RealmMethod,
        request: common::ServiceRequest,
        timeout_nanos: Option<u64>,
    ) -> ttrpc::Result<common::ServiceResponse> {
        let mutating = matches!(
            method,
            RealmMethod::Bootstrap
                | RealmMethod::Enroll
                | RealmMethod::AuthorizeShortcut
                | RealmMethod::RevokeShortcut
                | RealmMethod::ReportShortcutClose
        );
        request
            .validate_wire(mutating)
            .map_err(|error| rpc_error(ttrpc::Code::INVALID_ARGUMENT, error.to_string()))?;
        let metadata = request
            .metadata
            .as_ref()
            .ok_or_else(|| rpc_error(ttrpc::Code::INVALID_ARGUMENT, "realm-metadata-missing"))?;
        let scope = request
            .scope
            .as_ref()
            .ok_or_else(|| rpc_error(ttrpc::Code::INVALID_ARGUMENT, "realm-scope-missing"))?;
        if scope.realm_id != self.authority.realm.as_str()
            || !scope.workload_id.is_empty()
            || !scope.provider_id.is_empty()
            || !scope.role_id.is_empty()
            || metadata.session_generation != self.driver.generation()
        {
            self.record(method, RealmAuditOutcome::Denied);
            return Err(rpc_error(
                ttrpc::Code::PERMISSION_DENIED,
                "realm-session-authority-mismatch",
            ));
        }
        admit_metadata(
            metadata,
            mutating,
            self.clock.now_unix_ms(),
            MAX_REQUEST_LIFETIME_MS,
            None,
            timeout_nanos,
        )
        .map_err(|error| rpc_error(ttrpc::Code::DEADLINE_EXCEEDED, error.to_string()))?;
        if !request.attachment_indexes.is_empty() {
            return Err(rpc_error(
                ttrpc::Code::INVALID_ARGUMENT,
                "realm-attachments-disabled",
            ));
        }
        self.authorize_method(method)?;
        let request_id = RequestId::new(metadata.request_id.clone())
            .map_err(|_| rpc_error(ttrpc::Code::INVALID_ARGUMENT, "realm-request-id-invalid"))?;
        let inbound = InboundCall::register(self.driver.clone(), request_id).await?;
        self.active_calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(metadata.request_id.clone(), inbound.cancellation().clone());
        let result = tokio::select! {
            biased;
            () = inbound.cancellation().cancelled() => {
                Err(rpc_error(ttrpc::Code::CANCELLED, "realm-request-cancelled"))
            }
            result = async { self.dispatch(method, &request) } => result,
        };
        self.active_calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&metadata.request_id);
        let outcome = match &result {
            Ok(_) => RealmAuditOutcome::Completed,
            Err(ttrpc::Error::RpcStatus(status))
                if status.code.enum_value().ok() == Some(ttrpc::Code::CANCELLED) =>
            {
                RealmAuditOutcome::Cancelled
            }
            Err(_) => RealmAuditOutcome::Denied,
        };
        self.record(method, outcome);
        inbound.finish(result).await
    }

    fn authorize_method(&self, method: RealmMethod) -> ttrpc::Result<()> {
        let allowed = match self.authority.purpose {
            PurposeClass::Bootstrap => matches!(
                method,
                RealmMethod::Bootstrap | RealmMethod::Enroll | RealmMethod::Inspect
            ),
            PurposeClass::Enrolled | PurposeClass::Local => true,
        };
        if allowed {
            Ok(())
        } else {
            Err(rpc_error(
                ttrpc::Code::PERMISSION_DENIED,
                "realm-purpose-denied",
            ))
        }
    }

    fn dispatch(
        &self,
        method: RealmMethod,
        request: &common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        if matches!(
            method,
            RealmMethod::Bootstrap
                | RealmMethod::Enroll
                | RealmMethod::AuthorizeShortcut
                | RealmMethod::RevokeShortcut
                | RealmMethod::ReportShortcutClose
        ) {
            return self.dispatch_mutation(method, request);
        }
        match method {
            RealmMethod::ResolveRoute => self.resolve_route(request),
            RealmMethod::Inspect => self.inspect(request),
            _ => Err(rpc_error(ttrpc::Code::INTERNAL, "realm-method-invalid")),
        }
    }

    fn dispatch_mutation(
        &self,
        method: RealmMethod,
        request: &common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        let metadata = request.metadata.as_ref().expect("validated metadata");
        let fingerprint = request_fingerprint(method, request);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = state.mutations.get(&metadata.idempotency_key) {
            if existing.fingerprint == fingerprint {
                return Ok(existing.response.clone());
            }
            return Err(rpc_error(
                ttrpc::Code::ALREADY_EXISTS,
                "realm-idempotency-conflict",
            ));
        }
        if state.mutations.len() >= self.limits.max_mutation_records {
            return Err(rpc_error(
                ttrpc::Code::RESOURCE_EXHAUSTED,
                "realm-mutation-table-full",
            ));
        }
        let response = match method {
            RealmMethod::Bootstrap => self.bootstrap(&mut state, request)?,
            RealmMethod::Enroll => self.enroll(&mut state, request)?,
            RealmMethod::AuthorizeShortcut => self.authorize_shortcut(&mut state, request)?,
            RealmMethod::RevokeShortcut => {
                self.finish_shortcut(&mut state, request, ShortcutState::Revoked)?
            }
            RealmMethod::ReportShortcutClose => {
                self.finish_shortcut(&mut state, request, ShortcutState::Closed)?
            }
            _ => return Err(rpc_error(ttrpc::Code::INTERNAL, "realm-method-invalid")),
        };
        state.mutations.insert(
            metadata.idempotency_key.clone(),
            MutationRecord {
                fingerprint,
                response: response.clone(),
            },
        );
        Ok(response)
    }

    fn bootstrap(
        &self,
        state: &mut RealmState,
        request: &common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        let realm = canonical_realm_resource(request)?;
        require_operation_and_digest(request)?;
        if !matches!(
            self.authority.peer_role,
            EndpointRole::LocalRootController | EndpointRole::RealmController
        ) {
            return Err(rpc_error(
                ttrpc::Code::PERMISSION_DENIED,
                "realm-bootstrap-role-denied",
            ));
        }
        let digest = exact_digest(&request.request_digest)?;
        let binding = BootstrapBinding {
            envelope_digest: digest,
            operation_id: request.operation_id.clone(),
            controller_generation: self.driver.generation(),
        };
        if let Some(existing) = state.bootstraps.get(&realm) {
            if existing != &binding {
                return Err(rpc_error(
                    ttrpc::Code::ALREADY_EXISTS,
                    "realm-bootstrap-binding-conflict",
                ));
            }
        } else {
            if state.bootstraps.len() >= self.limits.max_bindings {
                return Err(rpc_error(
                    ttrpc::Code::RESOURCE_EXHAUSTED,
                    "realm-binding-table-full",
                ));
            }
            state.bootstraps.insert(realm.clone(), binding);
        }
        success_response(
            &request.operation_id,
            &realm,
            binding_digest(
                self.authority.realm.as_str(),
                self.driver.generation(),
                self.policy_epoch,
                &[digest],
            ),
        )
    }

    fn enroll(
        &self,
        state: &mut RealmState,
        request: &common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        let realm = canonical_realm_resource(request)?;
        require_operation_and_digest(request)?;
        let bootstrap = state.bootstraps.get(&realm).ok_or_else(|| {
            rpc_error(ttrpc::Code::FAILED_PRECONDITION, "realm-bootstrap-required")
        })?;
        if bootstrap.controller_generation != self.driver.generation() {
            return Err(rpc_error(
                ttrpc::Code::FAILED_PRECONDITION,
                "realm-controller-generation-stale",
            ));
        }
        let enrollment_digest = exact_digest(&request.request_digest)?;
        let binding = EnrollmentBinding {
            bootstrap_digest: bootstrap.envelope_digest,
            enrollment_digest,
            operation_id: request.operation_id.clone(),
            controller_generation: self.driver.generation(),
        };
        if let Some(existing) = state.enrollments.get(&realm) {
            if existing != &binding {
                return Err(rpc_error(
                    ttrpc::Code::ALREADY_EXISTS,
                    "realm-enrollment-binding-conflict",
                ));
            }
        } else {
            if state.enrollments.len() >= self.limits.max_bindings {
                return Err(rpc_error(
                    ttrpc::Code::RESOURCE_EXHAUSTED,
                    "realm-binding-table-full",
                ));
            }
            state.enrollments.insert(realm.clone(), binding.clone());
        }
        success_response(
            &request.operation_id,
            &realm,
            binding_digest(
                self.authority.realm.as_str(),
                binding.controller_generation,
                self.policy_epoch,
                &[binding.bootstrap_digest, binding.enrollment_digest],
            ),
        )
    }

    fn resolve_route(
        &self,
        request: &common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        let realm = canonical_realm_resource(request)?;
        require_operation_and_digest(request)?;
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let enrollment = state
            .enrollments
            .get(&realm)
            .ok_or_else(|| rpc_error(ttrpc::Code::NOT_FOUND, "realm-route-not-enrolled"))?;
        if enrollment.controller_generation != self.driver.generation() {
            return Err(rpc_error(
                ttrpc::Code::FAILED_PRECONDITION,
                "realm-controller-generation-stale",
            ));
        }
        let request_digest = exact_digest(&request.request_digest)?;
        success_response(
            &request.operation_id,
            &realm,
            binding_digest(
                self.authority.realm.as_str(),
                enrollment.controller_generation,
                self.policy_epoch,
                &[
                    enrollment.bootstrap_digest,
                    enrollment.enrollment_digest,
                    request_digest,
                ],
            ),
        )
    }

    fn authorize_shortcut(
        &self,
        state: &mut RealmState,
        request: &common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        let shortcut_id = canonical_shortcut_resource(request)?;
        require_operation_and_digest(request)?;
        let route_handle = RealmId::parse(request.stream_id.clone())
            .map_err(|_| rpc_error(ttrpc::Code::INVALID_ARGUMENT, "realm-route-handle-invalid"))?
            .as_str()
            .to_owned();
        let enrollment = state
            .enrollments
            .get(&route_handle)
            .ok_or_else(|| rpc_error(ttrpc::Code::NOT_FOUND, "realm-route-not-enrolled"))?;
        let metadata = request.metadata.as_ref().expect("validated metadata");
        if metadata.expires_at_unix_ms <= self.clock.now_unix_ms() {
            return Err(rpc_error(
                ttrpc::Code::DEADLINE_EXCEEDED,
                "realm-shortcut-expired",
            ));
        }
        let shortcut = ShortcutBinding {
            route_handle: route_handle.clone(),
            route_digest: exact_digest(&request.request_digest)?,
            operation_id: request.operation_id.clone(),
            controller_generation: enrollment.controller_generation,
            policy_epoch: self.policy_epoch,
            expires_at_unix_ms: metadata.expires_at_unix_ms,
            state: ShortcutState::Authorized,
        };
        if let Some(existing) = state.shortcuts.get(&shortcut_id) {
            if existing != &shortcut {
                return Err(rpc_error(
                    ttrpc::Code::ALREADY_EXISTS,
                    "realm-shortcut-binding-conflict",
                ));
            }
        } else {
            if state.shortcuts.len() >= self.limits.max_shortcuts {
                return Err(rpc_error(
                    ttrpc::Code::RESOURCE_EXHAUSTED,
                    "realm-shortcut-table-full",
                ));
            }
            state
                .shortcuts
                .insert(shortcut_id.clone(), shortcut.clone());
        }
        success_response(
            &request.operation_id,
            &shortcut_id,
            shortcut_digest(self.authority.realm.as_str(), &shortcut),
        )
    }

    fn finish_shortcut(
        &self,
        state: &mut RealmState,
        request: &common::ServiceRequest,
        target_state: ShortcutState,
    ) -> ttrpc::Result<common::ServiceResponse> {
        let shortcut_id = canonical_shortcut_resource(request)?;
        require_operation_and_digest(request)?;
        let shortcut = state
            .shortcuts
            .get_mut(&shortcut_id)
            .ok_or_else(|| rpc_error(ttrpc::Code::NOT_FOUND, "realm-shortcut-not-found"))?;
        if shortcut.controller_generation != self.driver.generation()
            || shortcut.policy_epoch != self.policy_epoch
            || shortcut.route_digest != exact_digest(&request.request_digest)?
        {
            return Err(rpc_error(
                ttrpc::Code::FAILED_PRECONDITION,
                "realm-shortcut-binding-stale",
            ));
        }
        match (shortcut.state, target_state) {
            (ShortcutState::Authorized, ShortcutState::Revoked)
            | (ShortcutState::Authorized, ShortcutState::Closed)
            | (ShortcutState::Revoked, ShortcutState::Closed)
            | (ShortcutState::Revoked, ShortcutState::Revoked)
            | (ShortcutState::Closed, ShortcutState::Closed) => shortcut.state = target_state,
            (ShortcutState::Closed, ShortcutState::Revoked) => {
                return Err(rpc_error(
                    ttrpc::Code::FAILED_PRECONDITION,
                    "realm-shortcut-already-closed",
                ));
            }
            _ => {
                return Err(rpc_error(
                    ttrpc::Code::FAILED_PRECONDITION,
                    "realm-shortcut-transition-invalid",
                ));
            }
        }
        success_response(
            &request.operation_id,
            &shortcut_id,
            shortcut_digest(self.authority.realm.as_str(), shortcut),
        )
    }

    fn inspect(&self, request: &common::ServiceRequest) -> ttrpc::Result<common::ServiceResponse> {
        if !request.resource_id.is_empty() || !request.request_digest.is_empty() {
            return Err(rpc_error(
                ttrpc::Code::INVALID_ARGUMENT,
                "realm-inspect-filter-invalid",
            ));
        }
        let limit = if request.page_size == 0 {
            64
        } else {
            request.page_size as usize
        };
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut rows = Vec::new();
        for (realm, binding) in &state.enrollments {
            rows.push((
                format!("realm-{realm}"),
                ObservationState::OBSERVATION_STATE_READY,
                binding.controller_generation,
                binding_digest(
                    self.authority.realm.as_str(),
                    binding.controller_generation,
                    self.policy_epoch,
                    &[binding.bootstrap_digest, binding.enrollment_digest],
                ),
            ));
        }
        for (id, shortcut) in &state.shortcuts {
            let state = match shortcut.state {
                ShortcutState::Authorized => ObservationState::OBSERVATION_STATE_READY,
                ShortcutState::Revoked | ShortcutState::Closed => {
                    ObservationState::OBSERVATION_STATE_STOPPED
                }
            };
            rows.push((
                format!("shortcut-{id}"),
                state,
                shortcut.controller_generation,
                shortcut_digest(self.authority.realm.as_str(), shortcut),
            ));
        }
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        let start = if request.page_cursor.is_empty() {
            0
        } else {
            rows.partition_point(|row| row.0 <= request.page_cursor)
        };
        let end = start.saturating_add(limit).min(rows.len());
        let mut response = common::ServiceResponse::new();
        response.outcome = EnumOrUnknown::new(Outcome::OUTCOME_SUCCEEDED);
        response.operation_id = request.operation_id.clone();
        response.observations = rows[start..end]
            .iter()
            .map(
                |(resource_id, state, generation, digest)| common::Observation {
                    resource_id: resource_id.clone(),
                    state: EnumOrUnknown::new(*state),
                    generation: *generation,
                    digest: digest.to_vec(),
                    ..Default::default()
                },
            )
            .collect();
        if end < rows.len() {
            response.next_page_cursor = rows[end - 1].0.clone();
        }
        validate_response(response)
    }

    fn record(&self, method: RealmMethod, outcome: RealmAuditOutcome) {
        self.audit.record(RealmAuditEvent { method, outcome });
    }
}

#[async_trait]
impl RealmService for RealmServiceServer {
    async fn bootstrap(
        &self,
        context: &TtrpcContext,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        self.call(RealmMethod::Bootstrap, request, ttrpc_timeout(context))
            .await
    }

    async fn enroll(
        &self,
        context: &TtrpcContext,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        self.call(RealmMethod::Enroll, request, ttrpc_timeout(context))
            .await
    }

    async fn resolve_route(
        &self,
        context: &TtrpcContext,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        self.call(RealmMethod::ResolveRoute, request, ttrpc_timeout(context))
            .await
    }

    async fn authorize_shortcut(
        &self,
        context: &TtrpcContext,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        self.call(
            RealmMethod::AuthorizeShortcut,
            request,
            ttrpc_timeout(context),
        )
        .await
    }

    async fn revoke_shortcut(
        &self,
        context: &TtrpcContext,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        self.call(RealmMethod::RevokeShortcut, request, ttrpc_timeout(context))
            .await
    }

    async fn report_shortcut_close(
        &self,
        context: &TtrpcContext,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        self.call(
            RealmMethod::ReportShortcutClose,
            request,
            ttrpc_timeout(context),
        )
        .await
    }

    async fn inspect(
        &self,
        context: &TtrpcContext,
        request: common::ServiceRequest,
    ) -> ttrpc::Result<common::ServiceResponse> {
        self.call(RealmMethod::Inspect, request, ttrpc_timeout(context))
            .await
    }

    async fn cancel(
        &self,
        _: &TtrpcContext,
        request: common::CancelRequest,
    ) -> ttrpc::Result<common::CancelResponse> {
        request
            .validate_wire(false)
            .map_err(|error| rpc_error(ttrpc::Code::INVALID_ARGUMENT, error.to_string()))?;
        let outcome = if request.session_generation != self.driver.generation() {
            CancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH
        } else {
            RequestId::new(request.request_id.clone()).map_err(|_| {
                rpc_error(ttrpc::Code::INVALID_ARGUMENT, "realm-request-id-invalid")
            })?;
            let cancellation = self
                .active_calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&request.request_id)
                .cloned();
            match cancellation {
                Some(cancellation) if cancellation.cancel() => {
                    CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
                }
                Some(_) => CancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL,
                None => CancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST,
            }
        };
        self.record(RealmMethod::Cancel, RealmAuditOutcome::Completed);
        let response = common::CancelResponse {
            outcome: EnumOrUnknown::new(outcome),
            ..Default::default()
        };
        response
            .validate_wire(false)
            .map_err(|error| rpc_error(ttrpc::Code::INTERNAL, error.to_string()))?;
        Ok(response)
    }
}

pub struct RealmServiceProcess {
    driver: Arc<dyn ComponentSessionDriver>,
    server: Arc<RealmServiceServer>,
    services: Arc<HashMap<String, Service>>,
    dispatch_permits: Arc<Semaphore>,
}

impl fmt::Debug for RealmServiceProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RealmServiceProcess")
            .field("service", &REALM_SERVICE_NAME)
            .finish_non_exhaustive()
    }
}

impl RealmServiceProcess {
    pub fn new(
        authority: RealmSessionAuthority,
        driver: Arc<dyn ComponentSessionDriver>,
        policy_epoch: u64,
    ) -> Result<Arc<Self>, RealmServiceError> {
        let server = RealmServiceServer::new(authority, driver.clone(), policy_epoch)?;
        Self::from_server(server, driver)
    }

    pub fn from_server(
        server: Arc<RealmServiceServer>,
        driver: Arc<dyn ComponentSessionDriver>,
    ) -> Result<Arc<Self>, RealmServiceError> {
        if driver.generation() == 0 || driver.generation() != server.driver.generation() {
            return Err(RealmServiceError::InvalidGeneration);
        }
        let services = Arc::new(server.generated_services());
        if services.len() != 1 || !services.contains_key(REALM_SERVICE_NAME) {
            return Err(RealmServiceError::ProtocolViolation);
        }
        Ok(Arc::new(Self {
            driver,
            server,
            services,
            dispatch_permits: Arc::new(Semaphore::new(MAX_DISPATCH_IN_FLIGHT)),
        }))
    }

    pub fn service_names(&self) -> Vec<String> {
        let mut names = self.services.keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }

    pub fn server(&self) -> &Arc<RealmServiceServer> {
        &self.server
    }

    pub async fn serve(self: Arc<Self>) -> Result<(), RealmServiceError> {
        let mut tasks = JoinSet::new();
        loop {
            while tasks.try_join_next().is_some() {}
            let frame = match self.driver.receive_ttrpc().await {
                Ok(frame) => frame,
                Err(_) => {
                    finish_tasks(&mut tasks).await;
                    return Err(RealmServiceError::SessionClosed);
                }
            };
            let (header, request) = match decode_request_frame(&frame) {
                Ok(request) => request,
                Err(rejection) => {
                    self.server
                        .record(RealmMethod::Inspect, RealmAuditOutcome::ProtocolViolation);
                    if let Some(stream_id) = rejection.stream_id {
                        let response = error_response(stream_id, rejection.code, rejection.reason)?;
                        self.driver
                            .send_ttrpc(response)
                            .await
                            .map_err(|_| RealmServiceError::SessionClosed)?;
                    }
                    if rejection.fatal {
                        finish_tasks(&mut tasks).await;
                        return Err(RealmServiceError::ProtocolViolation);
                    }
                    continue;
                }
            };
            let permit = match self.dispatch_permits.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    self.server
                        .record(RealmMethod::Inspect, RealmAuditOutcome::Overloaded);
                    self.driver
                        .send_ttrpc(error_response(
                            header.stream_id,
                            ttrpc::Code::RESOURCE_EXHAUSTED,
                            "realm-service-overloaded",
                        )?)
                        .await
                        .map_err(|_| RealmServiceError::SessionClosed)?;
                    continue;
                }
            };
            let process = self.clone();
            tasks.spawn(async move {
                let _permit = permit;
                let response = process.dispatch(header, request).await;
                if let Ok(response) = response {
                    let _ = process.driver.send_ttrpc(response).await;
                }
            });
        }
    }

    async fn dispatch(
        &self,
        header: MessageHeader,
        request: ttrpc::Request,
    ) -> Result<Vec<u8>, RealmServiceError> {
        let Some(service) = self.services.get(&request.service) else {
            return error_response(
                header.stream_id,
                ttrpc::Code::INVALID_ARGUMENT,
                "realm-service-unavailable",
            );
        };
        let Some(method) = service.methods.get(&request.method) else {
            return error_response(
                header.stream_id,
                ttrpc::Code::UNIMPLEMENTED,
                "realm-method-unavailable",
            );
        };
        let timeout_nano = request.timeout_nano;
        if timeout_nano < 0 {
            return error_response(
                header.stream_id,
                ttrpc::Code::INVALID_ARGUMENT,
                "realm-deadline-invalid",
            );
        }
        let context = TtrpcContext {
            mh: header,
            metadata: context::from_pb(&request.metadata),
            timeout_nano,
        };
        let response = if timeout_nano == 0 {
            method.handler(context, request).await
        } else {
            match tokio::time::timeout(
                Duration::from_nanos(u64::try_from(timeout_nano).unwrap_or(0)),
                method.handler(context, request),
            )
            .await
            {
                Ok(response) => response,
                Err(_) => {
                    return error_response(
                        header.stream_id,
                        ttrpc::Code::DEADLINE_EXCEEDED,
                        "realm-deadline-expired",
                    );
                }
            }
        };
        match response {
            Ok(response) => encode_response(header.stream_id, response),
            Err(ttrpc::Error::RpcStatus(status)) => encode_response(
                header.stream_id,
                ttrpc::Response {
                    status: MessageField::some(status),
                    ..Default::default()
                },
            ),
            Err(_) => error_response(
                header.stream_id,
                ttrpc::Code::INTERNAL,
                "realm-request-rejected",
            ),
        }
    }
}

struct InboundCall {
    driver: Arc<dyn ComponentSessionDriver>,
    request_id: Option<RequestId>,
    cancellation: Cancellation,
}

impl InboundCall {
    async fn register(
        driver: Arc<dyn ComponentSessionDriver>,
        request_id: RequestId,
    ) -> ttrpc::Result<Self> {
        let cancellation = driver
            .register_inbound_call(request_id.clone())
            .await
            .map_err(session_error)?;
        Ok(Self {
            driver,
            request_id: Some(request_id),
            cancellation,
        })
    }

    fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }

    async fn finish<T>(mut self, result: ttrpc::Result<T>) -> ttrpc::Result<T> {
        let request_id = self
            .request_id
            .take()
            .ok_or_else(|| rpc_error(ttrpc::Code::INTERNAL, "realm-call-state-invalid"))?;
        let completed = if result.is_ok() {
            self.driver.complete_inbound_call(request_id).await
        } else {
            self.driver.remove_inbound_call(request_id).await
        }
        .map_err(session_error)?;
        if !completed {
            return Err(rpc_error(ttrpc::Code::INTERNAL, "realm-call-state-missing"));
        }
        result
    }
}

impl Drop for InboundCall {
    fn drop(&mut self) {
        let Some(request_id) = self.request_id.take() else {
            return;
        };
        let driver = self.driver.clone();
        tokio::spawn(async move {
            let _ = driver.remove_inbound_call(request_id).await;
        });
    }
}

fn canonical_realm_resource(request: &common::ServiceRequest) -> ttrpc::Result<String> {
    RealmId::parse(request.resource_id.clone())
        .map(|realm| realm.as_str().to_owned())
        .map_err(|_| rpc_error(ttrpc::Code::INVALID_ARGUMENT, "realm-resource-invalid"))
}

fn canonical_shortcut_resource(request: &common::ServiceRequest) -> ttrpc::Result<String> {
    if request.resource_id.starts_with("shortcut-") {
        Ok(request.resource_id.clone())
    } else {
        Err(rpc_error(
            ttrpc::Code::INVALID_ARGUMENT,
            "realm-shortcut-id-invalid",
        ))
    }
}

fn require_operation_and_digest(request: &common::ServiceRequest) -> ttrpc::Result<()> {
    if request.operation_id.is_empty() {
        return Err(rpc_error(
            ttrpc::Code::INVALID_ARGUMENT,
            "realm-operation-id-missing",
        ));
    }
    exact_digest(&request.request_digest).map(|_| ())
}

fn exact_digest(value: &[u8]) -> ttrpc::Result<[u8; 32]> {
    let digest: [u8; 32] = value.try_into().map_err(|_| {
        rpc_error(
            ttrpc::Code::INVALID_ARGUMENT,
            "realm-request-digest-required",
        )
    })?;
    if digest == [0; 32] {
        return Err(rpc_error(
            ttrpc::Code::INVALID_ARGUMENT,
            "realm-request-digest-required",
        ));
    }
    Ok(digest)
}

fn request_fingerprint(method: RealmMethod, request: &common::ServiceRequest) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update([method as u8]);
    hash_field(&mut hash, request.resource_id.as_bytes());
    hash_field(&mut hash, request.operation_id.as_bytes());
    hash_field(&mut hash, &request.request_digest);
    hash_field(&mut hash, request.stream_id.as_bytes());
    hash.update(request.desired_state.value().to_be_bytes());
    hash.finalize().into()
}

fn binding_digest(
    realm: &str,
    generation: u64,
    policy_epoch: u64,
    digests: &[[u8; 32]],
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash_field(&mut hash, realm.as_bytes());
    hash.update(generation.to_be_bytes());
    hash.update(policy_epoch.to_be_bytes());
    for digest in digests {
        hash.update(digest);
    }
    hash.finalize().into()
}

fn shortcut_digest(realm: &str, shortcut: &ShortcutBinding) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash_field(&mut hash, realm.as_bytes());
    hash_field(&mut hash, shortcut.route_handle.as_bytes());
    hash.update(shortcut.route_digest);
    hash_field(&mut hash, shortcut.operation_id.as_bytes());
    hash.update(shortcut.controller_generation.to_be_bytes());
    hash.update(shortcut.policy_epoch.to_be_bytes());
    hash.update(shortcut.expires_at_unix_ms.to_be_bytes());
    hash.update([shortcut.state as u8]);
    hash.finalize().into()
}

fn hash_field(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}

fn success_response(
    operation_id: &str,
    resource_handle: &str,
    result_digest: [u8; 32],
) -> ttrpc::Result<common::ServiceResponse> {
    let response = common::ServiceResponse {
        outcome: EnumOrUnknown::new(Outcome::OUTCOME_SUCCEEDED),
        operation_id: operation_id.to_owned(),
        resource_handle: resource_handle.to_owned(),
        result_digest: result_digest.to_vec(),
        ..Default::default()
    };
    validate_response(response)
}

fn validate_response(response: common::ServiceResponse) -> ttrpc::Result<common::ServiceResponse> {
    response
        .validate_wire(false)
        .map_err(|error| rpc_error(ttrpc::Code::INTERNAL, error.to_string()))?;
    Ok(response)
}

fn rpc_error(code: ttrpc::Code, reason: impl Into<String>) -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(code, reason.into()))
}

fn session_error(error: SessionError) -> ttrpc::Error {
    let code = match error.code() {
        SessionErrorCode::GenerationMismatch => ttrpc::Code::FAILED_PRECONDITION,
        SessionErrorCode::DeadlineInvalid | SessionErrorCode::DeadlineExpired => {
            ttrpc::Code::DEADLINE_EXCEEDED
        }
        SessionErrorCode::Cancelled => ttrpc::Code::CANCELLED,
        SessionErrorCode::QueueBackpressure => ttrpc::Code::RESOURCE_EXHAUSTED,
        _ => ttrpc::Code::UNAVAILABLE,
    };
    rpc_error(code, error.code().as_str())
}

fn ttrpc_timeout(context: &TtrpcContext) -> Option<u64> {
    u64::try_from(context.timeout_nano)
        .ok()
        .filter(|timeout| *timeout > 0)
}

struct FrameRejection {
    stream_id: Option<u32>,
    code: ttrpc::Code,
    reason: &'static str,
    fatal: bool,
}

fn decode_request_frame(frame: &[u8]) -> Result<(MessageHeader, ttrpc::Request), FrameRejection> {
    let header_bytes: [u8; MESSAGE_HEADER_LENGTH] = frame
        .get(..MESSAGE_HEADER_LENGTH)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(FrameRejection {
            stream_id: None,
            code: ttrpc::Code::INVALID_ARGUMENT,
            reason: "realm-frame-invalid",
            fatal: true,
        })?;
    let header = MessageHeader::from(header_bytes);
    if header.stream_id == 0
        || header.stream_id % 2 == 0
        || header.type_ != MESSAGE_TYPE_REQUEST
        || header.flags != 0
    {
        return Err(FrameRejection {
            stream_id: Some(header.stream_id),
            code: ttrpc::Code::INVALID_ARGUMENT,
            reason: "realm-frame-invalid",
            fatal: true,
        });
    }
    let body = &frame[MESSAGE_HEADER_LENGTH..];
    if header.length as usize != body.len() || body.len() > MAX_LOGICAL_MESSAGE_BYTES as usize {
        return Err(FrameRejection {
            stream_id: Some(header.stream_id),
            code: ttrpc::Code::INVALID_ARGUMENT,
            reason: "realm-frame-invalid",
            fatal: true,
        });
    }
    let request = ttrpc::Request::parse_from_bytes(body).map_err(|_| FrameRejection {
        stream_id: Some(header.stream_id),
        code: ttrpc::Code::INVALID_ARGUMENT,
        reason: "realm-request-invalid",
        fatal: false,
    })?;
    Ok((header, request))
}

fn error_response(
    stream_id: u32,
    code: ttrpc::Code,
    reason: &'static str,
) -> Result<Vec<u8>, RealmServiceError> {
    encode_response(
        stream_id,
        ttrpc::Response {
            status: MessageField::some(ttrpc::get_status(code, reason)),
            ..Default::default()
        },
    )
}

fn encode_response(
    stream_id: u32,
    response: ttrpc::Response,
) -> Result<Vec<u8>, RealmServiceError> {
    let body = response
        .write_to_bytes()
        .map_err(|_| RealmServiceError::ProtocolViolation)?;
    if body.len() > MAX_LOGICAL_MESSAGE_BYTES as usize {
        return Err(RealmServiceError::ProtocolViolation);
    }
    let length = u32::try_from(body.len()).map_err(|_| RealmServiceError::ProtocolViolation)?;
    let mut frame = Vec::from(MessageHeader::new_response(stream_id, length));
    frame.extend_from_slice(&body);
    Ok(frame)
}

async fn finish_tasks(tasks: &mut JoinSet<()>) {
    let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    tasks.abort_all();
}
