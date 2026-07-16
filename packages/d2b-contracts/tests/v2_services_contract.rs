#![cfg(feature = "v2-services")]

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::v2_component_session::BoundedVec;
use d2b_contracts::v2_identity::{ProviderId, ProviderType, RealmId, WorkloadId};
use d2b_contracts::v2_provider::{
    AdoptionState, AudioChannel, AudioDirection, AuthorizedProviderScope, CorrelationId,
    Fingerprint, Generation, IdempotencyKey, InfrastructurePowerState,
    OBSERVABILITY_RECORD_ENCODED_UPPER_BOUND_BYTES, ObservabilityExportFormat, ObservabilityLabels,
    ObservabilityMetricLabel, ObservabilityOperationLabel, ObservabilityOutcomeLabel,
    ObservabilityProjectionKind, ObservabilityQueryResult, ObservabilityRecord, ObservabilityView,
    ObservationReason, ObservedLifecycleState, OperationId, PROVIDER_SCHEMA_VERSION, PrincipalRef,
    ProviderCapability, ProviderHealth, ProviderHealthReason, ProviderHealthState, ProviderMethod,
    ProviderObservation, ProviderOperationContext, ProviderOperationInput,
    ProviderOperationRequest, ProviderRemediation, ProviderTarget,
};
use d2b_contracts::v2_services::{
    SERVICE_INVENTORY, SERVICE_PACKAGES, ServiceContractError, ServiceInventoryDocument,
    StrictWireMessage, broker, common, decode_strict, encode_strict,
    observability_query_response_from_wire, observability_query_result_to_wire,
    provider_method_for_capability, provider_operation_input, service_inventory_document,
    validate_provider_response_for_method,
};
use protobuf::{Enum, EnumOrUnknown, MessageField};

const TTRPC_SOURCES: &[(&str, &str, &str)] = &[
    (
        "d2b.daemon.v2",
        "DaemonService",
        include_str!("../src/generated_v2_services/daemon_ttrpc.rs"),
    ),
    (
        "d2b.realm.v2",
        "RealmService",
        include_str!("../src/generated_v2_services/realm_ttrpc.rs"),
    ),
    (
        "d2b.guest.v2",
        "GuestService",
        include_str!("../src/generated_v2_services/guest_ttrpc.rs"),
    ),
    (
        "d2b.provider.v2",
        "RuntimeProviderService",
        include_str!("../src/generated_v2_services/provider_runtime_ttrpc.rs"),
    ),
    (
        "d2b.provider.v2",
        "InfrastructureProviderService",
        include_str!("../src/generated_v2_services/provider_infrastructure_ttrpc.rs"),
    ),
    (
        "d2b.provider.v2",
        "TransportProviderService",
        include_str!("../src/generated_v2_services/provider_transport_ttrpc.rs"),
    ),
    (
        "d2b.provider.v2",
        "SubstrateProviderService",
        include_str!("../src/generated_v2_services/provider_substrate_ttrpc.rs"),
    ),
    (
        "d2b.provider.v2",
        "CredentialProviderService",
        include_str!("../src/generated_v2_services/provider_credential_ttrpc.rs"),
    ),
    (
        "d2b.provider.v2",
        "DisplayProviderService",
        include_str!("../src/generated_v2_services/provider_display_ttrpc.rs"),
    ),
    (
        "d2b.provider.v2",
        "NetworkProviderService",
        include_str!("../src/generated_v2_services/provider_network_ttrpc.rs"),
    ),
    (
        "d2b.provider.v2",
        "StorageProviderService",
        include_str!("../src/generated_v2_services/provider_storage_ttrpc.rs"),
    ),
    (
        "d2b.provider.v2",
        "DeviceProviderService",
        include_str!("../src/generated_v2_services/provider_device_ttrpc.rs"),
    ),
    (
        "d2b.provider.v2",
        "AudioProviderService",
        include_str!("../src/generated_v2_services/provider_audio_ttrpc.rs"),
    ),
    (
        "d2b.provider.v2",
        "ObservabilityProviderService",
        include_str!("../src/generated_v2_services/provider_observability_ttrpc.rs"),
    ),
    (
        "d2b.broker.v2",
        "BrokerService",
        include_str!("../src/generated_v2_services/broker_ttrpc.rs"),
    ),
    (
        "d2b.user.v2",
        "UserService",
        include_str!("../src/generated_v2_services/user_ttrpc.rs"),
    ),
    (
        "d2b.runtime.systemd-user.v2",
        "RuntimeSystemdUserService",
        include_str!("../src/generated_v2_services/runtime_systemd_user_ttrpc.rs"),
    ),
    (
        "d2b.shell.v2",
        "ShellService",
        include_str!("../src/generated_v2_services/shell_ttrpc.rs"),
    ),
    (
        "d2b.clipboard.v2",
        "ClipboardService",
        include_str!("../src/generated_v2_services/clipboard_ttrpc.rs"),
    ),
    (
        "d2b.clipboard.picker.v2",
        "ClipboardPickerService",
        include_str!("../src/generated_v2_services/clipboard_picker_ttrpc.rs"),
    ),
    (
        "d2b.notify.v2",
        "NotifyService",
        include_str!("../src/generated_v2_services/notify_ttrpc.rs"),
    ),
    (
        "d2b.security-key.v2",
        "SecurityKeyService",
        include_str!("../src/generated_v2_services/security_key_ttrpc.rs"),
    ),
    (
        "d2b.wayland.v2",
        "WaylandService",
        include_str!("../src/generated_v2_services/wayland_ttrpc.rs"),
    ),
    (
        "d2b.activation.v2",
        "ActivationService",
        include_str!("../src/generated_v2_services/activation_ttrpc.rs"),
    ),
    (
        "d2b.tty.v2",
        "TtyService",
        include_str!("../src/generated_v2_services/tty_ttrpc.rs"),
    ),
];

