//! Guest-side lifecycle state for the security-key CTAPHID channel
//! binding + reconnect epoch that
//! [`crate::services::security_key::SessionConfig`] ultimately
//! consumes.
//!
//! This module is the W8 `secrets-lifecycle` component's guest-side
//! counterpart to the broker's `d2b_priv_broker::ops::secrets_lifecycle`
//! engine and `SecretKind::SecurityKeyChannelState`. It is intentionally
//! a standalone, in-memory, zero-internal-dependency module (only
//! `std`) so it can be validated on its own without depending on this
//! crate's `d2b-session`/`d2b-contracts`/transport wiring, and so a
//! future integrator can wire it into
//! [`crate::services::security_key::SessionConfig`] without this
//! component needing to touch that file.
//!
//! # Design summary (post-review v2)
//!
//! The prior draft of this module conflated two independent
//! properties into a single `ChannelGeneration` counter:
//!
//!   * the broker's **lineage epoch** — the active generation
//!     identity from `secrets_lifecycle`'s durable state
//!     (`DurableState::active`'s `GenerationRecord::epoch`, as of the
//!     W8fu6 ports-and-adapters rewrite; an earlier draft of this doc
//!     comment referred to a since-removed on-disk `MarkerData::active`
//!     field from that engine's original filesystem-anchored design).
//!     This value is **rollbackable**: a legitimate `rollback()` moves
//!     it *backwards* to a previously-retained generation.
//!   * a **monotonic anti-replay counter** — a value that must never
//!     go backwards or repeat for the lifetime of this channel state,
//!     independent of whether the lineage epoch itself moved forward
//!     or backward.
//!
//! Using one counter for both meant a legitimate rollback (which
//! *must* decrease the lineage epoch) was indistinguishable from a
//! stale/replayed message (which *must* be rejected). This revision
//! splits them into [`LineageEpoch`] (rollbackable, no monotonicity
//! enforced by this module — the broker/authenticator is the
//! authority for whether a given epoch transition is legitimate) and
//! [`DeliveryCounter`] (strictly monotonic for the entire lifetime of
//! a [`ChannelState`], **including across `retire` + re-`provision`**,
//! enforced here as the sole replay guard).
//!
//! Every state transition is tagged with an explicit
//! [`ChannelAction`] discriminator (`Provision` / `Rotate` /
//! `Rollback` / `Retire`) carried on the wire alongside the epoch and
//! counter, rather than inferred from field deltas. [`ChannelState`]
//! never re-derives "was this a rotate or a rollback" from whether
//! the epoch went up or down.
//!
//! # What this module deliberately does NOT do
//!
//!   * **It does not authenticate messages.** [`ChannelUpdate`]
//!     values are assumed, by the time they reach [`ChannelState::apply`],
//!     to have already been authenticated (signature/MAC verified,
//!     origin checked) by the caller using whatever primitive the
//!     real guest-control/vsock transport specifies. This crate has
//!     no cryptographic dependency (no `sha2`/`hmac`/`ring`), and
//!     adding one is out of scope for this component (it would
//!     require a `Cargo.toml` edit, which this component does not
//!     own). Wiring in real authentication is an explicit integration
//!     wiring point below.
//!   * **It does not define or parse a wire byte format.** A prior
//!     draft of this module proposed a concrete 40-byte layout
//!     (`from_wire_bytes`); that was an ad-hoc format finalized
//!     without closed validation against the broker's actual
//!     `SecretKind::SecurityKeyChannelState` dispatch/serialization
//!     (which does not exist yet — `runtime.rs` is out of scope for
//!     this component). This module now exposes only a typed,
//!     in-process API ([`ChannelUpdate`] and its named constructors);
//!     the integrator must define the real wire schema (likely in
//!     `d2b-contracts`, also out of scope here) and translate it into
//!     a [`ChannelUpdate`] value, not the other way around.
//!
//! # Integration wiring points (deliberately NOT performed here)
//!
//!   1. `src/lib.rs` needs `pub mod secrets_channel;`.
//!   2. `services/security_key/mod.rs`'s `SessionConfig::from_env` (or
//!      a new constructor alongside it) should source its
//!      `channel_binding`/`reconnect_generation` from a
//!      [`ChannelState::with_current`] snapshot instead of the static
//!      `D2B_SK_CHANNEL_BINDING_HEX`/`D2B_SK_RECONNECT_GENERATION`
//!      environment variables it reads today. Neither `lib.rs` nor
//!      `services/security_key/mod.rs` is owned by this component.
//!   3. A real wire schema for the guest-control message that carries
//!      `SecretKind::SecurityKeyChannelState` material must be defined
//!      (in `d2b-contracts` or equivalent) and its receiver must
//!      authenticate the message (origin + integrity) BEFORE
//!      constructing a [`ChannelUpdate`] — this module trusts its
//!      caller on that point and performs no cryptographic
//!      verification itself.
//!   4. The broker side (`packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs`,
//!      out of scope here) needs a dispatch arm that maps its
//!      `provision`/`rotate`/`rollback`/`retire` outcomes onto the
//!      same four-way [`ChannelAction`] discriminator, so the two
//!      sides of the channel share one vocabulary for what happened.
//!   5. The persistent delivery-counter high-water mark
//!      ([`ChannelState`]'s internal state) is currently **process
//!      memory only** — it does not survive a guest process restart.
//!      If replay protection must also survive a guest reboot (not
//!      just a broker-side retire/reprovision), the integrator needs
//!      to persist [`ChannelState`]'s high-water value to guest-local
//!      storage and restore it at startup before the first `apply`
//!      call. That storage design is out of scope for this
//!      zero-dependency, in-memory module.

