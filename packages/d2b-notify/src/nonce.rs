// SPDX-License-Identifier: Apache-2.0
//! Single-use CSPRNG action nonces for Freedesktop notification actions.
//!
//! ## Security model
//!
//! A Freedesktop desktop notification can carry action buttons (e.g.
//! "Cancel request"). When the user clicks a button, the notification daemon
//! emits a `NotificationClosed` or `ActionInvoked` D-Bus signal carrying the
//! action key string that was registered with `Notify`. Because _any_ process
//! connected to the session D-Bus can monitor those signals, an action key
//! that encodes a bare VM name or session ID could be replayed by a hostile
//! desktop client to cancel someone else's ceremony.
//!
//! The nonce layer prevents this: the action key embeds a 32-byte CSPRNG
//! token that the daemon registered in [`ActionNonceStore`] at notification
//! emit time. The callback (d2bd or CLI) validates the token before executing
//! the action. Validation is fail-closed: missing, expired, or mismatched
//! tokens are rejected; the token is consumed on first valid use.
//!
//! ## Token wire format
//!
//! Action keys passed to the notification daemon use the format:
//!
//! ```text
//! d2b-sk-<action_key>:<hex_nonce_64_chars>
//! ```
//!
//! Example: `d2b-sk-cancel:a3f1...dead` (64 hex digits).
//!
//! The daemon registers the token immediately before emitting the
//! notification; the callback decodes the action key and calls
//! [`ActionNonceStore::validate_and_consume`].

use std::collections::HashMap;

/// Lifetime of a newly minted nonce in seconds.  Chosen to outlive the
/// notification's on-screen lifetime (typically ≤ 30 s) with margin, while
/// still providing a hard expiry so a delayed or retried callback cannot
/// succeed minutes later.
pub const NONCE_TTL_SECS: u64 = 120;

/// Length of the raw entropy bytes.
pub const NONCE_BYTES: usize = 32;

/// Error returned by [`ActionNonceStore::validate_and_consume`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NonceError {
    /// The token was not found in the store (never registered, already
    /// consumed, or GC'd after expiry).
    NotFound,
    /// The token exists but its wall-clock expiry has passed.
    Expired,
    /// The token exists and is live but the provided `session_id` does not
    /// match the one bound at registration time.
    SessionMismatch,
    /// The token exists and is live but the provided `action_key` does not
    /// match the one bound at registration time.
    ActionMismatch,
}

impl std::fmt::Display for NonceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("nonce not found (consumed, expired, or never issued)"),
            Self::Expired => f.write_str("nonce has expired"),
            Self::SessionMismatch => f.write_str("nonce session_id mismatch"),
            Self::ActionMismatch => f.write_str("nonce action_key mismatch"),
        }
    }
}

impl std::error::Error for NonceError {}

/// Metadata stored for a registered nonce.
#[derive(Debug, Clone)]
struct NonceMeta {
    session_id: String,
    action_key: String,
    expires_at: u64,
}

/// Server-side single-use nonce registry.
///
/// The store is intentionally `!Send` (caller-owned map, no Arc/Mutex): in
/// production, the daemon holds it on a single Tokio task that processes
/// notification-action callbacks. Tests inject it directly.
///
/// Nonces are _consumed_ on successful validation — a second call with the
/// same token returns [`NonceError::NotFound`].
///
/// Expired entries are garbage-collected lazily on any
/// [`ActionNonceStore::gc`] call (explicit) or on insert when the store
/// exceeds [`MAX_STORE_SIZE`].
pub const MAX_STORE_SIZE: usize = 256;

#[derive(Debug, Default)]
pub struct ActionNonceStore {
    map: HashMap<String, NonceMeta>,
}

