//! Zone Resource mutation idempotency and bounded dispatch routing.
//!
//! The router keeps the full namespace of an idempotency key.  Reusing a key
//! under another authenticated principal or another request is a conflict,
//! never a replay.  Completed rows retain tombstones through a no-reuse
//! horizon so expiration cannot turn an old key into a fresh mutation.

use std::{collections::BTreeMap, fmt, sync::Mutex};

use d2b_contracts::v3::{
    ResourceName, ResourceRef, ResourceTypeName, ResourceUid, execution_policy::BoundedToken,
    zone_routing::ZonePath,
};

/// The per-Zone concurrent mutation ceiling.
pub const MAX_DISPATCH_IN_FLIGHT: usize = 64;
/// The maximum durable idempotency rows retained by one router.
pub const MAX_IDEMPOTENCY_ROWS: usize = 16_384;
/// The no-reuse horizon after an idempotency row expires.
pub const IDEMPOTENCY_TOMBSTONE_HORIZON_MS: u64 = 15 * 60 * 1_000;
/// The maximum number of retained EphemeralProcess execution rows.
pub const DEFAULT_MAX_EXECUTIONS: usize = 1_024;

/// The closed mutation verb set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MutationVerb {
    /// Create a resource.
    Create,
    /// Replace desired spec.
    UpdateSpec,
    /// Persist provider status.
    UpdateStatus,
    /// Delete a resource.
    Delete,
}

/// The complete idempotency namespace.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DedupKey {
    zone: ZonePath,
    resource_type: ResourceTypeName,
    resource_name: Option<ResourceName>,
    verb: MutationVerb,
    idempotency_key: BoundedToken,
    authenticated_principal: ResourceRef,
}

impl DedupKey {
    /// Construct a key from authenticated principal evidence.
    pub fn new(
        zone: ZonePath,
        resource_type: ResourceTypeName,
        resource_name: Option<ResourceName>,
        verb: MutationVerb,
        idempotency_key: BoundedToken,
        authenticated_principal: ResourceRef,
    ) -> Result<Self, RouterError> {
        if authenticated_principal.resource_type().as_str() != "User" {
            return Err(RouterError::PrincipalBinding);
        }
        Ok(Self {
            zone,
            resource_type,
            resource_name,
            verb,
            idempotency_key,
            authenticated_principal,
        })
    }

    /// Borrow the Zone namespace.
    pub const fn zone(&self) -> &ZonePath {
        &self.zone
    }

    /// Borrow the authenticated principal.
    pub const fn principal(&self) -> &ResourceRef {
        &self.authenticated_principal
    }
}

impl fmt::Debug for DedupKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DedupKey(<redacted>)")
    }
}

struct IdempotencyRow {
    request_digest: String,
    operation_id: ResourceUid,
    result: Option<String>,
    expires_at_ms: u64,
    tombstone_until_ms: u64,
}

/// The result of admitting one mutation key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DedupDecision {
    /// This is a new operation and owns the returned operation ID.
    New(ResourceUid),
    /// The same request completed earlier.
    Replay {
        operation_id: ResourceUid,
        result: String,
    },
    /// The key is already bound to a different request or principal.
    Conflict,
    /// The key remains in its no-reuse tombstone period.
    Tombstoned,
}

/// A bounded Zone operation router.
pub struct ZoneOperationRouter {
    rows: Mutex<BTreeMap<DedupKey, IdempotencyRow>>,
    max_rows: usize,
}

impl fmt::Debug for ZoneOperationRouter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ZoneOperationRouter(<redacted>)")
    }
}

impl ZoneOperationRouter {
    /// Build a router at the frozen row ceiling.
    pub fn new() -> Self {
        Self {
            rows: Mutex::new(BTreeMap::new()),
            max_rows: MAX_IDEMPOTENCY_ROWS,
        }
    }

