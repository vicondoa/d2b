//! The buffer ledger — a pure state machine.
//!
//! This module deliberately contains **no** file descriptors, Smithay types or
//! wayland-client types (plan §5.2). It manipulates opaque ids, reference sets
//! and flags, and emits [`Effect`]s that an adapter executes. That is what makes
//! the safety-critical accounting exhaustively testable without a compositor.
//!
//! # The invariant
//!
//! `wl_buffer.release` is emitted for an epoch **exactly once**, and only when
//! the epoch is owed a release, is nobody's front buffer, and has no downstream
//! references in any state.
//!
//! # Why each rule exists
//!
//! Earlier designs of this ledger were rejected by review for three distinct
//! unsafe behaviours, all preserved here as regression rules:
//!
//! * Releasing "the previous front on replacement" let the application mutate
//!   memory the compositor was still sampling.
//! * Releasing once per `wl_buffer` *object* stalled any client that reuses
//!   buffers — which is to say, essentially every client.
//! * Retiring references orphaned by a crash on a timer could never prove the
//!   compositor had stopped reading. Elapsed time is not evidence.

use std::collections::{HashMap, HashSet};

use super::ids::{AppBufferId, BackingId, BufferUseId, Generation, IdAllocator, SurfaceId};

/// Outputs a surface occupied when a reference was submitted.
///
/// Recorded because a replacement presented on a *different* output says
/// nothing about the output that previously scanned out this buffer.
pub type OutputSet = smallvec::SmallVec<[u32; 2]>;

/// One downstream reference, keyed individually.
///
/// Counts-per-generation were rejected: a count cannot say *which* reference an
/// incoming signal retires.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DownRef {
    pub generation: Generation,
    pub surface: SurfaceId,
    pub use_id: BufferUseId,
    /// This surface's commit sequence, so repeated commits are distinguishable.
    pub seq: u32,
}

/// Lifecycle of a downstream reference.
///
/// `Imported` exists because creating a host `wl_buffer` does **not** mean the
/// compositor ever read it; without this state a buffer created and then
/// abandoned would leave the ledger waiting forever for a release that cannot
/// arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownState {
    /// Sent to the frontend; import outcome unknown.
    Reserved,
    /// Host `wl_buffer` created, not yet submitted.
    Imported,
    /// Frontend declared intent to commit; the compositor may read it.
    HostHeld,
    /// Orphaned by an unclean exit. Never retired before session end.
    Quarantined,
}

#[derive(Debug)]
struct BufferUse {
    app_buffer: AppBufferId,
    #[allow(dead_code)]
    backing: BackingId,
    front_of: HashSet<SurfaceId>,
    down: HashMap<DownRef, DownState>,
    release_owed: bool,
}

impl BufferUse {
    fn is_drained(&self) -> bool {
        self.front_of.is_empty() && self.down.is_empty()
    }
}

/// Something the adapter layer must carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Emit `wl_buffer.release` to the application for this buffer.
    ReleaseAppBuffer {
        app_buffer: AppBufferId,
        use_id: BufferUseId,
    },
}

/// Rejected operations. These are contained, never panics (plan §5.3).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LedgerError {
    #[error("unknown buffer use")]
    UnknownUse,
    #[error("unknown downstream reference")]
    UnknownRef,
    #[error("illegal downstream state transition")]
    IllegalTransition,
    #[error("ledger quota exceeded")]
    QuotaExceeded,
}

/// Bounds so an untrusted peer cannot exhaust host memory (plan §5.3).
#[derive(Debug, Clone, Copy)]
pub struct LedgerQuota {
    pub max_uses: usize,
    pub max_refs_per_use: usize,
}

impl Default for LedgerQuota {
    fn default() -> Self {
        Self {
            max_uses: 4096,
            max_refs_per_use: 256,
        }
    }
}

#[derive(Debug)]
pub struct Ledger {
    uses: HashMap<BufferUseId, BufferUse>,
    /// The currently-open epoch for an app buffer, if it is still busy.
    active_epoch: HashMap<AppBufferId, BufferUseId>,
    /// Current front buffer per surface.
    front: HashMap<SurfaceId, BufferUseId>,
    ids: IdAllocator,
    quota: LedgerQuota,
    quarantined: usize,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::new(LedgerQuota::default())
    }
}

impl Ledger {
    pub fn new(quota: LedgerQuota) -> Self {
        Self {
            uses: HashMap::new(),
            active_epoch: HashMap::new(),
            front: HashMap::new(),
            ids: IdAllocator::new(),
            quota,
            quarantined: 0,
        }
    }

    /// Number of references currently quarantined by an unclean exit.
    ///
    /// Surfaced as `degraded(quarantined:N)` so the cost of a forced kill is
    /// visible rather than hidden.
    pub fn quarantined(&self) -> usize {
        self.quarantined
    }

