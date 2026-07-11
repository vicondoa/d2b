use std::collections::VecDeque;
use std::fmt;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct RingRead {
    pub data: Vec<u8>,
    pub next_cursor: u64,
    pub eof: bool,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub timed_out: bool,
}

struct RingState {
    bytes: VecDeque<u8>,
    start_cursor: u64,
    end_cursor: u64,
    eof: bool,
}

pub(crate) struct OutputRing {
    capacity: usize,
    state: Mutex<RingState>,
    changed: Condvar,
}

impl fmt::Debug for RingRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RingRead")
            .field("data_len", &self.data.len())
            .field("next_cursor", &self.next_cursor)
            .field("eof", &self.eof)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .field("timed_out", &self.timed_out)
            .finish()
    }
}

impl fmt::Debug for OutputRing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutputRing")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl OutputRing {
    pub(crate) fn new(capacity: usize) -> Option<Self> {
        (capacity > 0).then(|| Self {
            capacity,
            state: Mutex::new(RingState {
                bytes: VecDeque::with_capacity(capacity),
                start_cursor: 0,
                end_cursor: 0,
                eof: false,
            }),
            changed: Condvar::new(),
        })
    }

    pub(crate) fn append(&self, input: &[u8]) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        for byte in input {
            if state.bytes.len() == self.capacity {
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
            let remaining = deadline.saturating_duration_since(now);
            let (next, result) = self
                .changed
                .wait_timeout(state, remaining)
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
        let data_is_empty = data.is_empty();
        let next_cursor = effective_cursor.saturating_add(data.len() as u64);
        RingRead {
            data,
            next_cursor,
            eof: state.eof && next_cursor >= state.end_cursor,
            dropped_bytes,
            truncated: dropped_bytes > 0 || available > take,
            timed_out: timed_out && data_is_empty && !state.eof,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn cursor_reads_wrap_and_report_exact_gap() {
        let ring = OutputRing::new(5).unwrap();
        ring.append(b"abc");
        assert_eq!(
            ring.read(0, 2, false, Duration::ZERO),
            RingRead {
                data: b"ab".to_vec(),
                next_cursor: 2,
                eof: false,
                dropped_bytes: 0,
                truncated: true,
                timed_out: false,
            }
        );

        ring.append(b"defgh");
        let wrapped = ring.read(0, 16, false, Duration::ZERO);
        assert_eq!(wrapped.data, b"defgh");
        assert_eq!(wrapped.next_cursor, 8);
        assert_eq!(wrapped.dropped_bytes, 3);
        assert!(wrapped.truncated);
    }

    #[test]
    fn wait_wakes_for_output_and_eof() {
        let ring = Arc::new(OutputRing::new(32).unwrap());
        let producer = Arc::clone(&ring);
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            producer.append(b"ready");
            producer.close();
        });
        let read = ring.read(0, 32, true, Duration::from_secs(1));
        writer.join().unwrap();
        assert_eq!(read.data, b"ready");
        assert!(read.eof);
        assert!(!read.timed_out);
    }

    #[test]
    fn wait_timeout_is_bounded_and_empty() {
        let ring = OutputRing::new(8).unwrap();
        let started = Instant::now();
        let read = ring.read(0, 8, true, Duration::from_millis(10));
        assert!(read.timed_out);
        assert!(read.data.is_empty());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn debug_never_exposes_ring_bytes() {
        let canary = b"private-terminal-canary";
        let ring = OutputRing::new(64).unwrap();
        ring.append(canary);
        let read = ring.read(0, 64, false, Duration::ZERO);
        assert!(!format!("{ring:?}").contains("private-terminal-canary"));
        assert!(!format!("{read:?}").contains("private-terminal-canary"));
    }
}
