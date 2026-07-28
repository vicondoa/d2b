use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use d2b_contracts::v3::component_session::{
    CancelAck, CancelRequest, CancelResult, RequestId, SessionErrorCode,
};
use tokio::sync::Notify;

use crate::{Result, SessionError};

struct CancellationInner {
    cancelled: AtomicBool,
    notify: Notify,
    admission: Mutex<AdmissionState>,
    drained: Notify,
}

#[derive(Default)]
struct AdmissionState {
    revoked: bool,
    ordered_revocation: bool,
    active: usize,
}

#[derive(Clone)]
pub struct Cancellation {
    inner: Arc<CancellationInner>,
}

impl Cancellation {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(CancellationInner {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
                admission: Mutex::new(AdmissionState::default()),
                drained: Notify::new(),
            }),
        }
    }

    pub fn cancel(&self) -> bool {
        self.cancel_with_order(false)
    }

    fn cancel_with_order(&self, ordered_revocation: bool) -> bool {
        let first = {
            let mut admission = self
                .inner
                .admission
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            admission.revoked = true;
            let first = !self.inner.cancelled.swap(true, Ordering::AcqRel);
            if first {
                admission.ordered_revocation = ordered_revocation;
            }
            first
        };
        if first {
            self.inner.notify.notify_waiters();
        }
        first
    }

    /// Revoke future write admissions and wait for the writer to acknowledge
    /// every admission that preceded revocation.
    pub fn cancel_and_wait(&self) -> impl Future<Output = bool> + Send + 'static {
        let first = self.cancel_with_order(true);
        let cancellation = self.clone();
        async move {
            loop {
                let drained = cancellation.inner.drained.notified();
                tokio::pin!(drained);
                drained.as_mut().enable();
                if cancellation
                    .inner
                    .admission
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .active
                    == 0
                {
                    return first;
                }
                drained.as_mut().await;
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub async fn cancelled(&self) {
        loop {
            let notified = self.inner.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn admit_write(&self) -> Option<WriteAdmission> {
        let mut admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if admission.revoked {
            return None;
        }
        admission.active = admission
            .active
            .checked_add(1)
            .expect("bounded writer admission count cannot overflow");
        Some(WriteAdmission {
            inner: Arc::clone(&self.inner),
        })
    }

    pub(crate) fn preserves_admitted_write(&self) -> bool {
        self.inner
            .admission
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .ordered_revocation
    }
}

pub(crate) struct WriteAdmission {
    inner: Arc<CancellationInner>,
}

impl Drop for WriteAdmission {
    fn drop(&mut self) {
        let drained = {
            let mut admission = self
                .inner
                .admission
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            admission.active -= 1;
            admission.active == 0 && admission.revoked
        };
        if drained {
            self.inner.drained.notify_waiters();
        }
    }
}

impl fmt::Debug for Cancellation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cancellation")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

struct RequestState {
    cancellation: Cancellation,
    dispatched: bool,
}

pub struct RequestRegistry {
    generation: u64,
    max_active: usize,
    requests: BTreeMap<RequestId, RequestState>,
}

impl RequestRegistry {
    pub fn new(generation: u64) -> Result<Self> {
        Self::with_limit(generation, 256)
    }

    pub fn with_limit(generation: u64, max_active: usize) -> Result<Self> {
        if generation == 0 {
            return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
        }
        if max_active == 0 {
            return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
        }
        Ok(Self {
            generation,
            max_active,
            requests: BTreeMap::new(),
        })
    }

    pub fn register(&mut self, request_id: RequestId) -> Result<Cancellation> {
        self.register_with_cancellation(request_id, Cancellation::new())
    }

    pub(crate) fn register_with_cancellation(
        &mut self,
        request_id: RequestId,
        cancellation: Cancellation,
    ) -> Result<Cancellation> {
        if self.requests.contains_key(&request_id) {
            return Err(SessionError::new(SessionErrorCode::RequestIdDuplicate));
        }
        if self.requests.len() >= self.max_active {
            return Err(SessionError::new(SessionErrorCode::QueueBackpressure));
        }
        self.requests.insert(
            request_id,
            RequestState {
                cancellation: cancellation.clone(),
                dispatched: false,
            },
        );
        Ok(cancellation)
    }

    pub fn mark_dispatched(&mut self, request_id: &RequestId) -> Result<()> {
        let state = self
            .requests
            .get_mut(request_id)
            .ok_or_else(|| SessionError::new(SessionErrorCode::Cancelled))?;
        if state.cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled));
        }
        state.dispatched = true;
        Ok(())
    }

    pub fn cancel(&mut self, request: CancelRequest) -> CancelAck {
        if request.reconnect_generation != self.generation {
            return request.acknowledge(self.generation, CancelResult::GenerationMismatch);
        }
        let result = match self.requests.get(&request.request_id) {
            None => CancelResult::UnknownRequest,
            Some(state) if state.cancellation.is_cancelled() => CancelResult::AlreadyTerminal,
            Some(state) => {
                state.cancellation.cancel();
                if state.dispatched {
                    CancelResult::CancellationSignalled
                } else {
                    CancelResult::CancelledBeforeDispatch
                }
            }
        };
        request.acknowledge(self.generation, result)
    }

    pub fn complete(&mut self, request_id: &RequestId) -> bool {
        self.requests.remove(request_id).is_some()
    }

    pub fn remove(&mut self, request_id: &RequestId) -> bool {
        let Some(state) = self.requests.remove(request_id) else {
            return false;
        };
        state.cancellation.cancel();
        true
    }

    pub fn signal(&self, request_id: &RequestId) -> bool {
        self.requests
            .get(request_id)
            .is_some_and(|state| state.cancellation.cancel())
    }

    pub fn cancel_all(&mut self) {
        for state in self.requests.values() {
            state.cancellation.cancel();
        }
        self.requests.clear();
    }

    pub fn active(&self) -> usize {
        self.requests.len()
    }
}

impl fmt::Debug for RequestRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestRegistry")
            .field("generation", &"<redacted>")
            .field("active", &self.requests.len())
            .field("request_ids", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(value: u8) -> RequestId {
        RequestId::new(vec![value; 16]).unwrap()
    }

    #[test]
    fn active_request_limit_backpressures_and_terminal_removal_releases_capacity() {
        let mut registry = RequestRegistry::with_limit(1, 2).unwrap();
        registry.register(request(1)).unwrap();
        registry.register(request(2)).unwrap();
        assert_eq!(
            registry.register(request(3)).unwrap_err().code(),
            SessionErrorCode::QueueBackpressure
        );
        assert!(registry.complete(&request(1)));
        registry.register(request(3)).unwrap();
        registry.cancel_all();
        assert_eq!(registry.active(), 0);
        registry.register(request(4)).unwrap();
    }
}
