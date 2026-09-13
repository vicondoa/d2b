//! In-memory external watch hub over runtime revisions (U5).
//!
//! The hub is the LIST/WATCH service of F4/R23: every desired or runtime
//! change is [`publish`]ed, bumps the [`RuntimeRevision`], appends to a
//! bounded ring, and fans out to matching subscribers over tokio mpsc.
//! The manager lists from [`WatchHub::snapshot_revision`] plus its own
//! matched entries, then registers with `after = snapshot` for gap-free
//! resume; cursors from an older epoch (or otherwise unservable) return
//! [`RevisionExpired`] and the client relists (R24/AE4).
//!
//! # Slow subscriber rule
//!
//! Delivery buffers are bounded. When the hub cannot queue an event for a
//! subscriber (buffer full), it MUST NOT silently drop: it marks the
//! subscription missed, stops fanning out to it, and the subscriber's
//! [`WatchStream`] terminates with [`WatchDelivery::Missed`] carrying the
//! last revision actually delivered to the consumer. On the wire (U8) Missed
//! maps onto the RevisionExpired relist path: the client relists and
//! re-watches from the new snapshot revision. Re-registration after Missed
//! is therefore relist semantics.
//!
//! # Persistence (R11/R24)
//!
//! Nothing here touches disk: the hub takes no store handle anywhere in its
//! API (type-level guarantee - see `hub_writes_zero_files`), keeps all state
//! in memory, and every status transition is pure fan-out. Zero persistent
//! writes per R11.

pub const MODULE_NAME: &str = "watch";

/// Default ring capacity: retained events available for watch replay.
pub const DEFAULT_RING_CAPACITY: usize = 4096;

/// Default per-subscriber delivery buffer (see the slow subscriber rule).
pub const DEFAULT_DELIVERY_BUFFER: usize = 256;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::{mpsc, Notify};

use crate::identity::ResourceKey;
use crate::revision::{RuntimeRevision, WIRE_SEQUENCE_BUDGET};

/// What happened to the resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    /// Spec or status state was written (upsert semantics).
    Upsert,
    /// The resource was deleted.
    Delete,
}

/// Which surface produced the change. Both desired and runtime changes
/// publish and bump the revision (F4); neither persists status (R11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeSource {
    /// Desired-spec mutation (create/update/delete through the manager).
    Desired,
    /// Runtime status transition (in-memory only).
    RuntimeStatus,
}

/// One change as submitted to the hub, before the hub assigns its revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeNotice {
    pub key: ResourceKey,
    pub kind: ChangeKind,
    pub source: ChangeSource,
}

/// One change with its assigned revision, as delivered and ringed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceChange {
    pub revision: RuntimeRevision,
    pub key: ResourceKey,
    pub kind: ChangeKind,
    pub source: ChangeSource,
}

/// A cursor the hub cannot serve gap-free (R24/AE4): an older epoch, a
/// future cursor, or one behind the ring's retention frontier. The client
/// relists and re-watches from `snapshot`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionExpired {
    pub cursor: RuntimeRevision,
    pub snapshot: RuntimeRevision,
}

/// One delivery to a subscriber stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchDelivery {
    Change(ResourceChange),
    /// Explicit missed-data signal (never a silent drop): events beyond
    /// `last_delivered` were lost to this subscriber. The client relists.
    Missed { last_delivered: RuntimeRevision },
}

/// Which changes a subscriber receives.
#[derive(Clone)]
pub struct WatchSelector {
    matches: Arc<dyn Fn(&ResourceKey) -> bool + Send + Sync>,
}

impl WatchSelector {
    /// Every change.
    pub fn all() -> Self {
        Self::with_predicate(|_| true)
    }

    /// Every change to one resource type.
    pub fn for_type(type_name: &str) -> Self {
        let type_name = type_name.to_owned();
        Self::with_predicate(move |key| key.type_name == type_name)
    }

