//! Revision-log replay and bounded watch admission.
//!
//! The Zone resource API owns the named-stream protocol.  This module owns
//! the storage-side cursor, replay, and delivery accounting primitives that
//! protocol uses.  The writer actor calls these functions while it owns the
//! ordering boundary, so a replay can be registered before the next commit is
//! dispatched without opening a replay/live gap.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use d2b_contracts::v3::{ResourceName, ResourceTypeName, ZoneRevision};
use d2b_resource_store::{StoreError, StoreFilter};
use redb::{Database, ReadableDatabase};
use tokio::sync::mpsc;

use crate::actor::{SharedChangeBatch, filter_batch_with};
use crate::transaction::{ChangeBatch, REVISION_LOG, decode, revision_key};

/// One global bounded admission budget for queued watch deliveries.
pub const WATCH_ADMISSION_CAPACITY: usize = 1024;
/// Maximum initial credit window accepted for one watch.
pub const MAX_INITIAL_WATCH_CREDITS: u32 = WATCH_ADMISSION_CAPACITY as u32;
/// Maximum retained resume cursors after deterministic slow-watcher eviction.
pub const MAX_RETAINED_RESUME_CURSORS: usize = WATCH_ADMISSION_CAPACITY;
/// Maximum simultaneously registered watches.
pub const MAX_WATCH_REGISTRATIONS: usize = WATCH_ADMISSION_CAPACITY;

/// A closed selector carried by one watch registration.
#[derive(Clone, PartialEq, Eq)]
pub struct WatchSelector {
    resource_types: BTreeSet<ResourceTypeName>,
    resource_names: BTreeSet<ResourceName>,
    filters: Vec<StoreFilter>,
}

impl core::fmt::Debug for WatchSelector {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WatchSelector")
            .field("resource_type_count", &self.resource_types.len())
            .field("resource_name_count", &self.resource_names.len())
            .field("filter_count", &self.filters.len())
            .finish()
    }
}

impl WatchSelector {
    /// Construct a selector without retaining caller-owned collection order.
    pub fn new(
        resource_types: impl IntoIterator<Item = ResourceTypeName>,
        resource_names: impl IntoIterator<Item = ResourceName>,
        filters: impl IntoIterator<Item = StoreFilter>,
    ) -> Self {
        let resource_types = resource_types.into_iter().collect::<BTreeSet<_>>();
        let resource_names = resource_names.into_iter().collect::<BTreeSet<_>>();
        let mut filters = filters.into_iter().collect::<Vec<_>>();
        filters.sort_by(|left, right| {
            left.field
                .cmp(&right.field)
                .then_with(|| left.values.cmp(&right.values))
        });
        Self {
            resource_types,
            resource_names,
            filters,
        }
    }

    /// Match one persisted change without inspecting its payload.
    pub(crate) fn matches(&self, entry: &crate::transaction::ChangeEntry) -> bool {
        if !self.resource_types.is_empty()
            && !self.resource_types.contains(entry.resource_type())
        {
            return false;
        }
        if !self.resource_names.is_empty() && !self.resource_names.contains(entry.resource_name())
        {
            return false;
        }
        self.filters.iter().all(|filter| match filter.field.as_str() {
            "metadata.name" => filter
                .values
                .iter()
                .any(|value| value == entry.resource_name().as_str()),
            "type" => filter
                .values
                .iter()
                .any(|value| value == entry.resource_type().as_str()),
            _ => false,
        })
    }
}

/// Opaque identifier for a live registration.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WatchRegistrationId(u64);

impl core::fmt::Debug for WatchRegistrationId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("WatchRegistrationId(<opaque>)")
    }
}

impl WatchRegistrationId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Receiver returned by the storage-side admission helper.
pub struct WatchStream {
    id: WatchRegistrationId,
    receiver: mpsc::Receiver<SharedChangeBatch>,
}

impl core::fmt::Debug for WatchStream {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WatchStream")
            .field("registration", &self.id)
            .finish()
    }
}