const PROTO_SOURCES: &[&str] = &[
    include_str!("../proto/v2/common.proto"),
    include_str!("../proto/v2/activation.proto"),
    include_str!("../proto/v2/broker.proto"),
    include_str!("../proto/v2/clipboard.proto"),
    include_str!("../proto/v2/clipboard_picker.proto"),
    include_str!("../proto/v2/daemon.proto"),
    include_str!("../proto/v2/guest.proto"),
    include_str!("../proto/v2/notify.proto"),
    include_str!("../proto/v2/provider_audio.proto"),
    include_str!("../proto/v2/provider_credential.proto"),
    include_str!("../proto/v2/provider_device.proto"),
    include_str!("../proto/v2/provider_display.proto"),
    include_str!("../proto/v2/provider_infrastructure.proto"),
    include_str!("../proto/v2/provider_network.proto"),
    include_str!("../proto/v2/provider_observability.proto"),
    include_str!("../proto/v2/provider_runtime.proto"),
    include_str!("../proto/v2/provider_storage.proto"),
    include_str!("../proto/v2/provider_substrate.proto"),
    include_str!("../proto/v2/provider_transport.proto"),
    include_str!("../proto/v2/realm.proto"),
    include_str!("../proto/v2/runtime_systemd_user.proto"),
    include_str!("../proto/v2/security_key.proto"),
    include_str!("../proto/v2/shell.proto"),
    include_str!("../proto/v2/tty.proto"),
    include_str!("../proto/v2/user.proto"),
    include_str!("../proto/v2/wayland.proto"),
];

fn generated_methods(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("methods.insert(\"")?
                .split_once("\"")
                .map(|(name, _)| name.to_owned())
        })
        .collect()
}

#[test]
fn package_service_and_method_inventory_is_exact() {
    assert_eq!(SERVICE_PACKAGES.len(), 15);
    assert_eq!(SERVICE_INVENTORY.len(), TTRPC_SOURCES.len());
    let inventory_packages: BTreeSet<_> = SERVICE_INVENTORY
        .iter()
        .map(|service| service.package)
        .collect();
    assert_eq!(inventory_packages, SERVICE_PACKAGES.into_iter().collect());

    for service in SERVICE_INVENTORY {
        let (_, _, source) = TTRPC_SOURCES
            .iter()
            .find(|(package, name, _)| *package == service.package && *name == service.service)
            .expect("every inventory service has generated bindings");
        assert!(source.contains(&format!("\"{}.{}\"", service.package, service.service)));
        let expected: Vec<_> = service
            .methods
            .iter()
            .map(|method| method.name.to_owned())
            .collect();
        assert_eq!(
            generated_methods(source),
            expected,
            "{}.{}",
            service.package,
            service.service
        );
    }
}

#[test]
fn generated_bindings_are_async_only_and_compile_as_traits_and_clients() {
    fn service<T: ?Sized + Sync>() {}
    fn client<T: Clone>() {}

    use d2b_contracts::v2_services::*;
    service::<dyn daemon_ttrpc::DaemonService>();
    client::<daemon_ttrpc::DaemonServiceClient>();
    service::<dyn realm_ttrpc::RealmService>();
    client::<realm_ttrpc::RealmServiceClient>();
    service::<dyn guest_ttrpc::GuestService>();
    client::<guest_ttrpc::GuestServiceClient>();
    service::<dyn broker_ttrpc::BrokerService>();
    client::<broker_ttrpc::BrokerServiceClient>();
    service::<dyn user_ttrpc::UserService>();
    client::<user_ttrpc::UserServiceClient>();
    service::<dyn runtime_systemd_user_ttrpc::RuntimeSystemdUserService>();
    client::<runtime_systemd_user_ttrpc::RuntimeSystemdUserServiceClient>();
    service::<dyn shell_ttrpc::ShellService>();
    client::<shell_ttrpc::ShellServiceClient>();
    service::<dyn clipboard_ttrpc::ClipboardService>();
    client::<clipboard_ttrpc::ClipboardServiceClient>();
    service::<dyn clipboard_picker_ttrpc::ClipboardPickerService>();
    client::<clipboard_picker_ttrpc::ClipboardPickerServiceClient>();
    service::<dyn notify_ttrpc::NotifyService>();
    client::<notify_ttrpc::NotifyServiceClient>();
    service::<dyn security_key_ttrpc::SecurityKeyService>();
    client::<security_key_ttrpc::SecurityKeyServiceClient>();
    service::<dyn wayland_ttrpc::WaylandService>();
    client::<wayland_ttrpc::WaylandServiceClient>();
    service::<dyn activation_ttrpc::ActivationService>();
    client::<activation_ttrpc::ActivationServiceClient>();
    service::<dyn tty_ttrpc::TtyService>();
    client::<tty_ttrpc::TtyServiceClient>();
    service::<dyn provider_runtime_ttrpc::RuntimeProviderService>();
    client::<provider_runtime_ttrpc::RuntimeProviderServiceClient>();
    service::<dyn provider_infrastructure_ttrpc::InfrastructureProviderService>();
    client::<provider_infrastructure_ttrpc::InfrastructureProviderServiceClient>();
    service::<dyn provider_transport_ttrpc::TransportProviderService>();
    client::<provider_transport_ttrpc::TransportProviderServiceClient>();
    service::<dyn provider_substrate_ttrpc::SubstrateProviderService>();
    client::<provider_substrate_ttrpc::SubstrateProviderServiceClient>();
    service::<dyn provider_credential_ttrpc::CredentialProviderService>();
    client::<provider_credential_ttrpc::CredentialProviderServiceClient>();
    service::<dyn provider_display_ttrpc::DisplayProviderService>();
    client::<provider_display_ttrpc::DisplayProviderServiceClient>();
    service::<dyn provider_network_ttrpc::NetworkProviderService>();
    client::<provider_network_ttrpc::NetworkProviderServiceClient>();
    service::<dyn provider_storage_ttrpc::StorageProviderService>();
    client::<provider_storage_ttrpc::StorageProviderServiceClient>();
    service::<dyn provider_device_ttrpc::DeviceProviderService>();
    client::<provider_device_ttrpc::DeviceProviderServiceClient>();
    service::<dyn provider_audio_ttrpc::AudioProviderService>();
    client::<provider_audio_ttrpc::AudioProviderServiceClient>();
    service::<dyn provider_observability_ttrpc::ObservabilityProviderService>();
    client::<provider_observability_ttrpc::ObservabilityProviderServiceClient>();

    for (_, _, source) in TTRPC_SOURCES {
        assert!(source.contains("::ttrpc::r#async::Client"));
        assert!(source.contains("#[async_trait]"));
        assert!(source.contains("pub async fn"));
        assert!(!source.contains("::ttrpc::Client"));
        assert!(!source.contains("::ttrpc::sync_client_request!"));
    }
}

