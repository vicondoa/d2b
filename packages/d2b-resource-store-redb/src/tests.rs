use std::fs::OpenOptions;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};

use d2b_contracts::v3::{
    CanonicalJsonValue, ConfigurationGeneration, RESOURCE_ENVELOPE_DOMAIN_TAG, ResourceEnvelope,
    ResourceRef, ResourceTypeName, ResourceUid, Timestamp, ZoneId, canonical_digest,
};
use d2b_resource_store::mutation_seal::{
    MutationSealAcceptor, MutationSealBody, mutation_seal_pair,
};
use d2b_resource_store::{
    AdmittedAuthorization, AdmittedAuthorizationTarget, AdmittedVerb, ExpectedRevision,
    PolicySnapshot, PreparedStoreMutation, ResourceMutationKind, StoreGetRequest, StoreListRequest,
    StoreMutation, StoreOperationContext, StoreProjection, StoreSlot, StoreWatchRequest,
};
use redb::{Database, Durability};
use rustix::io::{FdFlags, fcntl_getfd, fcntl_setfd};
use rustix::net::{
    AddressFamily, SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketFlags, SocketType,
    sendmsg, socketpair,
};

use super::*;

fn identity() -> StoreIdentity {
    identity_for(
        StoreSlot::new(0).unwrap(),
        "work",
        "11111111-1111-4111-8111-111111111111",
    )
}

fn identity_for(slot: StoreSlot, zone: &str, store_uuid: &str) -> StoreIdentity {
    StoreIdentity::new(
        slot,
        ResourceUid::parse(store_uuid).unwrap(),
        ZoneId::parse(zone).unwrap(),
        ResourceUid::parse("22222222-2222-4222-8222-222222222222").unwrap(),
        Timestamp::parse("2026-07-31T00:00:00.000Z").unwrap(),
        PolicySnapshot {
            policy_revision: 7,
            api_catalog_revision: 8,
            active_configuration_revision: ConfigurationGeneration::new(9).unwrap(),
            controller_generation: None,
        },
    )
}

fn acceptor(identity: &StoreIdentity) -> MutationSealAcceptor {
    let (_, acceptor) = mutation_seal_pair(identity.seal_identity());
    acceptor
}

async fn provision_store(
    file: File,
    marker: File,
    identity: StoreIdentity,
) -> Result<RedbResourceStore, d2b_resource_store::StoreError> {
    RedbResourceStore::provision_owned(file, marker, identity.clone(), acceptor(&identity)).await
}

async fn open_store(
    file: File,
    identity: StoreIdentity,
) -> Result<RedbResourceStore, d2b_resource_store::StoreError> {
    RedbResourceStore::open_owned(file, identity.clone(), acceptor(&identity)).await
}

fn empty_seal_body() -> MutationSealBody {
    MutationSealBody {
        mutations: Vec::new(),
        authorization: d2b_resource_store::AdmittedAuthorization {
            zone: ZoneId::parse("work").unwrap(),
            subject_ref: ResourceRef::parse("Provider/system-core").unwrap(),
            subject_uid: ResourceUid::parse("33333333-3333-4333-8333-333333333333").unwrap(),
            targets: Vec::new(),
        },
        policy_snapshot: PolicySnapshot {
            policy_revision: 7,
            api_catalog_revision: 8,
            active_configuration_revision: ConfigurationGeneration::new(9).unwrap(),
            controller_generation: None,
        },
        operation: operation("seal"),
    }
}

fn create_seal_body(operation_id: &str, name: &str, payload_digest: String) -> MutationSealBody {
    create_seal_body_with_resource(operation_id, name, create_body(name), payload_digest)
}

fn create_seal_body_with_resource(
    operation_id: &str,
    name: &str,
    canonical_resource: Vec<u8>,
    payload_digest: String,
) -> MutationSealBody {
    let target = ResourceRef::parse(&format!("Host/{name}")).unwrap();
    MutationSealBody {
        mutations: vec![PreparedStoreMutation::new(
            StoreMutation {
                kind: ResourceMutationKind::Create,
                zone: ZoneId::parse("work").unwrap(),
                target: target.clone(),
                expected: ExpectedRevision::CreateAbsent,
                expected_uid: None,
                owner: None,
                canonical_resource: Some(canonical_resource),
                add_finalizers: Vec::new(),
                remove_finalizers: Vec::new(),
                wait_for_reconcile: false,
                reconcile_deadline_ms: None,
            },
            None,
            Some(payload_digest),
        )],
        authorization: AdmittedAuthorization {
            zone: ZoneId::parse("work").unwrap(),
            subject_ref: ResourceRef::parse("Provider/system-core").unwrap(),
            subject_uid: ResourceUid::parse("33333333-3333-4333-8333-333333333333").unwrap(),
            targets: vec![AdmittedAuthorizationTarget {
                resource_type: ResourceTypeName::parse("Host").unwrap(),
                resource_name: Some(target.name().clone()),
                verb: AdmittedVerb::Create,
                subresource: None,
                execution_ref: None,
            }],
        },
        policy_snapshot: PolicySnapshot {
            policy_revision: 7,
            api_catalog_revision: 8,
            active_configuration_revision: ConfigurationGeneration::new(9).unwrap(),
            controller_generation: None,
        },
        operation: operation(operation_id),
    }
}

fn owned_file() -> (tempfile::TempDir, File) {
    let directory = tempfile::tempdir().unwrap();
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .unwrap();
    assert!(fcntl_getfd(&file).unwrap().contains(FdFlags::CLOEXEC));
    (directory, file)
}

