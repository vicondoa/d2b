use std::{
    fs::File,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use d2b_contracts::{
    broker_wire::BrokerRequest,
    v2_component_session::{
        AttachmentDescriptor, EndpointRole, GuestSessionCredentialBytes, GuestSessionCredentialV1,
        Locality, RequestId,
    },
    v2_guest_configured_launches::{GuestConfiguredLaunchEntryV1, GuestConfiguredLaunchesV1},
    v2_identity::{RealmId, WorkloadId},
    v2_services::{
        SERVICE_INVENTORY, StrictWireMessage,
        common::{DesiredState, IdentityScope, ServiceRequest},
        service_schema_fingerprint,
    },
};
use d2b_core::configured_argv::ConfiguredArgv;
use d2b_priv_broker::{
    guest_session_material::{
        CONFIGURED_LAUNCH_CREDENTIAL_NAME, CONFIGURED_LAUNCH_STORAGE_PREFIX,
        GUEST_SESSION_CREDENTIAL_NAME, GuestAuthorityLookup, GuestMaterialAuditRecord,
        GuestMaterialAuditSink, GuestMaterialBundle, GuestMaterialBundlePort, GuestMaterialClock,
        GuestMaterialError, GuestMaterialRequestDigestInput, GuestMaterialStore,
        GuestMaterialTarget, GuestMaterialTransaction, GuestSessionAuthority,
        GuestSessionAuthorityPort, GuestSessionMaterialBroker, guest_material_request_digest,
    },
    runtime::serve_parent_spawned_realm_broker_with_ports,
    service_v2::{
        BrokerCallContext, BrokerMethod, BrokerPeerRole, BrokerReply, BrokerRuntimeDispatch,
        BrokerServiceFailure, broker_channel_binding, broker_endpoint_policy,
    },
};
use d2b_realm_core::ProtocolToken;
use d2b_session::{HandshakeCredentials, RequestRegistry, SessionEngine};
use d2b_session_unix::{
    CreditPool, CreditScopeSet, DescriptorPolicyResolver, PeerIdentityPolicy, SeqpacketSocket,
    UnixAttachmentPayload, UnixSeqpacketTransport, UnixSessionError,
};
use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
use protobuf::MessageField;
use sha2::{Digest, Sha256};

const REALM: &str = "work";
const WORKLOAD: &str = "editor";
const OPERATION: &str = "materialize-editor";
const GENERATION: u64 = 4;
const INVENTORY: &[u8] = br#"{"items":[{"argv":["private-integration-canary"]}]}"#;

#[test]
fn final_public_proxy_and_direct_guest_fingerprints_are_pinned() {
    let fingerprint = |package: &str| {
        let service = SERVICE_INVENTORY
            .iter()
            .find(|service| service.package == package)
            .unwrap();
        service_schema_fingerprint(service)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    assert_eq!(
        fingerprint("d2b.daemon.v2"),
        "4b2834c89162e5a2c17ea879052c066fd546cdc440d1473955a99e2d9521a54a"
    );
    assert_eq!(
        fingerprint("d2b.guest.v2"),
        "e6d2fd47db903deff84b5b9cb58a0aed17e2f6ef43010182925890878a15dd3d"
    );
}

#[test]
fn legacy_guest_control_sign_request_is_unknown_and_rejected() {
    let encoded = serde_json::json!({
        "kind": "GuestControlSign",
        "payload": {}
    });
    assert!(serde_json::from_value::<BrokerRequest>(encoded).is_err());
}

struct Authority;

#[async_trait]
impl GuestSessionAuthorityPort for Authority {
    async fn resolve(
        &self,
        _: GuestAuthorityLookup,
    ) -> Result<GuestSessionAuthority, GuestMaterialError> {
        Ok(GuestSessionAuthority {
            realm_id: REALM.to_owned(),
            workload_id: WORKLOAD.to_owned(),
            session_generation: GENERATION,
            parent_static_public_key: [0x11; 32],
            channel_binding: [0x22; 32],
            guest_identity_digest: [0x33; 32],
            guest_static_public_key: [0x44; 32],
            bootstrap: None,
        })
    }
}

struct RejectingFallback;

#[async_trait]
impl BrokerRuntimeDispatch for RejectingFallback {
    async fn dispatch(
        &self,
        _: BrokerMethod,
        _: ServiceRequest,
        _: Vec<d2b_session::OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<
        BrokerReply<d2b_contracts::v2_services::common::ServiceResponse>,
        BrokerServiceFailure,
    > {
        Err(BrokerServiceFailure::PermissionDenied)
    }
}

fn credit_scopes(limit: usize) -> CreditScopeSet {
    let pool = || CreditPool::new(limit).unwrap();
    CreditScopeSet::new(pool(), pool(), pool(), pool(), pool(), pool())
}

struct Bundle;

fn configured_launches() -> d2b_contracts::v2_guest_configured_launches::GuestConfiguredLaunchesBytes
{
    GuestConfiguredLaunchesV1::new(
        RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap(),
        WorkloadId::parse("bbbbbbbbbbbbbbbbbbba").unwrap(),
        Sha256::digest(INVENTORY).into(),
        vec![
            GuestConfiguredLaunchEntryV1::new(
                ProtocolToken::parse("editor").unwrap(),
                ConfiguredArgv::new(vec!["private-integration-canary".to_owned()]).unwrap(),
                false,
            )
            .unwrap(),
        ],
    )
    .unwrap()
    .encode()
    .unwrap()
}

impl GuestMaterialBundlePort for Bundle {
    fn resolve(
        &self,
        storage_ref: &str,
        _: &str,
        workload_id: &str,
    ) -> Result<GuestMaterialBundle, GuestMaterialError> {
        let root = PathBuf::from("integration-material");
        let configured_launches = configured_launches();
        let configured_launch_digest = configured_launches.sha256();
        Ok(GuestMaterialBundle {
            session_target: GuestMaterialTarget {
                storage_ref: storage_ref.to_owned(),
                path: root.join(GUEST_SESSION_CREDENTIAL_NAME),
                owner_uid: 0,
                owner_gid: 1,
                mode: 0o440,
            },
            configured_launch_target: GuestMaterialTarget {
                storage_ref: format!("{CONFIGURED_LAUNCH_STORAGE_PREFIX}{workload_id}"),
                path: root.join(CONFIGURED_LAUNCH_CREDENTIAL_NAME),
                owner_uid: 0,
                owner_gid: 1,
                mode: 0o440,
            },
            configured_launches,
            configured_launch_digest,
        })
    }
}

struct Store(Arc<AtomicBool>);

impl GuestMaterialStore for Store {
    fn stage_pair(
        &self,
        _: &GuestMaterialTarget,
        _: &GuestSessionCredentialBytes,
        _: &GuestMaterialTarget,
        _: &[u8],
    ) -> Result<Box<dyn GuestMaterialTransaction>, GuestMaterialError> {
        Ok(Box::new(StoreTransaction(Arc::clone(&self.0))))
    }
}

struct StoreTransaction(Arc<AtomicBool>);

impl GuestMaterialTransaction for StoreTransaction {
    fn commit(&mut self) -> Result<(), GuestMaterialError> {
        Ok(())
    }

    fn mark_committed(&mut self) -> Result<(), GuestMaterialError> {
        Ok(())
    }

    fn mark_audit_committed(&mut self) -> Result<(), GuestMaterialError> {
        Ok(())
    }

    fn finalize(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn rollback(&mut self) -> Result<(), GuestMaterialError> {
        self.0.store(false, Ordering::SeqCst);
        Ok(())
    }
}

struct Audit(AtomicBool);

impl GuestMaterialAuditSink for Audit {
    fn record(&self, _: &GuestMaterialAuditRecord) -> Result<(), GuestMaterialError> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct Clock;

impl GuestMaterialClock for Clock {
    fn now_unix_ms(&self) -> u64 {
        1
    }
}

fn context() -> BrokerCallContext {
    let request_id = RequestId::new(vec![0x55; 16]).unwrap();
    BrokerCallContext {
        peer_role: BrokerPeerRole::RealmController,
        request_id: request_id.clone(),
        session_generation: GENERATION,
        remaining: Duration::from_secs(1),
        cancellation: RequestRegistry::new(GENERATION)
            .unwrap()
            .register(request_id)
            .unwrap(),
    }
}

#[tokio::test]
async fn broker_output_round_trips_through_exact_guest_shared_codec() {
    let session_ref = d2b_host::guest_runtime::guest_material_resource_id(WORKLOAD);
    let request_digest = guest_material_request_digest(GuestMaterialRequestDigestInput {
        realm_id: REALM,
        workload_id: WORKLOAD,
        operation_id: OPERATION,
        session_storage_ref: &session_ref,
        session_generation: GENERATION,
    });
    let store = Arc::new(Store(Arc::new(AtomicBool::new(false))));
    let audit = Arc::new(Audit(AtomicBool::new(false)));
    let broker = GuestSessionMaterialBroker::new(
        Arc::new(Authority),
        Arc::new(Bundle),
        store.clone(),
        audit.clone(),
        Arc::new(Clock),
    );
    let reply = broker
        .apply(
            ServiceRequest {
                scope: MessageField::some(IdentityScope {
                    realm_id: REALM.to_owned(),
                    workload_id: WORKLOAD.to_owned(),
                    ..Default::default()
                }),
                resource_id: session_ref,
                operation_id: OPERATION.to_owned(),
                request_digest: request_digest.to_vec(),
                desired_state: DesiredState::DESIRED_STATE_PRESENT.into(),
                ..Default::default()
            },
            &context(),
        )
        .await
        .expect("guest material response");

    assert_eq!(reply.message.attachment_indexes, vec![0, 1]);
    reply.message.validate_wire(false).unwrap();
    assert_eq!(reply.attachments.len(), 2);
    let fd = reply.attachments[0]
        .payload()
        .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
        .and_then(UnixAttachmentPayload::file)
        .unwrap()
        .try_clone_to_owned()
        .unwrap();
    let mut session = Vec::new();
    File::from(fd).read_to_end(&mut session).unwrap();
    let credential = GuestSessionCredentialV1::decode(&session).unwrap();
    assert_eq!(credential.session_generation(), GENERATION);
    assert_eq!(credential.guest_identity_digest(), Some(&[0x33; 32]));
    assert_eq!(credential.encode().unwrap().as_slice(), session);
    let configured_fd = reply.attachments[1]
        .payload()
        .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
        .and_then(UnixAttachmentPayload::file)
        .unwrap()
        .try_clone_to_owned()
        .unwrap();
    let mut configured = Vec::new();
    File::from(configured_fd)
        .read_to_end(&mut configured)
        .unwrap();
    let catalog = GuestConfiguredLaunchesV1::decode(&configured).unwrap();
    assert_eq!(
        catalog.resolve_id("editor").unwrap().argv().as_slice(),
        &["private-integration-canary"]
    );
    assert!(store.0.load(Ordering::SeqCst));
    assert!(audit.0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn parent_spawned_realm_runtime_accepts_authenticated_child_endpoint() {
    let uid = rustix::process::getuid().as_raw();
    let gid = rustix::process::getgid().as_raw();
    if uid == 0 || gid == 0 {
        return;
    }
    let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-scratch/parent-spawned-realm-runtime");
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::set_permissions(&scratch, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ledger = scratch.join(format!("replay-{}.ledger", std::process::id()));
    let _ = std::fs::remove_file(&ledger);
    let authority = GuestSessionAuthority {
        realm_id: REALM.to_owned(),
        workload_id: WORKLOAD.to_owned(),
        session_generation: GENERATION,
        parent_static_public_key: [0x11; 32],
        channel_binding: [0x22; 32],
        guest_identity_digest: [0x33; 32],
        guest_static_public_key: [0x44; 32],
        bootstrap: None,
    };

    let (server_fd, client_fd) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
    )
    .unwrap();
    let server = tokio::spawn(serve_parent_spawned_realm_broker_with_ports(
        server_fd,
        REALM.to_owned(),
        uid,
        gid,
        GENERATION,
        ledger.clone(),
        uid,
        gid,
        Arc::new(Bundle),
        Arc::new(Store(Arc::new(AtomicBool::new(false)))),
        Arc::new(Audit(AtomicBool::new(false))),
        Arc::new(Clock),
        RejectingFallback,
        vec![authority],
    ));
    let socket = SeqpacketSocket::from_owned(client_fd).unwrap();
    let verifier = Arc::new(move |peer: &SeqpacketSocket| {
        let credentials = peer.acceptor_peer_credentials()?;
        if credentials.uid().as_raw() == uid && credentials.gid().as_raw() == gid {
            Ok(())
        } else {
            Err(UnixSessionError::CredentialMismatch)
        }
    });
    let policy = broker_endpoint_policy(
        BrokerPeerRole::RealmController,
        EndpointRole::RealmBroker,
        GENERATION,
        broker_channel_binding(uid, gid, EndpointRole::RealmBroker),
    )
    .unwrap();
    let descriptor_resolver: DescriptorPolicyResolver =
        Arc::new(|_: &AttachmentDescriptor| Err(UnixSessionError::DescriptorMismatch));
    let transport = UnixSeqpacketTransport::new(
        socket,
        Locality::HostLocal,
        policy.limits,
        policy.attachment_policy,
        credit_scopes(usize::from(policy.attachment_policy.max_per_session)),
        descriptor_resolver,
        PeerIdentityPolicy::pathname(verifier),
    )
    .unwrap();
    let client = SessionEngine::establish_initiator(
        transport,
        policy,
        HandshakeCredentials::Nn,
        Instant::now(),
    )
    .await
    .expect("realm broker handshake");
    drop(client);
    if let Ok(result) = tokio::time::timeout(Duration::from_secs(2), server).await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(BrokerServiceFailure::Backend)) => {}
            Ok(Err(error)) => panic!("realm server failed: {error}"),
            Err(error) => panic!("realm server task failed: {error}"),
        }
    }
    let _ = std::fs::remove_file(ledger);
}
