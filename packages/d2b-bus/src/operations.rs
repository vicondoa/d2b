//! Bounded operation tracking with pinned reverse routes and cancellation.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::registry::RouteKey;

/// Maximum concurrent operations retained by the default bus.
pub const DEFAULT_MAX_OPERATIONS: usize = 4096;

/// An opaque operation identifier.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationId(String);

impl OperationId {
    /// Parse a bounded printable ASCII identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self, OperationError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(OperationError::InvalidOperationId);
        }
        Ok(Self(value))
    }

    /// Borrow the exact identifier for an authorized wire encoding.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for OperationId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OperationId(<redacted>)")
    }
}

/// Immutable operation identity and deadline.
#[derive(Clone, PartialEq, Eq)]
pub struct OperationSpec {
    id: OperationId,
    deadline_tick: u64,
}

impl OperationSpec {
    /// Construct an operation with a nonzero monotonic deadline.
    pub fn new(id: OperationId, deadline_tick: u64) -> Result<Self, OperationError> {
        if deadline_tick == 0 {
            return Err(OperationError::InvalidDeadline);
        }
        Ok(Self { id, deadline_tick })
    }

    /// Borrow the operation identifier.
    pub const fn id(&self) -> &OperationId {
        &self.id
    }

    /// Return the monotonic deadline tick.
    pub const fn deadline_tick(&self) -> u64 {
        self.deadline_tick
    }
}

impl core::fmt::Debug for OperationSpec {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OperationSpec")
            .field("id", &self.id)
            .field("deadline_tick", &"<redacted>")
            .finish()
    }
}

/// Cancellation state delivered to a handler without exposing the operation table.
#[derive(Clone)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

impl core::fmt::Debug for Cancellation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Cancellation")
            .field("is_cancelled", &self.is_cancelled())
            .finish()
    }
}

/// Internal, non-forgeable session identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SessionId(pub(crate) u64);

struct OperationRecord {
    source: SessionId,
    destination: SessionId,
    reverse_route: RouteKey,
    deadline_tick: u64,
    cancellation: Cancellation,
}

/// Exact destination retained for a cancellation dispatch.
pub(crate) struct CancelTarget {
    pub(crate) destination: SessionId,
    pub(crate) route: RouteKey,
    pub(crate) cancellation: Cancellation,
}

/// In-memory table for bounded in-flight work.
pub(crate) struct OperationTable {
    max_operations: usize,
    records: BTreeMap<OperationId, OperationRecord>,
}

impl OperationTable {
    pub(crate) fn new(max_operations: usize) -> Result<Self, OperationError> {
        if max_operations == 0 {
            return Err(OperationError::InvalidLimit);
        }
        Ok(Self {
            max_operations,
            records: BTreeMap::new(),
        })
    }

    pub(crate) fn begin(
        &mut self,
        operation: &OperationSpec,
        source: SessionId,
        destination: SessionId,
        reverse_route: RouteKey,
        now_tick: u64,
    ) -> Result<Cancellation, OperationError> {
        if now_tick >= operation.deadline_tick {
            return Err(OperationError::DeadlineExceeded);
        }
        if self.records.contains_key(&operation.id) {
            return Err(OperationError::DuplicateOperation);
        }
        if self.records.len() >= self.max_operations {
            return Err(OperationError::CapacityExceeded);
        }
        let cancellation = Cancellation::new();
        self.records.insert(
            operation.id.clone(),
            OperationRecord {
                source,
                destination,
                reverse_route,
                deadline_tick: operation.deadline_tick,
                cancellation: cancellation.clone(),
            },
        );
        Ok(cancellation)
    }