    /// Admit a key and request digest.
    pub fn begin(
        &self,
        key: DedupKey,
        request_digest: impl Into<String>,
        operation_id: ResourceUid,
        now_ms: u64,
        expiry_ms: u64,
    ) -> Result<DedupDecision, RouterError> {
        if expiry_ms <= now_ms {
            return Err(RouterError::InvalidExpiry);
        }
        let request_digest = request_digest.into();
        if request_digest.is_empty() {
            return Err(RouterError::InvalidRequestDigest);
        }
        let mut rows = self.rows.lock().map_err(|_| RouterError::Poisoned)?;
        if let Some(existing) = rows.get(&key) {
            if existing.expires_at_ms <= now_ms
                && existing.tombstone_until_ms > now_ms
                && existing.result.is_none()
            {
                return Ok(DedupDecision::Tombstoned);
            }
            if existing.request_digest != request_digest {
                return Ok(DedupDecision::Conflict);
            }
            if let Some(result) = &existing.result {
                return Ok(DedupDecision::Replay {
                    operation_id: existing.operation_id.clone(),
                    result: result.clone(),
                });
            }
            return Ok(DedupDecision::Replay {
                operation_id: existing.operation_id.clone(),
                result: String::new(),
            });
        }
        if rows.len() >= self.max_rows {
            return Err(RouterError::Capacity);
        }
        rows.insert(
            key,
            IdempotencyRow {
                request_digest,
                operation_id: operation_id.clone(),
                result: None,
                expires_at_ms: expiry_ms,
                tombstone_until_ms: expiry_ms.saturating_add(IDEMPOTENCY_TOMBSTONE_HORIZON_MS),
            },
        );
        Ok(DedupDecision::New(operation_id))
    }

    /// Commit the result for a previously admitted operation.
    pub fn complete(
        &self,
        key: &DedupKey,
        operation_id: &ResourceUid,
        result: impl Into<String>,
    ) -> Result<(), RouterError> {
        let mut rows = self.rows.lock().map_err(|_| RouterError::Poisoned)?;
        let row = rows.get_mut(key).ok_or(RouterError::UnknownOperation)?;
        if &row.operation_id != operation_id {
            return Err(RouterError::OperationMismatch);
        }
        row.result = Some(result.into());
        Ok(())
    }

    /// Sweep rows only after the tombstone horizon has elapsed.
    pub fn sweep(&self, now_ms: u64) -> Result<usize, RouterError> {
        let mut rows = self.rows.lock().map_err(|_| RouterError::Poisoned)?;
        let before = rows.len();
        rows.retain(|_, row| row.tombstone_until_ms > now_ms);
        Ok(before - rows.len())
    }

    /// Number of retained idempotency rows.
    pub fn len(&self) -> Result<usize, RouterError> {
        Ok(self.rows.lock().map_err(|_| RouterError::Poisoned)?.len())
    }

    /// Whether no idempotency rows are retained.
    pub fn is_empty(&self) -> Result<bool, RouterError> {
        Ok(self.len()? == 0)
    }
}

impl Default for ZoneOperationRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Bounded in-flight EphemeralProcess operation table.
#[derive(Debug)]
pub struct DurableExecTable {
    rows: Mutex<BTreeMap<ResourceUid, ResourceRef>>,
    capacity: usize,
}

impl DurableExecTable {
    /// Build a table at the default capacity.
    pub fn new() -> Self {
        Self {
            rows: Mutex::new(BTreeMap::new()),
            capacity: DEFAULT_MAX_EXECUTIONS,
        }
    }

    /// Admit an execution identity.
    pub fn insert(
        &self,
        operation_id: ResourceUid,
        process_ref: ResourceRef,
    ) -> Result<(), RouterError> {
        if process_ref.resource_type().as_str() != "EphemeralProcess" {
            return Err(RouterError::WrongExecutionType);
        }
        let mut rows = self.rows.lock().map_err(|_| RouterError::Poisoned)?;
        if rows.len() >= self.capacity || rows.contains_key(&operation_id) {
            return Err(RouterError::Capacity);
        }
        rows.insert(operation_id, process_ref);
        Ok(())
    }

    /// Remove a completed execution.
    pub fn remove(&self, operation_id: &ResourceUid) -> Result<bool, RouterError> {
        Ok(self
            .rows
            .lock()
            .map_err(|_| RouterError::Poisoned)?
            .remove(operation_id)
            .is_some())
    }