fn provisioned_store() -> (tempfile::TempDir, File, File) {
    let (directory, file) = owned_file();
    let mut marker = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(directory.path().join("store.marker"))
        .unwrap();
    write_provisioning_marker(&mut marker, &identity()).unwrap();
    (directory, file, marker)
}

fn operation(id: &str) -> StoreOperationContext {
    StoreOperationContext {
        operation_id: id.to_owned(),
        idempotency_key: Some(format!("key-{id}")),
        correlation_id: format!("correlation-{id}"),
        trace_id: None,
        deadline_ms: 1_000,
    }
}

fn stored_body(name: &str) -> Vec<u8> {
    format!(
        r#"{{"apiVersion":"resources.d2bus.org/v3","metadata":{{"configurationGeneration":7,"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"configuration","name":"{name}","ownerRef":null,"revision":1,"uid":"123e4567-e89b-42d3-a456-426614174000","updatedAt":"2026-07-22T00:00:00.000Z","zone":"work"}},"spec":{{"providerRef":"Provider/system-core","updatePolicy":{{"disruptive":"manual","nonDisruptive":"automatic"}}}},"status":{{"completedAt":null,"conditions":[],"lastReconciledAt":null,"observedGeneration":0,"outcome":null,"phase":"Pending","resource":{{}},"startedAt":null,"update":{{"dependencies":{{"count":0,"refs":[]}},"disruption":"None","lastAssessedAt":null,"observedGeneration":0,"operationId":null,"owned":{{"count":0,"refs":[]}},"preserveState":true,"reasons":[],"state":"Unknown","targetGeneration":1}}}},"type":"Host"}}"#
    )
    .into_bytes()
}

fn create_body(name: &str) -> Vec<u8> {
    let mut value = CanonicalJsonValue::parse(&stored_body(name)).unwrap();
    let CanonicalJsonValue::Object(root) = &mut value else {
        unreachable!()
    };
    let CanonicalJsonValue::Object(metadata) = root.get_mut("metadata").unwrap() else {
        unreachable!()
    };
    metadata.remove("uid");
    value.to_canonical_bytes()
}

fn seed_host(directory: &tempfile::TempDir, name: &str) {
    use crate::transaction::{
        CONTROLLER_INDEX, RESOURCES, REVISION_LOG, ResourceRecord, STORE_META, TYPE_INDEX, encode,
        resource_key, revision_key, type_index_key,
    };

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .unwrap();
    let backend = redb::backends::FileBackend::new(file).unwrap();
    let database = Database::builder().create_with_backend(backend).unwrap();
    crate::transaction::initialize(&database, &identity()).unwrap();
    let target = ResourceRef::parse(&format!("Host/{name}")).unwrap();
    let canonical_json = stored_body(name);
    let envelope = d2b_contracts::v3::ResourceEnvelope::from_json(&canonical_json).unwrap();
    let record = ResourceRecord {
        canonical_json,
        owner_uid: None,
        controller_binding_id: "Provider/system-core".to_owned(),
        payload_digest: envelope.digest().unwrap(),
    };
    let value = encode(ValueKind::ResourceRecord, &record).unwrap();
    let type_value = encode(
        ValueKind::TypeIndexRecord,
        &envelope.metadata().uid().as_str(),
    )
    .unwrap();
    let controller_value = encode(
        ValueKind::ControllerIndexRecord,
        &envelope.metadata().uid().as_str(),
    )
    .unwrap();
    let batch = ChangeBatch::new(d2b_contracts::v3::ZoneRevision::new(1), Vec::new()).unwrap();
    let batch_value = encode(ValueKind::ChangeBatch, &batch).unwrap();
    let mut meta = crate::transaction::current_meta(&database).unwrap();
    meta.current_revision = 1;
    let meta_value = encode(ValueKind::StoreMetaScalar, &meta).unwrap();
    let mut write = database.begin_write().unwrap();
    write.set_durability(Durability::Immediate).unwrap();
    write
        .open_table(RESOURCES)
        .unwrap()
        .insert(resource_key(&target).unwrap().as_slice(), value.as_slice())
        .unwrap();
    write
        .open_table(TYPE_INDEX)
        .unwrap()
        .insert(
            type_index_key(&target).unwrap().as_slice(),
            type_value.as_slice(),
        )
        .unwrap();
    let controller_key = crate::encode_key(
        KeySpace::ControllerIndex,
        &[
            KeyComponent::Text("Provider/system-core"),
            KeyComponent::Text("Host"),
            KeyComponent::Text(name),
        ],
    )
    .unwrap();
    write
        .open_table(CONTROLLER_INDEX)
        .unwrap()
        .insert(controller_key.as_bytes(), controller_value.as_slice())
        .unwrap();
    write
        .open_table(REVISION_LOG)
        .unwrap()
        .insert(revision_key(1).unwrap().as_slice(), batch_value.as_slice())
        .unwrap();
    write
        .open_table(STORE_META)
        .unwrap()
        .insert(
            crate::encode_key(KeySpace::StoreMeta, &[KeyComponent::Text("store")])
                .unwrap()
                .as_bytes(),
            meta_value.as_slice(),
        )
        .unwrap();
    write.commit().unwrap();
}