    pub(crate) fn finish(
        &mut self,
        operation: &OperationId,
        source: SessionId,
        now_tick: u64,
    ) -> Result<(), OperationError> {
        let record = self
            .records
            .get(operation)
            .ok_or(OperationError::OperationNotFound)?;
        if record.source != source {
            return Err(OperationError::OperationOwnerMismatch);
        }
        if record.cancellation.is_cancelled() {
            self.records.remove(operation);
            return Err(OperationError::Cancelled);
        }
        if now_tick >= record.deadline_tick {
            record.cancellation.cancel();
            self.records.remove(operation);
            return Err(OperationError::DeadlineExceeded);
        }
        self.records.remove(operation);
        Ok(())
    }

    pub(crate) fn route_for_cancel(
        &self,
        operation: &OperationId,
        source: SessionId,
    ) -> Result<RouteKey, OperationError> {
        let record = self
            .records
            .get(operation)
            .ok_or(OperationError::OperationNotFound)?;
        if record.source != source {
            return Err(OperationError::OperationOwnerMismatch);
        }
        Ok(record.reverse_route.clone())
    }

    pub(crate) fn cancel(
        &mut self,
        operation: &OperationId,
        source: SessionId,
    ) -> Result<CancelTarget, OperationError> {
        let record = self
            .records
            .remove(operation)
            .ok_or(OperationError::OperationNotFound)?;
        if record.source != source {
            self.records.insert(operation.clone(), record);
            return Err(OperationError::OperationOwnerMismatch);
        }
        record.cancellation.cancel();
        Ok(CancelTarget {
            destination: record.destination,
            route: record.reverse_route,
            cancellation: record.cancellation,
        })
    }

    pub(crate) fn cancel_session(&mut self, session: SessionId) -> Vec<OperationId> {
        let cancelled = self
            .records
            .iter()
            .filter(|(_, record)| record.source == session || record.destination == session)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in &cancelled {
            if let Some(record) = self.records.remove(id) {
                record.cancellation.cancel();
            }
        }
        cancelled
    }
}

/// Closed operation failure classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationError {
    InvalidOperationId,
    InvalidDeadline,
    InvalidLimit,
    DuplicateOperation,
    CapacityExceeded,
    DeadlineExceeded,
    OperationNotFound,
    OperationOwnerMismatch,
    Cancelled,
}

impl core::fmt::Display for OperationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidOperationId => "operation identifier is invalid",
            Self::InvalidDeadline => "operation deadline must be nonzero",
            Self::InvalidLimit => "operation table limit must be nonzero",
            Self::DuplicateOperation => "operation identifier is already active",
            Self::CapacityExceeded => "operation table capacity is exhausted",
            Self::DeadlineExceeded => "operation deadline has elapsed",
            Self::OperationNotFound => "operation is not active",
            Self::OperationOwnerMismatch => "operation is owned by another session",
            Self::Cancelled => "operation was cancelled",
        })
    }
}

