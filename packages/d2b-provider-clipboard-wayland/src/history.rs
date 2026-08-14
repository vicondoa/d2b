//! Bounded in-memory clipboard history and lifecycle controls.

use crate::policy::Policy;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// History operation errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryError {
    /// MIME is not permitted.
    MimeRejected,
    /// MIME indicates secret content.
    SecretHintMime,
    /// Item exceeds the per-item bound.
    ItemTooLarge,
    /// The total quota cannot hold the item.
    TotalQuotaExceeded,
    /// Guest is suspended.
    GuestSuspended,
    /// Guest is rate limited.
    RateLimitExceeded,
    /// Entry is unknown or expired.
    EntryUnavailable,
}

impl core::fmt::Display for HistoryError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::MimeRejected => "mime-rejected",
            Self::SecretHintMime => "secret-hint-mime",
            Self::ItemTooLarge => "item-too-large",
            Self::TotalQuotaExceeded => "total-quota-exceeded",
            Self::GuestSuspended => "zone-suspended",
            Self::RateLimitExceeded => "rate-limit-exceeded",
            Self::EntryUnavailable => "entry-unavailable",
        })
    }
}

impl std::error::Error for HistoryError {}

/// A clipboard item retained only in clipd-host process memory.
pub struct ClipboardEntry {
    token: String,
    guest: String,
    mime: String,
    bytes: Vec<u8>,
    created_at: u64,
}

impl ClipboardEntry {
    /// Validate and construct an in-memory entry.
    pub fn new(
        guest: impl Into<String>,
        mime: impl Into<String>,
        bytes: &[u8],
        created_at: u64,
    ) -> Result<Self, HistoryError> {
        let guest = guest.into();
        let mime = crate::policy::normalize_mime(&mime.into());
        if guest.is_empty() {
            return Err(HistoryError::EntryUnavailable);
        }
        if Policy::is_secret_hint(&mime) {
            return Err(HistoryError::SecretHintMime);
        }
        if !Policy::default().allows_mime(&mime) {
            return Err(HistoryError::MimeRejected);
        }
        let mut hasher = Sha256::new();
        hasher.update(guest.as_bytes());
        hasher.update([0]);
        hasher.update(mime.as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        hasher.update(created_at.to_le_bytes());
        let token = format!("sha256:{:x}", hasher.finalize());
        Ok(Self {
            token,
            guest,
            mime,
            bytes: bytes.to_vec(),
            created_at,
        })
    }

    /// Borrow the opaque entry token.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Borrow the authenticated owner label.
    pub fn owner(&self) -> &str {
        &self.guest
    }

    /// Return the in-memory byte length.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Borrow the allowlisted MIME type.
    pub fn mime(&self) -> &str {
        &self.mime
    }

    /// Whether the entry has no bytes.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl core::fmt::Debug for ClipboardEntry {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ClipboardEntry(<redacted>)")
    }
}

/// Bounded in-memory history.
pub struct ClipboardHistory {
    config: crate::ClipboardConfig,
    entries: BTreeMap<String, ClipboardEntry>,
    order: VecDeque<String>,
    total_bytes: usize,
    suspended: BTreeSet<String>,
    guest_requests: BTreeMap<String, VecDeque<u64>>,
}

impl ClipboardHistory {
    /// Construct an empty history.
    pub fn new(config: crate::ClipboardConfig) -> Result<Self, HistoryError> {
        Ok(Self {
            config,
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            total_bytes: 0,
            suspended: BTreeSet::new(),
            guest_requests: BTreeMap::new(),
        })
    }

    /// Insert an entry after policy, quota, and rate checks.
    pub fn insert(&mut self, entry: ClipboardEntry) -> Result<(), HistoryError> {
        if entry.len() > self.config.max_item_bytes() {
            return Err(HistoryError::ItemTooLarge);
        }
        let token = entry.token().to_owned();
        if self.entries.contains_key(&token) {
            return Ok(());
        }
        if self.total_bytes.saturating_add(entry.len()) > self.config.max_total_bytes() {
            self.evict_until(entry.len());
        }
        if self.total_bytes.saturating_add(entry.len()) > self.config.max_total_bytes() {
            return Err(HistoryError::TotalQuotaExceeded);
        }
        self.total_bytes = self.total_bytes.saturating_add(entry.len());
        self.order.push_back(token.clone());
        self.entries.insert(token, entry);
        while self.entries.len() > self.config.max_history_entries() {
            self.evict_oldest();
        }
        Ok(())
    }

    /// Check whether a Guest may paste.
    pub fn authorize_guest(&self, guest: &str) -> Result<(), HistoryError> {
        if self.suspended.contains(guest) {
            Err(HistoryError::GuestSuspended)
        } else {
            Ok(())
        }
    }

    /// Record one guest materialization request under a sliding window.
    pub fn record_guest_request(&mut self, guest: &str, now_secs: u64) -> Result<(), HistoryError> {
        self.check_guest_request(guest, now_secs)?;
        let requests = self.guest_requests.entry(guest.to_owned()).or_default();
        while requests
            .front()
            .is_some_and(|timestamp| now_secs.saturating_sub(*timestamp) >= 60)
        {
            requests.pop_front();
        }
        if requests.len() >= self.config.max_guest_rate_per_min() as usize {
            return Err(HistoryError::RateLimitExceeded);
        }
        requests.push_back(now_secs);
        Ok(())
    }