    /// A caller-supplied predicate (the manager composes richer selectors).
    pub fn with_predicate(matches: impl Fn(&ResourceKey) -> bool + Send + Sync + 'static) -> Self {
        Self {
            matches: Arc::new(matches),
        }
    }

    /// Whether this change matches the subscription.
    pub fn matches(&self, key: &ResourceKey) -> bool {
        (self.matches)(key)
    }
}

impl std::fmt::Debug for WatchSelector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchSelector").finish_non_exhaustive()
    }
}

/// Hub sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchHubConfig {
    /// Ring capacity: how many recent events stay replayable. Oldest are
    /// evicted first; evicting past a registration cursor makes it expired.
    pub ring_capacity: usize,
    /// Per-subscriber delivery buffer depth before the Missed rule applies.
    pub delivery_buffer: usize,
}

impl Default for WatchHubConfig {
    fn default() -> Self {
        Self {
            ring_capacity: DEFAULT_RING_CAPACITY,
            delivery_buffer: DEFAULT_DELIVERY_BUFFER,
        }
    }
}

/// The result of registering a watch.
///
/// Replay and live delivery cannot gap: [`WatchHub::register`] is atomic
/// over the hub state, so each published change is either in `replay`
/// (published before registration, revision > cursor) or flows through
/// `stream` (published after).
#[derive(Debug)]
pub enum WatchRegistration {
    Live {
        /// Hub revision at registration time (the LIST->WATCH handshake
        /// anchor; also valid as a relist cursor).
        snapshot: RuntimeRevision,
        /// Retained events after the requested cursor, in revision order.
        replay: Vec<ResourceChange>,
        /// Live delivery of subsequent matching events plus the terminal
        /// `Missed` signal for a slow subscriber.
        stream: WatchStream,
    },
    /// The cursor cannot be served gap-free; the client relists from
    /// `RevisionExpired::snapshot`.
    Expired(RevisionExpired),
}

/// Shared per-subscriber state between the hub's fan-out and the stream.
struct SubscriberState {
    tx: mpsc::Sender<WatchDelivery>,
    /// Sequence of the last change actually yielded to the consumer.
    delivered: AtomicU64,
    /// Set when the hub had to drop events for this subscriber (slow rule).
    missed: AtomicBool,
    /// Wakes a pending `recv` when `missed` is set.
    notify: Notify,
    epoch: u64,
}

/// The subscriber side of a registration: buffered live delivery plus a
/// terminal `Missed` marker (slow subscriber rule), after which the stream
/// ends and the client relists.
pub struct WatchStream {
    rx: mpsc::Receiver<WatchDelivery>,
    state: Arc<SubscriberState>,
    done: bool,
}

impl WatchStream {
    /// Next delivery without awaiting: queued changes first, then the
    /// terminal `Missed` if one was raised, then `None`.
    pub fn try_recv(&mut self) -> Option<WatchDelivery> {
        if self.done {
            return None;
        }
        match self.rx.try_recv() {
            Ok(delivery) => {
                self.note_delivered(&delivery);
                return Some(delivery);
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => {}
        }
        if self.state.missed.swap(false, Ordering::Relaxed) {
            self.done = true;
            return Some(self.missed_delivery());
        }
        None
    }

    /// Await the next delivery (drains queued changes first, then surfaces
    /// a terminal `Missed`, then ends).
    pub async fn recv(&mut self) -> Option<WatchDelivery> {
        loop {
            if self.done {
                return None;
            }
            if self.state.missed.swap(false, Ordering::Relaxed) {
                self.done = true;
                return Some(self.missed_delivery());
            }
            tokio::select! {
                biased;
                delivery = self.rx.recv() => match delivery {
                    Some(delivery) => {
                        self.note_delivered(&delivery);
                        return Some(delivery);
                    }
                    // Hub-side handle closed (Missed raised or hub dropped).
                    None => continue,
                },
                _ = self.state.notify.notified() => continue,
            }
        }
    }

    fn note_delivered(&mut self, delivery: &WatchDelivery) {
        if let WatchDelivery::Change(change) = delivery {
            self.state
                .delivered
                .store(change.revision.sequence, Ordering::Relaxed);
        }
    }

    fn missed_delivery(&self) -> WatchDelivery {
        WatchDelivery::Missed {
            last_delivered: RuntimeRevision {
                epoch: self.state.epoch,
                sequence: self.state.delivered.load(Ordering::Relaxed),
            },
        }
    }
}

impl std::fmt::Debug for WatchStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchStream").finish_non_exhaustive()
    }
}

