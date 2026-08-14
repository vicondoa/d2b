//! Single-use, in-memory action capabilities.

use getrandom::getrandom;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const NONCE_BYTES: usize = 32;
const MAX_ACTION_BYTES: usize = 64;

/// An opaque action nonce.
#[derive(Clone, PartialEq, Eq)]
pub struct ActionNonce {
    token: String,
    session_digest: [u8; NONCE_BYTES],
    action: String,
    expires_at: u64,
}

impl ActionNonce {
    /// Return the fixed D-Bus action key.
    pub fn action_key(&self) -> String {
        format!("d2b-action:{}", self.token)
    }
}

impl core::fmt::Debug for ActionNonce {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ActionNonce(REDACTED)")
    }
}

/// Action capability failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionNonceError {
    /// The nonce store has reached its capacity.
    Capacity,
    /// Random nonce generation failed.
    Entropy,
    /// The action key is malformed or not present.
    Unavailable,
    /// The action key has expired.
    Expired,
    /// The authenticated observer session does not match.
    SessionMismatch,
    /// The action ID does not match.
    ActionMismatch,
    /// The action text exceeds the bounded capability representation.
    Invalid,
}

impl core::fmt::Display for ActionNonceError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Capacity => "action-capability-capacity",
            Self::Entropy => "action-capability-unavailable",
            Self::Unavailable => "action-capability-unavailable",
            Self::Expired => "action-capability-expired",
            Self::SessionMismatch | Self::ActionMismatch => "action-capability-denied",
            Self::Invalid => "action-capability-invalid",
        })
    }
}

impl std::error::Error for ActionNonceError {}

/// Bounded in-memory action nonce store.
pub struct ActionNonceStore {
    capacity: usize,
    ttl_secs: u64,
    entries: BTreeMap<String, ActionNonce>,
}

impl ActionNonceStore {
    /// Construct a store with an explicit capacity and TTL.
    pub fn new(capacity: usize, ttl_secs: u64) -> Self {
        Self {
            capacity,
            ttl_secs,
            entries: BTreeMap::new(),
        }
    }

    /// Register one action capability.
    pub fn register(
        &mut self,
        session: impl AsRef<str>,
        action: impl AsRef<str>,
        now_secs: u64,
    ) -> Result<ActionNonce, ActionNonceError> {
        self.gc(now_secs);
        if self.entries.len() >= self.capacity {
            return Err(ActionNonceError::Capacity);
        }
        let session = session.as_ref();
        let action = action.as_ref();
        if action.len() > MAX_ACTION_BYTES {
            return Err(ActionNonceError::Invalid);
        }
        let mut raw = [0_u8; NONCE_BYTES];
        getrandom(&mut raw).map_err(|_| ActionNonceError::Entropy)?;
        let token = raw
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let nonce = ActionNonce {
            token: token.clone(),
            session_digest: session_digest(session),
            action: action.to_owned(),
            expires_at: now_secs.saturating_add(self.ttl_secs),
        };
        self.entries.insert(token, nonce.clone());
        Ok(nonce)
    }

    /// Validate and consume a capability.
    pub fn consume(
        &mut self,
        action_key: &str,
        session: &str,
        now_secs: u64,
    ) -> Result<String, ActionNonceError> {
        self.consume_for_action(action_key, session, None, now_secs)
    }

    /// Validate and consume a capability while checking the stable action ID.
    pub fn consume_for_action(
        &mut self,
        action_key: &str,
        session: &str,
        expected_action: Option<&str>,
        now_secs: u64,
    ) -> Result<String, ActionNonceError> {
        let token = Self::token_from_key(action_key).ok_or(ActionNonceError::Unavailable)?;
        let nonce = self
            .entries
            .get(token)
            .ok_or(ActionNonceError::Unavailable)?;
        if now_secs >= nonce.expires_at {
            self.entries.remove(token);
            return Err(ActionNonceError::Expired);
        }
        if nonce.session_digest != session_digest(session) {
            return Err(ActionNonceError::SessionMismatch);
        }
        if expected_action.is_some_and(|action| action != nonce.action) {
            return Err(ActionNonceError::ActionMismatch);
        }
        let action = nonce.action.clone();
        self.entries.remove(token);
        Ok(action)
    }

    /// Revoke one capability without consuming it.
    pub fn revoke(&mut self, action_key: &str) -> bool {
        Self::token_from_key(action_key)
            .and_then(|token| self.entries.remove(token))
            .is_some()
    }

    /// Remove expired entries.
    pub fn gc(&mut self, now_secs: u64) {
        self.entries.retain(|_, nonce| nonce.expires_at > now_secs);
    }

    /// Return the current in-memory entry count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether an action key is still live in the store.
    pub(crate) fn contains(&self, action_key: &str) -> bool {
        Self::token_from_key(action_key).is_some_and(|token| self.entries.contains_key(token))
    }

    /// Return how many additional entries can be registered.
    pub fn available_capacity(&self) -> usize {
        self.capacity.saturating_sub(self.entries.len())
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all in-memory state while preserving the configured bounds.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Revoke every capability owned by one authenticated session.
    pub(crate) fn revoke_session(&mut self, session: &str) {
        let digest = session_digest(session);
        self.entries.retain(|_, nonce| nonce.session_digest != digest);
    }

    fn token_from_key(action_key: &str) -> Option<&str> {
        action_key.strip_prefix("d2b-action:").filter(|token| {
            token.len() == NONCE_BYTES * 2 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    }
}

fn session_digest(session: &str) -> [u8; NONCE_BYTES] {
    Sha256::digest(session.as_bytes()).into()
}

impl core::fmt::Debug for ActionNonceStore {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ActionNonceStore")
            .field("capacity", &self.capacity)
            .field("ttl_secs", &self.ttl_secs)
            .field("entry_count", &self.entries.len())
            .finish()
    }
}