fn valid_request() -> common::ServiceRequest {
    let mut metadata = common::RequestMetadata::new();
    metadata.request_id = vec![0x11; 16];
    metadata.correlation_id = "correlation-1".to_owned();
    metadata.trace_id = vec![0x22; 16];
    metadata.idempotency_key = vec![0x33; 16];
    metadata.issued_at_unix_ms = 1_000;
    metadata.expires_at_unix_ms = 2_000;
    metadata.session_generation = 1;
    let mut scope = common::IdentityScope::new();
    scope.realm_id = "aaaaaaaaaaaaaaaaaaaa".to_owned();
    let mut request = common::ServiceRequest::new();
    request.metadata = MessageField::some(metadata);
    request.scope = MessageField::some(scope);
    request
}

fn valid_provider_request() -> common::ProviderRequest {
    let request = valid_request();
    let mut context = common::ProviderOperationContext::new();
    context.metadata = request.metadata;
    context.scope = request.scope;
    context.operation_id = "operation-1".to_owned();
    context.provider_id = "caaaaaaaaaaaaaaaaaaq".to_owned();
    context.provider_type = common::ProviderType::PROVIDER_TYPE_RUNTIME.into();
    context.provider_generation = 1;
    context.policy_epoch = 1;
    context.authorization_digest = vec![0x44; 32];
    context.request_digest = vec![0x55; 32];
    let mut input = common::ProviderOperationInput::new();
    input.set_no_input(common::NoProviderOperationInput::new());
    common::ProviderRequest {
        context: MessageField::some(context),
        input: MessageField::some(input),
        ..Default::default()
    }
}

fn valid_allocate_request() -> broker::AllocateRequest {
    let request = valid_request();
    let mut owner = broker::LeaseOwner::new();
    owner.realm_path = "work".to_owned();
    owner.controller_generation_id = "controller-generation-1".to_owned();
    let mut order = broker::ResourceAcquisitionOrder::new();
    order.phase = 1;
    order.ordinal = 2;
    let mut resource = broker::LeaseResourceRequest::new();
    resource.resource_id = "resource-bridge-1".to_owned();
    resource.kind = broker::HostResourceKind::HOST_RESOURCE_KIND_BRIDGE.into();
    resource.share = broker::ResourceShareMode::RESOURCE_SHARE_MODE_EXCLUSIVE.into();
    resource.acquisition_order = MessageField::some(order);
    broker::AllocateRequest {
        metadata: request.metadata,
        scope: request.scope,
        operation_id: "operation-allocate-1".to_owned(),
        owner: MessageField::some(owner),
        resources: vec![resource],
        request_digest: vec![0x66; 32],
        ..Default::default()
    }
}

fn valid_allocate_response() -> broker::AllocateResponse {
    let mut order = broker::ResourceAcquisitionOrder::new();
    order.phase = 1;
    order.ordinal = 2;
    let mut resource = broker::GrantedHostResource::new();
    resource.resource_id = "resource-bridge-1".to_owned();
    resource.kind = broker::HostResourceKind::HOST_RESOURCE_KIND_BRIDGE.into();
    resource.share = broker::ResourceShareMode::RESOURCE_SHARE_MODE_EXCLUSIVE.into();
    resource.delegation =
        broker::ResourceDelegationKind::RESOURCE_DELEGATION_KIND_FILE_DESCRIPTOR.into();
    resource.delegation_id = "delegation-bridge-1".to_owned();
    resource.acquisition_order = MessageField::some(order);
    resource.attachment_index = Some(0);
    broker::AllocateResponse {
        outcome: common::Outcome::OUTCOME_SUCCEEDED.into(),
        operation_id: "operation-allocate-1".to_owned(),
        status: broker::AllocationStatus::ALLOCATION_STATUS_GRANTED.into(),
        lease_id: "lease-1".to_owned(),
        resources: vec![resource],
        ..Default::default()
    }
}

fn child_fd(
    role: broker::RealmChildRole,
    kind: broker::RealmChildFdKind,
    attachment_index: u32,
) -> broker::RealmChildFd {
    broker::RealmChildFd {
        role: role.into(),
        kind: kind.into(),
        attachment_index,
        ..Default::default()
    }
}

