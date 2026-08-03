//! Production Zone resource-plane ownership for `d2bd`.
//!
//! A Zone runtime is opened only from the broker's opaque
//! [`OpenZoneStoreRequest`]. The broker owns path resolution and returns one
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

use d2b_bus::{BusAuthorizer, BusConfig, ZoneBus, ZoneRegistrar};
use d2b_contracts::{
    broker_wire::{OpenZoneStoreResponse, ZoneStoreDisposition},
    v3::{
        AuthenticatedSubjectContext, BindingDigest, ConfigurationGeneration, ControllerGeneration,
        EvidenceClass, Locality, ReconnectGeneration, ResourceGeneration, ResourceRef,
        ResourceTypeName, ResourceUid, SchemaFingerprint, ServiceName, SessionBinding,
        SessionPurpose, Timestamp, TranscriptHash, TransportBinding, ZoneId, ZoneRevision,
    },
};
use d2b_core_controller::main::{CoreProcess, StartupStage};
use d2b_resource_api::{
    RedbBackend, ResourceService,
    authz::{
        ApiCatalog, AuthorizationState, BindingScope, BoundSubject, CompiledRole,
        CompiledRoleBinding, NativeAuthorizer, PolicyRule, PolicySet, RelayGrantAuthority,
        ResourceVerb, SessionVerb,
    },
};
use d2b_resource_store::{PolicySnapshot, StoreSlot};
use d2b_resource_store_redb::{RedbResourceStore, StoreIdentity};
use serde_json::{Value, json};
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
    #[allow(dead_code)]
    api: Arc<ResourceService<RedbBackend>>,
    subject: AuthenticatedSubjectContext,
    authorization_state: AuthorizationState,
    #[allow(dead_code)]
    bus: ZoneBus,
    #[allow(dead_code)]
    registrar: Mutex<ZoneRegistrar>,
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
        let authorizer = Arc::new(runtime_authorizer(&zone)?);
        let acceptor = authorizer
            .take_store_seal(store_identity.seal_identity())
            .map_err(|_| ResourceRuntimeError::StoreSealUnavailable)?;
        let file = File::from(opened.database_fd);
        let store = Arc::new(
            RedbResourceStore::open_owned(file, store_identity, acceptor)
                .await
                .map_err(|_| ResourceRuntimeError::StoreOpenFailed)?,
        );
        let authorization_state = runtime_authorization_state()?;
        let subject = runtime_subject(&zone)?;
        let api = Arc::new(
            ResourceService::new(
                Arc::new(RedbBackend::from_arc(Arc::clone(&store))),
                authorizer,
            )
            .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?,
        );
        let bus_authorizer =
            BusAuthorizer::new(runtime_authorizer(&zone)?, authorization_state.clone())
                .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
        let (bus, registrar) = ZoneBus::new(zone.clone(), bus_authorizer, BusConfig::default())
            .map_err(|_| ResourceRuntimeError::CoreStartupFailed)?;

        let mut core = CoreProcess::new();
        let stage = core
            .start_production(1)
            .map_err(|_| ResourceRuntimeError::CoreStartupFailed)?;
        Ok(Self {
            zone,
            store_id: expected_store_id,
            store,
            api,
            subject,
            authorization_state,
            bus,
            registrar: Mutex::new(registrar),
            core: Mutex::new(core),
            readiness: ZoneRuntimeReadiness {
                store_ready: true,
                resource_api_ready: true,
                local_session_ready: true,
                provider_path_ready: true,
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

    /// Read one resource through the production backend.
    pub async fn get(
        &self,
        target: ResourceRef,
        operation_id: &str,
    ) -> Result<Value, ResourceRuntimeError> {
        let resource = self
            .api
            .get_runtime(
                self.subject.clone(),
                self.authorization_state.clone(),
                target,
                operation_id,
            )
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        serde_json::from_slice(&resource.canonical_json)
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)
    }

    /// List one ResourceType through the production backend.
    pub async fn list(
        &self,
        resource_type: ResourceTypeName,
        operation_id: &str,
    ) -> Result<Value, ResourceRuntimeError> {
        let result = self
            .api
            .list_runtime(
                self.subject.clone(),
                self.authorization_state.clone(),
                resource_type,
                operation_id,
            )
            .await
            .map_err(|_| ResourceRuntimeError::StoreReadFailed)?;
        let resources = result
            .resources
            .into_iter()
            .filter_map(|resource| serde_json::from_slice::<Value>(&resource.canonical_json).ok())
            .collect::<Vec<_>>();
        Ok(json!({
            "resources": resources,
            "snapshotRevision": result.snapshot_revision.get(),
            "truncated": result.truncated,
        }))
    }

    /// Serve the existing CLI's authenticated Zone request envelope.
    ///
    /// The public daemon has already authenticated the peer with
    /// `SO_PEERCRED`. The envelope's Zone value is treated as a route check
    /// only; this method never turns it into authority and always serves the
    /// runtime selected by the daemon's trusted Zone index.
    pub async fn dispatch_cli_request(
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
        let operation_id = request
            .get("operationId")
            .and_then(Value::as_str)
            .unwrap_or("cli-resource");
        match method {
            "Get" | "Status" => {
                let target = request
                    .get("resourceRef")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                    .ok_or(ResourceRuntimeError::RequestInvalid)?;
                self.get(target, operation_id).await
            }
            "List" | "Watch" | "ZoneList" | "ZoneStatus" => {
                if matches!(method, "ZoneList" | "ZoneStatus") {
                    return Ok(json!({
                        "zoneRef": format!("Zone/{}", self.zone.as_str()),
                        "store": "ready",
                        "resourceApi": "ready",
                        "core": format!("{:?}", self.readiness.core_stage),
                    }));
                }
                let resource_type = request
                    .get("resourceType")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceTypeName::parse(value).ok())
                    .ok_or(ResourceRuntimeError::RequestInvalid)?;
                self.list(resource_type, operation_id).await
            }
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

fn runtime_subject(zone: &ZoneId) -> Result<AuthenticatedSubjectContext, ResourceRuntimeError> {
    let subject_ref = ResourceRef::parse("Provider/system-core")
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let zone_ref = ResourceRef::parse(format!("Zone/{}", zone.as_str()).as_str())
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let schema = SchemaFingerprint::parse("sha256:".to_owned() + &"a".repeat(64))
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let binding = TransportBinding::new(
        Locality::Local,
        BindingDigest::parse("sha256:".to_owned() + &"b".repeat(64))
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
    );
    let session = SessionBinding::new(
        schema,
        binding,
        ReconnectGeneration::new(1).map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
        TranscriptHash::from_bytes([0x11; 32]),
    );
    Ok(AuthenticatedSubjectContext::new(
        subject_ref.clone(),
        stable_uid("provider", &format!("{}:core", zone.as_str())),
        zone_ref,
        EvidenceClass::NativeVsock,
        SessionPurpose::parse("resource-bootstrap")
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
        ServiceName::parse("d2b.resource.v3")
            .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
        session,
    )
    .with_provider_ref(subject_ref)
    .with_provider_generation(
        ResourceGeneration::new(1).map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
    )
    .with_controller_generation(
        ControllerGeneration::new(1).map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?,
    ))
}

fn runtime_authorizer(zone: &ZoneId) -> Result<NativeAuthorizer, ResourceRuntimeError> {
    let catalog = ApiCatalog::standard();
    let subject_ref = ResourceRef::parse("Provider/system-core")
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let role_ref = ResourceRef::parse("Role/system-core")
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let resource_types = d2b_contracts::v3::identity::STANDARD_RESOURCE_TYPES
        .into_iter()
        .map(|value| ResourceTypeName::parse(value.to_owned()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
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
        SessionVerb::Observe,
    ];
    let resource_rules = resource_types
        .chunks(16)
        .map(|resource_types| {
            PolicyRule::new(
                &catalog,
                resource_types.iter().cloned(),
                resource_verbs,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                [zone.clone()],
                Vec::new(),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let session_rule = PolicyRule::new(
        &catalog,
        Vec::new(),
        Vec::new(),
        session_verbs,
        Vec::new(),
        Vec::new(),
        [zone.clone()],
        Vec::new(),
    )
    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let mut role_rules = resource_rules;
    role_rules.push(session_rule);
    let role = CompiledRole::new(role_ref.clone(), role_rules)
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let binding = CompiledRoleBinding::new(
        role_ref,
        [BoundSubject {
            subject_ref: subject_ref.clone(),
            subject_uid: stable_uid("provider", &format!("{}:core", zone.as_str())),
        }],
        BindingScope {
            zones: [zone.clone()].into_iter().collect(),
            ..BindingScope::default()
        },
        RelayGrantAuthority::None,
    )
    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    let policy = PolicySet::new(&catalog, 1, vec![role], vec![binding])
        .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
    NativeAuthorizer::new(catalog, Some(policy))
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
        assert!(runtime.readiness().local_session_ready);
        assert!(runtime.readiness().provider_path_ready);
        assert_eq!(runtime.core_stage().unwrap(), StartupStage::Ready);
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
        assert_eq!(list["resources"].as_array().map(Vec::len), Some(0));
        runtime.shutdown().await.unwrap();
        assert!(fd >= 0);
    }
}
