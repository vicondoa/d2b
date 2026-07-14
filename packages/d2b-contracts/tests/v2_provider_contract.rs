#![cfg(feature = "v2-provider")]

use std::{fs, path::PathBuf};

use d2b_contracts::{
    v2_component_session::{BoundedVec, EndpointRole, ServicePackage},
    v2_identity::{ProviderId, ProviderType, RealmId, RoleId, WorkloadId},
    v2_provider::*,
};
use schemars::schema_for;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

const ZERO: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const ONE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TWO: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn realm_id() -> RealmId {
    RealmId::parse("yl2hpmks5td5dkeso6qq").unwrap()
}

fn workload_id() -> WorkloadId {
    WorkloadId::parse("q5h7jtqteem7kua4tfva").unwrap()
}

fn role_id() -> RoleId {
    RoleId::parse("7xrbjonser3hpi7hqojq").unwrap()
}

fn provider_id(value: &str) -> ProviderId {
    ProviderId::parse(value).unwrap()
}

fn fingerprint(value: &str) -> Fingerprint {
    Fingerprint::parse(value).unwrap()
}

fn generation(value: u64) -> Generation {
    Generation::new(value).unwrap()
}

fn capabilities(provider_type: ProviderType) -> ProviderCapabilitySet {
    ProviderCapabilitySet::new(
        ProviderMethod::ALL
            .iter()
            .copied()
            .filter(|method| method.provider_type() == provider_type)
            .map(ProviderCapability)
            .collect(),
    )
    .unwrap()
}

fn agent_binding() -> AgentPlacementBinding {
    AgentPlacementBinding {
        realm_id: realm_id(),
        workload_id: workload_id(),
        role_id: role_id(),
        agent_generation: generation(3),
    }
}

fn agent_placement() -> ProviderPlacement {
    let binding = agent_binding();
    ProviderPlacement::ProviderAgent {
        realm_id: binding.realm_id,
        workload_id: binding.workload_id,
        role_id: binding.role_id,
        endpoint_role: EndpointRole::ProviderAgent,
        service: ServicePackage::ProviderV2,
        agent_generation: binding.agent_generation,
    }
}

fn authority(provider_type: ProviderType) -> ProviderAuthority {
    match provider_type {
        ProviderType::Runtime => ProviderAuthority::Runtime {
            posture: RuntimeAuthorityPosture {
                process: ProcessAuthority::ProviderManagedRemote,
                cgroup: CgroupAuthority::ProviderManagedRemote,
                network: NetworkPosture::IsolatedNamespace,
                user_namespace: UserNamespacePosture::None,
                persistent_identity: PersistentIdentityPosture::None,
                device_mediation: DeviceMediationPosture::ProviderManagedTyped,
            },
        },
        ProviderType::Infrastructure => ProviderAuthority::Infrastructure,
        ProviderType::Transport => ProviderAuthority::Transport,
        ProviderType::Substrate => ProviderAuthority::Substrate,
        ProviderType::Credential => ProviderAuthority::Credential,
        ProviderType::Display => ProviderAuthority::Display,
        ProviderType::Network => ProviderAuthority::Network,
        ProviderType::Storage => ProviderAuthority::Storage,
        ProviderType::Device => ProviderAuthority::Device,
        ProviderType::Audio => ProviderAuthority::Audio,
        ProviderType::Observability => ProviderAuthority::Observability,
    }
}

fn descriptor(id: &str, provider_type: ProviderType, implementation: &str) -> ProviderDescriptor {
    ProviderDescriptor {
        schema_version: PROVIDER_SCHEMA_VERSION,
        provider_id: provider_id(id),
        authority: authority(provider_type),
        implementation_id: ImplementationId::parse(implementation).unwrap(),
        api_version: ProviderApiVersion::V2,
        capabilities: capabilities(provider_type),
        configuration_schema_fingerprint: fingerprint(ONE),
        configured_scope_digest: fingerprint(TWO),
        registry_generation: generation(7),
        placement: agent_placement(),
    }
}

