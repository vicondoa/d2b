use std::fmt;
use std::time::Instant;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResourceKey {
    pub resource_type: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    pub key: ResourceKey,
    pub uid: String,
    pub owner_uid: Option<String>,
    pub producer_uid: Option<String>,
    pub controller: String,
    pub generation: u64,
    pub revision: u64,
    pub spec_json: String,
    pub status_json: String,
}

impl Resource {
    pub fn payload_bytes(&self) -> usize {
        self.spec_json.len() + self.status_json.len()
    }
}

#[derive(Debug, Clone)]
pub struct Mutation {
    pub resource: Resource,
    pub expected_revision: u64,
    pub operation_id: String,
}

impl Mutation {
    pub fn create(resource: Resource) -> Self {
        let operation_id = format!("create-{}", resource.uid);
        Self {
            resource,
            expected_revision: 0,
            operation_id,
        }
    }

    pub fn update(resource: Resource, expected_revision: u64, operation_id: String) -> Self {
        Self {
            resource,
            expected_revision,
            operation_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeEntry {
    pub ordinal: u16,
    pub resource: Resource,
    pub event: String,
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeBatch {
    pub revision: u64,
    pub entries: Vec<ChangeEntry>,
}

#[derive(Debug, Clone)]
pub struct WriteReceipt {
    pub resource: Resource,
    pub revision: u64,
    pub ordinal: u16,
    pub batch_size: usize,
    pub committed_at: Instant,
}

#[derive(Debug, Clone)]
pub struct OracleCheckpoint {
    pub changed_resource: Resource,
    pub resource_count: u64,
    pub owner_count: u64,
    pub producer_count: u64,
    pub operation_count: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    Backpressure,
    Conflict { current_revision: u64 },
    Integrity(String),
    Closed,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backpressure => formatter.write_str("store-backpressure"),
            Self::Conflict { current_revision } => {
                write!(formatter, "resource-conflict:{current_revision}")
            }
            Self::Integrity(message) => write!(formatter, "store-integrity-failure:{message}"),
            Self::Closed => formatter.write_str("store-closed"),
        }
    }
}

impl std::error::Error for StoreError {}

pub type StoreResult<T> = Result<T, StoreError>;

pub fn synthetic_resource(index: usize) -> Resource {
    const TYPES: [&str; 6] = ["Process", "Endpoint", "Volume", "Device", "Guest", "Policy"];
    let resource_type = TYPES[index % TYPES.len()];
    let uid = format!("uid-{index:08}");
    let owner_uid = (index >= 8).then(|| format!("uid-{:08}", (index - 1) / 8));
    let producer_uid = (resource_type == "Endpoint").then(|| format!("producer-{:08}", index / 6));
    let spec_padding = "s".repeat(384 + index % 64);
    let status_padding = "o".repeat(192 + index % 32);
    Resource {
        key: ResourceKey {
            resource_type: resource_type.to_owned(),
            name: format!("resource-{index:08}"),
        },
        uid,
        owner_uid,
        producer_uid,
        controller: format!("controller-{:02}", index % 12),
        generation: 1,
        revision: 0,
        spec_json: format!(r#"{{"enabled":true,"index":{index},"payload":"{spec_padding}"}}"#),
        status_json: format!(r#"{{"phase":"Ready","index":{index},"details":"{status_padding}"}}"#),
    }
}