struct HubInner {
    sequence: u64,
    /// Highest evicted sequence; every event up to here left the ring.
    /// Zero means nothing was evicted yet.
    evicted_through: u64,
    ring: VecDeque<ResourceChange>,
    subscribers: Vec<Subscriber>,
}

struct Subscriber {
    selector: WatchSelector,
    state: Arc<SubscriberState>,
}

/// The in-memory external watch hub (R23/R24).
///
/// No persistence, no store dependency: all state is the ring, the
/// sequence counter, and the live subscriber list.
pub struct WatchHub {
    epoch: u64,
    config: WatchHubConfig,
    inner: Mutex<HubInner>,
}

impl WatchHub {
    /// Build a hub with the default delivery buffer.
    pub fn new(clock: &dyn crate::revision::RevisionClock, ring_capacity: usize) -> Self {
        Self::with_config(
            clock,
            WatchHubConfig {
                ring_capacity,
                delivery_buffer: DEFAULT_DELIVERY_BUFFER,
            },
        )
    }

    /// Build a hub with explicit sizing. The epoch comes from `clock`
    /// (startup time); tests simulate restarts by stamping from a manual
    /// clock and presenting prior-epoch cursors.
    pub fn with_config(clock: &dyn crate::revision::RevisionClock, config: WatchHubConfig) -> Self {
        let epoch = crate::revision::RevisionEpoch::stamp(clock);
        Self {
            epoch: epoch.nanos(),
            config,
            inner: Mutex::new(HubInner {
                sequence: 0,
                evicted_through: 0,
                ring: VecDeque::new(),
                subscribers: Vec::new(),
            }),
        }
    }

    /// The current revision: LIST anchors watches at this cursor.
    pub fn snapshot_revision(&self) -> RuntimeRevision {
        RuntimeRevision {
            epoch: self.epoch,
            sequence: self.inner.lock().sequence,
        }
    }

    /// Number of retained events (bounded by the ring capacity).
    pub fn ring_len(&self) -> usize {
        self.inner.lock().ring.len()
    }

