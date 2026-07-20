use std::{
    collections::BTreeMap,
    fs::File,
    future::Future,
    io::{Read, Seek, SeekFrom, Write},
    os::fd::{AsRawFd, OwnedFd},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        AttachmentAccess, AttachmentCreditClass, AttachmentDescriptor, AttachmentKind,
        AttachmentPolicy, AttachmentPolicyKind, BoundedVec, EndpointPolicy, EndpointPurpose,
        EndpointRole, GuestSessionCredentialV1, IdentityEvidenceRequirement, KernelObjectType,
        LimitProfile, Locality, MAX_REQUEST_LIFETIME_MS, NoiseProfile, PurposeClass, Remediation,
        RequestId, ServicePackage, TransportBinding, TransportClass,
    },
    v2_guest_configured_launches::GuestConfiguredLaunchesV1,
    v2_identity::{RealmId, RealmPath, WorkloadId, WorkloadName},
    v2_services::{
        SERVICE_INVENTORY, StrictWireMessage,
        broker_ttrpc::BrokerServiceClient,
        common::{self, DesiredState, IdentityScope, Outcome, ServiceRequest},
        decode_strict,
        guest::{
            GuestArtifactId, GuestBootstrapRequest, GuestFileTransferDirection,
            GuestFileTransferRequest, GuestOperationContext, GuestReconnectRequest,
            GuestSessionResponse,
        },
        guest_contract::{
            validate_guest_session_response_for_bootstrap,
            validate_guest_session_response_for_reconnect,
        },
        guest_ttrpc::GuestServiceClient,
        service_schema_fingerprint,
    },
};
use d2b_core::{
    bundle_resolver::BundleResolver, processes::ProcessRole,
    realm_controller_config::RealmControllerPlacement, workload_identity::WorkloadIdentity,
};
use d2b_host::guest_runtime::{
    GUEST_V2_VSOCK_PORT, GuestEnrollmentApplyDigestInput, GuestMaterialApplyDigestInput,
    GuestRuntimeChannelBindingInput, controller_session_generation, guest_enrollment_apply_digest,
    guest_enrollment_resource_id, guest_material_apply_digest, guest_material_resource_id,
    guest_runtime_channel_binding,
};
use d2b_session::{
    BootstrapAdmission, BootstrapPsk, ComponentSessionDriver, HandshakeCredentials,
    OwnedAttachment, OwnedTransport, SessionEngine, TransportDescriptor, TransportError,
    TransportPacket,
};
use d2b_session_unix::{
    CreditPool, CreditScopeSet, DescriptorPolicy, DescriptorPolicyResolver, NativeVsockListener,
    NativeVsockTransport, ObjectIdentity, OwnedUnixAttachment, PeerIdentityPolicy, SeqpacketSocket,
    UnixAttachmentPayload, UnixSeqpacketTransport, UnixSessionError,
};
use nix::{
    fcntl::{FcntlArg, SealFlag, fcntl},
    sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket},
    unistd::{Group, User},
};
use protobuf::{EnumOrUnknown, Message, MessageField};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use ttrpc::{
    r#async::transport::Socket,
    proto::{MESSAGE_HEADER_LENGTH, MESSAGE_TYPE_REQUEST, MESSAGE_TYPE_RESPONSE, MessageHeader},
};
use zeroize::{Zeroize, Zeroizing};

use crate::{
    VM_RUNNER_ROLE_ID,
    controller_static_identity::{
        ControllerIdentityAuthority, ControllerProcessBinding, ControllerStaticIdentity,
    },
    daemon_terminal::TerminalFailure,
    production_guest_terminal::{
        AppliedGuestSessionMaterial, BootstrapGuestSession, DirectGuestSessionPort,
        GuestAuthorityPort, GuestSessionAuthority, ProductionGuestTerminalConnector,
        RealmGuestMaterialPort,
    },
    supervisor::pidfd_table::PidfdTable,
};

const BROKER_DEADLINE: Duration = Duration::from_secs(5);
const GUEST_DEADLINE: Duration = Duration::from_secs(10);
const ACTIVATION_CALL_DEADLINE: Duration = Duration::from_secs(15);
const ACTIVATION_PAYLOAD_SCHEMA_VERSION: u32 = 1;
const ACTIVATION_PAYLOAD_MAGIC: [u8; 8] = *b"D2BACT2\0";
const MAX_MATERIAL_BYTES: u64 = 16 * 1024;
const BOOTSTRAP_EVIDENCE_MAGIC: &[u8; 8] = b"D2BBEV2\0";
const BROKER_APPLY_METHOD: &str = "Apply";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectGuestActivationMode {
    Switch,
    Test,
}

#[derive(Clone)]
pub(crate) struct DirectGuestActivationStart {
    pub workload: String,
    pub operation_id: String,
    pub switch_script_path: String,
    pub mode: DirectGuestActivationMode,
    pub timeout_ms: u64,
}

impl std::fmt::Debug for DirectGuestActivationStart {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectGuestActivationStart")
            .field("workload", &"<redacted>")
            .field("operation_id", &"<redacted>")
            .field("switch_script_path", &"<redacted>")
            .field("mode", &self.mode)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectGuestActivationState {
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirectGuestActivationStatus {
    pub state: DirectGuestActivationState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectGuestActivationError {
    Unavailable,
    Unauthorized,
    NotFound,
    Conflict,
    Deadline,
    Protocol,
}

pub(crate) fn local_controller_generation(bundle_path: &std::path::Path) -> Option<u64> {
    let resolver = BundleResolver::load(bundle_path).ok()?;
    let bundle_hash = resolver.bundle.bundle_hash.as_deref()?;
    Some(controller_session_generation("local-root", bundle_hash))
}

pub(crate) struct BundleGuestAuthorityPort {
    bundle_path: PathBuf,
    controller: ControllerProcessBinding,
    pidfd_table: Arc<PidfdTable>,
}

impl BundleGuestAuthorityPort {
    pub(crate) fn new(
        bundle_path: PathBuf,
        controller: ControllerProcessBinding,
        pidfd_table: Arc<PidfdTable>,
    ) -> Arc<Self> {
        Arc::new(Self {
            bundle_path,
            controller,
            pidfd_table,
        })
    }

    fn resolve_verified(&self, workload: &str) -> Result<GuestSessionAuthority, TerminalFailure> {
        let resolver =
            BundleResolver::load(&self.bundle_path).map_err(|_| TerminalFailure::Unavailable)?;
        let catalog = resolver
            .realm_workloads_launcher_v2
            .as_ref()
            .ok_or(TerminalFailure::Unavailable)?;
        let (summary, realm_id, workload_id) = catalog
            .workloads
            .iter()
            .filter_map(|summary| {
                let (realm_id, workload_id) = v2_identity(&summary.identity).ok()?;
                let matches = workload == summary.identity.workload_id.as_str()
                    || workload == summary.identity.canonical_target.to_canonical()
                    || workload == workload_id.as_str()
                    || summary
                        .identity
                        .legacy_vm_name
                        .as_ref()
                        .is_some_and(|vm| vm.as_str() == workload);
                matches.then_some((summary, realm_id, workload_id))
            })
            .next()
            .ok_or(TerminalFailure::Protocol)?;
        if summary.provider_kind != d2b_realm_core::WorkloadProviderKind::LocalVm {
            return Err(TerminalFailure::InvalidSelection);
        }
        let controllers = resolver
            .realm_controllers
            .as_ref()
            .ok_or(TerminalFailure::Unavailable)?;
        let controller = controllers
            .controllers
            .iter()
            .find(|candidate| {
                candidate.realm_id.as_str() == self.controller.realm_id()
                    && candidate.realm_path.as_str() == summary.identity.realm_path.target_form()
                    && candidate.broker.enabled
                    && candidate.placement == RealmControllerPlacement::HostLocal
            })
            .ok_or(TerminalFailure::Unavailable)?;
        if controller.realm_id.as_str() != realm_id.as_str()
            || controller.broker.socket_path.as_str() == d2b_contracts::BROKER_SOCKET_PATH
        {
            return Err(TerminalFailure::GenerationMismatch);
        }
        let local_workload = controller
            .local_runtime
            .as_ref()
            .and_then(|runtime| {
                runtime.workloads.iter().find(|candidate| {
                    candidate.workload_id.as_str() == summary.identity.workload_id.as_str()
                        && candidate.runtime.capabilities.guest_control
                })
            })
            .ok_or(TerminalFailure::Unavailable)?;
        let workload_name = summary
            .identity
            .legacy_vm_name
            .as_ref()
            .map(|vm| vm.as_str())
            .unwrap_or(local_workload.vm_name.as_str())
            .to_owned();
        if workload_name != local_workload.vm_name.as_str() {
            return Err(TerminalFailure::Protocol);
        }
        let vm = resolver
            .manifest
            .vms
            .get(&workload_name)
            .filter(|vm| vm.runtime.capabilities.guest_control)
            .ok_or(TerminalFailure::Unavailable)?;
        let vsock_cid = vm
            .observability
            .vsock_cid
            .filter(|cid| *cid > 2)
            .ok_or(TerminalFailure::Unavailable)?;
        let dag = resolver
            .processes
            .vms
            .iter()
            .find(|dag| dag.vm == workload_name)
            .ok_or(TerminalFailure::Unavailable)?;
        let runner = dag
            .nodes
            .iter()
            .find(|node| node.role == ProcessRole::CloudHypervisorRunner)
            .ok_or(TerminalFailure::Unavailable)?;
        if !self
            .pidfd_table
            .still_alive_same_start_time(&workload_name, VM_RUNNER_ROLE_ID)
        {
            return Err(TerminalFailure::Unavailable);
        }
        let (_, pid, start_time) = self
            .pidfd_table
            .dup_pidfd_for(&workload_name, VM_RUNNER_ROLE_ID)
            .ok_or(TerminalFailure::Unavailable)?;
        let runner_bytes = serde_json::to_vec(runner).map_err(|_| TerminalFailure::Internal)?;
        let mut runtime_digest = Sha256::new();
        runtime_digest.update(b"d2b-guest-runtime-instance-v1\0");
        digest_field(&mut runtime_digest, realm_id.as_str().as_bytes());
        digest_field(&mut runtime_digest, workload_id.as_str().as_bytes());
        digest_field(&mut runtime_digest, workload_name.as_bytes());
        runtime_digest.update(vsock_cid.to_be_bytes());
        runtime_digest.update(GUEST_V2_VSOCK_PORT.to_be_bytes());
        runtime_digest.update(pid.to_be_bytes());
        runtime_digest.update(start_time.to_be_bytes());
        digest_field(&mut runtime_digest, &runner_bytes);
        let broker_uid = resolve_user(controller.broker.user.as_str())?;
        let broker_gid = resolve_group(controller.broker.group.as_str())?;
        let broker_realm_id = controller.realm_id.as_str().to_owned();
        let broker_workload_id = workload_id.as_str().to_owned();
        Ok(GuestSessionAuthority {
            realm_id,
            workload_id,
            broker_realm_id,
            broker_workload_id,
            broker_endpoint: PathBuf::from(controller.broker.socket_path.as_str()),
            broker_uid,
            broker_gid,
            controller_uid: self.controller.controller_uid(),
            controller_gid: self.controller.controller_gid(),
            controller_generation: self.controller.generation(),
            workload_name,
            vsock_cid,
            vsock_port: GUEST_V2_VSOCK_PORT,
            runtime_instance_digest: runtime_digest.finalize().into(),
            direct_schema_fingerprint: guest_service_fingerprint()?,
        })
    }

    pub(crate) fn validate_active(
        &self,
        authority: &GuestSessionAuthority,
    ) -> Result<(), TerminalFailure> {
        let current = self.resolve_verified(&authority.workload_name)?;
        if current.realm_id != authority.realm_id
            || current.workload_id != authority.workload_id
            || current.controller_generation != authority.controller_generation
            || current.vsock_cid != authority.vsock_cid
            || current.vsock_port != authority.vsock_port
            || current.runtime_instance_digest != authority.runtime_instance_digest
            || current.broker_endpoint != authority.broker_endpoint
        {
            return Err(TerminalFailure::Protocol);
        }
        Ok(())
    }
}

impl std::fmt::Debug for BundleGuestAuthorityPort {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BundleGuestAuthorityPort(REDACTED)")
    }
}

#[async_trait]
impl GuestAuthorityPort for BundleGuestAuthorityPort {
    async fn resolve(&self, workload: &str) -> Result<GuestSessionAuthority, TerminalFailure> {
        self.resolve_verified(workload)
    }
}

pub(crate) struct BrokerRealmGuestMaterialPort {
    identity: ControllerIdentityAuthority,
    bindings: Mutex<VerifiedRuntimeBindings>,
}

type VerifiedRuntimeBindingKey = (String, String, u64, [u8; 32]);
type VerifiedRuntimeBindings = BTreeMap<VerifiedRuntimeBindingKey, [u8; 32]>;

impl BrokerRealmGuestMaterialPort {
    pub(crate) fn new(identity: ControllerIdentityAuthority) -> Arc<Self> {
        Arc::new(Self {
            identity,
            bindings: Mutex::new(BTreeMap::new()),
        })
    }

    fn validate_controller(
        &self,
        authority: &GuestSessionAuthority,
    ) -> Result<Arc<ControllerStaticIdentity>, TerminalFailure> {
        let identity = self
            .identity
            .require()
            .map_err(|_| TerminalFailure::Protocol)?;
        if identity.binding().realm_id() != self.identity.binding().realm_id()
            || identity.binding().realm_id() != authority.broker_realm_id
            || authority.controller_generation != identity.binding().generation()
            || authority.controller_uid != identity.binding().controller_uid()
            || authority.controller_gid != identity.binding().controller_gid()
        {
            return Err(TerminalFailure::GenerationMismatch);
        }
        Ok(identity)
    }

    fn validate_material_response(
        &self,
        authority: &GuestSessionAuthority,
        identity: &ControllerStaticIdentity,
        material: &AppliedGuestSessionMaterial,
    ) -> Result<(), TerminalFailure> {
        let credential = &material.credential;
        if credential.session_generation() != authority.controller_generation
            || credential.parent_static_public_key() != identity.public_key()
            || credential.channel_binding() == &[0; 32]
            || material.configured_launches.realm_id() != &authority.realm_id
            || material.configured_launches.workload_id() != &authority.workload_id
            || material.configured_launches.workload_digest() == &[0; 32]
        {
            return Err(TerminalFailure::GenerationMismatch);
        }
        let key = (
            authority.realm_id.as_str().to_owned(),
            authority.workload_id.as_str().to_owned(),
            authority.controller_generation,
            authority.runtime_instance_digest,
        );
        let expected = credential.bootstrap().map(|bootstrap| {
            guest_runtime_channel_binding(GuestRuntimeChannelBindingInput {
                realm_id: authority.realm_id.as_str(),
                workload_id: authority.workload_id.as_str(),
                controller_generation: authority.controller_generation,
                runtime_instance_digest: &authority.runtime_instance_digest,
                vsock_cid: authority.vsock_cid,
                vsock_port: authority.vsock_port,
                boot_nonce: &bootstrap.binding().replay_nonce,
            })
        });
        let mut bindings = self
            .bindings
            .lock()
            .map_err(|_| TerminalFailure::Protocol)?;
        match (expected, bindings.get(&key)) {
            (Some(expected), _) if credential.channel_binding() != &expected => {
                return Err(TerminalFailure::GenerationMismatch);
            }
            (None, Some(expected)) if credential.channel_binding() != expected => {
                return Err(TerminalFailure::GenerationMismatch);
            }
            _ => {}
        }
        bindings.insert(key, *credential.channel_binding());
        Ok(())
    }
}

impl std::fmt::Debug for BrokerRealmGuestMaterialPort {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BrokerRealmGuestMaterialPort(REDACTED)")
    }
}

#[async_trait]
impl RealmGuestMaterialPort for BrokerRealmGuestMaterialPort {
    async fn apply(
        &self,
        authority: &GuestSessionAuthority,
    ) -> Result<AppliedGuestSessionMaterial, TerminalFailure> {
        let identity = self.validate_controller(authority)?;
        let operation = random_operation("guest-material")?;
        let storage_ref = guest_material_resource_id(&authority.broker_workload_id);
        let request_digest = guest_material_apply_digest(GuestMaterialApplyDigestInput {
            realm_id: &authority.broker_realm_id,
            workload_id: &authority.broker_workload_id,
            operation_id: &operation.operation_id,
            session_storage_ref: &storage_ref,
            session_generation: authority.controller_generation,
        });
        let request = ServiceRequest {
            metadata: MessageField::some(request_metadata(
                authority.controller_generation,
                operation.request_id,
                request_digest,
            )?),
            scope: MessageField::some(broker_scope(authority)),
            resource_id: storage_ref,
            operation_id: operation.operation_id,
            request_digest: request_digest.to_vec(),
            desired_state: DesiredState::DESIRED_STATE_PRESENT.into(),
            ..Default::default()
        };
        request
            .validate_wire(true)
            .map_err(|_| TerminalFailure::Protocol)?;
        let connection = connect_realm_broker(authority, identity.binding()).await?;
        let response = match connection
            .apply(request, operation.request_id, Vec::new(), true)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                connection.close().await;
                return Err(error);
            }
        };
        let material = match decode_material_reply(authority, &response) {
            Ok(material) => material,
            Err(error) => {
                connection.close().await;
                return Err(error);
            }
        };
        if let Err(error) = self.validate_material_response(authority, &identity, &material) {
            connection.close().await;
            return Err(error);
        }
        connection.close().await;
        Ok(material)
    }

    async fn persist_enrolled(
        &self,
        authority: &GuestSessionAuthority,
        credential: GuestSessionCredentialV1,
    ) -> Result<(), TerminalFailure> {
        let identity = self.validate_controller(authority)?;
        if credential.session_generation() != authority.controller_generation
            || credential.parent_static_public_key() != identity.public_key()
            || credential.guest_identity_is_unbound()
            || credential.bootstrap().is_some()
        {
            return Err(TerminalFailure::GenerationMismatch);
        }
        let encoded = credential.encode().map_err(|_| TerminalFailure::Protocol)?;
        let credential_digest: [u8; 32] = Sha256::digest(encoded.as_slice()).into();
        let operation = random_operation("guest-enrollment")?;
        let enrollment_ref = guest_enrollment_resource_id(&authority.broker_workload_id);
        let request_digest = guest_enrollment_apply_digest(GuestEnrollmentApplyDigestInput {
            realm_id: &authority.broker_realm_id,
            workload_id: &authority.broker_workload_id,
            operation_id: &operation.operation_id,
            enrollment_ref: &enrollment_ref,
            session_generation: authority.controller_generation,
            credential_digest: &credential_digest,
        });
        let request = ServiceRequest {
            metadata: MessageField::some(request_metadata(
                authority.controller_generation,
                operation.request_id,
                request_digest,
            )?),
            scope: MessageField::some(broker_scope(authority)),
            resource_id: enrollment_ref,
            operation_id: operation.operation_id,
            request_digest: request_digest.to_vec(),
            attachment_indexes: vec![0],
            desired_state: DesiredState::DESIRED_STATE_PRESENT.into(),
            ..Default::default()
        };
        request
            .validate_wire(true)
            .map_err(|_| TerminalFailure::Protocol)?;
        let attachment = credential_attachment(
            operation.request_id,
            authority.controller_generation,
            encoded.as_slice(),
        )?;
        let connection = connect_realm_broker(authority, identity.binding()).await?;
        let response = match connection
            .apply(request, operation.request_id, vec![attachment], false)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                connection.close().await;
                return Err(error);
            }
        };
        if response.message.outcome.enum_value().ok() != Some(Outcome::OUTCOME_SUCCEEDED)
            || response.message.result_digest != credential_digest
            || !response.attachments.is_empty()
        {
            connection.close().await;
            return Err(TerminalFailure::Protocol);
        }
        connection.close().await;
        Ok(())
    }
}

