//! Persisted store DTOs, recovery validation, and crash-safe write transactions.

use d2b_contracts::v3::{
    CanonicalJsonValue, ControllerGeneration, FinalizerId, RESOURCE_ENVELOPE_DOMAIN_TAG,
    ResourceEnvelope, ResourceGeneration, ResourceName, ResourceRef, ResourceTypeName, ResourceUid,
    RetryClass, Timestamp, ZoneId, ZoneRevision, canonical_digest,
};
use d2b_resource_store::{
    AdmittedAuthorization, ExpectedRevision, MutationOrdinal, PolicySnapshot, ResourceMutationKind,
    StoreCommitResult, StoreError, StoreErrorKind, StoreMutation, StoreOperationContext,
    StoredResource,
};
use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::{DecodedKey, KeyComponent, KeySpace, ValueKind, encode_key, encode_value};
use d2b_resource_store::mutation_seal::OpenedMutation;

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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub(crate) struct ResourceRecord {
    pub canonical_json: Vec<u8>,
    pub owner_uid: Option<String>,
    pub controller_binding_id: String,
    pub payload_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwnerIndexRecord {
    pub resource_type: String,
    pub resource_name: String,
    pub latest_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProducerIndexRecord {
    pub endpoint_type: String,
    pub endpoint_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperationRecord {
    pub request_digest: String,
    pub resource_uids: Vec<String>,
    pub resources: Vec<OperationResourceRecord>,
    pub outcome: String,
    pub accepted_revision: u64,
    pub finished_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperationResourceRecord {
    pub resource_type: String,
    pub resource_name: String,
    pub zone: String,
    pub canonical_json: Vec<u8>,
    pub payload_digest: String,
}

/// Closed resource mutation event persisted in the revision log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChangeEvent {
    Created,
    SpecUpdated,
    StatusUpdated,
    MetadataUpdated,
    DeletionRequested,
    Deleted,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct ChangeIdentity(String);

impl ChangeIdentity {
    fn parse(value: impl Into<String>) -> Result<Self, StoreError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 512
            || value.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(integrity("change-identity-invalid"));
        }
        Ok(Self(value))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for ChangeIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ChangeIdentity(<redacted>)")
    }
}

/// One validated entry in a bounded revision batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeEntry {
    ordinal: u32,
    resource_type: ResourceTypeName,
    resource_name: ResourceName,
    resource_uid: ResourceUid,
    event: ChangeEvent,
    old_generation: Option<ResourceGeneration>,
    new_generation: Option<ResourceGeneration>,
    owner_uid: Option<ResourceUid>,
    payload_digest: String,
    canonical_resource: Option<Vec<u8>>,
    operation_id: ChangeIdentity,
    correlation_id: ChangeIdentity,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeEntryWire {
    ordinal: u32,
    resource_type: ResourceTypeName,
    resource_name: ResourceName,
    resource_uid: ResourceUid,
    event: ChangeEvent,
    old_generation: Option<ResourceGeneration>,
    new_generation: Option<ResourceGeneration>,
    owner_uid: Option<ResourceUid>,
    payload_digest: String,
    canonical_resource: Option<Vec<u8>>,
    operation_id: ChangeIdentity,
    correlation_id: ChangeIdentity,
}

impl<'de> Deserialize<'de> for ChangeEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ChangeEntryWire::deserialize(deserializer)?;
        Self::new(
            wire.ordinal,
            wire.resource_type,
            wire.resource_name,
            wire.resource_uid,
            wire.event,
            wire.old_generation,
            wire.new_generation,
            wire.owner_uid,
            wire.payload_digest,
            wire.canonical_resource,
            wire.operation_id.0,
            wire.correlation_id.0,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ChangeEntry {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        ordinal: u32,
        resource_type: ResourceTypeName,
        resource_name: ResourceName,
        resource_uid: ResourceUid,
        event: ChangeEvent,
        old_generation: Option<ResourceGeneration>,
        new_generation: Option<ResourceGeneration>,
        owner_uid: Option<ResourceUid>,
        payload_digest: String,
        canonical_resource: Option<Vec<u8>>,
        operation_id: String,
        correlation_id: String,
    ) -> Result<Self, StoreError> {
        if usize::try_from(ordinal).map_or(true, |ordinal| {
            ordinal >= crate::actor::GROUP_COMMIT_MAX * d2b_contracts::v3::MAX_BATCH_MUTATIONS
        }) || !valid_digest(&payload_digest)
        {
            return Err(integrity("change-entry-invalid"));
        }
        Ok(Self {
            ordinal,
            resource_type,
            resource_name,
            resource_uid,
            event,
            old_generation,
            new_generation,
            owner_uid,
            payload_digest,
            canonical_resource,
            operation_id: ChangeIdentity::parse(operation_id)?,
            correlation_id: ChangeIdentity::parse(correlation_id)?,
        })
    }

    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    pub const fn resource_type(&self) -> &ResourceTypeName {
        &self.resource_type
    }

    pub const fn resource_name(&self) -> &ResourceName {
        &self.resource_name
    }

    pub const fn event(&self) -> ChangeEvent {
        self.event
    }
}

/// One validated, nonempty revision batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeBatch {
    revision: ZoneRevision,
    entries: Vec<ChangeEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeBatchWire {
    revision: ZoneRevision,
    entries: Vec<ChangeEntry>,
}

impl<'de> Deserialize<'de> for ChangeBatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ChangeBatchWire::deserialize(deserializer)?;
        Self::new(wire.revision, wire.entries).map_err(serde::de::Error::custom)
    }
}

impl ChangeBatch {
    pub(crate) fn new(
        revision: ZoneRevision,
        entries: Vec<ChangeEntry>,
    ) -> Result<Self, StoreError> {
        let max = crate::actor::GROUP_COMMIT_MAX * d2b_contracts::v3::MAX_BATCH_MUTATIONS;
        if revision.get() == 0
            || entries.len() > max
            || entries
                .iter()
                .enumerate()
                .any(|(ordinal, entry)| entry.ordinal as usize != ordinal)
        {
            return Err(integrity("change-batch-invalid"));
        }
        Ok(Self { revision, entries })
    }

    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    pub fn entries(&self) -> &[ChangeEntry] {
        &self.entries
    }
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
    prepared_payload_digest: Option<String>,
}

struct FinalizedMutation {
    canonical_json: Vec<u8>,
    payload_digest: String,
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

    fn prepared_payload_digest(&self) -> Option<&str> {
        self.prepared_payload_digest.as_deref()
    }
}