fn seed_two_hosts(directory: &tempfile::TempDir) {
    seed_host(directory, "host-system");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .unwrap();
    let backend = redb::backends::FileBackend::new(file).unwrap();
    let database = Database::builder().create_with_backend(backend).unwrap();
    let second = ResourceRef::parse("Host/host-worker").unwrap();
    let canonical_json = String::from_utf8(stored_body("host-worker"))
        .unwrap()
        .replace(
            "123e4567-e89b-42d3-a456-426614174000",
            "123e4567-e89b-42d3-a456-426614174001",
        )
        .into_bytes();
    let envelope = d2b_contracts::v3::ResourceEnvelope::from_json(&canonical_json).unwrap();
    let record = crate::transaction::ResourceRecord {
        canonical_json,
        owner_uid: None,
        controller_binding_id: "Provider/system-core".to_owned(),
        payload_digest: envelope.digest().unwrap(),
    };
    let write = database.begin_write().unwrap();
    let value = crate::transaction::encode(ValueKind::ResourceRecord, &record).unwrap();
    write
        .open_table(crate::transaction::RESOURCES)
        .unwrap()
        .insert(
            crate::transaction::resource_key(&second)
                .unwrap()
                .as_slice(),
            value.as_slice(),
        )
        .unwrap();
    let type_value = crate::transaction::encode(
        ValueKind::TypeIndexRecord,
        &envelope.metadata().uid().as_str(),
    )
    .unwrap();
    write
        .open_table(crate::transaction::TYPE_INDEX)
        .unwrap()
        .insert(
            crate::transaction::type_index_key(&second)
                .unwrap()
                .as_slice(),
            type_value.as_slice(),
        )
        .unwrap();
    let controller_key = crate::encode_key(
        KeySpace::ControllerIndex,
        &[
            KeyComponent::Text("Provider/system-core"),
            KeyComponent::Text("Host"),
            KeyComponent::Text("host-worker"),
        ],
    )
    .unwrap();
    let controller_value = crate::transaction::encode(
        ValueKind::ControllerIndexRecord,
        &envelope.metadata().uid().as_str(),
    )
    .unwrap();
    write
        .open_table(crate::transaction::CONTROLLER_INDEX)
        .unwrap()
        .insert(controller_key.as_bytes(), controller_value.as_slice())
        .unwrap();
    write.commit().unwrap();
}

fn seed_replay_log(directory: &tempfile::TempDir, rows: u64) {
    use crate::transaction::{REVISION_LOG, STORE_META, encode, revision_key};

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .unwrap();
    let backend = redb::backends::FileBackend::new(file).unwrap();
    let database = Database::builder().create_with_backend(backend).unwrap();
    crate::transaction::initialize(&database, &identity()).unwrap();
    let mut meta = crate::transaction::current_meta(&database).unwrap();
    meta.current_revision = rows;
    let mut write = database.begin_write().unwrap();
    write.set_durability(Durability::Immediate).unwrap();
    {
        let mut revisions = write.open_table(REVISION_LOG).unwrap();
        for revision in 1..=rows {
            let batch =
                ChangeBatch::new(d2b_contracts::v3::ZoneRevision::new(revision), Vec::new())
                    .unwrap();
            let value = encode(ValueKind::ChangeBatch, &batch).unwrap();
            revisions
                .insert(revision_key(revision).unwrap().as_slice(), value.as_slice())
                .unwrap();
        }
    }
    let value = encode(ValueKind::StoreMetaScalar, &meta).unwrap();
    write
        .open_table(STORE_META)
        .unwrap()
        .insert(
            crate::encode_key(KeySpace::StoreMeta, &[KeyComponent::Text("store")])
                .unwrap()
                .as_bytes(),
            value.as_slice(),
        )
        .unwrap();
    write.commit().unwrap();
}

#[test]
fn contract_constants_are_exact() {
    assert_eq!(WRITE_QUEUE_CAPACITY, 256);
    assert_eq!(GROUP_COMMIT_MAX, 16);
    assert_eq!(READ_POOL_THREADS, 4);
    assert_eq!(MAX_CONCURRENT_READS, 16);
    assert_eq!(READ_LIFETIME, std::time::Duration::from_millis(250));
}

#[test]
fn open_rejects_seal_from_a_foreign_pair() {
    let first = identity();
    let second = identity_for(
        StoreSlot::new(1).unwrap(),
        "work",
        "33333333-3333-4333-8333-333333333333",
    );
    let (issuer, _) = mutation_seal_pair(first.seal_identity());
    let (_, acceptor) = mutation_seal_pair(second.seal_identity());

    let error = acceptor
        .open(issuer.seal(empty_seal_body()))
        .err()
        .expect("a foreign seal must be refused");
    assert_eq!(error.reason_code(), "mutation-seal-authority-mismatch");
    assert_eq!(error.store_slot(), Some(StoreSlot::new(1).unwrap()));
}

#[test]
fn open_rejects_seal_bound_to_another_store_identity() {
    let first = identity();
    let sibling = identity_for(
        StoreSlot::new(0).unwrap(),
        "work",
        "44444444-4444-4444-8444-444444444444",
    );
    let (issuer, acceptor) = mutation_seal_pair(first.seal_identity());

    assert_eq!(
        acceptor.diagnose(&sibling.seal_identity()),
        Err(d2b_resource_store::SealIdentityMismatch::Store)
    );
    let (_, sibling_acceptor) = mutation_seal_pair(sibling.seal_identity());
    let error = sibling_acceptor
        .open(issuer.seal(empty_seal_body()))
        .err()
        .expect("a seal for another store must be refused");
    assert_eq!(error.reason_code(), "mutation-seal-authority-mismatch");
}

