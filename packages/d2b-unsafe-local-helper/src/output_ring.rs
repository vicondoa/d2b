use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub(crate) const DEFAULT_RING_BYTES: usize = 512 * 1024;
pub(crate) const MAX_RING_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_TOTAL_RING_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct RingRead {
    pub data: Vec<u8>,
    pub next_cursor: u64,
    pub eof: bool,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub timed_out: bool,
}

impl fmt::Debug for RingRead {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RingRead")
            .field("data_len", &self.data.len())
            .field("next_cursor", &self.next_cursor)
            .field("eof", &self.eof)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .field("timed_out", &self.timed_out)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RingReservationError {
    InvalidCapacity,
    Exhausted,
}

#[derive(Debug, Default)]
struct BudgetState {
    reserved: usize,
}

#[derive(Clone)]
pub(crate) struct OutputBudget {
    state: Arc<Mutex<BudgetState>>,
    limit: usize,
}

impl fmt::Debug for OutputBudget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reserved = self
            .state
            .lock()
            .map(|state| state.reserved)
            .unwrap_or(self.limit);
        formatter
            .debug_struct("OutputBudget")
            .field("limit", &self.limit)
            .field("reserved", &reserved)
            .finish()
    }
}

impl Default for OutputBudget {
    fn default() -> Self {
        Self::new(MAX_TOTAL_RING_BYTES)
    }
}

impl OutputBudget {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(BudgetState::default())),
            limit: limit.min(MAX_TOTAL_RING_BYTES),
        }
    }

    pub(crate) fn reserve(&self, capacity: usize) -> Result<RingReservation, RingReservationError> {
        if !(1..=MAX_RING_BYTES).contains(&capacity) {
            return Err(RingReservationError::InvalidCapacity);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| RingReservationError::Exhausted)?;
        let next = state
            .reserved
            .checked_add(capacity)
            .ok_or(RingReservationError::Exhausted)?;
        if next > self.limit {
            return Err(RingReservationError::Exhausted);
        }
        state.reserved = next;
        Ok(RingReservation {
            budget: Arc::clone(&self.state),
            capacity,
        })
    }

    #[cfg(test)]
    fn reserved(&self) -> usize {
        self.state.lock().unwrap().reserved
    }
}

pub(crate) struct RingReservation {
    budget: Arc<Mutex<BudgetState>>,
    capacity: usize,
}

impl RingReservation {
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }
}

impl fmt::Debug for RingReservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RingReservation")
            .field("capacity", &self.capacity)
            .finish()
    }
}

impl Drop for RingReservation {
    fn drop(&mut self) {
        if let Ok(mut state) = self.budget.lock() {
            state.reserved = state.reserved.saturating_sub(self.capacity);
        }
    }
}

struct RingState {
    bytes: VecDeque<u8>,
    start_cursor: u64,
    end_cursor: u64,
    eof: bool,
}

pub(crate) struct OutputRing {
    reservation: RingReservation,
    state: Mutex<RingState>,
    changed: Condvar,
}

impl fmt::Debug for OutputRing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutputRing")
            .field("capacity", &self.reservation.capacity())
            .finish_non_exhaustive()
    }
}

impl OutputRing {
    pub(crate) fn new(reservation: RingReservation) -> Self {
        let capacity = reservation.capacity();
        Self {
            reservation,
            state: Mutex::new(RingState {
                bytes: VecDeque::with_capacity(capacity),
                start_cursor: 0,
                end_cursor: 0,
                eof: false,
            }),
            changed: Condvar::new(),
        }
    }

    pub(crate) fn append(&self, input: &[u8]) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        for byte in input {
            if state.bytes.len() == self.reservation.capacity() {
                state.bytes.pop_front();
                state.start_cursor = state.start_cursor.saturating_add(1);
            }
            state.bytes.push_back(*byte);
            state.end_cursor = state.end_cursor.saturating_add(1);
        }
        self.changed.notify_all();
    }

    pub(crate) fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.eof = true;
            self.changed.notify_all();
        }
    }

    pub(crate) fn read(
        &self,
        cursor: u64,
        max_len: usize,
        wait: bool,
        timeout: Duration,
    ) -> RingRead {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut timed_out = false;
        loop {
            let effective_cursor = cursor.max(state.start_cursor);
            if effective_cursor < state.end_cursor || state.eof || !wait {
                break;
            }
            let now = Instant::now();
            if now >= deadline {
                timed_out = true;
                break;
            }
            let (next, result) = self
                .changed
                .wait_timeout(state, deadline.saturating_duration_since(now))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
            if result.timed_out() {
                timed_out = true;
                break;
            }
        }

        let effective_cursor = cursor.max(state.start_cursor);
        let dropped_bytes = state.start_cursor.saturating_sub(cursor);
        let available = state.end_cursor.saturating_sub(effective_cursor) as usize;
        let take = available.min(max_len);
        let skip = effective_cursor.saturating_sub(state.start_cursor) as usize;
        let data = state
            .bytes
            .iter()
            .skip(skip)
            .take(take)
            .copied()
            .collect::<Vec<_>>();
        let next_cursor = effective_cursor.saturating_add(data.len() as u64);
        RingRead {
            timed_out: timed_out && data.is_empty() && !state.eof,
            data,
            next_cursor,
            eof: state.eof && next_cursor >= state.end_cursor,
            dropped_bytes,
            truncated: dropped_bytes > 0 || available > take,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_are_bounded_and_released() {
        let budget = OutputBudget::new(10);
        let first = budget.reserve(6).unwrap();
        assert_eq!(budget.reserved(), 6);
        assert_eq!(
            budget.reserve(5).unwrap_err(),
            RingReservationError::Exhausted
        );
        drop(first);
        assert_eq!(budget.reserved(), 0);
        assert!(budget.reserve(0).is_err());
        assert!(budget.reserve(MAX_RING_BYTES + 1).is_err());
    }

    #[test]
    fn cursor_wrap_is_exact_and_debug_redacts_bytes() {
        let budget = OutputBudget::new(8);
        let ring = OutputRing::new(budget.reserve(5).unwrap());
        ring.append(b"private-terminal-canary");
        let read = ring.read(0, 16, false, Duration::ZERO);
        assert_eq!(read.data, b"anary");
        assert_eq!(read.dropped_bytes, 18);
        assert!(read.truncated);
        assert!(!format!("{ring:?}").contains("private-terminal-canary"));
        assert!(!format!("{read:?}").contains("private-terminal-canary"));
    }
}
