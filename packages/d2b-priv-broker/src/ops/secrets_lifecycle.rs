//! Per-realm secrets lifecycle: a **pure transaction core** for
//! provision / rotate / rollback / retire, decoupled from any storage
//! or locking adapter via the injected [`SecretsAuthorityPort`] trait.
//!
//! # W8fu6: ports-and-adapters rewrite
//!
//! Rounds 1-5 of this component built a **filesystem-anchored**
//! engine: raw fd-relative paths (`crate::sys::path_safe`), a private
//! `F_OFD_SETLK` cross-process lock, a durable on-disk JSON marker +
//! `txlog`, fsync-heavy crash recovery, and an anchored
//! enumerate-then-delete retirement walk. This round replaces all of
//! that with a **pure transaction core** that never touches a
//! filesystem primitive, a lock, or a byte of JSON directly. Every
//! stateful effect goes through the injected [`SecretsAuthorityPort`]
//! trait, and every crash/fault/concurrency/tamper test in this file
//! runs entirely against the in-memory [`FakeAuthorityPort`] test
//! double below -- there is no integration test in this file that
//! spins up a real directory tree, a real lock, or a real filesystem.
//!
//! ## Why a compare-and-swap fencing token instead of a lock + txlog
//!
//! The prior design's lock (mutual exclusion) and txlog (crash
//! recovery) both existed to make a *sequence* of raw filesystem
//! writes durable and atomic despite crashes and concurrent callers.
//! A storage substrate that already offers atomic compare-and-swap
//! (etcd's `mod_revision`, ZooKeeper's `version`, a database row's
//! optimistic-lock column, or -- for a filesystem adapter of the
//! integrator's own design -- a `rename(2)`-based scheme) makes both
//! unnecessary *from this module's point of view*: every action below
//! reads a [`DurableState`] plus an opaque [`OwnershipEpoch`] fencing
//! token via [`SecretsAuthorityPort::read_state`], computes the next
//! state, and calls [`SecretsAuthorityPort::cas_commit`] with the
//! token it read. Either the CAS succeeds (this call was the only
//! writer the whole time) or it is fenced (some other writer's
//! transition is now current; this call fails cleanly with
//! [`FailReason::OwnershipFenced`] and has mutated nothing). This
//! module never acquires or holds a lock, and never writes a txlog --
//! whatever recovery a concrete adapter needs for its own commit
//! primitive is the adapter's problem, entirely hidden behind
//! [`SecretsAuthorityPort::cas_commit`] returning `Ok`/`Err`
//! atomically.
//!
//! ## What "forward recovery" means without a txlog
//!
//! There is no separate "recovery mode": a caller that observes
//! [`FailReason::OwnershipFenced`] simply re-reads the current state
//! and retries (or gives up) -- there is no crashed, half-applied
//! transaction to detect or unwind, because [`SecretsAuthorityPort::cas_commit`]
//! is defined to be atomic (all-or-nothing) from this module's point
//! of view. "Forward recovery" instead applies to *pruning*: a
//! transition that supersedes a generation (`rotate` superseding the
//! old `previous`, `retire` superseding both `active` and `previous`)
//! commits the superseding [`DurableState`] first and only then
//! attempts to synchronously prune the superseded material via
//! [`SecretsAuthorityPort::prune_material`]. If that synchronous prune
//! does not fully succeed, the still-owed epochs are recorded in the
//! *already-durably-committed* state's [`DurableState::pending_prune`]
//! list (bounded at [`MAX_PENDING_PRUNE`]) rather than being lost or
//! blocking the caller's success -- every subsequent action for that
//! `(workload, kind)` resolves any outstanding debt (via
//! [`read_and_verify`]) before doing its own work, so the debt is
//! self-healing and monotonically shrinks. This is the module's
//! concrete answer to "never return an error after silently
//! activating unrecoverable state": the CAS commit that activates a
//! new generation and the best-effort prune of the old one are two
//! separate steps, and a failure in the second step is recorded as
//! durable, retriable debt -- never swallowed, and never allowed to
//! turn a successful activation into a reported failure.
//!
//! ## Why deterministic, high-water-keyed epoch allocation
//!
//! A new generation's epoch number is always
//! `state.high_water_epoch + 1` -- never "current epoch + 1" -- so a
//! `rotate` issued after a `rollback` can never collide with (or
//! silently resurrect) a still-materialized, newer-numbered epoch the
//! rollback moved away from. Because the next epoch number is a pure
//! function of the last *durably committed* high-water mark, staging
//! is naturally idempotent by epoch: two calls that stage the same
//! not-yet-committed epoch number simply race on whose bytes are
//! staged last (closed by the post-commit re-verification described
//! below), and there is never a need for the collision-resistant
//! random staging-name scheme rounds 1-5 required.
//!
//! ## Closing the "last stage wins" race
//!
//! [`SecretsAuthorityPort::stage_material`] is defined to run *before*
//! [`SecretsAuthorityPort::cas_commit`], so two concurrent callers
//! that both read the same pre-commit state and compute the same next
//! epoch number can each call `stage_material` for that epoch before
//! either of them calls `cas_commit`. Only one of the two `cas_commit`
//! calls can win the race, but the *order* of the two `stage_material`
//! calls relative to the winning `cas_commit` is not itself
//! CAS-serialized -- the loser's `stage_material` call could still run
//! (and overwrite the winner's staged bytes) *after* the winner's
//! `cas_commit` succeeds, silently corrupting the now-durably-active
//! generation's material without ever going through `cas_commit`
//! again. [`provision`] and [`rotate`] close this window by
//! **immediately re-reading the live digest of the epoch they just
//! committed** and comparing it against the digest they themselves
//! staged: a mismatch means some other writer's `stage_material` call
//! landed after this call's `cas_commit`, so this call quarantines the
//! authority and fails closed with [`FailReason::ChecksumMismatch`]
//! rather than certifying success over corrupted, no-longer-trusted
//! material.
//!
//! ## Canonical typed identity, no legacy VM-name string
//!
//! Every function below takes a [`d2b_contracts::v2_identity::WorkloadId`]
//! -- the same canonical v2 identity type already used elsewhere in
//! this crate (`guest_session_material.rs`,
//! `child_realm_guest_material.rs`) -- rather than a bare `vm_id: &str`
//! or the legacy `d2b_contracts::types::VmId` human-label newtype
//! rounds 1-5 used. `WorkloadId`'s own `parse`/`FromStr` already
//! enforces the canonical bounded-opaque-string shape, so this module
//! has no `valid_vm_id`-style runtime string check of its own to
//! maintain -- an invalid identity simply cannot exist as a
//! `WorkloadId` value in the first place. Every `SecretsAuthorityPort`
//! method is scoped by `(&WorkloadId, SecretKind)`; how (or whether) an
//! adapter maps that pair onto any underlying storage location -- a
//! path, a database key, a KV-store prefix -- is entirely the
//! adapter's concern and never observable here.
//!
//! # Integration wiring points (explicit, not yet performed)
//!
//! This module is a pure library with **zero** side effects of its
//! own -- no filesystem access, no locking, no process spawning. A
//! future integrator must, in follow-up commits **outside this
//! component's ownership**:
//!
//! 1. Implement a concrete [`SecretsAuthorityPort`] adapter over
//!    whatever real durable storage/CAS substrate lands for the
//!    broker (e.g. the ADR 0034 storage/lock contract once it exists,
//!    or a dedicated KV store). This is now the **single dominant**
//!    wiring blocker -- rounds 1-5's many fine-grained filesystem
//!    wiring points (lock file placement, `dir_mode`/`file_mode`,
//!    owner uid/gid, `state_root`) are superseded by this one seam.
//! 2. Add exactly one new `OperationFields::SecretsLifecycle(SecretsLifecycleAuditFields)`
//!    variant to `crate::ops::audit_op::OperationFields` (and a
//!    matching `from_operation_value` arm), and route each
//!    `Ok`/`Err(SecretsLifecycleError)` returned by [`provision`],
//!    [`rotate`], [`rollback`], [`retire`] into `crate::audit::AuditLog`
//!    via that new variant.
//! 3. Add a broker dispatch path (an existing operation-request
//!    enum's new variant, or a new one, per the integrator's chosen
//!    RPC shape) that resolves a caller's `(realm, workload label)`
//!    into a [`d2b_contracts::v2_identity::WorkloadId`] (e.g. via
//!    `WorkloadId::derive`) and a [`SecretMaterial`] payload, then
//!    calls the four public functions below against the concrete
//!    `SecretsAuthorityPort` adapter from (1).
//! 4. Decide and implement whatever real quarantine-clearing
//!    operation exists for [`QuarantineReason`] -- this module
//!    deliberately exposes no "un-quarantine" call (quarantine is a
//!    one-way, fail-closed door from this module's own point of view),
//!    so an operator-facing clear path is an adapter/broker-level
//!    concern.
//! 5. For `SecretKind::SecurityKeyChannelState`, wire the four public
//!    functions to whatever calls into `d2b-sk-frontend`'s
//!    `secrets_channel.rs` need this lifecycle -- see that module's
//!    own "Integration wiring points" note for its side of the seam
//!    (session config wiring, wire schema/authentication, broker
//!    dispatch mapping, delivery-counter persistence).

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[cfg(test)]
use crate::ops::secrets_rotation_audit::LifecycleResult;
use crate::ops::secrets_rotation_audit::{
    FailReason, LifecycleAction, SecretKind, SecretsLifecycleAuditContext,
    SecretsLifecycleAuditFields,
};
use d2b_contracts::v2_identity::WorkloadId;

// ---------------------------------------------------------------------
// Epoch / fencing primitives
// ---------------------------------------------------------------------

/// A 1-based generation number within one `(workload, kind)` lineage.
/// `0` is reserved and never a valid [`Epoch`] value -- this is the
/// same "epochs start at 1" convention the audit schema's
/// `LineageEpochIsZero` check enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Epoch(u64);