#[tokio::test]
async fn open_owned_rejects_acceptor_bound_to_another_zone() {
    let (_directory, file) = owned_file();
    let expected = identity();
    let foreign = identity_for(
        StoreSlot::new(0).unwrap(),
        "personal",
        "11111111-1111-4111-8111-111111111111",
    );
    let (_, acceptor) = mutation_seal_pair(foreign.seal_identity());

    let error = RedbResourceStore::open_owned(file, expected, acceptor)
        .await
        .expect_err("a cross-zone acceptor must be refused");
    assert_eq!(error.reason_code(), "mutation-seal-acceptor-zone-mismatch");
    assert_eq!(error.store_slot(), Some(StoreSlot::new(0).unwrap()));
}

#[tokio::test]
async fn open_owned_rejects_acceptor_bound_to_another_store_in_the_same_zone() {
    let (_directory, file) = owned_file();
    let expected = identity();
    let sibling = identity_for(
        StoreSlot::new(0).unwrap(),
        "work",
        "44444444-4444-4444-8444-444444444444",
    );
    let (_, acceptor) = mutation_seal_pair(sibling.seal_identity());

    let error = RedbResourceStore::open_owned(file, expected, acceptor)
        .await
        .expect_err("a sibling-store acceptor must be refused");
    assert_eq!(error.reason_code(), "mutation-seal-acceptor-store-mismatch");
    assert_eq!(error.store_slot(), Some(StoreSlot::new(0).unwrap()));
}

#[tokio::test]
async fn open_owned_rejects_acceptor_declaring_another_slot() {
    let (_directory, file) = owned_file();
    let expected = identity();
    let wrong_slot = identity_for(
        StoreSlot::new(1).unwrap(),
        "work",
        "11111111-1111-4111-8111-111111111111",
    );
    let (_, acceptor) = mutation_seal_pair(wrong_slot.seal_identity());

    let error = RedbResourceStore::open_owned(file, expected, acceptor)
        .await
        .expect_err("a wrong-slot acceptor must be refused");
    assert_eq!(error.reason_code(), "mutation-seal-acceptor-slot-mismatch");
    assert_eq!(error.store_slot(), Some(StoreSlot::new(0).unwrap()));
}

#[test]
fn diagnose_names_the_disagreeing_component_without_rendering_it() {
    let expected = identity();
    let matching = mutation_seal_pair(expected.seal_identity()).1;
    assert_eq!(matching.diagnose(&expected.seal_identity()), Ok(()));

    let zone = identity_for(
        StoreSlot::new(0).unwrap(),
        "personal",
        "11111111-1111-4111-8111-111111111111",
    );
    assert_eq!(
        matching.diagnose(&zone.seal_identity()),
        Err(d2b_resource_store::SealIdentityMismatch::Zone)
    );

    let store = identity_for(
        StoreSlot::new(0).unwrap(),
        "work",
        "44444444-4444-4444-8444-444444444444",
    );
    assert_eq!(
        matching.diagnose(&store.seal_identity()),
        Err(d2b_resource_store::SealIdentityMismatch::Store)
    );
    assert_eq!(
        d2b_resource_store::SealIdentityMismatch::Zone.reason_code(),
        "mutation-seal-acceptor-zone-mismatch"
    );
    assert_eq!(
        d2b_resource_store::SealIdentityMismatch::Store.reason_code(),
        "mutation-seal-acceptor-store-mismatch"
    );
}

#[tokio::test]
async fn errors_from_a_multi_store_startup_carry_distinct_slots() {
    let (_first_dir, first_file) = owned_file();
    let (_second_dir, second_file) = owned_file();
    let first = identity_for(
        StoreSlot::new(0).unwrap(),
        "work",
        "11111111-1111-4111-8111-111111111111",
    );
    let second = identity_for(
        StoreSlot::new(1).unwrap(),
        "work",
        "33333333-3333-4333-8333-333333333333",
    );
    let first_wrong = identity_for(
        StoreSlot::new(0).unwrap(),
        "personal",
        "11111111-1111-4111-8111-111111111111",
    );
    let second_wrong = identity_for(
        StoreSlot::new(1).unwrap(),
        "personal",
        "33333333-3333-4333-8333-333333333333",
    );
    let (_, first_acceptor) = mutation_seal_pair(first_wrong.seal_identity());
    let (_, second_acceptor) = mutation_seal_pair(second_wrong.seal_identity());

    let first_error = RedbResourceStore::open_owned(first_file, first, first_acceptor)
        .await
        .expect_err("slot zero startup must refuse its mismatched acceptor");
    let second_error = RedbResourceStore::open_owned(second_file, second, second_acceptor)
        .await
        .expect_err("slot one startup must refuse its mismatched acceptor");

    assert_eq!(first_error.store_slot(), Some(StoreSlot::new(0).unwrap()));
    assert_eq!(second_error.store_slot(), Some(StoreSlot::new(1).unwrap()));
    assert_eq!(
        first_error
            .clone()
            .with_store_slot(StoreSlot::new(1).unwrap()),
        second_error
    );
}

#[tokio::test]
async fn commit_rejects_seal_from_another_store() {
    let (_first_dir, first_file, first_marker) = provisioned_store();
    let first_identity = identity();
    let first = provision_store(first_file, first_marker, first_identity.clone())
        .await
        .unwrap();

    let second_identity = identity_for(
        StoreSlot::new(1).unwrap(),
        "work",
        "33333333-3333-4333-8333-333333333333",
    );
    let (second_dir, second_file) = owned_file();
    let mut second_marker = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(second_dir.path().join("store.marker"))
        .unwrap();
    write_provisioning_marker(&mut second_marker, &second_identity).unwrap();
    let _second = provision_store(second_file, second_marker, second_identity.clone())
        .await
        .unwrap();

    let (issuer, _) = mutation_seal_pair(second_identity.seal_identity());
    let error = first
        .commit_verified(issuer.seal(empty_seal_body()))
        .await
        .expect_err("cross-store evidence must be refused");
    assert_eq!(error.reason_code(), "mutation-seal-authority-mismatch");
    assert_eq!(error.store_slot(), Some(first_identity.slot()));
}

