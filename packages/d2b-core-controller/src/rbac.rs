//! Revision-bound positive authorization decision cache.

use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard},
};

use d2b_contracts::v3::{ConfigurationGeneration, ResourceRef, ResourceUid, ZoneRevision};

/// Policy revisions that make one positive decision valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PolicyRevisionSet {
    pub policy_revision: u64,
    pub api_catalog_revision: u64,
    pub active_configuration_revision: ConfigurationGeneration,
    pub zone_policy_revision: ZoneRevision,
}

/// Exact subject and authorization-attribute digest.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AuthorizationCacheKey {
    subject_ref: ResourceRef,
    subject_uid: ResourceUid,
    attributes_digest: [u8; 32],
}

impl AuthorizationCacheKey {
    pub const fn new(
        subject_ref: ResourceRef,
        subject_uid: ResourceUid,
        attributes_digest: [u8; 32],
    ) -> Self {
        Self {
            subject_ref,
            subject_uid,
            attributes_digest,
        }
    }
}

impl core::fmt::Debug for AuthorizationCacheKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AuthorizationCacheKey")
            .field("subject_kind", self.subject_ref.resource_type())
            .field("has_subject_uid", &true)
            .field("has_attributes_digest", &true)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PositiveEntry {
    revisions: PolicyRevisionSet,
    expires_at_tick: u64,
}

/// A bounded positive-only cache. Denial state is never converted into an allow.
pub struct PositiveDecisionCache {
    max_entries: usize,
    entries: Mutex<BTreeMap<AuthorizationCacheKey, PositiveEntry>>,
}

impl core::fmt::Debug for PositiveDecisionCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let is_poisoned = self.entries.is_poisoned();
        // A diagnostic must never wait on a lock: formatting can happen inside
        // a panic or a log line while another thread holds the cache, and a
        // blocking acquire would stall that thread behind the holder. Report
        // the count only when it can be read without contending.
        let entry_count = match self.entries.try_lock() {
            Ok(entries) => Some(entries.len()),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner().len()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };

        f.debug_struct("PositiveDecisionCache")
            .field("max_entries", &self.max_entries)
            .field("entry_count", &entry_count)
            .field("is_poisoned", &is_poisoned)
            .finish()
    }
}

impl PositiveDecisionCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn contains(
        &self,
        key: &AuthorizationCacheKey,
        revisions: PolicyRevisionSet,
        now_tick: u64,
    ) -> bool {
        let mut entries = self.lock_entries();
        entries.retain(|_, entry| entry.expires_at_tick > now_tick);
        entries
            .get(key)
            .is_some_and(|entry| entry.revisions == revisions)
    }

    pub fn insert_allow(
        &self,
        key: AuthorizationCacheKey,
        revisions: PolicyRevisionSet,
        expires_at_tick: u64,
        now_tick: u64,
    ) {
        if self.max_entries == 0 || expires_at_tick <= now_tick {
            return;
        }
        let mut entries = self.lock_entries();
        entries.retain(|_, entry| entry.expires_at_tick > now_tick);
        if entries.len() >= self.max_entries && !entries.contains_key(&key) {
            return;
        }
        entries.insert(
            key,
            PositiveEntry {
                revisions,
                expires_at_tick,
            },
        );
    }

    pub fn invalidate_revisions(&self, current: PolicyRevisionSet) {
        self.lock_entries()
            .retain(|_, entry| entry.revisions == current);
    }

    pub fn clear(&self) {
        self.lock_entries().clear();
    }

    fn lock_entries(&self) -> MutexGuard<'_, BTreeMap<AuthorizationCacheKey, PositiveEntry>> {
        self.entries.lock().unwrap_or_else(|poisoned| {
            let mut entries = poisoned.into_inner();
            entries.clear();
            entries
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> AuthorizationCacheKey {
        AuthorizationCacheKey::new(
            ResourceRef::parse("Provider/system-core").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            [byte; 32],
        )
    }

    fn revisions(policy_revision: u64) -> PolicyRevisionSet {
        PolicyRevisionSet {
            policy_revision,
            api_catalog_revision: 3,
            active_configuration_revision: ConfigurationGeneration::new(5).unwrap(),
            zone_policy_revision: ZoneRevision::new(7),
        }
    }

    #[test]
    fn authorization_cache_debug_redacts_every_protected_field() {
        const SUBJECT_NAME_SENTINEL: &str = "rbac-debug-sentinel";
        const SUBJECT_UID_SENTINEL: &str = "deadbeef-dead-4bad-8bad-deadbeef0001";
        const DIGEST_BYTE_SENTINEL: u8 = 197;
        const DIGEST_DEBUG_SENTINEL: &str = "197";

        let subject_ref = ResourceRef::parse(&format!("Provider/{SUBJECT_NAME_SENTINEL}")).unwrap();
        let subject_uid = ResourceUid::parse(SUBJECT_UID_SENTINEL).unwrap();
        // The contracts crate redacts `ResourceRef`'s own diagnostics, so assert
        // the sentinel is carried through an explicit accessor rather than
        // through formatting, which would make this precondition vacuous.
        assert!(
            subject_ref
                .to_canonical_string()
                .contains(SUBJECT_NAME_SENTINEL)
        );
        assert_eq!(subject_uid.as_str(), SUBJECT_UID_SENTINEL);

        let key = AuthorizationCacheKey::new(subject_ref, subject_uid, [DIGEST_BYTE_SENTINEL; 32]);
        let key_debug = format!("{key:?}");
        for marker in [
            SUBJECT_NAME_SENTINEL,
            SUBJECT_UID_SENTINEL,
            DIGEST_DEBUG_SENTINEL,
        ] {
            assert!(!key_debug.contains(marker), "{key_debug}");
        }
        assert!(key_debug.contains("subject_kind"));
        assert!(key_debug.contains("has_subject_uid: true"));
        assert!(key_debug.contains("has_attributes_digest: true"));

        let cache = PositiveDecisionCache::new(2);
        cache.insert_allow(key, revisions(11), 23, 1);
        let cache_debug = format!("{cache:?}");
        assert_eq!(
            cache_debug,
            "PositiveDecisionCache { max_entries: 2, entry_count: Some(1), is_poisoned: false }"
        );
    }

    #[test]
    fn positives_expire_and_revision_changes_invalidate_immediately() {
        let cache = PositiveDecisionCache::new(4);
        cache.insert_allow(key(1), revisions(2), 10, 1);
        assert!(cache.contains(&key(1), revisions(2), 9));
        assert!(!cache.contains(&key(1), revisions(3), 9));
        cache.invalidate_revisions(revisions(3));
        assert!(!cache.contains(&key(1), revisions(2), 9));
        assert!(!cache.contains(&key(1), revisions(2), 10));
    }
}
