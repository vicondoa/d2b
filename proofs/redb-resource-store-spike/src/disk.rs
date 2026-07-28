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
        write.commit().map_err(integrity)?;

        Ok(AppliedGroup {
            revision: Some(revision),
            receipts: results,
            batch: Some(batch),
        })
    }

    pub(crate) fn revision_batches_after(
        &self,
        after_revision: u64,
    ) -> StoreResult<Vec<ChangeBatch>> {
        let read = self.database.begin_read().map_err(integrity)?;
        let table = read.open_table(REVISION_LOG).map_err(integrity)?;
        let mut batches = Vec::new();
        for row in table.iter().map_err(integrity)? {
            let (_, bytes) = row.map_err(integrity)?;
            let batch: ChangeBatch = decode(0x0008, bytes.value())?;
            if batch.revision > after_revision {
                batches.push(batch);
            }
        }
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

pub fn crash_database_path(boundary: u8) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("spike-data")
        .join(format!("crash-{}-{boundary}.redb", std::process::id()))
}

pub fn prepare_crash_database(path: &Path) -> StoreResult<()> {
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
    Ok(())
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

pub fn verify_crash_database(path: &Path) -> StoreResult<CrashRecovery> {
    let store = match DiskStore::open_existing(path) {
        Ok(store) => store,
        Err(_) => return Ok(CrashRecovery::RefusedToOpen),
    };
    let snapshot = store.snapshot()?;
    if snapshot.is_empty() {
        return Err(StoreError::Integrity(
            "silent-empty-store-after-crash".to_owned(),
        ));
    }
    let baseline_key = crate::model::synthetic_resource(0).key;
    if !snapshot.contains_key(&baseline_key) {
        return Err(StoreError::Integrity(
            "committed-baseline-missing-after-crash".to_owned(),
        ));
    }
    let candidate_key = crate::model::synthetic_resource(10_001).key;
    let revision = store.current_revision()?;
    let batches = store.revision_batches_after(0)?;
    let state = match (
        revision,
        snapshot.contains_key(&candidate_key),
        batches.len(),
    ) {
        (1, false, 1) => CrashRecovery::LastCommittedState,
        (2, true, 2) => CrashRecovery::NewCommittedState,
        _ => {
            return Err(StoreError::Integrity(format!(
                "partial-state-after-crash:revision={revision} resources={} batches={}",
                snapshot.len(),
                batches.len()
            )));
        }
    };
    store.verify(&snapshot)?;
    Ok(state)
}