impl Epoch {
    pub const FIRST: Epoch = Epoch(1);

    /// `None` for `0`; every other value is a valid epoch.
    pub fn from_raw(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Epoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An opaque compare-and-swap fencing token returned by
/// [`SecretsAuthorityPort::read_state`] and required (by exact value)
/// by [`SecretsAuthorityPort::cas_commit`]. Distinct from [`Epoch`]:
/// this counts **every** accepted commit for a `(workload, kind)` pair
/// (including `rollback`, which never advances the lineage high-water
/// mark), while [`Epoch`] only counts materialized generations. This
/// module never inspects an [`OwnershipEpoch`]'s value -- it only ever
/// passes back exactly what it was given by the same port instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnershipEpoch(u64);

impl OwnershipEpoch {
    /// The fencing token a port must return from `read_state` for a
    /// `(workload, kind)` pair it has never accepted a commit for.
    pub const NEVER_COMMITTED: OwnershipEpoch = OwnershipEpoch(0);

    pub fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// Bound on [`DurableState::pending_prune`]. `retire` can orphan at
/// most two generations (the former `active` and the former
/// `previous`) in a single transition, and every other action either
/// orphans at most one (`rotate`, superseding the outgoing `previous`)
/// or none (`provision`, `rollback`) -- and [`read_and_verify`] always
/// fully drains any pre-existing debt before an action computes its
/// own transition, so this bound is never exceeded in practice. It
/// exists as a defensive, checked invariant, not a soft guideline.
pub const MAX_PENDING_PRUNE: usize = 2;

fn valid_digest_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

// ---------------------------------------------------------------------
// Durable state
// ---------------------------------------------------------------------

/// One materialized generation's identity as recorded in
/// [`DurableState`]: its epoch number and the SHA-256 hex digest of
/// its material. This module always independently re-verifies this
/// digest against the live authority (via
/// [`SecretsAuthorityPort::material_digest`]) before trusting it for
/// any mutation -- a [`GenerationRecord`] by itself is a claim, not
/// proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationRecord {
    pub epoch: Epoch,
    pub digest_hex: String,
}

impl GenerationRecord {
    pub fn new(epoch: Epoch, digest_hex: impl Into<String>) -> Self {
        Self {
            epoch,
            digest_hex: digest_hex.into(),
        }
    }
}

/// The complete durable state this module ever needs for one
/// `(workload, kind)` pair. This is the *entire* payload
/// [`SecretsAuthorityPort::cas_commit`] ever writes -- there is no
/// separate marker, txlog, or lock-state object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableState {
    /// Monotonic high-water mark: the highest epoch number the
    /// *current, unbroken* lineage (since the last `provision`) has
    /// committed as active. Never decreases within one such lineage --
    /// in particular, `rotate` always allocates `high_water_epoch + 1`
    /// (never `active.epoch + 1`), so a rotate issued after a
    /// `rollback` can never collide with a still-materialized, newer
    /// epoch the rollback moved away from. [`provision`] deliberately
    /// resets this to `1` for a *fresh* lineage: [`read_and_verify`]
    /// unconditionally requires any `pending_prune` debt from a prior
    /// `retire` to be fully drained (every previously-live epoch
    /// confirmed pruned by the authority) before `provision` may run
    /// at all, so reusing epoch `1`'s key can never resurrect a prior
    /// lineage's leftover bytes.
    pub high_water_epoch: u64,
    /// The currently active generation, when one exists. `None` for a
    /// never-provisioned or freshly-retired pair.
    pub active: Option<GenerationRecord>,
    /// The most recently superseded generation still retained for a
    /// possible `rollback`, when one exists. Always `None` when
    /// `active` is `None`.
    pub previous: Option<GenerationRecord>,
    /// `true` exactly for a pair that has been retired and not since
    /// re-provisioned. A retired pair always has `active: None,
    /// previous: None`.
    pub retired: bool,
    /// Epochs this pair's own prior committed transition determined
    /// are superseded and safe to prune, but whose synchronous
    /// best-effort prune attempt (at commit time) did not fully
    /// succeed. Bounded at [`MAX_PENDING_PRUNE`]; never contains the
    /// current `active` or `previous` epoch. See the module doc's
    /// "What forward recovery means" section.
    pub pending_prune: Vec<Epoch>,
}

impl DurableState {
    /// The state a `(workload, kind)` pair that has never been
    /// committed to is defined to have.
    pub fn never_provisioned() -> Self {
        Self {
            high_water_epoch: 0,
            active: None,
            previous: None,
            retired: false,
            pending_prune: Vec::new(),
        }
    }