fn valid_spawn_request() -> broker::SpawnRealmChildrenRequest {
    let request = valid_request();
    broker::SpawnRealmChildrenRequest {
        metadata: request.metadata,
        scope: request.scope,
        operation_id: "operation-spawn-1".to_owned(),
        realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
        controller_generation_id: "controller-generation-1".to_owned(),
        controller_process_id: "process-controller-1".to_owned(),
        broker_process_id: "process-broker-1".to_owned(),
        launch_record_digest: vec![0x77; 32],
        fds: vec![
            child_fd(
                broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER,
                broker::RealmChildFdKind::REALM_CHILD_FD_KIND_PUBLIC_LISTENER,
                0,
            ),
            child_fd(
                broker::RealmChildRole::REALM_CHILD_ROLE_BROKER,
                broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BROKER_LISTENER,
                1,
            ),
            child_fd(
                broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER,
                broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION,
                2,
            ),
            child_fd(
                broker::RealmChildRole::REALM_CHILD_ROLE_BROKER,
                broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION,
                3,
            ),
        ],
        ..Default::default()
    }
}

fn valid_spawn_response() -> broker::SpawnRealmChildrenResponse {
    let child = |role: broker::RealmChildRole,
                 process_id: &str,
                 attachment: u32|
     -> broker::SpawnedRealmChild {
        broker::SpawnedRealmChild {
            role: role.into(),
            process_id: process_id.to_owned(),
            pidfd_attachment_index: attachment,
            executable_digest: vec![0x88; 32],
            ..Default::default()
        }
    };
    broker::SpawnRealmChildrenResponse {
        outcome: common::Outcome::OUTCOME_SUCCEEDED.into(),
        operation_id: "operation-spawn-1".to_owned(),
        launch_record_digest: vec![0x77; 32],
        children: vec![
            child(
                broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER,
                "process-controller-1",
                0,
            ),
            child(
                broker::RealmChildRole::REALM_CHILD_ROLE_BROKER,
                "process-broker-1",
                1,
            ),
        ],
        ..Default::default()
    }
}

fn canonical_observability_request() -> ProviderOperationRequest {
    let realm_id = RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap();
    let workload_id = WorkloadId::parse("bbbbbbbbbbbbbbbbbbba").unwrap();
    ProviderOperationRequest {
        context: ProviderOperationContext {
            schema_version: PROVIDER_SCHEMA_VERSION,
            operation_id: OperationId::parse("operation-query").unwrap(),
            idempotency_key: IdempotencyKey::parse("idempotency-query").unwrap(),
            request_digest: Fingerprint::parse("1".repeat(64)).unwrap(),
            scope: AuthorizedProviderScope::Workload {
                realm_id: realm_id.clone(),
                workload_id: workload_id.clone(),
            },
            principal: PrincipalRef::parse("principal-query").unwrap(),
            provider_id: ProviderId::parse("caaaaaaaaaaaaaaaaaaq").unwrap(),
            provider_type: ProviderType::Observability,
            provider_generation: Generation::new(7).unwrap(),
            capability: ProviderCapability(ProviderMethod::ObservabilityQuery),
            method: ProviderMethod::ObservabilityQuery,
            policy_epoch: Generation::new(1).unwrap(),
            authorization_decision_digest: Fingerprint::parse("2".repeat(64)).unwrap(),
            issued_at_unix_ms: 1_000,
            expires_at_unix_ms: 5_000,
            correlation_id: CorrelationId::parse("correlation-query").unwrap(),
            trace_id: Fingerprint::parse("3".repeat(64)).unwrap(),
        },
        target: ProviderTarget::Workload {
            realm_id,
            workload_id,
        },
        expected_configuration_fingerprint: Fingerprint::parse("4".repeat(64)).unwrap(),
        input: ProviderOperationInput::ObservabilityQuery {
            view: ObservabilityView::Health,
            cursor: None,
            limit: 2,
        },
    }
}

fn canonical_observability_result() -> ObservabilityQueryResult {
    let request = canonical_observability_request();
    ObservabilityQueryResult {
        observation: ProviderObservation {
            provider_id: request.context.provider_id.clone(),
            provider_generation: request.context.provider_generation,
            realm_id: request.target.realm_id().clone(),
            workload_id: request.target.workload_id().cloned(),
            handle_id: None,
            resource_generation: None,
            observed_at_unix_ms: 4_000,
            lifecycle: ObservedLifecycleState::Ready,
            adoption: AdoptionState::NotAttempted,
            reason: ObservationReason::None,
            health: ProviderHealth {
                provider_id: request.context.provider_id,
                registry_generation: request.context.provider_generation,
                observed_at_unix_ms: 4_000,
                state: ProviderHealthState::Healthy,
                reason: ProviderHealthReason::None,
                remediation: ProviderRemediation::None,
            },
        },
        records: BoundedVec::new(vec![ObservabilityRecord {
            observed_at_unix_ms: 3_000,
            projection: ObservabilityProjectionKind::Metrics,
            labels: ObservabilityLabels {
                provider_type: ProviderType::Runtime,
                health_state: ProviderHealthState::Healthy,
                metric: ObservabilityMetricLabel::ProviderHealth,
                operation: ObservabilityOperationLabel::Health,
                outcome: ObservabilityOutcomeLabel::Success,
            },
            value: 1,
        }])
        .unwrap(),
        next_cursor: None,
        encoded_bytes_upper_bound: OBSERVABILITY_RECORD_ENCODED_UPPER_BOUND_BYTES,
        truncated: false,
    }
}

#[test]
fn strict_wire_rejects_unknown_over_limit_and_missing_idempotency() {
    let request = valid_request();
    let encoded = encode_strict(&request, true).unwrap();
    assert_eq!(
        decode_strict::<common::ServiceRequest>(&encoded, true).unwrap(),
        request
    );

    let mut unknown = encoded;
    unknown.extend_from_slice(&[0x98, 0x06, 0x01]);
    assert_eq!(
        decode_strict::<common::ServiceRequest>(&unknown, true),
        Err(ServiceContractError::UnknownField)
    );

    let mut missing = valid_request();
    missing.metadata.as_mut().unwrap().idempotency_key.clear();
    assert_eq!(
        missing.validate_wire(true),
        Err(ServiceContractError::MissingIdempotency)
    );

    let mut attachments = valid_request();
    attachments.attachment_indexes = vec![7, 7];
    assert_eq!(
        attachments.validate_wire(true),
        Err(ServiceContractError::DuplicateAttachment)
    );
    attachments.attachment_indexes = vec![u32::from(
        d2b_contracts::v2_component_session::MAX_REQUEST_ATTACHMENTS,
    )];
    assert_eq!(
        attachments.validate_wire(true),
        Err(ServiceContractError::BoundExceeded)
    );

    let oversized = vec![0_u8; d2b_contracts::v2_services::MAX_PROTOBUF_MESSAGE_BYTES + 1];
    assert_eq!(
        decode_strict::<common::ServiceRequest>(&oversized, true),
        Err(ServiceContractError::MessageTooLarge)
    );
}