impl WatchStream {
    pub const fn id(&self) -> WatchRegistrationId {
        self.id
    }

    pub async fn recv(&mut self) -> Option<SharedChangeBatch> {
        self.receiver.recv().await
    }
}

/// Fixed-cardinality watch saturation signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchSignals {
    pub current_registrations: u64,
    pub budget_used: u64,
    pub budget_capacity: u64,
    pub admission_rejections: u64,
    pub slow_watcher_evictions: u64,
    pub replay_work: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ReplaySignals {
    range_seeks: u64,
    rows_scanned: u64,
    rows_decoded: u64,
}

impl ReplaySignals {
    pub(crate) const fn range_seeks(self) -> u64 {
        self.range_seeks
    }

    pub(crate) const fn rows_scanned(self) -> u64 {
        self.rows_scanned
    }

    pub(crate) const fn rows_decoded(self) -> u64 {
        self.rows_decoded
    }
}

struct Registration {
    selector: WatchSelector,
    credits: usize,
    cursor: u64,
    last_delivered: u64,
    pending: VecDeque<u64>,
    sender: mpsc::Sender<SharedChangeBatch>,
}

/// Storage-side watch coordinator with one global queued-delivery budget.
pub struct WatchCoordinator {
    next_id: u64,
    registrations: BTreeMap<WatchRegistrationId, Registration>,
    budget_used: usize,
    admission_rejections: u64,
    slow_watcher_evictions: u64,
    replay_work: u64,
    evicted_cursors: VecDeque<(WatchRegistrationId, u64)>,
}

impl core::fmt::Debug for WatchCoordinator {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WatchCoordinator")
            .field("registration_count", &self.registrations.len())
            .field("budget_used", &self.budget_used)
            .field("budget_capacity", &WATCH_ADMISSION_CAPACITY)
            .finish()
    }
}

impl Default for WatchCoordinator {
    fn default() -> Self {
        Self {
            next_id: 0,
            registrations: BTreeMap::new(),
            budget_used: 0,
            admission_rejections: 0,
            slow_watcher_evictions: 0,
            replay_work: 0,
            evicted_cursors: VecDeque::new(),
        }
    }
}

impl WatchCoordinator {
    /// Admit one watch and allocate its bounded receiver.
    pub fn admit(
        &mut self,
        after_revision: ZoneRevision,
        selector: WatchSelector,
        initial_credits: u32,
    ) -> Result<WatchStream, StoreError> {
        if initial_credits == 0 || initial_credits > MAX_INITIAL_WATCH_CREDITS {
            self.admission_rejections = self.admission_rejections.saturating_add(1);
            return Err(crate::transaction::backpressure());
        }
        let (sender, receiver) = mpsc::channel(
            usize::try_from(initial_credits)
                .map_err(|_| crate::transaction::integrity("watch-credits-invalid"))?,
        );
        let id = self.register(after_revision, selector, initial_credits, sender)?;
        Ok(WatchStream { id, receiver })
    }