fn axes() -> BoundedVec<ProviderRegistryAxis, 11, 11> {
    BoundedVec::new(
        ProviderType::ALL
            .into_iter()
            .map(|provider_type| {
                let ids = match provider_type {
                    ProviderType::Runtime => vec![
                        provider_id("eaaaaaaaaaaaaaaaaaaq"),
                        provider_id("f7z3k5e3awgn43aljt2a"),
                    ],
                    ProviderType::Credential => vec![provider_id("caaaaaaaaaaaaaaaaaaq")],
                    _ => Vec::new(),
                };
                ProviderRegistryAxis {
                    provider_type,
                    providers: BoundedVec::new(ids).unwrap(),
                }
            })
            .collect(),
    )
    .unwrap()
}

fn registry() -> ProviderRegistrySnapshot {
    ProviderRegistrySnapshot {
        schema_version: PROVIDER_SCHEMA_VERSION,
        generation: generation(7),
        configuration_fingerprint: fingerprint(ZERO),
        published_at_unix_ms: 1_000,
        lifecycle: RegistryLifecycle::Accepting,
        axes: axes(),
        factories: BoundedVec::new(vec![
            ProviderFactoryKey {
                provider_type: ProviderType::Runtime,
                implementation_id: ImplementationId::parse("azure-container-apps").unwrap(),
            },
            ProviderFactoryKey {
                provider_type: ProviderType::Credential,
                implementation_id: ImplementationId::parse("entra").unwrap(),
            },
        ])
        .unwrap(),
        providers: BoundedVec::new(vec![
            descriptor("caaaaaaaaaaaaaaaaaaq", ProviderType::Credential, "entra"),
            descriptor(
                "eaaaaaaaaaaaaaaaaaaq",
                ProviderType::Runtime,
                "azure-container-apps",
            ),
            descriptor(
                "f7z3k5e3awgn43aljt2a",
                ProviderType::Runtime,
                "azure-container-apps",
            ),
        ])
        .unwrap(),
    }
}

fn operation_context() -> ProviderOperationContext {
    ProviderOperationContext {
        schema_version: PROVIDER_SCHEMA_VERSION,
        operation_id: OperationId::parse("operation-1").unwrap(),
        idempotency_key: IdempotencyKey::parse("idempotency-1").unwrap(),
        request_digest: fingerprint(ZERO),
        scope: AuthorizedProviderScope::Workload {
            realm_id: realm_id(),
            workload_id: workload_id(),
        },
        principal: PrincipalRef::parse("principal-1").unwrap(),
        provider_id: provider_id("f7z3k5e3awgn43aljt2a"),
        provider_type: ProviderType::Runtime,
        provider_generation: generation(7),
        capability: ProviderCapability(ProviderMethod::RuntimePlan),
        method: ProviderMethod::RuntimePlan,
        policy_epoch: generation(9),
        authorization_decision_digest: fingerprint(ONE),
        issued_at_unix_ms: 2_000,
        expires_at_unix_ms: 10_000,
        correlation_id: CorrelationId::parse("correlation-1").unwrap(),
        trace_id: fingerprint(TWO),
    }
}

fn operation_request() -> ProviderOperationRequest {
    ProviderOperationRequest {
        context: operation_context(),
        target: ProviderTarget::Workload {
            realm_id: realm_id(),
            workload_id: workload_id(),
        },
        expected_configuration_fingerprint: fingerprint(ONE),
    }
}

fn binding() -> OperationBinding {
    operation_context().binding()
}

fn plan() -> ProviderPlan {
    ProviderPlan {
        schema_version: PROVIDER_SCHEMA_VERSION,
        plan_id: PlanId::parse("plan-1").unwrap(),
        binding: binding(),
        realm_id: realm_id(),
        workload_id: Some(workload_id()),
        method: ProviderMethod::RuntimePlan,
        configuration_fingerprint: fingerprint(ONE),
        created_at_unix_ms: 2_500,
        expires_at_unix_ms: 9_000,
        resources: BoundedVec::new(vec![PlannedResourceClass::WorkloadExecution]).unwrap(),
    }
}