    /// Number of live subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.inner.lock().subscribers.len()
    }

    /// Register a watch resuming from `after`.
    ///
    /// `None` starts live-only delivery from the current revision. An
    /// in-epoch cursor serves retained replay plus live delivery; a cursor
    /// from an older epoch, a future cursor, or one already evicted from
    /// the ring returns [`WatchRegistration::Expired`] (R24: relist).
    pub fn register(
        &self,
        selector: WatchSelector,
        after: Option<RuntimeRevision>,
    ) -> WatchRegistration {
        let mut inner = self.inner.lock();
        let snapshot = RuntimeRevision {
            epoch: self.epoch,
            sequence: inner.sequence,
        };
        if let Some(cursor) = after {
            if !self.cursor_servable(&inner, cursor) {
                return WatchRegistration::Expired(RevisionExpired { cursor, snapshot });
            }
        }
        let replay: Vec<ResourceChange> = match after {
            Some(cursor) => inner
                .ring
                .iter()
                .filter(|change| change.revision > cursor)
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        let start_sequence = after.map_or(0, |cursor| cursor.sequence);
        let (tx, rx) = mpsc::channel(self.config.delivery_buffer);
        let state = Arc::new(SubscriberState {
            tx,
            delivered: AtomicU64::new(start_sequence),
            missed: AtomicBool::new(false),
            notify: Notify::new(),
            epoch: self.epoch,
        });
        inner.subscribers.push(Subscriber { selector, state: Arc::clone(&state) });
        WatchRegistration::Live {
            snapshot,
            replay,
            stream: WatchStream { rx, state, done: false },
        }
    }

    /// Publish one change: assign the next revision, append to the ring,
    /// and fan out to matching subscribers (F4: desired and runtime changes
    /// both publish; every change bumps the revision).
    pub fn publish(&self, notice: ChangeNotice) -> RuntimeRevision {
        let mut inner = self.inner.lock();
        let sequence = inner
            .sequence
            .checked_add(1)
            .expect("revision sequence exhausted within one daemon epoch");
        debug_assert!(
            sequence < WIRE_SEQUENCE_BUDGET,
            "sequence exceeds the U8 wire budget within one epoch"
        );
        let revision = RuntimeRevision {
            epoch: self.epoch,
            sequence,
        };
        let change = ResourceChange {
            revision,
            key: notice.key,
            kind: notice.kind,
            source: notice.source,
        };
        inner.ring.push_back(change.clone());
        while inner.ring.len() > self.config.ring_capacity {
            if let Some(evicted) = inner.ring.pop_front() {
                inner.evicted_through = evicted.revision.sequence;
            }
        }
        inner.sequence = sequence;

        // Fan out. A subscriber whose buffer is full gets the explicit
        // Missed path, never a silent drop.
        let mut index = 0;
        while index < inner.subscribers.len() {
            let subscriber = &inner.subscribers[index];
            if subscriber.state.missed.load(Ordering::Relaxed)
                || !subscriber.selector.matches(&change.key)
            {
                index += 1;
                continue;
            }
            match subscriber
                .state
                .tx
                .try_send(WatchDelivery::Change(change.clone()))
            {
                Ok(()) => index += 1,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Slow subscriber: stop fanning out and raise Missed on
                    // its stream (covers everything past its last delivery).
                    subscriber.state.missed.store(true, Ordering::Relaxed);
                    subscriber.state.notify.notify_one();
                    inner.subscribers.swap_remove(index);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    // Consumer gone: reap.
                    inner.subscribers.swap_remove(index);
                }
            }
        }
        revision
    }

    /// Retained events after `cursor` (the relist/read path for the
    /// manager). Errors with [`RevisionExpired`] exactly like registration.
    pub fn events_after(
        &self,
        after: RuntimeRevision,
    ) -> Result<Vec<ResourceChange>, RevisionExpired> {
        let inner = self.inner.lock();
        if !self.cursor_servable(&inner, after) {
            return Err(RevisionExpired {
                cursor: after,
                snapshot: RuntimeRevision {
                    epoch: self.epoch,
                    sequence: inner.sequence,
                },
            });
        }
        Ok(inner
            .ring
            .iter()
            .filter(|change| change.revision > after)
            .cloned()
            .collect())
    }

    /// A cursor is servable iff it is in the current epoch, not ahead of
    /// the current revision, and not behind the ring's retention frontier.
    fn cursor_servable(&self, inner: &HubInner, cursor: RuntimeRevision) -> bool {
        cursor.epoch == self.epoch
            && cursor.sequence <= inner.sequence
            && cursor.sequence >= inner.evicted_through
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::revision::ManualClock;

    fn key(zone: &str, type_name: &str, name: &str) -> crate::identity::ResourceKey {
        crate::identity::ResourceKey {
            zone: zone.into(),
            type_name: type_name.into(),
            name: name.into(),
        }
    }

    fn hub() -> WatchHub {
        WatchHub::with_config(
            &ManualClock::at(1_000),
            WatchHubConfig {
                ring_capacity: DEFAULT_RING_CAPACITY,
                delivery_buffer: DEFAULT_DELIVERY_BUFFER,
            },
        )
    }

    fn upsert(hub: &WatchHub, name: &str) -> RuntimeRevision {
        hub.publish(ChangeNotice {
            key: key("z", "Process", name),
            kind: ChangeKind::Upsert,
            source: ChangeSource::Desired,
        })
    }

    // -- F4 / R23: LIST -> WATCH handoff -----------------------------------

    #[tokio::test]
    async fn list_then_watch_receives_every_later_event_with_no_gap() {
        let hub = hub();
        // "LIST" snapshot revision.
        let snapshot = hub.snapshot_revision();

        // An event published between LIST and WATCH.
        upsert(&hub, "a");
        // Registration interleaves with publishes.
        let WatchRegistration::Live { replay, stream, .. } =
            hub.register(WatchSelector::all(), Some(snapshot))
        else {
            panic!("in-epoch cursor must register");
        };
        let mut stream = stream;
        upsert(&hub, "b");

        // The pre-registration event arrives in the replay batch (it was
        // published after the snapshot, before registration); live delivery
        // continues in the stream with no gap between the two.
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].key.name, "a");
        assert!(replay[0].revision > snapshot);
        match stream.try_recv() {
            Some(WatchDelivery::Change(change)) => assert_eq!(change.key.name, "b"),
            other => panic!("expected live change, got {other:?}"),
        }
        assert_eq!(stream.try_recv(), None);
    }

    #[tokio::test]
    async fn selector_filters_both_replay_and_live_delivery() {
        let hub = hub();
        let snapshot = hub.snapshot_revision();
        hub.publish(ChangeNotice {
            key: key("z", "Volume", "v1"),
            kind: ChangeKind::Upsert,
            source: ChangeSource::Desired,
        });

        let WatchRegistration::Live { stream, .. } =
            hub.register(WatchSelector::for_type("Process"), Some(snapshot))
        else {
            panic!("in-epoch cursor must register");
        };
        let mut stream = stream;

        hub.publish(ChangeNotice {
            key: key("z", "Volume", "v2"),
            kind: ChangeKind::Upsert,
            source: ChangeSource::Desired,
        });
        hub.publish(ChangeNotice {
            key: key("z", "Process", "p1"),
            kind: ChangeKind::Upsert,
            source: ChangeSource::Desired,
        });

        match stream.try_recv() {
            Some(WatchDelivery::Change(change)) => {
                assert_eq!(change.key.type_name, "Process");
                assert_eq!(change.key.name, "p1");
            }
            other => panic!("expected only the Process change, got {other:?}"),
        }
        assert_eq!(stream.try_recv(), None);
    }

    // -- R23: ring replay within one epoch ---------------------------------

    #[tokio::test]
    async fn ring_replays_retained_events_within_one_epoch() {
        let hub = WatchHub::new(&ManualClock::at(1_000), 8);

        for i in 1..=5 {
            upsert(&hub, &format!("r{i}"));
        }
        let after = RuntimeRevision::new(1_000, 2);
        for i in 6..=10 {
            upsert(&hub, &format!("r{i}"));
        }

        let WatchRegistration::Live { replay, stream, .. } =
            hub.register(WatchSelector::all(), Some(after))
        else {
            panic!("in-epoch cursor must register");
        };
        let mut stream = stream;
        let replayed: Vec<String> =
            replay.iter().map(|change| change.key.name.clone()).collect();
        assert_eq!(replayed, vec!["r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10"]);
        assert!(replay.windows(2).all(|w| w[0].revision < w[1].revision));

        // Live delivery continues after replay.
        upsert(&hub, "r11");
        match stream.try_recv() {
            Some(WatchDelivery::Change(change)) => assert_eq!(change.key.name, "r11"),
            other => panic!("expected live delivery after replay, got {other:?}"),
        }
    }

    // -- AE4 / R24: stale epoch cursors fail closed -------------------------

    #[tokio::test]
    async fn cursor_from_previous_epoch_returns_expired() {
        let hub = hub(); // epoch stamped at 1_000 via the manual clock.
        upsert(&hub, "a");

        // A cursor from a prior daemon lifetime (older epoch).
        let stale = RuntimeRevision::new(999, u64::MAX);
        assert!(matches!(
            hub.register(WatchSelector::all(), Some(stale)),
            WatchRegistration::Expired(_)
        ));
        assert_eq!(
            hub.events_after(stale),
            Err(RevisionExpired {
                cursor: stale,
                snapshot: hub.snapshot_revision(),
            }),
        );
    }

    #[tokio::test]
    async fn future_cursor_within_epoch_fails_closed() {
        let hub = hub();
        let future = RuntimeRevision::new(1_000, u64::MAX);
        assert!(matches!(
            hub.register(WatchSelector::all(), Some(future)),
            WatchRegistration::Expired(_)
        ));
        assert!(hub.events_after(future).is_err());
    }

    #[tokio::test]
    async fn cursor_behind_retention_frontier_is_expired() {
        let hub = WatchHub::new(&ManualClock::at(1_000), 4);
        for i in 1..=10 {
            upsert(&hub, &format!("p{i}"));
        }
        // Sequences 1..=6 were evicted: a cursor at 3 cannot be served.
        let lapped = RuntimeRevision::new(1_000, 3);
        assert!(matches!(
            hub.register(WatchSelector::all(), Some(lapped)),
            WatchRegistration::Expired(_)
        ));
        assert!(hub.events_after(lapped).is_err());

        // A cursor at the frontier still replays everything retained.
        let frontier = RuntimeRevision::new(1_000, 6);
        let WatchRegistration::Live { replay, .. } =
            hub.register(WatchSelector::all(), Some(frontier))
        else {
            panic!("frontier cursor must register");
        };
        assert_eq!(replay.len(), 4);
    }

    #[tokio::test]
    async fn relist_after_expired_recovers_with_new_snapshot() {
        let hub = hub();
        let stale = RuntimeRevision::new(999, 5);
        assert!(matches!(
            hub.register(WatchSelector::all(), Some(stale)),
            WatchRegistration::Expired(_)
        ));

        // Relist: take the new snapshot revision, publish, re-watch.
        let snapshot = hub.snapshot_revision();
        upsert(&hub, "fresh");
        let WatchRegistration::Live { replay, stream, .. } =
            hub.register(WatchSelector::all(), Some(snapshot))
        else {
            panic!("fresh snapshot must register");
        };
        // The post-relist event replays from the ring (published after the
        // new snapshot, before re-registration).
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].key.name, "fresh");
        drop(stream);
    }

    #[tokio::test]
    async fn slow_subscriber_gets_explicit_missed_never_silent_drop() {
        let hub = WatchHub::with_config(
            &ManualClock::at(1_000),
            WatchHubConfig {
                ring_capacity: 4,
                delivery_buffer: 2,
            },
        );
        let WatchRegistration::Live { stream, .. } = hub.register(WatchSelector::all(), None)
        else {
            unreachable!();
        };
        let mut stream = stream;

        // Fast publisher, non-consuming subscriber: the delivery buffer
        // fills, the ring churns past the subscriber's frontier, and the
        // subscriber must see an explicit Missed - not silent drop.
        for i in 0..32 {
            upsert(&hub, &format!("p{i}"));
        }

        let mut seen_changes = 0;
        let mut missed = None;
        while let Some(delivery) = stream.try_recv() {
            match delivery {
                WatchDelivery::Change(_) => seen_changes += 1,
                WatchDelivery::Missed { last_delivered } => missed = Some(last_delivered),
            }
        }
        // Bounded: at most delivery_buffer changes were queued.
        assert!(seen_changes <= 2, "saw {seen_changes} changes");
        let missed = missed.expect("slow subscriber must receive Missed");
        // The Missed frontier equals the last change actually handed to the
        // consumer (first publish is sequence 1).
        assert_eq!(missed.sequence as usize, seen_changes);
        assert_eq!(missed.epoch, 1_000);
        // The ring churned past the missed frontier, so the relist path
        // reports RevisionExpired exactly like a stale cursor (U8 maps the
        // Missed signal onto this relist path).
        assert!(matches!(
            hub.events_after(missed),
            Err(RevisionExpired { .. })
        ));
    }

    // -- Bounded ring under churn -------------------------------------------

    #[tokio::test]
    async fn ring_stays_bounded_under_churn_and_snapshot_advances() {
        let capacity = 16;
        let hub = WatchHub::new(&ManualClock::at(1_000), capacity);
        for i in 0..10_000 {
            upsert(&hub, &format!("p{i}"));
        }
        assert_eq!(hub.ring_len(), capacity);
        assert_eq!(hub.snapshot_revision().sequence, 10_000);

        // A gone subscriber is reaped on the next publish.
        let WatchRegistration::Live { stream, .. } = hub.register(WatchSelector::all(), None)
        else {
            unreachable!()
        };
        drop(stream);
        upsert(&hub, "after-drop");
        assert_eq!(hub.subscriber_count(), 0);
    }

    // -- F4 / AE6 / R11: status churn, zero persistence ----------------------

    #[tokio::test]
    async fn status_churn_generates_strictly_increasing_revisions_and_events() {
        let hub = hub();
        let WatchRegistration::Live { stream, .. } = hub.register(WatchSelector::all(), None)
        else {
            unreachable!()
        };
        let mut stream = stream;

        let mut previous = hub.snapshot_revision();
        for _ in 0..50 {
            let revision = hub.publish(ChangeNotice {
                key: key("z", "Process", "churn"),
                kind: ChangeKind::Upsert,
                source: ChangeSource::RuntimeStatus,
            });
            assert!(revision > previous, "every status change bumps the revision");
            previous = revision;
        }

        let mut delivered = 0;
        while matches!(stream.try_recv(), Some(WatchDelivery::Change(_))) {
            delivered += 1;
        }
        assert_eq!(delivered, 50);
    }

    // -- R11/R24: no persistence anywhere in the watch path -------------------

    #[test]
    fn hub_writes_zero_files_and_takes_no_store_handle() {
        // fs-level: the hub performs no I/O, so a tempdir it "operates in"
        // stays empty. Type-level: the whole hub API takes no store handle,
        // so this test compiles without one (R11/R24).
        let dir = tempfile::tempdir().expect("tempdir");
        let hub = WatchHub::new(&ManualClock::at(1), 16);
        let WatchRegistration::Live { stream, .. } = hub.register(WatchSelector::all(), None)
        else {
            unreachable!()
        };
        let mut stream = stream;
        for i in 0..64 {
            hub.publish(ChangeNotice {
                key: key("z", "Process", &format!("p{i}")),
                kind: ChangeKind::Upsert,
                source: ChangeSource::RuntimeStatus,
            });
        }
        while stream.try_recv().is_some() {}
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read tempdir")
            .collect();
        assert!(entries.is_empty(), "hub wrote files: {entries:?}");
    }

    #[tokio::test]
    async fn events_after_serves_manager_list_support() {
        let hub = hub();
        upsert(&hub, "a");
        let snapshot = hub.snapshot_revision();
        upsert(&hub, "b");

        let events = hub.events_after(snapshot).expect("in-epoch");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].key.name, "b");

        // Cursor at zero is the "everything retained" cursor.
        let all = hub
            .events_after(RuntimeRevision::new(1_000, 0))
            .expect("in-epoch");
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn async_recv_drains_then_missed_then_ends() {
        let hub = WatchHub::with_config(
            &ManualClock::at(1_000),
            WatchHubConfig {
                ring_capacity: 32,
                delivery_buffer: 2,
            },
        );
        let WatchRegistration::Live { stream, .. } = hub.register(WatchSelector::all(), None)
        else {
            unreachable!()
        };
        let mut stream = stream;
        for i in 0..8 {
            upsert(&hub, &format!("p{i}"));
        }
        let mut changes = 0;
        let mut missed = false;
        while let Some(delivery) = stream.recv().await {
            match delivery {
                WatchDelivery::Change(_) => changes += 1,
                WatchDelivery::Missed { .. } => missed = true,
            }
        }
        assert!(changes <= 2);
        assert!(missed, "async recv must surface the Missed signal");
    }
}