struct BrokerConnection {
    driver: Arc<dyn ComponentSessionDriver>,
    client: BrokerServiceClient,
}

struct BrokerResponse {
    message: common::ServiceResponse,
    attachments: Vec<OwnedAttachment>,
}

impl BrokerConnection {
    async fn apply(
        &self,
        request: ServiceRequest,
        request_id: [u8; 16],
        attachments: Vec<OwnedAttachment>,
        expect_response_attachments: bool,
    ) -> Result<BrokerResponse, TerminalFailure> {
        let expected_operation_id = request.operation_id.clone();
        if !attachments.is_empty() {
            self.driver
                .send_attachments(attachments)
                .await
                .map_err(|_| TerminalFailure::Unavailable)?;
        }
        let call = self.client.apply(
            ttrpc::context::with_timeout(BROKER_DEADLINE.as_nanos() as i64),
            &request,
        );
        let response = match tokio::time::timeout(BROKER_DEADLINE, call).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => return Err(TerminalFailure::Unavailable),
            Err(_) => {
                cancel_driver_request(&self.driver, request_id).await;
                return Err(TerminalFailure::Unavailable);
            }
        };
        response
            .validate_wire(false)
            .map_err(|_| TerminalFailure::Protocol)?;
        if response.operation_id != expected_operation_id {
            return Err(TerminalFailure::Protocol);
        }
        let attachments = if expect_response_attachments {
            let attachments =
                tokio::time::timeout(BROKER_DEADLINE, self.driver.receive_attachments())
                    .await
                    .map_err(|_| TerminalFailure::Unavailable)?
                    .map_err(|_| TerminalFailure::Unavailable)?;
            for (index, attachment) in attachments.iter().enumerate() {
                let descriptor = attachment.descriptor().ok_or(TerminalFailure::Protocol)?;
                if descriptor.index
                    != u16::try_from(index).map_err(|_| TerminalFailure::Protocol)?
                    || descriptor.request_id.as_bytes() != request_id
                    || descriptor.purpose
                        != d2b_contracts::v2_component_session::AttachmentPurpose::ResponseOutput
                {
                    return Err(TerminalFailure::Protocol);
                }
            }
            attachments
        } else {
            Vec::new()
        };
        Ok(BrokerResponse {
            message: response,
            attachments,
        })
    }

    async fn close(&self) {
        let _ = self
            .driver
            .close(
                d2b_contracts::v2_component_session::CloseReason::Normal,
                Remediation::None,
            )
            .await;
    }
}