impl ActionNonceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint and register a new nonce token.
    ///
    /// Returns the hex-encoded 64-character token on success.
    ///
    /// The `now_secs` parameter is the current Unix timestamp in seconds; it
    /// lets tests inject a fixed clock without mocking `SystemTime`. Pass
    /// `unix_now_secs()` in production.
    pub fn register(
        &mut self,
        session_id: impl Into<String>,
        action_key: impl Into<String>,
        now_secs: u64,
    ) -> Result<String, getrandom::Error> {
        self.gc(now_secs);
        let mut raw = [0u8; NONCE_BYTES];
        getrandom::getrandom(&mut raw)?;
        let token = hex_encode(&raw);
        self.map.insert(
            token.clone(),
            NonceMeta {
                session_id: session_id.into(),
                action_key: action_key.into(),
                expires_at: now_secs.saturating_add(NONCE_TTL_SECS),
            },
        );
        Ok(token)
    }

    /// Validate and consume a nonce token.
    ///
    /// Checks (in order):
    /// 1. Token exists in the store.
    /// 2. Token has not expired (current time < `expires_at`).
    /// 3. `session_id` matches the registered value.
    /// 4. `action_key` matches the registered value.
    ///
    /// On success the entry is removed (single-use). On failure the entry is
    /// **not** removed — the caller may log the error and the token remains
    /// in the store until it expires or is GC'd.
    pub fn validate_and_consume(
        &mut self,
        token: &str,
        session_id: &str,
        action_key: &str,
        now_secs: u64,
    ) -> Result<(), NonceError> {
        let meta = self.map.get(token).ok_or(NonceError::NotFound)?;
        if now_secs >= meta.expires_at {
            return Err(NonceError::Expired);
        }
        if meta.session_id != session_id {
            return Err(NonceError::SessionMismatch);
        }
        if meta.action_key != action_key {
            return Err(NonceError::ActionMismatch);
        }
        self.map.remove(token);
        Ok(())
    }

    /// Remove all expired entries (those with `expires_at <= now_secs`).
    pub fn gc(&mut self, now_secs: u64) {
        self.map.retain(|_, meta| meta.expires_at > now_secs);
        // Hard-evict oldest entries if the store is still over the size limit
        // after GC. In practice this should never trigger under normal
        // operation; if it does, it indicates a programming error (nonces
        // registered faster than they are consumed or expired).
        if self.map.len() > MAX_STORE_SIZE {
            // Evict the entries with the smallest `expires_at`.
            let mut entries: Vec<(String, u64)> = self
                .map
                .iter()
                .map(|(k, v)| (k.clone(), v.expires_at))
                .collect();
            entries.sort_by_key(|(_, exp)| *exp);
            let to_remove = entries.len() - MAX_STORE_SIZE;
            for (token, _) in entries.into_iter().take(to_remove) {
                self.map.remove(&token);
            }
        }
    }

    /// Number of live entries (including potentially expired ones not yet GC'd).
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Encode raw bytes as a lowercase hex string.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parse the Freedesktop notification action key produced by
/// [`action_key_for`] into its `(action_key_prefix, nonce)` parts.
///
/// Returns `None` for any string that does not have the `d2b-sk-<key>:<hex>`
/// shape.
pub fn parse_action_key(raw: &str) -> Option<(&str, &str)> {
    let rest = raw.strip_prefix("d2b-sk-")?;
    let colon = rest.find(':')?;
    let action = &rest[..colon];
    let nonce = &rest[colon + 1..];
    if action.is_empty() || nonce.len() != NONCE_BYTES * 2 {
        return None;
    }
    if !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((action, nonce))
}

/// Produce the Freedesktop notification action key string for a given action
/// and pre-minted nonce token.
///
/// Format: `d2b-sk-<action_key>:<hex_nonce_64_chars>`
pub fn action_key_for(action_key: &str, nonce: &str) -> String {
    format!("d2b-sk-{action_key}:{nonce}")
}

