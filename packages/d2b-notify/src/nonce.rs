// SPDX-License-Identifier: Apache-2.0
//! Opaque, single-use capabilities for authenticated desktop actions.
//!
//! A capability identifies no command, workload, VM, or ceremony. The action
//! service keeps that authority server-side and accepts the capability only on
//! its already-authenticated `ComponentSession`.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const NONCE_TTL_SECS: u64 = 120;
pub const NONCE_BYTES: usize = 32;
pub const MAX_STORE_SIZE: usize = 256;
pub const NOTIFICATION_ACTION_PREFIX: &str = "d2b-action:";

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ActionNonce(String);

impl ActionNonce {
    pub fn parse(value: &str) -> Result<Self, NonceError> {
        if value.len() != NONCE_BYTES * 2
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(NonceError::Invalid);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ActionNonce {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ActionNonce(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonceError {
    Invalid,
    MissingOrConsumed,
    Expired,
    Capacity,
    EntropyUnavailable,
}

impl std::fmt::Display for NonceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = match self {
            Self::Invalid => "action-capability-invalid",
            Self::MissingOrConsumed => "action-capability-unavailable",
            Self::Expired => "action-capability-expired",
            Self::Capacity => "action-capability-capacity",
            Self::EntropyUnavailable => "action-capability-entropy-unavailable",
        };
        formatter.write_str(code)
    }
}

impl std::error::Error for NonceError {}

struct NonceMeta<T> {
    value: T,
    expires_at: u64,
}

pub struct ActionNonceStore<T> {
    entries: HashMap<ActionNonce, NonceMeta<T>>,
}

impl<T> std::fmt::Debug for ActionNonceStore<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActionNonceStore")
            .field("entry_count", &self.entries.len())
            .finish()
    }
}

impl<T> Default for ActionNonceStore<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<T> ActionNonceStore<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn issue(&mut self, value: T, now_secs: u64) -> Result<ActionNonce, NonceError> {
        self.gc(now_secs);
        if self.entries.len() >= MAX_STORE_SIZE {
            return Err(NonceError::Capacity);
        }

        let mut raw = [0u8; NONCE_BYTES];
        getrandom::getrandom(&mut raw).map_err(|_| NonceError::EntropyUnavailable)?;
        let nonce = ActionNonce(hex_encode(&raw));
        if self.entries.contains_key(&nonce) {
            return Err(NonceError::EntropyUnavailable);
        }
        self.entries.insert(
            nonce.clone(),
            NonceMeta {
                value,
                expires_at: now_secs.saturating_add(NONCE_TTL_SECS),
            },
        );
        Ok(nonce)
    }

    pub fn consume(&mut self, nonce: &ActionNonce, now_secs: u64) -> Result<T, NonceError> {
        let entry = self
            .entries
            .remove(nonce)
            .ok_or(NonceError::MissingOrConsumed)?;
        if now_secs >= entry.expires_at {
            return Err(NonceError::Expired);
        }
        Ok(entry.value)
    }

    pub fn gc(&mut self, now_secs: u64) {
        self.entries.retain(|_, entry| entry.expires_at > now_secs);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn notification_action_key(nonce: &ActionNonce) -> String {
    format!("{NOTIFICATION_ACTION_PREFIX}{}", nonce.expose())
}

pub fn parse_notification_action_key(value: &str) -> Result<ActionNonce, NonceError> {
    let nonce = value
        .strip_prefix(NOTIFICATION_ACTION_PREFIX)
        .ok_or(NonceError::Invalid)?;
    ActionNonce::parse(nonce)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_750_000_000;

    #[test]
    fn capability_is_single_use_and_carries_no_target() {
        let mut store = ActionNonceStore::new();
        let nonce = store.issue("internal-target", NOW).unwrap();
        assert_eq!(nonce.expose().len(), NONCE_BYTES * 2);
        assert!(!nonce.expose().contains("internal-target"));
        assert_eq!(store.consume(&nonce, NOW + 1).unwrap(), "internal-target");
        assert_eq!(
            store.consume(&nonce, NOW + 1),
            Err(NonceError::MissingOrConsumed)
        );
    }

    #[test]
    fn expired_capability_is_consumed_fail_closed() {
        let mut store = ActionNonceStore::new();
        let nonce = store.issue((), NOW).unwrap();
        assert_eq!(
            store.consume(&nonce, NOW + NONCE_TTL_SECS),
            Err(NonceError::Expired)
        );
        assert_eq!(
            store.consume(&nonce, NOW + 1),
            Err(NonceError::MissingOrConsumed)
        );
    }

    #[test]
    fn store_is_bounded_without_evicting_live_authority() {
        let mut store = ActionNonceStore::new();
        for index in 0..MAX_STORE_SIZE {
            store.issue(index, NOW).unwrap();
        }
        assert_eq!(store.len(), MAX_STORE_SIZE);
        assert_eq!(store.issue(MAX_STORE_SIZE, NOW), Err(NonceError::Capacity));
    }

    #[test]
    fn notification_key_round_trip_is_opaque() {
        let mut store = ActionNonceStore::new();
        let nonce = store.issue((), NOW).unwrap();
        let key = notification_action_key(&nonce);
        assert!(key.starts_with(NOTIFICATION_ACTION_PREFIX));
        assert!(!key.contains("cancel"));
        assert_eq!(parse_notification_action_key(&key).unwrap(), nonce);
        assert!(parse_notification_action_key("cancel:aaaa").is_err());
    }

    #[test]
    fn debug_and_errors_do_not_disclose_capabilities() {
        let nonce = ActionNonce::parse(&"a".repeat(NONCE_BYTES * 2)).unwrap();
        assert_eq!(format!("{nonce:?}"), "ActionNonce(<redacted>)");
        assert!(
            !NonceError::MissingOrConsumed
                .to_string()
                .contains(nonce.expose())
        );
    }
}