async fn connect_realm_broker(
    authority: &GuestSessionAuthority,
    controller: &ControllerProcessBinding,
) -> Result<BrokerConnection, TerminalFailure> {
    if authority.broker_endpoint == std::path::Path::new(d2b_contracts::BROKER_SOCKET_PATH) {
        return Err(TerminalFailure::Unauthorized);
    }
    let path = authority.broker_endpoint.clone();
    let fd = tokio::task::spawn_blocking(move || connect_seqpacket(path))
        .await
        .map_err(|_| TerminalFailure::Unavailable)??;
    let socket = SeqpacketSocket::from_owned(fd).map_err(|_| TerminalFailure::Unavailable)?;
    let broker_uid = authority.broker_uid;
    let broker_gid = authority.broker_gid;
    let controller = controller.clone();
    let verifier = Arc::new(move |peer: &SeqpacketSocket| {
        controller.verify_broker_peer(peer, broker_uid, broker_gid)
    });
    let policy = realm_broker_policy(authority)?;
    let descriptor_resolver: DescriptorPolicyResolver = Arc::new(|descriptor| {
        if descriptor.service == ServicePackage::BrokerV2
            && descriptor.method_id == broker_apply_method_id()
            && descriptor.kind == AttachmentKind::FileDescriptor
            && descriptor.object_type == KernelObjectType::Memfd
            && descriptor.access == AttachmentAccess::ReadOnly
            && descriptor.purpose
                == d2b_contracts::v2_component_session::AttachmentPurpose::ResponseOutput
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
        credit_scopes(policy.attachment_policy.max_per_session),
        descriptor_resolver,
        PeerIdentityPolicy::pathname(verifier),
    )
    .map_err(|_| TerminalFailure::Unavailable)?;
    let engine = tokio::time::timeout(
        BROKER_DEADLINE,
        SessionEngine::establish_initiator(
            transport,
            policy,
            HandshakeCredentials::Nn,
            Instant::now(),
        ),
    )
    .await
    .map_err(|_| TerminalFailure::Unavailable)?
    .map_err(|_| TerminalFailure::Unavailable)?;
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
    let (client_transport, bridge_transport) = tokio::io::duplex(2 * 1024 * 1024);
    let client =
        BrokerServiceClient::new(ttrpc::r#async::Client::new(Socket::new(client_transport)));
    tokio::spawn(pump_broker_ttrpc(bridge_transport, Arc::clone(&driver)));
    Ok(BrokerConnection { driver, client })
}

fn decode_material_reply(
    authority: &GuestSessionAuthority,
    response: &BrokerResponse,
) -> Result<AppliedGuestSessionMaterial, TerminalFailure> {
    if response.message.outcome.enum_value().ok() != Some(Outcome::OUTCOME_SUCCEEDED)
        || response.message.attachment_indexes != [0, 1]
        || response.attachments.len() != 2
    {
        return Err(TerminalFailure::Protocol);
    }
    let credential_bytes =
        read_memfd_attachment(&response.attachments[0], 0, authority.controller_generation)?;
    let configured_bytes =
        read_memfd_attachment(&response.attachments[1], 1, authority.controller_generation)?;
    let credential = GuestSessionCredentialV1::decode(&credential_bytes)
        .map_err(|_| TerminalFailure::Protocol)?;
    let configured_launches = GuestConfiguredLaunchesV1::decode(&configured_bytes)
        .map_err(|_| TerminalFailure::Protocol)?;
    let session_digest: [u8; 32] = Sha256::digest(&credential_bytes).into();
    let configured_digest: [u8; 32] = Sha256::digest(&configured_bytes).into();
    let mut pair = Sha256::new();
    pair.update(b"d2b-guest-material-pair-v1\0");
    pair.update(session_digest);
    pair.update(configured_digest);
    if response.message.result_digest != pair.finalize().as_slice() {
        return Err(TerminalFailure::GenerationMismatch);
    }
    Ok(AppliedGuestSessionMaterial {
        credential,
        configured_launches,
    })
}

fn read_memfd_attachment(
    attachment: &OwnedAttachment,
    index: u16,
    generation: u64,
) -> Result<Zeroizing<Vec<u8>>, TerminalFailure> {
    let descriptor = attachment.descriptor().ok_or(TerminalFailure::NotFound)?;
    if descriptor.index != index
        || descriptor.service != ServicePackage::BrokerV2
        || descriptor.method_id != broker_apply_method_id()
        || descriptor.object_type != KernelObjectType::Memfd
        || descriptor.access != AttachmentAccess::ReadOnly
        || descriptor.reconnect_generation != generation
        || !descriptor.cloexec_required
    {
        return Err(TerminalFailure::GenerationMismatch);
    }
    let fd = attachment
        .payload()
        .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
        .and_then(UnixAttachmentPayload::file)
        .ok_or(TerminalFailure::Protocol)?
        .try_clone_to_owned()
        .map_err(|_| TerminalFailure::Unavailable)?;
    let mut bytes = Zeroizing::new(Vec::new());
    File::from(fd)
        .take(MAX_MATERIAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| TerminalFailure::Internal)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_MATERIAL_BYTES {
        return Err(TerminalFailure::Protocol);
    }
    Ok(bytes)
}

fn credential_attachment(
    request_id: [u8; 16],
    generation: u64,
    bytes: &[u8],
) -> Result<OwnedAttachment, TerminalFailure> {
    let fd = sealed_read_only_memfd("guest-enrollment-v1", bytes)?;
    let request_id = RequestId::new(request_id.to_vec()).map_err(|_| TerminalFailure::Internal)?;
    let descriptor = AttachmentDescriptor {
        index: 0,
        kind: AttachmentKind::FileDescriptor,
        object_type: KernelObjectType::Memfd,
        access: AttachmentAccess::ReadOnly,
        purpose: d2b_contracts::v2_component_session::AttachmentPurpose::RequestInput,
        service: ServicePackage::BrokerV2,
        method_id: broker_apply_method_id(),
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
        .map_err(|_| TerminalFailure::Internal)?,
    };
    let identity =
        ObjectIdentity::from_trusted(&fd, KernelObjectType::Memfd, AttachmentAccess::ReadOnly)
            .map_err(|_| TerminalFailure::Internal)?;
    OwnedUnixAttachment::file(descriptor, fd, DescriptorPolicy::File(identity))
        .map_err(|_| TerminalFailure::Internal)
}

fn sealed_read_only_memfd(name: &str, bytes: &[u8]) -> Result<OwnedFd, TerminalFailure> {
    let fd = rustix::fs::memfd_create(
        name,
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .map_err(|_| TerminalFailure::Internal)?;
    let mut writer = File::from(fd);
    writer
        .write_all(bytes)
        .and_then(|()| writer.flush())
        .and_then(|()| writer.seek(SeekFrom::Start(0)).map(|_| ()))
        .map_err(|_| TerminalFailure::Internal)?;
    fcntl(
        writer.as_raw_fd(),
        FcntlArg::F_ADD_SEALS(
            SealFlag::F_SEAL_WRITE
                | SealFlag::F_SEAL_GROW
                | SealFlag::F_SEAL_SHRINK
                | SealFlag::F_SEAL_SEAL,
        ),
    )
    .map_err(|_| TerminalFailure::Internal)?;
    let readonly = rustix::fs::open(
        format!("/proc/self/fd/{}", writer.as_raw_fd()),
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| TerminalFailure::Internal)?;
    drop(writer);
    Ok(readonly)
}

#[async_trait]
trait VsockRuntime: Send + Sync {
    async fn accept_bootstrap(
        &self,
        cid: u32,
        port: u32,
    ) -> Result<ErasedVsockTransport, TerminalFailure>;
    async fn connect(&self, cid: u32, port: u32) -> Result<ErasedVsockTransport, TerminalFailure>;
}

#[derive(Default)]
struct NativeVsockRuntime {
    bootstrap: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for NativeVsockRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NativeVsockRuntime")
    }
}

#[async_trait]
impl VsockRuntime for NativeVsockRuntime {
    async fn accept_bootstrap(
        &self,
        cid: u32,
        port: u32,
    ) -> Result<ErasedVsockTransport, TerminalFailure> {
        let _guard = self.bootstrap.lock().await;
        let mut listener =
            NativeVsockListener::bind(port).map_err(|_| TerminalFailure::Unavailable)?;
        listener
            .accept(cid)
            .await
            .map(ErasedVsockTransport::new)
            .map_err(|_| TerminalFailure::Unavailable)
    }

    async fn connect(&self, cid: u32, port: u32) -> Result<ErasedVsockTransport, TerminalFailure> {
        NativeVsockTransport::connect(cid, port)
            .await
            .map(ErasedVsockTransport::new)
            .map_err(|_| TerminalFailure::Unavailable)
    }
}

struct ErasedVsockTransport {
    inner: Box<dyn OwnedTransport>,
}

impl ErasedVsockTransport {
    fn new(transport: impl OwnedTransport + 'static) -> Self {
        Self {
            inner: Box::new(transport),
        }
    }
}

#[async_trait]
impl OwnedTransport for ErasedVsockTransport {
    fn descriptor(&self) -> TransportDescriptor {
        self.inner.descriptor()
    }

    async fn receive(&mut self, protected_limit: usize) -> Result<TransportPacket, TransportError> {
        self.inner.receive(protected_limit).await
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        self.inner.send(packet).await
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

pub(crate) struct VsockDirectGuestSessionPort {
    identity: ControllerIdentityAuthority,
    authority: Arc<dyn ActiveGuestRuntimeAuthority>,
    runtime: Arc<dyn VsockRuntime>,
}

trait ActiveGuestRuntimeAuthority: Send + Sync {
    fn validate_active(&self, authority: &GuestSessionAuthority) -> Result<(), TerminalFailure>;
}

impl ActiveGuestRuntimeAuthority for BundleGuestAuthorityPort {
    fn validate_active(&self, authority: &GuestSessionAuthority) -> Result<(), TerminalFailure> {
        BundleGuestAuthorityPort::validate_active(self, authority)
    }
}

impl VsockDirectGuestSessionPort {
    pub(crate) fn production(
        identity: ControllerIdentityAuthority,
        authority: Arc<BundleGuestAuthorityPort>,
    ) -> Arc<Self> {
        let authority: Arc<dyn ActiveGuestRuntimeAuthority> = authority;
        Arc::new(Self {
            identity,
            authority,
            runtime: Arc::new(NativeVsockRuntime::default()),
        })
    }

    #[cfg(test)]
    fn with_test_runtime(
        identity: ControllerIdentityAuthority,
        authority: Arc<dyn ActiveGuestRuntimeAuthority>,
        runtime: Arc<dyn VsockRuntime>,
    ) -> Arc<Self> {
        Arc::new(Self {
            identity,
            authority,
            runtime,
        })
    }

    fn controller_identity(
        &self,
        authority: &GuestSessionAuthority,
        material: &AppliedGuestSessionMaterial,
    ) -> Result<Arc<ControllerStaticIdentity>, TerminalFailure> {
        self.authority.validate_active(authority)?;
        let identity = self
            .identity
            .require()
            .map_err(|_| TerminalFailure::Unavailable)?;
        if identity.binding().generation() != authority.controller_generation
            || material.credential.session_generation() != authority.controller_generation
            || material.credential.parent_static_public_key() != identity.public_key()
            || material.credential.channel_binding() == &[0; 32]
        {
            return Err(TerminalFailure::GenerationMismatch);
        }
        Ok(identity)
    }
}

impl std::fmt::Debug for VsockDirectGuestSessionPort {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("VsockDirectGuestSessionPort(REDACTED)")
    }
}

#[async_trait]
impl DirectGuestSessionPort for VsockDirectGuestSessionPort {
    async fn bootstrap(
        &self,
        authority: &GuestSessionAuthority,
        material: &AppliedGuestSessionMaterial,
    ) -> Result<BootstrapGuestSession, TerminalFailure> {
        let identity = self.controller_identity(authority, material)?;
        let bootstrap = material
            .credential
            .bootstrap()
            .ok_or(TerminalFailure::Protocol)?;
        if !material.credential.guest_identity_is_unbound() {
            return Err(TerminalFailure::Protocol);
        }
        let now_ms = unix_time_ms()?;
        bootstrap
            .admit(now_ms)
            .map_err(|_| TerminalFailure::Unauthorized)?;
        let expected_binding = guest_runtime_channel_binding(GuestRuntimeChannelBindingInput {
            realm_id: authority.realm_id.as_str(),
            workload_id: authority.workload_id.as_str(),
            controller_generation: authority.controller_generation,
            runtime_instance_digest: &authority.runtime_instance_digest,
            vsock_cid: authority.vsock_cid,
            vsock_port: authority.vsock_port,
            boot_nonce: &bootstrap.binding().replay_nonce,
        });
        if material.credential.channel_binding() != &expected_binding {
            return Err(TerminalFailure::GenerationMismatch);
        }
        let mut transport = tokio::time::timeout(
            GUEST_DEADLINE,
            self.runtime
                .accept_bootstrap(authority.vsock_cid, authority.vsock_port),
        )
        .await
        .map_err(|_| TerminalFailure::Unavailable)??;
        transport
            .send(TransportPacket::new(bootstrap_evidence(bootstrap)?))
            .await
            .map_err(|_| TerminalFailure::Unavailable)?;
        let mut psk = *bootstrap.expose_psk();
        let mut admission = BootstrapAdmission::new(
            bootstrap.binding().clone(),
            BootstrapPsk::new(psk).map_err(|_| TerminalFailure::Unauthorized)?,
        )
        .map_err(|_| TerminalFailure::Unauthorized)?;
        psk.zeroize();
        let admitted = admission
            .consume(
                &bootstrap.binding().operation_id,
                &bootstrap.binding().replay_nonce,
                now_ms,
            )
            .map_err(|_| TerminalFailure::Unauthorized)?;
        let engine = SessionEngine::establish_responder(
            transport,
            guest_policy(
                true,
                authority.controller_generation,
                *material.credential.channel_binding(),
            )?,
            HandshakeCredentials::IkPsk2Responder {
                local_private: identity
                    .handshake_secret()
                    .map_err(|_| TerminalFailure::Unavailable)?,
                psk: admitted,
            },
            Instant::now(),
        )
        .await
        .map_err(|_| TerminalFailure::Unavailable)?;
        let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
        let operation = random_operation("guest-bootstrap")?;
        let request = guest_bootstrap_request(authority, &identity, operation)?;
        let response = match call_guest_bootstrap(Arc::clone(&driver), &request).await {
            Ok(response) => response,
            Err(error) => {
                close_driver(&driver, Remediation::RetryBounded).await;
                return Err(error);
            }
        };
        if validate_guest_session_response_for_bootstrap(&request, &response).is_err() {
            close_driver(&driver, Remediation::ReEnrollPeer).await;
            return Err(TerminalFailure::Protocol);
        }
        let guest_static_public_key: [u8; 32] = response
            .guest_static_public_key
            .as_slice()
            .try_into()
            .map_err(|_| TerminalFailure::Protocol)?;
        let guest_identity_digest: [u8; 32] = response
            .guest_identity_digest
            .as_slice()
            .try_into()
            .map_err(|_| TerminalFailure::Protocol)?;
        if response.outcome.enum_value().ok() != Some(Outcome::OUTCOME_SUCCEEDED)
            || guest_identity_digest != Sha256::digest(guest_static_public_key).as_slice()
            || response.guest_identity_handle != guest_identity_handle(&guest_identity_digest)
        {
            close_driver(&driver, Remediation::ReEnrollPeer).await;
            return Err(TerminalFailure::Protocol);
        }
        Ok(BootstrapGuestSession {
            driver,
            guest_identity_digest,
            guest_static_public_key,
        })
    }

    async fn reconnect(
        &self,
        authority: &GuestSessionAuthority,
        material: &AppliedGuestSessionMaterial,
    ) -> Result<Arc<dyn ComponentSessionDriver>, TerminalFailure> {
        let identity = self.controller_identity(authority, material)?;
        if material.credential.guest_identity_is_unbound()
            || material.credential.bootstrap().is_some()
        {
            return Err(TerminalFailure::Protocol);
        }
        let guest_public = *material
            .credential
            .guest_static_public_key()
            .ok_or(TerminalFailure::Protocol)?;
        let guest_identity = *material
            .credential
            .guest_identity_digest()
            .ok_or(TerminalFailure::Protocol)?;
        if guest_identity != Sha256::digest(guest_public).as_slice() {
            return Err(TerminalFailure::Protocol);
        }
        let transport = tokio::time::timeout(
            GUEST_DEADLINE,
            self.runtime
                .connect(authority.vsock_cid, authority.vsock_port),
        )
        .await
        .map_err(|_| TerminalFailure::Unavailable)??;
        let engine = SessionEngine::establish_initiator(
            transport,
            guest_policy(
                false,
                authority.controller_generation,
                *material.credential.channel_binding(),
            )?,
            HandshakeCredentials::Kk {
                local_private: identity
                    .handshake_secret()
                    .map_err(|_| TerminalFailure::Unavailable)?,
                remote_public: guest_public,
            },
            Instant::now(),
        )
        .await
        .map_err(|_| TerminalFailure::Unavailable)?;
        let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
        let operation = random_operation("guest-reconnect")?;
        let request = guest_reconnect_request(authority, &identity, material, operation)?;
        let response = match call_guest_reconnect(Arc::clone(&driver), &request).await {
            Ok(response) => response,
            Err(error) => {
                close_driver(&driver, Remediation::RetryBounded).await;
                return Err(error);
            }
        };
        if validate_guest_session_response_for_reconnect(&request, &response).is_err() {
            close_driver(&driver, Remediation::ReEnrollPeer).await;
            return Err(TerminalFailure::Protocol);
        }
        if response.outcome.enum_value().ok() != Some(Outcome::OUTCOME_SUCCEEDED) {
            close_driver(&driver, Remediation::ReEnrollPeer).await;
            return Err(TerminalFailure::Unauthorized);
        }
        Ok(driver)
    }

    async fn is_live(&self, session: &crate::guest_terminal::GuestTerminalSession) -> bool {
        session.probe_live(Duration::from_millis(250)).await
    }

    async fn close_bootstrap(&self, driver: Arc<dyn ComponentSessionDriver>) {
        let _ = driver
            .close(
                d2b_contracts::v2_component_session::CloseReason::Normal,
                Remediation::None,
            )
            .await;
    }
}

async fn call_guest_bootstrap(
    driver: Arc<dyn ComponentSessionDriver>,
    request: &GuestBootstrapRequest,
) -> Result<GuestSessionResponse, TerminalFailure> {
    let (client_transport, bridge_transport) = tokio::io::duplex(2 * 1024 * 1024);
    let client =
        GuestServiceClient::new(ttrpc::r#async::Client::new(Socket::new(client_transport)));
    tokio::spawn(pump_guest_session_ttrpc(
        bridge_transport,
        Arc::clone(&driver),
    ));
    let result = tokio::time::timeout(
        GUEST_DEADLINE,
        client.bootstrap(
            ttrpc::context::with_timeout(GUEST_DEADLINE.as_nanos() as i64),
            request,
        ),
    )
    .await;
    match result {
        Ok(result) => result.map_err(|_| TerminalFailure::Unavailable),
        Err(_) => {
            cancel_guest_request(&driver, request.context.as_ref()).await;
            Err(TerminalFailure::Unavailable)
        }
    }
}

async fn call_guest_reconnect(
    driver: Arc<dyn ComponentSessionDriver>,
    request: &GuestReconnectRequest,
) -> Result<GuestSessionResponse, TerminalFailure> {
    let (client_transport, bridge_transport) = tokio::io::duplex(2 * 1024 * 1024);
    let client =
        GuestServiceClient::new(ttrpc::r#async::Client::new(Socket::new(client_transport)));
    tokio::spawn(pump_guest_session_ttrpc(
        bridge_transport,
        Arc::clone(&driver),
    ));
    let result = tokio::time::timeout(
        GUEST_DEADLINE,
        client.reconnect(
            ttrpc::context::with_timeout(GUEST_DEADLINE.as_nanos() as i64),
            request,
        ),
    )
    .await;
    match result {
        Ok(result) => result.map_err(|_| TerminalFailure::Unavailable),
        Err(_) => {
            cancel_guest_request(&driver, request.context.as_ref()).await;
            Err(TerminalFailure::Unavailable)
        }
    }
}

async fn cancel_guest_request(
    driver: &Arc<dyn ComponentSessionDriver>,
    context: Option<&GuestOperationContext>,
) {
    let Some(request_id) = context
        .and_then(|context| context.metadata.as_ref())
        .and_then(|metadata| RequestId::new(metadata.request_id.clone()).ok())
    else {
        return;
    };
    let _ = driver.cancel(driver.generation(), request_id).await;
}

async fn close_driver(driver: &Arc<dyn ComponentSessionDriver>, remediation: Remediation) {
    let _ = driver
        .close(
            d2b_contracts::v2_component_session::CloseReason::AuthenticationFailed,
            remediation,
        )
        .await;
}

fn guest_bootstrap_request(
    authority: &GuestSessionAuthority,
    identity: &ControllerStaticIdentity,
    operation: RandomOperation,
) -> Result<GuestBootstrapRequest, TerminalFailure> {
    let mut digest = Sha256::new();
    digest.update(b"d2b.guest.v2\0Bootstrap\0");
    digest.update(authority.runtime_instance_digest);
    digest.update(authority.controller_generation.to_be_bytes());
    digest.update(operation.request_id);
    let request_digest: [u8; 32] = digest.finalize().into();
    Ok(GuestBootstrapRequest {
        context: MessageField::some(guest_context(authority, operation, request_digest)?),
        expected_generation: authority.controller_generation,
        expected_parent_static_public_key_digest: Sha256::digest(identity.public_key()).to_vec(),
        requested_capabilities: Vec::new(),
        ..Default::default()
    })
}

fn guest_reconnect_request(
    authority: &GuestSessionAuthority,
    identity: &ControllerStaticIdentity,
    material: &AppliedGuestSessionMaterial,
    operation: RandomOperation,
) -> Result<GuestReconnectRequest, TerminalFailure> {
    let guest_public = *material
        .credential
        .guest_static_public_key()
        .ok_or(TerminalFailure::Protocol)?;
    let guest_identity = *material
        .credential
        .guest_identity_digest()
        .ok_or(TerminalFailure::Protocol)?;
    let mut digest = Sha256::new();
    digest.update(b"d2b.guest.v2\0Reconnect\0");
    digest.update(authority.runtime_instance_digest);
    digest.update(authority.controller_generation.to_be_bytes());
    digest.update(guest_identity);
    digest.update(guest_public);
    digest.update(operation.request_id);
    let request_digest: [u8; 32] = digest.finalize().into();
    Ok(GuestReconnectRequest {
        context: MessageField::some(guest_context(authority, operation, request_digest)?),
        expected_generation: authority.controller_generation,
        guest_identity_handle: guest_identity_handle(&guest_identity),
        expected_guest_static_public_key_digest: Sha256::digest(guest_public).to_vec(),
        expected_parent_static_public_key_digest: Sha256::digest(identity.public_key()).to_vec(),
        required_capabilities: Vec::new(),
        expected_guest_identity_digest: guest_identity.to_vec(),
        expected_guest_static_public_key: guest_public.to_vec(),
        ..Default::default()
    })
}

fn guest_context(
    authority: &GuestSessionAuthority,
    operation: RandomOperation,
    request_digest: [u8; 32],
) -> Result<GuestOperationContext, TerminalFailure> {
    Ok(GuestOperationContext {
        metadata: MessageField::some(request_metadata(
            authority.controller_generation,
            operation.request_id,
            request_digest,
        )?),
        scope: MessageField::some(guest_scope(authority)),
        operation_id: operation.operation_id,
        request_digest: request_digest.to_vec(),
        ..Default::default()
    })
}

fn guest_policy(
    bootstrap: bool,
    generation: u64,
    channel_binding: [u8; 32],
) -> Result<EndpointPolicy, TerminalFailure> {
    let (purpose, purpose_class, initiator, responder, noise, evidence) = if bootstrap {
        (
            EndpointPurpose::GuestBootstrap,
            PurposeClass::Bootstrap,
            EndpointRole::GuestAgent,
            EndpointRole::RealmController,
            NoiseProfile::Ikpsk2_25519ChaChaPolySha256,
            IdentityEvidenceRequirement::ParentStaticAndSingleUsePsk,
        )
    } else {
        (
            EndpointPurpose::GuestControl,
            PurposeClass::Enrolled,
            EndpointRole::RealmController,
            EndpointRole::GuestAgent,
            NoiseProfile::Kk25519ChaChaPolySha256,
            IdentityEvidenceRequirement::EnrolledStaticKeys,
        )
    };
    Ok(EndpointPolicy {
        purpose,
        purpose_class,
        initiator_role: initiator,
        responder_role: responder,
        service: ServicePackage::GuestV2,
        schema_fingerprint: guest_service_fingerprint()?,
        noise_profile: noise,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::NativeVsock,
            locality: Locality::GuestLocal,
            channel_binding,
            identity_evidence: evidence,
        },
        reconnect_generation: generation,
        attachment_policy: AttachmentPolicy::disabled(),
    })
}

fn realm_broker_policy(
    authority: &GuestSessionAuthority,
) -> Result<EndpointPolicy, TerminalFailure> {
    let controller_namespace_uid = u32::from(authority.controller_uid != authority.broker_uid);
    let controller_namespace_gid = u32::from(authority.controller_gid != authority.broker_gid);
    let mut binding = Sha256::new();
    binding.update(b"d2b.broker.v2\0unix-seqpacket\0");
    binding.update(controller_namespace_uid.to_be_bytes());
    binding.update(controller_namespace_gid.to_be_bytes());
    binding.update(EndpointRole::RealmBroker.as_str().as_bytes());
    let service = SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.broker.v2" && service.service == "BrokerService")
        .ok_or(TerminalFailure::Internal)?;
    Ok(EndpointPolicy {
        purpose: EndpointPurpose::PrivilegedBroker,
        purpose_class: PurposeClass::Local,
        initiator_role: EndpointRole::RealmController,
        responder_role: EndpointRole::RealmBroker,
        service: ServicePackage::BrokerV2,
        schema_fingerprint: service_schema_fingerprint(service),
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            channel_binding: binding.finalize().into(),
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: authority.controller_generation,
        attachment_policy: AttachmentPolicy {
            kind: AttachmentPolicyKind::PacketAtomic,
            max_per_packet: 16,
            max_per_request: 16,
            max_per_operation: 32,
            max_per_session: 128,
            credentials_allowed: false,
        },
    })
}

fn broker_scope(authority: &GuestSessionAuthority) -> IdentityScope {
    IdentityScope {
        realm_id: authority.broker_realm_id.clone(),
        workload_id: authority.broker_workload_id.clone(),
        ..Default::default()
    }
}

fn guest_scope(authority: &GuestSessionAuthority) -> IdentityScope {
    IdentityScope {
        realm_id: authority.realm_id.as_str().to_owned(),
        workload_id: authority.workload_id.as_str().to_owned(),
        ..Default::default()
    }
}

struct RandomOperation {
    request_id: [u8; 16],
    operation_id: String,
}

fn random_operation(prefix: &str) -> Result<RandomOperation, TerminalFailure> {
    let mut request_id = [0_u8; 16];
    getrandom::getrandom(&mut request_id).map_err(|_| TerminalFailure::Internal)?;
    if request_id == [0; 16] {
        return Err(TerminalFailure::Internal);
    }
    Ok(RandomOperation {
        operation_id: format!("{prefix}-{}", hex(&request_id)),
        request_id,
    })
}

fn request_metadata(
    generation: u64,
    request_id: [u8; 16],
    idempotency: [u8; 32],
) -> Result<common::RequestMetadata, TerminalFailure> {
    request_metadata_with_deadline(generation, request_id, idempotency, BROKER_DEADLINE)
}

fn request_metadata_with_deadline(
    generation: u64,
    request_id: [u8; 16],
    idempotency: [u8; 32],
    deadline: Duration,
) -> Result<common::RequestMetadata, TerminalFailure> {
    let issued_at_unix_ms = unix_time_ms()?;
    let lifetime_ms = u64::try_from(deadline.as_millis())
        .map_err(|_| TerminalFailure::Internal)?
        .min(MAX_REQUEST_LIFETIME_MS);
    if lifetime_ms == 0 {
        return Err(TerminalFailure::Internal);
    }
    let expires_at_unix_ms = issued_at_unix_ms
        .checked_add(lifetime_ms)
        .ok_or(TerminalFailure::Internal)?;
    Ok(common::RequestMetadata {
        request_id: request_id.to_vec(),
        idempotency_key: idempotency.to_vec(),
        issued_at_unix_ms,
        expires_at_unix_ms,
        session_generation: generation,
        ..Default::default()
    })
}

fn bootstrap_evidence(
    bootstrap: &d2b_contracts::v2_component_session::GuestBootstrapCredentialV1,
) -> Result<Vec<u8>, TerminalFailure> {
    if bootstrap.binding().operation_id.as_bytes().len() != 16 {
        return Err(TerminalFailure::Protocol);
    }
    let mut evidence = Vec::with_capacity(56);
    evidence.extend_from_slice(BOOTSTRAP_EVIDENCE_MAGIC);
    evidence.extend_from_slice(bootstrap.binding().operation_id.as_bytes());
    evidence.extend_from_slice(&bootstrap.binding().replay_nonce);
    Ok(evidence)
}

fn guest_identity_handle(identity_digest: &[u8; 32]) -> String {
    format!("guest-{}", hex(&identity_digest[..28]))
}

fn guest_service_fingerprint() -> Result<[u8; 32], TerminalFailure> {
    SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.guest.v2")
        .map(service_schema_fingerprint)
        .ok_or(TerminalFailure::Internal)
}

pub(crate) fn start_direct_guest_activation(
    start: DirectGuestActivationStart,
    deadline: Duration,
) -> Result<DirectGuestActivationStatus, DirectGuestActivationError> {
    run_activation_thread(deadline, move || async move {
        let connector = ProductionGuestTerminalConnector::production()
            .map_err(map_terminal_activation_error)?;
        let (authority, session) = connector
            .connect_scoped_session(&start.workload)
            .await
            .map_err(map_terminal_activation_error)?;
        let intent_id = direct_activation_intent_id(&authority)?;
        let payload = encode_direct_activation_payload(
            &intent_id,
            &start.operation_id,
            &start.switch_script_path,
            start.mode,
            start.timeout_ms,
        )?;
        let payload_digest: [u8; 32] = Sha256::digest(&payload).into();
        let transfer_operation =
            random_operation("activation-payload").map_err(map_terminal_activation_error)?;
        let transfer_context = guest_context_with_deadline(
            &authority,
            transfer_operation,
            payload_digest,
            ACTIVATION_CALL_DEADLINE,
        )
        .map_err(map_terminal_activation_error)?;
        let transfer = GuestFileTransferRequest {
            context: MessageField::some(transfer_context),
            artifact: EnumOrUnknown::new(GuestArtifactId::GUEST_ARTIFACT_ID_ACTIVATION_PAYLOAD),
            configured_intent_id: intent_id.clone(),
            direction: EnumOrUnknown::new(
                GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_HOST_TO_GUEST,
            ),
            offset: 0,
            declared_size: payload.len() as u64,
            expected_digest: payload_digest.to_vec(),
            ..Default::default()
        };
        session
            .upload_activation_payload(transfer, &payload, ACTIVATION_CALL_DEADLINE)
            .await
            .map_err(map_terminal_activation_error)?;

        let readiness = activation_service_request(
            &authority,
            &intent_id,
            "readiness",
            payload_digest,
            false,
            ACTIVATION_CALL_DEADLINE,
        )?;
        let readiness = session
            .activation_inspect(readiness, ACTIVATION_CALL_DEADLINE)
            .await
            .map_err(map_terminal_activation_error)?;
        if readiness.outcome.enum_value().ok() != Some(Outcome::OUTCOME_SUCCEEDED) {
            return Err(map_activation_response_error(&readiness));
        }

        let request = activation_service_request_with_id(
            &authority,
            &intent_id,
            &start.operation_id,
            payload_digest,
            activation_request_id(&start.operation_id),
            true,
            ACTIVATION_CALL_DEADLINE,
        )?;
        let response = session
            .activation_activate(request, ACTIVATION_CALL_DEADLINE)
            .await
            .map_err(map_terminal_activation_error)?;
        let mut status = activation_status_from_response(&response)?;
        while status.state == DirectGuestActivationState::Running {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let request = activation_service_request(
                &authority,
                &intent_id,
                &start.operation_id,
                [0; 32],
                false,
                ACTIVATION_CALL_DEADLINE,
            )?;
            let response = session
                .activation_inspect(request, ACTIVATION_CALL_DEADLINE)
                .await
                .map_err(map_terminal_activation_error)?;
            status = activation_status_from_response(&response)?;
        }
        Ok(status)
    })
}

pub(crate) fn inspect_direct_guest_activation(
    workload: String,
    operation_id: String,
    deadline: Duration,
) -> Result<DirectGuestActivationStatus, DirectGuestActivationError> {
    run_activation_thread(deadline, move || async move {
        let connector = ProductionGuestTerminalConnector::production()
            .map_err(map_terminal_activation_error)?;
        let (authority, session) = connector
            .connect_scoped_session(&workload)
            .await
            .map_err(map_terminal_activation_error)?;
        let intent_id = direct_activation_intent_id(&authority)?;
        let request = activation_service_request(
            &authority,
            &intent_id,
            &operation_id,
            [0; 32],
            false,
            ACTIVATION_CALL_DEADLINE,
        )?;
        let response = session
            .activation_inspect(request, ACTIVATION_CALL_DEADLINE)
            .await
            .map_err(map_terminal_activation_error)?;
        activation_status_from_response(&response)
    })
}

pub(crate) fn cancel_direct_guest_activation(
    workload: String,
    operation_id: String,
    deadline: Duration,
) -> Result<common::CancelOutcome, DirectGuestActivationError> {
    run_activation_thread(deadline, move || async move {
        let connector = ProductionGuestTerminalConnector::production()
            .map_err(map_terminal_activation_error)?;
        let (_, session) = connector
            .connect_scoped_session(&workload)
            .await
            .map_err(map_terminal_activation_error)?;
        let response = session
            .activation_cancel(
                common::CancelRequest {
                    session_generation: session.generation(),
                    request_id: activation_request_id(&operation_id).to_vec(),
                    ..Default::default()
                },
                ACTIVATION_CALL_DEADLINE,
            )
            .await
            .map_err(map_terminal_activation_error)?;
        response
            .outcome
            .enum_value()
            .map_err(|_| DirectGuestActivationError::Protocol)
    })
}

fn run_activation_thread<F, Fut, T>(
    deadline: Duration,
    operation: F,
) -> Result<T, DirectGuestActivationError>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, DirectGuestActivationError>> + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .name("d2b-direct-guest-activation".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| DirectGuestActivationError::Unavailable)?;
            runtime.block_on(async move {
                tokio::time::timeout(deadline, operation())
                    .await
                    .map_err(|_| DirectGuestActivationError::Deadline)?
            })
        })
        .map_err(|_| DirectGuestActivationError::Unavailable)?
        .join()
        .map_err(|_| DirectGuestActivationError::Unavailable)?
}

fn direct_activation_intent_id(
    authority: &GuestSessionAuthority,
) -> Result<String, DirectGuestActivationError> {
    let value = format!("activation-{}", authority.workload_id.as_str());
    if value.len() > 128
        || !value.as_bytes()[0].is_ascii_lowercase()
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        return Err(DirectGuestActivationError::Protocol);
    }
    Ok(value)
}

