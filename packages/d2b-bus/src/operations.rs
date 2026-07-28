//! Bounded operation tracking with pinned reverse routes and cancellation.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Notify;

use crate::registry::{BusEndpoint, RevocableRouteLease, RouteKey};

/// Maximum concurrent operations retained by the default bus.
pub const DEFAULT_MAX_OPERATIONS: usize = 4096;
/// Default concurrent operations admitted for one source session.
pub const DEFAULT_MAX_OPERATIONS_PER_SESSION: usize = 256;

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
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

#[derive(Clone)]
pub struct Cancellation(Arc<CancellationState>);

impl Cancellation {
    pub(crate) fn new() -> Self {
        Self(Arc::new(CancellationState {
            cancelled: AtomicBool::new(false),
            notify: Notify::new(),
        }))
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        if !self.0.cancelled.swap(true, Ordering::AcqRel) {
            self.0.notify.notify_waiters();
        }
    }

    pub(crate) fn is_same_attempt(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) async fn cancelled(&self) {
        loop {
            let notified = self.0.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
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
    destination: SessionId,
    reverse_route: RouteKey,
    endpoint: Arc<dyn BusEndpoint>,
    generation: d2b_contracts::v3::ReconnectGeneration,
    deadline_tick: u64,
    cancellation: Cancellation,
    state: OperationState,
    retain_terminal: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OperationState {
    Active,
    Cancelling,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct OperationKey {
    source: SessionId,
    id: OperationId,
}

/// Exact destination retained for a cancellation dispatch.
#[derive(Clone)]
pub(crate) struct CancelTarget {
    pub(crate) route: RouteKey,
    pub(crate) endpoint: Arc<dyn BusEndpoint>,
    pub(crate) generation: d2b_contracts::v3::ReconnectGeneration,
    pub(crate) cancellation: Cancellation,
}

/// One cancellation dispatch bound to its exact source and attempt.
pub(crate) struct CancelDispatch {
    pub(crate) operation: OperationId,
    pub(crate) source: SessionId,
    pub(crate) target: CancelTarget,
}

struct OperationTombstone {
    key: OperationKey,
    cancellation: Cancellation,
}

/// In-memory table for bounded in-flight work.
pub(crate) struct OperationTable {
    max_operations: usize,
    max_operations_per_session: usize,
    records: BTreeMap<OperationKey, OperationRecord>,
    per_session: BTreeMap<SessionId, usize>,
    tombstones: VecDeque<OperationTombstone>,
}

impl OperationTable {
    pub(crate) fn new(
        max_operations: usize,
        max_operations_per_session: usize,
    ) -> Result<Self, OperationError> {
        if max_operations == 0
            || max_operations_per_session == 0
            || max_operations_per_session > max_operations
        {
            return Err(OperationError::InvalidLimit);
        }
        Ok(Self {
            max_operations,
            max_operations_per_session,
            records: BTreeMap::new(),
            per_session: BTreeMap::new(),
            tombstones: VecDeque::new(),
        })
    }

    pub(crate) fn begin(
        &mut self,
        operation: &OperationSpec,
        source: SessionId,
        route_lease: RevocableRouteLease,
        reverse_route: RouteKey,
        now_tick: u64,
    ) -> Result<Cancellation, OperationError> {
        if now_tick >= operation.deadline_tick {
            return Err(OperationError::DeadlineExceeded);
        }
        let key = OperationKey {
            source,
            id: operation.id.clone(),
        };
        route_lease
            .with_active(|| {
                if self.records.contains_key(&key) {
                    return Err(OperationError::DuplicateOperation);
                }
                if self.records.len() >= self.max_operations {
                    return Err(OperationError::CapacityExceeded);
                }
                if self.per_session.get(&source).copied().unwrap_or(0)
                    >= self.max_operations_per_session
                {
                    return Err(OperationError::SessionCapacityExceeded);
                }
                let cancellation = Cancellation::new();
                self.records.insert(
                    key,
                    OperationRecord {
                        destination: route_lease.destination(),
                        reverse_route,
                        endpoint: route_lease.endpoint(),
                        generation: route_lease.generation(),
                        deadline_tick: operation.deadline_tick,
                        cancellation: cancellation.clone(),
                        state: OperationState::Active,
                        retain_terminal: true,
                    },
                );
                *self.per_session.entry(source).or_default() += 1;
                Ok(cancellation)
            })
            .map_err(|_| OperationError::RouteRevoked)?
    }

    pub(crate) fn finish(
        &mut self,
        operation: &OperationId,
        source: SessionId,
        cancellation: &Cancellation,
        now_tick: u64,
    ) -> Result<(), OperationError> {
        let key = OperationKey {
            source,
            id: operation.clone(),
        };
        let Some(record) = self.records.get(&key) else {
            return if self.has_tombstone(&key, cancellation) {
                Err(OperationError::Cancelled)
            } else {
                Err(OperationError::OperationNotFound)
            };
        };
        if !record.cancellation.is_same_attempt(cancellation) {
            return if self.has_tombstone(&key, cancellation) {
                Err(OperationError::Cancelled)
            } else {
                Err(OperationError::OperationNotFound)
            };
        }
        if record.state != OperationState::Active {
            return Err(OperationError::Cancelled);
        }
        if now_tick >= record.deadline_tick {
            record.cancellation.cancel();
            self.remove(&key);
            return Err(OperationError::DeadlineExceeded);
        }
        self.remove(&key);
        Ok(())
    }

    pub(crate) fn abort(
        &mut self,
        operation: &OperationId,
        source: SessionId,
        cancellation: &Cancellation,
    ) -> Option<CancelTarget> {
        let key = OperationKey {
            source,
            id: operation.clone(),
        };
        let record = self.records.get_mut(&key)?;
        if record.state != OperationState::Active
            || !record.cancellation.is_same_attempt(cancellation)
        {
            return None;
        }
        record.cancellation.cancel();
        record.state = OperationState::Cancelling;
        Some(CancelTarget {
            route: record.reverse_route.clone(),
            endpoint: Arc::clone(&record.endpoint),
            generation: record.generation,
            cancellation: record.cancellation.clone(),
        })
    }

    pub(crate) fn route_for_cancel(
        &self,
        operation: &OperationId,
        source: SessionId,
    ) -> Result<RouteKey, OperationError> {
        let record = self
            .records
            .get(&OperationKey {
                source,
                id: operation.clone(),
            })
            .ok_or(OperationError::OperationNotFound)?;
        Ok(record.reverse_route.clone())
    }

    pub(crate) fn cancellation_complete(
        &self,
        operation: &OperationId,
        source: SessionId,
    ) -> Result<bool, OperationError> {
        let key = OperationKey {
            source,
            id: operation.clone(),
        };
        if self.records.contains_key(&key) {
            return Ok(false);
        }
        if self
            .tombstones
            .iter()
            .rev()
            .any(|tombstone| tombstone.key == key)
        {
            return Ok(true);
        }
        Err(OperationError::OperationNotFound)
    }

    pub(crate) fn cancel(
        &mut self,
        operation: &OperationId,
        source: SessionId,
    ) -> Result<Option<CancelTarget>, OperationError> {
        let key = OperationKey {
            source,
            id: operation.clone(),
        };
        let Some(record) = self.records.get_mut(&key) else {
            return if self
                .tombstones
                .iter()
                .rev()
                .any(|tombstone| tombstone.key == key)
            {
                Ok(None)
            } else {
                Err(OperationError::OperationNotFound)
            };
        };
        record.cancellation.cancel();
        record.state = OperationState::Cancelling;
        Ok(Some(CancelTarget {
            route: record.reverse_route.clone(),
            endpoint: Arc::clone(&record.endpoint),
            generation: record.generation,
            cancellation: record.cancellation.clone(),
        }))
    }

    pub(crate) fn complete_cancel(
        &mut self,
        operation: &OperationId,
        source: SessionId,
        cancellation: &Cancellation,
    ) -> Result<(), OperationError> {
        let key = OperationKey {
            source,
            id: operation.clone(),
        };
        let matches = self.records.get(&key).is_some_and(|record| {
            record.state == OperationState::Cancelling
                && record.cancellation.is_same_attempt(cancellation)
        });
        if matches {
            let record = self
                .remove(&key)
                .expect("matching operation record remains present");
            if record.retain_terminal {
                self.push_tombstone(key, record.cancellation);
            }
            return Ok(());
        }
        if self.has_tombstone(&key, cancellation) {
            Ok(())
        } else {
            Err(OperationError::OperationNotFound)
        }
    }

    pub(crate) fn complete_abort(
        &mut self,
        operation: &OperationId,
        source: SessionId,
        cancellation: &Cancellation,
    ) {
        let _ = self.complete_cancel(operation, source, cancellation);
    }

    pub(crate) fn cancel_session(&mut self, session: SessionId) -> Vec<CancelDispatch> {
        self.tombstones
            .retain(|tombstone| tombstone.key.source != session);
        let cancelled = self
            .records
            .iter()
            .filter(|(key, record)| key.source == session || record.destination == session)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        cancelled
            .into_iter()
            .filter_map(|key| {
                let record = self.records.get_mut(&key)?;
                if key.source == session {
                    record.retain_terminal = false;
                }
                record.cancellation.cancel();
                record.state = OperationState::Cancelling;
                Some(CancelDispatch {
                    operation: key.id,
                    source: key.source,
                    target: CancelTarget {
                        route: record.reverse_route.clone(),
                        endpoint: Arc::clone(&record.endpoint),
                        generation: record.generation,
                        cancellation: record.cancellation.clone(),
                    },
                })
            })
            .collect()
    }

    fn has_tombstone(&self, key: &OperationKey, cancellation: &Cancellation) -> bool {
        self.tombstones.iter().any(|tombstone| {
            tombstone.key == *key && tombstone.cancellation.is_same_attempt(cancellation)
        })
    }

    fn push_tombstone(&mut self, key: OperationKey, cancellation: Cancellation) {
        while self.tombstones.len() >= self.max_operations {
            self.tombstones.pop_front();
        }
        self.tombstones
            .push_back(OperationTombstone { key, cancellation });
    }

    fn remove(&mut self, key: &OperationKey) -> Option<OperationRecord> {
        let record = self.records.remove(key)?;
        if let Some(count) = self.per_session.get_mut(&key.source) {
            *count -= 1;
            if *count == 0 {
                self.per_session.remove(&key.source);
            }
        }
        Some(record)
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
    SessionCapacityExceeded,
    RouteRevoked,
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
            Self::SessionCapacityExceeded => "source session operation quota is exhausted",
            Self::RouteRevoked => "operation route was revoked",
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
    use crate::{
        registry::{
            BusEndpoint, BusResponse, EndpointError, RevocableRouteLease, RouteGenerations,
            RouteKey, RouteMember, RouteTarget,
        },
        router::{DeliveredInvocation, DeliveredStream},
    };
    use async_trait::async_trait;
    use d2b_contracts::v3::{
        ControllerGeneration, ReconnectGeneration, ResourceGeneration, ResourceRef,
        SchemaFingerprint, ServiceName, ZoneId,
    };

    struct TestEndpoint;

    #[async_trait]
    impl BusEndpoint for TestEndpoint {
        async fn invoke(
            &self,
            _request: DeliveredInvocation,
        ) -> Result<BusResponse, EndpointError> {
            Err(EndpointError::Unavailable)
        }

        async fn open_stream(&self, _request: DeliveredStream) -> Result<(), EndpointError> {
            Err(EndpointError::Unavailable)
        }
    }

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

    fn lease(destination: SessionId) -> RevocableRouteLease {
        RevocableRouteLease::test(
            destination,
            ReconnectGeneration::new(4).unwrap(),
            Arc::new(TestEndpoint),
        )
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
        let mut table = OperationTable::new(1, 1).unwrap();
        let first = operation("first", 10);
        table
            .begin(&first, SessionId(1), lease(SessionId(2)), route(), 1)
            .unwrap();
        assert_eq!(
            table
                .begin(&first, SessionId(1), lease(SessionId(2)), route(), 1)
                .unwrap_err(),
            OperationError::DuplicateOperation
        );
        assert_eq!(
            table
                .begin(
                    &operation("second", 10),
                    SessionId(1),
                    lease(SessionId(2)),
                    route(),
                    1,
                )
                .unwrap_err(),
            OperationError::CapacityExceeded
        );
        let mut empty = OperationTable::new(1, 1).unwrap();
        assert_eq!(
            empty
                .begin(
                    &operation("expired", 2),
                    SessionId(1),
                    lease(SessionId(2)),
                    route(),
                    2,
                )
                .unwrap_err(),
            OperationError::DeadlineExceeded
        );
    }

    #[test]
    fn cancellation_is_owner_checked_and_pins_the_reverse_route() {
        let mut table = OperationTable::new(2, 2).unwrap();
        let operation = operation("cancel-me", 10);
        let expected_route = route();
        let cancellation = table
            .begin(
                &operation,
                SessionId(1),
                lease(SessionId(2)),
                expected_route.clone(),
                1,
            )
            .unwrap();
        assert_eq!(
            table.route_for_cancel(operation.id(), SessionId(9)),
            Err(OperationError::OperationNotFound)
        );
        assert_eq!(
            table
                .route_for_cancel(operation.id(), SessionId(1))
                .unwrap(),
            expected_route
        );
        let target = table.cancel(operation.id(), SessionId(1)).unwrap().unwrap();
        assert_eq!(target.route, expected_route);
        assert_eq!(target.generation, ReconnectGeneration::new(4).unwrap());
        assert!(target.cancellation.is_cancelled());
        assert!(cancellation.is_cancelled());
        let retry = table.cancel(operation.id(), SessionId(1)).unwrap().unwrap();
        assert!(retry.cancellation.is_same_attempt(&target.cancellation));
        table
            .complete_cancel(operation.id(), SessionId(1), &target.cancellation)
            .unwrap();
        assert!(
            table
                .cancel(operation.id(), SessionId(1))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn session_loss_cancels_both_directions() {
        let mut table = OperationTable::new(3, 2).unwrap();
        let first = operation("first", 10);
        let second = operation("second", 10);
        let first_cancel = table
            .begin(&first, SessionId(1), lease(SessionId(2)), route(), 1)
            .unwrap();
        let second_cancel = table
            .begin(&second, SessionId(3), lease(SessionId(1)), route(), 1)
            .unwrap();
        assert_eq!(table.cancel_session(SessionId(1)).len(), 2);
        assert!(first_cancel.is_cancelled());
        assert!(second_cancel.is_cancelled());
    }

    #[test]
    fn finish_and_cancel_errors_do_not_release_another_sessions_operation() {
        let mut table = OperationTable::new(1, 1).unwrap();
        let operation = operation("owned", 5);
        let cancellation = table
            .begin(&operation, SessionId(1), lease(SessionId(2)), route(), 1)
            .unwrap();

        assert_eq!(
            table
                .finish(operation.id(), SessionId(9), &cancellation, 2)
                .unwrap_err(),
            OperationError::OperationNotFound
        );
        assert!(matches!(
            table.cancel(operation.id(), SessionId(9)),
            Err(OperationError::OperationNotFound)
        ));
        assert!(table.route_for_cancel(operation.id(), SessionId(1)).is_ok());
        assert_eq!(
            table
                .finish(operation.id(), SessionId(1), &cancellation, 5)
                .unwrap_err(),
            OperationError::DeadlineExceeded
        );
        assert!(matches!(
            table.cancel(operation.id(), SessionId(1)),
            Err(OperationError::OperationNotFound)
        ));
        assert_eq!(
            table
                .finish(operation.id(), SessionId(1), &cancellation, 6)
                .unwrap_err(),
            OperationError::OperationNotFound
        );
    }

    #[test]
    fn confirmed_cancellation_reclaims_quota_and_keeps_bounded_retry_state() {
        let mut table = OperationTable::new(1, 1).unwrap();
        let first = operation("first", 10);
        table
            .begin(&first, SessionId(1), lease(SessionId(2)), route(), 1)
            .unwrap();
        let first_target = table.cancel(first.id(), SessionId(1)).unwrap().unwrap();
        table
            .complete_cancel(first.id(), SessionId(1), &first_target.cancellation)
            .unwrap();
        assert!(
            table
                .cancellation_complete(first.id(), SessionId(1))
                .unwrap()
        );

        let second = operation("second", 10);
        table
            .begin(&second, SessionId(1), lease(SessionId(2)), route(), 1)
            .unwrap();
        let second_target = table.cancel(second.id(), SessionId(1)).unwrap().unwrap();
        table
            .complete_cancel(second.id(), SessionId(1), &second_target.cancellation)
            .unwrap();

        assert_eq!(
            table.cancellation_complete(first.id(), SessionId(1)),
            Err(OperationError::OperationNotFound)
        );
        assert!(
            table
                .cancellation_complete(second.id(), SessionId(1))
                .unwrap()
        );
    }

    #[test]
    fn reused_id_is_isolated_from_stale_cancel_completion_at_capacity_one() {
        let mut table = OperationTable::new(1, 1).unwrap();
        let operation = operation("reused", 10);
        let first = table
            .begin(&operation, SessionId(1), lease(SessionId(2)), route(), 1)
            .unwrap();
        let first_target = table.cancel(operation.id(), SessionId(1)).unwrap().unwrap();
        table
            .complete_cancel(operation.id(), SessionId(1), &first_target.cancellation)
            .unwrap();

        let second = table
            .begin(&operation, SessionId(1), lease(SessionId(2)), route(), 1)
            .unwrap();
        table
            .complete_cancel(operation.id(), SessionId(1), &first_target.cancellation)
            .unwrap();
        assert!(
            !table
                .cancellation_complete(operation.id(), SessionId(1))
                .unwrap()
        );
        assert_eq!(
            table
                .finish(operation.id(), SessionId(1), &first, 2)
                .unwrap_err(),
            OperationError::Cancelled
        );

        let second_target = table.cancel(operation.id(), SessionId(1)).unwrap().unwrap();
        assert!(second_target.cancellation.is_same_attempt(&second));
        table
            .complete_cancel(operation.id(), SessionId(1), &second_target.cancellation)
            .unwrap();
    }

    #[test]
    fn cancel_and_destination_teardown_share_one_terminal_transition() {
        let mut table = OperationTable::new(1, 1).unwrap();
        let operation = operation("race", 10);
        table
            .begin(&operation, SessionId(1), lease(SessionId(2)), route(), 1)
            .unwrap();
        let explicit = table.cancel(operation.id(), SessionId(1)).unwrap().unwrap();
        let teardown = table.cancel_session(SessionId(2));
        assert_eq!(teardown.len(), 1);
        assert!(
            teardown[0]
                .target
                .cancellation
                .is_same_attempt(&explicit.cancellation)
        );

        table
            .complete_cancel(operation.id(), SessionId(1), &explicit.cancellation)
            .unwrap();
        table
            .complete_cancel(
                operation.id(),
                SessionId(1),
                &teardown[0].target.cancellation,
            )
            .unwrap();
        assert!(
            table
                .cancel(operation.id(), SessionId(1))
                .unwrap()
                .is_none()
        );
    }
}