fn owner() -> HandleOwner {
    HandleOwner::Provider {
        realm_id: realm_id(),
        provider_id: provider_id("f7z3k5e3awgn43aljt2a"),
    }
}

fn handle() -> ProviderHandle {
    ProviderHandle {
        schema_version: PROVIDER_SCHEMA_VERSION,
        handle_id: HandleId::parse("handle-1").unwrap(),
        kind: ProviderHandleKind::Runtime,
        provider_id: provider_id("f7z3k5e3awgn43aljt2a"),
        realm_id: realm_id(),
        workload_id: Some(workload_id()),
        owner: owner(),
        provider_generation: generation(7),
        resource_generation: generation(2),
        configuration_fingerprint: fingerprint(ONE),
        created_by: binding(),
        created_at_unix_ms: 3_000,
        expires_at_unix_ms: None,
        ownership_transfer: OwnershipTransfer::Stationary {
            ownership_epoch: generation(1),
        },
    }
}

fn healthy(provider: &str, observed_at_unix_ms: u64) -> ProviderHealth {
    ProviderHealth {
        provider_id: provider_id(provider),
        registry_generation: generation(7),
        observed_at_unix_ms,
        state: ProviderHealthState::Healthy,
        reason: ProviderHealthReason::None,
        remediation: ProviderRemediation::None,
    }
}

fn observation() -> ProviderObservation {
    ProviderObservation {
        provider_id: provider_id("f7z3k5e3awgn43aljt2a"),
        provider_generation: generation(7),
        realm_id: realm_id(),
        workload_id: Some(workload_id()),
        handle_id: Some(HandleId::parse("handle-1").unwrap()),
        resource_generation: Some(generation(2)),
        observed_at_unix_ms: 4_000,
        lifecycle: ObservedLifecycleState::Running,
        adoption: AdoptionState::Adopted,
        reason: ObservationReason::None,
        health: healthy("f7z3k5e3awgn43aljt2a", 4_000),
    }
}

fn credential_lease() -> CredentialLease {
    CredentialLease {
        lease_id: LeaseId::parse("lease-1").unwrap(),
        credential_provider_id: provider_id("caaaaaaaaaaaaaaaaaaq"),
        consumer_provider_id: provider_id("eaaaaaaaaaaaaaaaaaaq"),
        agent_binding: agent_binding(),
        allowed_operations: BoundedVec::new(vec![
            SdkOperationClass::Authenticate,
            SdkOperationClass::Create,
        ])
        .unwrap(),
        issued_at_unix_ms: 2_000,
        expires_at_unix_ms: 8_000,
        credential_provider_generation: generation(7),
        consumer_provider_generation: generation(7),
        source_version: SourceVersion::parse("source-1").unwrap(),
        rotation_generation: generation(1),
        state: CredentialLeaseState::Active,
        transfer_policy: CredentialLeaseTransferPolicy::Forbidden,
        revoked_at_unix_ms: None,
    }
}

fn failure() -> ProviderFailure {
    ProviderFailure {
        kind: ProviderFailureKind::Unavailable,
        retry: RetryClass::SameOperation,
        provider_type: ProviderType::Runtime,
        binding: binding(),
        correlation_id: CorrelationId::parse("correlation-1").unwrap(),
        occurred_at_unix_ms: 4_500,
        reason: ProviderHealthReason::SessionDisconnected,
        remediation: ProviderRemediation::RestartAgent,
    }
}

fn document(contract_fingerprint: &str) -> ProviderContractDocument {
    ProviderContractDocument {
        schema_version: PROVIDER_SCHEMA_VERSION,
        contract_fingerprint: fingerprint(contract_fingerprint),
        registry: registry(),
        operation: operation_request(),
        plan: plan(),
        handle: handle(),
        observation: observation(),
        credential_lease: credential_lease(),
        failure: failure(),
    }
}

