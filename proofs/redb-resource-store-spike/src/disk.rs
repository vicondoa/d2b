use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use redb::backends::FileBackend;
use redb::{
    Database, Durability, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition,
    TableHandle,
};
use serde::{Deserialize, Serialize};

use crate::codec::{KeyPart, decode, key, value};
use crate::model::{
    ChangeBatch, ChangeEntry, Mutation, OracleCheckpoint, Resource, ResourceKey, StoreError,
    StoreResult,
};
use crate::schema::{
    API_SCHEMAS, CONTROLLER_INDEX, OPERATIONS, OWNER_INDEX, PRODUCER_INDEX, RESOURCES,
    REVISION_LOG, STORE_META, TABLES, TYPE_INDEX, ZONE_LINK_CURSORS,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreMetaRecord {
    schema_version: u32,
    current_revision: u64,
    sentinel: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct OwnerIndexRecord {
    resource_type: String,
    name: String,
    latest_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProducerIndexRecord {
    endpoint_type: String,
    endpoint_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OperationRecord {
    resource_uid: String,
    accepted_revision: u64,
    outcome: String,
}

#[derive(Debug)]
pub(crate) struct DiskReceipt {
    pub resource: Resource,
    pub ordinal: u16,
}

#[derive(Debug)]
pub(crate) struct AppliedGroup {
    pub revision: Option<u64>,
    pub receipts: Vec<StoreResult<DiskReceipt>>,
    pub batch: Option<ChangeBatch>,
}

/// Bounded backend replay signals for one registration scan.
///
/// Fixed cardinality: three counters per scan, no per-watch or per-revision
/// labels, so accumulating them across registrations cannot grow unbounded.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ReplayScan {
    pub range_seeks: u64,
    pub rows_scanned: u64,
    pub rows_decoded: u64,
}

pub(crate) struct DiskStore {
    database: Database,
}

fn integrity(error: impl std::fmt::Display) -> StoreError {
    StoreError::Integrity(error.to_string())
}

fn open_backend(path: &Path, create: bool) -> StoreResult<Database> {
    let file = if create {
        OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path)
            .map_err(integrity)?
    } else {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(integrity)?
    };
    if !create && file.metadata().map_err(integrity)?.len() == 0 {
        return Err(StoreError::Integrity("empty-database-file".to_owned()));
    }
    let backend = FileBackend::new(file).map_err(integrity)?;
    Database::builder()
        .create_with_backend(backend)
        .map_err(integrity)
}

impl DiskStore {
    pub(crate) fn open_or_create(path: &Path) -> StoreResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(integrity)?;
        }
        let create = !path.exists();
        let database = open_backend(path, create)?;
        let store = Self { database };
        if create {
            store.initialize()?;
        } else {
            store.validate_meta()?;
        }
        Ok(store)
    }

    fn open_existing(path: &Path) -> StoreResult<Self> {
        let store = Self {
            database: open_backend(path, false)?,
        };
        store.validate_meta()?;
        Ok(store)
    }

    fn initialize(&self) -> StoreResult<()> {
        let mut write = self.database.begin_write().map_err(integrity)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(integrity)?;
        for (definition, _, _) in TABLES {
            drop(write.open_table(definition).map_err(integrity)?);
        }
        {
            let mut meta = write.open_table(STORE_META).map_err(integrity)?;
            let meta_key = key(0x01, &[KeyPart::Text("store")]);
            let meta_value = value(
                0x0001,
                &StoreMetaRecord {
                    schema_version: 1,
                    current_revision: 0,
                    sentinel: "redb-resource-store-spike".to_owned(),
                },
            )?;
            meta.insert(meta_key.as_slice(), meta_value.as_slice())
                .map_err(integrity)?;
        }
        {
            let mut schemas = write.open_table(API_SCHEMAS).map_err(integrity)?;
            for resource_type in ["Process", "Endpoint", "Volume", "Device", "Guest", "Policy"] {
                let schema_key = key(0x02, &[KeyPart::Text(resource_type)]);
                let schema_value = value(
                    0x0002,
                    &serde_json::json!({
                        "resourceType": resource_type,
                        "validatorFingerprint": format!("sha256:{resource_type}")
                    }),
                )?;
                schemas
                    .insert(schema_key.as_slice(), schema_value.as_slice())
                    .map_err(integrity)?;
            }
        }
        {
            let mut cursors = write.open_table(ZONE_LINK_CURSORS).map_err(integrity)?;
            let cursor_key = key(0x0a, &[KeyPart::Text("peer-zone")]);
            let cursor_value = value(
                0x000a,
                &serde_json::json!({
                    "linkEpoch": 1,
                    "sent": 0,
                    "acked": 0,
                    "received": 0,
                    "applied": 0
                }),
            )?;
            cursors
                .insert(cursor_key.as_slice(), cursor_value.as_slice())
                .map_err(integrity)?;
        }
        write.commit().map_err(integrity)
    }

    fn validate_meta(&self) -> StoreResult<()> {
        let meta = self.meta()?;
        let read = self.database.begin_read().map_err(integrity)?;
        if meta.schema_version != 1
            || meta.sentinel != "redb-resource-store-spike"
            || read.list_tables().map_err(integrity)?.count() != 10
        {
            return Err(StoreError::Integrity(
                "unknown-or-incomplete-physical-schema".to_owned(),
            ));
        }
        Ok(())
    }

    fn meta(&self) -> StoreResult<StoreMetaRecord> {
        let read = self.database.begin_read().map_err(integrity)?;
        let table = read.open_table(STORE_META).map_err(integrity)?;
        let meta_key = key(0x01, &[KeyPart::Text("store")]);
        let bytes = table
            .get(meta_key.as_slice())
            .map_err(integrity)?
            .ok_or_else(|| StoreError::Integrity("missing-store-meta".to_owned()))?;
        decode(0x0001, bytes.value())
    }

    pub(crate) fn current_revision(&self) -> StoreResult<u64> {
        Ok(self.meta()?.current_revision)
    }

    fn resource_in_write(
        write: &redb::WriteTransaction,
        resource_key: &ResourceKey,
    ) -> StoreResult<Option<Resource>> {
        let table = write.open_table(RESOURCES).map_err(integrity)?;
        let encoded_key = resource_key_bytes(resource_key);
        table
            .get(encoded_key.as_slice())
            .map_err(integrity)?
            .map(|bytes| decode(0x0003, bytes.value()))
            .transpose()
    }

    pub(crate) fn apply_group(&self, mutations: &[Mutation]) -> StoreResult<AppliedGroup> {
        self.apply_group_with_commit_hook(mutations, || {})
    }

    fn apply_group_with_commit_hook(
        &self,
        mutations: &[Mutation],
        before_commit: impl FnOnce(),
    ) -> StoreResult<AppliedGroup> {
        let mut write = self.database.begin_write().map_err(integrity)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(integrity)?;

        let mut accepted = Vec::new();
        let mut results = Vec::with_capacity(mutations.len());
        for mutation in mutations {
            let current = Self::resource_in_write(&write, &mutation.resource.key)?;
            let current_revision = current.as_ref().map_or(0, |resource| resource.revision);
            if current_revision != mutation.expected_revision {
                results.push(Err(StoreError::Conflict { current_revision }));
                continue;
            }
            if mutation.resource.payload_bytes() > 8 * 1024 {
                results.push(Err(StoreError::Integrity(
                    "synthetic-resource-payload-oversize".to_owned(),
                )));
                continue;
            }
            accepted.push((results.len(), mutation.clone(), current));
            results.push(Err(StoreError::Integrity(
                "unresolved-write-result".to_owned(),
            )));
        }

        if accepted.is_empty() {
            write.abort().map_err(integrity)?;
            return Ok(AppliedGroup {
                revision: None,
                receipts: results,
                batch: None,
            });
        }

        let revision = self.current_revision()? + 1;
        let mut entries = Vec::with_capacity(accepted.len());
        for (ordinal_index, (result_index, mutation, previous)) in accepted.iter().enumerate() {
            let mut resource = mutation.resource.clone();
            resource.revision = revision;
            if let Some(previous) = previous {
                remove_previous_indexes(&write, previous)?;
            }
            insert_resource_and_indexes(&write, &resource)?;
            {
                let mut operations = write.open_table(OPERATIONS).map_err(integrity)?;
                let operation_key = key(0x09, &[KeyPart::Text(&mutation.operation_id)]);
                let operation_value = value(
                    0x0009,
                    &OperationRecord {
                        resource_uid: resource.uid.clone(),
                        accepted_revision: revision,
                        outcome: "Committed".to_owned(),
                    },
                )?;
                operations
                    .insert(operation_key.as_slice(), operation_value.as_slice())
                    .map_err(integrity)?;
            }
            let ordinal = u16::try_from(ordinal_index).map_err(integrity)?;
            entries.push(ChangeEntry {
                ordinal,
                resource: resource.clone(),
                event: if previous.is_some() {
                    "SpecUpdated".to_owned()
                } else {
                    "Created".to_owned()
                },
                operation_id: mutation.operation_id.clone(),
            });
            results[*result_index] = Ok(DiskReceipt { resource, ordinal });
        }

        let batch = ChangeBatch { revision, entries };
        {
            let mut log = write.open_table(REVISION_LOG).map_err(integrity)?;
            let revision_key = key(0x08, &[KeyPart::Revision(revision)]);
            let revision_value = value(0x0008, &batch)?;
            log.insert(revision_key.as_slice(), revision_value.as_slice())
                .map_err(integrity)?;
        }
        {
            let mut meta = write.open_table(STORE_META).map_err(integrity)?;
            let meta_key = key(0x01, &[KeyPart::Text("store")]);
            let meta_value = value(
                0x0001,
                &StoreMetaRecord {
                    schema_version: 1,
                    current_revision: revision,
                    sentinel: "redb-resource-store-spike".to_owned(),
                },
            )?;
            meta.insert(meta_key.as_slice(), meta_value.as_slice())
                .map_err(integrity)?;
        }
        before_commit();
        write.commit().map_err(integrity)?;

        Ok(AppliedGroup {
            revision: Some(revision),
            receipts: results,
            batch: Some(batch),
        })
    }

    /// Streaming replay bounded by a revision-key range seek.
    ///
    /// The revision log key encodes the revision as big-endian bytes after a
    /// fixed header, so lexicographic key order equals numeric revision order.
    /// Seeking to `after_revision + 1` therefore means rows at or below
    /// `after_revision` are never read and never decoded. Each row in range is
    /// decoded one at a time and handed to `visit`, which is expected to
    /// consume or drop it, so no older complete envelope is ever materialized
    /// and the caller never holds the whole log at once.
    pub(crate) fn stream_revision_batches_after<F>(
        &self,
        after_revision: u64,
        mut visit: F,
    ) -> StoreResult<ReplayScan>
    where
        F: FnMut(ChangeBatch) -> StoreResult<()>,
    {
        let read = self.database.begin_read().map_err(integrity)?;
        let table = read.open_table(REVISION_LOG).map_err(integrity)?;
        let mut scan = ReplayScan {
            range_seeks: 1,
            rows_scanned: 0,
            rows_decoded: 0,
        };
        let lower = key(0x08, &[KeyPart::Revision(after_revision.saturating_add(1))]);
        let range = table
            .range(lower.as_slice()..)
            .map_err(integrity)?;
        for row in range {
            let (_, bytes) = row.map_err(integrity)?;
            scan.rows_scanned += 1;
            scan.rows_decoded += 1;
            let batch: ChangeBatch = decode(0x0008, bytes.value())?;
            visit(batch)?;
        }
        Ok(scan)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn revision_batches_after(
        &self,
        after_revision: u64,
    ) -> StoreResult<Vec<ChangeBatch>> {
        let mut batches = Vec::new();
        self.stream_revision_batches_after(after_revision, |batch| {
            batches.push(batch);
            Ok(())
        })?;
        Ok(batches)
    }

    pub(crate) fn uid_index(&self) -> StoreResult<BTreeMap<String, ResourceKey>> {
        Ok(self
            .snapshot()?
            .into_values()
            .map(|resource| (resource.uid, resource.key))
            .collect())
    }

    pub(crate) fn snapshot(&self) -> StoreResult<BTreeMap<ResourceKey, Resource>> {
        let read = self.database.begin_read().map_err(integrity)?;
        let table = read.open_table(RESOURCES).map_err(integrity)?;
        let mut resources = BTreeMap::new();
        for row in table.iter().map_err(integrity)? {
            let (_, bytes) = row.map_err(integrity)?;
            let resource: Resource = decode(0x0003, bytes.value())?;
            resources.insert(resource.key.clone(), resource);
        }
        Ok(resources)
    }

    pub(crate) fn verify(&self, oracle: &BTreeMap<ResourceKey, Resource>) -> StoreResult<()> {
        let actual = self.snapshot()?;
        if actual != *oracle {
            return Err(StoreError::Integrity(format!(
                "resource-oracle-divergence:actual={} expected={}",
                actual.len(),
                oracle.len()
            )));
        }

        let read = self.database.begin_read().map_err(integrity)?;
        let expected_resources = oracle
            .values()
            .map(|resource| Ok((resource_key_bytes(&resource.key), value(0x0003, resource)?)))
            .collect::<StoreResult<BTreeMap<_, _>>>()?;
        verify_raw_table(&read, RESOURCES, &expected_resources)?;

        let expected_type = oracle
            .values()
            .map(|resource| {
                Ok((
                    type_index_key_bytes(&resource.key),
                    value(0x0004, &resource.uid)?,
                ))
            })
            .collect::<StoreResult<BTreeMap<_, _>>>()?;
        verify_raw_table(&read, TYPE_INDEX, &expected_type)?;

        let expected_owner = oracle
            .values()
            .filter_map(|resource| {
                resource.owner_uid.as_ref().map(|owner_uid| {
                    Ok((
                        key(
                            0x05,
                            &[KeyPart::Text(owner_uid), KeyPart::Text(&resource.uid)],
                        ),
                        value(
                            0x0005,
                            &OwnerIndexRecord {
                                resource_type: resource.key.resource_type.clone(),
                                name: resource.key.name.clone(),
                                latest_revision: resource.revision,
                            },
                        )?,
                    ))
                })
            })
            .collect::<StoreResult<BTreeMap<_, _>>>()?;
        verify_raw_table(&read, OWNER_INDEX, &expected_owner)?;

        let expected_producer = oracle
            .values()
            .filter_map(|resource| {
                resource.producer_uid.as_ref().map(|producer_uid| {
                    Ok((
                        key(
                            0x06,
                            &[KeyPart::Text(producer_uid), KeyPart::Text(&resource.uid)],
                        ),
                        value(
                            0x0006,
                            &ProducerIndexRecord {
                                endpoint_type: resource.key.resource_type.clone(),
                                endpoint_name: resource.key.name.clone(),
                            },
                        )?,
                    ))
                })
            })
            .collect::<StoreResult<BTreeMap<_, _>>>()?;
        verify_raw_table(&read, PRODUCER_INDEX, &expected_producer)?;

        let expected_controller = oracle
            .values()
            .map(|resource| {
                Ok((
                    key(
                        0x07,
                        &[
                            KeyPart::Text(&resource.controller),
                            KeyPart::Text(&resource.key.resource_type),
                            KeyPart::Text(&resource.key.name),
                        ],
                    ),
                    value(0x0007, &resource.uid)?,
                ))
            })
            .collect::<StoreResult<BTreeMap<_, _>>>()?;
        verify_raw_table(&read, CONTROLLER_INDEX, &expected_controller)?;

        let revision = self.current_revision()?;
        let batches = self.revision_batches_after(0)?;
        if batches.len() != usize::try_from(revision).map_err(integrity)?
            || batches
                .iter()
                .enumerate()
                .any(|(index, batch)| batch.revision != u64::try_from(index + 1).unwrap())
        {
            return Err(StoreError::Integrity(
                "revision-log-is-not-contiguous".to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn verify_transition(&self, checkpoint: &OracleCheckpoint) -> StoreResult<()> {
        let read = self.database.begin_read().map_err(integrity)?;
        let expected_counts = [
            (RESOURCES, checkpoint.resource_count),
            (TYPE_INDEX, checkpoint.resource_count),
            (OWNER_INDEX, checkpoint.owner_count),
            (PRODUCER_INDEX, checkpoint.producer_count),
            (CONTROLLER_INDEX, checkpoint.resource_count),
            (OPERATIONS, checkpoint.operation_count),
            (REVISION_LOG, checkpoint.revision),
            (API_SCHEMAS, 6),
            (ZONE_LINK_CURSORS, 1),
            (STORE_META, 1),
        ];
        for (definition, expected) in expected_counts {
            let actual = read
                .open_table(definition)
                .map_err(integrity)?
                .len()
                .map_err(integrity)?;
            if actual != expected {
                return Err(StoreError::Integrity(format!(
                    "{}-count-divergence:actual={actual} expected={expected}",
                    definition.name()
                )));
            }
        }

        let resource = &checkpoint.changed_resource;
        verify_point_value(
            &read,
            RESOURCES,
            &resource_key_bytes(&resource.key),
            0x0003,
            resource,
        )?;
        verify_point_value(
            &read,
            TYPE_INDEX,
            &type_index_key_bytes(&resource.key),
            0x0004,
            &resource.uid,
        )?;
        verify_point_value(
            &read,
            CONTROLLER_INDEX,
            &key(
                0x07,
                &[
                    KeyPart::Text(&resource.controller),
                    KeyPart::Text(&resource.key.resource_type),
                    KeyPart::Text(&resource.key.name),
                ],
            ),
            0x0007,
            &resource.uid,
        )?;
        if let Some(owner_uid) = &resource.owner_uid {
            verify_point_value(
                &read,
                OWNER_INDEX,
                &key(
                    0x05,
                    &[KeyPart::Text(owner_uid), KeyPart::Text(&resource.uid)],
                ),
                0x0005,
                &OwnerIndexRecord {
                    resource_type: resource.key.resource_type.clone(),
                    name: resource.key.name.clone(),
                    latest_revision: resource.revision,
                },
            )?;
        }
        if let Some(producer_uid) = &resource.producer_uid {
            verify_point_value(
                &read,
                PRODUCER_INDEX,
                &key(
                    0x06,
                    &[KeyPart::Text(producer_uid), KeyPart::Text(&resource.uid)],
                ),
                0x0006,
                &ProducerIndexRecord {
                    endpoint_type: resource.key.resource_type.clone(),
                    endpoint_name: resource.key.name.clone(),
                },
            )?;
        }
        let revision_key = key(0x08, &[KeyPart::Revision(checkpoint.revision)]);
        let log = read.open_table(REVISION_LOG).map_err(integrity)?;
        let bytes = log
            .get(revision_key.as_slice())
            .map_err(integrity)?
            .ok_or_else(|| StoreError::Integrity("missing-current-revision".to_owned()))?;
        let batch: ChangeBatch = decode(0x0008, bytes.value())?;
        if batch.revision != checkpoint.revision
            || batch.entries.len() != 1
            || batch.entries[0].resource != *resource
        {
            return Err(StoreError::Integrity(
                "current-revision-batch-divergence".to_owned(),
            ));
        }
        if self.meta()?.current_revision != checkpoint.revision {
            return Err(StoreError::Integrity(
                "store-meta-revision-divergence".to_owned(),
            ));
        }
        Ok(())
    }
}

fn resource_key_bytes(resource_key: &ResourceKey) -> Vec<u8> {
    key(
        0x03,
        &[
            KeyPart::Text(&resource_key.resource_type),
            KeyPart::Text(&resource_key.name),
        ],
    )
}

fn type_index_key_bytes(resource_key: &ResourceKey) -> Vec<u8> {
    key(
        0x04,
        &[
            KeyPart::Text(&resource_key.resource_type),
            KeyPart::Text(&resource_key.name),
        ],
    )
}

fn remove_previous_indexes(write: &redb::WriteTransaction, previous: &Resource) -> StoreResult<()> {
    if let Some(owner_uid) = &previous.owner_uid {
        let mut table = write.open_table(OWNER_INDEX).map_err(integrity)?;
        let encoded = key(
            0x05,
            &[KeyPart::Text(owner_uid), KeyPart::Text(&previous.uid)],
        );
        table.remove(encoded.as_slice()).map_err(integrity)?;
    }
    if let Some(producer_uid) = &previous.producer_uid {
        let mut table = write.open_table(PRODUCER_INDEX).map_err(integrity)?;
        let encoded = key(
            0x06,
            &[KeyPart::Text(producer_uid), KeyPart::Text(&previous.uid)],
        );
        table.remove(encoded.as_slice()).map_err(integrity)?;
    }
    {
        let mut table = write.open_table(CONTROLLER_INDEX).map_err(integrity)?;
        let encoded = key(
            0x07,
            &[
                KeyPart::Text(&previous.controller),
                KeyPart::Text(&previous.key.resource_type),
                KeyPart::Text(&previous.key.name),
            ],
        );
        table.remove(encoded.as_slice()).map_err(integrity)?;
    }
    Ok(())
}

fn insert_resource_and_indexes(
    write: &redb::WriteTransaction,
    resource: &Resource,
) -> StoreResult<()> {
    let encoded_resource_key = resource_key_bytes(&resource.key);
    {
        let mut table = write.open_table(RESOURCES).map_err(integrity)?;
        let encoded_value = value(0x0003, resource)?;
        table
            .insert(encoded_resource_key.as_slice(), encoded_value.as_slice())
            .map_err(integrity)?;
    }
    {
        let mut table = write.open_table(TYPE_INDEX).map_err(integrity)?;
        let encoded_type_key = type_index_key_bytes(&resource.key);
        let encoded_value = value(0x0004, &resource.uid)?;
        table
            .insert(encoded_type_key.as_slice(), encoded_value.as_slice())
            .map_err(integrity)?;
    }
    if let Some(owner_uid) = &resource.owner_uid {
        let mut table = write.open_table(OWNER_INDEX).map_err(integrity)?;
        let encoded_key = key(
            0x05,
            &[KeyPart::Text(owner_uid), KeyPart::Text(&resource.uid)],
        );
        let encoded_value = value(
            0x0005,
            &OwnerIndexRecord {
                resource_type: resource.key.resource_type.clone(),
                name: resource.key.name.clone(),
                latest_revision: resource.revision,
            },
        )?;
        table
            .insert(encoded_key.as_slice(), encoded_value.as_slice())
            .map_err(integrity)?;
    }
    if let Some(producer_uid) = &resource.producer_uid {
        let mut table = write.open_table(PRODUCER_INDEX).map_err(integrity)?;
        let encoded_key = key(
            0x06,
            &[KeyPart::Text(producer_uid), KeyPart::Text(&resource.uid)],
        );
        let encoded_value = value(
            0x0006,
            &ProducerIndexRecord {
                endpoint_type: resource.key.resource_type.clone(),
                endpoint_name: resource.key.name.clone(),
            },
        )?;
        table
            .insert(encoded_key.as_slice(), encoded_value.as_slice())
            .map_err(integrity)?;
    }
    {
        let mut table = write.open_table(CONTROLLER_INDEX).map_err(integrity)?;
        let encoded_key = key(
            0x07,
            &[
                KeyPart::Text(&resource.controller),
                KeyPart::Text(&resource.key.resource_type),
                KeyPart::Text(&resource.key.name),
            ],
        );
        let encoded_value = value(0x0007, &resource.uid)?;
        table
            .insert(encoded_key.as_slice(), encoded_value.as_slice())
            .map_err(integrity)?;
    }
    Ok(())
}

fn verify_raw_table(
    read: &redb::ReadTransaction,
    definition: TableDefinition<'static, &[u8], &[u8]>,
    expected: &BTreeMap<Vec<u8>, Vec<u8>>,
) -> StoreResult<()> {
    let table = read.open_table(definition).map_err(integrity)?;
    let actual = table
        .iter()
        .map_err(integrity)?
        .map(|row| {
            row.map(|(raw_key, raw_value)| (raw_key.value().to_vec(), raw_value.value().to_vec()))
                .map_err(integrity)
        })
        .collect::<StoreResult<BTreeMap<_, _>>>()?;
    if actual != *expected {
        return Err(StoreError::Integrity(format!(
            "{}-index-divergence:actual={} expected={}",
            definition.name(),
            actual.len(),
            expected.len()
        )));
    }
    Ok(())
}

fn verify_point_value<T>(
    read: &redb::ReadTransaction,
    definition: TableDefinition<'static, &[u8], &[u8]>,
    encoded_key: &[u8],
    value_kind: u16,
    expected: &T,
) -> StoreResult<()>
where
    T: serde::de::DeserializeOwned + PartialEq,
{
    let table = read.open_table(definition).map_err(integrity)?;
    let bytes = table
        .get(encoded_key)
        .map_err(integrity)?
        .ok_or_else(|| StoreError::Integrity(format!("missing-{}", definition.name())))?;
    let actual: T = decode(value_kind, bytes.value())?;
    if actual != *expected {
        return Err(StoreError::Integrity(format!(
            "{}-point-divergence",
            definition.name()
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashRecovery {
    LastCommittedState,
    NewCommittedState,
    RefusedToOpen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CrashCheckpoint {
    resources: BTreeMap<Vec<u8>, Vec<u8>>,
    type_index: BTreeMap<Vec<u8>, Vec<u8>>,
    owner_index: BTreeMap<Vec<u8>, Vec<u8>>,
    producer_index: BTreeMap<Vec<u8>, Vec<u8>>,
    controller_index: BTreeMap<Vec<u8>, Vec<u8>>,
    operations: BTreeMap<Vec<u8>, Vec<u8>>,
    revision_log: BTreeMap<Vec<u8>, Vec<u8>>,
    store_meta: BTreeMap<Vec<u8>, Vec<u8>>,
}

#[derive(Debug)]
pub struct CrashCheckpoints {
    old: CrashCheckpoint,
    new: CrashCheckpoint,
}

pub fn crash_database_path(boundary: u8) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("spike-data")
        .join(format!("crash-{}-{boundary}.redb", std::process::id()))
}

pub fn prepare_crash_database(path: &Path) -> StoreResult<CrashCheckpoints> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(integrity)?;
    }
    let _ = std::fs::remove_file(path);
    let store = DiskStore::open_or_create(path)?;
    let baseline = crate::model::synthetic_resource(0);
    let outcome = store.apply_group(&[Mutation::create(baseline)])?;
    if outcome.revision != Some(1) {
        return Err(StoreError::Integrity(
            "failed-to-create-crash-baseline".to_owned(),
        ));
    }
    let old = CrashCheckpoint::capture(&store)?;
    let new = old.with_crash_transaction()?;
    Ok(CrashCheckpoints { old, new })
}

fn kill_at(boundary: u8, selected: u8) -> StoreResult<()> {
    if boundary != selected {
        return Ok(());
    }
    kill(Pid::this(), Signal::SIGKILL).map_err(integrity)?;
    Err(StoreError::Integrity(
        "SIGKILL-did-not-terminate-process".to_owned(),
    ))
}

pub fn run_crash_transaction(path: &Path, selected: u8) -> StoreResult<()> {
    if !(1..=13).contains(&selected) {
        return Err(StoreError::Integrity(
            "kill-boundary-out-of-range".to_owned(),
        ));
    }
    let store = DiskStore::open_existing(path)?;
    kill_at(1, selected)?;
    let mut write = store.database.begin_write().map_err(integrity)?;
    write
        .set_durability(Durability::Immediate)
        .map_err(integrity)?;
    kill_at(2, selected)?;
    let _policy_generation = 1_u64;
    kill_at(3, selected)?;
    let mut candidate = crate::model::synthetic_resource(10_001);
    let current = DiskStore::resource_in_write(&write, &candidate.key)?;
    kill_at(4, selected)?;
    if current.is_some() || candidate.payload_bytes() > 8 * 1024 {
        return Err(StoreError::Integrity(
            "crash-candidate-validation-failed".to_owned(),
        ));
    }
    kill_at(5, selected)?;
    let expected_revision = 0_u64;
    if current.as_ref().map_or(0, |resource| resource.revision) != expected_revision {
        return Err(StoreError::Conflict {
            current_revision: current.map_or(0, |resource| resource.revision),
        });
    }
    kill_at(6, selected)?;
    candidate.revision = 2;
    insert_resource_and_indexes(&write, &candidate)?;
    kill_at(7, selected)?;
    {
        let mut operations = write.open_table(OPERATIONS).map_err(integrity)?;
        let operation_key = key(0x09, &[KeyPart::Text("crash-operation")]);
        let operation_value = value(
            0x0009,
            &OperationRecord {
                resource_uid: candidate.uid.clone(),
                accepted_revision: 2,
                outcome: "Committed".to_owned(),
            },
        )?;
        operations
            .insert(operation_key.as_slice(), operation_value.as_slice())
            .map_err(integrity)?;
    }
    kill_at(8, selected)?;
    let revision = store.current_revision()? + 1;
    if revision != 2 {
        return Err(StoreError::Integrity(
            "unexpected-crash-revision".to_owned(),
        ));
    }
    kill_at(9, selected)?;
    let batch = ChangeBatch {
        revision,
        entries: vec![ChangeEntry {
            ordinal: 0,
            resource: candidate,
            event: "Created".to_owned(),
            operation_id: "crash-operation".to_owned(),
        }],
    };
    {
        let mut log = write.open_table(REVISION_LOG).map_err(integrity)?;
        let revision_key = key(0x08, &[KeyPart::Revision(revision)]);
        let revision_value = value(0x0008, &batch)?;
        log.insert(revision_key.as_slice(), revision_value.as_slice())
            .map_err(integrity)?;
    }
    kill_at(10, selected)?;
    {
        let mut meta = write.open_table(STORE_META).map_err(integrity)?;
        let meta_key = key(0x01, &[KeyPart::Text("store")]);
        let meta_value = value(
            0x0001,
            &StoreMetaRecord {
                schema_version: 1,
                current_revision: revision,
                sentinel: "redb-resource-store-spike".to_owned(),
            },
        )?;
        meta.insert(meta_key.as_slice(), meta_value.as_slice())
            .map_err(integrity)?;
    }
    kill_at(11, selected)?;
    write.commit().map_err(integrity)?;
    kill_at(12, selected)?;
    let _in_memory_index_swap_and_dispatch = batch;
    kill_at(13, selected)?;
    Ok(())
}

pub fn verify_crash_database(
    path: &Path,
    expected: &CrashCheckpoints,
) -> StoreResult<CrashRecovery> {
    let store = match DiskStore::open_existing(path) {
        Ok(store) => store,
        Err(_) => return Ok(CrashRecovery::RefusedToOpen),
    };
    let actual = CrashCheckpoint::capture(&store)?;
    if actual == expected.old {
        Ok(CrashRecovery::LastCommittedState)
    } else if actual == expected.new {
        Ok(CrashRecovery::NewCommittedState)
    } else {
        Err(StoreError::Integrity(
            "recovered-state-matches-neither-full-checkpoint".to_owned(),
        ))
    }
}

impl CrashCheckpoint {
    fn capture(store: &DiskStore) -> StoreResult<Self> {
        let read = store.database.begin_read().map_err(integrity)?;
        Ok(Self {
            resources: raw_table(&read, RESOURCES)?,
            type_index: raw_table(&read, TYPE_INDEX)?,
            owner_index: raw_table(&read, OWNER_INDEX)?,
            producer_index: raw_table(&read, PRODUCER_INDEX)?,
            controller_index: raw_table(&read, CONTROLLER_INDEX)?,
            operations: raw_table(&read, OPERATIONS)?,
            revision_log: raw_table(&read, REVISION_LOG)?,
            store_meta: raw_table(&read, STORE_META)?,
        })
    }

    fn with_crash_transaction(&self) -> StoreResult<Self> {
        let mut next = self.clone();
        let mut candidate = crate::model::synthetic_resource(10_001);
        candidate.revision = 2;
        next.resources.insert(
            resource_key_bytes(&candidate.key),
            value(0x0003, &candidate)?,
        );
        next.type_index.insert(
            type_index_key_bytes(&candidate.key),
            value(0x0004, &candidate.uid)?,
        );
        if let Some(owner_uid) = &candidate.owner_uid {
            next.owner_index.insert(
                key(
                    0x05,
                    &[KeyPart::Text(owner_uid), KeyPart::Text(&candidate.uid)],
                ),
                value(
                    0x0005,
                    &OwnerIndexRecord {
                        resource_type: candidate.key.resource_type.clone(),
                        name: candidate.key.name.clone(),
                        latest_revision: candidate.revision,
                    },
                )?,
            );
        }
        if let Some(producer_uid) = &candidate.producer_uid {
            next.producer_index.insert(
                key(
                    0x06,
                    &[KeyPart::Text(producer_uid), KeyPart::Text(&candidate.uid)],
                ),
                value(
                    0x0006,
                    &ProducerIndexRecord {
                        endpoint_type: candidate.key.resource_type.clone(),
                        endpoint_name: candidate.key.name.clone(),
                    },
                )?,
            );
        }
        next.controller_index.insert(
            key(
                0x07,
                &[
                    KeyPart::Text(&candidate.controller),
                    KeyPart::Text(&candidate.key.resource_type),
                    KeyPart::Text(&candidate.key.name),
                ],
            ),
            value(0x0007, &candidate.uid)?,
        );
        next.operations.insert(
            key(0x09, &[KeyPart::Text("crash-operation")]),
            value(
                0x0009,
                &OperationRecord {
                    resource_uid: candidate.uid.clone(),
                    accepted_revision: 2,
                    outcome: "Committed".to_owned(),
                },
            )?,
        );
        let batch = ChangeBatch {
            revision: 2,
            entries: vec![ChangeEntry {
                ordinal: 0,
                resource: candidate,
                event: "Created".to_owned(),
                operation_id: "crash-operation".to_owned(),
            }],
        };
        next.revision_log
            .insert(key(0x08, &[KeyPart::Revision(2)]), value(0x0008, &batch)?);
        next.store_meta.insert(
            key(0x01, &[KeyPart::Text("store")]),
            value(
                0x0001,
                &StoreMetaRecord {
                    schema_version: 1,
                    current_revision: 2,
                    sentinel: "redb-resource-store-spike".to_owned(),
                },
            )?,
        );
        Ok(next)
    }
}

fn raw_table(
    read: &redb::ReadTransaction,
    definition: TableDefinition<'static, &[u8], &[u8]>,
) -> StoreResult<BTreeMap<Vec<u8>, Vec<u8>>> {
    read.open_table(definition)
        .map_err(integrity)?
        .iter()
        .map_err(integrity)?
        .map(|row| {
            row.map(|(raw_key, raw_value)| (raw_key.value().to_vec(), raw_value.value().to_vec()))
                .map_err(integrity)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::io;
    use std::sync::{Arc, Mutex};

    use redb::StorageBackend;

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FaultMode {
        BeforeData,
        PartialData,
        BeforeHeader,
        PartialHeader,
        DuringSync,
    }

    #[derive(Debug, Default)]
    struct FaultState {
        armed: Option<FaultMode>,
        fired: bool,
    }

    #[derive(Debug, Clone, Default)]
    struct FaultController {
        state: Arc<Mutex<FaultState>>,
    }

    impl FaultController {
        fn arm(&self, mode: FaultMode) {
            let mut state = self.state.lock().unwrap();
            state.armed = Some(mode);
            state.fired = false;
        }

        fn take_if(&self, predicate: impl FnOnce(FaultMode) -> bool) -> Option<FaultMode> {
            let mut state = self.state.lock().unwrap();
            let mode = state.armed.filter(|mode| predicate(*mode))?;
            state.armed = None;
            state.fired = true;
            Some(mode)
        }

        fn fired(&self) -> bool {
            self.state.lock().unwrap().fired
        }
    }

    #[derive(Debug)]
    struct FaultBackend {
        inner: FileBackend,
        controller: FaultController,
    }

    impl StorageBackend for FaultBackend {
        fn len(&self) -> Result<u64, io::Error> {
            self.inner.len()
        }

        fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), io::Error> {
            self.inner.read(offset, out)
        }

        fn set_len(&self, len: u64) -> Result<(), io::Error> {
            self.inner.set_len(len)
        }

        fn sync_data(&self) -> Result<(), io::Error> {
            if self
                .controller
                .take_if(|mode| mode == FaultMode::DuringSync)
                .is_some()
            {
                return Err(eio());
            }
            self.inner.sync_data()
        }

        fn write(&self, offset: u64, data: &[u8]) -> Result<(), io::Error> {
            let mode = self.controller.take_if(|mode| match mode {
                FaultMode::BeforeData | FaultMode::PartialData => offset != 0,
                FaultMode::BeforeHeader | FaultMode::PartialHeader => offset == 0,
                FaultMode::DuringSync => false,
            });
            match mode {
                Some(FaultMode::BeforeData | FaultMode::BeforeHeader) => Err(eio()),
                Some(FaultMode::PartialData | FaultMode::PartialHeader) => {
                    let partial_len = (data.len() / 2).max(1).min(data.len());
                    self.inner.write(offset, &data[..partial_len])?;
                    Err(eio())
                }
                Some(FaultMode::DuringSync) | None => self.inner.write(offset, data),
            }
        }

        fn close(&self) -> Result<(), io::Error> {
            self.inner.close()
        }
    }

    fn eio() -> io::Error {
        io::Error::from_raw_os_error(nix::libc::EIO)
    }

    fn open_fault_store(path: &Path, controller: FaultController) -> StoreResult<DiskStore> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(integrity)?;
        let inner = FileBackend::new(file).map_err(integrity)?;
        let database = Database::builder()
            .create_with_backend(FaultBackend { inner, controller })
            .map_err(integrity)?;
        let store = DiskStore { database };
        store.validate_meta()?;
        Ok(store)
    }

    fn apply_one(store: &DiskStore, mutation: Mutation) -> Resource {
        let mut outcome = store.apply_group(&[mutation]).unwrap();
        assert_eq!(outcome.receipts.len(), 1);
        outcome.receipts.remove(0).unwrap().resource
    }

    fn table_contains(
        store: &DiskStore,
        definition: TableDefinition<'static, &[u8], &[u8]>,
        encoded_key: &[u8],
    ) -> bool {
        let read = store.database.begin_read().unwrap();
        let table = read.open_table(definition).unwrap();
        table.get(encoded_key).unwrap().is_some()
    }

    #[test]
    fn commit_faults_recover_only_complete_raw_checkpoints_or_refuse_open() {
        for mode in [
            FaultMode::BeforeData,
            FaultMode::PartialData,
            FaultMode::BeforeHeader,
            FaultMode::PartialHeader,
            FaultMode::DuringSync,
        ] {
            let path = crate::fixture_path(&format!("commit-fault-{mode:?}"));
            let checkpoints = prepare_crash_database(&path).unwrap();
            let controller = FaultController::default();
            let store = open_fault_store(&path, controller.clone()).unwrap();
            let mutation = Mutation::update(
                crate::model::synthetic_resource(10_001),
                0,
                "crash-operation".to_owned(),
            );
            let arm = controller.clone();
            let result = store.apply_group_with_commit_hook(&[mutation], || arm.arm(mode));
            assert!(result.is_err(), "{mode:?} did not fail commit");
            assert!(controller.fired(), "{mode:?} did not reach its fault point");
            drop(store);

            let recovery = verify_crash_database(&path, &checkpoints).unwrap();
            println!("fault={mode:?} recovery={recovery:?} result=PASS");
            assert!(
                matches!(
                    recovery,
                    CrashRecovery::LastCommittedState
                        | CrashRecovery::NewCommittedState
                        | CrashRecovery::RefusedToOpen
                ),
                "{mode:?} recovered an unclassified state"
            );
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn updates_remove_and_replace_every_mutable_index_across_reopen() {
        let path = crate::fixture_path("index-update-removal");
        let store = DiskStore::open_or_create(&path).unwrap();
        let mut initial = crate::model::synthetic_resource(1);
        initial.owner_uid = Some("owner-old".to_owned());
        initial.producer_uid = Some("producer-old".to_owned());
        initial.controller = "controller-old".to_owned();
        let initial = apply_one(&store, Mutation::create(initial));
        let mut oracle = BTreeMap::from([(initial.key.clone(), initial.clone())]);
        store.verify(&oracle).unwrap();

        let old_owner = key(
            0x05,
            &[
                KeyPart::Text(initial.owner_uid.as_deref().unwrap()),
                KeyPart::Text(&initial.uid),
            ],
        );
        let old_producer = key(
            0x06,
            &[
                KeyPart::Text(initial.producer_uid.as_deref().unwrap()),
                KeyPart::Text(&initial.uid),
            ],
        );
        let old_controller = key(
            0x07,
            &[
                KeyPart::Text(&initial.controller),
                KeyPart::Text(&initial.key.resource_type),
                KeyPart::Text(&initial.key.name),
            ],
        );

        let mut changed = initial.clone();
        changed.generation += 1;
        changed.owner_uid = Some("owner-new".to_owned());
        changed.producer_uid = Some("producer-new".to_owned());
        changed.controller = "controller-new".to_owned();
        let changed = apply_one(
            &store,
            Mutation::update(changed, initial.revision, "change-bindings".to_owned()),
        );
        oracle.insert(changed.key.clone(), changed.clone());
        store.verify(&oracle).unwrap();
        assert!(!table_contains(&store, OWNER_INDEX, &old_owner));
        assert!(!table_contains(&store, PRODUCER_INDEX, &old_producer));
        assert!(!table_contains(&store, CONTROLLER_INDEX, &old_controller));

        let new_owner = key(
            0x05,
            &[
                KeyPart::Text(changed.owner_uid.as_deref().unwrap()),
                KeyPart::Text(&changed.uid),
            ],
        );
        let new_producer = key(
            0x06,
            &[
                KeyPart::Text(changed.producer_uid.as_deref().unwrap()),
                KeyPart::Text(&changed.uid),
            ],
        );
        let new_controller = key(
            0x07,
            &[
                KeyPart::Text(&changed.controller),
                KeyPart::Text(&changed.key.resource_type),
                KeyPart::Text(&changed.key.name),
            ],
        );
        assert!(table_contains(&store, OWNER_INDEX, &new_owner));
        assert!(table_contains(&store, PRODUCER_INDEX, &new_producer));
        assert!(table_contains(&store, CONTROLLER_INDEX, &new_controller));

        let mut removed = changed.clone();
        removed.generation += 1;
        removed.owner_uid = None;
        removed.producer_uid = None;
        removed.controller = "controller-final".to_owned();
        let removed = apply_one(
            &store,
            Mutation::update(removed, changed.revision, "remove-bindings".to_owned()),
        );
        oracle.insert(removed.key.clone(), removed.clone());
        store.verify(&oracle).unwrap();
        assert!(!table_contains(&store, OWNER_INDEX, &new_owner));
        assert!(!table_contains(&store, PRODUCER_INDEX, &new_producer));
        assert!(!table_contains(&store, CONTROLLER_INDEX, &new_controller));
        let final_controller = key(
            0x07,
            &[
                KeyPart::Text(&removed.controller),
                KeyPart::Text(&removed.key.resource_type),
                KeyPart::Text(&removed.key.name),
            ],
        );
        assert!(table_contains(&store, CONTROLLER_INDEX, &final_controller));

        drop(store);
        let reopened = DiskStore::open_existing(&path).unwrap();
        reopened.verify(&oracle).unwrap();
        assert!(!table_contains(&reopened, OWNER_INDEX, &old_owner));
        assert!(!table_contains(&reopened, OWNER_INDEX, &new_owner));
        assert!(!table_contains(&reopened, PRODUCER_INDEX, &old_producer));
        assert!(!table_contains(&reopened, PRODUCER_INDEX, &new_producer));
        assert!(!table_contains(
            &reopened,
            CONTROLLER_INDEX,
            &old_controller
        ));
        assert!(!table_contains(
            &reopened,
            CONTROLLER_INDEX,
            &new_controller
        ));
        assert!(table_contains(
            &reopened,
            CONTROLLER_INDEX,
            &final_controller
        ));
        let _ = std::fs::remove_file(path);
    }
}
