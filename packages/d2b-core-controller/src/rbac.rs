//! Revision-bound positive authorization decision cache.

use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard},
};

use d2b_contracts::v3::{ConfigurationGeneration, ResourceRef, ResourceUid, ZoneRevision};

/// Policy revisions that make one positive decision valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyRevisionSet {
    pub policy_revision: u64,
    pub api_catalog_revision: u64,
    pub active_configuration_revision: ConfigurationGeneration,
    pub zone_policy_revision: ZoneRevision,
}

/// Exact subject and authorization-attribute digest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PositiveEntry {
    revisions: PolicyRevisionSet,
    expires_at_tick: u64,
}

/// A bounded positive-only cache. Denial state is never converted into an allow.
#[derive(Debug)]
pub struct PositiveDecisionCache {
    max_entries: usize,
    entries: Mutex<BTreeMap<AuthorizationCacheKey, PositiveEntry>>,
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