fn encode_direct_activation_payload(
    intent_id: &str,
    operation_id: &str,
    switch_script_path: &str,
    mode: DirectGuestActivationMode,
    timeout_ms: u64,
) -> Result<Vec<u8>, DirectGuestActivationError> {
    let intent_len =
        u16::try_from(intent_id.len()).map_err(|_| DirectGuestActivationError::Protocol)?;
    let operation_len =
        u16::try_from(operation_id.len()).map_err(|_| DirectGuestActivationError::Protocol)?;
    let path_len = u16::try_from(switch_script_path.len())
        .map_err(|_| DirectGuestActivationError::Protocol)?;
    let mut bytes =
        Vec::with_capacity(32 + intent_id.len() + operation_id.len() + switch_script_path.len());
    bytes.extend_from_slice(&ACTIVATION_PAYLOAD_MAGIC);
    bytes.extend_from_slice(&ACTIVATION_PAYLOAD_SCHEMA_VERSION.to_be_bytes());
    bytes.push(match mode {
        DirectGuestActivationMode::Switch => 1,
        DirectGuestActivationMode::Test => 3,
    });
    bytes.extend_from_slice(&[0; 3]);
    bytes.extend_from_slice(&timeout_ms.to_be_bytes());
    bytes.extend_from_slice(&intent_len.to_be_bytes());
    bytes.extend_from_slice(&operation_len.to_be_bytes());
    bytes.extend_from_slice(&path_len.to_be_bytes());
    bytes.extend_from_slice(&[0; 2]);
    bytes.extend_from_slice(intent_id.as_bytes());
    bytes.extend_from_slice(operation_id.as_bytes());
    bytes.extend_from_slice(switch_script_path.as_bytes());
    Ok(bytes)
}

