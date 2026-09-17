use std::sync::Arc;

use tokio::sync::Mutex;

const MAX_TYPED_SHELL_SESSION_TARGETS: usize = 256;

struct CachedTypedShellSessionTarget {
    target: String,
}

#[derive(Default)]
pub struct TypedShellSessionTargetCache {
    entries: std::collections::BTreeMap<(u32, String), CachedTypedShellSessionTarget>,
    recency: std::collections::VecDeque<(u32, String)>,
    create_reservations: std::collections::BTreeSet<(u32, String)>,
}

impl std::fmt::Debug for TypedShellSessionTargetCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TypedShellSessionTargetCache")
            .field("entry_count", &self.entries.len())
            .finish()
    }
}

impl TypedShellSessionTargetCache {
    pub fn remember(&mut self, key: (u32, String), target: String) {
        if self.entries.contains_key(&key) {
            self.entries
                .insert(key.clone(), CachedTypedShellSessionTarget { target });
            self.touch(&key);
            return;
        }
        while self.entries.len() >= MAX_TYPED_SHELL_SESSION_TARGETS {
            let oldest = self
                .recency
                .pop_front()
                .or_else(|| self.entries.keys().next().cloned());
            let Some(oldest) = oldest else {
                break;
            };
            self.entries.remove(&oldest);
        }
        self.entries
            .insert(key.clone(), CachedTypedShellSessionTarget { target });
        self.touch(&key);
    }

    pub fn cached(&mut self, key: &(u32, String)) -> Option<String> {
        let target = self.entries.get(key)?.target.clone();
        self.touch(key);
        Some(target)
    }

    pub fn forget(&mut self, key: &(u32, String)) {
        self.entries.remove(key);
        self.recency.retain(|candidate| candidate != key);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn recency_len(&self) -> usize {
        self.recency.len()
    }

    /// Reserve one create seat per (uid, name).
    ///
    /// U17 sync seat: `try_lock` fails closed when the cache mutex is
    /// momentarily held (every protected critical section is a
    /// sub-microsecond set/map operation), mirroring the fail-closed
    /// `None` the poisoned std seat produced. `blocking_lock` is unusable
    /// here: concurrent create handlers also run on tokio runtime threads,
    /// where it panics (see the `lock_sync` seat in authority_persistence).
    pub fn reserve(
        cache: &Arc<Mutex<Self>>,
        key: (u32, String),
    ) -> Option<TypedShellSessionCreateReservation> {
        let mut guard = cache.try_lock().ok()?;
        if !guard.create_reservations.insert(key.clone()) {
            return None;
        }
        Some(TypedShellSessionCreateReservation {
            cache: Arc::clone(cache),
            key,
        })
    }

    fn touch(&mut self, key: &(u32, String)) {
        self.recency.retain(|candidate| candidate != key);
        self.recency.push_back(key.clone());
    }
}

pub struct TypedShellSessionCreateReservation {
    cache: Arc<Mutex<TypedShellSessionTargetCache>>,
    key: (u32, String),
}

impl TypedShellSessionCreateReservation {
    pub fn new(cache: Arc<Mutex<TypedShellSessionTargetCache>>, key: (u32, String)) -> Self {
        Self { cache, key }
    }
}

impl Drop for TypedShellSessionCreateReservation {
    fn drop(&mut self) {
        // A dropped reservation MUST release its create seat, so unlike
        // `reserve` this cannot fail closed (a leaked seat would block
        // every future create of the same uid/name). `blocking_lock` is
        // unusable: the reservation is released from dedicated request
        // threads AND tokio runtime contexts, where blocking_lock panics.
        // This is the `lock_sync` seat from authority_persistence.rs:
        // try_lock plus a bounded spin, safe in both contexts, correct
        // because the release critical section is a single
        // sub-microsecond BTreeSet remove.
        loop {
            match self.cache.try_lock() {
                Ok(mut cache) => {
                    cache.create_reservations.remove(&self.key);
                    return;
                }
                Err(_) => std::hint::spin_loop(),
            }
        }
    }
}

pub fn new_cache() -> Arc<Mutex<TypedShellSessionTargetCache>> {
    Arc::new(Mutex::new(TypedShellSessionTargetCache::default()))
}

pub fn max_entries() -> usize {
    MAX_TYPED_SHELL_SESSION_TARGETS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_admits_one_create_seat_per_uid_name_and_drop_releases() {
        let cache = new_cache();
        let first = TypedShellSessionTargetCache::reserve(&cache, (7, "primary".to_owned()))
            .expect("first seat");
        assert!(
            TypedShellSessionTargetCache::reserve(&cache, (7, "primary".to_owned())).is_none(),
            "duplicate uid/name must conflict"
        );
        let _other_uid =
            TypedShellSessionTargetCache::reserve(&cache, (8, "primary".to_owned()))
                .expect("different uid admitted");
        let _other_name =
            TypedShellSessionTargetCache::reserve(&cache, (7, "secondary".to_owned()))
                .expect("different name admitted");
        drop(first);
        assert!(
            TypedShellSessionTargetCache::reserve(&cache, (7, "primary".to_owned())).is_some(),
            "drop must release the seat"
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn reservation_release_works_inside_a_tokio_runtime() {
        // `blocking_lock` panics on a runtime worker thread; the drop
        // release must reach the cache through the spin seat instead.
        let cache = new_cache();
        let seat = TypedShellSessionTargetCache::reserve(&cache, (7, "primary".to_owned()))
            .expect("seat");
        drop(seat);
        assert!(
            TypedShellSessionTargetCache::reserve(&cache, (7, "primary".to_owned())).is_some(),
            "release inside a runtime must not panic or leak the seat"
        );
    }

    #[test]
    fn cache_remembers_caches_and_forgets_exact_targets() {
        let cache = new_cache();
        let mut guard = cache.blocking_lock();
        assert!(guard.is_empty());
        guard.remember((7, "primary".to_owned()), "tools.host.d2b".to_owned());
        assert_eq!(
            guard.cached(&(7, "primary".to_owned())).as_deref(),
            Some("tools.host.d2b")
        );
        assert_eq!(guard.len(), 1);
        guard.forget(&(7, "primary".to_owned()));
        assert!(guard.cached(&(7, "primary".to_owned())).is_none());
        assert!(guard.is_empty());
    }
}
