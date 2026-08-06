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

use d2b_contracts::{
    broker_wire::{OpenZoneStoreResponse, ZoneStoreDisposition},
    v3::{
        AuthenticatedSubjectContext, ConfigurationGeneration, ControllerGeneration, EvidenceClass,
        Locality, MAX_FILTER_VALUES, MAX_LIST_FILTERS, MAX_LIST_PAGE_SIZE, MAX_PAGE_CURSOR_BYTES,
        MAX_RESPONSE_CANONICAL_BYTES, ResourceError, ResourceErrorKind, ResourceName, ResourceRef,
        ResourceTypeName, ResourceUid, RetryClass, Timestamp, ZoneId, ZoneRevision,
    },
};
use d2b_core_controller::main::{CoreProcess, StartupStage};
use d2b_resource_api::{
    RedbBackend, ResourceService,
    authz::{ApiCatalog, AuthorizationState, NativeAuthorizer},
};
use d2b_resource_store::{PolicySnapshot, StoreListResult, StoreSlot};
use d2b_resource_store_redb::{RedbResourceStore, StoreIdentity, write_provisioning_marker};
use serde_json::{Map, Value, json};
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
#[derive(Debug)]
pub struct OpenedZoneStore {
    /// Opaque broker response metadata.
    pub response: OpenZoneStoreResponse,
    /// The one owned database descriptor received from the broker.
    pub database_fd: OwnedFd,
}

/// Readiness projection for one Zone runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneRuntimeReadiness {
    pub store_ready: bool,
    pub resource_api_ready: bool,
    pub local_session_ready: bool,
    pub provider_path_ready: bool,
    pub core_stage: StartupStage,
}

/// A production Resource API and core-controller runtime for one Zone.
pub struct ZoneResourceRuntime {
    zone: ZoneId,
    store_id: String,
    store: Arc<RedbResourceStore>,
    api: Arc<ResourceService<RedbBackend>>,
    authorization_state: AuthorizationState,
    core: Mutex<CoreProcess>,
    readiness: ZoneRuntimeReadiness,
}

impl core::fmt::Debug for ZoneResourceRuntime {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ZoneResourceRuntime")
            .field("zone", &self.zone)
            .field("store_id", &"<opaque>")
            .field("readiness", &self.readiness)
            .finish()
    }
}

