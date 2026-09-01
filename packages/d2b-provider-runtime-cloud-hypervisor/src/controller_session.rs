//! External Provider controller ResourceV3 ComponentSession bootstrap.

use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{CONTROLLER_ROLE_REF, PROVIDER_REF};
use d2b_contracts_provider::v3::ControllerTargetKind;
use d2b_contracts_resource::v3::{ResourceRef, ResourceTypeName, identity::ReconnectGeneration};
use d2b_core_controller::{
    AssignmentError, AssignmentScope, AssignmentVerb, CONTROLLER_ASSIGNMENT_STREAM_CREDIT,
    CONTROLLER_ASSIGNMENT_STREAM_ID, ControllerAssignmentExpectation,
    ControllerAssignmentGrantStore,
};
use d2b_session::{HandshakeCredentials, SessionEngine, SessionEvent, StreamEvent, StreamId};
use d2b_session_unix::{
    AncillaryCapacity, CONTROLLER_BOOTSTRAP_TIMEOUT, DescriptorPolicyResolver, PeerIdentityPolicy,
    SeqpacketSocket, UnixSeqpacketTransport, UnixSessionError,
    controller_bootstrap_attachment_policy, controller_credit_scopes,
    controller_resource_endpoint_policy, prearmed_seqpacket_pair,
};
#[cfg(test)]
use d2b_session_unix::{CreditPool, CreditScopeSet};

const CONTROLLER_BOOTSTRAP_FD: i32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControllerSessionError {
    Bootstrap,
    Transport,
    Handshake,
    Receive,
    Keepalive,
    Assignment,
}

impl std::fmt::Display for ControllerSessionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Bootstrap => "controller-bootstrap-failed",
            Self::Transport => "controller-transport-failed",
            Self::Handshake => "controller-handshake-failed",
            Self::Receive => "controller-receive-failed",
            Self::Keepalive => "controller-keepalive-failed",
            Self::Assignment => "controller-assignment-failed",
        })
    }
}

impl std::error::Error for ControllerSessionError {}

pub(crate) fn run_from_fd10() -> i32 {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return crate::RUNTIME_UNAVAILABLE_EXIT,
    };
    match runtime.block_on(async {
        let bootstrap = SeqpacketSocket::from_inherited_fd(CONTROLLER_BOOTSTRAP_FD)
            .map_err(|_| ControllerSessionError::Bootstrap)?;
        run_controller_session(bootstrap).await
    }) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            crate::RUNTIME_UNAVAILABLE_EXIT
        }
    }
}