#[tokio::test]
async fn sealed_create_mints_uid_in_the_store_and_replays_without_it() {
    let (_directory, file, marker) = provisioned_store();
    let store_identity = identity();
    let (issuer, acceptor) = mutation_seal_pair(store_identity.seal_identity());
    let store = RedbResourceStore::provision_owned(file, marker, store_identity.clone(), acceptor)
        .await
        .unwrap();
    let name = "sealed-create";
    let canonical = create_body(name);
    assert!(!String::from_utf8_lossy(&canonical).contains("\"uid\""));
    let payload_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical);

    let result = store
        .commit_verified(issuer.seal(create_seal_body(
            "sealed-create",
            name,
            payload_digest.clone(),
        )))
        .await
        .unwrap();
    let uid = result.resources[0].uid.clone();
    assert_eq!(uid.as_str().as_bytes()[14], b'4');
    assert!(matches!(
        uid.as_str().as_bytes()[19],
        b'8' | b'9' | b'a' | b'b'
    ));
    assert_eq!(result.resources[0].uid, uid);
    let final_digest = result.resources[0].payload_digest.clone();
    assert_eq!(
        ResourceEnvelope::from_json(&result.resources[0].canonical_json)
            .unwrap()
            .digest()
            .unwrap(),
        final_digest
    );
    let persisted = store
        .get(StoreGetRequest {
            operation: operation("read-sealed-create"),
            zone: ZoneId::parse("work").unwrap(),
            target: ResourceRef::parse("Host/sealed-create").unwrap(),
            expected_uid: Some(uid.clone()),
            projection: StoreProjection::Full,
        })
        .await
        .unwrap();
    assert_eq!(persisted.uid, uid);
    assert_eq!(persisted.payload_digest, final_digest);

    let replay = store
        .commit_verified(issuer.seal(create_seal_body_with_resource(
            "sealed-create",
            name,
            canonical.clone(),
            payload_digest,
        )))
        .await
        .unwrap();
    assert_eq!(replay.resources[0].uid, uid);
    assert_eq!(replay.resources[0].payload_digest, final_digest);

    let changed_canonical = String::from_utf8(canonical.clone())
        .unwrap()
        .replace(
            "\"nonDisruptive\":\"automatic\"",
            "\"nonDisruptive\":\"manual\"",
        )
        .into_bytes();
    let changed_payload_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &changed_canonical);
    let error = store
        .commit_verified(issuer.seal(create_seal_body_with_resource(
            "sealed-create",
            name,
            changed_canonical,
            changed_payload_digest,
        )))
        .await
        .unwrap_err();
    assert_eq!(error.reason_code(), "operation-id-reused");

    let replacement = format!("sha256:{}", "f".repeat(64));
    let error = store
        .commit_verified(issuer.seal(create_seal_body("sealed-create", name, replacement)))
        .await
        .unwrap_err();
    assert_eq!(error.reason_code(), "mutation-payload-digest-mismatch");
}

#[tokio::test]
async fn owned_file_open_initializes_and_reopens_only_matching_identity() {
    let (directory, file, marker) = provisioned_store();
    let store = provision_store(file, marker, identity()).await.unwrap();
    assert_eq!(store.identity().zone().as_str(), "work");
    store.shutdown().await.unwrap();

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .unwrap();
    open_store(file, identity()).await.unwrap();

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .unwrap();
    let mut mismatch = identity();
    mismatch.zone = ZoneId::parse("personal").unwrap();
    let error = open_store(file, mismatch).await.unwrap_err();
    assert_eq!(
        error.kind(),
        d2b_resource_store::StoreErrorKind::StoreIntegrityFailure
    );
}

#[tokio::test]
async fn empty_existing_store_is_quarantined_without_publication_marker() {
    let (_directory, file) = owned_file();
    let error = open_store(file, identity()).await.unwrap_err();
    assert_eq!(
        error.kind(),
        d2b_resource_store::StoreErrorKind::StoreQuarantined
    );
    assert_eq!(error.reason_code(), "provisioned-store-empty");
}

#[tokio::test]
async fn clean_drop_reopens_without_crash_recovery_and_dirty_open_is_reported() {
    let (directory, file) = owned_file();
    let backend = redb::backends::FileBackend::new(file).unwrap();
    let database = Database::builder().create_with_backend(backend).unwrap();
    crate::transaction::initialize(&database, &identity()).unwrap();
    drop(database);

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .unwrap();
    let store = open_store(file, identity()).await.unwrap();
    assert!(store.recovered_after_crash());
    store.shutdown().await.unwrap();

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .unwrap();
    let store = open_store(file, identity()).await.unwrap();
    assert!(!store.recovered_after_crash());
}

#[tokio::test]
async fn direct_owned_fd_without_cloexec_fails_closed() {
    let (_directory, file) = owned_file();
    fcntl_setfd(&file, FdFlags::empty()).unwrap();
    let error = open_store(file, identity()).await.unwrap_err();
    assert_eq!(
        error.kind(),
        d2b_resource_store::StoreErrorKind::StoreIntegrityFailure
    );
}

