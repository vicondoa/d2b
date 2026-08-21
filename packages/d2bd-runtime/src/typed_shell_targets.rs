use std::sync::{Arc, Mutex};

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

    pub fn recency_len(&self) -> usize {
        self.recency.len()
    }

    pub fn reserve(
        cache: &Arc<Mutex<Self>>,
        key: (u32, String),
    ) -> Option<TypedShellSessionCreateReservation> {
        let mut guard = cache.lock().ok()?;
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
        if let Ok(mut cache) = self.cache.lock() {
            cache.create_reservations.remove(&self.key);
        }
    }
}

pub fn new_cache() -> Arc<Mutex<TypedShellSessionTargetCache>> {
    Arc::new(Mutex::new(TypedShellSessionTargetCache::default()))
}

pub fn max_entries() -> usize {
    MAX_TYPED_SHELL_SESSION_TARGETS
}