use std::fmt;
use std::sync::RwLock;

/// Errors surfaced by every public function in this module. Never
/// carries a raw byte, path, or counter/epoch value — only a
/// closed-set discriminant, safe for any Debug/log/audit surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelStateError {
    InvalidBinding,
    InvalidLineageEpoch,
    InvalidDeliveryCounter,
    /// `apply` was given a `delivery_counter` that is not strictly
    /// greater than the highest one ever accepted by this
    /// [`ChannelState`] — refused as a possible replay regardless of
    /// which [`ChannelAction`] it carries. This check is independent
    /// of, and applied before, any lineage-epoch or provisioning-state
    /// validation.
    StaleDeliveryCounter,
    /// `rotate`/`rollback`/`current` was called before any `provision`.
    NotProvisioned,
    /// `provision` was called while material is already active
    /// (callers should `rotate` instead).
    AlreadyProvisioned,
    /// The channel state was retired; `current`/`rotate`/`rollback`
    /// refuse to resume without an explicit fresh `provision`.
    Retired,
    /// A `Provision`/`Rotate`/`Rollback` update was missing its
    /// required material, or a `Retire` update unexpectedly carried
    /// material/a lineage epoch. Unreachable through the public
    /// [`ChannelUpdate`] constructors; kept as defense in depth.
    MalformedUpdate,
}

impl fmt::Display for ChannelStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let slug = match self {
            Self::InvalidBinding => "invalid-channel-binding",
            Self::InvalidLineageEpoch => "invalid-lineage-epoch",
            Self::InvalidDeliveryCounter => "invalid-delivery-counter",
            Self::StaleDeliveryCounter => "stale-delivery-counter",
            Self::NotProvisioned => "channel-not-provisioned",
            Self::AlreadyProvisioned => "channel-already-provisioned",
            Self::Retired => "channel-retired",
            Self::MalformedUpdate => "malformed-channel-update",
        };
        f.write_str(slug)
    }
}

impl std::error::Error for ChannelStateError {}

/// Best-effort, dependency-free zeroization of a byte buffer.
///
/// This crate builds under `#![forbid(unsafe_code)]`, so the
/// `zeroize` crate's volatile-write approach is unavailable here (and
/// adding the `zeroize` dependency would require a `Cargo.toml` edit,
/// which this component does not own). [`std::hint::black_box`] is
/// used to defeat dead-store elimination: without it, an optimizing
/// compiler could prove the buffer's final writes are never observed
/// (since the memory is about to be dropped/freed) and elide them
/// entirely.
fn best_effort_zero(buf: &mut [u8]) {
    for byte in buf.iter_mut() {
        *byte = 0;
    }
    std::hint::black_box(buf);
}

