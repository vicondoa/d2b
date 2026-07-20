use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::fd::{AsRawFd, OwnedFd},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use d2b_contracts::{
    v2_component_session::{
        AttachmentAccess, AttachmentCreditClass, AttachmentDescriptor, AttachmentKind,
        AttachmentPurpose, BoundedVec, EndpointRole, GuestIdentityBindingV1,
        GuestSessionCredentialV1, KernelObjectType, Locality, RequestId, ServicePackage,
    },
    v2_guest_configured_launches::{GuestConfiguredLaunchEntryV1, GuestConfiguredLaunchesV1},
    v2_identity::{RealmId, WorkloadId},
    v2_services::{
        broker::{
            AllocateRequest, HostResourceKind, LeaseOwner, LeaseResourceRequest,
            ResourceAcquisitionOrder, ResourceShareMode,
        },
        common::{
            DesiredState, IdentityScope, Outcome, RequestMetadata, ServiceRequest, ServiceResponse,
        },
    },
};
use d2b_core::configured_argv::ConfiguredArgv;
use d2b_host::{
    guest_runtime::{
        GuestEnrollmentApplyDigestInput, GuestMaterialApplyDigestInput,
        controller_session_generation, guest_enrollment_apply_digest, guest_enrollment_resource_id,
        guest_material_apply_digest, guest_material_resource_id,
    },
    realm_broker_bootstrap::{
        RealmBrokerChildAuthority, RealmBrokerGuestRuntimeBootstrap,
        RealmBrokerGuestWorkloadBootstrap,
    },
};
use d2b_priv_broker::{
    guest_session_material::{
        BROKER_APPLY_METHOD_ID, CONFIGURED_LAUNCH_STORAGE_PREFIX, GUEST_SESSION_CREDENTIAL_NAME,
        GUEST_SESSION_STORAGE_PREFIX,
    },
    service_v2::{BrokerPeerRole, broker_channel_binding, broker_endpoint_policy},
};
use d2b_realm_core::ProtocolToken;
use d2b_session::{ComponentSessionDriver, HandshakeCredentials, OwnedAttachment, SessionEngine};
use d2b_session_unix::{
    CreditPool, CreditScopeSet, DescriptorPolicy, DescriptorPolicyResolver, ObjectIdentity,
    OwnedUnixAttachment, PeerIdentityPolicy, SeqpacketSocket, UnixAttachmentPayload,
    UnixSeqpacketTransport, UnixSessionError,
};
use nix::{
    fcntl::{FcntlArg, SealFlag, fcntl},
    sys::socket::{
        AddressFamily, Backlog, MsgFlags, SockFlag, SockType, UnixAddr, bind, connect, listen,
        recv, setsockopt, socket, socketpair, sockopt::PassCred,
    },
};
use protobuf::{Message, MessageField};
use sha2::{Digest, Sha256};
use ttrpc::proto::{MESSAGE_HEADER_LENGTH, MESSAGE_TYPE_RESPONSE, MessageHeader};

const BROKER_BIN: &str = env!("CARGO_BIN_EXE_d2b-priv-broker");
const REALM: &str = "aaaaaaaaaaaaaaaaaaaa";
const WORKLOAD: &str = "bbbbbbbbbbbbbbbbbbba";
const CONTROLLER_GENERATION: &str = "generation-1";
const PROCESS_ID: &str = "broker-1";