#[tokio::test]
async fn owned_open_rejects_a_non_regular_fd() {
    let pipe = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).unwrap();
    let file = File::from(pipe.0);
    let error = open_store(file, identity()).await.unwrap_err();
    assert_eq!(
        error.kind(),
        d2b_resource_store::StoreErrorKind::StoreIntegrityFailure
    );
}

#[test]
fn scm_rights_receipt_is_atomic_cloexec_and_not_inherited_across_exec() {
    let (_directory, file) = owned_file();
    let (sender, receiver) = socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let descriptors = [file.as_fd()];
    let mut control_bytes = vec![0_u8; rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut control_bytes);
    assert!(control.push(SendAncillaryMessage::ScmRights(&descriptors)));
    assert_eq!(
        sendmsg(
            &sender,
            &[rustix::io::IoSlice::new(b"x")],
            &mut control,
            SendFlags::empty(),
        )
        .unwrap(),
        1
    );
    let received = receive_database_file(&receiver).unwrap();
    assert!(fcntl_getfd(&received).unwrap().contains(FdFlags::CLOEXEC));
    let fd = received.as_raw_fd();
    let status = Command::new("test")
        .args(["!", "-e"])
        .arg(format!("/proc/self/fd/{fd}"))
        .status()
        .unwrap();
    assert!(status.success(), "database fd survived exec");
}

#[test]
fn scm_rights_receipt_rejects_multiple_descriptors() {
    let (_first_directory, first) = owned_file();
    let (_second_directory, second) = owned_file();
    let (sender, receiver) = socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let descriptors = [first.as_fd(), second.as_fd()];
    let mut control_bytes = vec![0_u8; rustix::cmsg_space!(ScmRights(2))];
    let mut control = SendAncillaryBuffer::new(&mut control_bytes);
    assert!(control.push(SendAncillaryMessage::ScmRights(&descriptors)));
    sendmsg(
        &sender,
        &[rustix::io::IoSlice::new(b"x")],
        &mut control,
        SendFlags::empty(),
    )
    .unwrap();
    let error = receive_database_file(&receiver).unwrap_err();
    assert_eq!(
        error.kind(),
        d2b_resource_store::StoreErrorKind::StoreIntegrityFailure
    );
}

#[test]
fn scm_rights_receipt_exec_status_helper() {
    const HELPER_ENV: &str = "D2B_RESOURCE_STORE_EXEC_STATUS_HELPER";
    const STATUS_DUP_MIN_FD: i32 = 10;

    if std::env::var_os(HELPER_ENV).is_none() {
        return;
    }

    // `Command::stderr` safely hands the status pipe to fd 2, but that dup
    // clears CLOEXEC. Preserve the pipe on a high descriptor before replacing
    // fd 2 with /dev/null, then let exec close the preserved descriptor.
    let status = rustix::io::fcntl_dupfd_cloexec(rustix::stdio::stderr(), STATUS_DUP_MIN_FD)
        .expect("duplicate exec status fd");
    let error = Command::new("sleep")
        .arg("1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .exec();
    let _ = rustix::io::write(&status, &[1]);
    eprintln!("exec status helper could not exec sleep: {error}");
    std::process::exit(1);
}

#[test]
fn scm_rights_receipt_racing_fork_exec_never_leaks_the_database_inode() {
    const HELPER_ENV: &str = "D2B_RESOURCE_STORE_EXEC_STATUS_HELPER";
    const HELPER_TEST: &str = "tests::scm_rights_receipt_exec_status_helper";

    for _ in 0..32 {
        let (_directory, file) = owned_file();
        let metadata = file.metadata().unwrap();
        let inode = format!("{}:{}", metadata.dev(), metadata.ino());
        let (sender, receiver) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let descriptors = [file.as_fd()];
        let mut control_bytes = vec![0_u8; rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut control_bytes);
        assert!(control.push(SendAncillaryMessage::ScmRights(&descriptors)));
        sendmsg(
            &sender,
            &[rustix::io::IoSlice::new(b"x")],
            &mut control,
            SendFlags::empty(),
        )
        .unwrap();

        let barrier = Arc::new(Barrier::new(2));
        let receiver_barrier = Arc::clone(&barrier);
        let receipt = std::thread::spawn(move || {
            receiver_barrier.wait();
            receive_database_file(&receiver)
        });
        barrier.wait();
        let status_pipe =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("exec status pipe");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", HELPER_TEST])
            .env(HELPER_ENV, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(status_pipe.1));
        let mut child = command.spawn().unwrap();
        // `Command` retains its parent-side stdio descriptors after spawn.
        // Release the status writer so EOF means the helper's exec completed.
        drop(command);

        let mut status_byte = [0_u8; 1];
        let status_len = rustix::io::read(&status_pipe.0, &mut status_byte).unwrap();
        assert_eq!(
            status_len, 0,
            "exec status helper reported failure byte {:?}",
            status_byte[0]
        );
        let leaked = std::fs::read_dir(format!("/proc/{}/fd", child.id()))
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::metadata(entry.path()).ok())
            .any(|metadata| format!("{}:{}", metadata.dev(), metadata.ino()) == inode);
        let received = receipt.join().unwrap().unwrap();
        assert!(fcntl_getfd(&received).unwrap().contains(FdFlags::CLOEXEC));
        let status = child.wait().unwrap();
        assert!(!leaked, "database inode survived racing exec");
        assert!(status.success(), "database inode survived racing exec");
    }
}

