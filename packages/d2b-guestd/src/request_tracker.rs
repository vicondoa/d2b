//! Guest request admission, replay, and cancellation state.

use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    sync::atomic::{AtomicU8, Ordering},
    sync::{Arc, Mutex},
    time::Duration,
};

use d2b_contracts::{
    v2_component_session::{MAX_REQUEST_LIFETIME_MS, RequestId},
    v2_services::{admit_metadata, common},
};
use d2b_session::{Cancellation, ComponentSessionDriver};
use tokio::sync::Notify;

use crate::service_v2::GuestSessionError;

const TERMINAL_REQUESTS: usize = 256;
const REPLAY_RESULTS: usize = 256;
const REQUEST_CANCELLABLE: u8 = 0;
const REQUEST_COMMITTING: u8 = 1;
const REQUEST_CANCELLED: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestAdmissionError {
    Deadline,
    Duplicate,
    ReplayConflict,
    Cancelled,
    Session,
}

struct ReplayFlight {
    result: Mutex<FlightResult>,
    notify: Notify,
}

#[derive(Default)]
struct FlightResult {
    response: Option<Vec<u8>>,
    failed: bool,
}

enum ReplayState {
    InFlight(Arc<ReplayFlight>),
    Complete(Vec<u8>),
}

struct ReplayEntry {
    method: &'static str,
    request_digest: Vec<u8>,
    state: ReplayState,
}

struct ActiveRequest {
    cancellation: Cancellation,
    commit_state: Arc<AtomicU8>,
}

#[derive(Default)]
struct TrackerState {
    active: BTreeMap<Vec<u8>, ActiveRequest>,
    terminal: VecDeque<Vec<u8>>,
    replays: BTreeMap<Vec<u8>, ReplayEntry>,
    replay_order: VecDeque<Vec<u8>>,
}

pub struct GuestRequestTracker {
    generation: u64,
    session: Arc<dyn ComponentSessionDriver>,
    state: Mutex<TrackerState>,
}

impl fmt::Debug for GuestRequestTracker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        formatter
            .debug_struct("GuestRequestTracker")
            .field("generation", &"<redacted>")
            .field("active", &state.active.len())
            .field("terminal", &state.terminal.len())
            .field("replays", &state.replays.len())
            .finish()
    }
}

#[derive(Clone)]
pub struct GuestRequestTicket {
    request_id: Vec<u8>,
    idempotency_key: Option<Vec<u8>>,
    flight: Option<Arc<ReplayFlight>>,
    pub cancellation: Cancellation,
    commit_state: Arc<AtomicU8>,
}

impl fmt::Debug for GuestRequestTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestRequestTicket")
            .field("request_id", &"<redacted>")
            .field("idempotency", &self.idempotency_key.is_some())
            .field("cancellation", &self.cancellation)
            .field(
                "non_cancellable",
                &(self.commit_state.load(Ordering::Acquire) == REQUEST_COMMITTING),
            )
            .finish()
    }
}

