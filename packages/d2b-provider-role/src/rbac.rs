//! Revision-bound positive authorization decision cache.
//!
//! A cached decision is an optimization over a decision the shared evaluator
//! already made, never a decision of its own. The cache therefore holds
//! positives only, and a hit is served only when the exact authority the
//! decision was made from is still current: the same policy revisions, and -
//! through [`AcceptedRoleEvidence`] - the same accepted `Role` row, byte for
//! byte. A denial is never converted into an allow, and an entry whose
//! authority no longer matches is a miss rather than a stale allow.

use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard},
};

use d2b_contracts_resource::v3::{
    CanonicalJsonObject, ConfigurationGeneration, DesiredDigest, ResourceRef, ResourceUid,
    ZoneRevision,
    execution_policy::redacted_debug,
};

/// The exact accepted `Role` row one positive decision was made from.
///
/// The graph is the authority: a decision is made for the `Role` and
/// `RoleBinding` rows the broker had already accepted, and it is valid only
/// while those exact rows are still the ones. The evidence is the accepted
/// row's own canonical digest under the contract's domain tag, so editing one
/// rule in that row changes it, and a decision made from the earlier row
/// stops being served.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AcceptedRoleEvidence {
    role_ref: ResourceRef,
    admitted_digest: DesiredDigest,
}

impl AcceptedRoleEvidence {
    /// Derive the evidence from the exact bytes the broker accepted.
    pub fn of_admitted_row(role_ref: ResourceRef, admitted: &CanonicalJsonObject) -> Self {
        Self {
            role_ref,
            admitted_digest: DesiredDigest::of(&admitted.to_canonical_bytes()),
        }
    }

    /// The accepted `Role` row this evidence names.
    pub const fn role_ref(&self) -> &ResourceRef {
        &self.role_ref
    }

    /// The framed digest of the accepted row's bytes.
    pub const fn admitted_digest(&self) -> &DesiredDigest {
        &self.admitted_digest
    }
}

redacted_debug!(AcceptedRoleEvidence);

/// Policy revisions that make one positive decision valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PolicyRevisionSet {
    /// The policy catalog revision the decision was evaluated against.
    pub policy_revision: u64,
    /// The API catalog revision the decision was evaluated against.
    pub api_catalog_revision: u64,
    /// The active configuration revision the decision was evaluated against.
    pub active_configuration_revision: ConfigurationGeneration,
    /// The zone policy revision the decision was evaluated against.
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
    /// Construct the exact subject-and-attribute evidence key.
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

redacted_debug!(AuthorizationCacheKey);

#[derive(Clone, PartialEq, Eq)]
struct PositiveEntry {
    revisions: PolicyRevisionSet,
    expires_at_tick: u64,
    /// The accepted `Role` row the decision was made from, when the entry was
    /// recorded through the graph-bound surface.
    role: Option<AcceptedRoleEvidence>,
}

/// A bounded positive-only cache. Denial state is never converted into an allow.
pub struct PositiveDecisionCache {
    max_entries: usize,
    entries: Mutex<BTreeMap<AuthorizationCacheKey, PositiveEntry>>,
}

impl core::fmt::Debug for PositiveDecisionCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // A diagnostic must never wait on a lock: formatting can happen inside
        // a panic or a log line while another thread holds the cache, and a
        // blocking acquire would stall that thread behind the holder. Report
        // the count only when it can be read without contending.
        //
        // Poison state is derived from the same attempt rather than sampled
        // beforehand, so a lock poisoned between the two observations cannot be
        // reported as healthy alongside a count read after it was poisoned.
        let (entry_count, is_poisoned) = match self.entries.try_lock() {
            Ok(entries) => (Some(entries.len()), false),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                (Some(poisoned.into_inner().len()), true)
            }
            Err(std::sync::TryLockError::WouldBlock) => (None, self.entries.is_poisoned()),
        };

        f.debug_struct("PositiveDecisionCache")
            .field("max_entries", &self.max_entries)
            .field("entry_count", &entry_count)
            .field("is_poisoned", &is_poisoned)
            .finish()
    }
}