#[test]
fn allocator_and_realm_child_contracts_round_trip_strictly() {
    let allocate = valid_allocate_request();
    let encoded = encode_strict(&allocate, true).expect("allocate request");
    assert_eq!(
        decode_strict::<broker::AllocateRequest>(&encoded, true).expect("allocate decode"),
        allocate
    );

    let allocation = valid_allocate_response();
    let encoded = encode_strict(&allocation, false).expect("allocate response");
    assert_eq!(
        decode_strict::<broker::AllocateResponse>(&encoded, false).expect("response decode"),
        allocation
    );

    let spawn = valid_spawn_request();
    let encoded = encode_strict(&spawn, true).expect("spawn request");
    assert_eq!(
        decode_strict::<broker::SpawnRealmChildrenRequest>(&encoded, true).expect("spawn decode"),
        spawn
    );

    let spawned = valid_spawn_response();
    let encoded = encode_strict(&spawned, false).expect("spawn response");
    assert_eq!(
        decode_strict::<broker::SpawnRealmChildrenResponse>(&encoded, false)
            .expect("spawn response decode"),
        spawned
    );
}

#[test]
fn allocator_and_realm_child_contracts_fail_closed() {
    let mut duplicate_resource = valid_allocate_request();
    duplicate_resource
        .resources
        .push(duplicate_resource.resources[0].clone());
    assert_eq!(
        duplicate_resource.validate_wire(true),
        Err(ServiceContractError::InvalidId)
    );

    let mut wrong_delegation = valid_allocate_response();
    wrong_delegation.resources[0].attachment_index = None;
    assert_eq!(
        wrong_delegation.validate_wire(false),
        Err(ServiceContractError::InconsistentResponse)
    );

    let mut missing_bootstrap = valid_spawn_request();
    missing_bootstrap.fds.pop();
    missing_bootstrap.fds.push(child_fd(
        broker::RealmChildRole::REALM_CHILD_ROLE_BROKER,
        broker::RealmChildFdKind::REALM_CHILD_FD_KIND_STATE_ROOT,
        3,
    ));
    assert_eq!(
        missing_bootstrap.validate_wire(true),
        Err(ServiceContractError::MissingOperationInput)
    );

    let mut wrong_listener_owner = valid_spawn_request();
    wrong_listener_owner.fds[0].role = broker::RealmChildRole::REALM_CHILD_ROLE_BROKER.into();
    assert_eq!(
        wrong_listener_owner.validate_wire(true),
        Err(ServiceContractError::InvalidOperationInput)
    );

    let mut duplicate_pidfd = valid_spawn_response();
    duplicate_pidfd.children[1].pidfd_attachment_index =
        duplicate_pidfd.children[0].pidfd_attachment_index;
    assert_eq!(
        duplicate_pidfd.validate_wire(false),
        Err(ServiceContractError::DuplicateAttachment)
    );
}

