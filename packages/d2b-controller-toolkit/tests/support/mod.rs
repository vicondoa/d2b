use std::fs::OpenOptions;
use std::sync::Arc;

use d2b_contracts::v3::{
    CanonicalJsonValue, ConfigurationGeneration, RESOURCE_ENVELOPE_DOMAIN_TAG, ResourceRef,
    ResourceTypeName, ResourceUid, Timestamp, ZoneId, canonical_digest,
};
use d2b_resource_store::mutation_seal::{MutationSealBody, MutationSealIssuer, mutation_seal_pair};
use d2b_resource_store::{
    AdmittedAuthorization, AdmittedAuthorizationTarget, AdmittedVerb, ExpectedRevision,
    PolicySnapshot, PreparedStoreMutation, ResourceMutationKind, StoreMutation,
    StoreOperationContext, StoreProjection, StoreSlot, StoreWatchRequest,
};
use d2b_resource_store_redb::{RedbResourceStore, StoreIdentity, write_provisioning_marker};

pub async fn provision_store() -> (
    tempfile::TempDir,
    Arc<RedbResourceStore>,
    MutationSealIssuer,
) {
    let directory = tempfile::tempdir().expect("create hermetic store directory");
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .expect("create hermetic redb file");
    let mut marker = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(directory.path().join("store.marker"))
        .expect("create hermetic store marker");
    let identity = StoreIdentity::new(
        StoreSlot::new(0).expect("valid store slot"),
        ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("valid store UID"),
        ZoneId::parse("work").expect("valid Zone"),
        ResourceUid::parse("22222222-2222-4222-8222-222222222222").expect("valid Zone UID"),
        Timestamp::parse("2026-07-31T00:00:00.000Z").expect("valid timestamp"),
        PolicySnapshot {
            policy_revision: 7,
            api_catalog_revision: 8,
            active_configuration_revision: ConfigurationGeneration::new(9)
                .expect("valid configuration revision"),
            controller_generation: None,
        },
    );
    write_provisioning_marker(&mut marker, &identity).expect("write store marker");
    let (issuer, acceptor) = mutation_seal_pair(identity.seal_identity());
    let store = RedbResourceStore::provision_owned(file, marker, identity, acceptor)
        .await
        .expect("provision production redb store");
    (directory, Arc::new(store), issuer)
}

pub fn watch_request(after_revision: u64, initial_credits: u32) -> StoreWatchRequest {
    StoreWatchRequest {
        operation: StoreOperationContext {
            operation_id: "controller-watch".to_owned(),
            idempotency_key: Some("controller-watch-key".to_owned()),
            correlation_id: "controller-watch-correlation".to_owned(),
            trace_id: None,
            deadline_ms: 1_000,
        },
        zone: ZoneId::parse("work").expect("valid Zone"),
        resource_types: vec![ResourceTypeName::parse("Host").expect("valid ResourceType")],
        resource_names: Vec::new(),
        filters: Vec::new(),
        after_revision: d2b_contracts::v3::ZoneRevision::new(after_revision),
        initial_credits,
        projection: StoreProjection::Full,
    }
}

pub fn host_body(name: &str, owner: Option<&str>) -> Vec<u8> {
    let raw = format!(
        r#"{{"apiVersion":"resources.d2bus.org/v3","metadata":{{"configurationGeneration":7,"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"configuration","name":"{name}","ownerRef":null,"revision":1,"uid":"123e4567-e89b-42d3-a456-426614174000","updatedAt":"2026-07-22T00:00:00.000Z","zone":"work"}},"spec":{{"providerRef":"Provider/system-core","updatePolicy":{{"disruptive":"manual","nonDisruptive":"automatic"}}}},"status":{{"completedAt":null,"conditions":[],"lastReconciledAt":null,"observedGeneration":0,"outcome":null,"phase":"Pending","resource":{{}},"startedAt":null,"update":{{"dependencies":{{"count":0,"refs":[]}},"disruption":"None","lastAssessedAt":null,"observedGeneration":0,"operationId":null,"owned":{{"count":0,"refs":[]}},"preserveState":true,"reasons":[],"state":"Unknown","targetGeneration":1}}}},"type":"Host"}}"#
    );
    let mut value = CanonicalJsonValue::parse(raw.as_bytes()).expect("valid Host envelope");
    let CanonicalJsonValue::Object(root) = &mut value else {
        unreachable!()
    };
    let CanonicalJsonValue::Object(metadata) =
        root.get_mut("metadata").expect("Host metadata is present")
    else {
        unreachable!()
    };
    metadata.remove("uid");
    if let Some(owner) = owner {
        metadata.insert(
            "ownerRef".to_owned(),
            CanonicalJsonValue::String(owner.to_owned()),
        );
    }
    value.to_canonical_bytes()
}

pub async fn commit_host(
    store: &RedbResourceStore,
    issuer: &MutationSealIssuer,
    name: &str,
    owner: Option<&str>,
    operation_id: &str,
) -> d2b_contracts::v3::ZoneRevision {
    let target = ResourceRef::parse(&format!("Host/{name}")).expect("valid Host ref");
    let canonical = host_body(name, owner);
    let digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical);
    let body = MutationSealBody {
        mutations: vec![PreparedStoreMutation::new(
            StoreMutation {
                kind: ResourceMutationKind::Create,
                zone: ZoneId::parse("work").expect("valid Zone"),
                target: target.clone(),
                expected: ExpectedRevision::CreateAbsent,
                expected_uid: None,
                owner: owner
                    .map(ResourceRef::parse)
                    .transpose()
                    .expect("valid owner"),
                canonical_resource: Some(canonical),
                add_finalizers: Vec::new(),
                remove_finalizers: Vec::new(),
                wait_for_reconcile: false,
                reconcile_deadline_ms: None,
            },
            None,
            Some(digest),
        )],
        authorization: AdmittedAuthorization {
            zone: ZoneId::parse("work").expect("valid Zone"),
            subject_ref: ResourceRef::parse("Provider/system-core").expect("valid Provider ref"),
            subject_uid: ResourceUid::parse("33333333-3333-4333-8333-333333333333")
                .expect("valid subject UID"),
            targets: vec![AdmittedAuthorizationTarget {
                resource_type: ResourceTypeName::parse("Host").expect("valid ResourceType"),
                resource_name: Some(target.name().clone()),
                verb: AdmittedVerb::Create,
                subresource: None,
                execution_ref: None,
            }],
        },
        policy_snapshot: PolicySnapshot {
            policy_revision: 7,
            api_catalog_revision: 8,
            active_configuration_revision: ConfigurationGeneration::new(9)
                .expect("valid configuration revision"),
            controller_generation: None,
        },
        operation: StoreOperationContext {
            operation_id: operation_id.to_owned(),
            idempotency_key: Some(format!("key-{operation_id}")),
            correlation_id: format!("correlation-{operation_id}"),
            trace_id: None,
            deadline_ms: 1_000,
        },
    };
    store
        .commit_verified(issuer.seal(body))
        .await
        .expect("commit production resource")
        .revision
}