    /// Check every structural invariant this module relies on before
    /// trusting a [`DurableState`] value read from the authority.
    /// This is deliberately independent of any live digest
    /// re-verification (that happens separately in
    /// [`read_and_verify`]) -- this only checks that the state's
    /// *own* fields are mutually consistent.
    pub fn validate_self_consistent(&self) -> Result<(), FailReason> {
        if self.pending_prune.len() > MAX_PENDING_PRUNE {
            return Err(FailReason::StateCorrupt);
        }
        {
            // Must already be strictly ascending AND duplicate-free --
            // comparing against a freshly sorted+deduped copy catches
            // both an out-of-order list and a list with repeats in one
            // check (a length-only comparison, as an earlier draft of
            // this check did, would miss a merely-reordered list: e.g.
            // `[2, 1]` sorts+dedups to `[1, 2]`, same length as the
            // original, but the original was never valid).
            let mut sorted = self.pending_prune.clone();
            sorted.sort_unstable();
            sorted.dedup();
            if sorted != self.pending_prune {
                return Err(FailReason::StateCorrupt);
            }
        }

        if self.retired {
            if self.active.is_some() || self.previous.is_some() {
                return Err(FailReason::StateCorrupt);
            }
            if self.high_water_epoch == 0 {
                // A pair cannot be retired without ever having been
                // provisioned.
                return Err(FailReason::StateCorrupt);
            }
        } else if self.active.is_none() {
            // Never-provisioned: nothing else may be set either.
            if self.previous.is_some() || self.high_water_epoch != 0 {
                return Err(FailReason::StateCorrupt);
            }
        }

        if let Some(active) = &self.active {
            if !valid_digest_hex(&active.digest_hex) {
                return Err(FailReason::StateCorrupt);
            }
            if active.epoch.get() == 0 || active.epoch.get() > self.high_water_epoch {
                return Err(FailReason::StateCorrupt);
            }
            if self.pending_prune.contains(&active.epoch) {
                return Err(FailReason::StateCorrupt);
            }
        }
        if let Some(previous) = &self.previous {
            if !valid_digest_hex(&previous.digest_hex) {
                return Err(FailReason::StateCorrupt);
            }
            let Some(active) = &self.active else {
                // `previous` without `active` is never valid.
                return Err(FailReason::StateCorrupt);
            };
            if previous.epoch == active.epoch {
                return Err(FailReason::StateCorrupt);
            }
            if previous.epoch.get() == 0 || previous.epoch.get() > self.high_water_epoch {
                return Err(FailReason::StateCorrupt);
            }
            if self.pending_prune.contains(&previous.epoch) {
                return Err(FailReason::StateCorrupt);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Secret material
// ---------------------------------------------------------------------

/// Caller-supplied secret bytes. Deliberately **not** `Copy` or
/// `Clone`: every holder of an owned [`SecretMaterial`] is a distinct,
/// independently zeroized buffer. The bytes are wrapped in
/// [`Zeroizing`] *before* validation, so a rejected (empty or
/// oversized) buffer is still zeroized on drop rather than discarded
/// as a plain `Vec<u8>`.
pub struct SecretMaterial {
    bytes: Zeroizing<Vec<u8>>,
}

impl SecretMaterial {
    /// 1 MiB. Generous for TPM-bound credential blobs, guest signing
    /// keys, and security-key channel state; prevents an unbounded
    /// allocation/hash from a misbehaving caller.
    pub const MAX_LEN: usize = 1 << 20;

    pub fn new(bytes: Vec<u8>) -> Result<Self, FailReason> {
        // Wrap FIRST: a validation-rejected buffer is zeroized on
        // drop here, not silently dropped as a plain `Vec<u8>`.
        let bytes = Zeroizing::new(bytes);
        if bytes.is_empty() || bytes.len() > Self::MAX_LEN {
            return Err(FailReason::InvalidMaterial);
        }
        Ok(Self { bytes })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn digest_hex(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&*self.bytes);
        hex_encode(&hasher.finalize())
    }
}

impl fmt::Debug for SecretMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretMaterial")
            .field("len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

// ---------------------------------------------------------------------
// The authority port
// ---------------------------------------------------------------------

/// Closed set of errors a [`SecretsAuthorityPort`] adapter may report.
/// This module never inspects or forwards an adapter's own internal
/// error detail -- every variant here is meaningful to *this* module's
/// own algorithm, not a leak of the adapter's internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortError {
    /// [`SecretsAuthorityPort::cas_commit`]'s `expected` token did not
    /// match the authority's current token; nothing was mutated.
    OwnershipFenced,
    /// The authority has quarantined this `(workload, kind)` pair; no
    /// operation may proceed.
    Quarantined,
    /// [`SecretsAuthorityPort::stage_material`] was called for an
    /// epoch already referenced as `active` or `previous` by the
    /// authority's currently committed [`DurableState`]. This module
    /// never calls `stage_material` for an epoch it can determine is
    /// already committed, so this is only reachable via a
    /// concurrently racing writer or a fault-injecting test double.
    EpochAlreadyCommitted,
    /// An adapter-internal failure (I/O, its own locking/storage
    /// substrate, etc) unrelated to this module's own protocol.
    Unavailable,
}

/// Closed set of reasons this module quarantines a `(workload, kind)`
/// pair. Quarantine is a one-way, fail-closed door from this module's
/// point of view: there is no "un-quarantine" call here (see the
/// module doc's "Integration wiring points" § 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuarantineReason {
    /// The live digest of the `active` generation did not match the
    /// digest recorded in the last-read [`DurableState`].
    ActiveChecksumMismatch,
    /// The live digest of the `previous` generation did not match the
    /// digest recorded in the last-read [`DurableState`].
    PreviousChecksumMismatch,
    /// The [`DurableState`] read from the authority failed its own
    /// [`DurableState::validate_self_consistent`] check.
    StateSelfInconsistent,
}

/// The single seam this pure transaction core depends on. An
/// integrator-owned adapter implements this against whatever real
/// storage/locking/CAS substrate lands (a filesystem tree with its own
/// `rename(2)`-based scheme, a KV store with native CAS, a database
/// row with an optimistic-lock column, etc) -- this module has no
/// opinion on, and no dependency on, that choice. Every method is
/// "guarded": scoped to exactly one `(workload, kind)` pair, taking
/// and returning only this module's own typed values -- never a raw
/// path, file descriptor, lock handle, or adapter-internal error
/// detail.
///
/// Implementations MUST provide the exact atomicity/idempotency
/// guarantees documented on each method; this module's correctness
/// (in particular "never return an error after silently activating
/// unrecoverable state") depends on `cas_commit` being genuinely
/// atomic and `prune_material`/`quarantine` being genuinely idempotent.
pub trait SecretsAuthorityPort {
    /// Load the current durable state and its CAS fencing token. A
    /// `(workload, kind)` pair that has never been committed to
    /// returns [`DurableState::never_provisioned`] paired with
    /// [`OwnershipEpoch::NEVER_COMMITTED`].
    fn read_state(
        &self,
        workload: &WorkloadId,
        kind: SecretKind,
    ) -> Result<(DurableState, OwnershipEpoch), PortError>;

    /// Durably store `material`'s bytes as the candidate content for
    /// `epoch`, returning the digest the adapter actually stored (the
    /// caller compares this against its own independently computed
    /// digest as an immediate defense-in-depth check that the adapter
    /// stored exactly what was asked). May be called more than once
    /// for an epoch not yet referenced as `active`/`previous` by the
    /// currently committed [`DurableState`] (each call's bytes
    /// supersede the previous, so re-staging after an interrupted
    /// attempt is always safe) -- but the adapter MUST refuse
    /// ([`PortError::EpochAlreadyCommitted`]) a stage call for an
    /// epoch already so referenced.
    fn stage_material(
        &self,
        workload: &WorkloadId,
        kind: SecretKind,
        epoch: Epoch,
        material: &SecretMaterial,
    ) -> Result<String, PortError>;

    /// Re-derive the digest of whatever is currently stored for
    /// `epoch`, without ever handing the raw bytes back to this
    /// module. Used to re-verify `active`/`previous` before any
    /// mutation, and to close the "last stage wins" race described in
    /// the module doc by re-checking a just-committed epoch.
    fn material_digest(
        &self,
        workload: &WorkloadId,
        kind: SecretKind,
        epoch: Epoch,
    ) -> Result<String, PortError>;

    /// Atomically replace the committed [`DurableState`] with `next`
    /// if and only if the adapter's currently stored fencing token
    /// equals `expected` -- all or nothing, exactly like a classic
    /// CAS/etcd transaction. On success returns the new fencing token
    /// (always different from `expected`). On a lost race returns
    /// [`PortError::OwnershipFenced`] and the adapter's own state is
    /// left completely unchanged.
    fn cas_commit(
        &self,
        workload: &WorkloadId,
        kind: SecretKind,
        expected: OwnershipEpoch,
        next: DurableState,
    ) -> Result<OwnershipEpoch, PortError>;

    /// Durably discard the material for `epoch`. Idempotent: already
    /// absent is `Ok(())`, not an error. This module never calls this
    /// for an epoch still referenced as `active` or `previous` by the
    /// last-known committed [`DurableState`] -- the superseding
    /// transition is always committed first, then pruned.
    fn prune_material(
        &self,
        workload: &WorkloadId,
        kind: SecretKind,
        epoch: Epoch,
    ) -> Result<(), PortError>;

    /// Durably mark `(workload, kind)` as quarantined: every
    /// subsequent call of any method above for this pair must fail
    /// closed with [`PortError::Quarantined`] until an out-of-band,
    /// integrator-owned clearing operation (this module exposes none)
    /// resets it. Idempotent.
    fn quarantine(
        &self,
        workload: &WorkloadId,
        kind: SecretKind,
        reason: QuarantineReason,
    ) -> Result<(), PortError>;
}

fn map_port_error(err: PortError) -> FailReason {
    match err {
        PortError::OwnershipFenced => FailReason::OwnershipFenced,
        PortError::Quarantined => FailReason::Quarantined,
        PortError::EpochAlreadyCommitted => FailReason::StateCorrupt,
        PortError::Unavailable => FailReason::PortUnavailable,
    }
}

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretsLifecycleError {
    pub reason: FailReason,
    pub audit: SecretsLifecycleAuditFields,
}

impl fmt::Display for SecretsLifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "secrets-lifecycle: {}", self.reason)
    }
}

impl std::error::Error for SecretsLifecycleError {}

fn denied(ctx: &SecretsLifecycleAuditContext, reason: FailReason) -> SecretsLifecycleError {
    SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::denied(ctx, reason),
    }
}

/// There is no caller-supplied `MarkerResult` here: `failed` itself
/// always records `MarkerResult::FailedClosed` (see that constructor's
/// own doc comment for why accepting one would be dead flexibility).
fn failed_closed(ctx: &SecretsLifecycleAuditContext, reason: FailReason) -> SecretsLifecycleError {
    SecretsLifecycleError {
        reason,
        audit: SecretsLifecycleAuditFields::failed(ctx, reason),
    }
}

fn context(
    workload: &WorkloadId,
    kind: SecretKind,
    action: LifecycleAction,
) -> SecretsLifecycleAuditContext {
    SecretsLifecycleAuditContext {
        workload_id: workload.clone(),
        kind,
        action,
    }
}

// ---------------------------------------------------------------------
// Shared read + verify + debt-resolution
// ---------------------------------------------------------------------

/// Read the current [`DurableState`], validate its own internal
/// consistency, independently re-verify the live digest of every
/// generation it claims to reference, and fully drain any outstanding
/// [`DurableState::pending_prune`] debt before returning.
///
/// **Invariant callers may rely on: a successful return always has an
/// empty `pending_prune` list.** Every one of [`provision`], [`rotate`],
/// [`rollback`], [`retire`] calls this first and therefore never needs
/// to merge its own new debt with any pre-existing debt -- there is
/// never any pre-existing debt left by the time this returns `Ok`.
fn read_and_verify(
    port: &dyn SecretsAuthorityPort,
    workload: &WorkloadId,
    kind: SecretKind,
    ctx: &SecretsLifecycleAuditContext,
) -> Result<(DurableState, OwnershipEpoch), SecretsLifecycleError> {
    let (mut state, mut ownership) = port
        .read_state(workload, kind)
        .map_err(|e| failed_closed(ctx, map_port_error(e)))?;

    state.validate_self_consistent().map_err(|reason| {
        let _ = port.quarantine(workload, kind, QuarantineReason::StateSelfInconsistent);
        failed_closed(ctx, reason)
    })?;

    if let Some(active) = state.active.clone() {
        let live = port
            .material_digest(workload, kind, active.epoch)
            .map_err(|e| failed_closed(ctx, map_port_error(e)))?;
        if live != active.digest_hex {
            let _ = port.quarantine(workload, kind, QuarantineReason::ActiveChecksumMismatch);
            return Err(failed_closed(ctx, FailReason::ChecksumMismatch));
        }
    }
    if let Some(previous) = state.previous.clone() {
        let live = port
            .material_digest(workload, kind, previous.epoch)
            .map_err(|e| failed_closed(ctx, map_port_error(e)))?;
        if live != previous.digest_hex {
            let _ = port.quarantine(workload, kind, QuarantineReason::PreviousChecksumMismatch);
            return Err(failed_closed(ctx, FailReason::ChecksumMismatch));
        }
    }

    if !state.pending_prune.is_empty() {
        let still_pending: Vec<Epoch> = state
            .pending_prune
            .iter()
            .copied()
            .filter(|&epoch| port.prune_material(workload, kind, epoch).is_err())
            .collect();
        if still_pending != state.pending_prune {
            let mut reduced = state.clone();
            reduced.pending_prune = still_pending.clone();
            ownership = port
                .cas_commit(workload, kind, ownership, reduced.clone())
                .map_err(|e| failed_closed(ctx, map_port_error(e)))?;
            state = reduced;
        }
        if !still_pending.is_empty() {
            return Err(denied(ctx, FailReason::PruneDebtUnresolved));
        }
    }

    Ok((state, ownership))
}

/// Attempt to synchronously clear every entry `committed.pending_prune`
/// still lists (the caller's own just-committed transition populated
/// this list; `ownership` is the fencing token that commit returned),
/// durably committing the (possibly smaller) result. Returns whether
/// any entry remains pending. **Never itself returns an error**: the
/// caller's own CAS transition already durably succeeded before this
/// runs, so a prune shortfall here is recorded as debt for
/// [`read_and_verify`] to resolve on a future call, never surfaced as
/// this call's own failure (see the module doc's "forward recovery"
/// section).
fn attempt_prune_now(
    port: &dyn SecretsAuthorityPort,
    workload: &WorkloadId,
    kind: SecretKind,
    ownership: OwnershipEpoch,
    committed: DurableState,
) -> bool {
    if committed.pending_prune.is_empty() {
        return false;
    }
    let still_pending: Vec<Epoch> = committed
        .pending_prune
        .iter()
        .copied()
        .filter(|&epoch| port.prune_material(workload, kind, epoch).is_err())
        .collect();
    if still_pending.len() == committed.pending_prune.len() {
        // Nothing cleared; the already-committed debt list is already
        // exactly this, so there is nothing new to commit.
        return true;
    }
    let mut reduced = committed;
    reduced.pending_prune = still_pending.clone();
    // Best-effort: if this follow-up commit itself loses a race, the
    // debt is not lost -- whoever won that race read (at least) our
    // full un-reduced `pending_prune` before their own commit (CAS is
    // fully serialized), so their own `read_and_verify` call will
    // itself attempt (idempotently) to prune the same epochs.
    let _ = port.cas_commit(workload, kind, ownership, reduced);
    !still_pending.is_empty()
}