    /// Register a caller-owned sender.  The writer uses this form when the
    /// named-stream layer owns the receiver.
    pub fn register(
        &mut self,
        after_revision: ZoneRevision,
        selector: WatchSelector,
        initial_credits: u32,
        sender: mpsc::Sender<SharedChangeBatch>,
    ) -> Result<WatchRegistrationId, StoreError> {
        if initial_credits == 0 || initial_credits > MAX_INITIAL_WATCH_CREDITS {
            self.admission_rejections = self.admission_rejections.saturating_add(1);
            return Err(crate::transaction::backpressure());
        }
        if self.registrations.len() >= MAX_WATCH_REGISTRATIONS {
            self.admission_rejections = self.admission_rejections.saturating_add(1);
            return Err(crate::transaction::backpressure());
        }
        let id = WatchRegistrationId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| crate::transaction::integrity("watch-registration-exhausted"))?;
        self.registrations.insert(
            id,
            Registration {
                selector,
                credits: usize::try_from(initial_credits)
                    .map_err(|_| crate::transaction::integrity("watch-credits-invalid"))?,
                cursor: after_revision.get(),
                last_delivered: after_revision.get(),
                pending: VecDeque::new(),
                sender,
            },
        );
        Ok(id)
    }

    /// Deliver one already-decoded immutable batch to matching registrations.
    pub fn dispatch(&mut self, batch: SharedChangeBatch) {
        let ids = self.registrations.keys().copied().collect::<Vec<_>>();
        for id in ids {
            let Some(selector) = self.registrations.get(&id).map(|entry| entry.selector.clone())
            else {
                continue;
            };
            let Some(filtered) = filter_batch_with(batch.batch_arc(), |entry| {
                selector.matches(entry)
            }) else {
                continue;
            };
            let _ = self.enqueue(id, filtered, false);
        }
    }

    /// Deliver one replay row while retaining only the caller's cursor state.
    pub fn enqueue_replay(
        &mut self,
        id: WatchRegistrationId,
        batch: SharedChangeBatch,
    ) -> Result<(), StoreError> {
        self.replay_work = self.replay_work.saturating_add(1);
        self.enqueue(id, batch, true)
    }

    /// Acknowledge all queued rows through one revision and release budget.
    pub fn acknowledge(
        &mut self,
        id: WatchRegistrationId,
        revision: ZoneRevision,
    ) -> Result<(), StoreError> {
        let Some(registration) = self.registrations.get_mut(&id) else {
            return Err(crate::transaction::integrity("watch-registration-missing"));
        };
        let revision = revision.get();
        if revision > registration.last_delivered {
            return Err(crate::transaction::integrity("watch-ack-beyond-delivery"));
        }
        if revision <= registration.cursor {
            return Ok(());
        }
        let mut released = 0_usize;
        while registration
            .pending
            .front()
            .is_some_and(|pending| *pending <= revision)
        {
            registration.pending.pop_front();
            released += 1;
        }
        registration.cursor = revision;
        self.budget_used = self.budget_used.saturating_sub(released);
        Ok(())
    }

    /// Remove one registration without counting it as a slow watcher.
    pub fn unregister(&mut self, id: WatchRegistrationId) -> Option<ZoneRevision> {
        self.remove_registration(id, false)
            .map(|cursor| ZoneRevision::new(cursor))
    }

    /// Return the last acknowledged cursor for an active or evicted watch.
    pub fn resume_cursor(&self, id: WatchRegistrationId) -> Option<ZoneRevision> {
        self.registrations
            .get(&id)
            .map(|registration| ZoneRevision::new(registration.cursor))
            .or_else(|| {
                self.evicted_cursors
                    .iter()
                    .find(|(candidate, _)| *candidate == id)
                    .map(|(_, cursor)| ZoneRevision::new(*cursor))
            })
    }

    /// Read and remove a retained cursor after a slow-watcher eviction.
    pub fn take_resume_cursor(&mut self, id: WatchRegistrationId) -> Option<ZoneRevision> {
        let position = self
            .evicted_cursors
            .iter()
            .position(|(candidate, _)| *candidate == id)?;
        self.evicted_cursors
            .remove(position)
            .map(|(_, cursor)| ZoneRevision::new(cursor))
    }

    pub fn signals(&self) -> WatchSignals {
        WatchSignals {
            current_registrations: self.registrations.len() as u64,
            budget_used: self.budget_used as u64,
            budget_capacity: WATCH_ADMISSION_CAPACITY as u64,
            admission_rejections: self.admission_rejections,
            slow_watcher_evictions: self.slow_watcher_evictions,
            replay_work: self.replay_work,
        }
    }

    /// Register and replay under a writer-owned ordering boundary.
    ///
    /// The caller must invoke this from the same serialized writer context
    /// that commits changes.  That ordering is what makes registration plus
    /// replay a no-gap operation.
    pub fn register_and_replay(
        &mut self,
        database: &Database,
        after_revision: ZoneRevision,
        selector: WatchSelector,
        initial_credits: u32,
    ) -> Result<(WatchStream, ZoneRevision), StoreError> {
        let meta = crate::transaction::current_meta(database)?;
        if after_revision.get() < meta.compaction_floor {
            return Err(crate::transaction::revision_expired(meta.current_revision));
        }
        if initial_credits == 0 || initial_credits > MAX_INITIAL_WATCH_CREDITS {
            self.admission_rejections = self.admission_rejections.saturating_add(1);
            return Err(crate::transaction::backpressure());
        }
        let (sender, receiver) = mpsc::channel(
            usize::try_from(initial_credits)
                .map_err(|_| crate::transaction::integrity("watch-credits-invalid"))?,
        );
        let id = self.register(after_revision, selector, initial_credits, sender)?;
        let mut replay = ReplaySignals::default();
        let replay_result = stream_after(database, after_revision.get(), &mut replay, |batch| {
            let batch = Arc::new(batch);
            let Some(selector) = self.registrations.get(&id).map(|entry| entry.selector.clone())
            else {
                return Err(crate::transaction::integrity(
                    "watch-registration-missing",
                ));
            };
            let Some(filtered) = filter_batch_with(batch, |entry| selector.matches(entry)) else {
                return Ok(());
            };
            self.enqueue_replay(id, filtered)
        });
        if let Err(error) = replay_result {
            self.unregister(id);
            return Err(error);
        }
        Ok((
            WatchStream { id, receiver },
            ZoneRevision::new(meta.current_revision),
        ))
    }

    fn enqueue(
        &mut self,
        id: WatchRegistrationId,
        batch: SharedChangeBatch,
        replay: bool,
    ) -> Result<(), StoreError> {
        let revision = batch.revision().get();
        let Some((sender, credits, pending_len)) = self.registrations.get(&id).map(|registration| {
            (
                registration.sender.clone(),
                registration.credits,
                registration.pending.len(),
            )
        }) else {
            return Err(crate::transaction::integrity("watch-registration-missing"));
        };

        if pending_len >= credits {
            self.remove_registration(id, true);
            return Err(crate::transaction::backpressure());
        }
        if self.budget_used >= WATCH_ADMISSION_CAPACITY {
            self.evict_slowest();
            if self.budget_used >= WATCH_ADMISSION_CAPACITY {
                if replay {
                    self.admission_rejections = self.admission_rejections.saturating_add(1);
                }
                return Err(crate::transaction::backpressure());
            }
        }

        match sender.try_send(batch) {
            Ok(()) => {
                let Some(registration) = self.registrations.get_mut(&id) else {
                    return Err(crate::transaction::integrity(
                        "watch-registration-missing",
                    ));
                };
                registration.pending.push_back(revision);
                registration.last_delivered = registration.last_delivered.max(revision);
                self.budget_used += 1;
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.remove_registration(id, true);
                Err(crate::transaction::backpressure())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.remove_registration(id, false);
                Err(crate::transaction::integrity("watch-stream-closed"))
            }
        }
    }

    fn evict_slowest(&mut self) {
        let candidate = self
            .registrations
            .iter()
            .filter(|(_, registration)| !registration.pending.is_empty())
            .min_by_key(|(id, registration)| (registration.cursor, id.0))
            .map(|(id, _)| *id);
        if let Some(id) = candidate {
            self.remove_registration(id, true);
        }
    }

    fn remove_registration(&mut self, id: WatchRegistrationId, slow: bool) -> Option<u64> {
        let registration = self.registrations.remove(&id)?;
        self.budget_used = self
            .budget_used
            .saturating_sub(registration.pending.len());
        if slow {
            self.slow_watcher_evictions = self.slow_watcher_evictions.saturating_add(1);
            self.evicted_cursors.push_back((id, registration.cursor));
            while self.evicted_cursors.len() > MAX_RETAINED_RESUME_CURSORS {
                self.evicted_cursors.pop_front();
            }
        }
        Some(registration.cursor)
    }
}