impl GuestRequestTicket {
    /// Point of no return for synchronous authorization mutation. Cancellation
    /// and this transition race on one atomic state; exactly one can win.
    pub fn begin_non_cancellable(&self) -> bool {
        if self.cancellation.is_cancelled() {
            return false;
        }
        if self
            .commit_state
            .compare_exchange(
                REQUEST_CANCELLABLE,
                REQUEST_COMMITTING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false;
        }
        if self.cancellation.is_cancelled() {
            let _ = self.commit_state.compare_exchange(
                REQUEST_COMMITTING,
                REQUEST_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            false
        } else {
            true
        }
    }
}

pub enum GuestRequestAdmission {
    New(GuestRequestTicket),
    Replay(Vec<u8>),
}

enum ReplayDecision {
    New(Option<Arc<ReplayFlight>>),
    Wait(Arc<ReplayFlight>),
    Complete(Vec<u8>),
}

impl GuestRequestTracker {
    pub fn new(
        generation: u64,
        session: Arc<dyn ComponentSessionDriver>,
    ) -> Result<Self, GuestSessionError> {
        if generation == 0 || generation != session.generation() {
            return Err(GuestSessionError::Session);
        }
        Ok(Self {
            generation,
            session,
            state: Mutex::new(TrackerState::default()),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn admit(
        &self,
        metadata: &common::RequestMetadata,
        method: &'static str,
        request_digest: &[u8],
        requires_idempotency: bool,
        now_unix_ms: u64,
        peer_timeout_nanos: Option<u64>,
    ) -> Result<GuestRequestAdmission, RequestAdmissionError> {
        let remaining_nanos = admit_metadata(
            metadata,
            requires_idempotency,
            now_unix_ms,
            MAX_REQUEST_LIFETIME_MS,
            None,
            peer_timeout_nanos,
        )
        .map_err(|_| RequestAdmissionError::Deadline)?;
        if metadata.session_generation != self.generation {
            return Err(RequestAdmissionError::Session);
        }
        let request_id = RequestId::new(metadata.request_id.clone())
            .map_err(|_| RequestAdmissionError::Session)?;
        let idempotency_key =
            (!metadata.idempotency_key.is_empty()).then(|| metadata.idempotency_key.clone());

        let decision = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.active.contains_key(&metadata.request_id) {
                return Err(RequestAdmissionError::Duplicate);
            }
            match idempotency_key.as_ref() {
                Some(key) => match state.replays.get(key) {
                    Some(entry)
                        if entry.method != method
                            || entry.request_digest.as_slice() != request_digest =>
                    {
                        return Err(RequestAdmissionError::ReplayConflict);
                    }
                    Some(ReplayEntry {
                        state: ReplayState::Complete(response),
                        ..
                    }) => ReplayDecision::Complete(response.clone()),
                    Some(ReplayEntry {
                        state: ReplayState::InFlight(flight),
                        ..
                    }) => ReplayDecision::Wait(Arc::clone(flight)),
                    None => {
                        if state.terminal.contains(&metadata.request_id) {
                            return Err(RequestAdmissionError::Duplicate);
                        }
                        let flight = Arc::new(ReplayFlight {
                            result: Mutex::new(FlightResult::default()),
                            notify: Notify::new(),
                        });
                        state.replays.insert(
                            key.clone(),
                            ReplayEntry {
                                method,
                                request_digest: request_digest.to_vec(),
                                state: ReplayState::InFlight(Arc::clone(&flight)),
                            },
                        );
                        state.replay_order.push_back(key.clone());
                        ReplayDecision::New(Some(flight))
                    }
                },
                None => {
                    if state.terminal.contains(&metadata.request_id) {
                        return Err(RequestAdmissionError::Duplicate);
                    }
                    ReplayDecision::New(None)
                }
            }
        };
        match decision {
            ReplayDecision::Complete(response) => Ok(GuestRequestAdmission::Replay(response)),
            ReplayDecision::Wait(flight) => {
                let wait = async {
                    loop {
                        let notified = flight.notify.notified();
                        {
                            let result = flight
                                .result
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            if let Some(response) = result.response.as_ref() {
                                return Ok(response.clone());
                            }
                            if result.failed {
                                return Err(RequestAdmissionError::Cancelled);
                            }
                        }
                        notified.await;
                    }
                };
                return tokio::time::timeout(Duration::from_nanos(remaining_nanos), wait)
                    .await
                    .map_err(|_| RequestAdmissionError::Deadline)?
                    .map(GuestRequestAdmission::Replay);
            }
            ReplayDecision::New(flight) => {
                let cancellation = match self.session.register_inbound_call(request_id).await {
                    Ok(cancellation) => cancellation,
                    Err(_) => {
                        self.abort_replay(idempotency_key.as_deref(), flight.as_ref());
                        return Err(RequestAdmissionError::Session);
                    }
                };
                let commit_state = Arc::new(AtomicU8::new(REQUEST_CANCELLABLE));
                let duplicate = {
                    let mut state = self
                        .state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    state
                        .active
                        .insert(
                            metadata.request_id.clone(),
                            ActiveRequest {
                                cancellation: cancellation.clone(),
                                commit_state: Arc::clone(&commit_state),
                            },
                        )
                        .is_some()
                };
                if duplicate {
                    let _ = self
                        .session
                        .remove_inbound_call(
                            RequestId::new(metadata.request_id.clone())
                                .map_err(|_| RequestAdmissionError::Session)?,
                        )
                        .await;
                    self.abort_replay(idempotency_key.as_deref(), flight.as_ref());
                    return Err(RequestAdmissionError::Duplicate);
                }
                Ok(GuestRequestAdmission::New(GuestRequestTicket {
                    request_id: metadata.request_id.clone(),
                    idempotency_key,
                    flight,
                    cancellation,
                    commit_state,
                }))
            }
        }
    }

    pub async fn complete_response(
        &self,
        ticket: &GuestRequestTicket,
        response: Vec<u8>,
        keep_active: bool,
    ) {
        if let Some(key) = ticket.idempotency_key.as_ref() {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(entry) = state.replays.get_mut(key) {
                entry.state = ReplayState::Complete(response.clone());
            }
            self.trim_replays(&mut state);
        }
        if let Some(flight) = ticket.flight.as_ref() {
            flight
                .result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .response = Some(response);
            flight.notify.notify_waiters();
        }
        if !keep_active {
            self.finish(ticket).await;
        }
    }

    pub async fn fail(&self, ticket: &GuestRequestTicket) {
        self.abort_replay(ticket.idempotency_key.as_deref(), ticket.flight.as_ref());
        self.finish(ticket).await;
    }

    pub async fn finish(&self, ticket: &GuestRequestTicket) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.active.remove(&ticket.request_id);
            if !state.terminal.contains(&ticket.request_id) {
                if state.terminal.len() == TERMINAL_REQUESTS {
                    state.terminal.pop_front();
                }
                state.terminal.push_back(ticket.request_id.clone());
            }
        }
        if let Ok(request_id) = RequestId::new(ticket.request_id.clone()) {
            let _ = self.session.complete_inbound_call(request_id).await;
        }
    }