// ---------------------------------------------------------------------
// Public actions
// ---------------------------------------------------------------------

/// Provision fresh generation-1 material. Fails closed
/// ([`FailReason::AlreadyProvisioned`], denied) if the authority
/// currently records an active generation for this `(workload, kind)`
/// pair.
pub fn provision(
    port: &dyn SecretsAuthorityPort,
    workload: &WorkloadId,
    kind: SecretKind,
    material: SecretMaterial,
) -> Result<SecretsLifecycleAuditFields, SecretsLifecycleError> {
    let ctx = context(workload, kind, LifecycleAction::Provision);
    let (state, ownership) = read_and_verify(port, workload, kind, &ctx)?;

    if state.active.is_some() {
        return Err(denied(&ctx, FailReason::AlreadyProvisioned));
    }

    let digest_hex = material.digest_hex();
    let epoch = Epoch::FIRST;
    let stored_digest = port
        .stage_material(workload, kind, epoch, &material)
        .map_err(|e| failed_closed(&ctx, map_port_error(e)))?;
    if stored_digest != digest_hex {
        let _ = port.quarantine(workload, kind, QuarantineReason::ActiveChecksumMismatch);
        return Err(failed_closed(&ctx, FailReason::ChecksumMismatch));
    }

    let next = DurableState {
        high_water_epoch: 1,
        active: Some(GenerationRecord::new(epoch, digest_hex.clone())),
        previous: None,
        retired: false,
        pending_prune: Vec::new(),
    };
    port.cas_commit(workload, kind, ownership, next)
        .map_err(|e| failed_closed(&ctx, map_port_error(e)))?;

    // Close the "last stage wins" race: refuse to certify success if
    // the durably-committed epoch's live bytes don't match what this
    // call itself just staged and committed.
    let live = port
        .material_digest(workload, kind, epoch)
        .map_err(|e| failed_closed(&ctx, map_port_error(e)))?;
    if live != digest_hex {
        let _ = port.quarantine(workload, kind, QuarantineReason::ActiveChecksumMismatch);
        return Err(failed_closed(&ctx, FailReason::ChecksumMismatch));
    }

    Ok(SecretsLifecycleAuditFields::provisioned(
        &ctx, 1, digest_hex,
    ))
}

/// Create and activate a new generation. Requires an active,
/// non-retired generation (else [`FailReason::NotProvisioned`],
/// denied). Allocates `to_epoch = high_water_epoch + 1` -- never
/// `current_epoch + 1` -- so a rotate issued after a rollback can
/// never collide with a still-materialized newer epoch the rollback
/// moved away from.
pub fn rotate(
    port: &dyn SecretsAuthorityPort,
    workload: &WorkloadId,
    kind: SecretKind,
    material: SecretMaterial,
) -> Result<SecretsLifecycleAuditFields, SecretsLifecycleError> {
    let ctx = context(workload, kind, LifecycleAction::Rotate);
    let (state, ownership) = read_and_verify(port, workload, kind, &ctx)?;

    let Some(active) = state.active.clone() else {
        return Err(denied(&ctx, FailReason::NotProvisioned));
    };

    let next_epoch_num = state.high_water_epoch.saturating_add(1);
    let Some(next_epoch) = Epoch::from_raw(next_epoch_num) else {
        // Only reachable if `high_water_epoch` were already
        // `u64::MAX`, which `validate_self_consistent` cannot itself
        // detect as corrupt but is not a realistic lineage length; a
        // defensive backstop, never expected to trigger.
        return Err(failed_closed(&ctx, FailReason::StateCorrupt));
    };
    let digest_hex = material.digest_hex();

    let stored_digest = port
        .stage_material(workload, kind, next_epoch, &material)
        .map_err(|e| failed_closed(&ctx, map_port_error(e)))?;
    if stored_digest != digest_hex {
        let _ = port.quarantine(workload, kind, QuarantineReason::ActiveChecksumMismatch);
        return Err(failed_closed(&ctx, FailReason::ChecksumMismatch));
    }

    // The invariant documented on `read_and_verify` guarantees
    // `state.pending_prune` is already empty here.
    let pending_prune = state
        .previous
        .as_ref()
        .map(|prior_previous| vec![prior_previous.epoch])
        .unwrap_or_default();

    let next = DurableState {
        high_water_epoch: next_epoch_num,
        active: Some(GenerationRecord::new(next_epoch, digest_hex.clone())),
        previous: Some(active),
        retired: false,
        pending_prune,
    };
    let new_ownership = port
        .cas_commit(workload, kind, ownership, next.clone())
        .map_err(|e| failed_closed(&ctx, map_port_error(e)))?;

    let live = port
        .material_digest(workload, kind, next_epoch)
        .map_err(|e| failed_closed(&ctx, map_port_error(e)))?;
    if live != digest_hex {
        let _ = port.quarantine(workload, kind, QuarantineReason::ActiveChecksumMismatch);
        return Err(failed_closed(&ctx, FailReason::ChecksumMismatch));
    }

    let retained = vec![next.previous.as_ref().expect("just set above").epoch.get()];
    let prune_deferred = attempt_prune_now(port, workload, kind, new_ownership, next);

    Ok(SecretsLifecycleAuditFields::rotated(
        &ctx,
        next_epoch.get(),
        next_epoch_num,
        retained,
        digest_hex,
        prune_deferred,
    ))
}

/// Swap the active generation back to the retained `previous`
/// generation. Requires a `previous` entry (else
/// [`FailReason::NoRollbackTarget`], denied). The generation being
/// rolled back *from* becomes the new `previous` (so a rollback may
/// itself be rolled back / "rolled forward"); `high_water_epoch` is
/// unchanged (rollback never grows the monotonic mark) and nothing is
/// pruned.
pub fn rollback(
    port: &dyn SecretsAuthorityPort,
    workload: &WorkloadId,
    kind: SecretKind,
) -> Result<SecretsLifecycleAuditFields, SecretsLifecycleError> {
    let ctx = context(workload, kind, LifecycleAction::Rollback);
    let (state, ownership) = read_and_verify(port, workload, kind, &ctx)?;

    let Some(active) = state.active.clone() else {
        return Err(denied(&ctx, FailReason::NotProvisioned));
    };
    let Some(previous) = state.previous.clone() else {
        return Err(denied(&ctx, FailReason::NoRollbackTarget));
    };

    // `previous`'s digest was already independently re-verified
    // against live storage inside `read_and_verify` above (the
    // "expected_digest_hex == expected_identity.digest_hex before
    // mutation" invariant carried over from rounds 1-5); re-assert it
    // here too so the check is visible at the exact call site that
    // performs the mutation, not only inside the shared helper.
    let live_previous_digest = port
        .material_digest(workload, kind, previous.epoch)
        .map_err(|e| failed_closed(&ctx, map_port_error(e)))?;
    if live_previous_digest != previous.digest_hex {
        let _ = port.quarantine(workload, kind, QuarantineReason::PreviousChecksumMismatch);
        return Err(failed_closed(&ctx, FailReason::ChecksumMismatch));
    }

    let next = DurableState {
        high_water_epoch: state.high_water_epoch,
        active: Some(previous.clone()),
        previous: Some(active.clone()),
        retired: false,
        // The invariant documented on `read_and_verify` guarantees
        // this is already empty; rollback supersedes nothing new.
        pending_prune: Vec::new(),
    };
    port.cas_commit(workload, kind, ownership, next)
        .map_err(|e| failed_closed(&ctx, map_port_error(e)))?;

    Ok(SecretsLifecycleAuditFields::rolled_back(
        &ctx,
        previous.epoch.get(),
        state.high_water_epoch,
        vec![active.epoch.get()],
    ))
}

/// Retire this `(workload, kind)` pair: remove every generation and
/// mark it retired. Always reachable, and always a no-op
/// ([`FailReason`]-free `verified_clean`) when there is no active
/// generation -- a missing/never-provisioned pair and an
/// already-retired pair are indistinguishable to a caller of `retire`
/// from this module's point of view, both correctly reported as
/// already clean.
pub fn retire(
    port: &dyn SecretsAuthorityPort,
    workload: &WorkloadId,
    kind: SecretKind,
) -> Result<SecretsLifecycleAuditFields, SecretsLifecycleError> {
    let ctx = context(workload, kind, LifecycleAction::Retire);
    let (state, ownership) = read_and_verify(port, workload, kind, &ctx)?;

    let Some(active) = state.active.clone() else {
        return Ok(SecretsLifecycleAuditFields::verified_clean(
            &ctx,
            state.high_water_epoch,
        ));
    };

    // The invariant documented on `read_and_verify` guarantees
    // `state.pending_prune` is already empty, so this never exceeds
    // `MAX_PENDING_PRUNE` (at most the former active + former
    // previous, i.e. at most 2). `active.epoch` is not always
    // numerically greater than `previous.epoch` -- a rollback can
    // leave `active` at a *lower* epoch than `previous` -- so this
    // must be explicitly sorted before it becomes part of a
    // [`DurableState`], which requires `pending_prune` ascending (see
    // [`DurableState::validate_self_consistent`]).
    let mut pending_prune = vec![active.epoch];
    if let Some(previous) = &state.previous {
        pending_prune.push(previous.epoch);
    }
    pending_prune.sort_unstable();
    debug_assert!(pending_prune.len() <= MAX_PENDING_PRUNE);

    let next = DurableState {
        high_water_epoch: state.high_water_epoch,
        active: None,
        previous: None,
        retired: true,
        pending_prune,
    };
    let new_ownership = port
        .cas_commit(workload, kind, ownership, next.clone())
        .map_err(|e| failed_closed(&ctx, map_port_error(e)))?;

    let prune_deferred = attempt_prune_now(port, workload, kind, new_ownership, next);

    Ok(SecretsLifecycleAuditFields::retired(
        &ctx,
        state.high_water_epoch,
        prune_deferred,
    ))
}