#[test]
fn provider_operation_input_wire_is_exact_bounded_and_strict() {
    let request = valid_provider_request();
    request.validate_wire(true).unwrap();
    assert_eq!(
        provider_operation_input(request.input.as_ref().unwrap()).unwrap(),
        ProviderOperationInput::NoInput
    );

    let mut missing = valid_provider_request();
    missing.input = MessageField::none();
    assert_eq!(
        missing.validate_wire(true),
        Err(ServiceContractError::MissingOperationInput)
    );
    missing.input = MessageField::some(common::ProviderOperationInput::new());
    assert_eq!(
        missing.validate_wire(true),
        Err(ServiceContractError::MissingOperationInput)
    );

    let mut configured = valid_provider_request();
    configured
        .input
        .as_mut()
        .unwrap()
        .set_configured_runtime_execution(common::ConfiguredRuntimeExecutionInput {
            configured_item_id: "configured-item".to_owned(),
            ..Default::default()
        });
    assert!(matches!(
        provider_operation_input(configured.input.as_ref().unwrap()).unwrap(),
        ProviderOperationInput::ConfiguredRuntimeExecution { .. }
    ));
    configured
        .input
        .as_mut()
        .unwrap()
        .mut_configured_runtime_execution()
        .configured_item_id = "Configured/Command".to_owned();
    assert_eq!(
        configured.validate_wire(true),
        Err(ServiceContractError::InvalidId)
    );

    for obsolete_field in [
        &[0x2a, 0x01, b'x'][..],
        &[0x38, 0x01][..],
        &[0x42, 0x01, b'x'][..],
    ] {
        let mut encoded = encode_strict(&valid_provider_request(), true).unwrap();
        encoded.extend_from_slice(obsolete_field);
        assert_eq!(
            decode_strict::<common::ProviderRequest>(&encoded, true),
            Err(ServiceContractError::UnknownField)
        );
    }

    let mut power = common::ProviderOperationInput::new();
    power.set_infrastructure_power_state(common::InfrastructurePowerStateInput {
        state: common::InfrastructurePowerState::INFRASTRUCTURE_POWER_STATE_STOPPED.into(),
        ..Default::default()
    });
    assert_eq!(
        provider_operation_input(&power).unwrap(),
        ProviderOperationInput::InfrastructurePowerState {
            state: InfrastructurePowerState::Stopped
        }
    );

    let mut transport = common::ProviderOperationInput::new();
    transport.set_transport_binding(common::TransportBindingInput {
        transport_binding_id: "transport-binding".to_owned(),
        ..Default::default()
    });
    assert!(matches!(
        provider_operation_input(&transport).unwrap(),
        ProviderOperationInput::TransportBinding { transport_binding_id }
            if transport_binding_id.as_str() == "transport-binding"
    ));

    let mut storage = common::ProviderOperationInput::new();
    storage.set_storage_snapshot(common::StorageSnapshotInput {
        snapshot_id: "snapshot-id".to_owned(),
        ..Default::default()
    });
    assert!(matches!(
        provider_operation_input(&storage).unwrap(),
        ProviderOperationInput::StorageSnapshot { snapshot_id }
            if snapshot_id.as_str() == "snapshot-id"
    ));

    let mut device = common::ProviderOperationInput::new();
    device.set_device_selector(common::DeviceSelectorInput {
        device_selector_id: "selector-id".to_owned(),
        ..Default::default()
    });
    assert!(matches!(
        provider_operation_input(&device).unwrap(),
        ProviderOperationInput::DeviceSelector { device_selector_id }
            if device_selector_id.as_str() == "selector-id"
    ));

    let mut audio = valid_provider_request();
    audio
        .input
        .as_mut()
        .unwrap()
        .set_audio_state(common::AudioStateInput {
            channel: common::AudioChannel::AUDIO_CHANNEL_SPEAKER.into(),
            direction: common::AudioDirection::AUDIO_DIRECTION_OUTPUT.into(),
            mute: Some(false),
            volume: Some(100),
            ..Default::default()
        });
    assert_eq!(
        provider_operation_input(audio.input.as_ref().unwrap()).unwrap(),
        ProviderOperationInput::AudioState {
            channel: AudioChannel::Speaker,
            direction: AudioDirection::Output,
            mute: Some(false),
            volume: Some(100),
        }
    );
    audio.input.as_mut().unwrap().mut_audio_state().volume = Some(101);
    assert_eq!(
        audio.validate_wire(true),
        Err(ServiceContractError::BoundExceeded)
    );
    let audio_state = audio.input.as_mut().unwrap().mut_audio_state();
    audio_state.volume = Some(50);
    audio_state.channel = common::AudioChannel::AUDIO_CHANNEL_MICROPHONE.into();
    assert_eq!(
        audio.validate_wire(true),
        Err(ServiceContractError::InvalidOperationInput)
    );

    let mut query = common::ProviderOperationInput::new();
    query.set_observability_query(common::ObservabilityQueryInput {
        view: common::ObservabilityView::OBSERVABILITY_VIEW_OPERATIONS.into(),
        cursor: Some("cursor-one".to_owned()),
        limit: 256,
        ..Default::default()
    });
    assert_eq!(
        provider_operation_input(&query).unwrap(),
        ProviderOperationInput::ObservabilityQuery {
            view: ObservabilityView::Operations,
            cursor: Some(
                d2b_contracts::v2_provider::ObservabilityCursor::parse("cursor-one").unwrap()
            ),
            limit: 256,
        }
    );
    query.mut_observability_query().limit = 0;
    assert_eq!(
        provider_operation_input(&query),
        Err(ServiceContractError::BoundExceeded)
    );

    let mut export = common::ProviderOperationInput::new();
    export.set_observability_export(common::ObservabilityExportInput {
        format: common::ObservabilityExportFormat::OBSERVABILITY_EXPORT_FORMAT_JSON_LINES.into(),
        start_at_unix_ms: 100,
        end_at_unix_ms: 200,
        ..Default::default()
    });
    assert_eq!(
        provider_operation_input(&export).unwrap(),
        ProviderOperationInput::ObservabilityExport {
            format: ObservabilityExportFormat::JsonLines,
            start_at_unix_ms: 100,
            end_at_unix_ms: 200,
        }
    );
    export.mut_observability_export().end_at_unix_ms = 100;
    assert_eq!(
        provider_operation_input(&export),
        Err(ServiceContractError::InvalidDeadline)
    );
}

#[test]
fn identity_scope_is_unambiguous() {
    let mut request = valid_request();
    let scope = request.scope.as_mut().unwrap();
    scope.workload_id = "baaaaaaaaaaaaaaaaaaq".to_owned();
    scope.provider_id = "caaaaaaaaaaaaaaaaaaq".to_owned();
    assert_eq!(
        request.validate_wire(true),
        Err(ServiceContractError::InvalidIdentity)
    );

    let scope = request.scope.as_mut().unwrap();
    scope.provider_id.clear();
    scope.role_id = "daaaaaaaaaaaaaaaaaaq".to_owned();
    request.validate_wire(true).unwrap();

    request.scope.as_mut().unwrap().provider_id = "caaaaaaaaaaaaaaaaaaq".to_owned();
    assert_eq!(
        request.validate_wire(true),
        Err(ServiceContractError::InvalidIdentity)
    );
}

fn valid_error() -> common::ErrorEnvelope {
    let mut error = common::ErrorEnvelope::new();
    error.kind = common::ErrorKind::ERROR_KIND_INTERNAL.into();
    error.retry = common::RetryClass::RETRY_CLASS_NEVER.into();
    error.correlation_id = "correlation-1".to_owned();
    error
}