fn activation_service_request(
    authority: &GuestSessionAuthority,
    intent_id: &str,
    operation_id: &str,
    request_digest: [u8; 32],
    mutating: bool,
    deadline: Duration,
) -> Result<ServiceRequest, DirectGuestActivationError> {
    let operation = random_operation("activation-call").map_err(map_terminal_activation_error)?;
    activation_service_request_with_id(
        authority,
        intent_id,
        operation_id,
        request_digest,
        operation.request_id,
        mutating,
        deadline,
    )
}

#[allow(clippy::too_many_arguments)]
fn activation_service_request_with_id(
    authority: &GuestSessionAuthority,
    intent_id: &str,
    operation_id: &str,
    request_digest: [u8; 32],
    request_id: [u8; 16],
    mutating: bool,
    deadline: Duration,
) -> Result<ServiceRequest, DirectGuestActivationError> {
    let idempotency = if request_digest == [0; 32] {
        let mut digest = Sha256::new();
        digest.update(b"d2b-activation-inspect-v2\0");
        digest.update(operation_id.as_bytes());
        digest.finalize().into()
    } else {
        request_digest
    };
    let metadata = request_metadata_with_deadline(
        authority.controller_generation,
        request_id,
        idempotency,
        deadline,
    )
    .map_err(map_terminal_activation_error)?;
    Ok(ServiceRequest {
        metadata: MessageField::some(metadata),
        scope: MessageField::some(guest_scope(authority)),
        resource_id: intent_id.to_owned(),
        operation_id: operation_id.to_owned(),
        request_digest: if request_digest == [0; 32] {
            Vec::new()
        } else {
            request_digest.to_vec()
        },
        desired_state: EnumOrUnknown::new(if mutating {
            DesiredState::DESIRED_STATE_RUNNING
        } else {
            DesiredState::DESIRED_STATE_UNSPECIFIED
        }),
        ..Default::default()
    })
}

fn guest_context_with_deadline(
    authority: &GuestSessionAuthority,
    operation: RandomOperation,
    request_digest: [u8; 32],
    deadline: Duration,
) -> Result<GuestOperationContext, TerminalFailure> {
    Ok(GuestOperationContext {
        metadata: MessageField::some(request_metadata_with_deadline(
            authority.controller_generation,
            operation.request_id,
            request_digest,
            deadline,
        )?),
        scope: MessageField::some(guest_scope(authority)),
        operation_id: operation.operation_id,
        request_digest: request_digest.to_vec(),
        ..Default::default()
    })
}

fn activation_request_id(operation_id: &str) -> [u8; 16] {
    let mut digest = Sha256::new();
    digest.update(b"d2b-activation-request-id-v2\0");
    digest.update(operation_id.as_bytes());
    let digest = digest.finalize();
    let mut request_id = [0_u8; 16];
    request_id.copy_from_slice(&digest[..16]);
    request_id
}

fn activation_status_from_response(
    response: &common::ServiceResponse,
) -> Result<DirectGuestActivationStatus, DirectGuestActivationError> {
    let outcome = response
        .outcome
        .enum_value()
        .map_err(|_| DirectGuestActivationError::Protocol)?;
    let state = match outcome {
        Outcome::OUTCOME_ACCEPTED => DirectGuestActivationState::Running,
        Outcome::OUTCOME_SUCCEEDED => DirectGuestActivationState::Succeeded,
        Outcome::OUTCOME_CANCELLED => DirectGuestActivationState::Cancelled,
        Outcome::OUTCOME_DEGRADED => DirectGuestActivationState::Lost,
        Outcome::OUTCOME_FAILED => {
            let kind = response
                .error
                .as_ref()
                .and_then(|error| error.kind.enum_value().ok())
                .ok_or(DirectGuestActivationError::Protocol)?;
            match kind {
                common::ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED => {
                    DirectGuestActivationState::TimedOut
                }
                common::ErrorKind::ERROR_KIND_INTERNAL => DirectGuestActivationState::Failed,
                common::ErrorKind::ERROR_KIND_NOT_FOUND => {
                    return Err(DirectGuestActivationError::NotFound);
                }
                common::ErrorKind::ERROR_KIND_UNAVAILABLE
                | common::ErrorKind::ERROR_KIND_CAPABILITY_DENIED => {
                    return Err(DirectGuestActivationError::Unavailable);
                }
                common::ErrorKind::ERROR_KIND_UNAUTHORIZED
                | common::ErrorKind::ERROR_KIND_UNAUTHENTICATED => {
                    return Err(DirectGuestActivationError::Unauthorized);
                }
                common::ErrorKind::ERROR_KIND_CONFLICT => {
                    return Err(DirectGuestActivationError::Conflict);
                }
                _ => return Err(DirectGuestActivationError::Protocol),
            }
        }
        Outcome::OUTCOME_DENIED => return Err(DirectGuestActivationError::Unauthorized),
        Outcome::OUTCOME_NOT_APPLICABLE | Outcome::OUTCOME_UNSPECIFIED => {
            return Err(DirectGuestActivationError::Protocol);
        }
    };
    Ok(DirectGuestActivationStatus { state })
}

fn map_activation_response_error(response: &common::ServiceResponse) -> DirectGuestActivationError {
    match response
        .error
        .as_ref()
        .and_then(|error| error.kind.enum_value().ok())
    {
        Some(
            common::ErrorKind::ERROR_KIND_UNAVAILABLE
            | common::ErrorKind::ERROR_KIND_CAPABILITY_DENIED,
        ) => DirectGuestActivationError::Unavailable,
        Some(
            common::ErrorKind::ERROR_KIND_UNAUTHORIZED
            | common::ErrorKind::ERROR_KIND_UNAUTHENTICATED,
        ) => DirectGuestActivationError::Unauthorized,
        Some(common::ErrorKind::ERROR_KIND_NOT_FOUND) => DirectGuestActivationError::NotFound,
        Some(common::ErrorKind::ERROR_KIND_CONFLICT) => DirectGuestActivationError::Conflict,
        Some(common::ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED) => {
            DirectGuestActivationError::Deadline
        }
        _ => DirectGuestActivationError::Protocol,
    }
}

fn map_terminal_activation_error(error: TerminalFailure) -> DirectGuestActivationError {
    match error {
        TerminalFailure::Unauthorized => DirectGuestActivationError::Unauthorized,
        TerminalFailure::NotFound => DirectGuestActivationError::NotFound,
        TerminalFailure::Conflict => DirectGuestActivationError::Conflict,
        TerminalFailure::Unavailable | TerminalFailure::ResourceExhausted => {
            DirectGuestActivationError::Unavailable
        }
        TerminalFailure::GenerationMismatch
        | TerminalFailure::Protocol
        | TerminalFailure::InvalidSelection
        | TerminalFailure::Internal => DirectGuestActivationError::Protocol,
    }
}

fn broker_apply_method_id() -> u32 {
    SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.broker.v2")
        .and_then(|service| {
            service
                .methods
                .iter()
                .find(|method| method.name == BROKER_APPLY_METHOD)
        })
        .expect("frozen BrokerService.Apply")
        .method_id("d2b.broker.v2", "BrokerService")
}

fn credit_scopes(limit: u16) -> CreditScopeSet {
    let limit = usize::from(limit.max(1));
    let pool = || CreditPool::new(limit).expect("positive broker credit");
    CreditScopeSet::new(pool(), pool(), pool(), pool(), pool(), pool())
}

fn connect_seqpacket(path: PathBuf) -> Result<OwnedFd, TerminalFailure> {
    if !path.is_absolute() {
        return Err(TerminalFailure::Unavailable);
    }
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
        None,
    )
    .map_err(|_| TerminalFailure::Unavailable)?;
    let address = UnixAddr::new(&path).map_err(|_| TerminalFailure::Unavailable)?;
    connect(fd.as_raw_fd(), &address).map_err(|_| TerminalFailure::Unavailable)?;
    Ok(fd)
}

fn resolve_user(name: &str) -> Result<u32, TerminalFailure> {
    User::from_name(name)
        .map_err(|_| TerminalFailure::Unavailable)?
        .map(|user| user.uid.as_raw())
        .ok_or(TerminalFailure::Unavailable)
}

fn resolve_group(name: &str) -> Result<u32, TerminalFailure> {
    Group::from_name(name)
        .map_err(|_| TerminalFailure::Unavailable)?
        .map(|group| group.gid.as_raw())
        .ok_or(TerminalFailure::Unavailable)
}

fn v2_identity(identity: &WorkloadIdentity) -> Result<(RealmId, WorkloadId), TerminalFailure> {
    let path = RealmPath::parse(format!("{}.local-root", identity.realm_path.target_form()))
        .map_err(|_| TerminalFailure::Protocol)?;
    let realm_id = RealmId::derive(&path);
    let name = WorkloadName::parse(identity.workload_id.as_str())
        .map_err(|_| TerminalFailure::Protocol)?;
    let workload_id = WorkloadId::derive(&realm_id, &name);
    Ok((realm_id, workload_id))
}

fn unix_time_ms() -> Result<u64, TerminalFailure> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| TerminalFailure::Internal)?
        .as_millis()
        .try_into()
        .map_err(|_| TerminalFailure::Internal)
}

async fn cancel_driver_request(driver: &Arc<dyn ComponentSessionDriver>, request_id: [u8; 16]) {
    if let Ok(request_id) = RequestId::new(request_id.to_vec()) {
        let _ = driver.cancel(driver.generation(), request_id).await;
    }
}

async fn pump_broker_ttrpc(
    bridge: tokio::io::DuplexStream,
    driver: Arc<dyn ComponentSessionDriver>,
) {
    pump_ttrpc(bridge, driver, "d2b.broker.v2.BrokerService", |request| {
        if request.method != BROKER_APPLY_METHOD {
            return None;
        }
        decode_strict::<ServiceRequest>(&request.payload, true)
            .ok()
            .and_then(|request| {
                request
                    .metadata
                    .as_ref()
                    .map(|value| value.request_id.clone())
            })
    })
    .await;
}

