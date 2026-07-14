#![cfg(feature = "v2-services")]

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::v2_provider::ProviderMethod;
use d2b_contracts::v2_services::{
    SERVICE_INVENTORY, SERVICE_PACKAGES, ServiceContractError, ServiceInventoryDocument,
    StrictWireMessage, common, decode_strict, encode_strict, provider_method_for_capability,
    service_inventory_document,
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