/// Current Unix timestamp in seconds for use in production callers.
///
/// Falls back to `0` only if `SystemTime::UNIX_EPOCH` is in the future
/// (impossible on sane clocks).
pub fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_TIME: u64 = 1_750_000_000;

    fn store_with_nonce(session_id: &str, action_key: &str) -> (ActionNonceStore, String) {
        let mut store = ActionNonceStore::new();
        let token = store
            .register(session_id, action_key, BASE_TIME)
            .expect("getrandom must succeed in tests");
        (store, token)
    }

    fn test_nonce(byte: u8, len: usize) -> String {
        String::from_utf8(vec![byte; len]).expect("test nonce bytes are ASCII")
    }

    #[test]
    fn happy_path_validates_and_consumes() {
        let (mut store, token) = store_with_nonce("session-abc", "cancel");
        store
            .validate_and_consume(&token, "session-abc", "cancel", BASE_TIME + 1)
            .expect("valid nonce should be accepted");
        // second use must fail
        assert_eq!(
            store
                .validate_and_consume(&token, "session-abc", "cancel", BASE_TIME + 1)
                .unwrap_err(),
            NonceError::NotFound,
            "consumed nonce must not be reusable"
        );
    }

    #[test]
    fn expired_nonce_is_rejected() {
        let (mut store, token) = store_with_nonce("session-abc", "cancel");
        assert_eq!(
            store
                .validate_and_consume(
                    &token,
                    "session-abc",
                    "cancel",
                    BASE_TIME + NONCE_TTL_SECS + 1
                )
                .unwrap_err(),
            NonceError::Expired
        );
    }

    #[test]
    fn session_mismatch_is_rejected() {
        let (mut store, token) = store_with_nonce("session-abc", "cancel");
        assert_eq!(
            store
                .validate_and_consume(&token, "session-WRONG", "cancel", BASE_TIME + 1)
                .unwrap_err(),
            NonceError::SessionMismatch
        );
    }

    #[test]
    fn action_mismatch_is_rejected() {
        let (mut store, token) = store_with_nonce("session-abc", "cancel");
        assert_eq!(
            store
                .validate_and_consume(&token, "session-abc", "open-status", BASE_TIME + 1)
                .unwrap_err(),
            NonceError::ActionMismatch
        );
    }

    #[test]
    fn missing_token_is_not_found() {
        let mut store = ActionNonceStore::new();
        let missing = test_nonce(b'd', NONCE_BYTES * 2);
        assert_eq!(
            store
                .validate_and_consume(&missing, "session-abc", "cancel", BASE_TIME)
                .unwrap_err(),
            NonceError::NotFound
        );
    }

    #[test]
    fn gc_removes_expired_entries() {
        let mut store = ActionNonceStore::new();
        store.register("s1", "cancel", BASE_TIME).unwrap();
        store.register("s2", "cancel", BASE_TIME + 60).unwrap();
        assert_eq!(store.len(), 2);
        // GC at BASE_TIME + NONCE_TTL_SECS + 1 should expire the first entry.
        store.gc(BASE_TIME + NONCE_TTL_SECS + 1);
        assert_eq!(store.len(), 1, "expired entry should have been GC'd");
    }

    #[test]
    fn action_key_parse_round_trip() {
        let nonce = test_nonce(b'a', NONCE_BYTES * 2);
        let key = action_key_for("cancel", &nonce);
        let (action, parsed_nonce) = parse_action_key(&key).expect("must parse");
        assert_eq!(action, "cancel");
        assert_eq!(parsed_nonce, nonce);
    }

    #[test]
    fn action_key_parse_rejects_short_nonce() {
        let short_nonce = test_nonce(b'a', 3);
        let short = action_key_for("cancel", &short_nonce);
        assert!(parse_action_key(&short).is_none());
    }

    #[test]
    fn action_key_parse_rejects_non_hex_nonce() {
        let bad = format!("d2b-sk-cancel:{}", test_nonce(b'z', NONCE_BYTES * 2));
        assert!(parse_action_key(&bad).is_none());
    }

    #[test]
    fn action_key_parse_rejects_missing_prefix() {
        assert!(parse_action_key("cancel:aabbcc").is_none());
    }

    #[test]
    fn nonce_token_is_64_hex_chars() {
        let (_, token) = store_with_nonce("s", "cancel");
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn failed_validation_does_not_consume_nonce() {
        let (mut store, token) = store_with_nonce("session-abc", "cancel");
        // Wrong session should fail but leave the token in the store.
        let _ = store.validate_and_consume(&token, "wrong-session", "cancel", BASE_TIME + 1);
        // Correct call still succeeds.
        store
            .validate_and_consume(&token, "session-abc", "cancel", BASE_TIME + 1)
            .expect("nonce must still be valid after a failed attempt");
    }
}