// ---------------------------------------------------------------------
// Test double: an in-memory `SecretsAuthorityPort` with fault injection
// ---------------------------------------------------------------------

/// An in-memory [`SecretsAuthorityPort`] used **only** by this module's
/// own test suite below. It is not `pub`, has no relation to any real
/// storage adapter, and exists purely so this file's crash/fault/
/// concurrency/tamper tests never touch a filesystem, a lock, or any
/// I/O primitive -- every test failure mode is expressed as an
/// explicit, deterministic fault flag on this double.
#[cfg(test)]
mod fake_port {
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    use super::{
        DurableState, Epoch, OwnershipEpoch, PortError, QuarantineReason, SecretKind,
        SecretMaterial, SecretsAuthorityPort, WorkloadId,
    };

    fn digest_of(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        super::hex_encode(&hasher.finalize())
    }

    /// One `(workload, kind)` pair's key in every map below. `WorkloadId`
    /// and `SecretKind` are both already `Hash + Eq`, so this is a plain
    /// owned tuple -- no derived string/path key.
    type Key = (WorkloadId, SecretKind);

    /// Fault-injection knobs. The four `fail_*` flags are **one-shot**:
    /// each is consumed (reset to `false`) the first time the matching
    /// method is called after being set, via [`std::mem::take`], so a
    /// test can inject exactly one failure at a precise step without
    /// permanently wedging the double. [`Self::fail_prune_epochs`] is
    /// deliberately **persistent** (not one-shot) so a test can model
    /// "this epoch's prune keeps failing across several attempts" and
    /// then explicitly clear it to observe eventual, self-healing
    /// resolution via [`read_and_verify`](super::read_and_verify)'s debt
    /// drain. [`Self::clobber_before_digest`] simulates the module doc's
    /// "last stage wins" race by mutating the *actually stored* bytes
    /// for a targeted epoch immediately before the next
    /// [`SecretsAuthorityPort::material_digest`] call for that exact
    /// epoch, then that call computes a real digest over the
    /// now-corrupted bytes -- this is a realistic simulation of a
    /// losing racer's late `stage_material` write, not a fabricated
    /// wrong string.
    #[derive(Default)]
    pub struct Faults {
        pub fail_read_state: bool,
        pub fail_stage_material: bool,
        pub fail_material_digest: bool,
        pub fail_cas_commit: bool,
        pub fail_prune_epochs: HashSet<u64>,
        pub clobber_before_digest: Option<(u64, Vec<u8>)>,
    }

    #[derive(Default)]
    struct Inner {
        state: HashMap<Key, (DurableState, u64)>,
        material: HashMap<(Key, u64), Vec<u8>>,
        quarantined: HashMap<Key, QuarantineReason>,
        faults: Faults,
        cas_commit_attempts: u64,
        prune_attempts: u64,
    }

    #[derive(Default)]
    pub struct FakePort(Mutex<Inner>);

    fn take_one_shot(flag: &mut bool) -> bool {
        std::mem::take(flag)
    }

    impl FakePort {
        pub fn new() -> Self {
            Self::default()
        }

        fn key(workload: &WorkloadId, kind: SecretKind) -> Key {
            (workload.clone(), kind)
        }

        /// Replace the fault set wholesale (also used to clear every
        /// flag between phases of a single multi-step test).
        pub fn set_faults(&self, faults: Faults) {
            self.0.lock().unwrap().faults = faults;
        }

        pub fn quarantined_for(
            &self,
            workload: &WorkloadId,
            kind: SecretKind,
        ) -> Option<QuarantineReason> {
            self.0
                .lock()
                .unwrap()
                .quarantined
                .get(&Self::key(workload, kind))
                .copied()
        }

        /// The exact durably committed [`DurableState`], bypassing this
        /// module's own read path entirely -- used by tests to assert
        /// on ground truth independent of `read_and_verify`.
        pub fn raw_state(&self, workload: &WorkloadId, kind: SecretKind) -> Option<DurableState> {
            self.0
                .lock()
                .unwrap()
                .state
                .get(&Self::key(workload, kind))
                .map(|(state, _)| state.clone())
        }

        pub fn raw_material_epochs(&self, workload: &WorkloadId, kind: SecretKind) -> Vec<u64> {
            let inner = self.0.lock().unwrap();
            let k = Self::key(workload, kind);
            let mut epochs: Vec<u64> = inner
                .material
                .keys()
                .filter(|(key, _)| *key == k)
                .map(|(_, epoch)| *epoch)
                .collect();
            epochs.sort_unstable();
            epochs
        }

        pub fn cas_commit_attempts(&self) -> u64 {
            self.0.lock().unwrap().cas_commit_attempts
        }

        pub fn prune_attempts(&self) -> u64 {
            self.0.lock().unwrap().prune_attempts
        }
    }

    impl SecretsAuthorityPort for FakePort {
        fn read_state(
            &self,
            workload: &WorkloadId,
            kind: SecretKind,
        ) -> Result<(DurableState, OwnershipEpoch), PortError> {
            let mut inner = self.0.lock().unwrap();
            let k = Self::key(workload, kind);
            if inner.quarantined.contains_key(&k) {
                return Err(PortError::Quarantined);
            }
            if take_one_shot(&mut inner.faults.fail_read_state) {
                return Err(PortError::Unavailable);
            }
            Ok(match inner.state.get(&k) {
                Some((state, token)) => (state.clone(), OwnershipEpoch::from_raw(*token)),
                None => (
                    DurableState::never_provisioned(),
                    OwnershipEpoch::NEVER_COMMITTED,
                ),
            })
        }

        fn stage_material(
            &self,
            workload: &WorkloadId,
            kind: SecretKind,
            epoch: Epoch,
            material: &SecretMaterial,
        ) -> Result<String, PortError> {
            let mut inner = self.0.lock().unwrap();
            let k = Self::key(workload, kind);
            if inner.quarantined.contains_key(&k) {
                return Err(PortError::Quarantined);
            }
            if take_one_shot(&mut inner.faults.fail_stage_material) {
                return Err(PortError::Unavailable);
            }
            if let Some((state, _)) = inner.state.get(&k) {
                let already_committed = state.active.as_ref().map(|g| g.epoch) == Some(epoch)
                    || state.previous.as_ref().map(|g| g.epoch) == Some(epoch);
                if already_committed {
                    return Err(PortError::EpochAlreadyCommitted);
                }
            }
            let bytes = material.as_bytes().to_vec();
            let digest = digest_of(&bytes);
            inner.material.insert((k, epoch.get()), bytes);
            Ok(digest)
        }

        fn material_digest(
            &self,
            workload: &WorkloadId,
            kind: SecretKind,
            epoch: Epoch,
        ) -> Result<String, PortError> {
            let mut inner = self.0.lock().unwrap();
            let k = Self::key(workload, kind);
            if inner.quarantined.contains_key(&k) {
                return Err(PortError::Quarantined);
            }
            if take_one_shot(&mut inner.faults.fail_material_digest) {
                return Err(PortError::Unavailable);
            }
            // Check the target epoch WITHOUT consuming the fault first:
            // an `if let Some(...) = opt.take() && guard` chain would
            // unconditionally consume `opt` while evaluating the
            // pattern, even when `guard` then fails for the wrong
            // epoch -- silently discarding a fault meant for a later
            // call. Only `.take()` once the epoch is confirmed to
            // match.
            if inner
                .faults
                .clobber_before_digest
                .as_ref()
                .map(|(epoch, _)| *epoch)
                == Some(epoch.get())
                && let Some((_, clobbered)) = inner.faults.clobber_before_digest.take()
            {
                inner.material.insert((k.clone(), epoch.get()), clobbered);
            }
            let bytes = inner
                .material
                .get(&(k, epoch.get()))
                .cloned()
                .ok_or(PortError::Unavailable)?;
            Ok(digest_of(&bytes))
        }

        fn cas_commit(
            &self,
            workload: &WorkloadId,
            kind: SecretKind,
            expected: OwnershipEpoch,
            next: DurableState,
        ) -> Result<OwnershipEpoch, PortError> {
            let mut inner = self.0.lock().unwrap();
            let k = Self::key(workload, kind);
            if inner.quarantined.contains_key(&k) {
                return Err(PortError::Quarantined);
            }
            inner.cas_commit_attempts += 1;
            if take_one_shot(&mut inner.faults.fail_cas_commit) {
                return Err(PortError::Unavailable);
            }
            let current_token = inner
                .state
                .get(&k)
                .map(|(_, token)| *token)
                .unwrap_or(OwnershipEpoch::NEVER_COMMITTED.get());
            if current_token != expected.get() {
                return Err(PortError::OwnershipFenced);
            }
            let new_token = current_token + 1;
            inner.state.insert(k, (next, new_token));
            Ok(OwnershipEpoch::from_raw(new_token))
        }

        fn prune_material(
            &self,
            workload: &WorkloadId,
            kind: SecretKind,
            epoch: Epoch,
        ) -> Result<(), PortError> {
            let mut inner = self.0.lock().unwrap();
            let k = Self::key(workload, kind);
            if inner.quarantined.contains_key(&k) {
                return Err(PortError::Quarantined);
            }
            inner.prune_attempts += 1;
            if inner.faults.fail_prune_epochs.contains(&epoch.get()) {
                return Err(PortError::Unavailable);
            }
            inner.material.remove(&(k, epoch.get()));
            Ok(())
        }

