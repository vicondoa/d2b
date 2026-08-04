use d2b_contracts::v3::{
    CanonicalJsonObject, ResourceEnvelope, ResourceRef, ResourceSpec,
    provider::ProviderContractError,
    semantic_services::{SemanticContractError, SemanticFamily},
};
use d2b_provider_device_security_key::{
    DEFAULT_LEASE_TIMEOUT_SECS, DEFAULT_SESSION_RING_SIZE, DEFAULT_VSOCK_PORT,
    MAX_LEASE_TIMEOUT_SECS, MAX_SESSION_RING_SIZE, MIN_LEASE_TIMEOUT_SECS, MIN_SESSION_RING_SIZE,
    SECURITY_KEY_PROJECTION_PROTOCOL_VERSION, SECURITY_KEY_SERVICE_RESOURCE_TYPE, SessionRing,
    security_key_factory_fingerprint, security_key_projection_factory,
    security_key_projection_schema_fingerprint, security_key_semantic_descriptor,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settings {
    #[serde(default = "default_port")]
    vsock_port: u16,
    #[serde(default = "default_ring")]
    session_ring_size: usize,
    #[serde(default = "default_timeout")]
    lease_timeout_secs: u64,
}

fn default_port() -> u16 {
    DEFAULT_VSOCK_PORT
}
fn default_ring() -> usize {
    DEFAULT_SESSION_RING_SIZE
}
fn default_timeout() -> u64 {
    DEFAULT_LEASE_TIMEOUT_SECS
}

#[test]
fn settings_defaults_and_unknown_fields_are_closed() {
    let settings: Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(settings.vsock_port, DEFAULT_VSOCK_PORT);
    assert_eq!(settings.session_ring_size, DEFAULT_SESSION_RING_SIZE);
    assert_eq!(settings.lease_timeout_secs, DEFAULT_LEASE_TIMEOUT_SECS);
    assert!(serde_json::from_str::<Settings>(r#"{"path":"/dev/hidraw0"}"#).is_err());
    assert!(SessionRing::new(MIN_SESSION_RING_SIZE - 1).is_err());
    assert!(SessionRing::new(MAX_SESSION_RING_SIZE + 1).is_err());
    assert!(
        (MIN_LEASE_TIMEOUT_SECS..=MAX_LEASE_TIMEOUT_SECS).contains(&settings.lease_timeout_secs)
    );
}

#[test]
fn semantic_descriptor_is_derived_from_the_security_key_catalog() {
    let descriptor = security_key_semantic_descriptor().unwrap();
    let pair = SemanticFamily::SecurityKey.contract();
    let factory = descriptor.projection_factory();

    assert_eq!(
        descriptor.service_binding().resource_type(),
        pair.service().resource_type()
    );
    assert_eq!(
        descriptor.binding_binding().resource_type(),
        pair.binding().resource_type()
    );
    assert_eq!(
        factory.service_type().as_str(),
        SECURITY_KEY_SERVICE_RESOURCE_TYPE
    );
    assert_eq!(factory.binding_type(), pair.projection().binding_type());
    assert_eq!(
        factory.projection_protocol_version().as_str(),
        SECURITY_KEY_PROJECTION_PROTOCOL_VERSION
    );
    assert!(factory.allowed_backing_ref_types().is_empty());
    assert_eq!(
        factory.projection_schema_fingerprint(),
        pair.projection().projection_schema_fingerprint()
    );
    assert_eq!(
        factory.factory_fingerprint(),
        pair.projection().factory_fingerprint()
    );
    assert_eq!(
        factory.projection_schema_fingerprint().as_str(),
        "sha256:b849696b791bafdf020245315600402aa814386297588c3a42168d4e6222d25b"
    );
    assert_eq!(
        factory.factory_fingerprint().as_str(),
        "sha256:03ea1bca976cacb065995968bc850922f934a0bd482d53e6be57ef480838ca46"
    );
    assert_eq!(
        security_key_projection_schema_fingerprint().unwrap(),
        factory.projection_schema_fingerprint()
    );
    assert_eq!(
        security_key_factory_fingerprint().unwrap(),
        factory.factory_fingerprint()
    );
    descriptor.validate_against_catalog().unwrap();
}

#[test]
fn empty_security_key_backing_allowlist_is_an_unconditional_deny_all() {
    let factory = security_key_projection_factory().unwrap();
    assert!(factory.allowed_backing_ref_types().is_empty());
    for resource_type in ["Device", "Endpoint", SECURITY_KEY_SERVICE_RESOURCE_TYPE] {
        assert_eq!(
            factory.admits_backing_ref(&resource(resource_type)),
            Err(ProviderContractError::ProjectionFactoryInvalid)
        );
    }
}

#[test]
fn projection_branch_accepts_only_the_catalog_projection_shape() {
    let descriptor = security_key_semantic_descriptor().unwrap();
    let projection = ResourceSpec::new(
        Some(resource_ref("Provider/device-security-key")),
        None,
        object(r#"{"mode":"projection"}"#),
        None,
    )
    .unwrap();
    descriptor.validate_projection_spec(&projection).unwrap();
    assert!(projection.provider().is_none());

    let authority = ResourceSpec::new(
        Some(resource_ref("Provider/device-security-key")),
        None,
        object(r#"{"authority":{},"mode":"projection"}"#),
        None,
    )
    .unwrap();
    assert_eq!(
        descriptor.validate_projection_spec(&authority),
        Err(SemanticContractError::SchemaViolation)
    );
}

#[test]
fn base_bindings_match_the_provider_neutral_conformance_fixtures() {
    let descriptor = security_key_semantic_descriptor().unwrap();
    let pair = SemanticFamily::SecurityKey.contract();
    let provider = resource_ref("Provider/device-security-key");

    let service = pair
        .service()
        .minimal_base_spec(provider.clone(), object(r#"{"mode":"authority"}"#))
        .unwrap();
    let binding = pair
        .binding()
        .minimal_base_spec(
            provider,
            object(
                r#"{"serviceRef":"security-key.d2bus.org.SecurityKeyService/yubikey-primary","target":{"guestRef":"Guest/corp-vm","userRef":"User/alice"}}"#,
            ),
        )
        .unwrap();
    assert!(service.provider().is_none());
    assert!(binding.provider().is_none());
    pair.service()
        .schema_contract(std::iter::empty())
        .unwrap()
        .validate_minimal_base_spec(&service)
        .unwrap();
    pair.binding()
        .schema_contract(std::iter::empty())
        .unwrap()
        .validate_minimal_base_spec(&binding)
        .unwrap();

    let bindings = descriptor.api_bindings();
    assert_eq!(bindings.len(), 2);
    assert_eq!(
        bindings[0].base_spec_fingerprint(),
        pair.service().spec().fingerprint()
    );
    assert_eq!(
        bindings[0].base_status_fingerprint(),
        pair.service().status().fingerprint()
    );
    assert_eq!(
        bindings[1].base_spec_fingerprint(),
        pair.binding().spec().fingerprint()
    );
    assert_eq!(
        bindings[1].base_status_fingerprint(),
        pair.binding().status().fingerprint()
    );
}

#[test]
fn semantic_descriptor_round_trips_and_rejects_factory_tampering() {
    let descriptor = security_key_semantic_descriptor().unwrap();
    let encoded = serde_json::to_vec(&descriptor).unwrap();
    let decoded: d2b_provider_device_security_key::SecurityKeySemanticDescriptor =
        serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, descriptor);

    let mut tampered: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    tampered["projectionFactory"]["allowedBackingRefTypes"] = serde_json::json!(["Device"]);
    assert!(
        serde_json::from_value::<d2b_provider_device_security_key::SecurityKeySemanticDescriptor>(
            tampered
        )
        .is_err()
    );
}

fn object(json: &str) -> CanonicalJsonObject {
    CanonicalJsonObject::parse(json.as_bytes()).unwrap()
}

fn resource_ref(value: &str) -> ResourceRef {
    ResourceRef::parse(value).unwrap()
}

fn resource(resource_type: &str) -> ResourceEnvelope {
    let value = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": resource_type,
        "metadata": {
            "name": "resource",
            "zone": "dev",
            "uid": "123e4567-e89b-42d3-a456-426614174000",
            "generation": 1,
            "revision": 1,
            "ownerRef": null,
            "finalizers": [],
            "deletionRequestedAt": null,
            "createdAt": "2026-07-22T00:00:00.000Z",
            "updatedAt": "2026-07-22T00:00:00.000Z",
            "managedBy": "controller",
            "configurationGeneration": null,
            "controllerGeneration": null,
            "providerGeneration": null
        },
        "spec": {},
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
        }
    });
    ResourceEnvelope::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
}