async fn pump_guest_session_ttrpc(
    bridge: tokio::io::DuplexStream,
    driver: Arc<dyn ComponentSessionDriver>,
) {
    pump_ttrpc(
        bridge,
        driver,
        "d2b.guest.v2.GuestService",
        |request| match request.method.as_str() {
            "Bootstrap" => decode_strict::<GuestBootstrapRequest>(&request.payload, true)
                .ok()
                .and_then(|request| {
                    request
                        .context
                        .as_ref()
                        .and_then(|context| context.metadata.as_ref())
                        .map(|metadata| metadata.request_id.clone())
                }),
            "Reconnect" => decode_strict::<GuestReconnectRequest>(&request.payload, true)
                .ok()
                .and_then(|request| {
                    request
                        .context
                        .as_ref()
                        .and_then(|context| context.metadata.as_ref())
                        .map(|metadata| metadata.request_id.clone())
                }),
            _ => None,
        },
    )
    .await;
}

async fn pump_ttrpc(
    bridge: tokio::io::DuplexStream,
    driver: Arc<dyn ComponentSessionDriver>,
    expected_service: &'static str,
    request_id: fn(&ttrpc::Request) -> Option<Vec<u8>>,
) {
    let (mut reader, mut writer) = tokio::io::split(bridge);
    let in_flight = Arc::new(tokio::sync::Mutex::new(BTreeMap::new()));
    let request_driver = Arc::clone(&driver);
    let request_map = Arc::clone(&in_flight);
    let send = async move {
        loop {
            let (header, frame) = crate::ttrpc_frame::read_ttrpc_frame(
                &mut reader,
                LimitProfile::local_default().logical_ttrpc_bytes,
            )
            .await
            .map_err(|_| ())?
            .ok_or(())?;
            if header.type_ != MESSAGE_TYPE_REQUEST {
                return Err(());
            }
            let request = ttrpc::Request::parse_from_bytes(&frame[MESSAGE_HEADER_LENGTH..])
                .map_err(|_| ())?;
            if request.service != expected_service {
                return Err(());
            }
            let request_id = RequestId::new(request_id(&request).ok_or(())?).map_err(|_| ())?;
            if request_map
                .lock()
                .await
                .insert(header.stream_id, request_id.clone())
                .is_some()
            {
                return Err(());
            }
            request_driver
                .start_ttrpc(request_id, frame)
                .await
                .map_err(|_| ())?;
        }
    };
    let receive = async move {
        loop {
            let frame = driver.receive_ttrpc().await.map_err(|_| ())?;
            let header = ttrpc_frame_header(&frame)?;
            if header.type_ != MESSAGE_TYPE_RESPONSE {
                return Err(());
            }
            let request_id = in_flight.lock().await.remove(&header.stream_id).ok_or(())?;
            if !driver.complete_ttrpc(request_id).await.map_err(|_| ())? {
                return Err(());
            }
            writer.write_all(&frame).await.map_err(|_| ())?;
            writer.flush().await.map_err(|_| ())?;
        }
    };
    let _: Result<(), ()> = tokio::select! {
        result = send => result,
        result = receive => result,
    };
}

fn ttrpc_frame_header(frame: &[u8]) -> Result<MessageHeader, ()> {
    let bytes: [u8; MESSAGE_HEADER_LENGTH] = frame
        .get(..MESSAGE_HEADER_LENGTH)
        .ok_or(())?
        .try_into()
        .map_err(|_| ())?;
    let header = MessageHeader::from(bytes);
    if header.length as usize != frame.len().saturating_sub(MESSAGE_HEADER_LENGTH) {
        return Err(());
    }
    Ok(header)
}