impl PositiveDecisionCache {
    /// Construct a bounded positive-only cache.max_entries = 0
    /// disables caching entirely.
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Whether a non-expired entry matching the exact evidence is present.
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

    /// Whether a non-expired entry recorded from the graph's current `Role`
    /// row is present.
    ///
    /// This is the surface the cutover authorizer uses. It differs from
    /// [`Self::contains`] in one respect: an entry recorded without accepted
    /// role evidence, or recorded from a different `Role` row, is a miss. A
    /// cache can therefore only ever make a decision faster while the exact
    /// authority that produced it is still the authority.
    pub fn contains_for_role(
        &self,
        key: &AuthorizationCacheKey,
        revisions: PolicyRevisionSet,
        role: &AcceptedRoleEvidence,
        now_tick: u64,
    ) -> bool {
        let mut entries = self.lock_entries();
        entries.retain(|_, entry| entry.expires_at_tick > now_tick);
        entries.get(key).is_some_and(|entry| {
            entry.revisions == revisions
                && entry
                    .role
                    .as_ref()
                    .is_some_and(|recorded| recorded == role)
        })
    }

    /// Insert one positive decision, evicting expired entries and refusing
    /// insertions past the bound. An already-expired entry is never stored.
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
                role: None,
            },
        );
    }

    /// Insert one positive decision bound to the accepted `Role` row it was
    /// made from.
    ///
    /// The entry is stored under the same bound, expiry, and positive-only
    /// rules as [`Self::insert_allow`]; the accepted role evidence is what
    /// makes a later hit conditional on that row still being current.
    pub fn insert_allow_for_role(
        &self,
        key: AuthorizationCacheKey,
        revisions: PolicyRevisionSet,
        role: AcceptedRoleEvidence,
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
                role: Some(role),
            },
        );
    }

    /// Evict every cached decision.
    pub fn clear(&self) {
        self.lock_entries().clear();
    }

    // The cache is a synchronous in-memory boundary behind a sync public
    // surface (`contains`/`insert_allow`/`clear`):
    // consumers consult it on their own threads, so a short blocking
    // acquire has no async form to convert to.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
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
        assert!(key_debug.contains("AuthorizationCacheKey(<redacted>)"));
        assert!(!key_debug.contains("subject_kind"));

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
    }

    /// The `expires_at_tick > now_tick` boundary itself: containment is
    /// false at the expiry tick and after it, not just inside the window.
    #[test]
    fn positives_expire_at_the_boundary_tick() {
        let cache = PositiveDecisionCache::new(4);
        cache.insert_allow(key(1), revisions(2), 10, 1);
        assert!(cache.contains(&key(1), revisions(2), 9), "inside the window");
        assert!(
            !cache.contains(&key(1), revisions(2), 10),
            "at the expiry tick the entry is gone"
        );
        assert!(
            !cache.contains(&key(1), revisions(2), 11),
            "past the expiry tick the entry is gone"
        );
    }

    /// The bounded ceiling: a new key is refused once the cache holds
    /// `max_entries` live entries, while the resident entries survive.
    #[test]
    fn bounded_capacity_refuses_new_keys_past_the_ceiling() {
        let cache = PositiveDecisionCache::new(2);
        cache.insert_allow(key(1), revisions(1), 100, 1);
        cache.insert_allow(key(2), revisions(1), 100, 1);
        assert!(cache.contains(&key(1), revisions(1), 50));
        assert!(cache.contains(&key(2), revisions(1), 50));

        cache.insert_allow(key(3), revisions(1), 100, 50);
        assert!(
            !cache.contains(&key(3), revisions(1), 50),
            "a new key past the ceiling is refused"
        );
        assert!(
            cache.contains(&key(1), revisions(1), 50),
            "resident keys survive the refused insertion"
        );
        assert!(cache.contains(&key(2), revisions(1), 50));
    }

    /// A zero-capacity cache stores nothing: `max_entries == 0` is a no-op
    /// admission, never an unbounded fallback.
    #[test]
    fn zero_capacity_cache_never_stores() {
        let cache = PositiveDecisionCache::new(0);
        cache.insert_allow(key(1), revisions(1), 100, 1);
        assert!(!cache.contains(&key(1), revisions(1), 50));
    }

    /// One accepted `Role` row, in the bytes the graph holds for it.
    fn admitted_role(rules: &str) -> CanonicalJsonObject {
        CanonicalJsonObject::parse(
            format!(r#"{{"operationRefs":[],"rules":[{rules}]}}"#).as_bytes(),
        )
        .expect("the admitted row is a canonical object")
    }

    fn role_ref() -> ResourceRef {
        ResourceRef::parse("Role/volume-operator").expect("canonical reference")
    }

    /// A decision recorded from the accepted `Role` row is served only while
    /// that exact row is still the one in the graph: editing one rule in the
    /// row changes its admitted digest, and the earlier decision stops being
    /// served rather than outliving the authority that produced it.
    #[test]
    fn a_graph_bound_decision_stops_being_served_when_the_accepted_role_row_changes() {
        let original = admitted_role(r#"{"resourceTypes":["Volume"]}"#);
        let narrowed = admitted_role(r#"{"resourceTypes":["VolumeBinding"]}"#);
        let cache = PositiveDecisionCache::new(4);
        let evidence = AcceptedRoleEvidence::of_admitted_row(role_ref(), &original);
        cache.insert_allow_for_role(key(1), revisions(1), evidence.clone(), 100, 1);

        assert!(cache.contains_for_role(&key(1), revisions(1), &evidence, 50));
        let rederived = AcceptedRoleEvidence::of_admitted_row(role_ref(), &narrowed);
        assert_ne!(
            rederived,
            evidence,
            "a different accepted row is not the row the decision was made from"
        );
        assert!(
            !cache.contains_for_role(&key(1), revisions(1), &rederived, 50),
            "a decision made from the earlier row is not served for the edited one"
        );
    }

    /// An entry recorded without accepted role evidence is never served
    /// through the graph-bound surface, so a pre-cutover caller cannot leave
    /// behind an entry the cutover authorizer would treat as current.
    #[test]
    fn an_entry_without_accepted_role_evidence_is_never_served_to_the_graph_bound_reader() {
        let original = admitted_role(r#"{"resourceTypes":["Volume"]}"#);
        let evidence = AcceptedRoleEvidence::of_admitted_row(role_ref(), &original);
        let cache = PositiveDecisionCache::new(4);
        cache.insert_allow(key(1), revisions(1), 100, 1);
        assert!(cache.contains(&key(1), revisions(1), 50));
        assert!(
            !cache.contains_for_role(&key(1), revisions(1), &evidence, 50),
            "the graph-bound reader demands the accepted role evidence"
        );
    }

    /// The same bound, expiry, and positive-only rules apply to the
    /// graph-bound surface: a role-bound entry past the ceiling is refused,
    /// and an already-expired one is never stored.
    #[test]
    fn the_graph_bound_surface_keeps_the_bound_and_the_expiry() {
        let original = admitted_role(r#"{"resourceTypes":["Volume"]}"#);
        let evidence = AcceptedRoleEvidence::of_admitted_row(role_ref(), &original);
        let cache = PositiveDecisionCache::new(1);
        cache.insert_allow_for_role(key(1), revisions(1), evidence.clone(), 100, 1);
        cache.insert_allow_for_role(key(2), revisions(1), evidence.clone(), 100, 50);
        assert!(
            !cache.contains_for_role(&key(2), revisions(1), &evidence, 60),
            "a new key past the ceiling is refused"
        );
        assert!(cache.contains_for_role(&key(1), revisions(1), &evidence, 99));
        assert!(!cache.contains_for_role(&key(1), revisions(1), &evidence, 100));

        let expiring = PositiveDecisionCache::new(2);
        expiring.insert_allow_for_role(key(3), revisions(1), evidence.clone(), 5, 5);
        assert!(!expiring.contains_for_role(&key(3), revisions(1), &evidence, 6));
    }
}