/// Stream only rows after `after_revision` using the ordered revision key.
///
/// The visitor receives one decoded row at a time.  Older rows are excluded
/// by the key range before their values are read or decoded.
pub(crate) fn stream_after<F>(
    database: &Database,
    after_revision: u64,
    signals: &mut ReplaySignals,
    mut visit: F,
) -> Result<(), StoreError>
where
    F: FnMut(ChangeBatch) -> Result<(), StoreError>,
{
    let Some(first) = after_revision.checked_add(1) else {
        return Ok(());
    };
    let read = database
        .begin_read()
        .map_err(crate::transaction::integrity)?;
    let table = read
        .open_table(REVISION_LOG)
        .map_err(crate::transaction::integrity)?;
    let lower = revision_key(first)?;
    signals.range_seeks = signals.range_seeks.saturating_add(1);
    for row in table
        .range(lower.as_slice()..)
        .map_err(crate::transaction::integrity)?
    {
        let (_, value) = row.map_err(crate::transaction::integrity)?;
        signals.rows_scanned = signals.rows_scanned.saturating_add(1);
        let batch = decode(crate::values::ValueKind::ChangeBatch, value.value())?;
        signals.rows_decoded = signals.rows_decoded.saturating_add(1);
        visit(batch)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::{ResourceGeneration, ResourceUid};

    fn batch(revision: u64) -> SharedChangeBatch {
        let entry = crate::transaction::ChangeEntry::new(
            0,
            ResourceTypeName::parse("Process").unwrap(),
            ResourceName::parse("worker").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            crate::transaction::ChangeEvent::Created,
            None,
            Some(ResourceGeneration::new(1).unwrap()),
            None,
            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_owned(),
            None,
            "operation".to_owned(),
            "correlation".to_owned(),
        )
        .unwrap();
        crate::actor::filter_batch(
            Arc::new(ChangeBatch::new(ZoneRevision::new(revision), vec![entry]).unwrap()),
            &BTreeSet::from([ResourceTypeName::parse("Process").unwrap()]),
        )
        .unwrap()
    }

    #[test]
    fn budget_eviction_releases_entries_and_retains_ack_cursor() {
        let mut coordinator = WatchCoordinator::default();
        let selector = WatchSelector::new(
            [ResourceTypeName::parse("Process").unwrap()],
            [],
            [],
        );
        let mut stream = coordinator.admit(ZoneRevision::new(0), selector, 1).unwrap();
        let first = batch(1);
        coordinator.dispatch(first);
        assert_eq!(coordinator.signals().budget_used, 1);
        coordinator.dispatch(batch(2));
        let signals = coordinator.signals();
        assert_eq!(signals.budget_used, 0);
        assert_eq!(signals.current_registrations, 0);
        assert_eq!(signals.slow_watcher_evictions, 1);
        assert_eq!(
            coordinator.resume_cursor(stream.id()),
            Some(ZoneRevision::new(0))
        );
        assert!(stream.receiver.try_recv().is_ok());
    }

    #[test]
    fn acknowledgement_releases_global_budget() {
        let mut coordinator = WatchCoordinator::default();
        let selector = WatchSelector::new(
            [ResourceTypeName::parse("Process").unwrap()],
            [],
            [],
        );
        let stream = coordinator.admit(ZoneRevision::new(0), selector, 2).unwrap();
        coordinator.dispatch(batch(1));
        coordinator.acknowledge(stream.id(), ZoneRevision::new(1)).unwrap();
        assert_eq!(coordinator.signals().budget_used, 0);
        assert_eq!(
            coordinator.resume_cursor(stream.id()),
            Some(ZoneRevision::new(1))
        );
    }
}