fn artifact_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/reference")
        .join(name)
}

fn canonical_json(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

fn contract_fingerprint(schema: &Value, fixture: &Value) -> String {
    let mut normalized = fixture.clone();
    normalized["contractFingerprint"] = Value::String(ZERO.to_owned());
    let mut hasher = Sha256::new();
    hasher.update(b"d2b-provider-contract-v2\0");
    hasher.update(canonical_json(schema));
    hasher.update([0]);
    hasher.update(canonical_json(&normalized));
    format!("{:x}", hasher.finalize())
}

#[test]
fn provider_artifacts_are_exact_and_fingerprint_bound() {
    let schema = serde_json::to_value(schema_for!(ProviderContractDocument)).unwrap();
    if std::env::var_os("D2B_UPDATE_PROVIDER_ARTIFACTS").is_some() {
        let mut fixture_value = serde_json::to_value(document(ZERO)).unwrap();
        let digest = contract_fingerprint(&schema, &fixture_value);
        fixture_value["contractFingerprint"] = Value::String(digest.clone());
        fs::write(
            artifact_path("provider-contract-v2.schema.json"),
            serde_json::to_vec_pretty(&schema).unwrap(),
        )
        .unwrap();
        fs::write(
            artifact_path("provider-contract-v2-fixture.json"),
            serde_json::to_vec_pretty(&fixture_value).unwrap(),
        )
        .unwrap();
        panic!("provider artifacts updated; set PROVIDER_CONTRACT_FINGERPRINT to {digest}");
    }

    let committed_schema: Value = serde_json::from_slice(
        &fs::read(artifact_path("provider-contract-v2.schema.json")).unwrap(),
    )
    .unwrap();
    let fixture_value: Value = serde_json::from_slice(
        &fs::read(artifact_path("provider-contract-v2-fixture.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(committed_schema, schema);
    assert_eq!(
        contract_fingerprint(&schema, &fixture_value),
        PROVIDER_CONTRACT_FINGERPRINT
    );
    let fixture: ProviderContractDocument = serde_json::from_value(fixture_value).unwrap();
    fixture.validate(5_000).unwrap();
}

#[test]
fn eleven_authority_axes_and_method_inventory_are_closed() {
    assert_eq!(ProviderType::ALL.len(), 11);
    assert_eq!(ProviderAuthority::ALL_TYPES, ProviderType::ALL);
    for provider_type in ProviderType::ALL {
        assert_eq!(authority(provider_type).provider_type(), provider_type);
        assert!(
            ProviderMethod::ALL
                .iter()
                .any(|method| method.provider_type() == provider_type && method.required())
        );
        capabilities(provider_type)
            .validate_for(provider_type)
            .unwrap();
    }
    assert!(serde_json::from_str::<ProviderAuthority>(r#"{"type":"unknown"}"#).is_err());
    assert!(serde_json::from_str::<ProviderMethod>("\"runtime.unknown-method\"").is_err());
}

#[test]
fn descriptors_capabilities_and_agent_placement_fail_closed() {
    let mut runtime = descriptor(
        "f7z3k5e3awgn43aljt2a",
        ProviderType::Runtime,
        "azure-container-apps",
    );
    runtime.validate().unwrap();
    runtime.capabilities = capabilities(ProviderType::Credential);
    assert_eq!(
        runtime.validate(),
        Err(ProviderContractError::ProviderTypeMismatch)
    );

    let mut missing = capabilities(ProviderType::Runtime)
        .as_slice()
        .iter()
        .copied()
        .filter(|capability| capability.0 != ProviderMethod::RuntimeStart)
        .collect::<Vec<_>>();
    missing.sort_unstable();
    runtime.capabilities = ProviderCapabilitySet::new(missing).unwrap();
    assert_eq!(
        runtime.validate(),
        Err(ProviderContractError::MissingRequiredCapability)
    );

    runtime.placement = ProviderPlacement::ProviderAgent {
        realm_id: realm_id(),
        workload_id: workload_id(),
        role_id: role_id(),
        endpoint_role: EndpointRole::GuestAgent,
        service: ServicePackage::ProviderV2,
        agent_generation: generation(1),
    };
    assert_eq!(
        runtime.placement.validate(),
        Err(ProviderContractError::PlacementMismatch)
    );
}

#[test]
fn operation_idempotency_scope_expiry_and_cancellation_are_exact() {
    let registry = registry();
    let descriptor = registry
        .descriptor(&provider_id("f7z3k5e3awgn43aljt2a"))
        .unwrap();
    let request = operation_request();
    request.validate(descriptor, 5_000).unwrap();
    request
        .validate_method(descriptor, 5_000, ProviderMethod::RuntimePlan)
        .unwrap();
    assert_eq!(
        request.validate_method(descriptor, 5_000, ProviderMethod::RuntimeStart),
        Err(ProviderContractError::CapabilityMismatch)
    );
    plan().validate(&request, 5_000).unwrap();

    let mut mismatch = request.clone();
    mismatch.context.provider_generation = generation(8);
    assert_eq!(
        mismatch.validate(descriptor, 5_000),
        Err(ProviderContractError::OperationBindingMismatch)
    );
    let mut changed_retry = request.clone();
    changed_retry.context.request_digest = fingerprint(TWO);
    assert_ne!(changed_retry.context.binding(), request.context.binding());
    assert_eq!(
        request.validate(descriptor, 10_000),
        Err(ProviderContractError::RequestExpired)
    );

    let call = ProviderCallContext {
        operation: &request.context,
        peer_role: EndpointRole::RealmController,
        service: ServicePackage::ProviderV2,
        monotonic_deadline_remaining_ms: 1,
        cancelled: true,
    };
    assert_eq!(call.validate(), Err(ProviderContractError::RequestExpired));
}

#[test]
fn handles_bind_identity_generation_owner_and_transfer() {
    let mut transferred_handle = handle();
    transferred_handle.validate().unwrap();
    let transfer_id = TransferId::parse("transfer-1").unwrap();
    transferred_handle.ownership_transfer = OwnershipTransfer::Pending {
        transfer_id: transfer_id.clone(),
        ownership_epoch: generation(1),
        from: owner(),
        to: HandleOwner::WorkloadRole {
            realm_id: realm_id(),
            workload_id: workload_id(),
            role_id: role_id(),
        },
        issued_at_unix_ms: 5_000,
        expires_at_unix_ms: 6_000,
    };
    transferred_handle.validate().unwrap();
    transferred_handle
        .complete_transfer(&transfer_id, 5_500)
        .unwrap();
    assert!(matches!(
        transferred_handle.ownership_transfer,
        OwnershipTransfer::Stationary { ownership_epoch } if ownership_epoch == generation(2)
    ));

    let mut wrong = handle();
    wrong.provider_generation = generation(8);
    assert_eq!(
        wrong.validate(),
        Err(ProviderContractError::HandleBindingMismatch)
    );
}

#[test]
fn ambiguous_adoption_is_quarantined_failed_and_non_admitting() {
    let mut ambiguous = observation();
    ambiguous.lifecycle = ObservedLifecycleState::Quarantined;
    ambiguous.adoption = AdoptionState::Ambiguous;
    ambiguous.reason = ObservationReason::MultipleCandidates;
    ambiguous.health.state = ProviderHealthState::Failed;
    ambiguous.health.reason = ProviderHealthReason::AdoptionAmbiguous;
    ambiguous.health.remediation = ProviderRemediation::OperatorInteraction;
    ambiguous.validate().unwrap();
    assert!(!ambiguous.admits_mutation());

    ambiguous.lifecycle = ObservedLifecycleState::Running;
    assert_eq!(
        ambiguous.validate(),
        Err(ProviderContractError::AdoptionAmbiguous)
    );
}

#[test]
fn registry_is_versioned_transactional_and_default_deny() {
    let registry = registry();
    registry.validate().unwrap();

    let selection = ProviderSelectionRequest {
        realm_id: realm_id(),
        workload_id: Some(workload_id()),
        provider_type: ProviderType::Runtime,
        capability: ProviderCapability(ProviderMethod::RuntimeStart),
        required_registry_generation: generation(7),
        configuration_fingerprint: fingerprint(ZERO),
        preferred_provider_id: Some(provider_id("f7z3k5e3awgn43aljt2a")),
    }
    .select(&registry)
    .unwrap();
    assert_eq!(
        selection.reason,
        ProviderSelectionReason::ExactConfiguredProvider
    );

    let ambiguous = ProviderSelectionRequest {
        preferred_provider_id: None,
        ..ProviderSelectionRequest {
            realm_id: realm_id(),
            workload_id: Some(workload_id()),
            provider_type: ProviderType::Runtime,
            capability: ProviderCapability(ProviderMethod::RuntimeStart),
            required_registry_generation: generation(7),
            configuration_fingerprint: fingerprint(ZERO),
            preferred_provider_id: None,
        }
    };
    assert_eq!(
        ambiguous.select(&registry),
        Err(ProviderContractError::NoEligibleProvider)
    );

    let mut duplicate = registry.clone();
    duplicate.factories = BoundedVec::new(vec![
        duplicate.factories[0].clone(),
        duplicate.factories[0].clone(),
    ])
    .unwrap();
    assert_eq!(
        duplicate.validate(),
        Err(ProviderContractError::DuplicateFactory)
    );

    let mut replacement = registry.clone();
    replacement.generation = generation(8);
    replacement.configuration_fingerprint = fingerprint(TWO);
    for descriptor in replacement.providers.iter_mut() {
        descriptor.registry_generation = generation(8);
    }
    ProviderRegistryUpdate {
        from_generation: generation(7),
        from_configuration_fingerprint: fingerprint(ZERO),
        replacement,
        drain_policy: RegistryDrainPolicy {
            drain_deadline_ms: 30_000,
            cancel_in_flight_at_deadline: true,
            revoke_transport_bindings: true,
            revoke_credential_leases: true,
            close_provider_sessions: true,
        },
    }
    .validate(&registry)
    .unwrap();
}

#[test]
fn credential_leases_are_opaque_colocated_revocable_and_nontransferable() {
    let registry = registry();
    let credential = registry
        .descriptor(&provider_id("caaaaaaaaaaaaaaaaaaq"))
        .unwrap();
    let consumer = registry
        .descriptor(&provider_id("eaaaaaaaaaaaaaaaaaaq"))
        .unwrap();
    let mut lease = credential_lease();
    lease.validate(credential, consumer, 5_000).unwrap();
    assert_eq!(
        lease.transfer_to(&provider_id("f7z3k5e3awgn43aljt2a")),
        Err(ProviderContractError::LeaseTransferForbidden)
    );
    lease.revoke(5_500).unwrap();
    assert_eq!(
        lease.validate(credential, consumer, 5_600),
        Err(ProviderContractError::LeaseRevoked)
    );

    let mut remote_consumer = consumer.clone();
    let binding = agent_binding();
    remote_consumer.placement = ProviderPlacement::ProviderAgent {
        realm_id: binding.realm_id,
        workload_id: binding.workload_id,
        role_id: binding.role_id,
        endpoint_role: EndpointRole::ProviderAgent,
        service: ServicePackage::ProviderV2,
        agent_generation: generation(4),
    };
    assert_eq!(
        credential_lease().validate(credential, &remote_consumer, 5_000),
        Err(ProviderContractError::LeaseNotColocated)
    );

    let mut refreshed = credential_lease();
    refreshed
        .refresh(
            5_000,
            9_000,
            SourceVersion::parse("source-2").unwrap(),
            generation(2),
        )
        .unwrap();
    refreshed.validate(credential, consumer, 8_500).unwrap();
    assert_eq!(
        refreshed.validate(credential, consumer, 9_000),
        Err(ProviderContractError::LeaseExpired)
    );
}

fn assert_no_forbidden_keys(value: &Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let normalized = key.to_ascii_lowercase();
                assert!(
                    ![
                        "argv",
                        "token",
                        "secret",
                        "rawpath",
                        "hostpath",
                        "diagnostic"
                    ]
                    .iter()
                    .any(|forbidden| normalized.contains(forbidden)),
                    "forbidden serialized key {key}"
                );
                assert_no_forbidden_keys(child);
            }
        }
        Value::Array(values) => values.iter().for_each(assert_no_forbidden_keys),
        _ => {}
    }
}

#[test]
fn serialized_contract_has_no_secret_path_argv_or_unbounded_diagnostic_entrypoint() {
    let value = serde_json::to_value(document(PROVIDER_CONTRACT_FINGERPRINT)).unwrap();
    assert_no_forbidden_keys(&value);
    let encoded = serde_json::to_string(&value).unwrap();
    for canary in [
        "/run/d2b/private.sock",
        "--credential-file",
        "super-secret-value",
        "eyJhbGciOi",
    ] {
        assert!(!encoded.contains(canary));
    }
}

#[test]
fn serde_rejects_unknown_fields_and_bounds_are_in_schema() {
    let mut value = serde_json::to_value(operation_request()).unwrap();
    value["unexpected"] = json!(true);
    assert!(serde_json::from_value::<ProviderOperationRequest>(value).is_err());

    let schema = serde_json::to_value(schema_for!(ProviderContractDocument)).unwrap();
    let rendered = serde_json::to_string(&schema).unwrap();
    assert!(rendered.contains("\"maxItems\":64"));
    assert!(rendered.contains("\"maxItems\":256"));
    assert!(rendered.contains("\"maxItems\":11"));
    assert!(rendered.contains("\"additionalProperties\":false"));
}

#[test]
fn debug_output_is_redacted() {
    for rendered in [
        format!(
            "{:?}",
            descriptor(
                "f7z3k5e3awgn43aljt2a",
                ProviderType::Runtime,
                "azure-container-apps"
            )
        ),
        format!("{:?}", operation_request()),
        format!("{:?}", operation_context()),
        format!("{:?}", handle()),
        format!("{:?}", observation()),
        format!("{:?}", credential_lease()),
        format!("{:?}", failure()),
    ] {
        assert!(!rendered.contains("operation-1"));
        assert!(!rendered.contains("principal-1"));
        assert!(!rendered.contains("lease-1"));
        assert!(!rendered.contains("f7z3k5e3awgn43aljt2a"));
        assert!(!rendered.contains("q5h7jtqteem7kua4tfva"));
        assert!(!rendered.contains(ZERO));
    }
}

#[test]
fn every_provider_trait_is_object_safe_for_in_process_or_agent_proxies() {
    fn assert_base(_: Option<&dyn Provider>) {}
    fn assert_runtime(_: Option<&dyn RuntimeProvider>) {}
    fn assert_infrastructure(_: Option<&dyn InfrastructureProvider>) {}
    fn assert_transport(_: Option<&dyn TransportProvider>) {}
    fn assert_substrate(_: Option<&dyn SubstrateProvider>) {}
    fn assert_credential(_: Option<&dyn CredentialProvider>) {}
    fn assert_display(_: Option<&dyn DisplayProvider>) {}
    fn assert_network(_: Option<&dyn NetworkProvider>) {}
    fn assert_storage(_: Option<&dyn StorageProvider>) {}
    fn assert_device(_: Option<&dyn DeviceProvider>) {}
    fn assert_audio(_: Option<&dyn AudioProvider>) {}
    fn assert_observability(_: Option<&dyn ObservabilityProvider>) {}

    assert_base(None);
    assert_runtime(None);
    assert_infrastructure(None);
    assert_transport(None);
    assert_substrate(None);
    assert_credential(None);
    assert_display(None);
    assert_network(None);
    assert_storage(None);
    assert_device(None);
    assert_audio(None);
    assert_observability(None);
}