#[test]
fn responses_bind_attachments_streams_and_error_outcomes() {
    let mut response = common::ServiceResponse::new();
    response.outcome = common::Outcome::OUTCOME_SUCCEEDED.into();
    response.stream_id = "stream-1".to_owned();
    response.attachment_indexes = vec![0, 1];
    response.validate_wire(false).unwrap();

    response.attachment_indexes = vec![1, 1];
    assert_eq!(
        response.validate_wire(false),
        Err(ServiceContractError::DuplicateAttachment)
    );
    response.attachment_indexes = vec![0];
    response.error = MessageField::some(valid_error());
    assert_eq!(
        response.validate_wire(false),
        Err(ServiceContractError::InconsistentResponse)
    );

    response.outcome = common::Outcome::OUTCOME_FAILED.into();
    response.validate_wire(false).unwrap();
    response.error = MessageField::none();
    assert_eq!(
        response.validate_wire(false),
        Err(ServiceContractError::InconsistentResponse)
    );

    let mut provider = common::ProviderResponse::new();
    provider.outcome = common::Outcome::OUTCOME_SUCCEEDED.into();
    provider.operation_id = "operation-1".to_owned();
    provider.stream_id = "stream-1".to_owned();
    provider.attachment_indexes = vec![0];
    provider.validate_wire(false).unwrap();
    provider.stream_id = "x".repeat(65);
    assert_eq!(
        provider.validate_wire(false),
        Err(ServiceContractError::BoundExceeded)
    );
}

#[test]
fn observability_query_result_round_trips_exactly_with_actual_provider_type() {
    let request = canonical_observability_request();
    let result = canonical_observability_result();
    let wire = observability_query_result_to_wire(&result, &request).unwrap();
    let mut response = common::ProviderResponse::new();
    response.outcome = common::Outcome::OUTCOME_SUCCEEDED.into();
    response.operation_id = request.context.operation_id.as_str().to_owned();
    response.observability_query_result = MessageField::some(wire);

    validate_provider_response_for_method(&response, ProviderMethod::ObservabilityQuery).unwrap();
    let decoded = observability_query_response_from_wire(&response, &request).unwrap();
    assert_eq!(decoded, result);
    assert_eq!(
        decoded.records[0].labels.provider_type,
        ProviderType::Runtime
    );
}

#[test]
fn observability_query_wire_rejects_invalid_enums_and_response_field_mixing() {
    let request = canonical_observability_request();
    let result = canonical_observability_result();
    let mut response = common::ProviderResponse::new();
    response.outcome = common::Outcome::OUTCOME_SUCCEEDED.into();
    response.operation_id = request.context.operation_id.as_str().to_owned();
    response.observability_query_result =
        MessageField::some(observability_query_result_to_wire(&result, &request).unwrap());

    let mut invalid_enum = response.clone();
    invalid_enum
        .observability_query_result
        .as_mut()
        .unwrap()
        .records[0]
        .labels
        .as_mut()
        .unwrap()
        .metric = EnumOrUnknown::from_i32(999);
    assert_eq!(
        invalid_enum.validate_wire(false),
        Err(ServiceContractError::InvalidEnum)
    );

    let mut too_many = response.clone();
    let record = too_many
        .observability_query_result
        .as_ref()
        .unwrap()
        .records[0]
        .clone();
    too_many
        .observability_query_result
        .as_mut()
        .unwrap()
        .records = vec![record; 257];
    assert_eq!(
        too_many.validate_wire(false),
        Err(ServiceContractError::BoundExceeded)
    );

    let mut cursor_too_long = response.clone();
    let result = cursor_too_long.observability_query_result.as_mut().unwrap();
    result.next_cursor = Some(format!("c{}", "1".repeat(64)));
    result.truncated = true;
    assert_eq!(
        cursor_too_long.validate_wire(false),
        Err(ServiceContractError::InvalidId)
    );

    let mut mixed = response.clone();
    mixed.result_digest = vec![0x44; 32];
    assert_eq!(
        mixed.validate_wire(false),
        Err(ServiceContractError::InconsistentResponse)
    );
    assert_eq!(
        validate_provider_response_for_method(&response, ProviderMethod::RuntimeInspect),
        Err(ServiceContractError::InconsistentResponse)
    );

    let mut failed = response;
    failed.outcome = common::Outcome::OUTCOME_FAILED.into();
    failed.error = MessageField::some(valid_error());
    assert_eq!(
        failed.validate_wire(false),
        Err(ServiceContractError::InconsistentResponse)
    );
}

#[test]
fn observability_query_result_rejects_operation_scope_and_generation_mismatch() {
    let request = canonical_observability_request();
    let result = canonical_observability_result();
    let mut response = common::ProviderResponse::new();
    response.outcome = common::Outcome::OUTCOME_SUCCEEDED.into();
    response.operation_id = "operation-other".to_owned();
    response.observability_query_result =
        MessageField::some(observability_query_result_to_wire(&result, &request).unwrap());
    assert_eq!(
        observability_query_response_from_wire(&response, &request),
        Err(ServiceContractError::InconsistentResponse)
    );

    response.operation_id = request.context.operation_id.as_str().to_owned();
    let mut mismatched_scope = request.clone();
    mismatched_scope.target = ProviderTarget::Realm {
        realm_id: request.target.realm_id().clone(),
    };
    assert_eq!(
        observability_query_response_from_wire(&response, &mismatched_scope),
        Err(ServiceContractError::InconsistentResponse)
    );

    let mut mismatched_generation = result;
    mismatched_generation.observation.provider_generation = Generation::new(8).unwrap();
    assert_eq!(
        observability_query_result_to_wire(&mismatched_generation, &request),
        Err(ServiceContractError::InconsistentResponse)
    );
}

#[test]
fn observability_record_wire_has_no_free_form_or_high_cardinality_labels() {
    let source = include_str!("../proto/v2/common.proto");
    let labels = source
        .split("message ObservabilityLabels {")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    let record = source
        .split("message ObservabilityRecord {")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    assert!(!labels.contains("string "));
    assert!(!labels.contains("bytes "));
    assert!(!record.contains("string "));
    assert!(!record.contains("bytes "));
    for forbidden in [
        "workload",
        "provider_instance",
        "identifier",
        "path",
        "command",
        "secret",
        "json",
    ] {
        assert!(!labels.contains(forbidden));
        assert!(!record.contains(forbidden));
    }
}

