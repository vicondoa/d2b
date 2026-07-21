//! Guest-side lifecycle state for the security-key CTAPHID channel
//! binding + reconnect generation that
//! [`crate::services::security_key::SessionConfig`] ultimately
//! consumes.
//!
//! This module is the W8 `secrets-lifecycle` component's guest-side
//! counterpart to the broker's
//! `d2b_priv_broker::ops::secrets_lifecycle` engine and
//! `SecretKind::SecurityKeyChannelState`. It is intentionally a
//! standalone, in-memory, zero-internal-dependency module (only
//! `std`) so it can be validated on its own without depending on this
//! crate's `d2b-session`/`d2b-contracts`/transport wiring, and so a
//! future integrator can wire it into
//! [`crate::services::security_key::SessionConfig`] without this
//! component needing to touch that file.
//!
//! # Integration wiring points (deliberately NOT performed here)
//!
//!   1. `src/lib.rs` needs `pub mod secrets_channel;`.
//!   2. `services/security_key/mod.rs`'s `SessionConfig::from_env` (or
//!      a new constructor alongside it) should source its
//!      `channel_binding`/`reconnect_generation` from a
//!      [`ChannelState::current`] snapshot — most likely populated by
//!      a fresh vsock/guest-control message carrying broker-rotated
//!      `SecretKind::SecurityKeyChannelState` material — instead of
//!      the static `D2B_SK_CHANNEL_BINDING_HEX`/
//!      `D2B_SK_RECONNECT_GENERATION` environment variables it reads
//!      today. Neither `lib.rs` nor `services/security_key/mod.rs` is
//!      owned by this component.
//!   3. The exact wire format [`ChannelMaterial::from_wire_bytes`]
//!      parses (32 bytes of channel binding followed by an 8-byte
//!      big-endian generation counter) is **a proposal**, not a
//!      confirmed contract: the integrator must confirm it against
//!      whatever the broker's future `SecretKind::SecurityKeyChannelState`
//!      dispatch arm (in `packages/d2b-priv-broker/src/runtime.rs`,
//!      out of scope here) actually serializes, and adjust or replace
//!      it before depending on it in production.

use std::fmt;
use std::sync::RwLock;

/// Errors surfaced by every public function in this module. Never
/// carries a raw byte, path, or generation counter value — only a
/// closed-set discriminant, safe for any Debug/log/audit surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelStateError {
    InvalidBinding,
    InvalidGeneration,
    /// `rotate` was called with a generation that is not strictly
    /// greater than the currently active one — refused as a possible
    /// replay/rollback rather than silently accepted.
    StaleGeneration,
    /// `rotate`/`current` was called before any `provision`.
    NotProvisioned,
    /// `provision` was called while material is already active
    /// (callers should `rotate` instead).
    AlreadyProvisioned,
    /// The channel state was retired; `current`/`rotate` refuse to
    /// resume without an explicit fresh `provision`.
    Retired,
    /// `from_wire_bytes` was given a buffer of the wrong length.
    InvalidWireLength,
}

impl fmt::Display for ChannelStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let slug = match self {
            Self::InvalidBinding => "invalid-channel-binding",
            Self::InvalidGeneration => "invalid-reconnect-generation",
            Self::StaleGeneration => "stale-reconnect-generation",
            Self::NotProvisioned => "channel-not-provisioned",
            Self::AlreadyProvisioned => "channel-already-provisioned",
            Self::Retired => "channel-retired",
            Self::InvalidWireLength => "invalid-channel-material-wire-length",
        };
        f.write_str(slug)
    }
}

impl std::error::Error for ChannelStateError {}

/// Length in bytes of the [`ChannelMaterial::from_wire_bytes`]
/// proposed wire format: 32 bytes of channel binding plus an 8-byte
/// big-endian generation counter. See the module-level "Integration
/// wiring points" note — this exact layout needs integrator
/// confirmation before any production dependency on it.
pub const PROPOSED_WIRE_LEN: usize = 32 + 8;

/// A validated, non-zero 32-byte CTAPHID channel-binding value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ChannelBinding([u8; 32]);

impl ChannelBinding {
    pub fn new(bytes: [u8; 32]) -> Result<Self, ChannelStateError> {
        if bytes == [0; 32] {
            return Err(ChannelStateError::InvalidBinding);
        }
        Ok(Self(bytes))
    }

    pub fn into_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for ChannelBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ChannelBinding")
            .field(&"<redacted>")
            .finish()
    }
}

/// A validated, non-zero reconnect-generation counter.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChannelGeneration(u64);

impl ChannelGeneration {
    pub fn new(value: u64) -> Result<Self, ChannelStateError> {
        if value == 0 {
            return Err(ChannelStateError::InvalidGeneration);
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for ChannelGeneration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ChannelGeneration")
            .field(&"<redacted>")
            .finish()
    }
}

/// One provisioned/rotated channel material snapshot: a binding plus
/// the generation counter it was delivered at.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ChannelMaterial {
    binding: ChannelBinding,
    generation: ChannelGeneration,
}

