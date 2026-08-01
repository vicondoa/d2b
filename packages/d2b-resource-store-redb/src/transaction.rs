//! One crash-safe redb transaction for a verified resource mutation.

use d2b_contracts::v3::{
    CanonicalJsonValue, ControllerGeneration, ResourceEnvelope, ResourceRef, ResourceUid,
    RetryClass, ZoneId, ZoneRevision,
};
use d2b_resource_store::{
    AdmittedAuthorization, ExpectedRevision, MutationOrdinal, PolicySnapshot, ResourceMutationKind,
    StoreCommitResult, StoreError, StoreErrorKind, StoreMutation, StoreOperationContext,
    StoredResource,
};
use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::{
    CheckedMutation, DecodedKey, KeyComponent, KeySpace, ValueKind, encode_key, encode_value,
};

pub(crate) const STORE_META: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("store_meta");
pub(crate) const API_SCHEMAS: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("api_schemas");
pub(crate) const RESOURCES: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("resources");
pub(crate) const TYPE_INDEX: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("type_index");
pub(crate) const OWNER_INDEX: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("owner_index");
pub(crate) const PRODUCER_INDEX: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("producer_index");
pub(crate) const CONTROLLER_INDEX: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("controller_index");
pub(crate) const REVISION_LOG: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("revision_log");
pub(crate) const OPERATIONS: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("operations");
pub(crate) const ZONE_LINK_CURSORS: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("zone_link_cursors");

pub(crate) const ALL_TABLES: [TableDefinition<'static, &[u8], &[u8]>; 10] = [
    STORE_META,
    API_SCHEMAS,
    RESOURCES,
    TYPE_INDEX,
    OWNER_INDEX,
    PRODUCER_INDEX,
    CONTROLLER_INDEX,
    REVISION_LOG,
    OPERATIONS,
    ZONE_LINK_CURSORS,
];