#[test]
fn strict_wire_errors_classify_the_rejected_field() {
    let mut generation = valid_request();
    generation.metadata.as_mut().unwrap().session_generation = 0;
    assert_eq!(
        generation.validate_wire(true),
        Err(ServiceContractError::InvalidId)
    );

    let mut request = valid_request();
    request.desired_state = EnumOrUnknown::from_i32(999);
    assert_eq!(
        request.validate_wire(true),
        Err(ServiceContractError::InvalidEnum)
    );
    request.desired_state = common::DesiredState::DESIRED_STATE_UNSPECIFIED.into();
    request.request_digest = vec![0x11];
    assert_eq!(
        request.validate_wire(true),
        Err(ServiceContractError::InvalidDigest)
    );

    let mut context = common::ProviderOperationContext::new();
    context.metadata = request.metadata.clone();
    context.scope = request.scope.clone();
    context.operation_id = "operation-1".to_owned();
    context.provider_id = "caaaaaaaaaaaaaaaaaaq".to_owned();
    context.provider_type = common::ProviderType::PROVIDER_TYPE_RUNTIME.into();
    context.provider_generation = 1;
    context.policy_epoch = 1;
    context.authorization_digest = vec![0x22];
    context.request_digest = vec![0x33; 32];
    let mut provider = common::ProviderRequest::new();
    provider.context = MessageField::some(context);
    assert_eq!(
        provider.validate_wire(true),
        Err(ServiceContractError::InvalidDigest)
    );

    let mut response = common::ServiceResponse::new();
    response.outcome = EnumOrUnknown::from_i32(999);
    assert_eq!(
        response.validate_wire(false),
        Err(ServiceContractError::InvalidEnum)
    );

    let mut error = valid_error();
    error.correlation_id = "x".repeat(65);
    response.outcome = common::Outcome::OUTCOME_FAILED.into();
    response.error = MessageField::some(error);
    assert_eq!(
        response.validate_wire(false),
        Err(ServiceContractError::InvalidId)
    );

    let mut observation = common::Observation::new();
    observation.resource_id = "resource-1".to_owned();
    observation.generation = 1;
    observation.state = EnumOrUnknown::from_i32(999);
    observation.digest = vec![0x44; 32];
    response.error = MessageField::some(valid_error());
    response.observations.push(observation);
    assert_eq!(
        response.validate_wire(false),
        Err(ServiceContractError::InvalidEnum)
    );
}

#[test]
fn wire_provider_capabilities_are_bijective_with_provider_methods() {
    let wire = &common::ProviderCapability::VALUES[1..];
    assert_eq!(wire.len(), ProviderMethod::ALL.len());
    let mapped = wire
        .iter()
        .copied()
        .map(provider_method_for_capability)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(mapped, ProviderMethod::ALL);

    let mut response = common::CapabilityResponse::new();
    response.capabilities = wire.iter().copied().map(Into::into).collect();
    response.provider_generation = 1;
    response.descriptor_digest = vec![0x44; 32];
    response.validate_wire(false).unwrap();

    let mut failed = common::CapabilityResponse::new();
    failed.error = MessageField::some(valid_error());
    failed.validate_wire(false).unwrap();
    failed.provider_generation = 1;
    assert_eq!(
        failed.validate_wire(false),
        Err(ServiceContractError::InconsistentResponse)
    );
}

#[test]
fn inventory_fixture_and_schema_are_local_and_strict() {
    let fixture: ServiceInventoryDocument =
        serde_json::from_str(include_str!("../../../docs/reference/v2-services.json")).unwrap();
    assert_eq!(fixture, service_inventory_document());
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/reference/v2-services-schema.json"
    ))
    .unwrap();
    assert_eq!(schema["additionalProperties"], false);

    let mut value = serde_json::to_value(&fixture).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("legacy".to_owned(), serde_json::Value::Bool(true));
    assert!(serde_json::from_value::<ServiceInventoryDocument>(value).is_err());
}

#[test]
fn method_ids_are_stable_and_collision_free() {
    let mut ids = BTreeMap::new();
    for service in SERVICE_INVENTORY {
        for method in service.methods {
            let id = method.method_id(service.package, service.service);
            assert_ne!(id, 0);
            assert!(
                ids.insert(
                    id,
                    format!("{}.{}/{}", service.package, service.service, method.name)
                )
                .is_none()
            );
        }
    }
}

#[test]
fn payload_surface_has_no_secret_path_or_execution_authority_fields() {
    let combined = PROTO_SOURCES.join("\n");
    for forbidden in [
        "secret_bytes",
        "credential_bytes",
        "raw_path",
        "host_path",
        "argv",
        "command",
        "environment",
        "principal_id",
        "required_capability",
        "provider_response",
    ] {
        assert!(
            !combined.contains(forbidden),
            "forbidden protobuf field: {forbidden}"
        );
    }
    assert!(!combined.contains(".v1"));
    for (_, _, generated) in TTRPC_SOURCES {
        assert!(!generated.contains(".v1"));
    }
}

#[test]
fn request_debug_wrapper_redacts_values() {
    let request = valid_request();
    let rendered = format!(
        "{:?}",
        d2b_contracts::v2_services::RedactedRequest {
            package: "d2b.daemon.v2",
            service: "DaemonService",
            method: "Inspect",
            request: &request,
        }
    );
    assert!(rendered.contains("has_correlation: true"));
    assert!(!rendered.contains("correlation-1"));
    assert!(!rendered.contains("aaaaaaaaaaaaaaaaaaaa"));
    assert!(!rendered.contains("11, 11"));
}