#[tokio::test(start_paused = true)]
async fn read_lifetime_is_enforced_by_the_paused_clock() {
    let (_directory, file, marker) = provisioned_store();
    let store = provision_store(file, marker, identity()).await.unwrap();
    let store = Arc::new(store);
    let (started, started_receiver) = tokio::sync::oneshot::channel();
    let (release, release_receiver) = std::sync::mpsc::channel();
    let (completed, completed_receiver) = tokio::sync::oneshot::channel();
    let probe_store = Arc::clone(&store);
    let probe = tokio::spawn(async move {
        probe_store
            .reads
            .expiry_probe(started, release_receiver, completed)
            .await
    });
    started_receiver.await.unwrap();
    tokio::time::advance(READ_LIFETIME + std::time::Duration::from_millis(1)).await;
    let error = probe.await.unwrap().unwrap_err();
    assert_eq!(error.kind(), d2b_resource_store::StoreErrorKind::Timeout);
    assert_eq!(store.reads.available_permits(), MAX_CONCURRENT_READS - 1);
    release.send(()).unwrap();
    completed_receiver.await.unwrap();
    assert_eq!(store.reads.available_permits(), MAX_CONCURRENT_READS);
}

#[tokio::test]
async fn range_seek_skips_every_older_row() {
    let (_directory, file, marker) = provisioned_store();
    let store = provision_store(file, marker, identity()).await.unwrap();
    let process = ResourceTypeName::parse("Process").unwrap();
    let first = store
        .replay_backend(0, [process.clone()], |_| Ok(()))
        .await
        .unwrap();
    let second = store
        .replay_backend(0, [process], |_| Ok(()))
        .await
        .unwrap();
    assert_eq!(first.get(), 0);
    assert_eq!(second.get(), 0);
    let signals = loop {
        let signals = store.signals();
        if signals.revision_range_seeks == 2 {
            break signals;
        }
        tokio::task::yield_now().await;
    };
    assert_eq!(signals.revision_range_seeks, 2);
    assert_eq!(signals.replay_rows_scanned, 0);
    assert_eq!(signals.replay_rows_decoded, 0);
    assert_eq!(signals.writer_queue_capacity, 256);
}

#[tokio::test]
async fn replay_primitive_scans_larger_history_without_a_backend_queue() {
    let (directory, file) = owned_file();
    drop(file);
    seed_replay_log(&directory, 300);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .unwrap();
    let store = open_store(file, identity()).await.unwrap();
    let high_water = store
        .replay_backend(0, [ResourceTypeName::parse("Process").unwrap()], |_| Ok(()))
        .await
        .unwrap();
    assert_eq!(high_water.get(), 300);
    while {
        let signals = store.signals();
        signals.replay_rows_scanned < 300 || signals.replay_rows_decoded < 300
    } {
        tokio::task::yield_now().await;
    }
    let signals = store.signals();
    assert_eq!(signals.replay_rows_scanned, 300);
    assert_eq!(signals.replay_rows_decoded, 300);
}

#[tokio::test]
async fn public_read_path_enforces_zone_and_projection() {
    let (directory, file) = owned_file();
    drop(file);
    seed_two_hosts(&directory);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .unwrap();
    let store = open_store(file, identity()).await.unwrap();
    let target = ResourceRef::parse("Host/host-system").unwrap();
    let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let request = |zone: &str, projection| StoreGetRequest {
        operation: operation("get-host"),
        zone: ZoneId::parse(zone).unwrap(),
        target: target.clone(),
        expected_uid: Some(uid.clone()),
        projection,
    };

    let full = store
        .get(request("work", StoreProjection::Full))
        .await
        .unwrap();
    assert!(
        std::str::from_utf8(&full.canonical_json)
            .unwrap()
            .contains("\"status\"")
    );
    let base = store
        .get(request("work", StoreProjection::BaseOnly))
        .await
        .unwrap();
    assert_eq!(base.canonical_json, full.canonical_json);
    let metadata = store
        .get(request("work", StoreProjection::MetadataOnly))
        .await
        .unwrap();
    let metadata = std::str::from_utf8(&metadata.canonical_json).unwrap();
    assert!(metadata.contains("\"metadata\""));
    assert!(!metadata.contains("\"spec\""));
    assert!(!metadata.contains("\"status\""));
    let wrong_zone = store
        .get(request("personal", StoreProjection::Full))
        .await
        .unwrap_err();
    assert_eq!(
        wrong_zone.kind(),
        d2b_resource_store::StoreErrorKind::StoreIntegrityFailure
    );
}

#[tokio::test]
async fn list_cursor_is_bound_to_snapshot_and_selector() {
    let (directory, file) = owned_file();
    drop(file);
    seed_two_hosts(&directory);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("store.redb"))
        .unwrap();
    let store = open_store(file, identity()).await.unwrap();
    let request = |cursor, resource_types| StoreListRequest {
        operation: operation("list-host"),
        zone: ZoneId::parse("work").unwrap(),
        resource_types,
        resource_names: Vec::new(),
        filters: Vec::new(),
        page_size: 1,
        cursor,
        projection: StoreProjection::MetadataOnly,
    };
    let first = store.list(request(None, Vec::new())).await.unwrap();
    assert!(first.truncated);
    let cursor = first.next_cursor.unwrap();
    let error = store
        .list(request(
            Some(cursor.clone()),
            vec![ResourceTypeName::parse("Host").unwrap()],
        ))
        .await
        .unwrap_err();
    assert_eq!(error.reason_code(), "list-cursor-selector-mismatch");

    let mut stale = cursor.split('.').map(str::to_owned).collect::<Vec<_>>();
    stale[1] = "0".to_owned();
    let error = store
        .list(request(Some(stale.join(".")), Vec::new()))
        .await
        .unwrap_err();
    assert_eq!(
        error.kind(),
        d2b_resource_store::StoreErrorKind::RevisionExpired
    );
}