impl fmt::Debug for ChannelMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChannelMaterial")
            .field("binding", &"<redacted>")
            .field("generation", &"<redacted>")
            .finish()
    }
}

impl ChannelMaterial {
    pub fn new(binding: ChannelBinding, generation: ChannelGeneration) -> Self {
        Self {
            binding,
            generation,
        }
    }

    pub fn binding(&self) -> ChannelBinding {
        self.binding
    }

    pub fn generation(&self) -> ChannelGeneration {
        self.generation
    }

    /// Convenience accessor returning the plain `([u8; 32], u64)`
    /// shape [`crate::services::security_key::SessionConfig::new`]
    /// expects, without this module depending on that type directly.
    pub fn into_session_config_args(self) -> ([u8; 32], u64) {
        (self.binding.into_bytes(), self.generation.get())
    }

    /// Parse the **proposed** wire format: 32 bytes of channel
    /// binding followed by an 8-byte big-endian generation counter.
    /// See the module-level doc comment — this layout is a proposal
    /// pending integrator/security confirmation, not a settled
    /// contract.
    pub fn from_wire_bytes(bytes: &[u8]) -> Result<Self, ChannelStateError> {
        if bytes.len() != PROPOSED_WIRE_LEN {
            return Err(ChannelStateError::InvalidWireLength);
        }
        let mut binding_bytes = [0u8; 32];
        binding_bytes.copy_from_slice(&bytes[..32]);
        let mut generation_bytes = [0u8; 8];
        generation_bytes.copy_from_slice(&bytes[32..]);
        let binding = ChannelBinding::new(binding_bytes)?;
        let generation = ChannelGeneration::new(u64::from_be_bytes(generation_bytes))?;
        Ok(Self::new(binding, generation))
    }
}

struct Inner {
    material: Option<ChannelMaterial>,
    retired: bool,
}

