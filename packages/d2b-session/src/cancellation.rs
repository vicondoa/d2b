use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use d2b_contracts::{
    v2_component_session::{CancelAck, CancelRequest, CancelResult, RequestId, SessionErrorCode},
    v2_services::{StrictWireMessage, common},
};
use tokio::sync::Notify;

use crate::{Result, SessionError};

struct CancellationInner {
    cancelled: AtomicBool,
    notify: Notify,
}

#[derive(Clone)]
pub struct Cancellation {
    inner: Arc<CancellationInner>,
}

impl Cancellation {
    fn new() -> Self {
        Self {
            inner: Arc::new(CancellationInner {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }

    pub fn cancel(&self) -> bool {
        let first = !self.inner.cancelled.swap(true, Ordering::AcqRel);
        if first {
            self.inner.notify.notify_waiters();
        }
        first
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
    requests: BTreeMap<RequestId, RequestState>,
}

impl RequestRegistry {
    pub fn new(generation: u64) -> Result<Self> {
        if generation == 0 {
            return Err(SessionError::new(SessionErrorCode::GenerationMismatch));
        }
        Ok(Self {
            generation,
            requests: BTreeMap::new(),
        })
    }

    pub fn register(&mut self, request_id: RequestId) -> Result<Cancellation> {
        if self.requests.contains_key(&request_id) {
            return Err(SessionError::new(SessionErrorCode::RequestIdDuplicate));
        }
        let cancellation = Cancellation::new();
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

    pub fn cancel_generated(
        &mut self,
        request: &common::CancelRequest,
    ) -> Result<common::CancelResponse> {
        request
            .validate_wire(false)
            .map_err(|_| SessionError::new(SessionErrorCode::RecordMalformed))?;
        let ack = self.cancel(CancelRequest {
            reconnect_generation: request.session_generation,
            request_id: RequestId::new(request.request_id.clone())?,
        });
        let outcome = match ack.result {
            CancelResult::CancelledBeforeDispatch => {
                common::CancelOutcome::CANCEL_OUTCOME_CANCELLED_BEFORE_DISPATCH
            }
            CancelResult::CancellationSignalled => {
                common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
            }
            CancelResult::AlreadyTerminal => common::CancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL,
            CancelResult::UnknownRequest => common::CancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST,
            CancelResult::GenerationMismatch => {
                common::CancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH
            }
        };
        let mut response = common::CancelResponse::new();
        response.outcome = outcome.into();
        Ok(response)
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