pub(crate) const PHYSICAL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoreMeta {
    pub store_uuid: String,
    pub zone_name: String,
    pub zone_uid: String,
    pub created_at: String,
    pub schema_version: u32,
    pub current_revision: u64,
    pub compaction_floor: u64,
    pub active_configuration_revision: u64,
    pub policy_revision: u64,
    pub api_catalog_revision: u64,
    pub controller_generation: Option<u64>,
    pub clean_shutdown: bool,
    pub backup_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ResourceRecord {
    pub canonical_json: Vec<u8>,
    pub owner_uid: Option<String>,
    pub controller_binding_id: String,
    pub payload_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OwnerIndexRecord {
    pub resource_type: String,
    pub resource_name: String,
    pub latest_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProducerIndexRecord {
    pub endpoint_type: String,
    pub endpoint_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OperationRecord {
    pub request_digest: String,
    pub resource_uids: Vec<String>,
    pub resources: Vec<OperationResourceRecord>,
    pub outcome: String,
    pub accepted_revision: u64,
    pub finished_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OperationResourceRecord {
    pub resource_type: String,
    pub resource_name: String,
    pub zone: String,
    pub canonical_json: Vec<u8>,
    pub payload_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEntry {
    pub ordinal: u32,
    pub resource_type: String,
    pub resource_name: String,
    pub resource_uid: String,
    pub event: String,
    pub old_generation: Option<u64>,
    pub new_generation: Option<u64>,
    pub owner_uid: Option<String>,
    pub payload_digest: String,
    pub canonical_resource: Option<Vec<u8>>,
    pub operation_id: String,
    pub correlation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeBatch {
    pub revision: u64,
    pub entries: Vec<ChangeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommittedGroup {
    pub results: Vec<Result<StoreCommitResult, StoreError>>,
    pub batch: Option<ChangeBatch>,
}

pub(crate) struct VerifiedWrite {
    authorization: AdmittedAuthorization,
    policy_snapshot: PolicySnapshot,
    operation: StoreOperationContext,
    mutations: Vec<VerifiedPreparedMutation>,
}

pub(crate) struct VerifiedPreparedMutation {
    mutation: StoreMutation,
    resource_uid: Option<ResourceUid>,
    payload_digest: Option<String>,
}

#[cfg(test)]
pub(crate) fn empty_write_request_for_test(
    sequence: u64,
    principal: &str,
    resource: ResourceRef,
    queue_permit: tokio::sync::OwnedSemaphorePermit,
) -> crate::actor::WriteRequest {
    let (response, _receiver) = tokio::sync::oneshot::channel();
    crate::actor::WriteRequest {
        sequence,
        principal: principal.to_owned(),
        resources: vec![resource],
        mutation: VerifiedWrite {
            authorization: AdmittedAuthorization {
                zone: ZoneId::parse("work").unwrap(),
                subject_ref: ResourceRef::parse("Provider/system-core").unwrap(),
                subject_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                targets: Vec::new(),
            },
            policy_snapshot: PolicySnapshot {
                policy_revision: 1,
                api_catalog_revision: 1,
                active_configuration_revision: d2b_contracts::v3::ConfigurationGeneration::new(1)
                    .unwrap(),
                controller_generation: None,
            },
            operation: StoreOperationContext {
                operation_id: format!("op-{sequence}"),
                idempotency_key: None,
                correlation_id: format!("corr-{sequence}"),
                trace_id: None,
                deadline_ms: 1,
            },
            mutations: Vec::new(),
        },
        queue_permit,
        response,
    }
}

impl VerifiedPreparedMutation {
    fn mutation(&self) -> &StoreMutation {
        &self.mutation
    }

    fn resource_uid(&self) -> Option<&ResourceUid> {
        self.resource_uid.as_ref()
    }

    fn payload_digest(&self) -> Option<&str> {
        self.payload_digest.as_deref()
    }
}

impl From<CheckedMutation> for VerifiedWrite {
    fn from(verified: CheckedMutation) -> Self {
        Self {
            authorization: verified.authorization,
            policy_snapshot: verified.policy_snapshot,
            operation: verified.operation,
            mutations: verified
                .mutations
                .into_iter()
                .map(|prepared| VerifiedPreparedMutation {
                    mutation: prepared.mutation,
                    resource_uid: prepared.resource_uid,
                    payload_digest: prepared.payload_digest,
                })
                .collect(),
        }
    }
}

pub(crate) fn initialize(
    database: &Database,
    identity: &crate::StoreIdentity,
) -> Result<(), StoreError> {
    let mut write = database.begin_write().map_err(integrity)?;
    set_full_durability(&mut write)?;
    for definition in ALL_TABLES {
        drop(write.open_table(definition).map_err(integrity)?);
    }
    let mut meta = write.open_table(STORE_META).map_err(integrity)?;
    let key = meta_key();
    if meta.get(key.as_slice()).map_err(integrity)?.is_some() {
        return Err(integrity("store-meta-already-exists"));
    }
    let record = StoreMeta {
        store_uuid: identity.store_uuid.clone(),
        zone_name: identity.zone.as_str().to_owned(),
        zone_uid: identity.zone_uid.as_str().to_owned(),
        created_at: identity.created_at.clone(),
        schema_version: PHYSICAL_SCHEMA_VERSION,
        current_revision: 0,
        compaction_floor: 0,
        active_configuration_revision: identity.revisions.active_configuration_revision.get(),
        policy_revision: identity.revisions.policy_revision,
        api_catalog_revision: identity.revisions.api_catalog_revision,
        controller_generation: identity
            .revisions
            .controller_generation
            .map(ControllerGeneration::get),
        clean_shutdown: false,
        backup_generation: 0,
    };
    let value = encode(ValueKind::StoreMetaScalar, &record)?;
    meta.insert(key.as_slice(), value.as_slice())
        .map_err(integrity)?;
    drop(meta);
    write.commit().map_err(integrity)
}

pub(crate) fn validate_identity(
    database: &Database,
    identity: &crate::StoreIdentity,
) -> Result<StoreMeta, StoreError> {
    let read = database.begin_read().map_err(integrity)?;
    if read.list_tables().map_err(integrity)?.count() != ALL_TABLES.len() {
        return Err(integrity("physical-table-set-invalid"));
    }
    let table = read.open_table(STORE_META).map_err(integrity)?;
    let bytes = table
        .get(meta_key().as_slice())
        .map_err(integrity)?
        .ok_or_else(|| integrity("store-meta-missing"))?;
    let meta: StoreMeta = decode(ValueKind::StoreMetaScalar, bytes.value())?;
    if meta.schema_version != PHYSICAL_SCHEMA_VERSION
        || meta.store_uuid != identity.store_uuid
        || meta.zone_name != identity.zone.as_str()
        || meta.zone_uid != identity.zone_uid.as_str()
        || meta.compaction_floor > meta.current_revision
        || !revisions_match(&meta, identity.revisions)
    {
        return Err(integrity("store-identity-mismatch"));
    }
    Ok(meta)
}

pub(crate) fn validate_consistency(database: &Database) -> Result<(), StoreError> {
    use redb::ReadableTableMetadata;

    let read = database.begin_read().map_err(integrity)?;
    let meta = read_meta(&read)?;
    let revisions = read.open_table(REVISION_LOG).map_err(integrity)?;
    let mut revision_count = 0_u64;
    for row in revisions.iter().map_err(integrity)? {
        let (key, value) = row.map_err(integrity)?;
        let decoded = DecodedKey::decode(key.value()).map_err(integrity)?;
        let [crate::DecodedKeyComponent::U64(revision)] = decoded.components() else {
            return Err(integrity("revision-key-shape-invalid"));
        };
        if *revision <= meta.compaction_floor || *revision > meta.current_revision {
            return Err(integrity("revision-log-range-invalid"));
        }
        let batch: ChangeBatch = decode(ValueKind::ChangeBatch, value.value())?;
        if batch.revision != *revision {
            return Err(integrity("revision-log-key-value-mismatch"));
        }
        revision_count += 1;
    }
    if revision_count != meta.current_revision.saturating_sub(meta.compaction_floor) {
        return Err(integrity("revision-log-not-contiguous"));
    }

    let resources = read.open_table(RESOURCES).map_err(integrity)?;
    let types = read.open_table(TYPE_INDEX).map_err(integrity)?;
    let owners = read.open_table(OWNER_INDEX).map_err(integrity)?;
    let producers = read.open_table(PRODUCER_INDEX).map_err(integrity)?;
    let controllers = read.open_table(CONTROLLER_INDEX).map_err(integrity)?;
    let mut expected_owners = 0_u64;
    let mut expected_producers = 0_u64;
    for row in resources.iter().map_err(integrity)? {
        let (key, value) = row.map_err(integrity)?;
        let resource_ref = resource_ref_from_key(key.value())?;
        let record: ResourceRecord = decode(ValueKind::ResourceRecord, value.value())?;
        let envelope = ResourceEnvelope::from_json(&record.canonical_json)
            .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
        if envelope.resource_type() != resource_ref.resource_type()
            || envelope.metadata().name() != resource_ref.name()
            || envelope.metadata().zone().as_str() != meta.zone_name
            || envelope.metadata().revision().get() > meta.current_revision
        {
            return Err(integrity("stored-resource-identity-invalid"));
        }
        let uid = envelope.metadata().uid();
        let type_value = types
            .get(type_index_key(&resource_ref)?.as_slice())
            .map_err(integrity)?
            .ok_or_else(|| integrity("type-index-entry-missing"))?;
        let indexed_uid: String = decode(ValueKind::TypeIndexRecord, type_value.value())?;
        if indexed_uid != uid.as_str() {
            return Err(integrity("type-index-entry-mismatch"));
        }
        let controller_key = encode_key(
            KeySpace::ControllerIndex,
            &[
                KeyComponent::Text(&record.controller_binding_id),
                KeyComponent::Text(resource_ref.resource_type().as_str()),
                KeyComponent::Text(resource_ref.name().as_str()),
            ],
        )
        .map_err(integrity)?;
        let controller_value = controllers
            .get(controller_key.as_bytes())
            .map_err(integrity)?
            .ok_or_else(|| integrity("controller-index-entry-missing"))?;
        let controller_uid: String =
            decode(ValueKind::ControllerIndexRecord, controller_value.value())?;
        if controller_uid != uid.as_str() {
            return Err(integrity("controller-index-entry-mismatch"));
        }
        if let Some(owner_uid) = &record.owner_uid {
            expected_owners += 1;
            let owner_key = encode_key(
                KeySpace::OwnerIndex,
                &[
                    KeyComponent::Text(owner_uid),
                    KeyComponent::Text(uid.as_str()),
                ],
            )
            .map_err(integrity)?;
            let owner_value = owners
                .get(owner_key.as_bytes())
                .map_err(integrity)?
                .ok_or_else(|| integrity("owner-index-entry-missing"))?;
            let owner_record: OwnerIndexRecord =
                decode(ValueKind::OwnerIndexRecord, owner_value.value())?;
            if owner_record.resource_type != resource_ref.resource_type().as_str()
                || owner_record.resource_name != resource_ref.name().as_str()
                || owner_record.latest_revision != envelope.metadata().revision().get()
            {
                return Err(integrity("owner-index-entry-mismatch"));
            }
        }
        if let Some(producer_ref) = endpoint_producer(&envelope) {
            expected_producers += 1;
            let producer_uid = types
                .get(type_index_key(&producer_ref)?.as_slice())
                .map_err(integrity)?
                .ok_or_else(|| integrity("producer-resource-missing"))?;
            let producer_uid: String = decode(ValueKind::TypeIndexRecord, producer_uid.value())?;
            let producer_key = encode_key(
                KeySpace::ProducerIndex,
                &[
                    KeyComponent::Text(&producer_uid),
                    KeyComponent::Text(uid.as_str()),
                ],
            )
            .map_err(integrity)?;
            let producer_value = producers
                .get(producer_key.as_bytes())
                .map_err(integrity)?
                .ok_or_else(|| integrity("producer-index-entry-missing"))?;
            let producer_record: ProducerIndexRecord =
                decode(ValueKind::ProducerIndexRecord, producer_value.value())?;
            if producer_record.endpoint_type != resource_ref.resource_type().as_str()
                || producer_record.endpoint_name != resource_ref.name().as_str()
            {
                return Err(integrity("producer-index-entry-mismatch"));
            }
        }
    }
    let resource_count = resources.len().map_err(integrity)?;
    if types.len().map_err(integrity)? != resource_count
        || controllers.len().map_err(integrity)? != resource_count
        || owners.len().map_err(integrity)? != expected_owners
        || producers.len().map_err(integrity)? != expected_producers
    {
        return Err(integrity("resource-index-count-mismatch"));
    }
    let operations = read.open_table(OPERATIONS).map_err(integrity)?;
    for row in operations.iter().map_err(integrity)? {
        let (_, value) = row.map_err(integrity)?;
        let operation: OperationRecord = decode(ValueKind::OperationRecord, value.value())?;
        if operation.accepted_revision > operation.finished_revision
            || operation.finished_revision > meta.current_revision
        {
            return Err(integrity("operation-revision-invalid"));
        }
    }
    Ok(())
}

fn resource_ref_from_key(bytes: &[u8]) -> Result<ResourceRef, StoreError> {
    let decoded = DecodedKey::decode(bytes).map_err(integrity)?;
    if decoded.key_space() != KeySpace::Resources {
        return Err(integrity("resource-key-space-invalid"));
    }
    let [
        crate::DecodedKeyComponent::Text(resource_type),
        crate::DecodedKeyComponent::Text(resource_name),
    ] = decoded.components()
    else {
        return Err(integrity("resource-key-shape-invalid"));
    };
    ResourceRef::parse(&format!("{resource_type}/{resource_name}")).map_err(integrity)
}

pub(crate) fn current_meta(database: &Database) -> Result<StoreMeta, StoreError> {
    let read = database.begin_read().map_err(integrity)?;
    read_meta(&read)
}

pub(crate) fn read_meta(read: &redb::ReadTransaction) -> Result<StoreMeta, StoreError> {
    let table = read.open_table(STORE_META).map_err(integrity)?;
    let bytes = table
        .get(meta_key().as_slice())
        .map_err(integrity)?
        .ok_or_else(|| integrity("store-meta-missing"))?;
    decode(ValueKind::StoreMetaScalar, bytes.value())
}

pub(crate) fn set_clean_shutdown(
    database: &Database,
    clean_shutdown: bool,
) -> Result<(), StoreError> {
    let mut write = database.begin_write().map_err(integrity)?;
    set_full_durability(&mut write)?;
    let mut meta = read_meta_in_write(&write)?;
    if meta.clean_shutdown == clean_shutdown {
        write.abort().map_err(integrity)?;
        return Ok(());
    }
    meta.clean_shutdown = clean_shutdown;
    let value = encode(ValueKind::StoreMetaScalar, &meta)?;
    write
        .open_table(STORE_META)
        .map_err(integrity)?
        .insert(meta_key().as_slice(), value.as_slice())
        .map_err(integrity)?;
    write.commit().map_err(integrity)
}

pub(crate) fn apply_group(
    database: &Database,
    group: Vec<VerifiedWrite>,
) -> Result<CommittedGroup, StoreError> {
    if group.is_empty() {
        return Ok(CommittedGroup {
            results: Vec::new(),
            batch: None,
        });
    }

    let mut write = database.begin_write().map_err(integrity)?;
    set_full_durability(&mut write)?;
    let mut meta = read_meta_in_write(&write)?;
    let Some(revision) = meta.current_revision.checked_add(1) else {
        return Err(integrity("zone-revision-exhausted"));
    };
    let mut results = Vec::with_capacity(group.len());
    let mut entries = Vec::new();
    let mut accepted_targets = std::collections::BTreeSet::new();

    for verified in group {
        let snapshot = verified.policy_snapshot;
        if verified.mutations.is_empty()
            || verified.mutations.len() > d2b_contracts::v3::MAX_BATCH_MUTATIONS
        {
            results.push(Err(integrity("empty-verified-mutation")));
            continue;
        }
        if !revisions_match(&meta, snapshot) {
            results.push(Err(authorization_denied(meta.current_revision)));
            continue;
        }
        let operation_id = verified.operation.operation_id.clone();
        let correlation_id = verified.operation.correlation_id.clone();
        let operation_key = operation_key(&operation_id)?;
        {
            let operations = write.open_table(OPERATIONS).map_err(integrity)?;
            if let Some(bytes) = operations
                .get(operation_key.as_slice())
                .map_err(integrity)?
            {
                let prior: OperationRecord = decode(ValueKind::OperationRecord, bytes.value())?;
                if prior.request_digest == operation_digest(&verified) {
                    results.push(Ok(StoreCommitResult {
                        resources: prior
                            .resources
                            .iter()
                            .map(operation_resource)
                            .collect::<Result<Vec<_>, _>>()?,
                        revision: ZoneRevision::new(prior.finished_revision),
                    }));
                } else {
                    results.push(Err(conflict(
                        meta.current_revision,
                        0,
                        "operation-id-reused",
                    )));
                }
                continue;
            }
        }

        let result_index = results.len();
        results.push(Err(integrity("unresolved-write-result")));
        if let Err(error) = validate_verified_write(&write, &verified, revision, &accepted_targets)
        {
            results[result_index] = Err(error);
            continue;
        }
        let mut simulated = read_simulated_state(&write)?;
        if let Err(error) = validate_structural_group(&verified, &mut simulated) {
            results[result_index] = Err(error);
            continue;
        }
        let mut group_resources = Vec::new();
        let mut group_entries = Vec::new();
        for (ordinal, prepared) in verified.mutations.iter().enumerate() {
            let (resource, entry) = apply_prepared(
                &write,
                prepared,
                revision,
                u32::try_from(ordinal).map_err(integrity)?,
                &operation_id,
                &correlation_id,
            )?;
            group_resources.push(resource);
            group_entries.push(entry);
        }
        let operation = OperationRecord {
            request_digest: operation_digest(&verified),
            resource_uids: group_resources
                .iter()
                .map(|resource| resource.uid.as_str().to_owned())
                .collect(),
            resources: group_resources
                .iter()
                .map(|resource| OperationResourceRecord {
                    resource_type: resource.resource_ref.resource_type().as_str().to_owned(),
                    resource_name: resource.resource_ref.name().as_str().to_owned(),
                    zone: resource.zone.as_str().to_owned(),
                    canonical_json: resource.canonical_json.clone(),
                    payload_digest: resource.payload_digest.clone(),
                })
                .collect(),
            outcome: "committed".to_owned(),
            accepted_revision: revision,
            finished_revision: revision,
        };
        let operation_value = encode(ValueKind::OperationRecord, &operation)?;
        write
            .open_table(OPERATIONS)
            .map_err(integrity)?
            .insert(operation_key.as_slice(), operation_value.as_slice())
            .map_err(integrity)?;
        results[result_index] = Ok(StoreCommitResult {
            resources: group_resources.clone(),
            revision: ZoneRevision::new(revision),
        });
        accepted_targets.extend(
            verified
                .mutations
                .iter()
                .map(|prepared| prepared.mutation().target.clone()),
        );
        entries.extend(group_entries);
    }

    if entries.is_empty() {
        write.abort().map_err(integrity)?;
        return Ok(CommittedGroup {
            results,
            batch: None,
        });
    }
    for (ordinal, entry) in entries.iter_mut().enumerate() {
        entry.ordinal = u32::try_from(ordinal).map_err(integrity)?;
    }
    let batch = ChangeBatch { revision, entries };
    let batch_key = revision_key(revision)?;
    let batch_value = encode(ValueKind::ChangeBatch, &batch)?;
    write
        .open_table(REVISION_LOG)
        .map_err(integrity)?
        .insert(batch_key.as_slice(), batch_value.as_slice())
        .map_err(integrity)?;
    meta.current_revision = revision;
    let meta_value = encode(ValueKind::StoreMetaScalar, &meta)?;
    write
        .open_table(STORE_META)
        .map_err(integrity)?
        .insert(meta_key().as_slice(), meta_value.as_slice())
        .map_err(integrity)?;
    write.commit().map_err(integrity)?;
    Ok(CommittedGroup {
        results,
        batch: Some(batch),
    })
}

fn read_simulated_state(
    write: &redb::WriteTransaction,
) -> Result<std::collections::BTreeMap<ResourceRef, (ResourceUid, Option<ResourceRef>)>, StoreError>
{
    let table = write.open_table(RESOURCES).map_err(integrity)?;
    table
        .iter()
        .map_err(integrity)?
        .map(|row| {
            let (key, value) = row.map_err(integrity)?;
            let decoded = DecodedKey::decode(key.value()).map_err(integrity)?;
            let [
                crate::DecodedKeyComponent::Text(resource_type),
                crate::DecodedKeyComponent::Text(resource_name),
            ] = decoded.components()
            else {
                return Err(integrity("resource-key-shape-invalid"));
            };
            let resource_ref = ResourceRef::parse(&format!("{resource_type}/{resource_name}"))
                .map_err(integrity)?;
            let record: ResourceRecord = decode(ValueKind::ResourceRecord, value.value())?;
            let envelope = ResourceEnvelope::from_json(&record.canonical_json)
                .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
            Ok((
                resource_ref,
                (
                    envelope.metadata().uid().clone(),
                    envelope.metadata().owner_ref().cloned(),
                ),
            ))
        })
        .collect()
}

fn validate_structural_group(
    verified: &VerifiedWrite,
    state: &mut std::collections::BTreeMap<ResourceRef, (ResourceUid, Option<ResourceRef>)>,
) -> Result<(), StoreError> {
    for prepared in &verified.mutations {
        let mutation = prepared.mutation();
        if mutation.kind == ResourceMutationKind::Delete {
            let children_remain = state
                .values()
                .any(|(_, owner)| owner.as_ref() == Some(&mutation.target));
            if children_remain {
                return Err(error(
                    StoreErrorKind::ResourceFinalizerDenied,
                    None,
                    "owned-children-remain",
                ));
            }
            state.remove(&mutation.target);
            continue;
        }
        let uid = prepared
            .resource_uid()
            .cloned()
            .ok_or_else(|| integrity("mutation-resource-uid-missing"))?;
        if let Some(owner) = &mutation.owner {
            if !state.contains_key(owner) {
                return Err(error(
                    StoreErrorKind::ResourceRefInvalid,
                    None,
                    "owner-ref-not-found",
                ));
            }
            if owner == &mutation.target || owner_path_reaches(state, owner, &mutation.target) {
                return Err(error(
                    StoreErrorKind::ResourceOwnerCycle,
                    None,
                    "resource-owner-cycle",
                ));
            }
            if owner_path_depth(state, owner)? >= crate::MAX_OWNER_CHAIN_DEPTH {
                return Err(error(
                    StoreErrorKind::ResourceOwnerDepth,
                    None,
                    "resource-owner-depth",
                ));
            }
        }
        state.insert(mutation.target.clone(), (uid, mutation.owner.clone()));
    }
    Ok(())
}

fn owner_path_reaches(
    state: &std::collections::BTreeMap<ResourceRef, (ResourceUid, Option<ResourceRef>)>,
    start: &ResourceRef,
    target: &ResourceRef,
) -> bool {
    let mut cursor = Some(start);
    let mut visited = std::collections::BTreeSet::new();
    while let Some(resource_ref) = cursor {
        if resource_ref == target || !visited.insert(resource_ref.clone()) {
            return true;
        }
        cursor = state
            .get(resource_ref)
            .and_then(|(_, owner)| owner.as_ref());
    }
    false
}

fn owner_path_depth(
    state: &std::collections::BTreeMap<ResourceRef, (ResourceUid, Option<ResourceRef>)>,
    start: &ResourceRef,
) -> Result<usize, StoreError> {
    let mut cursor = Some(start);
    let mut visited = std::collections::BTreeSet::new();
    let mut depth = 0;
    while let Some(resource_ref) = cursor {
        if !visited.insert(resource_ref.clone()) {
            return Err(error(
                StoreErrorKind::ResourceOwnerCycle,
                None,
                "resource-owner-cycle",
            ));
        }
        depth += 1;
        cursor = state
            .get(resource_ref)
            .and_then(|(_, owner)| owner.as_ref());
    }
    Ok(depth)
}

fn operation_resource(record: &OperationResourceRecord) -> Result<StoredResource, StoreError> {
    let resource_ref = ResourceRef::parse(&format!(
        "{}/{}",
        record.resource_type, record.resource_name
    ))
    .map_err(integrity)?;
    stored_resource(
        &ZoneId::parse(&record.zone).map_err(integrity)?,
        &resource_ref,
        &ResourceRecord {
            canonical_json: record.canonical_json.clone(),
            owner_uid: None,
            controller_binding_id: String::new(),
            payload_digest: record.payload_digest.clone(),
        },
    )
}

fn apply_prepared(
    write: &redb::WriteTransaction,
    prepared: &VerifiedPreparedMutation,
    revision: u64,
    ordinal: u32,
    operation_id: &str,
    correlation_id: &str,
) -> Result<(StoredResource, ChangeEntry), StoreError> {
    let mutation = prepared.mutation();
    let key = resource_key(&mutation.target)?;
    let previous = {
        let resources = write.open_table(RESOURCES).map_err(integrity)?;
        resources
            .get(key.as_slice())
            .map_err(integrity)?
            .map(|bytes| decode::<ResourceRecord>(ValueKind::ResourceRecord, bytes.value()))
            .transpose()?
    };
    let previous_resource = previous
        .as_ref()
        .map(|record| stored_resource(&mutation.zone, &mutation.target, record))
        .transpose()?;

    if mutation.kind == ResourceMutationKind::Delete {
        let Some(old) = previous_resource else {
            return Err(error(
                StoreErrorKind::ResourceNotFound,
                None,
                "resource-not-found",
            ));
        };
        remove_indexes(write, &old, previous.as_ref().unwrap())?;
        write
            .open_table(RESOURCES)
            .map_err(integrity)?
            .remove(key.as_slice())
            .map_err(integrity)?;
        return Ok((
            old.clone(),
            ChangeEntry {
                ordinal,
                resource_type: mutation.target.resource_type().as_str().to_owned(),
                resource_name: mutation.target.name().as_str().to_owned(),
                resource_uid: old.uid.as_str().to_owned(),
                event: "deleted".to_owned(),
                old_generation: Some(old.generation.get()),
                new_generation: None,
                owner_uid: previous.unwrap().owner_uid.clone(),
                payload_digest: old.payload_digest.clone(),
                canonical_resource: None,
                operation_id: operation_id.to_owned(),
                correlation_id: correlation_id.to_owned(),
            },
        ));
    }

    let canonical_json = prepared
        .mutation()
        .canonical_resource
        .clone()
        .ok_or_else(|| integrity("mutation-resource-body-missing"))?;
    let canonical_json = stamp_revision(&canonical_json, revision)?;
    let envelope = ResourceEnvelope::from_json(&canonical_json)
        .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
    let uid = prepared
        .resource_uid()
        .cloned()
        .ok_or_else(|| integrity("mutation-resource-uid-missing"))?;
    if envelope.metadata().uid() != &uid {
        return Err(integrity("mutation-resource-uid-mismatch"));
    }
    if envelope.resource_type() != mutation.target.resource_type()
        || envelope.metadata().name() != mutation.target.name()
        || envelope.metadata().zone() != &mutation.zone
        || envelope.metadata().owner_ref() != mutation.owner.as_ref()
    {
        return Err(integrity("mutation-resource-identity-mismatch"));
    }
    let owner_uid = match &mutation.owner {
        Some(owner_ref) => Some(resolve_uid_in_write(write, owner_ref)?.as_str().to_owned()),
        None => None,
    };
    if let (Some(previous_resource), Some(previous_record)) = (&previous_resource, &previous) {
        remove_indexes(write, previous_resource, previous_record)?;
    }
    let payload_digest = envelope.digest().map_err(integrity)?;
    let record = ResourceRecord {
        canonical_json: canonical_json.clone(),
        owner_uid: owner_uid.clone(),
        controller_binding_id: controller_binding_id(prepared),
        payload_digest: payload_digest.clone(),
    };
    let producer = endpoint_producer(&envelope);
    insert_resource_and_indexes(
        write,
        &mutation.target,
        &uid,
        revision,
        &record,
        producer.as_ref(),
    )?;
    let resource = stored_resource(&mutation.zone, &mutation.target, &record)?;
    let event = match mutation.kind {
        ResourceMutationKind::Create => "created",
        ResourceMutationKind::UpdateSpec => "spec-updated",
        ResourceMutationKind::UpdateStatus => "status-updated",
        ResourceMutationKind::UpdateMetadata | ResourceMutationKind::UpdateFinalizers => {
            "metadata-updated"
        }
        ResourceMutationKind::Delete => unreachable!("delete returned above"),
    };
    Ok((
        resource.clone(),
        ChangeEntry {
            ordinal,
            resource_type: mutation.target.resource_type().as_str().to_owned(),
            resource_name: mutation.target.name().as_str().to_owned(),
            resource_uid: uid.as_str().to_owned(),
            event: event.to_owned(),
            old_generation: previous_resource
                .as_ref()
                .map(|resource| resource.generation.get()),
            new_generation: Some(resource.generation.get()),
            owner_uid,
            payload_digest,
            canonical_resource: Some(canonical_json),
            operation_id: operation_id.to_owned(),
            correlation_id: correlation_id.to_owned(),
        },
    ))
}

fn validate_verified_write(
    write: &redb::WriteTransaction,
    verified: &VerifiedWrite,
    revision: u64,
    accepted_targets: &std::collections::BTreeSet<ResourceRef>,
) -> Result<(), StoreError> {
    let meta = read_meta_in_write(write)?;
    if verified.authorization.zone.as_str() != meta.zone_name {
        return Err(integrity("mutation-zone-mismatch"));
    }
    let mut staged = std::collections::BTreeMap::<ResourceRef, Option<ResourceUid>>::new();
    for (ordinal, prepared) in verified.mutations.iter().enumerate() {
        let mutation = prepared.mutation();
        let ordinal = u32::try_from(ordinal).map_err(integrity)?;
        if accepted_targets.contains(&mutation.target) {
            return Err(conflict(
                meta.current_revision,
                ordinal,
                "group-resource-conflict",
            ));
        }
        if mutation.zone != verified.authorization.zone {
            return Err(integrity("mutation-zone-mismatch"));
        }
        let current = if let Some(uid) = staged.get(&mutation.target) {
            uid.clone().map(|uid| (uid, revision))
        } else {
            current_identity_in_write(write, &mutation.target)?
        };
        if !authorization_matches(&verified.authorization, mutation) {
            return Err(authorization_denied(meta.current_revision));
        }
        match mutation.expected {
            ExpectedRevision::CreateAbsent if current.is_some() => {
                return Err(error(
                    StoreErrorKind::ResourceAlreadyExists,
                    current.map(|(_, revision)| ZoneRevision::new(revision)),
                    "resource-already-exists",
                ));
            }
            ExpectedRevision::Exact(expected)
                if current
                    .as_ref()
                    .is_none_or(|(_, current_revision)| *current_revision != expected.get()) =>
            {
                return Err(conflict(
                    current.as_ref().map_or(0, |(_, revision)| *revision),
                    ordinal,
                    "resource-revision-changed",
                ));
            }
            ExpectedRevision::CreateAbsent | ExpectedRevision::Exact(_) => {}
        }
        if mutation.expected_uid.as_ref().is_some_and(|expected| {
            current
                .as_ref()
                .is_none_or(|(current_uid, _)| current_uid != expected)
        }) {
            return Err(conflict(
                current.as_ref().map_or(0, |(_, revision)| *revision),
                ordinal,
                "resource-uid-changed",
            ));
        }

        if mutation.kind == ResourceMutationKind::Delete {
            if current.is_none() {
                return Err(error(
                    StoreErrorKind::ResourceNotFound,
                    None,
                    "resource-not-found",
                ));
            }
            staged.insert(mutation.target.clone(), None);
            continue;
        }

        let bytes = mutation
            .canonical_resource
            .as_deref()
            .ok_or_else(|| integrity("mutation-resource-body-missing"))?;
        let envelope = ResourceEnvelope::from_json(bytes)
            .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
        let uid = prepared
            .resource_uid()
            .cloned()
            .ok_or_else(|| integrity("mutation-resource-uid-missing"))?;
        if envelope.metadata().uid() != &uid
            || envelope.resource_type() != mutation.target.resource_type()
            || envelope.metadata().name() != mutation.target.name()
            || envelope.metadata().zone() != &mutation.zone
            || envelope.metadata().owner_ref() != mutation.owner.as_ref()
        {
            return Err(integrity("mutation-resource-identity-mismatch"));
        }
        if mutation
            .expected_uid
            .as_ref()
            .is_some_and(|expected| expected != &uid)
            || (mutation.kind != ResourceMutationKind::Create
                && current
                    .as_ref()
                    .is_some_and(|(current_uid, _)| current_uid != &uid))
        {
            return Err(conflict(
                current.as_ref().map_or(0, |(_, revision)| *revision),
                ordinal,
                "resource-uid-changed",
            ));
        }
        if let Some(owner_ref) = &mutation.owner {
            let owner_uid = if let Some(owner) = staged.get(owner_ref) {
                owner.clone()
            } else {
                current_identity_in_write(write, owner_ref)?.map(|(uid, _)| uid)
            };
            if owner_uid.is_none() {
                return Err(error(
                    StoreErrorKind::ResourceRefInvalid,
                    None,
                    "owner-ref-not-found",
                ));
            }
            if owner_uid.as_ref() == Some(&uid)
                || owner_chain_reaches(write, &staged, owner_ref, &uid)?
            {
                return Err(error(
                    StoreErrorKind::ResourceOwnerCycle,
                    None,
                    "resource-owner-cycle",
                ));
            }
        }
        staged.insert(mutation.target.clone(), Some(uid));
    }
    Ok(())
}

fn authorization_matches(authorization: &AdmittedAuthorization, mutation: &StoreMutation) -> bool {
    let verb = match mutation.kind {
        ResourceMutationKind::Create => d2b_resource_store::AdmittedVerb::Create,
        ResourceMutationKind::UpdateSpec => d2b_resource_store::AdmittedVerb::UpdateSpec,
        ResourceMutationKind::UpdateStatus => d2b_resource_store::AdmittedVerb::UpdateStatus,
        ResourceMutationKind::UpdateMetadata => d2b_resource_store::AdmittedVerb::UpdateMetadata,
        ResourceMutationKind::UpdateFinalizers => {
            d2b_resource_store::AdmittedVerb::UpdateFinalizers
        }
        ResourceMutationKind::Delete => d2b_resource_store::AdmittedVerb::Delete,
    };
    authorization.targets.iter().any(|target| {
        target.resource_type == *mutation.target.resource_type()
            && target
                .resource_name
                .as_ref()
                .is_none_or(|name| name == mutation.target.name())
            && target.verb == verb
    })
}

fn owner_chain_reaches(
    write: &redb::WriteTransaction,
    staged: &std::collections::BTreeMap<ResourceRef, Option<ResourceUid>>,
    owner_ref: &ResourceRef,
    child_uid: &ResourceUid,
) -> Result<bool, StoreError> {
    let mut current = Some(owner_ref.clone());
    let mut depth = 0_usize;
    let mut visited = std::collections::BTreeSet::new();
    while let Some(resource_ref) = current {
        depth += 1;
        if depth > crate::MAX_OWNER_CHAIN_DEPTH {
            return Err(error(
                StoreErrorKind::ResourceOwnerDepth,
                None,
                "resource-owner-depth",
            ));
        }
        let uid = if let Some(staged_uid) = staged.get(&resource_ref) {
            staged_uid.clone()
        } else {
            current_identity_in_write(write, &resource_ref)?.map(|(uid, _)| uid)
        };
        let Some(uid) = uid else {
            return Ok(false);
        };
        if &uid == child_uid || !visited.insert(uid) {
            return Ok(true);
        }
        current = current_owner_ref_in_write(write, &resource_ref)?;
    }
    Ok(false)
}

fn current_owner_ref_in_write(
    write: &redb::WriteTransaction,
    resource_ref: &ResourceRef,
) -> Result<Option<ResourceRef>, StoreError> {
    let table = write.open_table(RESOURCES).map_err(integrity)?;
    let key = resource_key(resource_ref)?;
    table
        .get(key.as_slice())
        .map_err(integrity)?
        .map(|bytes| {
            let record: ResourceRecord = decode(ValueKind::ResourceRecord, bytes.value())?;
            let envelope = ResourceEnvelope::from_json(&record.canonical_json)
                .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
            Ok(envelope.metadata().owner_ref().cloned())
        })
        .transpose()
        .map(Option::flatten)
}

fn current_identity_in_write(
    write: &redb::WriteTransaction,
    resource_ref: &ResourceRef,
) -> Result<Option<(ResourceUid, u64)>, StoreError> {
    let table = write.open_table(RESOURCES).map_err(integrity)?;
    let key = resource_key(resource_ref)?;
    table
        .get(key.as_slice())
        .map_err(integrity)?
        .map(|bytes| {
            let record: ResourceRecord = decode(ValueKind::ResourceRecord, bytes.value())?;
            let envelope = ResourceEnvelope::from_json(&record.canonical_json)
                .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
            Ok((
                envelope.metadata().uid().clone(),
                envelope.metadata().revision().get(),
            ))
        })
        .transpose()
}

fn revisions_match(meta: &StoreMeta, snapshot: PolicySnapshot) -> bool {
    meta.policy_revision == snapshot.policy_revision
        && meta.api_catalog_revision == snapshot.api_catalog_revision
        && meta.active_configuration_revision == snapshot.active_configuration_revision.get()
        && meta.controller_generation
            == snapshot
                .controller_generation
                .map(ControllerGeneration::get)
}

fn read_meta_in_write(write: &redb::WriteTransaction) -> Result<StoreMeta, StoreError> {
    let table = write.open_table(STORE_META).map_err(integrity)?;
    let bytes = table
        .get(meta_key().as_slice())
        .map_err(integrity)?
        .ok_or_else(|| integrity("store-meta-missing"))?;
    decode(ValueKind::StoreMetaScalar, bytes.value())
}

fn resolve_uid_in_write(
    write: &redb::WriteTransaction,
    resource_ref: &ResourceRef,
) -> Result<ResourceUid, StoreError> {
    let table = write.open_table(TYPE_INDEX).map_err(integrity)?;
    let key = type_index_key(resource_ref)?;
    let bytes = table
        .get(key.as_slice())
        .map_err(integrity)?
        .ok_or_else(|| {
            error(
                StoreErrorKind::ResourceRefInvalid,
                None,
                "owner-ref-not-found",
            )
        })?;
    let uid: String = decode(ValueKind::TypeIndexRecord, bytes.value())?;
    ResourceUid::parse(uid).map_err(|_| integrity("type-index-uid-invalid"))
}

fn insert_resource_and_indexes(
    write: &redb::WriteTransaction,
    resource_ref: &ResourceRef,
    uid: &ResourceUid,
    revision: u64,
    record: &ResourceRecord,
    producer: Option<&ResourceRef>,
) -> Result<(), StoreError> {
    let resource_key = resource_key(resource_ref)?;
    let resource_value = encode(ValueKind::ResourceRecord, record)?;
    write
        .open_table(RESOURCES)
        .map_err(integrity)?
        .insert(resource_key.as_slice(), resource_value.as_slice())
        .map_err(integrity)?;
    let type_key = type_index_key(resource_ref)?;
    let type_value = encode(ValueKind::TypeIndexRecord, &uid.as_str())?;
    write
        .open_table(TYPE_INDEX)
        .map_err(integrity)?
        .insert(type_key.as_slice(), type_value.as_slice())
        .map_err(integrity)?;
    if let Some(owner_uid) = &record.owner_uid {
        let owner_key = encode_key(
            KeySpace::OwnerIndex,
            &[
                KeyComponent::Text(owner_uid),
                KeyComponent::Text(uid.as_str()),
            ],
        )
        .map_err(integrity)?;
        let owner_value = encode(
            ValueKind::OwnerIndexRecord,
            &OwnerIndexRecord {
                resource_type: resource_ref.resource_type().as_str().to_owned(),
                resource_name: resource_ref.name().as_str().to_owned(),
                latest_revision: revision,
            },
        )?;
        write
            .open_table(OWNER_INDEX)
            .map_err(integrity)?
            .insert(owner_key.as_bytes(), owner_value.as_slice())
            .map_err(integrity)?;
    }
    if let Some(producer_ref) = producer {
        let producer_uid = resolve_uid_in_write(write, producer_ref)?;
        let producer_key = encode_key(
            KeySpace::ProducerIndex,
            &[
                KeyComponent::Text(producer_uid.as_str()),
                KeyComponent::Text(uid.as_str()),
            ],
        )
        .map_err(integrity)?;
        let producer_value = encode(
            ValueKind::ProducerIndexRecord,
            &ProducerIndexRecord {
                endpoint_type: resource_ref.resource_type().as_str().to_owned(),
                endpoint_name: resource_ref.name().as_str().to_owned(),
            },
        )?;
        write
            .open_table(PRODUCER_INDEX)
            .map_err(integrity)?
            .insert(producer_key.as_bytes(), producer_value.as_slice())
            .map_err(integrity)?;
    }
    let controller_key = encode_key(
        KeySpace::ControllerIndex,
        &[
            KeyComponent::Text(&record.controller_binding_id),
            KeyComponent::Text(resource_ref.resource_type().as_str()),
            KeyComponent::Text(resource_ref.name().as_str()),
        ],
    )
    .map_err(integrity)?;
    let controller_value = encode(ValueKind::ControllerIndexRecord, &uid.as_str())?;
    write
        .open_table(CONTROLLER_INDEX)
        .map_err(integrity)?
        .insert(controller_key.as_bytes(), controller_value.as_slice())
        .map_err(integrity)?;
    Ok(())
}

fn remove_indexes(
    write: &redb::WriteTransaction,
    resource: &StoredResource,
    record: &ResourceRecord,
) -> Result<(), StoreError> {
    write
        .open_table(TYPE_INDEX)
        .map_err(integrity)?
        .remove(type_index_key(&resource.resource_ref)?.as_slice())
        .map_err(integrity)?;
    if let Some(owner_uid) = &record.owner_uid {
        let key = encode_key(
            KeySpace::OwnerIndex,
            &[
                KeyComponent::Text(owner_uid),
                KeyComponent::Text(resource.uid.as_str()),
            ],
        )
        .map_err(integrity)?;
        write
            .open_table(OWNER_INDEX)
            .map_err(integrity)?
            .remove(key.as_bytes())
            .map_err(integrity)?;
    }
    remove_producer_index_entries(write, &resource.uid)?;
    let controller_key = encode_key(
        KeySpace::ControllerIndex,
        &[
            KeyComponent::Text(&record.controller_binding_id),
            KeyComponent::Text(resource.resource_ref.resource_type().as_str()),
            KeyComponent::Text(resource.resource_ref.name().as_str()),
        ],
    )
    .map_err(integrity)?;
    write
        .open_table(CONTROLLER_INDEX)
        .map_err(integrity)?
        .remove(controller_key.as_bytes())
        .map_err(integrity)?;
    Ok(())
}

fn remove_producer_index_entries(
    write: &redb::WriteTransaction,
    endpoint_uid: &ResourceUid,
) -> Result<(), StoreError> {
    let mut table = write.open_table(PRODUCER_INDEX).map_err(integrity)?;
    let keys = table
        .iter()
        .map_err(integrity)?
        .filter_map(|row| match row {
            Ok((key, _)) => match DecodedKey::decode(key.value()) {
                Ok(decoded)
                    if matches!(
                        decoded.components(),
                        [
                            crate::DecodedKeyComponent::Text(_),
                            crate::DecodedKeyComponent::Text(uid)
                        ] if uid == endpoint_uid.as_str()
                    ) =>
                {
                    Some(Ok(key.value().to_vec()))
                }
                Ok(_) => None,
                Err(error) => Some(Err(integrity(error))),
            },
            Err(error) => Some(Err(integrity(error))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    for key in keys {
        table.remove(key.as_slice()).map_err(integrity)?;
    }
    Ok(())
}

fn endpoint_producer(envelope: &ResourceEnvelope) -> Option<ResourceRef> {
    if envelope.resource_type().as_str() != "Endpoint" {
        return None;
    }
    match envelope.spec().base().get("producerRef") {
        Some(CanonicalJsonValue::String(reference)) => ResourceRef::parse(reference).ok(),
        _ => None,
    }
}

pub(crate) fn stored_resource(
    zone: &ZoneId,
    resource_ref: &ResourceRef,
    record: &ResourceRecord,
) -> Result<StoredResource, StoreError> {
    let envelope = ResourceEnvelope::from_json(&record.canonical_json)
        .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
    Ok(StoredResource {
        resource_ref: resource_ref.clone(),
        zone: zone.clone(),
        uid: envelope.metadata().uid().clone(),
        generation: envelope.metadata().generation(),
        revision: envelope.metadata().revision(),
        canonical_json: record.canonical_json.clone(),
        payload_digest: record.payload_digest.clone(),
    })
}

fn stamp_revision(bytes: &[u8], revision: u64) -> Result<Vec<u8>, StoreError> {
    let mut value = CanonicalJsonValue::parse(bytes)
        .map_err(|_| integrity("mutation-resource-envelope-invalid"))?;
    let CanonicalJsonValue::Object(root) = &mut value else {
        return Err(integrity("mutation-resource-envelope-invalid"));
    };
    let Some(CanonicalJsonValue::Object(metadata)) = root.get_mut("metadata") else {
        return Err(integrity("mutation-resource-metadata-missing"));
    };
    let revision = i64::try_from(revision).map_err(|_| integrity("zone-revision-out-of-range"))?;
    metadata.insert("revision".to_owned(), CanonicalJsonValue::Integer(revision));
    Ok(value.to_canonical_bytes())
}

fn controller_binding_id(prepared: &VerifiedPreparedMutation) -> String {
    let Some(bytes) = prepared.mutation().canonical_resource.as_deref() else {
        return prepared
            .mutation()
            .target
            .resource_type()
            .as_str()
            .to_owned();
    };
    ResourceEnvelope::from_json(bytes)
        .ok()
        .and_then(|envelope| envelope.spec().provider_ref().cloned())
        .map_or_else(
            || {
                prepared
                    .mutation()
                    .target
                    .resource_type()
                    .as_str()
                    .to_owned()
            },
            |provider| provider.to_canonical_string(),
        )
}

fn operation_digest(verified: &VerifiedWrite) -> String {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    digest.update(verified.operation.operation_id.as_bytes());
    if let Some(idempotency_key) = &verified.operation.idempotency_key {
        digest.update(idempotency_key.as_bytes());
    }
    digest.update(verified.operation.correlation_id.as_bytes());
    digest.update(verified.authorization.subject_uid.as_str().as_bytes());
    for mutation in &verified.mutations {
        digest.update(mutation.mutation().target.to_canonical_string().as_bytes());
        digest.update([mutation_kind_discriminant(mutation.mutation().kind)]);
        match mutation.mutation().expected {
            ExpectedRevision::CreateAbsent => digest.update([0]),
            ExpectedRevision::Exact(revision) => {
                digest.update([1]);
                digest.update(revision.get().to_be_bytes());
            }
        }
        if let Some(payload_digest) = mutation.payload_digest() {
            digest.update(payload_digest.as_bytes());
        }
    }
    format!("sha256:{:x}", digest.finalize())
}

const fn mutation_kind_discriminant(kind: ResourceMutationKind) -> u8 {
    match kind {
        ResourceMutationKind::Create => 0,
        ResourceMutationKind::UpdateSpec => 1,
        ResourceMutationKind::UpdateStatus => 2,
        ResourceMutationKind::UpdateMetadata => 3,
        ResourceMutationKind::UpdateFinalizers => 4,
        ResourceMutationKind::Delete => 5,
    }
}

pub(crate) fn resource_key(resource_ref: &ResourceRef) -> Result<Vec<u8>, StoreError> {
    encode_key(
        KeySpace::Resources,
        &[
            KeyComponent::Text(resource_ref.resource_type().as_str()),
            KeyComponent::Text(resource_ref.name().as_str()),
        ],
    )
    .map(|key| key.into_bytes())
    .map_err(integrity)
}

pub(crate) fn type_index_key(resource_ref: &ResourceRef) -> Result<Vec<u8>, StoreError> {
    encode_key(
        KeySpace::TypeIndex,
        &[
            KeyComponent::Text(resource_ref.resource_type().as_str()),
            KeyComponent::Text(resource_ref.name().as_str()),
        ],
    )
    .map(|key| key.into_bytes())
    .map_err(integrity)
}

pub(crate) fn revision_key(revision: u64) -> Result<Vec<u8>, StoreError> {
    encode_key(KeySpace::RevisionLog, &[KeyComponent::U64(revision)])
        .map(|key| key.into_bytes())
        .map_err(integrity)
}

fn operation_key(operation_id: &str) -> Result<Vec<u8>, StoreError> {
    encode_key(KeySpace::Operations, &[KeyComponent::Text(operation_id)])
        .map(|key| key.into_bytes())
        .map_err(integrity)
}

fn meta_key() -> Vec<u8> {
    encode_key(KeySpace::StoreMeta, &[KeyComponent::Text("store")])
        .expect("the fixed store-meta key is valid")
        .into_bytes()
}

pub(crate) fn encode<T: Serialize>(kind: ValueKind, value: &T) -> Result<Vec<u8>, StoreError> {
    let json = d2b_contracts::v3::canonical_json_bytes(value).map_err(integrity)?;
    encode_value(kind, &json)
        .map(|value| value.into_bytes())
        .map_err(integrity)
}

pub(crate) fn decode<T>(kind: ValueKind, bytes: &[u8]) -> Result<T, StoreError>
where
    T: for<'de> Deserialize<'de>,
{
    let decoded = crate::DecodedValue::decode(bytes).map_err(integrity)?;
    if decoded.kind() != kind {
        return Err(integrity("table-value-kind-mismatch"));
    }
    serde_json::from_slice(decoded.canonical_json()).map_err(integrity)
}

pub(crate) fn integrity(_detail: impl core::fmt::Display) -> StoreError {
    error(
        StoreErrorKind::StoreIntegrityFailure,
        None,
        "redb-store-integrity-failure",
    )
}

fn set_full_durability(write: &mut redb::WriteTransaction) -> Result<(), StoreError> {
    write
        .set_durability(Durability::Immediate)
        .map_err(integrity)
}

pub(crate) fn backpressure() -> StoreError {
    error(
        StoreErrorKind::StoreBackpressure,
        None,
        "redb-store-backpressure",
    )
}

pub(crate) fn timeout() -> StoreError {
    error(StoreErrorKind::Timeout, None, "redb-read-lifetime-exceeded")
}

pub(crate) fn revision_expired(current_revision: u64) -> StoreError {
    error(
        StoreErrorKind::RevisionExpired,
        Some(ZoneRevision::new(current_revision)),
        "redb-revision-expired",
    )
}

fn authorization_denied(current_revision: u64) -> StoreError {
    StoreError::new(
        StoreErrorKind::AuthorizationDenied,
        Some(ZoneRevision::new(current_revision)),
        None,
        RetryClass::Reauthorize,
        "store-generation-recheck-failed",
    )
}

fn conflict(current_revision: u64, ordinal: u32, reason: &'static str) -> StoreError {
    StoreError::batch_conflict(
        ZoneRevision::new(current_revision),
        MutationOrdinal::new(ordinal)
            .unwrap_or_else(|_| MutationOrdinal::new(0).expect("zero is a valid mutation ordinal")),
        RetryClass::Reauthorize,
        reason,
    )
}

fn error(
    kind: StoreErrorKind,
    current_revision: Option<ZoneRevision>,
    reason: &'static str,
) -> StoreError {
    StoreError::new(kind, current_revision, None, RetryClass::Never, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::{ConfigurationGeneration, ResourceTypeName, Timestamp};
    use d2b_resource_store::{AdmittedAuthorizationTarget, AdmittedVerb, ResourceMutationKind};
    use redb::ReadableTableMetadata;
    use std::fs::OpenOptions;

    const RESOURCE: &[u8] = br#"{"apiVersion":"resources.d2bus.org/v3","metadata":{"configurationGeneration":7,"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"configuration","name":"host-system","ownerRef":null,"revision":1,"uid":"123e4567-e89b-42d3-a456-426614174000","updatedAt":"2026-07-22T00:00:00.000Z","zone":"dev"},"spec":{"providerRef":"Provider/system-core","updatePolicy":{"disruptive":"manual","nonDisruptive":"automatic"}},"status":{"completedAt":null,"conditions":[],"lastReconciledAt":null,"observedGeneration":0,"outcome":null,"phase":"Pending","resource":{},"startedAt":null,"update":{"dependencies":{"count":0,"refs":[]},"disruption":"None","lastAssessedAt":null,"observedGeneration":0,"operationId":null,"owned":{"count":0,"refs":[]},"preserveState":true,"reasons":[],"state":"Unknown","targetGeneration":1}},"type":"Host"}"#;

    fn fixture() -> (tempfile::TempDir, Database, crate::StoreIdentity) {
        let directory = tempfile::tempdir().unwrap();
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(directory.path().join("store.redb"))
            .unwrap();
        let database = Database::builder().create_file(file).unwrap();
        let identity = crate::StoreIdentity::new(
            ResourceUid::parse("11111111-1111-4111-8111-111111111111").unwrap(),
            ZoneId::parse("dev").unwrap(),
            ResourceUid::parse("22222222-2222-4222-8222-222222222222").unwrap(),
            Timestamp::parse("2026-07-31T00:00:00.000Z").unwrap(),
            PolicySnapshot {
                policy_revision: 7,
                api_catalog_revision: 8,
                active_configuration_revision: ConfigurationGeneration::new(9).unwrap(),
                controller_generation: None,
            },
        );
        initialize(&database, &identity).unwrap();
        (directory, database, identity)
    }

    fn verified(operation_id: &str, mutation: StoreMutation, uid: ResourceUid) -> VerifiedWrite {
        let verb = match mutation.kind {
            ResourceMutationKind::Create => AdmittedVerb::Create,
            ResourceMutationKind::UpdateSpec => AdmittedVerb::UpdateSpec,
            ResourceMutationKind::UpdateStatus => AdmittedVerb::UpdateStatus,
            ResourceMutationKind::UpdateMetadata => AdmittedVerb::UpdateMetadata,
            ResourceMutationKind::UpdateFinalizers => AdmittedVerb::UpdateFinalizers,
            ResourceMutationKind::Delete => AdmittedVerb::Delete,
        };
        VerifiedWrite {
            authorization: AdmittedAuthorization {
                zone: ZoneId::parse("dev").unwrap(),
                subject_ref: ResourceRef::parse("Provider/system-core").unwrap(),
                subject_uid: ResourceUid::parse("33333333-3333-4333-8333-333333333333").unwrap(),
                targets: vec![AdmittedAuthorizationTarget {
                    resource_type: ResourceTypeName::parse("Host").unwrap(),
                    resource_name: Some(mutation.target.name().clone()),
                    verb,
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
            operation: StoreOperationContext {
                operation_id: operation_id.to_owned(),
                idempotency_key: None,
                correlation_id: format!("corr-{operation_id}"),
                trace_id: None,
                deadline_ms: 1_000,
            },
            mutations: vec![VerifiedPreparedMutation {
                mutation,
                resource_uid: Some(uid),
                payload_digest: Some("sha256:prepared".to_owned()),
            }],
        }
    }

    fn create_mutation(target: ResourceRef) -> StoreMutation {
        StoreMutation {
            kind: ResourceMutationKind::Create,
            zone: ZoneId::parse("dev").unwrap(),
            target,
            expected: ExpectedRevision::CreateAbsent,
            expected_uid: None,
            owner: None,
            canonical_resource: Some(RESOURCE.to_vec()),
            add_finalizers: Vec::new(),
            remove_finalizers: Vec::new(),
            wait_for_reconcile: false,
            reconcile_deadline_ms: None,
        }
    }

    #[test]
    fn verified_write_atomically_updates_resource_indexes_revision_and_operation() {
        let (_directory, database, identity) = fixture();
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let outcome = apply_group(
            &database,
            vec![verified(
                "create-host",
                create_mutation(target.clone()),
                uid,
            )],
        )
        .unwrap();
        let result = outcome.results[0].as_ref().unwrap();
        assert_eq!(result.revision, ZoneRevision::new(1));
        assert_eq!(result.resources.len(), 1);
        assert_eq!(result.resources[0].revision, ZoneRevision::new(1));
        assert_eq!(outcome.batch.as_ref().unwrap().revision, 1);

        let read = database.begin_read().unwrap();
        assert_eq!(read.open_table(RESOURCES).unwrap().len().unwrap(), 1);
        assert_eq!(read.open_table(TYPE_INDEX).unwrap().len().unwrap(), 1);
        assert_eq!(read.open_table(CONTROLLER_INDEX).unwrap().len().unwrap(), 1);
        assert_eq!(read.open_table(REVISION_LOG).unwrap().len().unwrap(), 1);
        assert_eq!(read.open_table(OPERATIONS).unwrap().len().unwrap(), 1);
        drop(read);
        assert_eq!(current_meta(&database).unwrap().current_revision, 1);
        assert_eq!(
            validate_identity(&database, &identity).unwrap().zone_name,
            "dev"
        );
    }

    #[test]
    fn conflicting_create_cannot_mutate_any_table_or_allocate_a_revision() {
        let (_directory, database, _identity) = fixture();
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        apply_group(
            &database,
            vec![verified(
                "create-host",
                create_mutation(target.clone()),
                uid.clone(),
            )],
        )
        .unwrap();
        let outcome = apply_group(
            &database,
            vec![verified("conflict", create_mutation(target), uid)],
        )
        .unwrap();
        assert_eq!(
            outcome.results[0].as_ref().unwrap_err().kind(),
            StoreErrorKind::ResourceAlreadyExists
        );
        assert!(outcome.batch.is_none());
        assert_eq!(current_meta(&database).unwrap().current_revision, 1);
        let read = database.begin_read().unwrap();
        assert_eq!(read.open_table(RESOURCES).unwrap().len().unwrap(), 1);
        assert_eq!(read.open_table(REVISION_LOG).unwrap().len().unwrap(), 1);
        assert_eq!(read.open_table(OPERATIONS).unwrap().len().unwrap(), 1);
    }

    #[test]
    fn generation_recheck_failure_happens_inside_the_write_transaction() {
        let (_directory, database, _identity) = fixture();
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let mut request = verified("stale-policy", create_mutation(target), uid);
        request.policy_snapshot.policy_revision = 99;
        let outcome = apply_group(&database, vec![request]).unwrap();
        assert_eq!(
            outcome.results[0].as_ref().unwrap_err().kind(),
            StoreErrorKind::AuthorizationDenied
        );
        assert_eq!(current_meta(&database).unwrap().current_revision, 0);
        let read = database.begin_read().unwrap();
        assert_eq!(read.open_table(RESOURCES).unwrap().len().unwrap(), 0);
        assert_eq!(read.open_table(OPERATIONS).unwrap().len().unwrap(), 0);
        assert_eq!(read.open_table(REVISION_LOG).unwrap().len().unwrap(), 0);
    }

    #[test]
    fn controller_generation_recheck_is_part_of_the_same_write_transaction() {
        let (_directory, database, _identity) = fixture();
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let mut request = verified("stale-controller", create_mutation(target), uid);
        request.policy_snapshot.controller_generation = Some(ControllerGeneration::new(2).unwrap());
        let outcome = apply_group(&database, vec![request]).unwrap();
        assert_eq!(
            outcome.results[0].as_ref().unwrap_err().kind(),
            StoreErrorKind::AuthorizationDenied
        );
        assert_eq!(current_meta(&database).unwrap().current_revision, 0);
    }

    #[test]
    fn failed_request_does_not_abort_an_independent_request_in_the_group() {
        let (_directory, database, _identity) = fixture();
        let first_target = ResourceRef::parse("Host/host-system").unwrap();
        let first_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        apply_group(
            &database,
            vec![verified(
                "seed-host",
                create_mutation(first_target.clone()),
                first_uid.clone(),
            )],
        )
        .unwrap();

        let second_target = ResourceRef::parse("Host/host-backup").unwrap();
        let second_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap();
        let mut second_mutation = create_mutation(second_target.clone());
        second_mutation.canonical_resource = Some(
            String::from_utf8(RESOURCE.to_vec())
                .unwrap()
                .replace("host-system", "host-backup")
                .replace(first_uid.as_str(), second_uid.as_str())
                .into_bytes(),
        );
        let outcome = apply_group(
            &database,
            vec![
                verified("independent-success", second_mutation, second_uid.clone()),
                verified(
                    "expected-conflict",
                    create_mutation(first_target),
                    first_uid,
                ),
            ],
        )
        .unwrap();
        assert_eq!(outcome.results[0].as_ref().unwrap().revision.get(), 2);
        assert_eq!(
            outcome.results[1].as_ref().unwrap_err().kind(),
            StoreErrorKind::ResourceAlreadyExists
        );
        assert_eq!(current_meta(&database).unwrap().current_revision, 2);
        let read = database.begin_read().unwrap();
        assert_eq!(read.open_table(RESOURCES).unwrap().len().unwrap(), 2);
        assert_eq!(read.open_table(OPERATIONS).unwrap().len().unwrap(), 2);
        assert_eq!(read.open_table(REVISION_LOG).unwrap().len().unwrap(), 2);
        let stored = read
            .open_table(RESOURCES)
            .unwrap()
            .get(resource_key(&second_target).unwrap().as_slice())
            .unwrap();
        assert!(stored.is_some());
    }

    #[test]
    fn expected_uid_mismatch_cannot_replace_an_existing_resource() {
        let (_directory, database, _identity) = fixture();
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let created = apply_group(
            &database,
            vec![verified(
                "seed-host",
                create_mutation(target.clone()),
                uid.clone(),
            )],
        )
        .unwrap();
        let mut update = create_mutation(target);
        update.kind = ResourceMutationKind::UpdateSpec;
        update.expected = ExpectedRevision::Exact(ZoneRevision::new(1));
        update.expected_uid =
            Some(ResourceUid::parse("123e4567-e89b-42d3-a456-426614174099").unwrap());
        let outcome = apply_group(&database, vec![verified("wrong-uid", update, uid)]).unwrap();
        assert_eq!(
            outcome.results[0].as_ref().unwrap_err().kind(),
            StoreErrorKind::ResourceConflict
        );
        assert_eq!(current_meta(&database).unwrap().current_revision, 1);
        assert_eq!(created.results[0].as_ref().unwrap().revision.get(), 1);
    }

    #[test]
    fn idempotent_replay_returns_the_original_committed_resources() {
        let (_directory, database, _identity) = fixture();
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let first = apply_group(
            &database,
            vec![verified(
                "idempotent-create",
                create_mutation(target.clone()),
                uid.clone(),
            )],
        )
        .unwrap();
        let replay = apply_group(
            &database,
            vec![verified("idempotent-create", create_mutation(target), uid)],
        )
        .unwrap();
        assert!(replay.batch.is_none());
        assert_eq!(replay.results[0], first.results[0]);
        assert_eq!(current_meta(&database).unwrap().current_revision, 1);
    }
}