    pub async fn cancel(&self, request: &common::CancelRequest) -> common::CancelResponse {
        let outcome = if request.session_generation != self.generation {
            common::CancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH
        } else {
            let active = {
                let state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.active.get(&request.request_id).map(|active| {
                    (
                        active.cancellation.clone(),
                        Arc::clone(&active.commit_state),
                    )
                })
            };
            match active {
                Some((cancellation, commit_state))
                    if commit_state
                        .compare_exchange(
                            REQUEST_CANCELLABLE,
                            REQUEST_CANCELLED,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok() =>
                {
                    cancellation.cancel();
                    if let Ok(request_id) = RequestId::new(request.request_id.clone()) {
                        let _ = self.session.remove_inbound_call(request_id).await;
                    }
                    common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
                }
                Some(_) => common::CancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL,
                None => {
                    let state = self
                        .state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if state.terminal.contains(&request.request_id) {
                        common::CancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL
                    } else {
                        common::CancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST
                    }
                }
            }
        };
        common::CancelResponse {
            outcome: outcome.into(),
            ..Default::default()
        }
    }

    fn abort_replay(&self, key: Option<&[u8]>, flight: Option<&Arc<ReplayFlight>>) {
        if let Some(flight) = flight {
            flight
                .result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .failed = true;
            flight.notify.notify_waiters();
        }
        if let Some(key) = key {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.replays.remove(key);
            state.replay_order.retain(|candidate| candidate != key);
        }
    }

    fn trim_replays(&self, state: &mut TrackerState) {
        while state.replay_order.len() > REPLAY_RESULTS {
            if let Some(key) = state.replay_order.pop_front() {
                state.replays.remove(&key);
            }
        }
    }
}