/// A validated, non-zero 32-byte CTAPHID channel-binding value.
///
/// Deliberately **not** `Copy` or `Clone`: every live instance is a
/// distinct, independently zeroized buffer. Callers that need the raw
/// bytes for a final handoff (e.g. to
/// `crate::services::security_key::SessionConfig::new`) must use
/// [`expose_bytes`](Self::expose_bytes) explicitly, and are
/// responsible for zeroizing that returned copy themselves once done.
pub struct ChannelBinding([u8; 32]);

impl ChannelBinding {
    /// Wraps `bytes` immediately, before validating them, so that
    /// **every** exit path from this function — including the
    /// rejection path below — drops (and therefore zeroizes, via
    /// [`Drop`]) the candidate buffer rather than leaving a rejected
    /// buffer to be cleaned up by ordinary non-zeroizing `Drop`.
    pub fn new(bytes: [u8; 32]) -> Result<Self, ChannelStateError> {
        let candidate = Self(bytes);
        if candidate.0 == [0u8; 32] {
            return Err(ChannelStateError::InvalidBinding);
        }
        Ok(candidate)
    }

    /// Explicit, one-shot exposure of the raw bytes for handoff to an
    /// external consumer that requires an owned `[u8; 32]` (such as
    /// `SessionConfig::new`). Named deliberately unlike `.clone()` so
    /// call sites make the exposure visible in review. The caller
    /// owns zeroizing the returned array once it is no longer needed.
    pub fn expose_bytes(&self) -> [u8; 32] {
        self.0
    }
}

impl Drop for ChannelBinding {
    fn drop(&mut self) {
        best_effort_zero(&mut self.0);
    }
}

impl fmt::Debug for ChannelBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ChannelBinding")
            .field(&"<redacted>")
            .finish()
    }
}

/// The broker-side lineage epoch this material was activated at.
/// **Rollbackable**: a legitimate `Rollback` action moves this value
/// backwards to a previously-retained generation. This module does
/// not enforce monotonicity on this value — see the module-level
/// design summary for why that guard lives on [`DeliveryCounter`]
/// instead.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LineageEpoch(u64);

impl LineageEpoch {
    pub fn new(value: u64) -> Result<Self, ChannelStateError> {
        if value == 0 {
            return Err(ChannelStateError::InvalidLineageEpoch);
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for LineageEpoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("LineageEpoch").field(&"<redacted>").finish()
    }
}

/// A strictly-monotonic, replay-guarding delivery sequence number.
/// Distinct from [`LineageEpoch`]: this value must never repeat or
/// decrease for the entire lifetime of a [`ChannelState`], **including
/// across `Retire` followed by a fresh `Provision`** — unlike the
/// broker's on-disk storage generation counter (which legitimately
/// restarts at 1 after a retire), the anti-replay high-water mark is
/// a property of this in-memory channel object, not of the
/// rollbackable storage state.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeliveryCounter(u64);

impl DeliveryCounter {
    pub fn new(value: u64) -> Result<Self, ChannelStateError> {
        if value == 0 {
            return Err(ChannelStateError::InvalidDeliveryCounter);
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for DeliveryCounter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DeliveryCounter")
            .field(&"<redacted>")
            .finish()
    }
}

/// Explicit discriminator for what a [`ChannelUpdate`] represents.
/// Carried on the wire (once a real wire schema exists — see the
/// module-level wiring points) rather than inferred from whether the
/// lineage epoch increased or decreased.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelAction {
    Provision,
    Rotate,
    Rollback,
    Retire,
}

/// A single, already-authenticated (by the caller — see module docs)
/// state transition to apply to a [`ChannelState`].
///
/// Fields are private; the only way to construct a value is through
/// the named constructors below, which guarantee the
/// action/epoch/material combination is internally consistent (e.g.
/// `retire()` can never accidentally carry material).
///
/// Not `Clone`/`Copy`: an update owns its [`ChannelBinding`] (when
/// present) and consumes it exactly once via [`ChannelState::apply`].
/// If `apply` rejects the update, the update (and its embedded
/// material, if any) is dropped at the end of that call, which
/// zeroizes it via `ChannelBinding`'s `Drop` impl — no separate
/// "zeroize on rejection" code path is needed.
pub struct ChannelUpdate {
    action: ChannelAction,
    lineage_epoch: Option<LineageEpoch>,
    delivery_counter: DeliveryCounter,
    material: Option<ChannelBinding>,
}

impl ChannelUpdate {
    pub fn provision(
        lineage_epoch: LineageEpoch,
        delivery_counter: DeliveryCounter,
        binding: ChannelBinding,
    ) -> Self {
        Self {
            action: ChannelAction::Provision,
            lineage_epoch: Some(lineage_epoch),
            delivery_counter,
            material: Some(binding),
        }
    }