impl VerifiedWrite {
    pub(crate) fn from_opened(opened: OpenedMutation) -> Self {
        let body = opened.into_body();
        Self {
            authorization: body.authorization,
            policy_snapshot: body.policy_snapshot,
            operation: body.operation,
            mutations: body
                .mutations
                .into_iter()
                .map(|prepared| VerifiedPreparedMutation {
                    mutation: prepared.mutation().clone(),
                    resource_uid: prepared.resource_uid().cloned(),
                    prepared_payload_digest: prepared.payload_digest().map(str::to_owned),
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
        store_uuid: identity.store_uuid.as_str().to_owned(),
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
        || meta.store_uuid != identity.store_uuid.as_str()
        || meta.zone_name != identity.zone.as_str()
        || meta.zone_uid != identity.zone_uid.as_str()
        || meta.created_at != identity.created_at
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
        if batch.revision().get() != *revision {
            return Err(integrity("revision-log-key-value-mismatch"));
        }
        validate_change_batch(&batch, &meta)?;
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
            || envelope.canonical_bytes().map_err(integrity)? != record.canonical_json
            || envelope.digest().map_err(integrity)? != record.payload_digest
        {
            return Err(integrity("stored-resource-identity-invalid"));
        }
        let uid = envelope.metadata().uid();
        let expected_owner_uid = envelope
            .metadata()
            .owner_ref()
            .map(|owner| resolve_uid_in_read(&types, owner))
            .transpose()?
            .map(|uid| uid.as_str().to_owned());
        if record.owner_uid != expected_owner_uid
            || record.controller_binding_id != controller_binding_id(&envelope)
        {
            return Err(integrity("stored-resource-derived-fields-invalid"));
        }
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
        if let Some(producer_ref) = endpoint_producer(&envelope)? {
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
        let (key, value) = row.map_err(integrity)?;
        let operation_id = operation_id_from_key(key.value())?;
        let operation: OperationRecord = decode(ValueKind::OperationRecord, value.value())?;
        if operation.request_digest.is_empty()
            || !valid_digest(&operation.request_digest)
            || operation.outcome != "committed"
            || operation.resources.len() != operation.resource_uids.len()
            || operation.accepted_revision > operation.finished_revision
            || operation.finished_revision > meta.current_revision
        {
            return Err(integrity("operation-revision-invalid"));
        }
        for (resource, uid) in operation.resources.iter().zip(&operation.resource_uids) {
            let resource_ref = ResourceRef::parse(&format!(
                "{}/{}",
                resource.resource_type, resource.resource_name
            ))
            .map_err(integrity)?;
            let zone = ZoneId::parse(&resource.zone).map_err(integrity)?;
            let stored = operation_resource(resource)?;
            if stored.resource_ref != resource_ref
                || stored.zone != zone
                || stored.uid.as_str() != uid
                || !valid_digest(&resource.payload_digest)
            {
                return Err(integrity("operation-resource-invalid"));
            }
        }
        if operation_id.is_empty() {
            return Err(integrity("operation-key-invalid"));
        }
    }
    validate_api_schemas(&read, &meta)?;
    validate_zone_link_cursors(&read, &meta)?;
    Ok(())
}

pub(crate) fn resource_ref_from_key(bytes: &[u8]) -> Result<ResourceRef, StoreError> {
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

fn resolve_uid_in_read(
    types: &impl ReadableTable<&'static [u8], &'static [u8]>,
    resource_ref: &ResourceRef,
) -> Result<ResourceUid, StoreError> {
    let value = types
        .get(type_index_key(resource_ref)?.as_slice())
        .map_err(integrity)?
        .ok_or_else(|| integrity("owner-resource-missing"))?;
    let uid: String = decode(ValueKind::TypeIndexRecord, value.value())?;
    ResourceUid::parse(uid).map_err(integrity)
}

fn operation_id_from_key(bytes: &[u8]) -> Result<String, StoreError> {
    let decoded = DecodedKey::decode(bytes).map_err(integrity)?;
    if decoded.key_space() != KeySpace::Operations {
        return Err(integrity("operation-key-space-invalid"));
    }
    let [crate::DecodedKeyComponent::Text(operation_id)] = decoded.components() else {
        return Err(integrity("operation-key-shape-invalid"));
    };
    Ok((*operation_id).to_owned())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiSchemaRecord {
    #[serde(rename = "resourceType", alias = "x-d2b-resource-type")]
    resource_type: ResourceTypeName,
    #[serde(rename = "validatorFingerprint", alias = "x-d2b-schema-fingerprint")]
    validator_fingerprint: String,
    #[serde(rename = "additionalProperties", default)]
    additional_properties: Option<bool>,
    #[serde(default)]
    properties: std::collections::BTreeMap<String, CanonicalJsonValue>,
    #[serde(default)]
    required: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ZoneLinkCursorRecord {
    link_epoch: u64,
    sent: u64,
    acked: u64,
    received: u64,
    applied: u64,
}

fn validate_api_schemas(read: &redb::ReadTransaction, _meta: &StoreMeta) -> Result<(), StoreError> {
    let table = read.open_table(API_SCHEMAS).map_err(integrity)?;
    for row in table.iter().map_err(integrity)? {
        let (key, value) = row.map_err(integrity)?;
        let decoded = DecodedKey::decode(key.value()).map_err(integrity)?;
        if decoded.key_space() != KeySpace::ApiSchemas {
            return Err(integrity("api-schema-key-space-invalid"));
        }
        let [crate::DecodedKeyComponent::Text(schema_key)] = decoded.components() else {
            return Err(integrity("api-schema-key-shape-invalid"));
        };
        let schema: ApiSchemaRecord = decode(ValueKind::ApiSchemaRecord, value.value())?;
        if schema.resource_type.as_str() != schema_key
            || !valid_digest(&schema.validator_fingerprint)
            || schema.additional_properties == Some(true)
            || !schema
                .required
                .iter()
                .all(|field| schema.properties.contains_key(field))
        {
            return Err(integrity("api-schema-record-invalid"));
        }
    }
    Ok(())
}

fn validate_zone_link_cursors(
    read: &redb::ReadTransaction,
    meta: &StoreMeta,
) -> Result<(), StoreError> {
    let table = read.open_table(ZONE_LINK_CURSORS).map_err(integrity)?;
    for row in table.iter().map_err(integrity)? {
        let (key, value) = row.map_err(integrity)?;
        let decoded = DecodedKey::decode(key.value()).map_err(integrity)?;
        if decoded.key_space() != KeySpace::ZoneLinkCursors {
            return Err(integrity("zone-link-cursor-key-space-invalid"));
        }
        let [crate::DecodedKeyComponent::Text(peer_zone_uid)] = decoded.components() else {
            return Err(integrity("zone-link-cursor-key-shape-invalid"));
        };
        ResourceUid::parse((*peer_zone_uid).to_owned())
            .map_err(|_| integrity("zone-link-cursor-peer-invalid"))?;
        let cursor: ZoneLinkCursorRecord = decode(ValueKind::ZoneLinkCursor, value.value())?;
        if cursor.link_epoch == 0
            || cursor.acked > cursor.sent
            || cursor.applied > cursor.received
            || cursor.sent > meta.current_revision
            || cursor.received > meta.current_revision
        {
            return Err(integrity("zone-link-cursor-record-invalid"));
        }
    }
    Ok(())
}

fn validate_change_batch(batch: &ChangeBatch, meta: &StoreMeta) -> Result<(), StoreError> {
    if batch.entries().iter().any(|entry| {
        entry.canonical_resource.as_ref().is_some_and(|bytes| {
            ResourceEnvelope::from_json(bytes).map_or(true, |envelope| {
                envelope.resource_type() != entry.resource_type()
                    || envelope.metadata().name() != entry.resource_name()
                    || envelope.metadata().uid() != &entry.resource_uid
                    || envelope.metadata().revision() != batch.revision()
                    || envelope.digest().ok().as_deref() != Some(entry.payload_digest.as_str())
            })
        }) || entry.event == ChangeEvent::Deleted && entry.canonical_resource.is_some()
            || entry.new_generation.is_none() && entry.event != ChangeEvent::Deleted
            || entry.old_generation.is_none() && !matches!(entry.event, ChangeEvent::Created)
            || entry.operation_id.as_str().is_empty()
            || entry.correlation_id.as_str().is_empty()
    }) || batch.revision().get() > meta.current_revision
    {
        return Err(integrity("change-batch-content-invalid"));
    }
    Ok(())
}

fn validate_active_schema(
    write: &redb::WriteTransaction,
    envelope: &ResourceEnvelope,
) -> Result<(), StoreError> {
    if let Some(contract) = d2b_contracts::v3::semantic_service_catalog()
        .into_iter()
        .flat_map(|pair| [pair.service(), pair.binding()])
        .find(|contract| contract.resource_type() == envelope.resource_type())
    {
        contract
            .schema_contract(std::iter::empty())
            .map_err(|_| schema_invalid("resource-schema-contract-invalid"))?
            .validate_envelope(envelope)
            .map_err(|_| schema_invalid("resource-schema-invalid"))?;
        return Ok(());
    }

    let standard = validate_standard_base(envelope)?;
    if !standard {
        return Err(schema_invalid("resource-type-schema-not-installed"));
    }
    let schemas = write.open_table(API_SCHEMAS).map_err(integrity)?;
    let key = encode_key(
        KeySpace::ApiSchemas,
        &[KeyComponent::Text(envelope.resource_type().as_str())],
    )
    .map_err(integrity)?;
    let installed = schemas.get(key.as_bytes()).map_err(integrity)?;
    if let Some(value) = installed {
        let schema: ApiSchemaRecord = decode(ValueKind::ApiSchemaRecord, value.value())?;
        if schema.resource_type != *envelope.resource_type()
            || !valid_digest(&schema.validator_fingerprint)
            || schema.additional_properties == Some(true)
            || !schema
                .required
                .iter()
                .all(|field| schema.properties.contains_key(field))
        {
            return Err(schema_invalid("resource-schema-record-invalid"));
        }
    }
    if envelope.spec().provider().is_some() || envelope.status().provider().is_some() {
        return Err(schema_invalid("provider-schema-not-installed"));
    }
    Ok(())
}

fn validate_standard_base(envelope: &ResourceEnvelope) -> Result<bool, StoreError> {
    let bytes = envelope.spec().base().to_canonical_bytes();
    let valid = match envelope.resource_type().as_str() {
        "Host" => serde_json::from_slice::<d2b_contracts::v3::host::HostSpec>(&bytes).is_ok(),
        "Guest" => serde_json::from_slice::<d2b_contracts::v3::guest::GuestSpec>(&bytes).is_ok(),
        "Process" => {
            serde_json::from_slice::<d2b_contracts::v3::process::ProcessSpec>(&bytes).is_ok()
        }
        "User" => serde_json::from_slice::<d2b_contracts::v3::user::UserSpec>(&bytes).is_ok(),
        "Provider" => {
            serde_json::from_slice::<d2b_contracts::v3::provider::ProviderSpec>(&bytes).is_ok()
        }
        "Zone" => envelope.spec().base().is_empty(),
        _ => return Ok(false),
    };
    if !valid {
        return Err(schema_invalid("resource-base-schema-invalid"));
    }
    Ok(true)
}

fn schema_invalid(reason: &'static str) -> StoreError {
    error(StoreErrorKind::ResourceSchemaInvalid, None, reason)
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
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
    #[cfg(test)]
    if FAIL_NEXT_APPLY_GROUP.swap(false, std::sync::atomic::Ordering::SeqCst) {
        return Err(durability_failure("injected-commit-failure"));
    }
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
        if let Err(error) = validate_prepared_payloads(&verified) {
            results.push(Err(error));
            continue;
        }
        let operation_id = verified.operation.operation_id.clone();
        let correlation_id = verified.operation.correlation_id.clone();
        let request_digest = operation_digest(&verified)?;
        let operation_key = operation_key(&operation_id)?;
        {
            let operations = write.open_table(OPERATIONS).map_err(integrity)?;
            if let Some(bytes) = operations
                .get(operation_key.as_slice())
                .map_err(integrity)?
            {
                let prior: OperationRecord = decode(ValueKind::OperationRecord, bytes.value())?;
                if prior.request_digest == request_digest {
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
        let mut verified = verified;
        for prepared in &mut verified.mutations {
            if prepared.mutation.kind == ResourceMutationKind::Create {
                prepared.resource_uid = Some(mint_resource_uid()?);
            }
        }
        let finalized =
            match validate_verified_write(&write, &verified, revision, &accepted_targets) {
                Ok(finalized) => finalized,
                Err(error) => {
                    results[result_index] = Err(error);
                    continue;
                }
            };
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
                finalized
                    .get(ordinal)
                    .ok_or_else(|| integrity("finalized-mutation-missing"))?
                    .as_ref(),
                revision,
                u32::try_from(ordinal).map_err(integrity)?,
                &operation_id,
                &correlation_id,
            )?;
            group_resources.push(resource);
            group_entries.push(entry);
        }
        let operation = OperationRecord {
            request_digest,
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
    let batch = ChangeBatch::new(ZoneRevision::new(revision), entries)?;
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

#[cfg(test)]
static FAIL_NEXT_APPLY_GROUP: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
pub(crate) fn fail_next_apply_group_for_test() {
    FAIL_NEXT_APPLY_GROUP.store(true, std::sync::atomic::Ordering::SeqCst);
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
            let resource_ref = resource_ref_from_key(key.value())?;
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
            continue;
        }
        let (uid, owner) = if mutation.kind == ResourceMutationKind::UpdateFinalizers {
            state
                .get(&mutation.target)
                .map(|(uid, owner)| (uid.clone(), owner.clone()))
                .ok_or_else(|| integrity("mutation-resource-uid-missing"))?
        } else {
            (
                prepared
                    .resource_uid()
                    .cloned()
                    .ok_or_else(|| integrity("mutation-resource-uid-missing"))?,
                mutation.owner.clone(),
            )
        };
        if let Some(owner) = &owner {
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
        state.insert(mutation.target.clone(), (uid, owner));
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
    let envelope = ResourceEnvelope::from_json(&record.canonical_json)
        .map_err(|_| integrity("operation-resource-envelope-invalid"))?;
    let zone = ZoneId::parse(&record.zone).map_err(integrity)?;
    if envelope.resource_type() != resource_ref.resource_type()
        || envelope.metadata().name() != resource_ref.name()
        || envelope.metadata().zone() != &zone
        || envelope.digest().map_err(integrity)? != record.payload_digest
    {
        return Err(integrity("operation-resource-invalid"));
    }
    Ok(StoredResource {
        resource_ref,
        zone,
        uid: envelope.metadata().uid().clone(),
        generation: envelope.metadata().generation(),
        revision: envelope.metadata().revision(),
        canonical_json: record.canonical_json.clone(),
        payload_digest: record.payload_digest.clone(),
    })
}

fn apply_prepared(
    write: &redb::WriteTransaction,
    prepared: &VerifiedPreparedMutation,
    finalized: Option<&FinalizedMutation>,
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
        let old_record = previous.as_ref().expect("previous resource was checked");
        let old_envelope = ResourceEnvelope::from_json(&old_record.canonical_json)
            .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
        if !deletion_requested(&old_record.canonical_json)? {
            let canonical_json = merge_deletion_request(&old_record.canonical_json, revision)?;
            let envelope = ResourceEnvelope::from_json(&canonical_json)
                .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
            validate_active_schema(write, &envelope)?;
            let payload_digest = envelope.digest().map_err(integrity)?;
            let record = ResourceRecord {
                canonical_json: canonical_json.clone(),
                owner_uid: old_record.owner_uid.clone(),
                controller_binding_id: old_record.controller_binding_id.clone(),
                payload_digest: payload_digest.clone(),
            };
            write
                .open_table(RESOURCES)
                .map_err(integrity)?
                .insert(
                    key.as_slice(),
                    encode(ValueKind::ResourceRecord, &record)?.as_slice(),
                )
                .map_err(integrity)?;
            let resource = stored_resource(&mutation.zone, &mutation.target, &record)?;
            return Ok((
                resource.clone(),
                ChangeEntry::new(
                    ordinal,
                    mutation.target.resource_type().clone(),
                    mutation.target.name().clone(),
                    old.uid.clone(),
                    ChangeEvent::DeletionRequested,
                    Some(old.generation),
                    Some(resource.generation),
                    parse_optional_uid(old_record.owner_uid.as_deref())?,
                    payload_digest,
                    Some(canonical_json),
                    operation_id.to_owned(),
                    correlation_id.to_owned(),
                )?,
            ));
        }
        if has_finalizers(&old_record.canonical_json)? {
            return Err(error(
                StoreErrorKind::ResourceFinalizerDenied,
                None,
                "resource-finalizers-remain",
            ));
        }
        if owned_children_remain(write, &mutation.target)? {
            return Err(error(
                StoreErrorKind::ResourceFinalizerDenied,
                None,
                "owned-children-remain",
            ));
        }
        if produced_endpoints_remain(write, &old.uid)? {
            return Err(error(
                StoreErrorKind::ResourceFinalizerDenied,
                None,
                "produced-endpoints-remain",
            ));
        }
        remove_indexes(write, &old, old_record, &old_envelope)?;
        write
            .open_table(RESOURCES)
            .map_err(integrity)?
            .remove(key.as_slice())
            .map_err(integrity)?;
        return Ok((
            old.clone(),
            ChangeEntry::new(
                ordinal,
                mutation.target.resource_type().clone(),
                mutation.target.name().clone(),
                old.uid.clone(),
                ChangeEvent::Deleted,
                Some(old.generation),
                None,
                parse_optional_uid(old_record.owner_uid.as_deref())?,
                old.payload_digest.clone(),
                None,
                operation_id.to_owned(),
                correlation_id.to_owned(),
            )?,
        ));
    }

    let finalized = finalized.ok_or_else(|| integrity("finalized-mutation-missing"))?;
    let canonical_json = finalized.canonical_json.clone();
    let envelope = ResourceEnvelope::from_json(&canonical_json)
        .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
    validate_active_schema(write, &envelope)?;
    let uid = envelope.metadata().uid().clone();
    let effective_owner = if matches!(
        mutation.kind,
        ResourceMutationKind::Create | ResourceMutationKind::UpdateMetadata
    ) {
        mutation.owner.clone()
    } else {
        previous_resource
            .as_ref()
            .and(previous.as_ref())
            .and_then(|record| ResourceEnvelope::from_json(&record.canonical_json).ok())
            .and_then(|envelope| envelope.metadata().owner_ref().cloned())
    };
    if envelope.resource_type() != mutation.target.resource_type()
        || envelope.metadata().name() != mutation.target.name()
        || envelope.metadata().zone() != &mutation.zone
        || envelope.metadata().owner_ref() != effective_owner.as_ref()
    {
        return Err(integrity("mutation-resource-identity-mismatch"));
    }
    let owner_uid = match &effective_owner {
        Some(owner_ref) => Some(resolve_uid_in_write(write, owner_ref)?.as_str().to_owned()),
        None => None,
    };
    if let (Some(previous_resource), Some(previous_record)) = (&previous_resource, &previous) {
        let previous_envelope = ResourceEnvelope::from_json(&previous_record.canonical_json)
            .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
        remove_indexes(
            write,
            previous_resource,
            previous_record,
            &previous_envelope,
        )?;
    }
    let payload_digest = envelope.digest().map_err(integrity)?;
    if payload_digest != finalized.payload_digest {
        return Err(integrity("finalized-payload-digest-mismatch"));
    }
    let record = ResourceRecord {
        canonical_json: canonical_json.clone(),
        owner_uid: owner_uid.clone(),
        controller_binding_id: controller_binding_id(&envelope),
        payload_digest: payload_digest.clone(),
    };
    let producer = endpoint_producer(&envelope)?;
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
        ResourceMutationKind::Create => ChangeEvent::Created,
        ResourceMutationKind::UpdateSpec => ChangeEvent::SpecUpdated,
        ResourceMutationKind::UpdateStatus => ChangeEvent::StatusUpdated,
        ResourceMutationKind::UpdateMetadata | ResourceMutationKind::UpdateFinalizers => {
            ChangeEvent::MetadataUpdated
        }
        ResourceMutationKind::Delete => unreachable!("delete returned above"),
    };
    Ok((
        resource.clone(),
        ChangeEntry::new(
            ordinal,
            mutation.target.resource_type().clone(),
            mutation.target.name().clone(),
            uid,
            event,
            previous_resource
                .as_ref()
                .map(|resource| resource.generation),
            Some(resource.generation),
            parse_optional_uid(owner_uid.as_deref())?,
            payload_digest,
            Some(canonical_json),
            operation_id.to_owned(),
            correlation_id.to_owned(),
        )?,
    ))
}

fn validate_verified_write(
    write: &redb::WriteTransaction,
    verified: &VerifiedWrite,
    revision: u64,
    accepted_targets: &std::collections::BTreeSet<ResourceRef>,
) -> Result<Vec<Option<FinalizedMutation>>, StoreError> {
    let meta = read_meta_in_write(write)?;
    if verified.authorization.zone.as_str() != meta.zone_name {
        return Err(integrity("mutation-zone-mismatch"));
    }
    let mut staged = std::collections::BTreeMap::<ResourceRef, Option<ResourceUid>>::new();
    let mut finalized = Vec::with_capacity(verified.mutations.len());
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
        if mutation.kind != ResourceMutationKind::Delete
            && mutation.kind != ResourceMutationKind::Create
            && mutation.kind != ResourceMutationKind::UpdateFinalizers
        {
            let prepared_uid = prepared
                .resource_uid()
                .ok_or_else(|| integrity("mutation-resource-uid-missing"))?;
            if current
                .as_ref()
                .is_none_or(|(current_uid, _)| current_uid != prepared_uid)
            {
                return Err(conflict(
                    current.as_ref().map_or(0, |(_, revision)| *revision),
                    ordinal,
                    "resource-uid-changed",
                ));
            }
        }

        if mutation.kind == ResourceMutationKind::Delete {
            if current.is_none() {
                return Err(error(
                    StoreErrorKind::ResourceNotFound,
                    None,
                    "resource-not-found",
                ));
            }
            let (record, envelope) = current_record_in_write(write, &mutation.target)?
                .ok_or_else(|| integrity("mutation-current-resource-missing"))?;
            if deletion_requested(&record.canonical_json)? {
                if has_finalizers(&record.canonical_json)? {
                    return Err(error(
                        StoreErrorKind::ResourceFinalizerDenied,
                        None,
                        "resource-finalizers-remain",
                    ));
                }
                if owned_children_remain(write, &mutation.target)? {
                    return Err(error(
                        StoreErrorKind::ResourceFinalizerDenied,
                        None,
                        "owned-children-remain",
                    ));
                }
                if produced_endpoints_remain(write, envelope.metadata().uid())? {
                    return Err(error(
                        StoreErrorKind::ResourceFinalizerDenied,
                        None,
                        "produced-endpoints-remain",
                    ));
                }
            }
            staged.insert(
                mutation.target.clone(),
                current.as_ref().map(|(uid, _)| uid.clone()),
            );
            finalized.push(None);
            continue;
        }

        if mutation.kind == ResourceMutationKind::UpdateFinalizers {
            if current.is_none() {
                return Err(error(
                    StoreErrorKind::ResourceNotFound,
                    None,
                    "resource-not-found",
                ));
            }
            if mutation.canonical_resource.is_some() {
                return Err(integrity("finalizer-mutation-body-present"));
            }
            let uid = current
                .as_ref()
                .map(|(uid, _)| uid.clone())
                .ok_or_else(|| integrity("mutation-resource-uid-missing"))?;
            if prepared
                .resource_uid()
                .is_some_and(|prepared_uid| prepared_uid != &uid)
            {
                return Err(conflict(
                    current.as_ref().map_or(0, |(_, revision)| *revision),
                    ordinal,
                    "resource-uid-changed",
                ));
            }
            let previous =
                current_record_in_write(write, &mutation.target)?.map(|(record, _)| record);
            finalized.push(Some(finalize_authorized_mutation(
                prepared,
                previous.as_ref(),
                revision,
                &uid,
            )?));
            staged.insert(mutation.target.clone(), Some(uid));
            continue;
        }

        let bytes = mutation
            .canonical_resource
            .as_deref()
            .ok_or_else(|| integrity("mutation-resource-body-missing"))?;
        let uid = prepared
            .resource_uid()
            .cloned()
            .ok_or_else(|| integrity("mutation-resource-uid-missing"))?;
        if mutation.kind != ResourceMutationKind::Create {
            let envelope = ResourceEnvelope::from_json(bytes)
                .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
            if envelope.resource_type() != mutation.target.resource_type()
                || envelope.metadata().name() != mutation.target.name()
                || envelope.metadata().zone() != &mutation.zone
            {
                return Err(integrity("mutation-resource-identity-mismatch"));
            }
            if envelope.metadata().uid() != &uid {
                return Err(integrity("mutation-resource-uid-mismatch"));
            }
        }
        if mutation
            .expected_uid
            .as_ref()
            .is_some_and(|expected| expected != &uid)
        {
            return Err(conflict(
                current.as_ref().map_or(0, |(_, revision)| *revision),
                ordinal,
                "resource-uid-changed",
            ));
        }
        let current_owner = current_record_in_write(write, &mutation.target)?
            .and_then(|(_, envelope)| envelope.metadata().owner_ref().cloned());
        let owner = if mutation.kind == ResourceMutationKind::Create
            || mutation.kind == ResourceMutationKind::UpdateMetadata
        {
            mutation.owner.as_ref()
        } else {
            current_owner.as_ref()
        };
        if let Some(owner_ref) = owner {
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
        let previous = if mutation.kind == ResourceMutationKind::Create {
            None
        } else {
            current_record_in_write(write, &mutation.target)?.map(|(record, _)| record)
        };
        finalized.push(Some(finalize_authorized_mutation(
            prepared,
            previous.as_ref(),
            revision,
            &uid,
        )?));
        staged.insert(mutation.target.clone(), Some(uid));
    }
    Ok(finalized)
}

fn validate_prepared_payloads(verified: &VerifiedWrite) -> Result<(), StoreError> {
    for prepared in &verified.mutations {
        validate_prepared_source_digest(prepared)?;
    }
    Ok(())
}

fn validate_prepared_source_digest(prepared: &VerifiedPreparedMutation) -> Result<(), StoreError> {
    let mutation = prepared.mutation();
    let Some(bytes) = mutation.canonical_resource.as_deref() else {
        if prepared.prepared_payload_digest().is_some() {
            return Err(integrity("mutation-payload-digest-without-body"));
        }
        return Ok(());
    };
    let expected = prepared
        .prepared_payload_digest()
        .ok_or_else(|| integrity("mutation-payload-digest-missing"))?;
    let digest = if mutation.kind == ResourceMutationKind::Create {
        let value = CanonicalJsonValue::parse(bytes)
            .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
        canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &value.to_canonical_bytes())
    } else {
        let envelope = ResourceEnvelope::from_json(bytes)
            .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
        envelope.digest().map_err(integrity)?
    };
    if digest != expected {
        return Err(integrity("mutation-payload-digest-mismatch"));
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
    Ok(current_record_in_write(write, resource_ref)?
        .and_then(|(_, envelope)| envelope.metadata().owner_ref().cloned()))
}

fn current_identity_in_write(
    write: &redb::WriteTransaction,
    resource_ref: &ResourceRef,
) -> Result<Option<(ResourceUid, u64)>, StoreError> {
    Ok(
        current_record_in_write(write, resource_ref)?.map(|(_, envelope)| {
            (
                envelope.metadata().uid().clone(),
                envelope.metadata().revision().get(),
            )
        }),
    )
}

fn current_record_in_write(
    write: &redb::WriteTransaction,
    resource_ref: &ResourceRef,
) -> Result<Option<(ResourceRecord, ResourceEnvelope)>, StoreError> {
    let table = write.open_table(RESOURCES).map_err(integrity)?;
    let key = resource_key(resource_ref)?;
    table
        .get(key.as_slice())
        .map_err(integrity)?
        .map(|bytes| {
            let record: ResourceRecord = decode(ValueKind::ResourceRecord, bytes.value())?;
            let envelope = ResourceEnvelope::from_json(&record.canonical_json)
                .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
            Ok((record, envelope))
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
    envelope: &ResourceEnvelope,
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
    if let Some(producer_ref) = endpoint_producer(envelope)? {
        let producer_uid = resolve_uid_in_write(write, &producer_ref)?;
        let key = encode_key(
            KeySpace::ProducerIndex,
            &[
                KeyComponent::Text(producer_uid.as_str()),
                KeyComponent::Text(resource.uid.as_str()),
            ],
        )
        .map_err(integrity)?;
        write
            .open_table(PRODUCER_INDEX)
            .map_err(integrity)?
            .remove(key.as_bytes())
            .map_err(integrity)?;
    }
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

fn produced_endpoints_remain(
    write: &redb::WriteTransaction,
    producer_uid: &ResourceUid,
) -> Result<bool, StoreError> {
    let table = write.open_table(PRODUCER_INDEX).map_err(integrity)?;
    for row in table.iter().map_err(integrity)? {
        let (key, _) = row.map_err(integrity)?;
        let decoded = DecodedKey::decode(key.value()).map_err(integrity)?;
        let [
            crate::DecodedKeyComponent::Text(indexed_producer_uid),
            crate::DecodedKeyComponent::Text(_),
        ] = decoded.components()
        else {
            return Err(integrity("producer-index-key-shape-invalid"));
        };
        if indexed_producer_uid == producer_uid.as_str() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn endpoint_producer(envelope: &ResourceEnvelope) -> Result<Option<ResourceRef>, StoreError> {
    if envelope.resource_type().as_str() != "Endpoint" {
        return Ok(None);
    }
    match envelope.spec().base().get("producerRef") {
        Some(CanonicalJsonValue::String(reference)) => ResourceRef::parse(reference)
            .map(Some)
            .map_err(|_| integrity("endpoint-producer-ref-invalid")),
        _ => Err(integrity("endpoint-producer-ref-missing")),
    }
}

fn owned_children_remain(
    write: &redb::WriteTransaction,
    target: &ResourceRef,
) -> Result<bool, StoreError> {
    let table = write.open_table(RESOURCES).map_err(integrity)?;
    for row in table.iter().map_err(integrity)? {
        let (_, value) = row.map_err(integrity)?;
        let record: ResourceRecord = decode(ValueKind::ResourceRecord, value.value())?;
        let envelope = ResourceEnvelope::from_json(&record.canonical_json)
            .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
        if envelope.metadata().owner_ref() == Some(target) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn finalize_authorized_mutation(
    prepared: &VerifiedPreparedMutation,
    previous: Option<&ResourceRecord>,
    revision: u64,
    resource_uid: &ResourceUid,
) -> Result<FinalizedMutation, StoreError> {
    let mutation = prepared.mutation();
    let canonical_json = merge_authorized_mutation(prepared, previous, revision)?;
    let envelope = ResourceEnvelope::from_json(&canonical_json)
        .map_err(|_| integrity("stored-resource-envelope-invalid"))?;

    let effective_owner = if matches!(
        mutation.kind,
        ResourceMutationKind::Create | ResourceMutationKind::UpdateMetadata
    ) {
        mutation.owner.clone()
    } else {
        let previous = previous.ok_or_else(|| integrity("mutation-current-resource-missing"))?;
        ResourceEnvelope::from_json(&previous.canonical_json)
            .map_err(|_| integrity("stored-resource-envelope-invalid"))?
            .metadata()
            .owner_ref()
            .cloned()
    };
    if envelope.resource_type() != mutation.target.resource_type()
        || envelope.metadata().name() != mutation.target.name()
        || envelope.metadata().zone() != &mutation.zone
        || envelope.metadata().uid() != resource_uid
        || envelope.metadata().owner_ref() != effective_owner.as_ref()
    {
        return Err(integrity("mutation-resource-identity-mismatch"));
    }

    Ok(FinalizedMutation {
        canonical_json,
        payload_digest: envelope.digest().map_err(integrity)?,
    })
}

fn merge_authorized_mutation(
    prepared: &VerifiedPreparedMutation,
    previous: Option<&ResourceRecord>,
    revision: u64,
) -> Result<Vec<u8>, StoreError> {
    let mutation = prepared.mutation();
    if mutation.kind == ResourceMutationKind::Create {
        let source = mutation
            .canonical_resource
            .as_deref()
            .ok_or_else(|| integrity("mutation-resource-body-missing"))?;
        let mut value = CanonicalJsonValue::parse(source)
            .map_err(|_| integrity("mutation-resource-envelope-invalid"))?;
        let uid = prepared
            .resource_uid()
            .cloned()
            .ok_or_else(|| integrity("mutation-resource-uid-missing"))?;
        let metadata = metadata_object_mut(&mut value)?;
        metadata.insert(
            "name".to_owned(),
            CanonicalJsonValue::String(mutation.target.name().as_str().to_owned()),
        );
        metadata.insert(
            "zone".to_owned(),
            CanonicalJsonValue::String(mutation.zone.as_str().to_owned()),
        );
        metadata.insert(
            "ownerRef".to_owned(),
            mutation
                .owner
                .as_ref()
                .map_or(CanonicalJsonValue::Null, |owner| {
                    CanonicalJsonValue::String(owner.to_canonical_string())
                }),
        );
        metadata.insert(
            "uid".to_owned(),
            CanonicalJsonValue::String(uid.as_str().to_owned()),
        );
        metadata.insert("generation".to_owned(), CanonicalJsonValue::Integer(1));
        metadata.insert(
            "revision".to_owned(),
            CanonicalJsonValue::Integer(
                i64::try_from(revision).map_err(|_| integrity("zone-revision-out-of-range"))?,
            ),
        );
        let now = canonical_timestamp()?;
        metadata.insert(
            "createdAt".to_owned(),
            CanonicalJsonValue::String(now.clone()),
        );
        metadata.insert("updatedAt".to_owned(), CanonicalJsonValue::String(now));
        metadata.insert(
            "finalizers".to_owned(),
            CanonicalJsonValue::Array(Vec::new()),
        );
        metadata.insert("deletionRequestedAt".to_owned(), CanonicalJsonValue::Null);
        metadata.insert(
            "managedBy".to_owned(),
            CanonicalJsonValue::String("api".to_owned()),
        );
        for field in [
            "configurationGeneration",
            "controllerGeneration",
            "providerGeneration",
        ] {
            metadata.remove(field);
        }
        let CanonicalJsonValue::Object(root) = &mut value else {
            return Err(integrity("mutation-resource-envelope-invalid"));
        };
        root.insert(
            "type".to_owned(),
            CanonicalJsonValue::String(mutation.target.resource_type().as_str().to_owned()),
        );
        let canonical = value.to_canonical_bytes();
        let envelope = ResourceEnvelope::from_json(&canonical)
            .map_err(|_| integrity("mutation-resource-envelope-invalid"))?;
        if envelope.metadata().uid() != &uid {
            return Err(integrity("mutation-resource-uid-mismatch"));
        }
        return Ok(canonical);
    }

    let previous = previous.ok_or_else(|| integrity("mutation-current-resource-missing"))?;
    let mut stored = CanonicalJsonValue::parse(&previous.canonical_json)
        .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
    match mutation.kind {
        ResourceMutationKind::UpdateSpec => {
            let caller = mutation_body_object(mutation)?;
            replace_layer(&mut stored, &caller, "spec")?;
            bump_generation(&mut stored)?;
        }
        ResourceMutationKind::UpdateStatus => {
            let caller = mutation_body_object(mutation)?;
            replace_layer(&mut stored, &caller, "status")?;
        }
        ResourceMutationKind::UpdateMetadata => {
            let caller = mutation_body_object(mutation)?;
            let caller_metadata = caller
                .get("metadata")
                .and_then(|value| match value {
                    CanonicalJsonValue::Object(value) => Some(value),
                    _ => None,
                })
                .ok_or_else(|| integrity("mutation-resource-metadata-missing"))?;
            let stored_metadata = metadata_object_mut(&mut stored)?;
            for field in ["ownerRef", "labels", "annotations"] {
                match caller_metadata.get(field) {
                    Some(value) => {
                        stored_metadata.insert(field.to_owned(), value.clone());
                    }
                    None => {
                        stored_metadata.remove(field);
                    }
                }
            }
        }
        ResourceMutationKind::UpdateFinalizers => {
            apply_finalizer_delta(&mut stored, mutation)?;
        }
        ResourceMutationKind::Create | ResourceMutationKind::Delete => {
            unreachable!("create and delete have dedicated transitions")
        }
    }
    let metadata = metadata_object_mut(&mut stored)?;
    metadata.insert(
        "revision".to_owned(),
        CanonicalJsonValue::Integer(
            i64::try_from(revision).map_err(|_| integrity("zone-revision-out-of-range"))?,
        ),
    );
    metadata.insert(
        "updatedAt".to_owned(),
        CanonicalJsonValue::String(canonical_timestamp()?),
    );
    Ok(stored.to_canonical_bytes())
}

fn merge_deletion_request(bytes: &[u8], revision: u64) -> Result<Vec<u8>, StoreError> {
    let mut value = CanonicalJsonValue::parse(bytes)
        .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
    let timestamp = canonical_timestamp()?;
    let metadata = metadata_object_mut(&mut value)?;
    metadata.insert(
        "deletionRequestedAt".to_owned(),
        CanonicalJsonValue::String(timestamp.clone()),
    );
    metadata.insert(
        "updatedAt".to_owned(),
        CanonicalJsonValue::String(timestamp),
    );
    metadata.insert(
        "revision".to_owned(),
        CanonicalJsonValue::Integer(
            i64::try_from(revision).map_err(|_| integrity("zone-revision-out-of-range"))?,
        ),
    );
    Ok(value.to_canonical_bytes())
}

fn mutation_body_object(
    mutation: &StoreMutation,
) -> Result<std::collections::BTreeMap<String, CanonicalJsonValue>, StoreError> {
    let bytes = mutation
        .canonical_resource
        .as_deref()
        .ok_or_else(|| integrity("mutation-resource-body-missing"))?;
    let value = CanonicalJsonValue::parse(bytes)
        .map_err(|_| integrity("mutation-resource-envelope-invalid"))?;
    let CanonicalJsonValue::Object(root) = value else {
        return Err(integrity("mutation-resource-envelope-invalid"));
    };
    Ok(root)
}

fn replace_layer(
    stored: &mut CanonicalJsonValue,
    caller: &std::collections::BTreeMap<String, CanonicalJsonValue>,
    layer: &str,
) -> Result<(), StoreError> {
    let CanonicalJsonValue::Object(root) = stored else {
        return Err(integrity("stored-resource-envelope-invalid"));
    };
    let value = caller
        .get(layer)
        .cloned()
        .ok_or_else(|| integrity("mutation-authorized-layer-missing"))?;
    root.insert(layer.to_owned(), value);
    Ok(())
}

fn metadata_object_mut(
    value: &mut CanonicalJsonValue,
) -> Result<&mut std::collections::BTreeMap<String, CanonicalJsonValue>, StoreError> {
    let CanonicalJsonValue::Object(root) = value else {
        return Err(integrity("mutation-resource-envelope-invalid"));
    };
    let Some(CanonicalJsonValue::Object(metadata)) = root.get_mut("metadata") else {
        return Err(integrity("mutation-resource-metadata-missing"));
    };
    Ok(metadata)
}

fn bump_generation(value: &mut CanonicalJsonValue) -> Result<(), StoreError> {
    let metadata = metadata_object_mut(value)?;
    let generation = match metadata.get("generation") {
        Some(CanonicalJsonValue::Integer(generation)) => *generation,
        _ => return Err(integrity("stored-resource-generation-invalid")),
    };
    metadata.insert(
        "generation".to_owned(),
        CanonicalJsonValue::Integer(
            generation
                .checked_add(1)
                .ok_or_else(|| integrity("resource-generation-exhausted"))?,
        ),
    );
    Ok(())
}

fn apply_finalizer_delta(
    value: &mut CanonicalJsonValue,
    mutation: &StoreMutation,
) -> Result<(), StoreError> {
    let metadata = metadata_object_mut(value)?;
    let Some(CanonicalJsonValue::Array(current)) = metadata.get("finalizers") else {
        return Err(integrity("stored-resource-finalizers-invalid"));
    };
    let mut finalizers = current
        .iter()
        .map(|value| match value {
            CanonicalJsonValue::String(value) => FinalizerId::parse(value.clone())
                .map_err(|_| integrity("stored-resource-finalizers-invalid")),
            _ => Err(integrity("stored-resource-finalizers-invalid")),
        })
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    for finalizer in &mutation.remove_finalizers {
        finalizers.remove(finalizer);
    }
    finalizers.extend(mutation.add_finalizers.iter().cloned());
    if finalizers.len() > d2b_contracts::v3::resource::MAX_FINALIZERS {
        return Err(error(
            StoreErrorKind::ResourceSchemaInvalid,
            None,
            "too-many-finalizers",
        ));
    }
    metadata.insert(
        "finalizers".to_owned(),
        CanonicalJsonValue::Array(
            finalizers
                .into_iter()
                .map(|finalizer| CanonicalJsonValue::String(finalizer.to_canonical_string()))
                .collect(),
        ),
    );
    Ok(())
}

fn deletion_requested(bytes: &[u8]) -> Result<bool, StoreError> {
    let value = CanonicalJsonValue::parse(bytes)
        .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
    let CanonicalJsonValue::Object(root) = value else {
        return Err(integrity("stored-resource-envelope-invalid"));
    };
    let Some(CanonicalJsonValue::Object(metadata)) = root.get("metadata") else {
        return Err(integrity("stored-resource-metadata-missing"));
    };
    match metadata.get("deletionRequestedAt") {
        Some(CanonicalJsonValue::Null) => Ok(false),
        Some(CanonicalJsonValue::String(value)) => {
            Timestamp::parse(value.clone()).map_err(integrity)?;
            Ok(true)
        }
        _ => Err(integrity("stored-resource-deletion-state-invalid")),
    }
}

fn has_finalizers(bytes: &[u8]) -> Result<bool, StoreError> {
    let value = CanonicalJsonValue::parse(bytes)
        .map_err(|_| integrity("stored-resource-envelope-invalid"))?;
    let CanonicalJsonValue::Object(root) = value else {
        return Err(integrity("stored-resource-envelope-invalid"));
    };
    let Some(CanonicalJsonValue::Object(metadata)) = root.get("metadata") else {
        return Err(integrity("stored-resource-metadata-missing"));
    };
    match metadata.get("finalizers") {
        Some(CanonicalJsonValue::Array(values)) => Ok(!values.is_empty()),
        _ => Err(integrity("stored-resource-finalizers-invalid")),
    }
}

fn parse_optional_uid(value: Option<&str>) -> Result<Option<ResourceUid>, StoreError> {
    value
        .map(|value| ResourceUid::parse(value.to_owned()).map_err(integrity))
        .transpose()
}

fn mint_resource_uid() -> Result<ResourceUid, StoreError> {
    use std::io::Read as _;

    let mut bytes = [0_u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(|_| integrity("resource-uid-entropy-unavailable"))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let rendered = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    );
    ResourceUid::parse(rendered).map_err(|_| integrity("resource-uid-mint-invalid"))
}

fn canonical_timestamp() -> Result<String, StoreError> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| integrity("system-clock-invalid"))?;
    let seconds = elapsed.as_secs();
    let days = i64::try_from(seconds / 86_400).map_err(integrity)?;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let rendered = format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{:03}Z",
        elapsed.subsec_millis()
    );
    Timestamp::parse(rendered.clone()).map_err(integrity)?;
    Ok(rendered)
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { y + 1 } else { y } as i32;
    (year, month, day)
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

fn controller_binding_id(envelope: &ResourceEnvelope) -> String {
    envelope.spec().provider_ref().cloned().map_or_else(
        || envelope.resource_type().as_str().to_owned(),
        |provider| provider.to_canonical_string(),
    )
}

fn operation_digest(verified: &VerifiedWrite) -> Result<String, StoreError> {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    digest_field(&mut digest, verified.operation.operation_id.as_bytes())?;
    digest_optional_field(
        &mut digest,
        verified
            .operation
            .idempotency_key
            .as_deref()
            .map(str::as_bytes),
    )?;
    digest_field(&mut digest, verified.operation.correlation_id.as_bytes())?;
    digest_optional_field(
        &mut digest,
        verified.operation.trace_id.as_deref().map(str::as_bytes),
    )?;
    digest.update(verified.operation.deadline_ms.to_be_bytes());
    digest_field(&mut digest, verified.authorization.zone.as_str().as_bytes())?;
    digest_field(
        &mut digest,
        verified
            .authorization
            .subject_ref
            .to_canonical_string()
            .as_bytes(),
    )?;
    digest.update(verified.authorization.subject_uid.as_str().as_bytes());
    digest.update(verified.policy_snapshot.policy_revision.to_be_bytes());
    digest.update(verified.policy_snapshot.api_catalog_revision.to_be_bytes());
    digest.update(
        verified
            .policy_snapshot
            .active_configuration_revision
            .get()
            .to_be_bytes(),
    );
    digest_optional_u64(
        &mut digest,
        verified
            .policy_snapshot
            .controller_generation
            .map(ControllerGeneration::get),
    );
    digest.update(
        u32::try_from(verified.mutations.len())
            .map_err(|_| integrity("operation-request-too-large"))?
            .to_be_bytes(),
    );
    for mutation in &verified.mutations {
        let prepared = mutation.mutation();
        digest_field(&mut digest, prepared.zone.as_str().as_bytes())?;
        digest_field(
            &mut digest,
            prepared.target.to_canonical_string().as_bytes(),
        )?;
        digest.update([mutation_kind_discriminant(prepared.kind)]);
        match prepared.expected {
            ExpectedRevision::CreateAbsent => digest.update([0]),
            ExpectedRevision::Exact(revision) => {
                digest.update([1]);
                digest.update(revision.get().to_be_bytes());
            }
        }
        digest_optional_field(
            &mut digest,
            prepared
                .expected_uid
                .as_ref()
                .map(|uid| uid.as_str().as_bytes()),
        )?;
        digest_optional_field(
            &mut digest,
            prepared
                .owner
                .as_ref()
                .map(|owner| owner.to_canonical_string())
                .as_deref()
                .map(str::as_bytes),
        )?;
        let request_body = canonical_request_body(prepared)?;
        digest_optional_field(&mut digest, request_body.as_deref())?;
        digest_finalizers(&mut digest, &prepared.add_finalizers)?;
        digest_finalizers(&mut digest, &prepared.remove_finalizers)?;
        digest.update([u8::from(prepared.wait_for_reconcile)]);
        digest_optional_u64(&mut digest, prepared.reconcile_deadline_ms);
        if prepared.kind != ResourceMutationKind::Create {
            digest_optional_field(
                &mut digest,
                mutation.resource_uid().map(|uid| uid.as_str().as_bytes()),
            )?;
            digest_optional_field(
                &mut digest,
                mutation.prepared_payload_digest().map(str::as_bytes),
            )?;
        }
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn digest_field(digest: &mut sha2::Sha256, bytes: &[u8]) -> Result<(), StoreError> {
    use sha2::Digest;
    digest.update(
        u32::try_from(bytes.len())
            .map_err(|_| integrity("operation-request-too-large"))?
            .to_be_bytes(),
    );
    digest.update(bytes);
    Ok(())
}

fn digest_optional_field(
    digest: &mut sha2::Sha256,
    bytes: Option<&[u8]>,
) -> Result<(), StoreError> {
    use sha2::Digest;
    match bytes {
        Some(bytes) => {
            digest.update([1]);
            digest_field(digest, bytes)
        }
        None => {
            digest.update([0]);
            Ok(())
        }
    }
}

fn digest_optional_u64(digest: &mut sha2::Sha256, value: Option<u64>) {
    use sha2::Digest;
    match value {
        Some(value) => {
            digest.update([1]);
            digest.update(value.to_be_bytes());
        }
        None => digest.update([0]),
    }
}

fn digest_finalizers(
    digest: &mut sha2::Sha256,
    finalizers: &[FinalizerId],
) -> Result<(), StoreError> {
    use sha2::Digest;
    digest.update(
        u32::try_from(finalizers.len())
            .map_err(|_| integrity("operation-request-too-large"))?
            .to_be_bytes(),
    );
    for finalizer in finalizers {
        digest_field(digest, finalizer.as_str().as_bytes())?;
    }
    Ok(())
}

fn canonical_request_body(mutation: &StoreMutation) -> Result<Option<Vec<u8>>, StoreError> {
    let Some(bytes) = mutation.canonical_resource.as_deref() else {
        return Ok(None);
    };
    if mutation.kind != ResourceMutationKind::Create {
        return Ok(Some(bytes.to_vec()));
    }
    let mut value = CanonicalJsonValue::parse(bytes)
        .map_err(|_| integrity("mutation-resource-envelope-invalid"))?;
    let metadata = metadata_object_mut(&mut value)?;
    for field in [
        "uid",
        "generation",
        "revision",
        "createdAt",
        "updatedAt",
        "finalizers",
        "deletionRequestedAt",
        "managedBy",
        "configurationGeneration",
        "controllerGeneration",
        "providerGeneration",
    ] {
        metadata.remove(field);
    }
    Ok(Some(value.to_canonical_bytes()))
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

pub(crate) fn integrity<T>(detail: T) -> StoreError
where
    T: core::fmt::Display + 'static,
{
    let reason = (&detail as &dyn std::any::Any)
        .downcast_ref::<&'static str>()
        .copied()
        .unwrap_or("redb-engine-failure");
    error(StoreErrorKind::StoreIntegrityFailure, None, reason)
}

pub(crate) fn integrity_reason(reason: &'static str) -> StoreError {
    error(StoreErrorKind::StoreIntegrityFailure, None, reason)
}

pub(crate) fn durability_failure(_detail: impl core::fmt::Display) -> StoreError {
    integrity_reason("redb-durability-failure")
}

pub(crate) fn quarantined() -> StoreError {
    quarantined_reason("redb-store-quarantined")
}

pub(crate) fn quarantined_reason(reason: &'static str) -> StoreError {
    error(StoreErrorKind::StoreQuarantined, None, reason)
}

pub(crate) fn unavailable(reason: &'static str) -> StoreError {
    error(StoreErrorKind::ResourcePlaneUnavailable, None, reason)
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
    use d2b_resource_store::{
        AdmittedAuthorizationTarget, AdmittedVerb, ResourceMutationKind, StoreSlot,
    };
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
            StoreSlot::new(0).unwrap(),
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
        let payload_digest = mutation.canonical_resource.as_deref().map(|bytes| {
            ResourceEnvelope::from_json(bytes)
                .unwrap()
                .digest()
                .unwrap()
        });
        let resource_uid = (mutation.kind != ResourceMutationKind::UpdateFinalizers).then_some(uid);
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
                resource_uid,
                prepared_payload_digest: payload_digest,
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

    fn create_mutation_with_uid(target: ResourceRef, uid: &ResourceUid) -> StoreMutation {
        let mut mutation = create_mutation(target);
        mutation.canonical_resource = Some(
            String::from_utf8(RESOURCE.to_vec())
                .unwrap()
                .replace("123e4567-e89b-42d3-a456-426614174000", uid.as_str())
                .into_bytes(),
        );
        mutation
    }

    fn stored_envelope(database: &Database, target: &ResourceRef) -> ResourceEnvelope {
        let read = database.begin_read().unwrap();
        let table = read.open_table(RESOURCES).unwrap();
        let value = table
            .get(resource_key(target).unwrap().as_slice())
            .unwrap()
            .unwrap();
        let record: ResourceRecord = decode(ValueKind::ResourceRecord, value.value()).unwrap();
        ResourceEnvelope::from_json(&record.canonical_json).unwrap()
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
        assert_eq!(outcome.batch.as_ref().unwrap().revision().get(), 1);

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
    fn prepared_uid_mismatch_cannot_replace_an_existing_resource() {
        let (_directory, database, _identity) = fixture();
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let requested_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        apply_group(
            &database,
            vec![verified(
                "seed-host",
                create_mutation(target.clone()),
                requested_uid,
            )],
        )
        .unwrap();
        let current_uid = stored_envelope(&database, &target).metadata().uid().clone();

        let prepared_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174099").unwrap();
        let mut update = create_mutation_with_uid(target.clone(), &prepared_uid);
        update.kind = ResourceMutationKind::UpdateSpec;
        update.expected = ExpectedRevision::Exact(ZoneRevision::new(1));
        update.expected_uid = Some(current_uid.clone());
        let outcome = apply_group(
            &database,
            vec![verified("prepared-uid-mismatch", update, prepared_uid)],
        )
        .unwrap();
        assert_eq!(
            outcome.results[0].as_ref().unwrap_err().reason_code(),
            "resource-uid-changed"
        );
        assert_eq!(
            stored_envelope(&database, &target).metadata().uid(),
            &current_uid
        );
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
                create_mutation_with_uid(target.clone(), &uid),
                uid.clone(),
            )],
        )
        .unwrap();
        let replay_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174099").unwrap();
        let replay = apply_group(
            &database,
            vec![verified(
                "idempotent-create",
                create_mutation_with_uid(target.clone(), &replay_uid),
                replay_uid,
            )],
        )
        .unwrap();
        assert!(replay.batch.is_none());
        assert_eq!(replay.results[0], first.results[0]);
        let persisted_digest = stored_envelope(&database, &target).digest().unwrap();
        assert_eq!(
            first.results[0].as_ref().unwrap().resources[0].payload_digest,
            persisted_digest
        );
        assert_eq!(
            replay.results[0].as_ref().unwrap().resources[0].payload_digest,
            persisted_digest
        );
        assert_eq!(current_meta(&database).unwrap().current_revision, 1);
    }

    #[test]
    fn status_update_preserves_spec_and_store_metadata() {
        let (_directory, database, _identity) = fixture();
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        apply_group(
            &database,
            vec![verified(
                "seed-host",
                create_mutation(target.clone()),
                uid.clone(),
            )],
        )
        .unwrap();
        let before = stored_envelope(&database, &target);
        let uid = before.metadata().uid().clone();
        let mut body = CanonicalJsonValue::parse(RESOURCE).unwrap();
        let CanonicalJsonValue::Object(root) = &mut body else {
            unreachable!()
        };
        let CanonicalJsonValue::Object(metadata) = root.get_mut("metadata").unwrap() else {
            unreachable!()
        };
        metadata.insert(
            "uid".to_owned(),
            CanonicalJsonValue::String(uid.as_str().to_owned()),
        );
        root.insert(
            "spec".to_owned(),
            CanonicalJsonValue::Object(Default::default()),
        );
        let mut update = create_mutation(target.clone());
        update.kind = ResourceMutationKind::UpdateStatus;
        update.expected = ExpectedRevision::Exact(ZoneRevision::new(1));
        update.expected_uid = Some(uid.clone());
        update.canonical_resource = Some(body.to_canonical_bytes());
        apply_group(&database, vec![verified("update-status", update, uid)])
            .unwrap()
            .results[0]
            .as_ref()
            .unwrap();
        let after = stored_envelope(&database, &target);

        assert_eq!(after.spec(), before.spec());
        assert_eq!(after.metadata().uid(), before.metadata().uid());
        assert_eq!(
            after.metadata().generation(),
            before.metadata().generation()
        );
        assert_eq!(after.metadata().revision(), ZoneRevision::new(2));
    }

    #[test]
    fn finalizer_delta_and_two_step_delete_preserve_resource_until_clear() {
        let (_directory, database, _identity) = fixture();
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        apply_group(
            &database,
            vec![verified(
                "seed-host",
                create_mutation(target.clone()),
                uid.clone(),
            )],
        )
        .unwrap();
        let uid = stored_envelope(&database, &target).metadata().uid().clone();
        let finalizer = FinalizerId::parse("core.cleanup").unwrap();
        let mut add = create_mutation(target.clone());
        add.kind = ResourceMutationKind::UpdateFinalizers;
        add.expected = ExpectedRevision::Exact(ZoneRevision::new(1));
        add.expected_uid = Some(uid.clone());
        add.canonical_resource = None;
        add.add_finalizers = vec![finalizer.clone()];
        apply_group(&database, vec![verified("add-finalizer", add, uid.clone())])
            .unwrap()
            .results[0]
            .as_ref()
            .unwrap();
        assert!(
            has_finalizers(
                &stored_envelope(&database, &target)
                    .canonical_bytes()
                    .unwrap()
            )
            .unwrap()
        );

        let mut delete = create_mutation(target.clone());
        delete.kind = ResourceMutationKind::Delete;
        delete.expected = ExpectedRevision::Exact(ZoneRevision::new(2));
        delete.expected_uid = Some(uid.clone());
        delete.canonical_resource = None;
        apply_group(
            &database,
            vec![verified("request-delete", delete, uid.clone())],
        )
        .unwrap();
        let requested = stored_envelope(&database, &target);
        assert!(deletion_requested(&requested.canonical_bytes().unwrap()).unwrap());

        let mut blocked = create_mutation(target.clone());
        blocked.kind = ResourceMutationKind::Delete;
        blocked.expected = ExpectedRevision::Exact(ZoneRevision::new(3));
        blocked.expected_uid = Some(uid.clone());
        blocked.canonical_resource = None;
        let outcome = apply_group(
            &database,
            vec![verified("blocked-delete", blocked, uid.clone())],
        )
        .unwrap();
        assert_eq!(
            outcome.results[0].as_ref().unwrap_err().kind(),
            StoreErrorKind::ResourceFinalizerDenied
        );

        let mut remove = create_mutation(target.clone());
        remove.kind = ResourceMutationKind::UpdateFinalizers;
        remove.expected = ExpectedRevision::Exact(ZoneRevision::new(3));
        remove.expected_uid = Some(uid.clone());
        remove.canonical_resource = None;
        remove.remove_finalizers = vec![finalizer];
        apply_group(
            &database,
            vec![verified("remove-finalizer", remove, uid.clone())],
        )
        .unwrap();
        let mut finish = create_mutation(target.clone());
        finish.kind = ResourceMutationKind::Delete;
        finish.expected = ExpectedRevision::Exact(ZoneRevision::new(4));
        finish.expected_uid = Some(uid.clone());
        finish.canonical_resource = None;
        apply_group(&database, vec![verified("finish-delete", finish, uid)]).unwrap();
        let read = database.begin_read().unwrap();
        assert!(
            read.open_table(RESOURCES)
                .unwrap()
                .get(resource_key(&target).unwrap().as_slice())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn finalizer_only_update_uses_stored_uid_without_prepared_uid() {
        let (_directory, database, _identity) = fixture();
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let requested_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        apply_group(
            &database,
            vec![verified(
                "seed-finalizer-target",
                create_mutation(target.clone()),
                requested_uid,
            )],
        )
        .unwrap();
        let stored_uid = stored_envelope(&database, &target).metadata().uid().clone();

        let mut finalizer = create_mutation(target.clone());
        finalizer.kind = ResourceMutationKind::UpdateFinalizers;
        finalizer.expected = ExpectedRevision::Exact(ZoneRevision::new(1));
        finalizer.canonical_resource = None;
        finalizer.add_finalizers = vec![FinalizerId::parse("core.cleanup").unwrap()];
        let request = verified(
            "finalizer-without-prepared-uid",
            finalizer,
            stored_uid.clone(),
        );
        assert!(request.mutations[0].resource_uid.is_none());

        let result = apply_group(&database, vec![request])
            .unwrap()
            .results
            .remove(0)
            .unwrap();
        assert_eq!(result.resources[0].uid, stored_uid);
        assert!(has_finalizers(&result.resources[0].canonical_json).unwrap());
    }

    #[test]
    fn operation_digest_covers_expected_uid_and_finalizer_delta() {
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let mut first = create_mutation(target.clone());
        first.kind = ResourceMutationKind::UpdateFinalizers;
        first.expected = ExpectedRevision::Exact(ZoneRevision::new(1));
        first.expected_uid = Some(uid.clone());
        first.canonical_resource = None;
        first.add_finalizers = vec![FinalizerId::parse("core.first").unwrap()];
        let first = verified("same-operation", first, uid.clone());
        let mut changed_uid = create_mutation(target.clone());
        changed_uid.kind = ResourceMutationKind::UpdateFinalizers;
        changed_uid.expected = ExpectedRevision::Exact(ZoneRevision::new(1));
        changed_uid.expected_uid =
            Some(ResourceUid::parse("123e4567-e89b-42d3-a456-426614174099").unwrap());
        changed_uid.canonical_resource = None;
        changed_uid.add_finalizers = vec![FinalizerId::parse("core.first").unwrap()];
        let changed_uid = verified("same-operation", changed_uid, uid.clone());
        let mut changed_delta = create_mutation(target);
        changed_delta.kind = ResourceMutationKind::UpdateFinalizers;
        changed_delta.expected = ExpectedRevision::Exact(ZoneRevision::new(1));
        changed_delta.expected_uid = Some(uid.clone());
        changed_delta.canonical_resource = None;
        changed_delta.add_finalizers = vec![FinalizerId::parse("core.second").unwrap()];
        let changed_delta = verified("same-operation", changed_delta, uid);

        assert_ne!(
            operation_digest(&first).unwrap(),
            operation_digest(&changed_uid).unwrap()
        );
        assert_ne!(
            operation_digest(&first).unwrap(),
            operation_digest(&changed_delta).unwrap()
        );
    }

    #[test]
    fn create_operation_digest_ignores_sealed_uid_but_detects_caller_input_changes() {
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let first_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let retry_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174099").unwrap();
        let first = verified(
            "same-create-operation",
            create_mutation_with_uid(target.clone(), &first_uid),
            first_uid,
        );
        let retry = verified(
            "same-create-operation",
            create_mutation_with_uid(target.clone(), &retry_uid),
            retry_uid.clone(),
        );
        let mut changed_mutation = create_mutation_with_uid(target, &retry_uid);
        changed_mutation.canonical_resource = Some(
            String::from_utf8(changed_mutation.canonical_resource.take().unwrap())
                .unwrap()
                .replace(
                    "\"nonDisruptive\":\"automatic\"",
                    "\"nonDisruptive\":\"manual\"",
                )
                .into_bytes(),
        );
        let changed = verified("same-create-operation", changed_mutation, retry_uid);

        assert_eq!(
            operation_digest(&first).unwrap(),
            operation_digest(&retry).unwrap()
        );
        assert_ne!(
            operation_digest(&first).unwrap(),
            operation_digest(&changed).unwrap()
        );
    }

    #[test]
    fn producer_index_uses_producer_uid_as_its_first_component() {
        let (_directory, database, _identity) = fixture();
        let producer_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174010").unwrap();
        let endpoint_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174011").unwrap();
        let key = encode_key(
            KeySpace::ProducerIndex,
            &[
                KeyComponent::Text(producer_uid.as_str()),
                KeyComponent::Text(endpoint_uid.as_str()),
            ],
        )
        .unwrap();
        let value = encode(
            ValueKind::ProducerIndexRecord,
            &ProducerIndexRecord {
                endpoint_type: "Endpoint".to_owned(),
                endpoint_name: "worker".to_owned(),
            },
        )
        .unwrap();
        let write = database.begin_write().unwrap();
        write
            .open_table(PRODUCER_INDEX)
            .unwrap()
            .insert(key.as_bytes(), value.as_slice())
            .unwrap();
        write.commit().unwrap();
        let write = database.begin_write().unwrap();
        assert!(produced_endpoints_remain(&write, &producer_uid).unwrap());
        assert!(!produced_endpoints_remain(&write, &endpoint_uid).unwrap());
        write.abort().unwrap();
    }

    #[test]
    fn endpoint_producer_ref_is_required_and_strict() {
        let mut value = CanonicalJsonValue::parse(RESOURCE).unwrap();
        let CanonicalJsonValue::Object(root) = &mut value else {
            unreachable!()
        };
        root.insert(
            "type".to_owned(),
            CanonicalJsonValue::String("Endpoint".to_owned()),
        );
        {
            let CanonicalJsonValue::Object(spec) = root.get_mut("spec").unwrap() else {
                unreachable!()
            };
            spec.remove("providerRef");
            spec.remove("updatePolicy");
        }
        let missing = ResourceEnvelope::from_json(&value.to_canonical_bytes()).unwrap();
        assert_eq!(
            endpoint_producer(&missing).unwrap_err().reason_code(),
            "endpoint-producer-ref-missing"
        );
        let CanonicalJsonValue::Object(root) = &mut value else {
            unreachable!()
        };
        let CanonicalJsonValue::Object(spec) = root.get_mut("spec").unwrap() else {
            unreachable!()
        };
        spec.insert(
            "producerRef".to_owned(),
            CanonicalJsonValue::String("not-a-ref".to_owned()),
        );
        let malformed = ResourceEnvelope::from_json(&value.to_canonical_bytes()).unwrap();
        assert_eq!(
            endpoint_producer(&malformed).unwrap_err().reason_code(),
            "endpoint-producer-ref-invalid"
        );
    }

    #[test]
    fn active_schema_rejects_unknown_base_fields_before_mutation() {
        let (_directory, database, _identity) = fixture();
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        apply_group(
            &database,
            vec![verified("seed-host", create_mutation(target.clone()), uid)],
        )
        .unwrap();
        let uid = stored_envelope(&database, &target).metadata().uid().clone();
        let mut value = CanonicalJsonValue::parse(RESOURCE).unwrap();
        let CanonicalJsonValue::Object(root) = &mut value else {
            unreachable!()
        };
        let CanonicalJsonValue::Object(metadata) = root.get_mut("metadata").unwrap() else {
            unreachable!()
        };
        metadata.insert(
            "uid".to_owned(),
            CanonicalJsonValue::String(uid.as_str().to_owned()),
        );
        let CanonicalJsonValue::Object(spec) = root.get_mut("spec").unwrap() else {
            unreachable!()
        };
        spec.insert(
            "unknownHostField".to_owned(),
            CanonicalJsonValue::Bool(true),
        );
        let mut update = create_mutation(target);
        update.kind = ResourceMutationKind::UpdateSpec;
        update.expected = ExpectedRevision::Exact(ZoneRevision::new(1));
        update.expected_uid = Some(uid.clone());
        update.canonical_resource = Some(value.to_canonical_bytes());
        let error = apply_group(&database, vec![verified("bad-schema", update, uid)]).unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::ResourceSchemaInvalid);
        assert_eq!(current_meta(&database).unwrap().current_revision, 1);
    }

    #[test]
    fn change_log_types_reject_unknown_events_zero_generations_and_oversize_batches() {
        assert!(serde_json::from_str::<ChangeEvent>("\"invented\"").is_err());
        assert!(serde_json::from_str::<ResourceGeneration>("0").is_err());
        assert!(ChangeBatch::new(ZoneRevision::new(0), Vec::new()).is_err());
        let entry = ChangeEntry::new(
            0,
            ResourceTypeName::parse("Host").unwrap(),
            ResourceName::parse("host-system").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ChangeEvent::Created,
            None,
            Some(ResourceGeneration::new(1).unwrap()),
            None,
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
            None,
            "op".to_owned(),
            "corr".to_owned(),
        )
        .unwrap();
        let mut entries = vec![
            entry.clone();
            crate::GROUP_COMMIT_MAX * d2b_contracts::v3::MAX_BATCH_MUTATIONS + 1
        ];
        for (ordinal, entry) in entries.iter_mut().enumerate() {
            entry.ordinal = u32::try_from(ordinal).unwrap();
        }
        assert!(ChangeBatch::new(ZoneRevision::new(1), entries).is_err());
    }

    #[test]
    fn recovery_rejects_derived_resource_drift_and_invalid_auxiliary_tables() {
        let (_directory, database, _identity) = fixture();
        let target = ResourceRef::parse("Host/host-system").unwrap();
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        apply_group(
            &database,
            vec![verified("seed-host", create_mutation(target.clone()), uid)],
        )
        .unwrap();
        let write = database.begin_write().unwrap();
        let mut resources = write.open_table(RESOURCES).unwrap();
        let key = resource_key(&target).unwrap();
        let current = resources.get(key.as_slice()).unwrap().unwrap();
        let mut record: ResourceRecord =
            decode(ValueKind::ResourceRecord, current.value()).unwrap();
        record.payload_digest =
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned();
        drop(current);
        resources
            .insert(
                key.as_slice(),
                encode(ValueKind::ResourceRecord, &record)
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
        drop(resources);
        write.commit().unwrap();
        assert_eq!(
            validate_consistency(&database).unwrap_err().reason_code(),
            "stored-resource-identity-invalid"
        );

        let (_directory, database, identity) = fixture();
        let write = database.begin_write().unwrap();
        let schema_key = encode_key(KeySpace::ApiSchemas, &[KeyComponent::Text("Host")]).unwrap();
        let schema = encode(
            ValueKind::ApiSchemaRecord,
            &serde_json::json!({
                "resourceType": "Guest",
                "validatorFingerprint": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            }),
        )
        .unwrap();
        write
            .open_table(API_SCHEMAS)
            .unwrap()
            .insert(schema_key.as_bytes(), schema.as_slice())
            .unwrap();
        write.commit().unwrap();
        assert_eq!(
            validate_consistency(&database).unwrap_err().reason_code(),
            "api-schema-record-invalid"
        );
        assert_eq!(
            validate_identity(&database, &identity).unwrap().zone_name,
            "dev"
        );
    }
}