        fn quarantine(
            &self,
            workload: &WorkloadId,
            kind: SecretKind,
            reason: QuarantineReason,
        ) -> Result<(), PortError> {
            let mut inner = self.0.lock().unwrap();
            inner.quarantined.insert(Self::key(workload, kind), reason);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::fake_port::{FakePort, Faults};
    use super::*;

    fn wl_a() -> WorkloadId {
        WorkloadId::parse("aaaaaaaaaaaaaaaaaaaa").expect("valid fixture workload id")
    }

    fn wl_b() -> WorkloadId {
        WorkloadId::parse("bbbbbbbbbbbbbbbbbbbq").expect("valid fixture workload id")
    }

    const KIND: SecretKind = SecretKind::GuestSigningKey;

    fn material(bytes: &[u8]) -> SecretMaterial {
        SecretMaterial::new(bytes.to_vec()).expect("valid fixture material")
    }

    fn digest_of(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex_encode(&hasher.finalize())
    }

    // -- Epoch / OwnershipEpoch -----------------------------------------

    #[test]
    fn epoch_from_raw_rejects_zero() {
        assert_eq!(Epoch::from_raw(0), None);
        assert_eq!(Epoch::from_raw(1), Some(Epoch::FIRST));
        assert_eq!(Epoch::from_raw(7).unwrap().get(), 7);
    }

    #[test]
    fn epoch_display_is_plain_decimal() {
        assert_eq!(Epoch::FIRST.to_string(), "1");
        assert_eq!(Epoch::from_raw(42).unwrap().to_string(), "42");
    }

    #[test]
    fn ownership_epoch_never_committed_is_zero() {
        assert_eq!(OwnershipEpoch::NEVER_COMMITTED.get(), 0);
    }

    // -- DurableState self-consistency -----------------------------------

    #[test]
    fn durable_state_never_provisioned_is_self_consistent() {
        DurableState::never_provisioned()
            .validate_self_consistent()
            .expect("never_provisioned must be self-consistent");
    }

    #[test]
    fn durable_state_rejects_pending_prune_over_max() {
        let state = DurableState {
            high_water_epoch: 5,
            active: Some(GenerationRecord::new(
                Epoch::from_raw(5).unwrap(),
                "a".repeat(64),
            )),
            previous: None,
            retired: false,
            pending_prune: vec![
                Epoch::from_raw(1).unwrap(),
                Epoch::from_raw(2).unwrap(),
                Epoch::from_raw(3).unwrap(),
            ],
        };
        assert_eq!(
            state.validate_self_consistent(),
            Err(FailReason::StateCorrupt)
        );
    }

    #[test]
    fn durable_state_rejects_unsorted_pending_prune() {
        let state = DurableState {
            high_water_epoch: 5,
            active: Some(GenerationRecord::new(
                Epoch::from_raw(5).unwrap(),
                "a".repeat(64),
            )),
            previous: None,
            retired: false,
            pending_prune: vec![Epoch::from_raw(2).unwrap(), Epoch::from_raw(1).unwrap()],
        };
        assert_eq!(
            state.validate_self_consistent(),
            Err(FailReason::StateCorrupt)
        );
    }

    #[test]
    fn durable_state_rejects_duplicate_pending_prune() {
        let state = DurableState {
            high_water_epoch: 5,
            active: Some(GenerationRecord::new(
                Epoch::from_raw(5).unwrap(),
                "a".repeat(64),
            )),
            previous: None,
            retired: false,
            pending_prune: vec![Epoch::from_raw(1).unwrap(), Epoch::from_raw(1).unwrap()],
        };
        assert_eq!(
            state.validate_self_consistent(),
            Err(FailReason::StateCorrupt)
        );
    }

    #[test]
    fn durable_state_rejects_retired_with_active() {
        let state = DurableState {
            high_water_epoch: 1,
            active: Some(GenerationRecord::new(Epoch::FIRST, "a".repeat(64))),
            previous: None,
            retired: true,
            pending_prune: Vec::new(),
        };
        assert_eq!(
            state.validate_self_consistent(),
            Err(FailReason::StateCorrupt)
        );
    }

    #[test]
    fn durable_state_rejects_retired_with_zero_high_water() {
        let state = DurableState {
            high_water_epoch: 0,
            active: None,
            previous: None,
            retired: true,
            pending_prune: Vec::new(),
        };
        assert_eq!(
            state.validate_self_consistent(),
            Err(FailReason::StateCorrupt)
        );
    }

    #[test]
    fn durable_state_rejects_never_provisioned_shape_with_previous_set() {
        let state = DurableState {
            high_water_epoch: 0,
            active: None,
            previous: Some(GenerationRecord::new(Epoch::FIRST, "a".repeat(64))),
            retired: false,
            pending_prune: Vec::new(),
        };
        assert_eq!(
            state.validate_self_consistent(),
            Err(FailReason::StateCorrupt)
        );
    }

    #[test]
    fn durable_state_rejects_active_with_invalid_digest() {
        let state = DurableState {
            high_water_epoch: 1,
            active: Some(GenerationRecord::new(Epoch::FIRST, "not-hex".to_string())),
            previous: None,
            retired: false,
            pending_prune: Vec::new(),
        };
        assert_eq!(
            state.validate_self_consistent(),
            Err(FailReason::StateCorrupt)
        );
    }

    #[test]
    fn durable_state_rejects_active_epoch_above_high_water() {
        let state = DurableState {
            high_water_epoch: 1,
            active: Some(GenerationRecord::new(
                Epoch::from_raw(2).unwrap(),
                "a".repeat(64),
            )),
            previous: None,
            retired: false,
            pending_prune: Vec::new(),
        };
        assert_eq!(
            state.validate_self_consistent(),
            Err(FailReason::StateCorrupt)
        );
    }

    #[test]
    fn durable_state_rejects_previous_without_active() {
        let state = DurableState {
            high_water_epoch: 1,
            active: None,
            previous: Some(GenerationRecord::new(Epoch::FIRST, "a".repeat(64))),
            retired: false,
            pending_prune: Vec::new(),
        };
        assert_eq!(
            state.validate_self_consistent(),
            Err(FailReason::StateCorrupt)
        );
    }

    #[test]
    fn durable_state_rejects_previous_equal_to_active() {
        let state = DurableState {
            high_water_epoch: 1,
            active: Some(GenerationRecord::new(Epoch::FIRST, "a".repeat(64))),
            previous: Some(GenerationRecord::new(Epoch::FIRST, "b".repeat(64))),
            retired: false,
            pending_prune: Vec::new(),
        };
        assert_eq!(
            state.validate_self_consistent(),
            Err(FailReason::StateCorrupt)
        );
    }

    #[test]
    fn durable_state_rejects_pending_prune_overlapping_active() {
        let state = DurableState {
            high_water_epoch: 1,
            active: Some(GenerationRecord::new(Epoch::FIRST, "a".repeat(64))),
            previous: None,
            retired: false,
            pending_prune: vec![Epoch::FIRST],
        };
        assert_eq!(
            state.validate_self_consistent(),
            Err(FailReason::StateCorrupt)
        );
    }

    #[test]
    fn durable_state_rejects_pending_prune_overlapping_previous() {
        let state = DurableState {
            high_water_epoch: 2,
            active: Some(GenerationRecord::new(
                Epoch::from_raw(2).unwrap(),
                "a".repeat(64),
            )),
            previous: Some(GenerationRecord::new(Epoch::FIRST, "b".repeat(64))),
            retired: false,
            pending_prune: vec![Epoch::FIRST],
        };
        assert_eq!(
            state.validate_self_consistent(),
            Err(FailReason::StateCorrupt)
        );
    }

    // -- SecretMaterial ---------------------------------------------------

    #[test]
    fn secret_material_rejects_empty() {
        assert_eq!(
            SecretMaterial::new(Vec::new()).err(),
            Some(FailReason::InvalidMaterial)
        );
    }

    #[test]
    fn secret_material_rejects_oversized() {
        let oversized = vec![0_u8; SecretMaterial::MAX_LEN + 1];
        assert_eq!(
            SecretMaterial::new(oversized).err(),
            Some(FailReason::InvalidMaterial)
        );
    }

    #[test]
    fn secret_material_accepts_max_len() {
        let exact = vec![7_u8; SecretMaterial::MAX_LEN];
        assert!(SecretMaterial::new(exact).is_ok());
    }

    #[test]
    fn secret_material_debug_never_leaks_bytes() {
        let mat = material(b"super-secret-tpm-blob");
        let rendered = format!("{mat:?}");
        assert!(!rendered.contains("super-secret-tpm-blob"));
        assert!(rendered.contains("len"));
    }

    // -- provision --------------------------------------------------------

    #[test]
    fn provision_success_activates_epoch_one() {
        let port = FakePort::new();
        let wl = wl_a();
        let bytes = b"tpm-credential-v1".to_vec();
        let expected_digest = digest_of(&bytes);

        let audit = provision(&port, &wl, KIND, material(&bytes)).expect("provision must succeed");
        assert_eq!(audit.result, LifecycleResult::Created);
        assert_eq!(audit.lineage_epoch, Some(1));
        assert_eq!(audit.material_digest_hex, Some(expected_digest.clone()));
        audit.validate().expect("audit record must validate");

        let state = port.raw_state(&wl, KIND).expect("state must be committed");
        assert_eq!(state.high_water_epoch, 1);
        assert_eq!(state.active.unwrap().digest_hex, expected_digest);
        assert!(state.previous.is_none());
        assert!(!state.retired);
    }

    #[test]
    fn provision_denied_when_already_active() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"first")).expect("first provision must succeed");