pub(crate) async fn run_controller_session(
    bootstrap: SeqpacketSocket,
) -> Result<(), ControllerSessionError> {
    let expected_peer = bootstrap
        .acceptor_peer_credentials()
        .map_err(|_| ControllerSessionError::Bootstrap)?;
    let policy = controller_resource_endpoint_policy();
    let (daemon_endpoint, controller_endpoint) =
        prearmed_seqpacket_pair().map_err(|_| ControllerSessionError::Bootstrap)?;
    let controller_socket = SeqpacketSocket::from_parent_prearmed(controller_endpoint)
        .map_err(|_| ControllerSessionError::Bootstrap)?;

    let bootstrap_policy = controller_bootstrap_attachment_policy();
    let bootstrap_capacity = AncillaryCapacity::from_policy(bootstrap_policy)
        .map_err(|_| ControllerSessionError::Bootstrap)?;
    let bootstrap_scopes =
        controller_credit_scopes().map_err(|_| ControllerSessionError::Bootstrap)?;
    let packet = d2b_session_unix::OutboundPacket::with_current_credentials(
        d2b_session_unix::CONTROLLER_BOOTSTRAP_PROTOCOL_MARKER.to_vec(),
        vec![Arc::new(daemon_endpoint)],
        d2b_contracts_zone_session::v3::component_session::LimitProfile::local_default(),
        bootstrap_capacity,
        &bootstrap_scopes,
    )
    .map_err(|_| ControllerSessionError::Bootstrap)?;
    let mut queue = VecDeque::from([packet]);
    let sent = tokio::time::timeout(
        CONTROLLER_BOOTSTRAP_TIMEOUT,
        bootstrap.send_burst(&mut queue, bootstrap_capacity, 1),
    )
    .await
    .map_err(|_| ControllerSessionError::Bootstrap)?
    .map_err(|_| ControllerSessionError::Bootstrap)?;
    if sent.sent.len() != 1 || !queue.is_empty() {
        return Err(ControllerSessionError::Bootstrap);
    }
    for packet in sent.sent {
        packet.acknowledge();
    }
    let poll_interval = Duration::from_millis(u64::from(
        policy
            .limits
            .keepalive_interval_ms
            .min(policy.limits.keepalive_timeout_ms),
    ));
    let transport = controller_transport(controller_socket, &policy, expected_peer)?;
    let mut session = SessionEngine::establish_initiator(
        transport,
        policy,
        HandshakeCredentials::Nn,
        Instant::now(),
    )
    .await
    .map_err(|_| ControllerSessionError::Handshake)?;
    let assignment_stream = StreamId::new(CONTROLLER_ASSIGNMENT_STREAM_ID)
        .map_err(|_| ControllerSessionError::Assignment)?;
    session
        .open_named_stream(
            assignment_stream,
            CONTROLLER_ASSIGNMENT_STREAM_CREDIT,
            CONTROLLER_ASSIGNMENT_STREAM_CREDIT,
        )
        .map_err(|_| ControllerSessionError::Assignment)?;
    let mut assignments = ControllerAssignmentGrantStore::new(
        cloud_assignment_expectation(session.generation())
            .map_err(|_| ControllerSessionError::Assignment)?,
    )
    .map_err(|_| ControllerSessionError::Assignment)?;

    let result = loop {
        match tokio::time::timeout(poll_interval, session.receive()).await {
            Ok(Ok(SessionEvent::Close(_))) => break Ok(()),
            Ok(Ok(SessionEvent::NamedStream(StreamEvent::Data { stream, bytes })))
                if stream == assignment_stream =>
            {
                let byte_count = match u32::try_from(bytes.len()) {
                    Ok(byte_count) => byte_count,
                    Err(_) => break Err(ControllerSessionError::Assignment),
                };
                if assignments.accept_encoded(&bytes).is_err() {
                    break Err(ControllerSessionError::Assignment);
                }
                if session
                    .grant_named_stream_credit(assignment_stream, byte_count)
                    .await
                    .is_err()
                {
                    break Err(ControllerSessionError::Assignment);
                }
            }
            Ok(Ok(SessionEvent::NamedStream(StreamEvent::Reset { stream })))
                if stream == assignment_stream =>
            {
                if session
                    .open_named_stream(
                        assignment_stream,
                        CONTROLLER_ASSIGNMENT_STREAM_CREDIT,
                        CONTROLLER_ASSIGNMENT_STREAM_CREDIT,
                    )
                    .is_err()
                {
                    break Err(ControllerSessionError::Assignment);
                }
            }
            Ok(Ok(SessionEvent::NamedStream(_))) => {
                break Err(ControllerSessionError::Assignment);
            }
            Ok(Ok(_)) | Err(_) => {}
            Ok(Err(_)) => break Err(ControllerSessionError::Receive),
        }
        if session.drive_keepalive(Instant::now()).await.is_err() {
            break Err(ControllerSessionError::Keepalive);
        }
    };
    assignments.revoke();
    result
}

fn cloud_assignment_expectation(
    session_generation: u64,
) -> Result<ControllerAssignmentExpectation, AssignmentError> {
    let resource_types = BTreeSet::from([
        ResourceTypeName::parse("Guest").map_err(|_| AssignmentError::RoleContractInvalid)?
    ]);
    let primary_verbs = BTreeSet::from([
        AssignmentVerb::Get,
        AssignmentVerb::List,
        AssignmentVerb::Watch,
        AssignmentVerb::Create,
        AssignmentVerb::UpdateStatus,
        AssignmentVerb::UpdateFinalizers,
        AssignmentVerb::CommitBatch,
    ]);
    let owner_child_process_verbs = BTreeSet::from([
        AssignmentVerb::Create,
        AssignmentVerb::UpdateSpec,
        AssignmentVerb::Delete,
    ]);
    let scopes = BTreeSet::from([AssignmentScope::Primary, AssignmentScope::OwnerChildProcess]);
    ControllerAssignmentExpectation::new_for_session_with_target_kind(
        ResourceRef::parse(PROVIDER_REF).map_err(|_| AssignmentError::RoleContractInvalid)?,
        ResourceRef::parse(CONTROLLER_ROLE_REF)
            .map_err(|_| AssignmentError::RoleContractInvalid)?,
        ControllerTargetKind::Host,
        ReconnectGeneration::new(session_generation)
            .map_err(|_| AssignmentError::RoleContractInvalid)?,
        resource_types,
        primary_verbs,
        owner_child_process_verbs,
        scopes,
    )
}