struct Scratch(tempfile::TempDir);

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Scratch {
    fn new() -> Self {
        let root = std::env::var_os("D2B_VALIDATION_SOCKET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        std::fs::create_dir_all(&root).expect("create child broker test socket root");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("harden child broker test socket root");
        Self(tempfile::tempdir_in(root).expect("create child broker test tempdir"))
    }

    fn path(&self) -> &Path {
        self.0.path()
    }
}

fn listener(path: &Path) -> OwnedFd {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .unwrap();
    bind(fd.as_raw_fd(), &UnixAddr::new(path).unwrap()).unwrap();
    listen(&fd, Backlog::new(8).unwrap()).unwrap();
    fd
}

fn authority_fd(uid: u32, gid: u32, generation: u64, guest_runtime_digest: [u8; 32]) -> OwnedFd {
    let authority = RealmBrokerChildAuthority {
        realm_id: REALM.to_owned(),
        controller_generation: CONTROLLER_GENERATION.to_owned(),
        broker_process_id: PROCESS_ID.to_owned(),
        session_generation: generation,
        controller_uid: uid,
        controller_gid: gid,
        broker_uid: uid,
        broker_gid: gid,
        cgroup_digest: [9; 32],
        guest_runtime_digest,
    };
    let encoded = authority.encode().unwrap();
    sealed_readonly_fd("realm-broker-authority-v1", &encoded)
}

fn sealed_readonly_fd(name: &str, encoded: &[u8]) -> OwnedFd {
    let fd = rustix::fs::memfd_create(
        name,
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .unwrap();
    let mut writer = File::from(fd);
    writer.write_all(encoded).unwrap();
    writer.seek(SeekFrom::Start(0)).unwrap();
    fcntl(
        writer.as_raw_fd(),
        FcntlArg::F_ADD_SEALS(
            SealFlag::F_SEAL_WRITE
                | SealFlag::F_SEAL_GROW
                | SealFlag::F_SEAL_SHRINK
                | SealFlag::F_SEAL_SEAL,
        ),
    )
    .unwrap();
    let readonly = rustix::fs::open(
        format!("/proc/self/fd/{}", writer.as_raw_fd()),
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .unwrap();
    drop(writer);
    readonly
}

fn guest_runtime_fd(scratch: &Scratch, generation: u64) -> (OwnedFd, [u8; 32]) {
    let material_dir = scratch.path().join("material");
    std::fs::create_dir(&material_dir).unwrap();
    std::fs::set_permissions(&material_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let configured = GuestConfiguredLaunchesV1::new(
        RealmId::parse(REALM).unwrap(),
        WorkloadId::parse(WORKLOAD).unwrap(),
        [0x31; 32],
        vec![
            GuestConfiguredLaunchEntryV1::new(
                ProtocolToken::parse("editor").unwrap(),
                ConfiguredArgv::new(vec!["private-canary".to_owned()]).unwrap(),
                false,
            )
            .unwrap(),
        ],
    )
    .unwrap()
    .encode()
    .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let runtime = RealmBrokerGuestRuntimeBootstrap {
        realm_id: REALM.to_owned(),
        session_generation: generation,
        replay_ledger_path: scratch
            .path()
            .join("replay.ledger")
            .to_str()
            .unwrap()
            .to_owned(),
        audit_log_path: scratch
            .path()
            .join("material.audit")
            .to_str()
            .unwrap()
            .to_owned(),
        workloads: vec![RealmBrokerGuestWorkloadBootstrap {
            workload_id: WORKLOAD.to_owned(),
            parent_static_public_key: [0x11; 32],
            channel_binding: [0x22; 32],
            bootstrap_operation_id: [0x33; 16],
            replay_nonce: [0x44; 32],
            issued_at_unix_ms: now,
            expires_at_unix_ms: now + 60_000,
            bootstrap_psk: [0x55; 32],
            session_storage_ref: format!("{GUEST_SESSION_STORAGE_PREFIX}{WORKLOAD}"),
            session_path: material_dir
                .join(GUEST_SESSION_CREDENTIAL_NAME)
                .to_str()
                .unwrap()
                .to_owned(),
            configured_storage_ref: format!("{CONFIGURED_LAUNCH_STORAGE_PREFIX}{WORKLOAD}"),
            configured_path: material_dir
                .join("d2b-configured-launch-v2")
                .to_str()
                .unwrap()
                .to_owned(),
            owner_uid: 0,
            owner_gid: 0,
            mode: 0o440,
            configured_launch_digest: configured.sha256(),
            configured_launches: configured.as_slice().to_vec(),
        }],
    };
    let encoded = runtime.encode().unwrap();
    let digest = Sha256::digest(&encoded).into();
    (
        sealed_readonly_fd("realm-broker-guest-runtime-v1", &encoded),
        digest,
    )
}

fn clear_cloexec(fd: &OwnedFd) {
    let flags =
        nix::fcntl::FdFlag::from_bits_truncate(fcntl(fd.as_raw_fd(), FcntlArg::F_GETFD).unwrap());
    fcntl(
        fd.as_raw_fd(),
        FcntlArg::F_SETFD(flags - nix::fcntl::FdFlag::FD_CLOEXEC),
    )
    .unwrap();
}

fn connect_seqpacket(path: &Path) -> OwnedFd {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
        None,
    )
    .unwrap();
    connect(fd.as_raw_fd(), &UnixAddr::new(path).unwrap()).unwrap();
    fd
}

fn credits(limit: u16) -> CreditScopeSet {
    let limit = usize::from(limit);
    let pool = || CreditPool::new(limit).unwrap();
    CreditScopeSet::new(pool(), pool(), pool(), pool(), pool(), pool())
}

fn metadata(request_id: &RequestId, generation: u64, digest: [u8; 32]) -> RequestMetadata {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    RequestMetadata {
        request_id: request_id.as_bytes().to_vec(),
        idempotency_key: digest.to_vec(),
        issued_at_unix_ms: now,
        expires_at_unix_ms: now + 5_000,
        session_generation: generation,
        ..Default::default()
    }
}

fn material_request(generation: u64, request_id: &RequestId) -> ServiceRequest {
    let operation_id = "child-process-material-apply";
    let resource_id = guest_material_resource_id(WORKLOAD);
    let digest = guest_material_apply_digest(GuestMaterialApplyDigestInput {
        realm_id: REALM,
        workload_id: WORKLOAD,
        operation_id,
        session_storage_ref: &resource_id,
        session_generation: generation,
    });
    ServiceRequest {
        metadata: MessageField::some(metadata(request_id, generation, digest)),
        scope: MessageField::some(IdentityScope {
            realm_id: REALM.to_owned(),
            workload_id: WORKLOAD.to_owned(),
            ..Default::default()
        }),
        resource_id,
        operation_id: operation_id.to_owned(),
        request_digest: digest.to_vec(),
        desired_state: DesiredState::DESIRED_STATE_PRESENT.into(),
        ..Default::default()
    }
}

fn enrollment_request(
    generation: u64,
    request_id: &RequestId,
    credential: &GuestSessionCredentialV1,
) -> ServiceRequest {
    let operation_id = "child-process-enrollment-persist";
    let resource_id = guest_enrollment_resource_id(WORKLOAD);
    let encoded = credential.encode().unwrap();
    let credential_digest: [u8; 32] = Sha256::digest(encoded.as_slice()).into();
    let digest = guest_enrollment_apply_digest(GuestEnrollmentApplyDigestInput {
        realm_id: REALM,
        workload_id: WORKLOAD,
        operation_id,
        enrollment_ref: &resource_id,
        session_generation: generation,
        credential_digest: &credential_digest,
    });
    ServiceRequest {
        metadata: MessageField::some(metadata(request_id, generation, digest)),
        scope: MessageField::some(IdentityScope {
            realm_id: REALM.to_owned(),
            workload_id: WORKLOAD.to_owned(),
            ..Default::default()
        }),
        resource_id,
        operation_id: operation_id.to_owned(),
        request_digest: digest.to_vec(),
        attachment_indexes: vec![0],
        desired_state: DesiredState::DESIRED_STATE_PRESENT.into(),
        ..Default::default()
    }
}

async fn rpc<M: Message>(
    driver: &Arc<dyn ComponentSessionDriver>,
    request_id: RequestId,
    stream_id: u32,
    method: &str,
    message: &M,
) -> ttrpc::Response {
    let rpc = ttrpc::Request {
        service: "d2b.broker.v2.BrokerService".to_owned(),
        method: method.to_owned(),
        payload: message.write_to_bytes().unwrap(),
        ..Default::default()
    };
    let body = rpc.write_to_bytes().unwrap();
    let mut frame = Vec::from(MessageHeader::new_request(stream_id, body.len() as u32));
    frame.extend_from_slice(&body);
    driver.start_ttrpc(request_id.clone(), frame).await.unwrap();
    let response_frame = driver.receive_ttrpc().await.unwrap();
    let response_header = MessageHeader::from(&response_frame[..MESSAGE_HEADER_LENGTH]);
    assert_eq!(response_header.type_, MESSAGE_TYPE_RESPONSE);
    assert_eq!(response_header.stream_id, stream_id);
    assert!(driver.complete_ttrpc(request_id).await.unwrap());
    ttrpc::Response::parse_from_bytes(&response_frame[MESSAGE_HEADER_LENGTH..]).unwrap()
}

fn status(response: &ttrpc::Response) -> ttrpc::Code {
    response.status.as_ref().unwrap().code.enum_value().unwrap()
}

fn service_response(response: &ttrpc::Response) -> ServiceResponse {
    assert_eq!(
        status(response),
        ttrpc::Code::OK,
        "{}",
        response.status.as_ref().unwrap().message
    );
    ServiceResponse::parse_from_bytes(&response.payload).unwrap()
}

fn read_attachment(attachment: &OwnedAttachment) -> Vec<u8> {
    let fd = attachment
        .payload()
        .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
        .and_then(UnixAttachmentPayload::file)
        .unwrap()
        .try_clone_to_owned()
        .unwrap();
    let mut bytes = Vec::new();
    File::from(fd).read_to_end(&mut bytes).unwrap();
    bytes
}

fn enrollment_attachment(
    request_id: RequestId,
    generation: u64,
    credential: &GuestSessionCredentialV1,
) -> OwnedAttachment {
    let encoded = credential.encode().unwrap();
    let fd = sealed_readonly_fd("guest-enrollment-v1", encoded.as_slice());
    let descriptor = AttachmentDescriptor {
        index: 0,
        kind: AttachmentKind::FileDescriptor,
        object_type: KernelObjectType::Memfd,
        access: AttachmentAccess::ReadOnly,
        purpose: AttachmentPurpose::RequestInput,
        service: ServicePackage::BrokerV2,
        method_id: BROKER_APPLY_METHOD_ID,
        request_id,
        operation_id: None,
        packet_sequence: 1,
        reconnect_generation: generation,
        duplicate_object_allowed: false,
        cloexec_required: true,
        credit_classes: BoundedVec::new(vec![
            AttachmentCreditClass::Packet,
            AttachmentCreditClass::Request,
            AttachmentCreditClass::Operation,
            AttachmentCreditClass::Session,
            AttachmentCreditClass::Process,
            AttachmentCreditClass::Host,
        ])
        .unwrap(),
    };
    let identity =
        ObjectIdentity::from_trusted(&fd, KernelObjectType::Memfd, AttachmentAccess::ReadOnly)
            .unwrap();
    OwnedUnixAttachment::file(descriptor, fd, DescriptorPolicy::File(identity)).unwrap()
}

fn allocate_request(generation: u64, request_id: &RequestId) -> AllocateRequest {
    let digest = [0x66; 32];
    let mut request = AllocateRequest::new();
    request.metadata = MessageField::some(metadata(request_id, generation, digest));
    request.scope = MessageField::some(IdentityScope {
        realm_id: REALM.to_owned(),
        ..Default::default()
    });
    request.operation_id = "forbidden-child-allocator-op".to_owned();
    request.request_digest = digest.to_vec();
    request.owner = MessageField::some(LeaseOwner {
        realm_path: "work".to_owned(),
        controller_generation_id: CONTROLLER_GENERATION.to_owned(),
        ..Default::default()
    });
    for (resource_id, kind, ordinal) in [
        (
            "namespace-1",
            HostResourceKind::HOST_RESOURCE_KIND_NAMESPACE_BOUNDARY,
            0,
        ),
        ("bridge-1", HostResourceKind::HOST_RESOURCE_KIND_BRIDGE, 1),
    ] {
        request.resources.push(LeaseResourceRequest {
            resource_id: resource_id.to_owned(),
            kind: kind.into(),
            share: ResourceShareMode::RESOURCE_SHARE_MODE_EXCLUSIVE.into(),
            acquisition_order: MessageField::some(ResourceAcquisitionOrder {
                phase: 1,
                ordinal,
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    request
}

fn unprivileged_user_namespace_available() -> bool {
    match Command::new("unshare")
        .args(["--user", "--map-root-user", "--", "true"])
        .output()
    {
        Ok(output) if output.status.success() => true,
        Ok(output)
            if String::from_utf8_lossy(&output.stderr).contains("Operation not permitted")
                && String::from_utf8_lossy(&output.stderr).contains("uid_map") =>
        {
            eprintln!(
                "skipping child-realm process test: user namespace unavailable: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            false
        }
        Ok(output) => panic!(
            "unprivileged user namespace probe failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping child-realm process test: unshare unavailable");
            false
        }
        Err(error) => panic!("failed to probe unprivileged user namespaces: {error}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn parent_spawned_child_broker_adopts_fds_and_serves_realm_broker_v2() {
    let uid = rustix::process::getuid().as_raw();
    let gid = rustix::process::getgid().as_raw();
    if uid == 0 || gid == 0 {
        return;
    }
    if !unprivileged_user_namespace_available() {
        return;
    }
    let scratch = Scratch::new();
    let socket_path = scratch.path().join("broker.sock");
    let listener = listener(&socket_path);
    let (bootstrap_parent, bootstrap_child) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .unwrap();
    setsockopt(&bootstrap_parent, PassCred, &true).unwrap();
    let cgroup: OwnedFd = File::open("/sys/fs/cgroup").unwrap().into();
    let generation = controller_session_generation(REALM, CONTROLLER_GENERATION);
    let (guest_runtime, guest_runtime_digest) = guest_runtime_fd(&scratch, generation);
    let authority = authority_fd(uid, gid, generation, guest_runtime_digest);
    for fd in [
        &listener,
        &bootstrap_child,
        &cgroup,
        &authority,
        &guest_runtime,
    ] {
        clear_cloexec(fd);
    }
    let child = Command::new("unshare")
        .args([
            "--user",
            "--map-root-user",
            "--",
            BROKER_BIN,
            "serve-child-realm",
        ])
        .env_clear()
        .env("PATH", "/run/current-system/sw/bin:/usr/bin:/bin")
        .env("D2B_BROKER_LISTENER_FD", listener.as_raw_fd().to_string())
        .env(
            "D2B_BOOTSTRAP_SESSION_FD",
            bootstrap_child.as_raw_fd().to_string(),
        )
        .env("D2B_CGROUP_LEAF_FD", cgroup.as_raw_fd().to_string())
        .env(
            "D2B_REALM_BROKER_AUTHORITY_FD",
            authority.as_raw_fd().to_string(),
        )
        .env(
            "D2B_REALM_BROKER_GUEST_RUNTIME_FD",
            guest_runtime.as_raw_fd().to_string(),
        )
        .env("D2B_RESOURCE_FD_0", authority.as_raw_fd().to_string())
        .env("D2B_RESOURCE_FD_0_ID", "realm-broker-authority-v1")
        .env("D2B_RESOURCE_FD_1", guest_runtime.as_raw_fd().to_string())
        .env("D2B_RESOURCE_FD_1_ID", "realm-broker-guest-runtime-v1")
        .env("D2B_REALM_ID", REALM)
        .env("D2B_CONTROLLER_GENERATION", CONTROLLER_GENERATION)
        .env("D2B_CONTROLLER_SESSION_GENERATION", generation.to_string())
        .env("D2B_PROCESS_ID", PROCESS_ID)
        .env("D2B_CHILD_ROLE", "broker")
        .env(
            "D2B_CGROUP_DIGEST",
            "0909090909090909090909090909090909090909090909090909090909090909",
        )
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    drop(bootstrap_child);

    let ready = tokio::task::spawn_blocking(move || {
        let mut bytes = [0_u8; 16];
        recv(bootstrap_parent.as_raw_fd(), &mut bytes, MsgFlags::empty())
            .map(|count| bytes[..count].to_vec())
    });
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), ready)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        b"ready"
    );

    let socket = SeqpacketSocket::from_owned(connect_seqpacket(&socket_path)).unwrap();
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
        generation,
        broker_channel_binding(0, 0, EndpointRole::RealmBroker),
    )
    .unwrap();
    let descriptor_resolver: DescriptorPolicyResolver = Arc::new(|descriptor| {
        if descriptor.service == ServicePackage::BrokerV2
            && descriptor.method_id == BROKER_APPLY_METHOD_ID
            && descriptor.kind == AttachmentKind::FileDescriptor
            && descriptor.object_type == KernelObjectType::Memfd
            && descriptor.access == AttachmentAccess::ReadOnly
            && descriptor.purpose == AttachmentPurpose::ResponseOutput
            && descriptor.index <= 1
            && descriptor.cloexec_required
        {
            Ok(DescriptorPolicy::SealedReadOnlyMemfd)
        } else {
            Err(UnixSessionError::DescriptorMismatch)
        }
    });
    let transport = UnixSeqpacketTransport::new(
        socket,
        Locality::HostLocal,
        policy.limits,
        policy.attachment_policy,
        credits(policy.attachment_policy.max_per_session),
        descriptor_resolver,
        PeerIdentityPolicy::pathname(verifier),
    )
    .unwrap();
    let session = SessionEngine::establish_initiator(
        transport,
        policy,
        HandshakeCredentials::Nn,
        Instant::now(),
    )
    .await
    .unwrap();
    assert_eq!(session.generation(), generation);
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(session.into_driver());

    let material_request_id = RequestId::new(vec![0x41; 16]).unwrap();
    let response = rpc(
        &driver,
        material_request_id.clone(),
        1,
        "Apply",
        &material_request(generation, &material_request_id),
    )
    .await;
    let material_response = service_response(&response);
    assert_eq!(
        material_response.outcome.enum_value().unwrap(),
        Outcome::OUTCOME_SUCCEEDED
    );
    let attachments = driver.receive_attachments().await.unwrap();
    assert_eq!(attachments.len(), 2);
    let initial = GuestSessionCredentialV1::decode(&read_attachment(&attachments[0])).unwrap();
    assert!(initial.guest_identity_is_unbound());
    assert!(initial.bootstrap().is_some());
    let configured = GuestConfiguredLaunchesV1::decode(&read_attachment(&attachments[1])).unwrap();
    assert_eq!(configured.realm_id().as_str(), REALM);
    assert_eq!(configured.workload_id().as_str(), WORKLOAD);

    let guest_public = [0x77; 32];
    let enrolled = GuestSessionCredentialV1::new(
        generation,
        *initial.parent_static_public_key(),
        *initial.channel_binding(),
        GuestIdentityBindingV1::Enrolled {
            guest_identity_digest: Sha256::digest(guest_public).into(),
            guest_static_public_key: guest_public,
        },
        None,
    )
    .unwrap();
    let enrollment_request_id = RequestId::new(vec![0x42; 16]).unwrap();
    driver
        .send_attachments(vec![enrollment_attachment(
            enrollment_request_id.clone(),
            generation,
            &enrolled,
        )])
        .await
        .unwrap();
    let response = rpc(
        &driver,
        enrollment_request_id.clone(),
        3,
        "Apply",
        &enrollment_request(generation, &enrollment_request_id, &enrolled),
    )
    .await;
    let enrollment_response = service_response(&response);
    let enrolled_bytes = enrolled.encode().unwrap();
    assert_eq!(
        enrollment_response.result_digest,
        Sha256::digest(enrolled_bytes.as_slice()).as_slice()
    );

    let allocator_request_id = RequestId::new(vec![0x43; 16]).unwrap();
    let response = rpc(
        &driver,
        allocator_request_id.clone(),
        5,
        "Allocate",
        &allocate_request(generation, &allocator_request_id),
    )
    .await;
    assert_eq!(status(&response), ttrpc::Code::PERMISSION_DENIED);

    let persisted = GuestSessionCredentialV1::decode(
        &std::fs::read(
            scratch
                .path()
                .join("material")
                .join(GUEST_SESSION_CREDENTIAL_NAME),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(persisted.guest_static_public_key(), Some(&guest_public));
    assert!(
        std::fs::metadata(scratch.path().join("replay.ledger"))
            .unwrap()
            .len()
            > 0
    );
    assert!(
        std::fs::metadata(scratch.path().join("material.audit"))
            .unwrap()
            .len()
            >= 2 * (8 + 1 + 8 + 32 * 4)
    );
    driver
        .close(
            d2b_contracts::v2_component_session::CloseReason::Normal,
            d2b_contracts::v2_component_session::Remediation::None,
        )
        .await
        .unwrap();

    child.0.kill().unwrap();
    child.0.wait().unwrap();
}