/// In-memory, thread-safe holder for the guest frontend's active
/// channel material, supporting the same provision/rotate/retire
/// lifecycle shape as the broker's `secrets_lifecycle` engine (kept
/// independent — this module has no dependency on that crate).
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
        }))
    }

    /// Provision fresh channel material. Fails with
    /// [`ChannelStateError::AlreadyProvisioned`] if material is
    /// already active (use [`rotate`](Self::rotate) instead).
    pub fn provision(&self, material: ChannelMaterial) -> Result<(), ChannelStateError> {
        let mut inner = self.0.write().unwrap_or_else(|poison| poison.into_inner());
        if inner.material.is_some() && !inner.retired {
            return Err(ChannelStateError::AlreadyProvisioned);
        }
        inner.material = Some(material);
        inner.retired = false;
        Ok(())
    }

    /// Rotate to new material. Requires an active (non-retired)
    /// provisioning and a strictly greater generation than the one
    /// currently active — a stale or replayed generation is refused
    /// rather than silently accepted.
    pub fn rotate(&self, material: ChannelMaterial) -> Result<(), ChannelStateError> {
        let mut inner = self.0.write().unwrap_or_else(|poison| poison.into_inner());
        if inner.retired {
            return Err(ChannelStateError::Retired);
        }
        match inner.material {
            None => return Err(ChannelStateError::NotProvisioned),
            Some(current) if material.generation <= current.generation => {
                return Err(ChannelStateError::StaleGeneration);
            }
            Some(_) => {}
        }
        inner.material = Some(material);
        Ok(())
    }

    /// Retire the channel state. Idempotent: retiring an
    /// already-retired or never-provisioned state simply clears any
    /// material and marks retired.
    pub fn retire(&self) {
        let mut inner = self.0.write().unwrap_or_else(|poison| poison.into_inner());
        inner.material = None;
        inner.retired = true;
    }

    /// Current active material, if any.
    pub fn current(&self) -> Result<ChannelMaterial, ChannelStateError> {
        let inner = self.0.read().unwrap_or_else(|poison| poison.into_inner());
        if inner.retired {
            return Err(ChannelStateError::Retired);
        }
        inner.material.ok_or(ChannelStateError::NotProvisioned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(byte: u8) -> ChannelBinding {
        ChannelBinding::new([byte; 32]).expect("valid binding")
    }

    fn generation(value: u64) -> ChannelGeneration {
        ChannelGeneration::new(value).expect("valid generation")
    }

    #[test]
    fn zero_binding_and_generation_are_rejected() {
        assert_eq!(
            ChannelBinding::new([0; 32]).unwrap_err(),
            ChannelStateError::InvalidBinding
        );
        assert_eq!(
            ChannelGeneration::new(0).unwrap_err(),
            ChannelStateError::InvalidGeneration
        );
    }

    #[test]
    fn debug_impls_never_expose_bytes_or_counters() {
        let b = binding(0xAB);
        let g = generation(7);
        let m = ChannelMaterial::new(b, g);
        assert!(format!("{b:?}").contains("redacted"));
        assert!(format!("{g:?}").contains("redacted"));
        let m_debug = format!("{m:?}");
        assert!(m_debug.contains("redacted"));
        assert!(!m_debug.contains("171")); // 0xAB decimal
        assert!(!m_debug.contains('7'));
    }

    #[test]
    fn current_before_provision_is_not_provisioned() {
        let state = ChannelState::new();
        assert_eq!(
            state.current().unwrap_err(),
            ChannelStateError::NotProvisioned
        );
    }

    #[test]
    fn provision_then_current_round_trips() {
        let state = ChannelState::new();
        let material = ChannelMaterial::new(binding(1), generation(1));
        state.provision(material).expect("provision succeeds");
        assert_eq!(state.current().expect("current succeeds"), material);
    }

    #[test]
    fn provision_twice_without_retire_is_rejected() {
        let state = ChannelState::new();
        state
            .provision(ChannelMaterial::new(binding(1), generation(1)))
            .expect("first provision succeeds");
        let err = state
            .provision(ChannelMaterial::new(binding(2), generation(1)))
            .unwrap_err();
        assert_eq!(err, ChannelStateError::AlreadyProvisioned);
    }

    #[test]
    fn rotate_requires_strictly_increasing_generation() {
        let state = ChannelState::new();
        state
            .provision(ChannelMaterial::new(binding(1), generation(5)))
            .expect("provision succeeds");
        let stale = state.rotate(ChannelMaterial::new(binding(2), generation(5)));
        assert_eq!(stale.unwrap_err(), ChannelStateError::StaleGeneration);
        let older = state.rotate(ChannelMaterial::new(binding(2), generation(4)));
        assert_eq!(older.unwrap_err(), ChannelStateError::StaleGeneration);
        state
            .rotate(ChannelMaterial::new(binding(2), generation(6)))
            .expect("strictly-increasing rotate succeeds");
        assert_eq!(state.current().unwrap().generation().get(), 6);
    }

    #[test]
    fn rotate_without_provision_is_not_provisioned() {
        let state = ChannelState::new();
        let err = state
            .rotate(ChannelMaterial::new(binding(1), generation(1)))
            .unwrap_err();
        assert_eq!(err, ChannelStateError::NotProvisioned);
    }

    #[test]
    fn retire_then_current_and_rotate_are_refused() {
        let state = ChannelState::new();
        state
            .provision(ChannelMaterial::new(binding(1), generation(1)))
            .expect("provision succeeds");
        state.retire();
        assert_eq!(state.current().unwrap_err(), ChannelStateError::Retired);
        assert_eq!(
            state
                .rotate(ChannelMaterial::new(binding(2), generation(2)))
                .unwrap_err(),
            ChannelStateError::Retired
        );
    }

    #[test]
    fn retire_then_reprovision_resets_generation_tracking() {
        let state = ChannelState::new();
        state
            .provision(ChannelMaterial::new(binding(1), generation(9)))
            .expect("provision succeeds");
        state.retire();
        state
            .provision(ChannelMaterial::new(binding(2), generation(1)))
            .expect("re-provision after retire succeeds");
        assert_eq!(state.current().unwrap().generation().get(), 1);
    }

    #[test]
    fn retire_is_idempotent() {
        let state = ChannelState::new();
        state.retire();
        state.retire();
        assert_eq!(state.current().unwrap_err(), ChannelStateError::Retired);
    }

    #[test]
    fn from_wire_bytes_round_trips_and_rejects_bad_length() {
        let mut wire = [0u8; PROPOSED_WIRE_LEN];
        wire[..32].copy_from_slice(&[7u8; 32]);
        wire[32..].copy_from_slice(&42u64.to_be_bytes());
        let material = ChannelMaterial::from_wire_bytes(&wire).expect("valid wire bytes parse");
        assert_eq!(material.binding().into_bytes(), [7u8; 32]);
        assert_eq!(material.generation().get(), 42);

        assert_eq!(
            ChannelMaterial::from_wire_bytes(&wire[..PROPOSED_WIRE_LEN - 1]).unwrap_err(),
            ChannelStateError::InvalidWireLength
        );
    }

    #[test]
    fn from_wire_bytes_rejects_zero_binding_or_generation() {
        let mut zero_binding = [0u8; PROPOSED_WIRE_LEN];
        zero_binding[32..].copy_from_slice(&1u64.to_be_bytes());
        assert_eq!(
            ChannelMaterial::from_wire_bytes(&zero_binding).unwrap_err(),
            ChannelStateError::InvalidBinding
        );

        let mut zero_generation = [0u8; PROPOSED_WIRE_LEN];
        zero_generation[..32].copy_from_slice(&[3u8; 32]);
        assert_eq!(
            ChannelMaterial::from_wire_bytes(&zero_generation).unwrap_err(),
            ChannelStateError::InvalidGeneration
        );
    }

    #[test]
    fn into_session_config_args_matches_material() {
        let material = ChannelMaterial::new(binding(5), generation(3));
        let (bytes, generation_value) = material.into_session_config_args();
        assert_eq!(bytes, [5u8; 32]);
        assert_eq!(generation_value, 3);
    }
}
