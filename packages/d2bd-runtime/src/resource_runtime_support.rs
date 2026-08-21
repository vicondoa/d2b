//! Provider-independent resource-plane construction and materialization helpers.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Read},
    os::unix::fs::FileTypeExt,
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::resource_api::{ParsedListRequest, ResourceRuntimeError};
use d2b_bus::{BusIngress, ZoneRegistrar};
use d2b_contracts_resource::resource_proto as wire;
use d2b_contracts_resource::v3::identity::STANDARD_RESOURCE_TYPES;
use d2b_contracts_zone_session::v3::{
    component_session::{AttachmentPolicy, EndpointPolicy, EndpointPurpose, EndpointRole, IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, PurposeClass, ServicePackage, TransportBinding as ComponentTransportBinding, TransportClass},
    resource_bundle::ResourceBundle,
};
use d2b_contracts_resource::v3::{
    CanonicalJsonValue,
    ConfigurationGeneration,
    ControllerGeneration,
    MAX_PAGE_CURSOR_BYTES,
    MAX_RESPONSE_CANONICAL_BYTES,
    ResourceEnvelope,
    ResourceError,
    ResourceErrorKind,
    ResourceErrorReason,
    ResourceName,
    ResourcePhase,
    ResourceRef,
    ResourceTypeName,
    ResourceUid,
    RetryClass,
    SchemaFingerprint,
    Timestamp,
    ZoneId,
    ZoneRevision,
    host::HOST_PROVIDER_REF,
};
use d2b_contracts_resource::v3::identity::{
    AuthenticatedSubjectContext,
    BindingDigest,
    EvidenceClass,
    Locality as IdentityLocality,
    ReconnectGeneration,
    ServiceName,
    SessionBinding,
    SessionPurpose,
    TranscriptHash,
    TransportBinding,
};
use d2b_core_controller::{
    controller_assignment::ControllerAssignmentRegistry,
    authority::HostGlobalAuthorityIndex,
    controllers::{CoreHandlerKind, HandlerOutcome, HandlerPhase, HandlerStatus},
    main::{
        CoreProcess, RecoverySnapshot, RuntimeReadiness as CoreRuntimeReadiness, StartupError,
        StartupStage,
    },
    zone_status::ZoneRuntimeMetadata,
};

/// Provider-neutral Core assignment registry shared by Resource API and bus
/// admission for one Zone runtime.
pub type AssignmentRegistry = Arc<Mutex<ControllerAssignmentRegistry>>;

/// Construct one empty Zone assignment registry.
pub fn new_assignment_registry() -> AssignmentRegistry {
    Arc::new(Mutex::new(ControllerAssignmentRegistry::default()))
}
use d2b_resource_api::{
    RedbBackend, ResourceApiClient, ResourceBusAdapter, ResourceService,
    authz::{
        ApiCatalog, AuthorizationState, BindingScope, BootstrapPhase, BoundSubject, CompiledRole,
        CompiledRoleBinding, NativeAuthorizer, PolicyRule, PolicySet, RelayGrantAuthority,
        ResourceVerb, SessionVerb,
    },
    service::UnavailableUpgradeDispatcher,
};
use d2b_resource_store::{
    PolicySnapshot, StoreListRequest, StoreListResult, StoreOperationContext, StoreProjection,
    StoreSlot, StoredResource,
};
use d2b_resource_store_redb::{RedbResourceStore, StoreIdentity, StoreRuntimeMetadata};
use d2b_session::{
    HandshakeCredentials, SessionEngine, SessionServerError, TransportEvidence,
    serve_ttrpc_services,
};
use d2b_session_unix::{
    CreditPool, CreditScopeSet, DescriptorPolicyResolver, PeerIdentityPolicy, SeqpacketSocket,
    UnixSeqpacketTransport, UnixSessionError, VerifiedUnixPeer, prearmed_seqpacket_pair,
};
use protobuf;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const OPERATOR_SUBJECT_REF: &str = "User/d2bd-operator";
const OPERATOR_SUBJECT_UID: &str = "22222222-2222-4222-8222-222222222222";

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
    pub const fn is_ready(self) -> bool {
        self.store_ready
            && self.resource_api_ready
            && self.local_session_ready
            && self.provider_path_ready
            && self.authority_ready
            && matches!(self.core_stage, StartupStage::Ready)
    }
}

pub fn read_bounded(path: impl AsRef<Path>, limit: usize) -> io::Result<String> {
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

pub fn is_socket(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false)
}