fn controller_transport(
    socket: SeqpacketSocket,
    policy: &d2b_contracts_zone_session::v3::component_session::EndpointPolicy,
    expected_peer: d2b_session_unix::PeerCredentials,
) -> Result<UnixSeqpacketTransport, ControllerSessionError> {
    let resolver: DescriptorPolicyResolver =
        Arc::new(|_| Err(UnixSessionError::DescriptorMismatch));
    UnixSeqpacketTransport::new(
        socket,
        policy.transport_binding.locality,
        policy.limits,
        policy.attachment_policy,
        controller_credit_scopes().map_err(|_| ControllerSessionError::Transport)?,
        resolver,
        PeerIdentityPolicy::inherited_socketpair(expected_peer),
    )
    .map_err(|_| ControllerSessionError::Transport)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_manifest;
    use d2b_contracts_provider::v3::ControllerInstanceScope;
    use d2b_contracts_resource::v3::{
        ControllerGeneration, PlacementTargetKind, ResourceEnvelope, ResourceGeneration,
        ResourceRef, identity::ReconnectGeneration,
    };
    use d2b_core_controller::{
        AssignmentRequest, AssignmentTarget, ControllerAssignmentRegistry, ControllerRoleContract,
    };
    use d2b_session::{ComponentSessionDriver, SessionEngine};
    use d2b_session_unix::ReceivedPacket;

    async fn receive_bootstrap(
        socket: &SeqpacketSocket,
    ) -> Result<(SeqpacketSocket, d2b_session_unix::PeerCredentials), UnixSessionError> {
        let policy = controller_bootstrap_attachment_policy();
        let capacity = AncillaryCapacity::from_policy(policy).unwrap();
        let scopes = CreditScopeSet::new(
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
        );
        let mut burst = socket
            .recv_burst(
                d2b_contracts_zone_session::v3::component_session::LimitProfile::local_default(),
                capacity,
                &scopes,
                2,
            )
            .await?;
        assert_eq!(burst.packets.len(), 1);
        let packet: ReceivedPacket = burst.packets.pop().unwrap();
        assert_eq!(
            packet.payload(),
            d2b_session_unix::CONTROLLER_BOOTSTRAP_PROTOCOL_MARKER
        );
        let (fd, credentials) = packet.into_single_file_and_credentials()?;
        let socket = SeqpacketSocket::from_parent_prearmed(fd)?;
        assert_eq!(socket.acceptor_peer_credentials()?, credentials);
        Ok((socket, credentials))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn controller_sends_bootstrap_endpoint_before_establishing_resource_session() {
        let (controller_fd, daemon_fd) = prearmed_seqpacket_pair().unwrap();
        let controller_socket = SeqpacketSocket::from_parent_prearmed(controller_fd).unwrap();
        let daemon_socket = SeqpacketSocket::from_parent_prearmed(daemon_fd).unwrap();
        let controller_task = tokio::spawn(run_controller_session(controller_socket));

        let (resource_socket, credentials) = match receive_bootstrap(&daemon_socket).await {
            Ok(value) => value,
            Err(error) => {
                let result = controller_task.await.expect("controller task join");
                panic!("bootstrap receive failed: {error}; controller result: {result:?}");
            }
        };
        let policy = controller_resource_endpoint_policy();
        let transport = controller_transport(resource_socket, &policy, credentials).unwrap();
        let responder = tokio::time::timeout(
            Duration::from_secs(1),
            SessionEngine::establish_responder(
                transport,
                policy,
                HandshakeCredentials::Nn,
                Instant::now(),
            ),
        )
        .await
        .expect("controller handshake deadline")
        .expect("controller handshake");
        drop(responder);
        drop(daemon_socket);
        assert!(!controller_task.is_finished());
        controller_task.abort();
        let _ = controller_task.await;
    }

    #[test]
    fn controller_session_policy_is_resource_v3_and_inherited_socketpair_only() {
        let policy = controller_resource_endpoint_policy();
        assert_eq!(
            policy.service,
            d2b_contracts_zone_session::v3::component_session::ServicePackage::ResourceV3
        );
        assert_eq!(
            policy.transport_binding.transport,
            d2b_contracts_zone_session::v3::component_session::TransportClass::InheritedSocketpair
        );
        assert_eq!(
            policy.initiator_role,
            d2b_contracts_zone_session::v3::component_session::EndpointRole::Provider
        );
    }

    #[test]
    fn cloud_hypervisor_assignment_contract_is_fixed_to_host_target() {
        let role = ControllerRoleContract::from_signed_manifest(
            ResourceRef::parse(PROVIDER_REF).unwrap(),
            ResourceRef::parse(CONTROLLER_ROLE_REF).unwrap(),
            &provider_manifest().unwrap(),
        )
        .unwrap();
        assert_eq!(role.scope(), ControllerInstanceScope::FixedExecutionTarget);
        assert_eq!(
            role.resource_types(),
            &BTreeSet::from([ResourceTypeName::parse("Guest").unwrap()])
        );
    }

    #[test]
    fn provider_manifest_is_the_packaged_canonical_contract() {
        let packaged = include_bytes!("../provider-manifest.json");
        let manifest = provider_manifest().expect("packaged Provider manifest");
        assert_eq!(
            d2b_contracts_resource::v3::canonical_json_bytes(&manifest)
                .expect("canonical Provider manifest"),
            packaged
        );
    }

    #[test]
    fn cloud_assignment_expectation_separates_primary_and_owner_child_verbs() {
        let expectation = cloud_assignment_expectation(1).unwrap();
        assert!(
            !expectation
                .primary_verbs()
                .contains(&AssignmentVerb::UpdateSpec)
        );
        assert!(
            !expectation
                .primary_verbs()
                .contains(&AssignmentVerb::Delete)
        );
        assert_eq!(
            expectation.owner_child_process_verbs(),
            &BTreeSet::from([
                AssignmentVerb::Create,
                AssignmentVerb::UpdateSpec,
                AssignmentVerb::Delete,
            ])
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn controller_receives_idempotent_assignment_over_authenticated_session() {
        let (controller_fd, daemon_fd) = prearmed_seqpacket_pair().unwrap();
        let controller_socket = SeqpacketSocket::from_parent_prearmed(controller_fd).unwrap();
        let daemon_socket = SeqpacketSocket::from_parent_prearmed(daemon_fd).unwrap();
        let controller_task = tokio::spawn(run_controller_session(controller_socket));

        let (resource_socket, credentials) = receive_bootstrap(&daemon_socket).await.unwrap();
        let policy = controller_resource_endpoint_policy();
        let transport = controller_transport(resource_socket, &policy, credentials).unwrap();
        let responder = SessionEngine::establish_responder(
            transport,
            policy.clone(),
            HandshakeCredentials::Nn,
            Instant::now(),
        )
        .await
        .unwrap();
        let responder = responder.into_driver();
        let stream = StreamId::new(CONTROLLER_ASSIGNMENT_STREAM_ID).unwrap();
        responder
            .open_named_stream(
                stream,
                CONTROLLER_ASSIGNMENT_STREAM_CREDIT,
                CONTROLLER_ASSIGNMENT_STREAM_CREDIT,
            )
            .await
            .unwrap();

        let resource = guest_resource();
        let manifest = provider_manifest().unwrap();
        let role = ControllerRoleContract::from_signed_manifest(
            ResourceRef::parse(PROVIDER_REF).unwrap(),
            ResourceRef::parse(CONTROLLER_ROLE_REF).unwrap(),
            &manifest,
        )
        .unwrap();
        let target = AssignmentTarget::Execution {
            kind: PlacementTargetKind::Host,
            reference: ResourceRef::parse("Host/host-system").unwrap(),
        };
        let mut registry = ControllerAssignmentRegistry::default();
        let lease = registry
            .admit(
                AssignmentRequest::new(
                    &resource,
                    &role,
                    ResourceGeneration::new(7).unwrap(),
                    ControllerGeneration::new(8).unwrap(),
                    ReconnectGeneration::new(policy.reconnect_generation).unwrap(),
                    true,
                )
                .with_expected_target(target),
            )
            .unwrap();
        let bytes = lease.assignment_grant().encode().unwrap();
        responder
            .send_named_stream(stream, bytes.clone())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!controller_task.is_finished());
        responder
            .send_named_stream(stream, bytes.clone())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!controller_task.is_finished());
        responder.reset_named_stream(stream).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!controller_task.is_finished());
        responder
            .open_named_stream(
                stream,
                CONTROLLER_ASSIGNMENT_STREAM_CREDIT,
                CONTROLLER_ASSIGNMENT_STREAM_CREDIT,
            )
            .await
            .unwrap();
        responder
            .send_named_stream(stream, bytes.clone())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!controller_task.is_finished());
        let revocation = d2b_core_controller::ControllerAssignmentGrant::encode_revocation(
            lease.provider_ref(),
            lease.identity(),
        )
        .unwrap();
        responder
            .send_named_stream(stream, revocation)
            .await
            .unwrap();
        responder.send_named_stream(stream, bytes).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(controller_task.is_finished());
        assert_eq!(
            controller_task.await.unwrap(),
            Err(ControllerSessionError::Assignment)
        );
    }

    fn guest_resource() -> ResourceEnvelope {
        let value = serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Guest",
            "metadata": {
                "name": "guest",
                "zone": "dev",
                "uid": "123e4567-e89b-42d3-a456-426614174000",
                "generation": 1,
                "revision": 7,
                "ownerRef": null,
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "2026-07-22T00:00:00.000Z",
                "updatedAt": "2026-07-22T00:00:00.000Z",
                "managedBy": "api",
                "configurationGeneration": null,
                "controllerGeneration": null,
                "providerGeneration": null
            },
            "spec": {
                "providerRef": "Provider/runtime-cloud-hypervisor",
                "executionRef": "Host/host-system"
            },
            "status": {
                "completedAt": null,
                "conditions": [],
                "lastReconciledAt": null,
                "observedGeneration": 0,
                "outcome": null,
                "phase": "Pending",
                "resource": {},
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
            }
        });
        ResourceEnvelope::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
    }
}