    pub fn live_uses(&self) -> usize {
        self.uses.len()
    }

    fn state_of(&self, r: &DownRef) -> Option<DownState> {
        self.uses
            .get(&r.use_id)
            .and_then(|u| u.down.get(r))
            .copied()
    }

    /// The application attached `app_buffer` to `surface` and committed.
    ///
    /// Opens a new epoch if the buffer is idle, otherwise **joins** the active
    /// epoch. Joining matters: a client may legally attach one buffer to two
    /// surfaces, and opening a second epoch would emit a release while the other
    /// surface still reads it.
    pub fn attach(
        &mut self,
        app_buffer: AppBufferId,
        backing: BackingId,
        surface: SurfaceId,
        generation: Generation,
        seq: u32,
    ) -> Result<(BufferUseId, Vec<Effect>), LedgerError> {
        let use_id = match self.active_epoch.get(&app_buffer).copied() {
            // Busy -> join the open epoch.
            Some(existing) if self.uses.contains_key(&existing) => existing,
            // Idle -> open a new epoch.
            _ => {
                if self.uses.len() >= self.quota.max_uses {
                    return Err(LedgerError::QuotaExceeded);
                }
                let id = self.ids.buffer_use();
                self.uses.insert(
                    id,
                    BufferUse {
                        app_buffer,
                        backing,
                        front_of: HashSet::new(),
                        down: HashMap::new(),
                        release_owed: true,
                    },
                );
                self.active_epoch.insert(app_buffer, id);
                id
            }
        };

        let r = DownRef {
            generation,
            surface,
            use_id,
            seq,
        };

        {
            let u = self.uses.get_mut(&use_id).ok_or(LedgerError::UnknownUse)?;
            if u.down.len() >= self.quota.max_refs_per_use {
                return Err(LedgerError::QuotaExceeded);
            }
            u.down.insert(r, DownState::Reserved);
            u.front_of.insert(surface);
        }

        // This surface's previous front is superseded and may now drain.
        let mut effects = Vec::new();
        if let Some(prev) = self.front.insert(surface, use_id)
            && prev != use_id
            && let Some(p) = self.uses.get_mut(&prev)
        {
            p.front_of.remove(&surface);
            effects.extend(self.settle(prev));
        }

        Ok((use_id, effects))
    }

    /// Host `wl_buffer` created. `Reserved -> Imported`.
    pub fn import_created(&mut self, r: &DownRef) -> Result<Vec<Effect>, LedgerError> {
        self.transition(r, DownState::Reserved, Some(DownState::Imported))
    }

    /// Import failed. The reference is removed: no release will ever arrive for
    /// a buffer the compositor never received.
    pub fn import_failed(&mut self, r: &DownRef) -> Result<Vec<Effect>, LedgerError> {
        self.transition(r, DownState::Reserved, None)
    }

    /// Frontend is about to call `wl_surface.commit`. `Imported -> HostHeld`.
    ///
    /// Sent *before* the commit so the conservative state is always reached
    /// first: a death between the two leaves the reference already `HostHeld`,
    /// and it is quarantined rather than silently dropped.
    pub fn host_committed(&mut self, r: &DownRef) -> Result<Vec<Effect>, LedgerError> {
        self.transition(r, DownState::Imported, Some(DownState::HostHeld))
    }

    /// Frontend destroyed a created buffer without ever committing it.
    pub fn import_abandoned(&mut self, r: &DownRef) -> Result<Vec<Effect>, LedgerError> {
        self.transition(r, DownState::Imported, None)
    }

    /// Compositor released the host buffer. `HostHeld -> removed`.
    pub fn buffer_released(&mut self, r: &DownRef) -> Result<Vec<Effect>, LedgerError> {
        self.transition(r, DownState::HostHeld, None)
    }

    fn transition(
        &mut self,
        r: &DownRef,
        expect: DownState,
        to: Option<DownState>,
    ) -> Result<Vec<Effect>, LedgerError> {
        let current = self.state_of(r).ok_or(LedgerError::UnknownRef)?;
        if current == DownState::Quarantined {
            // Quarantine is terminal. Late signals from a dead generation must
            // never resurrect a reference.
            return Err(LedgerError::IllegalTransition);
        }
        if current != expect {
            return Err(LedgerError::IllegalTransition);
        }
        let u = self
            .uses
            .get_mut(&r.use_id)
            .ok_or(LedgerError::UnknownUse)?;
        match to {
            Some(next) => {
                u.down.insert(r.clone(), next);
            }
            None => {
                u.down.remove(r);
            }
        }
        Ok(self.settle(r.use_id))
    }