impl std::error::Error for OperationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{RouteGenerations, RouteKey, RouteMember, RouteTarget};
    use d2b_contracts::v3::{
        ControllerGeneration, ReconnectGeneration, ResourceGeneration, ResourceRef,
        SchemaFingerprint, ServiceName, ZoneId,
    };

    fn route() -> RouteKey {
        RouteKey::new(
            ZoneId::parse("dev").unwrap(),
            ServiceName::parse("d2b.resource.v3").unwrap(),
            RouteMember::method("ResourceService/Get").unwrap(),
            RouteTarget::provider(ResourceRef::parse("Provider/system-core").unwrap()).unwrap(),
            SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
            RouteGenerations::new(
                Some(ResourceGeneration::new(2).unwrap()),
                Some(ControllerGeneration::new(3).unwrap()),
                ReconnectGeneration::new(4).unwrap(),
            ),
        )
    }

    fn operation(value: &str, deadline: u64) -> OperationSpec {
        OperationSpec::new(OperationId::parse(value).unwrap(), deadline).unwrap()
    }

    #[test]
    fn operation_ids_and_deadlines_are_closed() {
        for invalid in ["", "bad id", "x/y", &"x".repeat(129)] {
            assert_eq!(
                OperationId::parse(invalid),
                Err(OperationError::InvalidOperationId)
            );
        }
        assert_eq!(
            OperationSpec::new(OperationId::parse("op-1").unwrap(), 0),
            Err(OperationError::InvalidDeadline)
        );
    }

    #[test]
    fn duplicate_capacity_and_deadline_fail_closed() {
        let mut table = OperationTable::new(1).unwrap();
        let first = operation("first", 10);
        table
            .begin(&first, SessionId(1), SessionId(2), route(), 1)
            .unwrap();
        assert_eq!(
            table
                .begin(&first, SessionId(1), SessionId(2), route(), 1)
                .unwrap_err(),
            OperationError::DuplicateOperation
        );
        assert_eq!(
            table
                .begin(
                    &operation("second", 10),
                    SessionId(1),
                    SessionId(2),
                    route(),
                    1,
                )
                .unwrap_err(),
            OperationError::CapacityExceeded
        );
        let mut empty = OperationTable::new(1).unwrap();
        assert_eq!(
            empty
                .begin(
                    &operation("expired", 2),
                    SessionId(1),
                    SessionId(2),
                    route(),
                    2,
                )
                .unwrap_err(),
            OperationError::DeadlineExceeded
        );
    }

    #[test]
    fn cancellation_is_owner_checked_and_pins_the_reverse_route() {
        let mut table = OperationTable::new(2).unwrap();
        let operation = operation("cancel-me", 10);
        let expected_route = route();
        let cancellation = table
            .begin(
                &operation,
                SessionId(1),
                SessionId(2),
                expected_route.clone(),
                1,
            )
            .unwrap();
        assert_eq!(
            table.route_for_cancel(operation.id(), SessionId(9)),
            Err(OperationError::OperationOwnerMismatch)
        );
        assert_eq!(
            table
                .route_for_cancel(operation.id(), SessionId(1))
                .unwrap(),
            expected_route
        );
        let target = table.cancel(operation.id(), SessionId(1)).unwrap();
        assert_eq!(target.destination, SessionId(2));
        assert_eq!(target.route, expected_route);
        assert!(target.cancellation.is_cancelled());
        assert!(cancellation.is_cancelled());
    }

    #[test]
    fn session_loss_cancels_both_directions() {
        let mut table = OperationTable::new(3).unwrap();
        let first = operation("first", 10);
        let second = operation("second", 10);
        let first_cancel = table
            .begin(&first, SessionId(1), SessionId(2), route(), 1)
            .unwrap();
        let second_cancel = table
            .begin(&second, SessionId(3), SessionId(1), route(), 1)
            .unwrap();
        assert_eq!(table.cancel_session(SessionId(1)).len(), 2);
        assert!(first_cancel.is_cancelled());
        assert!(second_cancel.is_cancelled());
    }

    #[test]
    fn finish_and_cancel_errors_do_not_release_another_sessions_operation() {
        let mut table = OperationTable::new(1).unwrap();
        let operation = operation("owned", 5);
        table
            .begin(&operation, SessionId(1), SessionId(2), route(), 1)
            .unwrap();

        assert_eq!(
            table.finish(operation.id(), SessionId(9), 2).unwrap_err(),
            OperationError::OperationOwnerMismatch
        );
        assert!(matches!(
            table.cancel(operation.id(), SessionId(9)),
            Err(OperationError::OperationOwnerMismatch)
        ));
        assert!(table.route_for_cancel(operation.id(), SessionId(1)).is_ok());
        assert_eq!(
            table.finish(operation.id(), SessionId(1), 5).unwrap_err(),
            OperationError::DeadlineExceeded
        );
        assert!(matches!(
            table.cancel(operation.id(), SessionId(1)),
            Err(OperationError::OperationNotFound)
        ));
        assert_eq!(
            table.finish(operation.id(), SessionId(1), 6).unwrap_err(),
            OperationError::OperationNotFound
        );
    }
}