    /// Check whether one Guest request can consume rate-limit capacity.
    pub fn check_guest_request(&self, guest: &str, now_secs: u64) -> Result<(), HistoryError> {
        self.authorize_guest(guest)?;
        let active_requests = self
            .guest_requests
            .get(guest)
            .map(|requests| {
                requests
                    .iter()
                    .filter(|timestamp| now_secs.saturating_sub(**timestamp) < 60)
                    .count()
            })
            .unwrap_or(0);
        if active_requests >= self.config.max_guest_rate_per_min() as usize {
            Err(HistoryError::RateLimitExceeded)
        } else {
            Ok(())
        }
    }

    /// Suspend one Guest.
    pub fn suspend_guest(&mut self, guest: &str) {
        self.suspended.insert(guest.to_owned());
    }

    /// Resume one Guest.
    pub fn resume_guest(&mut self, guest: &str) {
        self.suspended.remove(guest);
    }

    /// Purge all entries owned by a Guest.
    pub fn purge_guest(&mut self, guest: &str) {
        let tokens = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.guest == guest)
            .map(|(token, _)| token.clone())
            .collect::<Vec<_>>();
        for token in tokens {
            self.remove(&token);
        }
        self.guest_requests.remove(guest);
    }

    /// Remove expired entries.
    pub fn gc(&mut self, now_secs: u64) {
        let ttl = self.config.guest_entry_ttl_secs();
        let tokens = self
            .entries
            .iter()
            .filter(|(_, entry)| now_secs.saturating_sub(entry.created_at) >= ttl)
            .map(|(token, _)| token.clone())
            .collect::<Vec<_>>();
        for token in tokens {
            self.remove(&token);
        }
        for requests in self.guest_requests.values_mut() {
            while requests
                .front()
                .is_some_and(|timestamp| now_secs.saturating_sub(*timestamp) >= 60)
            {
                requests.pop_front();
            }
        }
        self.guest_requests
            .retain(|_, requests| !requests.is_empty());
    }

    /// Return the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the history is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return whether an entry is present, owned by `owner`, and within its
    /// configured retention window.
    pub fn entry_owned_and_live(&self, token: &str, owner: &str, now_secs: u64) -> bool {
        self.entries.get(token).is_some_and(|entry| {
            entry.guest == owner
                && now_secs.saturating_sub(entry.created_at) < self.config.guest_entry_ttl_secs()
        })
    }

    /// Return the expiry time for one owned, live entry.
    pub fn entry_expiry(&self, token: &str, owner: &str, now_secs: u64) -> Option<u64> {
        self.entries.get(token).and_then(|entry| {
            (entry.guest == owner
                && now_secs.saturating_sub(entry.created_at) < self.config.guest_entry_ttl_secs())
            .then_some(
                entry
                    .created_at
                    .saturating_add(self.config.guest_entry_ttl_secs()),
            )
        })
    }

    /// Return whether one owned, live entry matches an allowed MIME type.
    pub fn entry_matches_mime(
        &self,
        token: &str,
        owner: &str,
        allowed_mime_types: &[String],
        now_secs: u64,
    ) -> bool {
        self.entries.get(token).is_some_and(|entry| {
            entry.guest == owner
                && now_secs.saturating_sub(entry.created_at) < self.config.guest_entry_ttl_secs()
                && allowed_mime_types
                    .iter()
                    .any(|mime| crate::policy::normalize_mime(mime) == entry.mime())
        })
    }

    fn evict_until(&mut self, incoming: usize) {
        while self.total_bytes.saturating_add(incoming) > self.config.max_total_bytes()
            && !self.entries.is_empty()
        {
            self.evict_oldest();
        }
    }

    fn evict_oldest(&mut self) {
        if let Some(token) = self.order.pop_front() {
            self.remove(&token);
        }
    }

    fn remove(&mut self, token: &str) {
        if let Some(entry) = self.entries.remove(token) {
            self.total_bytes = self.total_bytes.saturating_sub(entry.len());
        }
        self.order.retain(|value| value != token);
    }
}

impl core::fmt::Debug for ClipboardHistory {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ClipboardHistory")
            .field("entry_count", &self.entries.len())
            .field("total_bytes", &self.total_bytes)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{ClipboardEntry, ClipboardHistory};
    use crate::ClipboardConfig;

    #[test]
    fn gc_prunes_idle_guest_rate_buckets() {
        let mut history = ClipboardHistory::new(ClipboardConfig::default()).unwrap();
        history.record_guest_request("Guest/work", 100).unwrap();
        assert_eq!(history.guest_requests.len(), 1);
        history.gc(160);
        assert!(history.guest_requests.is_empty());
    }

    #[test]
    fn history_normalizes_mime_values_before_storage_and_matching() {
        let mut history = ClipboardHistory::new(ClipboardConfig::default()).unwrap();
        let entry = ClipboardEntry::new("Guest/work", "TEXT/PLAIN", b"hello", 100).unwrap();
        let token = entry.token().to_owned();
        history.insert(entry).unwrap();
        assert!(history.entry_matches_mime(
            &token,
            "Guest/work",
            &[String::from("text/plain")],
            100,
        ));
        assert_eq!(
            history.entries.get(&token).map(|entry| entry.mime()),
            Some("text/plain")
        );
    }
}