    pub fn rotate(
        lineage_epoch: LineageEpoch,
        delivery_counter: DeliveryCounter,
        binding: ChannelBinding,
    ) -> Self {
        Self {
            action: ChannelAction::Rotate,
            lineage_epoch: Some(lineage_epoch),
            delivery_counter,
            material: Some(binding),
        }
    }

    /// `lineage_epoch` for a legitimate rollback is typically **lower**
    /// than the epoch currently active — that is expected and is not
    /// rejected by this module (see the module-level design summary).
    pub fn rollback(
        lineage_epoch: LineageEpoch,
        delivery_counter: DeliveryCounter,
        binding: ChannelBinding,
    ) -> Self {
        Self {
            action: ChannelAction::Rollback,
            lineage_epoch: Some(lineage_epoch),
            delivery_counter,
            material: Some(binding),
        }
    }

    /// Retire carries no material and no lineage epoch — mirrors the
    /// broker's own `SecretsLifecycleAuditFields::retired` audit shape,
    /// which likewise omits `lineage_epoch` (there is no longer an
    /// active generation once retired). A retire message still carries
    /// a `delivery_counter` so a stale replayed retire cannot be
    /// distinguished from a fresh one purely by inspection, and so a
    /// stale *pre-retire* message cannot be replayed after a retire to
    /// reactivate anything (the replay guard applies uniformly to all
    /// four actions).
    pub fn retire(delivery_counter: DeliveryCounter) -> Self {
        Self {
            action: ChannelAction::Retire,
            lineage_epoch: None,
            delivery_counter,
            material: None,
        }
    }

    pub fn action(&self) -> ChannelAction {
        self.action
    }
}

struct Inner {
    /// `Some((binding, lineage_epoch))` while provisioned/rotated/
    /// rolled-back and not retired.
    material: Option<(ChannelBinding, LineageEpoch)>,
    retired: bool,
    /// Highest `delivery_counter` ever accepted by this object.
    /// Deliberately **never reset** by `Provision` or `Retire` — see
    /// [`DeliveryCounter`]'s doc comment.
    delivery_high_water: u64,
}

/// In-memory, thread-safe holder for the guest frontend's active
/// channel material, supporting the same provision/rotate/rollback/
/// retire lifecycle shape as the broker's `secrets_lifecycle` engine
/// (kept independent — this module has no dependency on that crate).
pub struct ChannelState(RwLock<Inner>);

impl Default for ChannelState {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelState {
    pub fn new() -> Self {
        Self(RwLock::new(Inner {
            material: None,
            retired: false,
            delivery_high_water: 0,
        }))
    }