fn digest_field(digest: &mut Sha256, field: &[u8]) {
    digest.update(u32::try_from(field.len()).unwrap_or(u32::MAX).to_be_bytes());
    digest.update(field);
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use d2b_contracts::{
        v2_component_session::{
            BootstrapPskBinding, GuestBootstrapCredentialV1, GuestBootstrapPsk,
            GuestIdentityBindingV1, OperationId,
        },
        v2_guest_configured_launches::{GuestConfiguredLaunchEntryV1, GuestConfiguredLaunchesV1},
        v2_services::{
            broker_ttrpc::{BrokerService, create_broker_service},
            guest_ttrpc::{GuestService, create_guest_service},
        },
    };
    use d2b_core::configured_argv::ConfiguredArgv;
    use d2b_host::guest_runtime::{GUEST_ENROLLMENT_WIRE_PREFIX, GUEST_MATERIAL_WIRE_PREFIX};
    use d2b_realm_core::ProtocolToken;
    use d2b_session::Secret32;
    use d2b_session_unix::FramedVsockTransport;
    use futures::stream;
    use nix::sys::socket::{Backlog, UnixAddr, bind, listen};
    use rustix::net::{SocketFlags as RustixSocketFlags, accept_with};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

    use super::*;

    #[derive(Debug)]
    struct AlwaysActive;

    impl ActiveGuestRuntimeAuthority for AlwaysActive {
        fn validate_active(&self, _: &GuestSessionAuthority) -> Result<(), TerminalFailure> {
            Ok(())
        }
    }

    struct MemoryVsockRuntime {
        bootstrap: Mutex<Option<DuplexStream>>,
        reconnect: Mutex<Option<DuplexStream>>,
        accepts: AtomicUsize,
        connects: AtomicUsize,
    }

    impl MemoryVsockRuntime {
        fn new() -> (Arc<Self>, DuplexStream, DuplexStream) {
            let (bootstrap_host, bootstrap_guest) = tokio::io::duplex(2 * 1024 * 1024);
            let (reconnect_host, reconnect_guest) = tokio::io::duplex(2 * 1024 * 1024);
            (
                Arc::new(Self {
                    bootstrap: Mutex::new(Some(bootstrap_host)),
                    reconnect: Mutex::new(Some(reconnect_host)),
                    accepts: AtomicUsize::new(0),
                    connects: AtomicUsize::new(0),
                }),
                bootstrap_guest,
                reconnect_guest,
            )
        }
    }

    #[async_trait]
    impl VsockRuntime for MemoryVsockRuntime {
        async fn accept_bootstrap(
            &self,
            _: u32,
            _: u32,
        ) -> Result<ErasedVsockTransport, TerminalFailure> {
            self.accepts.fetch_add(1, Ordering::AcqRel);
            self.bootstrap
                .lock()
                .unwrap()
                .take()
                .map(FramedVsockTransport::new)
                .map(ErasedVsockTransport::new)
                .ok_or(TerminalFailure::Conflict)
        }

        async fn connect(&self, _: u32, _: u32) -> Result<ErasedVsockTransport, TerminalFailure> {
            self.connects.fetch_add(1, Ordering::AcqRel);
            self.reconnect
                .lock()
                .unwrap()
                .take()
                .map(FramedVsockTransport::new)
                .map(ErasedVsockTransport::new)
                .ok_or(TerminalFailure::Conflict)
        }
    }

    struct MockGuestSessionService {
        generation: u64,
        parent_public: [u8; 32],
        guest_public: [u8; 32],
        bootstrap: bool,
        driver: Mutex<Option<Arc<dyn ComponentSessionDriver>>>,
    }

    impl MockGuestSessionService {
        fn response(&self, context: &GuestOperationContext) -> GuestSessionResponse {
            let guest_identity: [u8; 32] = Sha256::digest(self.guest_public).into();
            GuestSessionResponse {
                outcome: Outcome::OUTCOME_SUCCEEDED.into(),
                operation_id: context.operation_id.clone(),
                session_generation: self.generation,
                request_id: context
                    .metadata
                    .as_ref()
                    .expect("metadata")
                    .request_id
                    .clone(),
                guest_identity_handle: guest_identity_handle(&guest_identity),
                guest_identity_digest: guest_identity.to_vec(),
                guest_static_public_key: self.guest_public.to_vec(),
                guest_static_public_key_digest: Sha256::digest(self.guest_public).to_vec(),
                parent_static_public_key_digest: Sha256::digest(self.parent_public).to_vec(),
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl GuestService for MockGuestSessionService {
        async fn bootstrap(
            &self,
            _: &ttrpc::r#async::TtrpcContext,
            request: GuestBootstrapRequest,
        ) -> ttrpc::Result<GuestSessionResponse> {
            if !self.bootstrap {
                return Err(ttrpc::Error::RpcStatus(ttrpc::get_status(
                    ttrpc::Code::PERMISSION_DENIED,
                    "wrong phase".to_owned(),
                )));
            }
            Ok(self.response(request.context.as_ref().expect("context")))
        }

        async fn reconnect(
            &self,
            _: &ttrpc::r#async::TtrpcContext,
            request: GuestReconnectRequest,
        ) -> ttrpc::Result<GuestSessionResponse> {
            if self.bootstrap {
                return Err(ttrpc::Error::RpcStatus(ttrpc::get_status(
                    ttrpc::Code::PERMISSION_DENIED,
                    "wrong phase".to_owned(),
                )));
            }
            Ok(self.response(request.context.as_ref().expect("context")))
        }

        async fn inspect_exec(
            &self,
            _: &ttrpc::r#async::TtrpcContext,
            request: d2b_contracts::v2_services::guest::GuestInspectExecRequest,
        ) -> ttrpc::Result<d2b_contracts::v2_services::guest::GuestInspectExecResponse> {
            use d2b_contracts::v2_services::guest::{
                GuestExecState, GuestExecStatus, GuestInspectExecResponse, GuestStdinState,
                guest_inspect_exec_query::Query, guest_inspect_exec_response::Result,
            };
            let context = request.context.as_ref().ok_or_else(mock_rpc_error)?;
            let metadata = context.metadata.as_ref().ok_or_else(mock_rpc_error)?;
            let handle = match request
                .query
                .as_ref()
                .and_then(|query| query.query.as_ref())
            {
                Some(Query::Status(status)) => status.resource_handle.clone(),
                _ => return Err(mock_rpc_error()),
            };
            Ok(GuestInspectExecResponse {
                outcome: Outcome::OUTCOME_SUCCEEDED.into(),
                operation_id: context.operation_id.clone(),
                session_generation: self.generation,
                request_id: metadata.request_id.clone(),
                result: Some(Result::Status(GuestExecStatus {
                    resource_handle: handle,
                    state: GuestExecState::GUEST_EXEC_STATE_RUNNING.into(),
                    stdin_state: GuestStdinState::GUEST_STDIN_STATE_NOT_INTERACTIVE.into(),
                    state_generation: 1,
                    ..Default::default()
                })),
                ..Default::default()
            })
        }

        async fn exec(
            &self,
            _: &ttrpc::r#async::TtrpcContext,
            request: d2b_contracts::v2_services::guest::GuestExecRequest,
        ) -> ttrpc::Result<d2b_contracts::v2_services::terminal::TerminalOpenResponse> {
            use d2b_contracts::v2_services::{
                server_stream_name,
                terminal::{
                    TerminalKind, TerminalOpenResponse, TerminalStarted, TerminalStreamFrame,
                    terminal_stream_frame,
                },
            };
            let terminal = request.terminal.as_ref().ok_or_else(mock_rpc_error)?;
            let metadata = terminal.metadata.as_ref().ok_or_else(mock_rpc_error)?;
            let driver = self
                .driver
                .lock()
                .unwrap()
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(mock_rpc_error)?;
            let stream = d2b_session::StreamId::new(0x100).map_err(|_| mock_rpc_error())?;
            driver
                .open_named_stream(
                    stream,
                    LimitProfile::local_default().named_stream_queue_bytes,
                    LimitProfile::local_default().named_stream_queue_bytes,
                )
                .await
                .map_err(|_| mock_rpc_error())?;
            let operation_id = terminal.operation_id.clone();
            let request_id = metadata.request_id.clone();
            let resource_handle = "exec-1".to_owned();
            let response = TerminalOpenResponse {
                outcome: Outcome::OUTCOME_ACCEPTED.into(),
                operation_id: operation_id.clone(),
                stream_id: server_stream_name(0x100).map_err(|_| mock_rpc_error())?,
                session_generation: self.generation,
                request_id: request_id.clone(),
                resource_handle: resource_handle.clone(),
                ..Default::default()
            };
            let generation = self.generation;
            tokio::spawn(async move {
                let Ok(d2b_session::StreamEvent::Data {
                    stream: selected,
                    bytes,
                }) = driver.receive_named_stream().await
                else {
                    return;
                };
                if selected != stream {
                    return;
                }
                let Ok(consumed) = u32::try_from(bytes.len()) else {
                    return;
                };
                let _ = driver.grant_named_stream_credit(stream, consumed).await;
                let frame = TerminalStreamFrame {
                    session_generation: generation,
                    request_id,
                    sequence: 0,
                    operation_id,
                    resource_handle,
                    frame: Some(terminal_stream_frame::Frame::Started(TerminalStarted {
                        kind: TerminalKind::TERMINAL_KIND_EXEC.into(),
                        tty: false,
                        ..Default::default()
                    })),
                    ..Default::default()
                };
                if let Ok(encoded) = frame.write_to_bytes() {
                    let _ = driver.send_named_stream(stream, encoded).await;
                }
            });
            Ok(response)
        }
    }

    async fn serve_mock_guest(
        driver: Arc<dyn ComponentSessionDriver>,
        service: Arc<dyn GuestService + Send + Sync>,
    ) -> Result<(), ()> {
        let (server_transport, bridge_transport) = tokio::io::duplex(2 * 1024 * 1024);
        let listener = ttrpc::r#async::transport::Listener::new(stream::once(async move {
            Ok::<_, std::io::Error>(server_transport)
        }));
        let mut server = ttrpc::r#async::Server::new()
            .add_listener(listener)
            .register_service(create_guest_service(service));
        server.start().await.map_err(|_| ())?;
        let (mut bridge_reader, mut bridge_writer) = tokio::io::split(bridge_transport);
        let receive_driver = Arc::clone(&driver);
        let receive = async move {
            loop {
                let frame = receive_driver.receive_ttrpc().await.map_err(|_| ())?;
                bridge_writer.write_all(&frame).await.map_err(|_| ())?;
                bridge_writer.flush().await.map_err(|_| ())?;
            }
        };
        let send = async move {
            let mut frame = vec![0_u8; 64 * 1024];
            loop {
                let read = bridge_reader.read(&mut frame).await.map_err(|_| ())?;
                if read == 0 {
                    return Ok::<(), ()>(());
                }
                driver
                    .send_ttrpc(frame[..read].to_vec())
                    .await
                    .map_err(|_| ())?;
            }
        };
        tokio::select! {
            result = receive => result,
            result = send => result,
        }
    }

    struct MockBrokerMaterialService {
        driver: Arc<dyn ComponentSessionDriver>,
        generation: u64,
        credential: Arc<Mutex<GuestSessionCredentialV1>>,
        configured: Arc<Vec<u8>>,
        persists: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl BrokerService for MockBrokerMaterialService {
        async fn apply(
            &self,
            _: &ttrpc::r#async::TtrpcContext,
            request: ServiceRequest,
        ) -> ttrpc::Result<common::ServiceResponse> {
            let metadata = request.metadata.as_ref().ok_or_else(mock_rpc_error)?;
            let request_id =
                RequestId::new(metadata.request_id.clone()).map_err(|_| mock_rpc_error())?;
            if request.resource_id.starts_with(GUEST_MATERIAL_WIRE_PREFIX) {
                let encoded = self
                    .credential
                    .lock()
                    .unwrap()
                    .encode()
                    .map_err(|_| mock_rpc_error())?;
                let session_digest: [u8; 32] = Sha256::digest(encoded.as_slice()).into();
                let configured_digest: [u8; 32] = Sha256::digest(&*self.configured).into();
                let attachments = vec![
                    mock_response_attachment(
                        request_id.clone(),
                        self.generation,
                        0,
                        encoded.as_slice(),
                    )?,
                    mock_response_attachment(request_id, self.generation, 1, &self.configured)?,
                ];
                self.driver
                    .send_attachments(attachments)
                    .await
                    .map_err(|_| mock_rpc_error())?;
                let mut pair = Sha256::new();
                pair.update(b"d2b-guest-material-pair-v1\0");
                pair.update(session_digest);
                pair.update(configured_digest);
                Ok(common::ServiceResponse {
                    outcome: Outcome::OUTCOME_SUCCEEDED.into(),
                    operation_id: request.operation_id,
                    result_digest: pair.finalize().to_vec(),
                    attachment_indexes: vec![0, 1],
                    ..Default::default()
                })
            } else if request
                .resource_id
                .starts_with(GUEST_ENROLLMENT_WIRE_PREFIX)
            {
                let mut attachments = self
                    .driver
                    .receive_attachments()
                    .await
                    .map_err(|_| mock_rpc_error())?;
                if attachments.len() != 1 {
                    return Err(mock_rpc_error());
                }
                let attachment = attachments.remove(0);
                let fd = attachment
                    .payload()
                    .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
                    .and_then(UnixAttachmentPayload::file)
                    .ok_or_else(mock_rpc_error)?
                    .try_clone_to_owned()
                    .map_err(|_| mock_rpc_error())?;
                let mut encoded = Zeroizing::new(Vec::new());
                File::from(fd)
                    .take(MAX_MATERIAL_BYTES + 1)
                    .read_to_end(&mut encoded)
                    .map_err(|_| mock_rpc_error())?;
                let credential =
                    GuestSessionCredentialV1::decode(&encoded).map_err(|_| mock_rpc_error())?;
                let digest: [u8; 32] = Sha256::digest(&encoded).into();
                *self.credential.lock().unwrap() = credential;
                self.persists.fetch_add(1, Ordering::AcqRel);
                Ok(common::ServiceResponse {
                    outcome: Outcome::OUTCOME_SUCCEEDED.into(),
                    operation_id: request.operation_id,
                    result_digest: digest.to_vec(),
                    ..Default::default()
                })
            } else {
                Err(mock_rpc_error())
            }
        }
    }

    fn mock_response_attachment(
        request_id: RequestId,
        generation: u64,
        index: u16,
        bytes: &[u8],
    ) -> ttrpc::Result<OwnedAttachment> {
        let fd = sealed_read_only_memfd("mock-material", bytes).map_err(|_| mock_rpc_error())?;
        let descriptor = AttachmentDescriptor {
            index,
            kind: AttachmentKind::FileDescriptor,
            object_type: KernelObjectType::Memfd,
            access: AttachmentAccess::ReadOnly,
            purpose: d2b_contracts::v2_component_session::AttachmentPurpose::ResponseOutput,
            service: ServicePackage::BrokerV2,
            method_id: broker_apply_method_id(),
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
            .map_err(|_| mock_rpc_error())?,
        };
        let identity =
            ObjectIdentity::from_trusted(&fd, KernelObjectType::Memfd, AttachmentAccess::ReadOnly)
                .map_err(|_| mock_rpc_error())?;
        OwnedUnixAttachment::file(descriptor, fd, DescriptorPolicy::File(identity))
            .map_err(|_| mock_rpc_error())
    }

    async fn serve_mock_broker(
        driver: Arc<dyn ComponentSessionDriver>,
        service: Arc<dyn BrokerService + Send + Sync>,
    ) -> Result<(), ()> {
        let (server_transport, bridge_transport) = tokio::io::duplex(2 * 1024 * 1024);
        let listener = ttrpc::r#async::transport::Listener::new(stream::once(async move {
            Ok::<_, std::io::Error>(server_transport)
        }));
        let mut server = ttrpc::r#async::Server::new()
            .add_listener(listener)
            .register_service(create_broker_service(service));
        server.start().await.map_err(|_| ())?;
        let (mut bridge_reader, mut bridge_writer) = tokio::io::split(bridge_transport);
        let receive_driver = Arc::clone(&driver);
        let receive = async move {
            loop {
                let frame = receive_driver.receive_ttrpc().await.map_err(|_| ())?;
                bridge_writer.write_all(&frame).await.map_err(|_| ())?;
                bridge_writer.flush().await.map_err(|_| ())?;
            }
        };
        let send = async move {
            let mut frame = vec![0_u8; 64 * 1024];
            loop {
                let read = bridge_reader.read(&mut frame).await.map_err(|_| ())?;
                if read == 0 {
                    return Ok::<(), ()>(());
                }
                driver
                    .send_ttrpc(frame[..read].to_vec())
                    .await
                    .map_err(|_| ())?;
            }
        };
        tokio::select! {
            result = receive => result,
            result = send => result,
        }
    }

    fn mock_rpc_error() -> ttrpc::Error {
        ttrpc::Error::RpcStatus(ttrpc::get_status(
            ttrpc::Code::INTERNAL,
            "mock-runtime-failure".to_owned(),
        ))
    }

    fn bind_mock_broker(path: &std::path::Path) -> OwnedFd {
        let fd = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .unwrap();
        bind(fd.as_raw_fd(), &UnixAddr::new(path).unwrap()).unwrap();
        listen(&fd, Backlog::new(4).unwrap()).unwrap();
        fd
    }

    async fn run_mock_broker_listener(
        listener: OwnedFd,
        authority: GuestSessionAuthority,
        credential: Arc<Mutex<GuestSessionCredentialV1>>,
        configured: Arc<Vec<u8>>,
        persists: Arc<AtomicUsize>,
    ) -> Result<(), ()> {
        for _ in 0..3 {
            let listener = listener.try_clone().map_err(|_| ())?;
            let accepted = tokio::task::spawn_blocking(move || {
                accept_with(
                    &listener,
                    RustixSocketFlags::CLOEXEC | RustixSocketFlags::NONBLOCK,
                )
            })
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
            let socket = SeqpacketSocket::from_owned(accepted).map_err(|_| ())?;
            let expected = socket.acceptor_peer_credentials().map_err(|_| ())?;
            let policy = realm_broker_policy(&authority).map_err(|_| ())?;
            let descriptor_resolver: DescriptorPolicyResolver = Arc::new(|descriptor| {
                if descriptor.service == ServicePackage::BrokerV2
                    && descriptor.method_id == broker_apply_method_id()
                    && descriptor.object_type == KernelObjectType::Memfd
                    && descriptor.access == AttachmentAccess::ReadOnly
                    && descriptor.purpose
                        == d2b_contracts::v2_component_session::AttachmentPurpose::RequestInput
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
                credit_scopes(policy.attachment_policy.max_per_session),
                descriptor_resolver,
                PeerIdentityPolicy::accepted(expected),
            )
            .map_err(|_| ())?;
            let engine = SessionEngine::establish_responder(
                transport,
                policy,
                HandshakeCredentials::Nn,
                Instant::now(),
            )
            .await
            .map_err(|_| ())?;
            let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
            let service = Arc::new(MockBrokerMaterialService {
                driver: Arc::clone(&driver),
                generation: authority.controller_generation,
                credential: Arc::clone(&credential),
                configured: Arc::clone(&configured),
                persists: Arc::clone(&persists),
            });
            let _ = serve_mock_broker(driver, service).await;
        }
        Ok(())
    }

    fn test_authority(generation: u64) -> GuestSessionAuthority {
        let realm_path = RealmPath::parse("work.local-root").unwrap();
        let realm_id = RealmId::derive(&realm_path);
        let workload_id = WorkloadId::derive(&realm_id, &WorkloadName::parse("editor").unwrap());
        let broker_realm_id = realm_id.as_str().to_owned();
        let broker_workload_id = workload_id.as_str().to_owned();
        GuestSessionAuthority {
            realm_id,
            workload_id,
            broker_realm_id,
            broker_workload_id,
            broker_endpoint: PathBuf::from("realm-broker"),
            broker_uid: 1001,
            broker_gid: 1001,
            controller_uid: 1000,
            controller_gid: 1000,
            controller_generation: generation,
            workload_name: "editor".to_owned(),
            vsock_cid: 42,
            vsock_port: GUEST_V2_VSOCK_PORT,
            runtime_instance_digest: [0x41; 32],
            direct_schema_fingerprint: guest_service_fingerprint().unwrap(),
        }
    }

    fn configured_launches(authority: &GuestSessionAuthority) -> GuestConfiguredLaunchesV1 {
        GuestConfiguredLaunchesV1::new(
            authority.realm_id.clone(),
            authority.workload_id.clone(),
            [0x51; 32],
            vec![
                GuestConfiguredLaunchEntryV1::new(
                    ProtocolToken::parse("editor").unwrap(),
                    ConfiguredArgv::new(vec!["editor-bin".to_owned()]).unwrap(),
                    false,
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn direct_port_runs_ikpsk2_then_kk_over_in_memory_native_vsock() {
        const GENERATION: u64 = 17;
        let binding = ControllerProcessBinding::for_test("work", GENERATION, 1000, 1000);
        let identity = ControllerIdentityAuthority::from_test_key(binding, [0x31; 32]);
        let parent_public = *identity.require().unwrap().public_key();
        let authority = test_authority(GENERATION);
        let (runtime, bootstrap_guest, reconnect_guest) = MemoryVsockRuntime::new();
        let direct = VsockDirectGuestSessionPort::with_test_runtime(
            identity,
            Arc::new(AlwaysActive),
            runtime.clone(),
        );
        let guest_private = [0x61; 32];
        let guest_public = d2b_session::x25519_public_key(&guest_private).unwrap();
        let operation_id = OperationId::new(vec![0x71; 16]).unwrap();
        let replay_nonce = [0x72; 32];
        let now = unix_time_ms().unwrap();
        let binding = BootstrapPskBinding {
            operation_id,
            replay_nonce,
            expires_at_unix_ms: now + 60_000,
        };
        let mut material_psk = [0x73; 32];
        let bootstrap = GuestBootstrapCredentialV1::new(
            binding.clone(),
            now.saturating_sub(1),
            GuestBootstrapPsk::copy_from_and_zeroize(&mut material_psk).unwrap(),
        )
        .unwrap();
        let channel_binding = guest_runtime_channel_binding(GuestRuntimeChannelBindingInput {
            realm_id: authority.realm_id.as_str(),
            workload_id: authority.workload_id.as_str(),
            controller_generation: GENERATION,
            runtime_instance_digest: &authority.runtime_instance_digest,
            vsock_cid: authority.vsock_cid,
            vsock_port: authority.vsock_port,
            boot_nonce: &replay_nonce,
        });
        let material = AppliedGuestSessionMaterial {
            credential: GuestSessionCredentialV1::new(
                GENERATION,
                parent_public,
                channel_binding,
                GuestIdentityBindingV1::UnboundBootstrap,
                Some(bootstrap),
            )
            .unwrap(),
            configured_launches: configured_launches(&authority),
        };

        let bootstrap_policy = guest_policy(true, GENERATION, channel_binding).unwrap();
        let bootstrap_service = Arc::new(MockGuestSessionService {
            generation: GENERATION,
            parent_public,
            guest_public,
            bootstrap: true,
            driver: Mutex::new(None),
        });
        let bootstrap_task = tokio::spawn(async move {
            let mut transport = FramedVsockTransport::new(bootstrap_guest);
            let evidence = transport.receive(64).await.unwrap();
            assert_eq!(&evidence.as_bytes()[..8], BOOTSTRAP_EVIDENCE_MAGIC);
            assert_eq!(&evidence.as_bytes()[8..24], binding.operation_id.as_bytes());
            assert_eq!(&evidence.as_bytes()[24..], &binding.replay_nonce);
            let mut admission =
                BootstrapAdmission::new(binding.clone(), BootstrapPsk::new([0x73; 32]).unwrap())
                    .unwrap();
            let psk = admission
                .consume(&binding.operation_id, &binding.replay_nonce, now)
                .unwrap();
            let engine = SessionEngine::establish_initiator(
                transport,
                bootstrap_policy,
                HandshakeCredentials::IkPsk2Initiator {
                    local_private: Secret32::new(guest_private).unwrap(),
                    remote_public: parent_public,
                    psk,
                },
                Instant::now(),
            )
            .await
            .unwrap();
            let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
            serve_mock_guest(driver, bootstrap_service).await
        });
        let established = direct.bootstrap(&authority, &material).await.unwrap();
        assert_eq!(established.guest_static_public_key, guest_public);
        assert_eq!(
            established.guest_identity_digest,
            Sha256::digest(guest_public).as_slice()
        );
        direct.close_bootstrap(established.driver).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), bootstrap_task).await;

        let enrolled = AppliedGuestSessionMaterial {
            credential: GuestSessionCredentialV1::new(
                GENERATION,
                parent_public,
                channel_binding,
                GuestIdentityBindingV1::Enrolled {
                    guest_identity_digest: Sha256::digest(guest_public).into(),
                    guest_static_public_key: guest_public,
                },
                None,
            )
            .unwrap(),
            configured_launches: configured_launches(&authority),
        };
        let reconnect_policy = guest_policy(false, GENERATION, channel_binding).unwrap();
        let reconnect_service = Arc::new(MockGuestSessionService {
            generation: GENERATION,
            parent_public,
            guest_public,
            bootstrap: false,
            driver: Mutex::new(None),
        });
        let reconnect_service_for_task = Arc::clone(&reconnect_service);
        let reconnect_task = tokio::spawn(async move {
            let engine = SessionEngine::establish_responder(
                FramedVsockTransport::new(reconnect_guest),
                reconnect_policy,
                HandshakeCredentials::Kk {
                    local_private: Secret32::new(guest_private).unwrap(),
                    remote_public: parent_public,
                },
                Instant::now(),
            )
            .await
            .unwrap();
            let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
            *reconnect_service_for_task.driver.lock().unwrap() = Some(Arc::clone(&driver));
            serve_mock_guest(driver, reconnect_service_for_task).await
        });
        let driver = direct.reconnect(&authority, &enrolled).await.unwrap();
        assert_eq!(driver.generation(), GENERATION);
        let session = crate::guest_terminal::GuestTerminalSession::from_driver(Arc::clone(&driver));
        let inspect_operation = random_operation("inspect").unwrap();
        let inspect_digest = [0x81; 32];
        let inspect_request = d2b_contracts::v2_services::guest::GuestInspectExecRequest {
            context: MessageField::some(
                guest_context(&authority, inspect_operation, inspect_digest).unwrap(),
            ),
            query: MessageField::some(d2b_contracts::v2_services::guest::GuestInspectExecQuery {
                query: Some(
                    d2b_contracts::v2_services::guest::guest_inspect_exec_query::Query::Status(
                        d2b_contracts::v2_services::guest::GuestExecStatusQuery {
                            resource_handle: "exec-1".to_owned(),
                            ..Default::default()
                        },
                    ),
                ),
                ..Default::default()
            }),
            ..Default::default()
        };
        let inspected = crate::guest_terminal::GuestProxySession::inspect_exec(
            session.as_ref(),
            inspect_request,
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(
            inspected.outcome.enum_value().ok(),
            Some(Outcome::OUTCOME_SUCCEEDED)
        );

        use d2b_contracts::v2_services::{
            guest::GuestExecRequest,
            terminal::{
                ArbitraryExecSelection, ExecAuthority, ExecSelection, TerminalOpenRequest,
                TerminalSelection, terminal_selection,
            },
        };
        let exec_operation = random_operation("exec").unwrap();
        let exec_digest = [0x82; 32];
        let exec_request = GuestExecRequest {
            terminal: MessageField::some(TerminalOpenRequest {
                metadata: MessageField::some(
                    request_metadata(GENERATION, exec_operation.request_id, exec_digest).unwrap(),
                ),
                scope: MessageField::some(guest_scope(&authority)),
                resource_id: "exec".to_owned(),
                operation_id: exec_operation.operation_id,
                request_digest: exec_digest.to_vec(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let selection = TerminalSelection {
            selection: Some(terminal_selection::Selection::Exec(ExecSelection {
                authority: ExecAuthority::EXEC_AUTHORITY_ADMIN_ARBITRARY.into(),
                selection: Some(
                    d2b_contracts::v2_services::terminal::exec_selection::Selection::Arbitrary(
                        ArbitraryExecSelection {
                            argv: vec![b"true".to_vec()],
                            ..Default::default()
                        },
                    ),
                ),
                ..Default::default()
            })),
            ..Default::default()
        };
        let opened = session
            .open_exec(exec_request, selection, Duration::from_secs(2))
            .await
            .unwrap();
        match opened {
            crate::daemon_terminal::TerminalOpenResult::Active { started, mut owner } => {
                assert_eq!(
                    started.kind.enum_value().ok(),
                    Some(d2b_contracts::v2_services::terminal::TerminalKind::TERMINAL_KIND_EXEC)
                );
                let _ = owner
                    .finish(crate::daemon_terminal::TerminalFinish::Disconnect)
                    .await;
            }
            _ => panic!("expected active guest exec terminal"),
        }
        session.close_session().await;
        let _ = tokio::time::timeout(Duration::from_secs(2), reconnect_task).await;
        assert_eq!(runtime.accepts.load(Ordering::Acquire), 1);
        assert_eq!(runtime.connects.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn concrete_realm_broker_client_applies_and_persists_enrollment() {
        const GENERATION: u64 = 23;
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let mut authority = test_authority(GENERATION);
        let binding = ControllerProcessBinding::for_test(
            authority.broker_realm_id.clone(),
            GENERATION,
            uid,
            gid,
        );
        let identity = ControllerIdentityAuthority::from_test_key(binding, [0x21; 32]);
        let parent_public = *identity.require().unwrap().public_key();
        let port = BrokerRealmGuestMaterialPort::new(identity);
        authority.controller_uid = uid;
        authority.controller_gid = gid;
        authority.broker_uid = uid;
        authority.broker_gid = gid;

        let root = tempfile::tempdir().unwrap();
        authority.broker_endpoint = root.path().join("realm-broker.sock");
        let listener = bind_mock_broker(&authority.broker_endpoint);
        let replay_nonce = [0x42; 32];
        let channel_binding = guest_runtime_channel_binding(GuestRuntimeChannelBindingInput {
            realm_id: authority.realm_id.as_str(),
            workload_id: authority.workload_id.as_str(),
            controller_generation: GENERATION,
            runtime_instance_digest: &authority.runtime_instance_digest,
            vsock_cid: authority.vsock_cid,
            vsock_port: authority.vsock_port,
            boot_nonce: &replay_nonce,
        });
        let now = unix_time_ms().unwrap();
        let mut psk = [0x43; 32];
        let initial = GuestSessionCredentialV1::new(
            GENERATION,
            parent_public,
            channel_binding,
            GuestIdentityBindingV1::UnboundBootstrap,
            Some(
                GuestBootstrapCredentialV1::new(
                    BootstrapPskBinding {
                        operation_id: OperationId::new(vec![0x44; 16]).unwrap(),
                        replay_nonce,
                        expires_at_unix_ms: now + 60_000,
                    },
                    now.saturating_sub(1),
                    GuestBootstrapPsk::copy_from_and_zeroize(&mut psk).unwrap(),
                )
                .unwrap(),
            ),
        )
        .unwrap();
        let configured = configured_launches(&authority).encode().unwrap();
        let shared_credential = Arc::new(Mutex::new(initial));
        let persists = Arc::new(AtomicUsize::new(0));
        let server = tokio::spawn(run_mock_broker_listener(
            listener,
            authority.clone(),
            Arc::clone(&shared_credential),
            Arc::new(configured.as_slice().to_vec()),
            Arc::clone(&persists),
        ));

        let bootstrap_material = port.apply(&authority).await.unwrap();
        assert!(bootstrap_material.credential.guest_identity_is_unbound());
        let guest_public = d2b_session::x25519_public_key(&[0x45; 32]).unwrap();
        let guest_identity: [u8; 32] = Sha256::digest(guest_public).into();
        let enrolled = GuestSessionCredentialV1::new(
            GENERATION,
            parent_public,
            channel_binding,
            GuestIdentityBindingV1::Enrolled {
                guest_identity_digest: guest_identity,
                guest_static_public_key: guest_public,
            },
            None,
        )
        .unwrap();
        port.persist_enrolled(&authority, enrolled).await.unwrap();
        let current = port.apply(&authority).await.unwrap();
        assert_eq!(
            current.credential.guest_identity_digest(),
            Some(&guest_identity)
        );
        assert_eq!(
            current.credential.guest_static_public_key(),
            Some(&guest_public)
        );
        assert_eq!(persists.load(Ordering::Acquire), 1);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[test]
    fn runtime_bindings_reject_wrong_realm_generation_nonce_and_parent_key() {
        const GENERATION: u64 = 31;
        let authority = test_authority(GENERATION);
        let binding = ControllerProcessBinding::for_test(
            authority.broker_realm_id.clone(),
            GENERATION,
            authority.controller_uid,
            authority.controller_gid,
        );
        let identity = ControllerIdentityAuthority::from_test_key(binding, [0x11; 32]);
        let port = BrokerRealmGuestMaterialPort::new(identity.clone());
        let parent_public = *identity.require().unwrap().public_key();
        let now = unix_time_ms().unwrap();
        let replay_nonce = [0x12; 32];
        let channel_binding = guest_runtime_channel_binding(GuestRuntimeChannelBindingInput {
            realm_id: authority.realm_id.as_str(),
            workload_id: authority.workload_id.as_str(),
            controller_generation: GENERATION,
            runtime_instance_digest: &authority.runtime_instance_digest,
            vsock_cid: authority.vsock_cid,
            vsock_port: authority.vsock_port,
            boot_nonce: &replay_nonce,
        });
        let credential = |parent: [u8; 32], channel: [u8; 32]| {
            let mut psk = [0x13; 32];
            GuestSessionCredentialV1::new(
                GENERATION,
                parent,
                channel,
                GuestIdentityBindingV1::UnboundBootstrap,
                Some(
                    GuestBootstrapCredentialV1::new(
                        BootstrapPskBinding {
                            operation_id: OperationId::new(vec![0x14; 16]).unwrap(),
                            replay_nonce,
                            expires_at_unix_ms: now + 60_000,
                        },
                        now.saturating_sub(1),
                        GuestBootstrapPsk::copy_from_and_zeroize(&mut psk).unwrap(),
                    )
                    .unwrap(),
                ),
            )
            .unwrap()
        };
        let valid = AppliedGuestSessionMaterial {
            credential: credential(parent_public, channel_binding),
            configured_launches: configured_launches(&authority),
        };
        port.validate_material_response(&authority, &identity.require().unwrap(), &valid)
            .unwrap();

        let wrong_nonce = AppliedGuestSessionMaterial {
            credential: credential(parent_public, [0x15; 32]),
            configured_launches: configured_launches(&authority),
        };
        assert_eq!(
            port.validate_material_response(&authority, &identity.require().unwrap(), &wrong_nonce)
                .unwrap_err(),
            TerminalFailure::GenerationMismatch
        );
        let wrong_parent = AppliedGuestSessionMaterial {
            credential: credential([0x16; 32], channel_binding),
            configured_launches: configured_launches(&authority),
        };
        assert_eq!(
            port.validate_material_response(
                &authority,
                &identity.require().unwrap(),
                &wrong_parent
            )
            .unwrap_err(),
            TerminalFailure::GenerationMismatch
        );
        let mut wrong_authority = authority.clone();
        wrong_authority.controller_generation += 1;
        assert_eq!(
            port.validate_controller(&wrong_authority).unwrap_err(),
            TerminalFailure::GenerationMismatch
        );
        wrong_authority = authority;
        wrong_authority.broker_realm_id = "personal".to_owned();
        assert_eq!(
            port.validate_controller(&wrong_authority).unwrap_err(),
            TerminalFailure::GenerationMismatch
        );
    }

    #[test]
    fn activation_request_is_id_only_and_cancel_binding_is_restart_stable() {
        let authority = test_authority(31);
        let intent_id = direct_activation_intent_id(&authority).unwrap();
        let operation_id = "activation-0123456789abcdef0123456789abcdef";
        let switch_script =
            "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-system/bin/switch-to-configuration";
        let payload = encode_direct_activation_payload(
            &intent_id,
            operation_id,
            switch_script,
            DirectGuestActivationMode::Switch,
            5_000,
        )
        .unwrap();
        assert_eq!(&payload[..8], &ACTIVATION_PAYLOAD_MAGIC);
        assert_eq!(
            u32::from_be_bytes(payload[8..12].try_into().unwrap()),
            ACTIVATION_PAYLOAD_SCHEMA_VERSION
        );
        assert_eq!(payload[12], 1);
        assert_eq!(
            u64::from_be_bytes(payload[16..24].try_into().unwrap()),
            5_000
        );
        assert!(payload.ends_with(switch_script.as_bytes()));
        assert!(!payload.starts_with(b"{"));
        let digest = [0x71; 32];
        let request = activation_service_request_with_id(
            &authority,
            &intent_id,
            operation_id,
            digest,
            activation_request_id(operation_id),
            true,
            Duration::from_secs(15),
        )
        .unwrap();
        assert_eq!(request.resource_id, intent_id);
        assert_eq!(request.operation_id, operation_id);
        assert_eq!(request.request_digest, digest);
        assert!(request.stream_id.is_empty());
        assert!(request.attachment_indexes.is_empty());
        let encoded = request.write_to_bytes().unwrap();
        assert!(!encoded.windows(11).any(|window| window == b"/nix/store/"));
        assert_eq!(
            activation_request_id(operation_id),
            activation_request_id(operation_id),
            "a daemon reconnect must target the same activation request"
        );
        assert_ne!(
            activation_request_id(operation_id),
            activation_request_id("activation-fedcba9876543210fedcba9876543210")
        );
    }
}