pub fn mark_core_handlers(
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

pub fn operator_subject_identity() -> (ResourceRef, ResourceUid) {
    (
        ResourceRef::parse(OPERATOR_SUBJECT_REF).expect("stable operator subject ref"),
        ResourceUid::parse(OPERATOR_SUBJECT_UID).expect("stable operator subject uid"),
    )
}

pub fn local_operator_subject_context(
    zone: &ZoneId,
    peer_uid: u32,
    operation_id: &str,
) -> Result<AuthenticatedSubjectContext, ResourceRuntimeError> {
    let (subject_ref, subject_uid) = operator_subject_identity();
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

pub fn drive_core_startup(
    core: &mut CoreProcess,
    readiness: CoreRuntimeReadiness,
    recovery: RecoverySnapshot,
    authority_index: &HostGlobalAuthorityIndex,
) -> Result<StartupStage, ResourceRuntimeError> {
    core.start_production(readiness, recovery, authority_index)
        .map_err(map_startup_error)?;
    core.publish_readiness().map_err(map_startup_error)
}

pub fn host_phase_for_resource_count(count: usize) -> HandlerPhase {
    if count == 0 {
        HandlerPhase::Degraded
    } else {
        HandlerPhase::Ready
    }
}

pub fn map_startup_error(error: StartupError) -> ResourceRuntimeError {
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

pub fn runtime_policy(
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

pub fn system_core_endpoint_policy() -> EndpointPolicy {
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
            kind: d2b_contracts_zone_session::v3::component_session::AttachmentPolicyKind::PacketAtomic,
            max_per_packet: 1,
            max_per_request: 1,
            max_per_operation: 1,
            max_per_session: 1,
            credentials_allowed: false,
        },
    }
}

pub fn unix_transport(
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

pub async fn register_system_core_session(
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
                d2b_contracts_resource::v3::identity::EvidenceClass::UnixPeer,
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
pub struct SystemCoreReconcileResult {
    pub core_phase: ResourcePhase,
    pub host_phase: HandlerPhase,
    pub user_phase: HandlerPhase,
    pub total_resource_count: u32,
    pub generation_cleanup_pending: bool,
    pub cleanup_pending_count: u32,
}

pub fn watch_needs_restart(slot: &mut Option<tokio::task::JoinHandle<()>>) -> bool {
    if slot.as_ref().is_some_and(|task| task.is_finished()) {
        *slot = None;
    }
    slot.is_none()
}

pub fn zone_runtime_metadata(
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

pub fn current_status_timestamp() -> Timestamp {
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

pub fn handler_phase_to_zone_phase(
    phase: HandlerPhase,
) -> d2b_contracts_zone_session::v3::ZoneHandlerPhase {
    match phase {
        HandlerPhase::Ready => d2b_contracts_zone_session::v3::ZoneHandlerPhase::Ready,
        HandlerPhase::Degraded => d2b_contracts_zone_session::v3::ZoneHandlerPhase::Degraded,
        HandlerPhase::Failed => d2b_contracts_zone_session::v3::ZoneHandlerPhase::Failed,
        HandlerPhase::Unknown => d2b_contracts_zone_session::v3::ZoneHandlerPhase::Unknown,
        HandlerPhase::Pending | HandlerPhase::Recovering => {
            d2b_contracts_zone_session::v3::ZoneHandlerPhase::Pending
        }
    }
}

pub fn runtime_authorizer(
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

pub fn initial_policy_snapshot() -> Result<PolicySnapshot, ResourceRuntimeError> {
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

pub async fn ensure_bootstrap_host_resource(
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
    body.payload_digest = d2b_contracts_resource::v3::canonical_digest(
        d2b_contracts_resource::v3::RESOURCE_ENVELOPE_DOMAIN_TAG,
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
pub async fn materialize_zone_resource_bundle(
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

pub fn resource_bundle_materialization_operation_id(
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
    resource: &d2b_contracts_zone_session::v3::resource_bundle::BundleResource,
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
    resource: &d2b_contracts_zone_session::v3::resource_bundle::BundleResource,
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
    resource: &d2b_contracts_zone_session::v3::resource_bundle::BundleResource,
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
        d2b_contracts_resource::v3::canonical_digest(
            d2b_contracts_resource::v3::RESOURCE_ENVELOPE_DOMAIN_TAG,
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

pub fn store_identity(
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

pub fn stable_uid(domain: &str, value: &str) -> ResourceUid {
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

pub fn resource_result_error(reason: &'static str) -> ResourceError {
    ResourceError::terminal(ResourceErrorKind::InternalIntegrityFailure, reason)
}

pub fn decode_resource_result(bytes: &[u8]) -> Result<Value, ResourceError> {
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

pub fn encode_list_result(result: StoreListResult) -> Result<Value, ResourceError> {
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

pub fn resource_error_envelope(error: &ResourceError) -> Value {
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

pub fn public_operation_id(request: &Value, peer_uid: u32, method: &str) -> String {
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

pub fn compatibility_error_envelope(error: ResourceRuntimeError) -> Value {
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

pub fn public_request_meta(operation_id: &str) -> wire::RequestMeta {
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = operation_id.to_owned();
    meta.idempotency_key = operation_id.to_owned();
    meta.correlation_id = operation_id.to_owned();
    meta.trace_id = operation_id.to_owned();
    meta.deadline_ms = 30_000;
    meta
}

pub fn public_list_request(parsed: ParsedListRequest, operation_id: &str) -> wire::ListRequest {
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

pub fn encode_public_resource(
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

pub fn public_api_error(error: &wire::ResourceError) -> Value {
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
    let retry_after_ms = error.retry_after_ms.filter(|delay| {
        (1..=d2b_contracts_resource::v3::MAX_RESOURCE_ERROR_RETRY_AFTER_MS).contains(delay)
    });
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

pub fn resource_error_kind_from_wire(kind: Option<wire::ResourceErrorKind>) -> ResourceErrorKind {
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

pub fn retry_class_from_wire(retry_class: Option<wire::RetryClass>) -> RetryClass {
    match retry_class {
        Some(wire::RetryClass::RETRY_CLASS_IMMEDIATE) => RetryClass::Immediate,
        Some(wire::RetryClass::RETRY_CLASS_AFTER_DELAY) => RetryClass::AfterDelay,
        Some(wire::RetryClass::RETRY_CLASS_REAUTHORIZE) => RetryClass::Reauthorize,
        Some(wire::RetryClass::RETRY_CLASS_NEVER)
        | Some(wire::RetryClass::RETRY_CLASS_UNSPECIFIED)
        | None => RetryClass::Never,
    }
}

pub fn encode_public_get_response(
    response: wire::GetResponse,
) -> Result<Value, ResourceRuntimeError> {
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

pub fn encode_public_list_response(
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

pub fn configuration_cleanup_pending(
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

pub async fn persist_resource_status(
    client: &ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>,
    resource: &StoredResource,
    status: &serde_json::Value,
) -> Result<(), ResourceRuntimeError> {
    persist_resource_status_with_projection(client, resource, status, None).await
}

pub async fn persist_resource_status_with_projection(
    client: &ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>,
    resource: &StoredResource,
    status: &serde_json::Value,
    resource_projection: Option<&serde_json::Value>,
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
    let resource_projection = select_resource_projection(resource_status, resource_projection)?;
    status.insert("resource".to_owned(), resource_projection);
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
        return Err(ResourceRuntimeError::ResourceStatusUpdateFailed(
            resource_error_kind_from_wire(error.kind.enum_value().ok()),
        ));
    }
    if response.resource.is_none() {
        return Err(ResourceRuntimeError::StoreReadFailed);
    }
    Ok(())
}

fn select_resource_projection(
    resource_status: BTreeMap<String, CanonicalJsonValue>,
    resource_projection: Option<&serde_json::Value>,
) -> Result<CanonicalJsonValue, ResourceRuntimeError> {
    match resource_projection {
        Some(resource_projection) => {
            let bytes = serde_json::to_vec(resource_projection)
                .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
            CanonicalJsonValue::parse(&bytes).map_err(|_| ResourceRuntimeError::HandlerNotReady)
        }
        None => Ok(CanonicalJsonValue::Object(resource_status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_api::parse_list_request;
    use d2b_contracts_resource::v3::ResourceGeneration;

    #[test]
    fn phase_only_status_preserves_existing_resource_projection() {
        let desired = CanonicalJsonValue::parse(br#"{"phase":"Ready"}"#).unwrap();
        let CanonicalJsonValue::Object(desired) = desired else {
            panic!("phase status must be an object");
        };
        let current = json!({
            "netVmRef": "Guest/net-work-net",
            "lanBridge": {"phase": "Ready"},
            "uplinkBridge": {"phase": "Ready"},
            "externalAttachment": null,
            "attachments": [],
        });
        let projection = select_resource_projection(desired, Some(&current)).unwrap();
        assert_eq!(
            projection.to_canonical_bytes(),
            CanonicalJsonValue::parse(
                br#"{"attachments":[],"externalAttachment":null,"lanBridge":{"phase":"Ready"},"netVmRef":"Guest/net-work-net","uplinkBridge":{"phase":"Ready"}}"#,
            )
            .unwrap()
            .to_canonical_bytes()
        );
        assert_eq!(
            select_resource_projection(
                CanonicalJsonValue::parse(br#"{"phase":"Pending"}"#)
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
                None,
            )
            .unwrap()
            .to_canonical_bytes(),
            br#"{"phase":"Pending"}"#
        );
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
            d2b_contracts_resource::v3::ResourceErrorReason::parse("revision-changed").unwrap(),
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
}