    /// Apply an already-authenticated state transition. The
    /// anti-replay `delivery_counter` check is evaluated first and
    /// applies uniformly to every [`ChannelAction`]; only if it passes
    /// does the action-specific provisioning-state validation run.
    ///
    /// On any `Err` return, `update` (and any [`ChannelBinding`] it
    /// carried) has already been dropped and therefore zeroized by
    /// the time this function returns.
    pub fn apply(&self, update: ChannelUpdate) -> Result<(), ChannelStateError> {
        let ChannelUpdate {
            action,
            lineage_epoch,
            delivery_counter,
            material,
        } = update;

        let mut inner = self.0.write().unwrap_or_else(|poison| poison.into_inner());

        if delivery_counter.get() <= inner.delivery_high_water {
            return Err(ChannelStateError::StaleDeliveryCounter);
        }

        match action {
            ChannelAction::Provision => {
                if inner.material.is_some() && !inner.retired {
                    return Err(ChannelStateError::AlreadyProvisioned);
                }
                let epoch = lineage_epoch.ok_or(ChannelStateError::MalformedUpdate)?;
                let binding = material.ok_or(ChannelStateError::MalformedUpdate)?;
                inner.material = Some((binding, epoch));
                inner.retired = false;
            }
            ChannelAction::Rotate | ChannelAction::Rollback => {
                if inner.retired {
                    return Err(ChannelStateError::Retired);
                }
                if inner.material.is_none() {
                    return Err(ChannelStateError::NotProvisioned);
                }
                let epoch = lineage_epoch.ok_or(ChannelStateError::MalformedUpdate)?;
                let binding = material.ok_or(ChannelStateError::MalformedUpdate)?;
                inner.material = Some((binding, epoch));
            }
            ChannelAction::Retire => {
                if lineage_epoch.is_some() || material.is_some() {
                    return Err(ChannelStateError::MalformedUpdate);
                }
                inner.material = None;
                inner.retired = true;
            }
        }

        inner.delivery_high_water = delivery_counter.get();
        Ok(())
    }

    /// Retired-ness and provisioned-ness, without exposing any
    /// secret material — safe for logging/metrics.
    pub fn is_retired(&self) -> bool {
        self.0
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .retired
    }

    /// The current anti-replay high-water mark. Not secret; safe to
    /// persist/log for the guest-restart wiring point described in
    /// the module docs.
    pub fn delivery_high_water(&self) -> u64 {
        self.0
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .delivery_high_water
    }