    /// A surface stopped using its front buffer (destroyed, or committed null).
    pub fn clear_front(&mut self, surface: SurfaceId) -> Vec<Effect> {
        let Some(prev) = self.front.remove(&surface) else {
            return Vec::new();
        };
        if let Some(u) = self.uses.get_mut(&prev) {
            u.front_of.remove(&surface);
        }
        self.settle(prev)
    }

    /// The frontend exited uncleanly.
    ///
    /// Every unresolved reference for that generation is quarantined —
    /// `Reserved` included, because "import unconfirmed" never means "import
    /// certainly absent": the frontend may have imported *and* committed before
    /// its report was durably delivered.
    ///
    /// Callers must drain and apply the socket queue *before* calling this, so
    /// an in-flight report is not lost to the race.
    pub fn quarantine_generation(&mut self, generation: Generation) {
        for u in self.uses.values_mut() {
            for (r, st) in u.down.iter_mut() {
                if r.generation == generation && *st != DownState::Quarantined {
                    *st = DownState::Quarantined;
                    self.quarantined += 1;
                }
            }
        }
        // Deliberately no settle(): a quarantined reference is never retired,
        // so no epoch can drain because of quarantining.
    }

    /// Emit a release if and only if the epoch has fully drained.
    fn settle(&mut self, use_id: BufferUseId) -> Vec<Effect> {
        let Some(u) = self.uses.get(&use_id) else {
            return Vec::new();
        };
        if !u.is_drained() || !u.release_owed {
            return Vec::new();
        }
        let app_buffer = u.app_buffer;
        if let Some(u) = self.uses.get_mut(&use_id) {
            u.release_owed = false;
        }
        // The epoch is closed; a later attach of this buffer opens a fresh one.
        if self.active_epoch.get(&app_buffer) == Some(&use_id) {
            self.active_epoch.remove(&app_buffer);
        }
        self.uses.remove(&use_id);
        vec![Effect::ReleaseAppBuffer { app_buffer, use_id }]
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn setup() -> (Ledger, AppBufferId, BackingId, SurfaceId, Generation) {
        (
            Ledger::default(),
            AppBufferId(10),
            BackingId(20),
            SurfaceId(30),
            Generation(1),
        )
    }

    /// The happy path: one attach, one full round trip, exactly one release.
    #[test]
    fn single_cycle_releases_exactly_once() {
        let (mut l, ab, bk, s, g) = setup();
        let (u, eff) = l.attach(ab, bk, s, g, 0).unwrap();
        assert!(eff.is_empty(), "nothing to release yet");
        let r = DownRef {
            generation: g,
            surface: s,
            use_id: u,
            seq: 0,
        };
        assert!(l.import_created(&r).unwrap().is_empty());
        assert!(l.host_committed(&r).unwrap().is_empty());
        // Still front, so still not released.
        assert!(l.buffer_released(&r).unwrap().is_empty());
        // Only once it is nobody's front does the release fire.
        let eff = l.clear_front(s);
        assert_eq!(
            eff,
            vec![Effect::ReleaseAppBuffer {
                app_buffer: ab,
                use_id: u
            }]
        );
    }

    /// Regression: a client reusing one `wl_buffer` must get a release *every*
    /// cycle. Releasing once per object stalls any double-buffered client.
    #[test]
    fn buffer_reuse_releases_every_epoch() {
        let (mut l, ab, bk, s, g) = setup();
        let mut releases = 0;
        for seq in 0..500u32 {
            let (u, _) = l.attach(ab, bk, s, g, seq).unwrap();
            let r = DownRef {
                generation: g,
                surface: s,
                use_id: u,
                seq,
            };
            l.import_created(&r).unwrap();
            l.host_committed(&r).unwrap();
            l.buffer_released(&r).unwrap();
            releases += l.clear_front(s).len();
        }
        assert_eq!(releases, 500, "one release per epoch, never fewer");
        assert_eq!(l.live_uses(), 0, "no epoch leaked");
    }

    /// Regression: the same buffer on two surfaces joins ONE epoch and yields
    /// exactly one release, after both surfaces drain.
    #[test]
    fn shared_buffer_two_surfaces_joins_one_epoch() {
        let (mut l, ab, bk, s1, g) = setup();
        let s2 = SurfaceId(31);

        let (u1, _) = l.attach(ab, bk, s1, g, 0).unwrap();
        let (u2, _) = l.attach(ab, bk, s2, g, 0).unwrap();
        assert_eq!(u1, u2, "second attach must JOIN the active epoch");

        let r1 = DownRef {
            generation: g,
            surface: s1,
            use_id: u1,
            seq: 0,
        };
        let r2 = DownRef {
            generation: g,
            surface: s2,
            use_id: u1,
            seq: 0,
        };
        for r in [&r1, &r2] {
            l.import_created(r).unwrap();
            l.host_committed(r).unwrap();
        }
        l.buffer_released(&r1).unwrap();
        // First surface drained, but the second still holds it.
        let eff = l.clear_front(s1);
        assert!(eff.is_empty(), "must NOT release while s2 still reads it");

        l.buffer_released(&r2).unwrap();
        let eff = l.clear_front(s2);
        assert_eq!(eff.len(), 1, "exactly one release once both drain");
    }

    /// A failed import must not leave a dangling reference awaiting a release
    /// that can never arrive.
    #[test]
    fn import_failure_retires_the_reference() {
        let (mut l, ab, bk, s, g) = setup();
        let (u, _) = l.attach(ab, bk, s, g, 0).unwrap();
        let r = DownRef {
            generation: g,
            surface: s,
            use_id: u,
            seq: 0,
        };
        l.import_failed(&r).unwrap();
        let eff = l.clear_front(s);
        assert_eq!(eff.len(), 1, "epoch drains despite the failed import");
    }

    /// A created-but-never-committed buffer must also retire cleanly.
    #[test]
    fn abandoned_import_retires_the_reference() {
        let (mut l, ab, bk, s, g) = setup();
        let (u, _) = l.attach(ab, bk, s, g, 0).unwrap();
        let r = DownRef {
            generation: g,
            surface: s,
            use_id: u,
            seq: 0,
        };
        l.import_created(&r).unwrap();
        l.import_abandoned(&r).unwrap();
        assert_eq!(l.clear_front(s).len(), 1);
    }

    /// The load-bearing safety property: after an unclean exit, no reference of
    /// that generation is ever released to the application — in ANY state,
    /// including `Reserved`.
    #[test]
    fn quarantine_never_releases_in_any_state() {
        for state_depth in 0..3 {
            let (mut l, ab, bk, s, g) = setup();
            let (u, _) = l.attach(ab, bk, s, g, 0).unwrap();
            let r = DownRef {
                generation: g,
                surface: s,
                use_id: u,
                seq: 0,
            };
            if state_depth >= 1 {
                l.import_created(&r).unwrap();
            }
            if state_depth >= 2 {
                l.host_committed(&r).unwrap();
            }

            l.quarantine_generation(g);
            assert_eq!(l.quarantined(), 1);

            // Even clearing the front must not release it.
            let eff = l.clear_front(s);
            assert!(
                eff.is_empty(),
                "quarantined reference released at depth {state_depth}"
            );
        }
    }

    /// Late signals from a dead generation must not resurrect a reference.
    #[test]
    fn quarantine_is_terminal() {
        let (mut l, ab, bk, s, g) = setup();
        let (u, _) = l.attach(ab, bk, s, g, 0).unwrap();
        let r = DownRef {
            generation: g,
            surface: s,
            use_id: u,
            seq: 0,
        };
        l.import_created(&r).unwrap();
        l.quarantine_generation(g);
        assert_eq!(
            l.host_committed(&r),
            Err(LedgerError::IllegalTransition),
            "a quarantined reference must not transition"
        );
        assert_eq!(l.buffer_released(&r), Err(LedgerError::IllegalTransition));
    }

    /// Out-of-order or duplicated signals are rejected, never panic.
    #[test]
    fn illegal_transitions_are_contained() {
        let (mut l, ab, bk, s, g) = setup();
        let (u, _) = l.attach(ab, bk, s, g, 0).unwrap();
        let r = DownRef {
            generation: g,
            surface: s,
            use_id: u,
            seq: 0,
        };
        // Skipping Imported is illegal.
        assert_eq!(l.host_committed(&r), Err(LedgerError::IllegalTransition));
        l.import_created(&r).unwrap();
        // Duplicate is illegal.
        assert_eq!(l.import_created(&r), Err(LedgerError::IllegalTransition));
    }

    #[test]
    fn unknown_reference_is_rejected() {
        let mut l = Ledger::default();
        let r = DownRef {
            generation: Generation(1),
            surface: SurfaceId(1),
            use_id: BufferUseId(999),
            seq: 0,
        };
        assert_eq!(l.import_created(&r), Err(LedgerError::UnknownRef));
    }

    #[test]
    fn quota_is_enforced() {
        let mut l = Ledger::new(LedgerQuota {
            max_uses: 2,
            max_refs_per_use: 8,
        });
        let bk = BackingId(1);
        let g = Generation(1);
        l.attach(AppBufferId(1), bk, SurfaceId(1), g, 0).unwrap();
        l.attach(AppBufferId(2), bk, SurfaceId(2), g, 0).unwrap();
        assert_eq!(
            l.attach(AppBufferId(3), bk, SurfaceId(3), g, 0).err(),
            Some(LedgerError::QuotaExceeded)
        );
    }
}