impl ZoneResourceRuntime {
    /// Open one Zone from a broker-owned descriptor.
    pub async fn open(zone: ZoneId, opened: OpenedZoneStore) -> Result<Self, ResourceRuntimeError> {
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
                RedbResourceStore::provision_owned(file, marker, store_identity, acceptor).await
            }
            ZoneStoreDisposition::Opened => {
                RedbResourceStore::open_owned(file, store_identity, acceptor).await
            }
        }
        .map_err(|_| ResourceRuntimeError::StoreOpenFailed)?;
        let store = Arc::new(store);
        let authorization_state = runtime_authorization_state()?;
        let api = Arc::new(
            ResourceService::new(
                Arc::new(RedbBackend::from_arc(Arc::clone(&store))),
                authorizer,
            )
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?,
        );

        // The public daemon has no authenticated Zone-session binding at this
        // seam, so the controller must remain waiting rather than fabricate
        // endpoint or session readiness.
        let core = CoreProcess::new();
        let stage = core.stage();
        Ok(Self {
            zone,
            store_id: expected_store_id,
            store,
            api,
            authorization_state,
            core: Mutex::new(core),
            readiness: ZoneRuntimeReadiness {
                store_ready: true,
                resource_api_ready: true,
                local_session_ready: false,
                provider_path_ready: false,
                core_stage: stage,
            },
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
        self.dispatch_cli_request_with_subject(request, None).await
    }

    /// Dispatch a request carrying an authenticated local session context.
    ///
    /// The context is supplied by the authenticated Zone/session boundary,
    /// never decoded from the request. The current public daemon path does
    /// not have this binding and consequently uses
    /// [`Self::dispatch_cli_request`], which fails closed.
    #[allow(dead_code)]
    pub(crate) async fn dispatch_authenticated_cli_request(
        &self,
        request: &Value,
        subject: AuthenticatedSubjectContext,
    ) -> Result<Value, ResourceRuntimeError> {
        self.dispatch_cli_request_with_subject(request, Some(&subject))
            .await
    }

    async fn dispatch_cli_request_with_subject(
        &self,
        request: &Value,
        subject: Option<&AuthenticatedSubjectContext>,
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
        match method {
            "Get" => {
                let target = request
                    .get("resourceRef")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                    .ok_or(ResourceRuntimeError::RequestInvalid)?;
                let operation_id = operation_id(request)?;
                let Some(subject) = subject else {
                    return Ok(resource_error_envelope(&identity_unbound_error()));
                };
                if !subject_matches_runtime(subject, &self.zone) {
                    return Ok(resource_error_envelope(&identity_unbound_error()));
                }
                let resource = match self
                    .api
                    .get_runtime(
                        subject.clone(),
                        self.authorization_state.clone(),
                        target,
                        operation_id,
                    )
                    .await
                {
                    Ok(resource) => resource,
                    Err(error) => return Ok(resource_error_envelope(&error)),
                };
                match decode_resource_result(&resource.canonical_json) {
                    Ok(value) => Ok(value),
                    Err(error) => Ok(resource_error_envelope(&error)),
                }
            }
            "List" => {
                let resource_type = match parse_list_request(request) {
                    Ok(resource_type) => resource_type,
                    Err(ResourceRuntimeError::CapabilityUnavailable) => {
                        return Ok(resource_error_envelope(&capability_error()));
                    }
                    Err(error) => return Err(error),
                };
                let operation_id = operation_id(request)?;
                let Some(subject) = subject else {
                    return Ok(resource_error_envelope(&identity_unbound_error()));
                };
                if !subject_matches_runtime(subject, &self.zone) {
                    return Ok(resource_error_envelope(&identity_unbound_error()));
                }
                let result = match self
                    .api
                    .list_runtime(
                        subject.clone(),
                        self.authorization_state.clone(),
                        resource_type,
                        operation_id,
                    )
                    .await
                {
                    Ok(result) => result,
                    Err(error) => return Ok(resource_error_envelope(&error)),
                };
                match encode_list_result(result) {
                    Ok(value) => Ok(value),
                    Err(error) => Ok(resource_error_envelope(&error)),
                }
            }
            "ZoneList" | "ZoneStatus" => Ok(json!({
                "zoneRef": format!("Zone/{}", self.zone.as_str()),
                "store": "ready",
                "resourceApi": "ready",
                "core": format!("{:?}", self.readiness.core_stage),
            })),
            "Watch" | "Status" | "Create" | "Update" | "UpdateSpec" | "UpdateStatus"
            | "UpdateMetadata" | "UpdateFinalizers" | "Delete" | "Upgrade" | "Reconcile"
            | "ProcessAttach" => Ok(resource_error_envelope(&capability_error())),
            _ => Err(ResourceRuntimeError::RequestInvalid),
        }
    }

    /// Close the production redb workers before the runtime is discarded.
    pub async fn shutdown(self) -> Result<(), ResourceRuntimeError> {
        let ZoneResourceRuntime { store, api, .. } = self;
        drop(api);
        let store = Arc::try_unwrap(store).map_err(|_| ResourceRuntimeError::CoreStartupFailed)?;
        store
            .shutdown()
            .await
            .map_err(|_| ResourceRuntimeError::StoreOpenFailed)
    }
}

const MAX_OPERATION_ID_BYTES: usize = 128;

fn operation_id(request: &Value) -> Result<String, ResourceRuntimeError> {
    match request.get("operationId") {
        None => Ok("cli-resource".to_owned()),
        Some(Value::String(value))
            if !value.is_empty() && value.len() <= MAX_OPERATION_ID_BYTES =>
        {
            Ok(value.clone())
        }
        _ => Err(ResourceRuntimeError::RequestInvalid),
    }
}