    /// Access the currently-active binding and lineage epoch via a
    /// closure, without ever handing out an owned/cloned
    /// [`ChannelBinding`] from inside the lock. Typical use:
    ///
    /// ```ignore
    /// state.with_current(|binding, epoch| {
    ///     SessionConfig::new(binding.expose_bytes(), epoch.get())
    /// })
    /// ```
    pub fn with_current<T>(
        &self,
        f: impl FnOnce(&ChannelBinding, LineageEpoch) -> T,
    ) -> Result<T, ChannelStateError> {
        let inner = self.0.read().unwrap_or_else(|poison| poison.into_inner());
        if inner.retired {
            return Err(ChannelStateError::Retired);
        }
        match &inner.material {
            Some((binding, epoch)) => Ok(f(binding, *epoch)),
            None => Err(ChannelStateError::NotProvisioned),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    fn binding(byte: u8) -> ChannelBinding {
        ChannelBinding::new([byte; 32]).expect("valid binding")
    }

    fn epoch(value: u64) -> LineageEpoch {
        LineageEpoch::new(value).expect("valid epoch")
    }

    fn counter(value: u64) -> DeliveryCounter {
        DeliveryCounter::new(value).expect("valid counter")
    }

    #[test]
    fn zero_binding_epoch_and_counter_are_rejected() {
        assert_eq!(
            ChannelBinding::new([0; 32]).unwrap_err(),
            ChannelStateError::InvalidBinding
        );
        assert_eq!(
            LineageEpoch::new(0).unwrap_err(),
            ChannelStateError::InvalidLineageEpoch
        );
        assert_eq!(
            DeliveryCounter::new(0).unwrap_err(),
            ChannelStateError::InvalidDeliveryCounter
        );
    }

    #[test]
    fn debug_impls_never_expose_bytes_or_counters() {
        let b = binding(0xAB);
        let e = epoch(7);
        let c = counter(9);
        assert!(format!("{b:?}").contains("redacted"));
        let e_debug = format!("{e:?}");
        let c_debug = format!("{c:?}");
        assert!(e_debug.contains("redacted"));
        assert!(c_debug.contains("redacted"));
        assert!(!e_debug.contains('7'));
        assert!(!c_debug.contains('9'));
    }

    #[test]
    fn with_current_before_provision_is_not_provisioned() {
        let state = ChannelState::new();
        assert_eq!(
            state.with_current(|_, _| ()).unwrap_err(),
            ChannelStateError::NotProvisioned
        );
    }

    #[test]
    fn provision_then_with_current_round_trips() {
        let state = ChannelState::new();
        state
            .apply(ChannelUpdate::provision(epoch(1), counter(1), binding(1)))
            .expect("provision succeeds");
        let (bytes, epoch_value) = state
            .with_current(|b, e| (b.expose_bytes(), e.get()))
            .expect("with_current succeeds");
        assert_eq!(bytes, [1u8; 32]);
        assert_eq!(epoch_value, 1);
    }

    #[test]
    fn provision_twice_without_retire_is_rejected() {
        let state = ChannelState::new();
        state
            .apply(ChannelUpdate::provision(epoch(1), counter(1), binding(1)))
            .expect("first provision succeeds");
        let err = state
            .apply(ChannelUpdate::provision(epoch(1), counter(2), binding(2)))
            .unwrap_err();
        assert_eq!(err, ChannelStateError::AlreadyProvisioned);
    }

    #[test]
    fn rotate_and_rollback_without_provision_are_not_provisioned() {
        let state = ChannelState::new();
        assert_eq!(
            state
                .apply(ChannelUpdate::rotate(epoch(1), counter(1), binding(1)))
                .unwrap_err(),
            ChannelStateError::NotProvisioned
        );
        assert_eq!(
            state
                .apply(ChannelUpdate::rollback(epoch(1), counter(1), binding(1)))
                .unwrap_err(),
            ChannelStateError::NotProvisioned
        );
    }

    #[test]
    fn rotate_advances_lineage_epoch() {
        let state = ChannelState::new();
        state
            .apply(ChannelUpdate::provision(epoch(1), counter(1), binding(1)))
            .expect("provision succeeds");
        state
            .apply(ChannelUpdate::rotate(epoch(2), counter(2), binding(2)))
            .expect("rotate succeeds");
        let epoch_value = state.with_current(|_, e| e.get()).unwrap();
        assert_eq!(epoch_value, 2);
    }

    #[test]
    fn rollback_may_legitimately_decrease_lineage_epoch() {
        let state = ChannelState::new();
        state
            .apply(ChannelUpdate::provision(epoch(1), counter(1), binding(1)))
            .expect("provision succeeds");
        state
            .apply(ChannelUpdate::rotate(epoch(2), counter(2), binding(2)))
            .expect("rotate succeeds");
        state
            .apply(ChannelUpdate::rollback(epoch(1), counter(3), binding(1)))
            .expect("rollback to a lower epoch succeeds");
        let epoch_value = state.with_current(|_, e| e.get()).unwrap();
        assert_eq!(epoch_value, 1);
    }

    #[test]
    fn stale_or_repeated_delivery_counter_is_rejected_regardless_of_action() {
        let state = ChannelState::new();
        state
            .apply(ChannelUpdate::provision(epoch(1), counter(5), binding(1)))
            .expect("provision succeeds");

        // Exact repeat.
        assert_eq!(
            state
                .apply(ChannelUpdate::rotate(epoch(2), counter(5), binding(2)))
                .unwrap_err(),
            ChannelStateError::StaleDeliveryCounter
        );
        // Older than high-water.
        assert_eq!(
            state
                .apply(ChannelUpdate::rotate(epoch(2), counter(4), binding(2)))
                .unwrap_err(),
            ChannelStateError::StaleDeliveryCounter
        );
        // A lower lineage epoch (legitimate rollback shape) does not
        // exempt the message from the delivery-counter replay guard.
        assert_eq!(
            state
                .apply(ChannelUpdate::rollback(epoch(1), counter(5), binding(1)))
                .unwrap_err(),
            ChannelStateError::StaleDeliveryCounter
        );
    }

    #[test]
    fn retire_then_current_rotate_and_rollback_are_refused() {
        let state = ChannelState::new();
        state
            .apply(ChannelUpdate::provision(epoch(1), counter(1), binding(1)))
            .expect("provision succeeds");
        state
            .apply(ChannelUpdate::retire(counter(2)))
            .expect("retire succeeds");
        assert!(state.is_retired());
        assert_eq!(
            state.with_current(|_, _| ()).unwrap_err(),
            ChannelStateError::Retired
        );
        assert_eq!(
            state
                .apply(ChannelUpdate::rotate(epoch(2), counter(3), binding(2)))
                .unwrap_err(),
            ChannelStateError::Retired
        );
        assert_eq!(
            state
                .apply(ChannelUpdate::rollback(epoch(1), counter(3), binding(1)))
                .unwrap_err(),
            ChannelStateError::Retired
        );
    }

    #[test]
    fn retire_then_reprovision_resets_lineage_epoch_but_not_delivery_high_water() {
        let state = ChannelState::new();
        state
            .apply(ChannelUpdate::provision(epoch(1), counter(9), binding(1)))
            .expect("provision succeeds");
        state
            .apply(ChannelUpdate::retire(counter(10)))
            .expect("retire succeeds");
        assert_eq!(state.delivery_high_water(), 10);

        // A fresh provision restarts the lineage epoch at 1 (mirrors
        // the broker's on-disk generation restart), but a replayed
        // delivery_counter from BEFORE the retire (or equal to the
        // retire's own counter) must still be rejected: the anti-replay
        // high-water mark is a property of this channel object, not
        // of the rollbackable storage generation, and must survive
        // retire + reprovision.
        assert_eq!(
            state
                .apply(ChannelUpdate::provision(epoch(1), counter(9), binding(2)))
                .unwrap_err(),
            ChannelStateError::StaleDeliveryCounter
        );
        assert_eq!(
            state
                .apply(ChannelUpdate::provision(epoch(1), counter(10), binding(2)))
                .unwrap_err(),
            ChannelStateError::StaleDeliveryCounter
        );

        // A genuinely fresh, higher counter succeeds and resets the
        // lineage epoch back to 1.
        state
            .apply(ChannelUpdate::provision(epoch(1), counter(11), binding(2)))
            .expect("reprovision with a fresh counter succeeds");
        assert!(!state.is_retired());
        let epoch_value = state.with_current(|_, e| e.get()).unwrap();
        assert_eq!(epoch_value, 1);
        assert_eq!(state.delivery_high_water(), 11);
    }

    #[test]
    fn retire_is_idempotent_and_still_replay_guarded() {
        let state = ChannelState::new();
        state
            .apply(ChannelUpdate::retire(counter(1)))
            .expect("retire of never-provisioned state succeeds");
        assert!(state.is_retired());
        state
            .apply(ChannelUpdate::retire(counter(2)))
            .expect("re-retire with a fresh counter succeeds");
        assert_eq!(
            state.apply(ChannelUpdate::retire(counter(2))).unwrap_err(),
            ChannelStateError::StaleDeliveryCounter
        );
    }

    #[test]
    fn provision_after_reject_leaves_state_unchanged() {
        let state = ChannelState::new();
        state
            .apply(ChannelUpdate::provision(epoch(1), counter(1), binding(1)))
            .expect("provision succeeds");
        // A stale-counter provision attempt must not disturb the
        // already-active material.
        let _ = state.apply(ChannelUpdate::provision(epoch(9), counter(1), binding(9)));
        let bytes = state.with_current(|b, _| b.expose_bytes()).unwrap();
        assert_eq!(bytes, [1u8; 32]);
    }

    #[test]
    fn concurrent_apply_with_same_counter_admits_exactly_one_winner() {
        let state = Arc::new(ChannelState::new());
        state
            .apply(ChannelUpdate::provision(epoch(1), counter(1), binding(1)))
            .expect("provision succeeds");

        let mut handles = Vec::new();
        for i in 0..8u8 {
            let state = Arc::clone(&state);
            handles.push(thread::spawn(move || {
                state.apply(ChannelUpdate::rotate(epoch(2), counter(2), binding(i + 2)))
            }));
        }
        let results: Vec<Result<(), ChannelStateError>> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();
        let successes = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(successes, 1, "exactly one concurrent rotate should win");
        let stale_failures = results
            .iter()
            .filter(|r| *r == &Err(ChannelStateError::StaleDeliveryCounter))
            .count();
        assert_eq!(stale_failures, 7);
    }

    #[test]
    fn action_accessor_matches_constructor() {
        let update = ChannelUpdate::provision(epoch(1), counter(1), binding(1));
        assert_eq!(update.action(), ChannelAction::Provision);
        let update = ChannelUpdate::retire(counter(2));
        assert_eq!(update.action(), ChannelAction::Retire);
    }
}