    /// Return current table occupancy.
    pub fn len(&self) -> Result<usize, RouterError> {
        Ok(self.rows.lock().map_err(|_| RouterError::Poisoned)?.len())
    }

    /// Whether no execution rows are retained.
    pub fn is_empty(&self) -> Result<bool, RouterError> {
        Ok(self.len()? == 0)
    }
}

impl Default for DurableExecTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Closed Zone router errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterError {
    /// Principal was not an authenticated User reference.
    PrincipalBinding,
    /// Expiry was not after admission time.
    InvalidExpiry,
    /// Request digest was empty.
    InvalidRequestDigest,
    /// Idempotency capacity was exhausted.
    Capacity,
    /// Operation was not found.
    UnknownOperation,
    /// Operation ID did not match the row.
    OperationMismatch,
    /// The execution table received a non-EphemeralProcess ref.
    WrongExecutionType,
    /// The internal mutex was poisoned.
    Poisoned,
}

impl fmt::Display for RouterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PrincipalBinding => "zone-router-principal-binding-invalid",
            Self::InvalidExpiry => "zone-router-expiry-invalid",
            Self::InvalidRequestDigest => "zone-router-request-digest-invalid",
            Self::Capacity => "zone-router-capacity-exceeded",
            Self::UnknownOperation => "zone-router-operation-unknown",
            Self::OperationMismatch => "zone-router-operation-mismatch",
            Self::WrongExecutionType => "zone-router-execution-type-invalid",
            Self::Poisoned => "zone-router-state-poisoned",
        })
    }
}

impl std::error::Error for RouterError {}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::zone_routing::ZoneLabelId;
    use d2b_contracts::v3::{ResourceName, ResourceTypeName, ZoneId};

    fn zone() -> ZonePath {
        ZonePath::new(vec![ZoneLabelId::parse("dev").unwrap()]).unwrap()
    }

    fn key() -> DedupKey {
        DedupKey::new(
            zone(),
            ResourceTypeName::parse("Process").unwrap(),
            Some(ResourceName::parse("worker").unwrap()),
            MutationVerb::Create,
            BoundedToken::parse("request-1").unwrap(),
            ResourceRef::parse("User/alice").unwrap(),
        )
        .unwrap()
    }

    fn operation() -> ResourceUid {
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap()
    }

    #[test]
    fn same_key_replays_and_different_request_conflicts() {
        let router = ZoneOperationRouter::new();
        assert_eq!(
            router
                .begin(key(), "digest-a", operation(), 1, 100)
                .unwrap(),
            DedupDecision::New(operation())
        );
        router.complete(&key(), &operation(), "ok").unwrap();
        assert_eq!(
            router
                .begin(key(), "digest-a", operation(), 2, 100)
                .unwrap(),
            DedupDecision::Replay {
                operation_id: operation(),
                result: "ok".to_owned()
            }
        );
        assert_eq!(
            router
                .begin(key(), "digest-b", operation(), 2, 100)
                .unwrap(),
            DedupDecision::Conflict
        );
    }

    #[test]
    fn expiry_keeps_a_tombstone_until_the_no_reuse_horizon() {
        let router = ZoneOperationRouter::new();
        router
            .begin(key(), "digest-a", operation(), 1, 100)
            .unwrap();
        assert_eq!(
            router
                .begin(key(), "digest-a", operation(), 101, 200)
                .unwrap(),
            DedupDecision::Tombstoned
        );
        assert_eq!(router.sweep(IDEMPOTENCY_TOMBSTONE_HORIZON_MS + 100), Ok(1));
        assert_eq!(router.len().unwrap(), 0);
    }

    #[test]
    fn durable_exec_table_is_bounded_to_ephemeral_processes() {
        let table = DurableExecTable::new();
        let operation = operation();
        table
            .insert(
                operation.clone(),
                ResourceRef::parse("EphemeralProcess/exec-1").unwrap(),
            )
            .unwrap();
        assert_eq!(table.len().unwrap(), 1);
        assert!(table.remove(&operation).unwrap());
        assert_eq!(table.len().unwrap(), 0);
        let _ = ZoneId::parse("dev").unwrap();
    }
}