#[tokio::test]
async fn public_watch_replays_and_delivers_one_shared_committed_batch() {
    let (directory, file) = owned_file();
    let store_identity = identity();
    let (issuer, acceptor) = mutation_seal_pair(store_identity.seal_identity());
    let mut marker = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(directory.path().join("store.marker"))
        .unwrap();
    write_provisioning_marker(&mut marker, &store_identity).unwrap();
    let store = RedbResourceStore::provision_owned(file, marker, store_identity, acceptor)
        .await
        .unwrap();
    let receipt = store
        .watch(StoreWatchRequest {
            operation: operation("watch-host"),
            zone: ZoneId::parse("work").unwrap(),
            resource_types: vec![ResourceTypeName::parse("Host").unwrap()],
            resource_names: Vec::new(),
            filters: Vec::new(),
            after_revision: d2b_contracts::v3::ZoneRevision::new(0),
            initial_credits: 1,
            projection: StoreProjection::Full,
        })
        .await
        .unwrap();
    let mut stream = store
        .take_watch_stream_named(&receipt.stream_name)
        .unwrap()
        .expect("receipt stream is retained until transfer");
    assert!(
        store
            .take_watch_stream_named(&receipt.stream_name)
            .unwrap()
            .is_none()
    );
    let (_second_receipt, mut second_stream) = store
        .watch_stream(StoreWatchRequest {
            operation: operation("watch-host-second"),
            zone: ZoneId::parse("work").unwrap(),
            resource_types: vec![ResourceTypeName::parse("Host").unwrap()],
            resource_names: Vec::new(),
            filters: Vec::new(),
            after_revision: d2b_contracts::v3::ZoneRevision::new(0),
            initial_credits: 1,
            projection: StoreProjection::Full,
        })
        .await
        .unwrap();
    assert_eq!(receipt.snapshot_revision.get(), 0);

    let canonical = create_body("watch-host");
    let payload_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical);
    let result = store
        .commit_verified(issuer.seal(create_seal_body("watch-host", "watch-host", payload_digest)))
        .await
        .unwrap();
    let batch = stream.recv().await.expect("committed batch is delivered");
    let second_batch = second_stream
        .recv()
        .await
        .expect("the second watcher receives the same batch");
    assert_eq!(batch.revision(), result.revision);
    assert!(batch.shares_batch_with(&second_batch));
    assert_eq!(batch.entries().len(), 1);
    assert!(batch.shares_batch_with(&batch));
    assert_eq!(store.watch_signals().unwrap().budget_used, 2);
    let backend_signals = store.signals();
    assert_eq!(backend_signals.shared_immutable_batches, 1);
    assert_eq!(backend_signals.fanout_references, 2);

    store
        .acknowledge_watch(stream.id(), result.revision)
        .await
        .unwrap();
    store
        .acknowledge_watch(second_stream.id(), result.revision)
        .await
        .unwrap();
    assert_eq!(store.watch_signals().unwrap().budget_used, 0);
}

#[test]
fn persisted_dtos_reject_unknown_fields() {
    let mut value = serde_json::to_value(crate::transaction::StoreMeta {
        store_uuid: "11111111-1111-4111-8111-111111111111".to_owned(),
        zone_name: "work".to_owned(),
        zone_uid: "22222222-2222-4222-8222-222222222222".to_owned(),
        created_at: "2026-07-31T00:00:00.000Z".to_owned(),
        schema_version: 1,
        current_revision: 0,
        compaction_floor: 0,
        active_configuration_revision: 9,
        policy_revision: 7,
        api_catalog_revision: 8,
        controller_generation: None,
        clean_shutdown: false,
        backup_generation: 0,
    })
    .unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("extra".to_owned(), serde_json::Value::Bool(true));
    let canonical = d2b_contracts::v3::canonical_json_bytes(&value).unwrap();
    let framed = encode_value(ValueKind::StoreMetaScalar, &canonical).unwrap();
    let error = crate::transaction::decode::<crate::transaction::StoreMeta>(
        ValueKind::StoreMetaScalar,
        framed.as_bytes(),
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        d2b_resource_store::StoreErrorKind::StoreIntegrityFailure
    );
}

#[test]
fn source_policy_pins_redb_features_and_forbids_reduced_durability_calls() {
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.contains("redb = { version = \"=4.1.0\", default-features = false }"));
    let sources = [
        include_str!("lib.rs"),
        include_str!("actor.rs"),
        include_str!("transaction.rs"),
    ];
    for source in sources {
        assert!(!source.contains("Durability::None"));
        assert!(!source.contains("Durability::Paranoid"));
        assert!(!source.contains("set_two_phase_commit"));
    }
    assert_eq!(
        include_str!("transaction.rs")
            .matches("set_durability(Durability::Immediate)")
            .count(),
        1
    );
}

#[test]
fn checked_mutation_constructors_and_raw_commit_path_are_not_public() {
    let source = include_str!("lib.rs");
    assert!(!source.contains("pub struct CheckedMutation"));
    assert!(!source.contains("pub struct CheckedPreparedMutation"));
    assert!(!source.contains("pub async fn commit_checked"));
    assert!(source.contains("pub struct RedbResourceStore"));
    assert!(source.contains("SealedMutation"));
    assert!(!source.contains("MutationView"));
    assert!(!source.contains("type_name"));
}