        let err = provision(&port, &wl, KIND, material(b"second")).expect_err("must be denied");
        assert_eq!(err.reason, FailReason::AlreadyProvisioned);
        assert_eq!(err.audit.result, LifecycleResult::Denied);
        err.audit.validate().expect("denied record must validate");
        // Denial must not have mutated anything.
        assert_eq!(port.raw_material_epochs(&wl, KIND), vec![1]);
    }

    #[test]
    fn provision_fails_closed_on_read_state_fault_with_zero_mutation() {
        let port = FakePort::new();
        let wl = wl_a();
        port.set_faults(Faults {
            fail_read_state: true,
            ..Default::default()
        });

        let err = provision(&port, &wl, KIND, material(b"x")).expect_err("must fail closed");
        assert_eq!(err.reason, FailReason::PortUnavailable);
        assert_eq!(err.audit.result, LifecycleResult::FailedClosed);
        err.audit.validate().expect("failed record must validate");
        assert!(port.raw_state(&wl, KIND).is_none(), "no partial commit");
        assert!(port.raw_material_epochs(&wl, KIND).is_empty());
    }

    #[test]
    fn provision_fails_closed_on_stage_material_fault_with_zero_mutation() {
        let port = FakePort::new();
        let wl = wl_a();
        port.set_faults(Faults {
            fail_stage_material: true,
            ..Default::default()
        });

        let err = provision(&port, &wl, KIND, material(b"x")).expect_err("must fail closed");
        assert_eq!(err.reason, FailReason::PortUnavailable);
        assert!(port.raw_state(&wl, KIND).is_none(), "no partial commit");
        assert!(port.raw_material_epochs(&wl, KIND).is_empty());
    }

    #[test]
    fn provision_fails_closed_on_cas_commit_fault_with_zero_activation() {
        let port = FakePort::new();
        let wl = wl_a();
        port.set_faults(Faults {
            fail_cas_commit: true,
            ..Default::default()
        });

        let err = provision(&port, &wl, KIND, material(b"x")).expect_err("must fail closed");
        assert_eq!(err.reason, FailReason::PortUnavailable);
        assert!(
            port.raw_state(&wl, KIND).is_none(),
            "a failed cas_commit must never leave a committed active generation"
        );
    }

    #[test]
    fn provision_detects_post_commit_clobber_and_quarantines() {
        let port = FakePort::new();
        let wl = wl_a();
        // Simulate a losing racer's `stage_material` call landing for
        // epoch 1 immediately after this call's own `cas_commit`
        // succeeds, corrupting the just-activated generation's bytes.
        port.set_faults(Faults {
            clobber_before_digest: Some((1, b"corrupted-by-loser".to_vec())),
            ..Default::default()
        });

        let err =
            provision(&port, &wl, KIND, material(b"legit")).expect_err("must detect the race");
        assert_eq!(err.reason, FailReason::ChecksumMismatch);
        assert_eq!(err.audit.result, LifecycleResult::FailedClosed);
        assert_eq!(
            port.quarantined_for(&wl, KIND),
            Some(QuarantineReason::ActiveChecksumMismatch)
        );
        // The commit itself was NOT rolled back -- it durably happened
        // and is now quarantined, not silently discarded.
        assert!(port.raw_state(&wl, KIND).is_some());
    }

    #[test]
    fn provision_after_quarantine_always_fails_closed() {
        let port = FakePort::new();
        let wl = wl_a();
        port.quarantine(&wl, KIND, QuarantineReason::StateSelfInconsistent)
            .unwrap();

        let err =
            provision(&port, &wl, KIND, material(b"x")).expect_err("quarantine blocks everything");
        assert_eq!(err.reason, FailReason::Quarantined);
    }

    // -- rotate -------------------------------------------------------------

    #[test]
    fn rotate_denied_when_never_provisioned() {
        let port = FakePort::new();
        let wl = wl_a();
        let err = rotate(&port, &wl, KIND, material(b"x")).expect_err("must be denied");
        assert_eq!(err.reason, FailReason::NotProvisioned);
    }

    #[test]
    fn rotate_success_allocates_high_water_plus_one_and_prunes_immediately() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();

        let audit = rotate(&port, &wl, KIND, material(b"gen2")).expect("rotate must succeed");
        assert_eq!(audit.result, LifecycleResult::Rotated);
        assert_eq!(audit.lineage_epoch, Some(2));
        assert_eq!(audit.high_water_epoch, Some(2));
        assert_eq!(audit.retained_generations, vec![1]);
        assert!(
            !audit.prune_deferred,
            "gen1 has no rival pending prune, should prune immediately"
        );
        audit.validate().expect("audit record must validate");

        let state = port.raw_state(&wl, KIND).unwrap();
        assert_eq!(state.active.unwrap().epoch.get(), 2);
        assert_eq!(state.previous.unwrap().epoch.get(), 1);
        assert!(state.pending_prune.is_empty());
    }

    #[test]
    fn rotate_twice_prunes_the_generation_two_steps_back() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        rotate(&port, &wl, KIND, material(b"gen2")).unwrap();
        let audit =
            rotate(&port, &wl, KIND, material(b"gen3")).expect("second rotate must succeed");
        assert_eq!(audit.lineage_epoch, Some(3));
        assert_eq!(audit.retained_generations, vec![2]);
        assert!(!audit.prune_deferred);
        assert_eq!(
            port.raw_material_epochs(&wl, KIND),
            vec![2, 3],
            "gen1 must be pruned"
        );
    }

    #[test]
    fn rotate_after_rollback_allocates_from_high_water_not_active_epoch() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        rotate(&port, &wl, KIND, material(b"gen2")).unwrap();
        // Roll back: active becomes epoch 1 again, previous becomes
        // epoch 2. `high_water_epoch` stays 2 (rollback never grows it).
        rollback(&port, &wl, KIND).unwrap();
        assert_eq!(
            port.raw_state(&wl, KIND)
                .unwrap()
                .active
                .unwrap()
                .epoch
                .get(),
            1
        );

        // A rotate now MUST allocate epoch 3 (high_water + 1), never
        // epoch 2 (active + 1) -- epoch 2 is still materialized as
        // `previous` and colliding with it would silently resurrect it.
        let audit = rotate(&port, &wl, KIND, material(b"gen3")).expect("rotate must succeed");
        assert_eq!(audit.lineage_epoch, Some(3));
        assert_eq!(audit.high_water_epoch, Some(3));
        assert_eq!(audit.retained_generations, vec![1]);
    }

    #[test]
    fn rotate_defers_prune_when_faulted_and_surfaces_prune_deferred() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        // gen1 -> active=1. First rotate has nothing to prune yet
        // (there is no pre-existing `previous` to supersede).
        rotate(&port, &wl, KIND, material(b"gen2")).unwrap();
        // Now active=2, previous=1 (retained). A *second* rotate
        // supersedes that retained `previous` (epoch 1) -- the epoch a
        // rotate schedules for pruning is always the generation being
        // dropped from `previous`, never the epoch this call itself
        // just activated.
        let mut failing = HashSet::new();
        failing.insert(1_u64);
        port.set_faults(Faults {
            fail_prune_epochs: failing,
            ..Default::default()
        });
        let audit =
            rotate(&port, &wl, KIND, material(b"gen3")).expect("rotate itself must still succeed");
        assert!(
            audit.prune_deferred,
            "the epoch-1 prune failed, must be surfaced as deferred"
        );
        assert_eq!(
            port.raw_state(&wl, KIND).unwrap().pending_prune,
            vec![Epoch::FIRST]
        );
        // The deferred generation's bytes are not lost.
        assert!(port.raw_material_epochs(&wl, KIND).contains(&1));

        // Clear the fault and let the next action's `read_and_verify`
        // drain the debt before doing its own work.
        port.set_faults(Faults::default());
        let audit2 = rotate(&port, &wl, KIND, material(b"gen4"))
            .expect("rotate must succeed and drain debt");
        assert!(
            !audit2.prune_deferred,
            "this rotate's own supersession (epoch 2) prunes cleanly"
        );
        assert!(port.raw_state(&wl, KIND).unwrap().pending_prune.is_empty());
        assert_eq!(
            port.raw_material_epochs(&wl, KIND),
            vec![3, 4],
            "epoch 1's debt must have been drained, and epoch 2 pruned by this rotate's own supersession"
        );
    }

    #[test]
    fn rotate_detects_post_commit_clobber_and_quarantines() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        port.set_faults(Faults {
            clobber_before_digest: Some((2, b"corrupted".to_vec())),
            ..Default::default()
        });

        let err = rotate(&port, &wl, KIND, material(b"gen2")).expect_err("must detect the race");
        assert_eq!(err.reason, FailReason::ChecksumMismatch);
        assert_eq!(
            port.quarantined_for(&wl, KIND),
            Some(QuarantineReason::ActiveChecksumMismatch)
        );
    }

    #[test]
    fn rotate_fails_closed_when_fenced_with_zero_mutation() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        // Fence exactly the next `cas_commit` call, simulating a
        // concurrent writer that committed first.
        port.set_faults(Faults {
            fail_cas_commit: true,
            ..Default::default()
        });

        let err = rotate(&port, &wl, KIND, material(b"gen2")).expect_err("must fail closed");
        assert_eq!(err.reason, FailReason::PortUnavailable);
        let state = port.raw_state(&wl, KIND).unwrap();
        assert_eq!(state.active.unwrap().epoch.get(), 1, "must still be gen1");
        assert!(state.previous.is_none());
    }

    // -- rollback -------------------------------------------------------------

    #[test]
    fn rollback_denied_never_provisioned() {
        let port = FakePort::new();
        let wl = wl_a();
        let err = rollback(&port, &wl, KIND).expect_err("must be denied");
        assert_eq!(err.reason, FailReason::NotProvisioned);
    }

    #[test]
    fn rollback_denied_no_rollback_target() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        let err = rollback(&port, &wl, KIND).expect_err("must be denied");
        assert_eq!(err.reason, FailReason::NoRollbackTarget);
    }

    #[test]
    fn rollback_success_swaps_active_and_previous() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        rotate(&port, &wl, KIND, material(b"gen2")).unwrap();

        let audit = rollback(&port, &wl, KIND).expect("rollback must succeed");
        assert_eq!(audit.result, LifecycleResult::RolledBack);
        assert_eq!(audit.lineage_epoch, Some(1));
        assert_eq!(
            audit.high_water_epoch,
            Some(2),
            "rollback never grows high_water"
        );
        assert_eq!(audit.retained_generations, vec![2]);

        let state = port.raw_state(&wl, KIND).unwrap();
        assert_eq!(state.active.unwrap().epoch.get(), 1);
        assert_eq!(state.previous.unwrap().epoch.get(), 2);
    }

    #[test]
    fn rollback_can_itself_be_rolled_forward() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        rotate(&port, &wl, KIND, material(b"gen2")).unwrap();
        rollback(&port, &wl, KIND).unwrap();

        let audit = rollback(&port, &wl, KIND).expect("rolling forward must succeed");
        assert_eq!(audit.lineage_epoch, Some(2));
        let state = port.raw_state(&wl, KIND).unwrap();
        assert_eq!(state.active.unwrap().epoch.get(), 2);
        assert_eq!(state.previous.unwrap().epoch.get(), 1);
    }

    #[test]
    fn rollback_detects_previous_tamper_and_quarantines() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        rotate(&port, &wl, KIND, material(b"gen2")).unwrap();
        // Tamper with the retained `previous` (epoch 1) generation's
        // live bytes directly, bypassing the protocol entirely.
        port.set_faults(Faults {
            clobber_before_digest: Some((1, b"tampered".to_vec())),
            ..Default::default()
        });

        let err = rollback(&port, &wl, KIND).expect_err("must detect the tamper");
        assert_eq!(err.reason, FailReason::ChecksumMismatch);
        assert_eq!(
            port.quarantined_for(&wl, KIND),
            Some(QuarantineReason::PreviousChecksumMismatch)
        );
    }

    // -- retire -------------------------------------------------------------

    #[test]
    fn retire_verified_clean_when_never_provisioned() {
        let port = FakePort::new();
        let wl = wl_a();
        let audit = retire(&port, &wl, KIND).expect("retire on never-provisioned must be clean");
        assert_eq!(audit.result, LifecycleResult::VerifiedClean);
        audit.validate().expect("must validate");
        assert!(
            port.raw_state(&wl, KIND).is_none(),
            "must not fabricate a committed state"
        );
    }

    #[test]
    fn retire_verified_clean_when_already_retired() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        retire(&port, &wl, KIND).unwrap();

        let audit = retire(&port, &wl, KIND).expect("second retire must be clean, not an error");
        assert_eq!(audit.result, LifecycleResult::VerifiedClean);
        assert_eq!(audit.high_water_epoch, Some(1));
    }

    #[test]
    fn retire_prunes_active_and_previous() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        rotate(&port, &wl, KIND, material(b"gen2")).unwrap();

        let audit = retire(&port, &wl, KIND).expect("retire must succeed");
        assert_eq!(audit.result, LifecycleResult::Retired);
        assert!(!audit.prune_deferred);
        audit.validate().expect("must validate");

        let state = port.raw_state(&wl, KIND).unwrap();
        assert!(state.retired);
        assert!(state.active.is_none());
        assert!(state.previous.is_none());
        assert!(port.raw_material_epochs(&wl, KIND).is_empty());
    }

    #[test]
    fn retire_sorts_pending_prune_when_active_epoch_is_lower_than_previous() {
        // Regression test: a rollback can leave `active` at a
        // *numerically lower* epoch than `previous` (e.g. active=1,
        // previous=2 after one rotate + one rollback). `retire` must
        // still produce an ascending-sorted `pending_prune`, or the
        // committed `DurableState` would fail its own
        // `validate_self_consistent` sortedness check.
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        rotate(&port, &wl, KIND, material(b"gen2")).unwrap();
        rollback(&port, &wl, KIND).unwrap();
        let state_before = port.raw_state(&wl, KIND).unwrap();
        assert_eq!(state_before.active.unwrap().epoch.get(), 1);
        assert_eq!(state_before.previous.unwrap().epoch.get(), 2);

        // Fault every prune so the committed `pending_prune` list is
        // directly observable (not immediately drained).
        let mut failing = HashSet::new();
        failing.insert(1_u64);
        failing.insert(2_u64);
        port.set_faults(Faults {
            fail_prune_epochs: failing,
            ..Default::default()
        });

        let audit = retire(&port, &wl, KIND).expect("retire itself must still succeed");
        assert!(audit.prune_deferred);
        let state = port.raw_state(&wl, KIND).unwrap();
        assert_eq!(
            state.pending_prune,
            vec![Epoch::FIRST, Epoch::from_raw(2).unwrap()]
        );
    }

    #[test]
    fn retire_defers_prune_when_faulted_then_next_action_drains_debt() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        rotate(&port, &wl, KIND, material(b"gen2")).unwrap();

        let mut failing = HashSet::new();
        failing.insert(1_u64);
        failing.insert(2_u64);
        port.set_faults(Faults {
            fail_prune_epochs: failing,
            ..Default::default()
        });
        let audit = retire(&port, &wl, KIND).expect("retire must still succeed");
        assert!(audit.prune_deferred);
        assert_eq!(port.raw_material_epochs(&wl, KIND), vec![1, 2]);

        port.set_faults(Faults::default());
        // A fresh provision must fully drain the retired lineage's
        // debt before it may reuse epoch 1's key.
        provision(&port, &wl, KIND, material(b"fresh-gen1"))
            .expect("provision must drain debt and succeed");
        assert_eq!(port.raw_material_epochs(&wl, KIND), vec![1]);
        let state = port.raw_state(&wl, KIND).unwrap();
        assert_eq!(
            state.high_water_epoch, 1,
            "provision resets high_water for a fresh lineage"
        );
        assert!(state.pending_prune.is_empty());
    }

    // -- Cross-cutting: concurrency, quarantine permanence ------------------

    #[test]
    fn concurrent_cas_commit_race_one_wins_one_fenced_with_zero_mutation() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();

        // Two callers both read the same pre-rotate state...
        let (state_a, ownership_a) = port.read_state(&wl, KIND).unwrap();
        let (state_b, ownership_b) = port.read_state(&wl, KIND).unwrap();
        assert_eq!(ownership_a, ownership_b);

        // ...and both compute a next state for epoch 2.
        let next_a = DurableState {
            high_water_epoch: 2,
            active: Some(GenerationRecord::new(
                Epoch::from_raw(2).unwrap(),
                "a".repeat(64),
            )),
            previous: state_a.active.clone(),
            retired: false,
            pending_prune: Vec::new(),
        };
        let next_b = DurableState {
            high_water_epoch: 2,
            active: Some(GenerationRecord::new(
                Epoch::from_raw(2).unwrap(),
                "b".repeat(64),
            )),
            previous: state_b.active.clone(),
            retired: false,
            pending_prune: Vec::new(),
        };

        let winner = port.cas_commit(&wl, KIND, ownership_a, next_a.clone());
        assert!(winner.is_ok(), "the first committer must win");
        let loser = port.cas_commit(&wl, KIND, ownership_b, next_b);
        assert_eq!(loser, Err(PortError::OwnershipFenced));

        // The loser's attempt must not have mutated anything: the
        // winner's own state is exactly what is committed.
        assert_eq!(port.raw_state(&wl, KIND).unwrap(), next_a);
    }

    #[test]
    fn quarantine_is_permanent_and_blocks_every_port_method() {
        let port = FakePort::new();
        let wl = wl_a();
        provision(&port, &wl, KIND, material(b"gen1")).unwrap();
        port.quarantine(&wl, KIND, QuarantineReason::StateSelfInconsistent)
            .unwrap();

        assert_eq!(
            port.read_state(&wl, KIND).unwrap_err(),
            PortError::Quarantined
        );
        assert_eq!(
            port.stage_material(&wl, KIND, Epoch::from_raw(2).unwrap(), &material(b"x"))
                .unwrap_err(),
            PortError::Quarantined
        );
        assert_eq!(
            port.material_digest(&wl, KIND, Epoch::FIRST).unwrap_err(),
            PortError::Quarantined
        );
        assert_eq!(
            port.cas_commit(
                &wl,
                KIND,
                OwnershipEpoch::NEVER_COMMITTED,
                DurableState::never_provisioned()
            )
            .unwrap_err(),
            PortError::Quarantined
        );
        assert_eq!(
            port.prune_material(&wl, KIND, Epoch::FIRST).unwrap_err(),
            PortError::Quarantined
        );

        for outcome in [
            provision(&port, &wl, KIND, material(b"x"))
                .map(|_| ())
                .unwrap_err()
                .reason,
            rotate(&port, &wl, KIND, material(b"x"))
                .map(|_| ())
                .unwrap_err()
                .reason,
            rollback(&port, &wl, KIND).map(|_| ()).unwrap_err().reason,
            retire(&port, &wl, KIND).map(|_| ()).unwrap_err().reason,
        ] {
            assert_eq!(outcome, FailReason::Quarantined);
        }
    }

    #[test]
    fn two_distinct_workloads_never_interfere() {
        let port = FakePort::new();
        let wl_x = wl_a();
        let wl_y = wl_b();
        provision(&port, &wl_x, KIND, material(b"x-gen1")).unwrap();
        provision(&port, &wl_y, KIND, material(b"y-gen1")).unwrap();
        retire(&port, &wl_x, KIND).unwrap();

        assert!(port.raw_state(&wl_x, KIND).unwrap().retired);
        assert!(!port.raw_state(&wl_y, KIND).unwrap().retired);
        assert_eq!(port.raw_material_epochs(&wl_y, KIND), vec![1]);
    }
}