fn parse_list_request(request: &Value) -> Result<ResourceTypeName, ResourceRuntimeError> {
    let resource_type = request
        .get("resourceType")
        .and_then(Value::as_str)
        .and_then(|value| ResourceTypeName::parse(value).ok())
        .ok_or(ResourceRuntimeError::RequestInvalid)?;

    if let Some(resource_types) = request.get("resourceTypes") {
        let resource_types = resource_types
            .as_array()
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if resource_types.is_empty() {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        let parsed = resource_types
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .and_then(|value| ResourceTypeName::parse(value).ok())
                    .ok_or(ResourceRuntimeError::RequestInvalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if parsed.len() != 1 || parsed[0] != resource_type {
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
    }

    if let Some(filters) = request.get("filters") {
        let filters = filters
            .as_array()
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if filters.len() > MAX_LIST_FILTERS {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        for filter in filters {
            let object = filter
                .as_object()
                .ok_or(ResourceRuntimeError::RequestInvalid)?;
            let field = object
                .get("field")
                .and_then(Value::as_str)
                .filter(|field| !field.is_empty() && field.len() <= 64)
                .ok_or(ResourceRuntimeError::RequestInvalid)?;
            let values = object
                .get("values")
                .and_then(Value::as_array)
                .ok_or(ResourceRuntimeError::RequestInvalid)?;
            if values.is_empty() || values.len() > MAX_FILTER_VALUES {
                return Err(ResourceRuntimeError::RequestInvalid);
            }
            for value in values {
                if !value
                    .as_str()
                    .is_some_and(|value| !value.is_empty() && value.len() <= 256)
                {
                    return Err(ResourceRuntimeError::RequestInvalid);
                }
            }
            if field == "metadata.name" {
                for value in values {
                    let value = value.as_str().ok_or(ResourceRuntimeError::RequestInvalid)?;
                    ResourceName::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
                }
            }
        }
        if !filters.is_empty() {
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
    }

    if let Some(domain) = request.get("domain").and_then(Value::as_str)
        && !domain.is_empty()
        && !matches!(domain, "system" | "user")
    {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    if let Some(execution_ref) = request.get("executionRef").and_then(Value::as_str)
        && !execution_ref.is_empty()
    {
        ResourceRef::parse(execution_ref).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
    }
    optional_list_string(request, "executionRef", 256, true)?;
    optional_list_string(request, "domain", 16, true)?;
    optional_list_string(request, "phase", 128, true)?;
    optional_list_string(request, "labelSelector", 320, true)?;
    optional_list_string(request, "pageToken", MAX_PAGE_CURSOR_BYTES, true)?;
    optional_list_string(request, "cursor", MAX_PAGE_CURSOR_BYTES, true)?;
    optional_list_string(request, "pageCursor", MAX_PAGE_CURSOR_BYTES, true)?;

    optional_list_number(request, "limit", true)?;
    optional_list_number(request, "pageSize", true)?;
    optional_list_number(request, "revisionCursor", false)?;
    optional_list_number(request, "sinceRevision", false)?;
    optional_list_number(request, "afterRevision", false)?;
    optional_list_number(request, "revision", false)?;
    if request.get("updates").is_some_and(|value| !value.is_null()) {
        let updates = request
            .get("updates")
            .and_then(Value::as_bool)
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if updates {
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
    }
    if request
        .get("resourceNames")
        .is_some_and(|value| !value.is_null())
    {
        let names = request
            .get("resourceNames")
            .and_then(Value::as_array)
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if names.len() > MAX_FILTER_VALUES {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        for name in names {
            let name = name.as_str().ok_or(ResourceRuntimeError::RequestInvalid)?;
            ResourceName::parse(name).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
        }
        if !names.is_empty() {
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
    }
    if request
        .get("projection")
        .is_some_and(|value| !value.is_null())
    {
        return Err(ResourceRuntimeError::CapabilityUnavailable);
    }

    Ok(resource_type)
}

fn optional_list_string(
    request: &Value,
    field: &str,
    max_bytes: usize,
    capability_on_value: bool,
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
    if capability_on_value && !value.is_empty() {
        return Err(ResourceRuntimeError::CapabilityUnavailable);
    }
    Ok(())
}

fn optional_list_number(
    request: &Value,
    field: &str,
    page_size: bool,
) -> Result<(), ResourceRuntimeError> {
    let Some(value) = request.get(field) else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    let value = value.as_u64().ok_or(ResourceRuntimeError::RequestInvalid)?;
    if page_size && (value == 0 || value > u64::from(MAX_LIST_PAGE_SIZE)) {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    if value != 0 {
        return Err(ResourceRuntimeError::CapabilityUnavailable);
    }
    Ok(())
}

fn subject_matches_runtime(subject: &AuthenticatedSubjectContext, zone: &ZoneId) -> bool {
    subject.zone_ref().resource_type().as_str() == "Zone"
        && subject.zone_ref().name().as_str() == zone.as_str()
        && subject.evidence_class() == EvidenceClass::UnixPeer
        && subject.transport_binding().locality() == Locality::Local
}

fn identity_unbound_error() -> ResourceError {
    ResourceError::new(
        ResourceErrorKind::AuthorizationDenied,
        None,
        None,
        RetryClass::Reauthorize,
        d2b_contracts::v3::ResourceErrorReason::parse(
            "authenticated resource subject is unavailable",
        )
        .expect("fixed resource error reason"),
    )
    .expect("fixed resource error fields")
}

fn capability_error() -> ResourceError {
    ResourceError::terminal(
        ResourceErrorKind::UnsupportedCapability,
        "resource operation is not registered on the Zone service",
    )
}

fn resource_result_error(reason: &'static str) -> ResourceError {
    ResourceError::terminal(ResourceErrorKind::InternalIntegrityFailure, reason)
}

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
        response.insert("nextPageToken".to_owned(), Value::String(cursor));
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
            "authenticate an exact local Zone session and retry"
        }
        ResourceErrorKind::UnsupportedCapability => {
            "use a method exposed by the registered Zone service"
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

    /// Return the number of ready Zone runtimes.
    pub fn ready_zone_count(&self) -> usize {
        self.zones
            .values()
            .filter(|runtime| runtime.readiness().store_ready)
            .count()
    }

    /// Return the authoritative Zone identities currently owned by the plane.
    pub fn zone_ids(&self) -> Vec<ZoneId> {
        self.zones.keys().cloned().collect()
    }

    /// Drain runtimes and close every production backend.
    pub async fn shutdown(mut self) -> Result<(), ResourceRuntimeError> {
        let runtimes = std::mem::take(&mut self.zones);
        for (_, runtime) in runtimes {
            let runtime =
                Arc::try_unwrap(runtime).map_err(|_| ResourceRuntimeError::CoreStartupFailed)?;
            runtime.shutdown().await?;
        }
        Ok(())
    }
}

fn runtime_authorizer() -> Result<NativeAuthorizer, ResourceRuntimeError> {
    NativeAuthorizer::new(ApiCatalog::standard(), None)
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)
}

fn runtime_authorization_state() -> Result<AuthorizationState, ResourceRuntimeError> {
    Ok(AuthorizationState {
        snapshot: PolicySnapshot {
            policy_revision: 1,
            api_catalog_revision: 1,
            active_configuration_revision: ConfigurationGeneration::new(1)
                .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
            controller_generation: Some(
                ControllerGeneration::new(1)
                    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
            ),
        },
        zone_policy_revision: ZoneRevision::new(1),
        bootstrap_phase: d2b_resource_api::authz::BootstrapPhase::Disabled,
        now_tick: 1,
    })
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
        policy_revision: 1,
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
    use std::{fs::OpenOptions, os::fd::AsRawFd};

    use d2b_resource_store::mutation_seal::mutation_seal_pair;
    use d2b_resource_store_redb::write_provisioning_marker;

    #[test]
    fn stable_identity_is_repeatable_and_uuid_v4_shaped() {
        let first = stable_uid("store", "sha256:aaa");
        assert_eq!(first, stable_uid("store", "sha256:aaa"));
        assert_ne!(first, stable_uid("store", "sha256:bbb"));
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
    fn list_rejects_query_fields_the_runtime_cannot_forward() {
        let request = json!({
            "resourceType": "Guest",
            "limit": 10,
            "pageToken": "opaque-cursor",
            "executionRef": "Host/host-system",
        });
        assert_eq!(
            parse_list_request(&request),
            Err(ResourceRuntimeError::CapabilityUnavailable)
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
        assert_eq!(result["nextPageToken"], "opaque-cursor");
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
        let provisioned = RedbResourceStore::provision_owned(database, marker, identity, acceptor)
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
            },
        )
        .await
        .unwrap();
        assert_eq!(runtime.zone(), &zone);
        assert!(runtime.readiness().store_ready);
        assert!(runtime.readiness().resource_api_ready);
        assert!(!runtime.readiness().local_session_ready);
        assert!(!runtime.readiness().provider_path_ready);
        assert_eq!(runtime.core_stage().unwrap(), StartupStage::WaitingForStore);
        let zone_status = runtime
            .dispatch_cli_request(&json!({
                "method": "ZoneStatus",
                "zoneRef": "Zone/work",
            }))
            .await
            .unwrap();
        assert_eq!(zone_status["store"], "ready");
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
        assert_eq!(watch["error"]["kind"], "unsupported-capability");
        let status = runtime
            .dispatch_cli_request(&json!({
                "method": "Status",
                "zoneRef": "Zone/work",
                "resourceRef": "Guest/corp-vm",
            }))
            .await
            .unwrap();
        assert_eq!(status["error"]["kind"], "unsupported-capability");
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
            },
        )
        .await
        .unwrap();
        assert!(runtime.readiness().store_ready);
        assert!(runtime.readiness().resource_api_ready);
        runtime.shutdown().await.unwrap();
    }
}
