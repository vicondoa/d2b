#![forbid(unsafe_code)]

//! Executable conformance model for nixling's selected Kata-style
//! chunked stdio guest-control protocol.
//!
//! The crate intentionally uses only `std`: it is a protocol proof, not
//! a production implementation. The tests exercise byte offsets,
//! idempotent stdin writes, slow-consumer caps, stale session detection,
//! terminal control ordering, and attached-session fairness thresholds.

use std::collections::HashSet;

pub const KIB: usize = 1024;
pub const MIB: usize = 1024 * KIB;
pub const DEFAULT_CHUNK: usize = 256 * KIB;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct SessionToken {
    pub session_id: u64,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    StaleSession,
    ChunkTooLarge,
    OffsetGap { expected: u64, got: u64 },
    OffsetBehind { expected: u64, got: u64 },
    OutputEvicted { earliest: u64, got: u64 },
    StdinClosed,
    SlowConsumer { retained: usize, cap: usize },
    DuplicateWriteId,
    ControlSequenceGap { expected: u64, got: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteDisposition {
    Accepted,
    Duplicate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriteStdinResponse {
    pub next_offset: u64,
    pub disposition: WriteDisposition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadOutputResponse {
    pub offset: u64,
    pub data: Vec<u8>,
    pub eof: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowSize {
    pub rows: u16,
    pub cols: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signal {
    Int,
    Term,
    Kill,
}

impl Signal {
    pub fn number(self) -> i32 {
        match self {
            Signal::Int => 2,
            Signal::Term => 15,
            Signal::Kill => 9,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitStatus {
    Code(i32),
    Signal { signal: Signal, status_code: i32 },
}

impl ExitStatus {
    pub fn from_signal(signal: Signal) -> Self {
        Self::Signal {
            signal,
            status_code: 128 + signal.number(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlEvent {
    Resize(WindowSize),
    Signal(Signal),
    Exit(ExitStatus),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrderedControlEvent {
    pub seq: u64,
    pub event: ControlEvent,
}

#[derive(Clone, Debug)]
struct OutputLog {
    base_offset: u64,
    bytes: Vec<u8>,
    eof: bool,
}

impl OutputLog {
    fn new() -> Self {
        Self {
            base_offset: 0,
            bytes: Vec::new(),
            eof: false,
        }
    }

    fn next_offset(&self) -> u64 {
        self.base_offset + self.bytes.len() as u64
    }

    fn append(&mut self, data: &[u8]) {
        self.bytes.extend_from_slice(data);
    }

    fn read(&self, offset: u64, max_len: usize) -> Result<ReadOutputResponse, ProtocolError> {
        if offset < self.base_offset {
            return Err(ProtocolError::OutputEvicted {
                earliest: self.base_offset,
                got: offset,
            });
        }
        if offset > self.next_offset() {
            return Err(ProtocolError::OffsetGap {
                expected: self.next_offset(),
                got: offset,
            });
        }
        let start = (offset - self.base_offset) as usize;
        let end = start.saturating_add(max_len).min(self.bytes.len());
        Ok(ReadOutputResponse {
            offset,
            data: self.bytes[start..end].to_vec(),
            eof: self.eof && end == self.bytes.len(),
        })
    }

    fn evict_through(&mut self, offset: u64) {
        let clamped = offset.clamp(self.base_offset, self.next_offset());
        let drain = (clamped - self.base_offset) as usize;
        self.bytes.drain(0..drain);
        self.base_offset = clamped;
    }
}

#[derive(Clone, Debug)]
pub struct ChunkedSession {
    token: SessionToken,
    max_chunk: usize,
    output_cap: usize,
    stdin_cap: usize,
    stdout: OutputLog,
    stderr: OutputLog,
    stdin_history: Vec<u8>,
    stdin_delivered_offset: u64,
    stdin_closed: bool,
    close_stdin_offset: Option<u64>,
    write_ids: HashSet<u64>,
    next_control_seq: u64,
    events: Vec<OrderedControlEvent>,
}

impl ChunkedSession {
    pub fn new(session_id: u64, output_cap: usize, stdin_cap: usize, max_chunk: usize) -> Self {
        Self {
            token: SessionToken {
                session_id,
                generation: 0,
            },
            max_chunk,
            output_cap,
            stdin_cap,
            stdout: OutputLog::new(),
            stderr: OutputLog::new(),
            stdin_history: Vec::new(),
            stdin_delivered_offset: 0,
            stdin_closed: false,
            close_stdin_offset: None,
            write_ids: HashSet::new(),
            next_control_seq: 0,
            events: Vec::new(),
        }
    }

    pub fn token(&self) -> SessionToken {
        self.token
    }

    pub fn restart(&mut self) -> SessionToken {
        self.token.generation += 1;
        self.token
    }

    pub fn retained_output_bytes(&self) -> usize {
        self.stdout.bytes.len() + self.stderr.bytes.len()
    }

    pub fn stdin_next_offset(&self) -> u64 {
        self.stdin_history.len() as u64
    }

    pub fn stdin_buffered_bytes(&self) -> usize {
        self.stdin_history.len() - self.stdin_delivered_offset as usize
    }

    pub fn append_output(
        &mut self,
        token: SessionToken,
        stream: Stream,
        data: &[u8],
    ) -> Result<u64, ProtocolError> {
        self.check_token(token)?;
        if data.len() > self.max_chunk {
            return Err(ProtocolError::ChunkTooLarge);
        }
        if self.retained_output_bytes() + data.len() > self.output_cap {
            return Err(ProtocolError::SlowConsumer {
                retained: self.retained_output_bytes(),
                cap: self.output_cap,
            });
        }
        let log = self.log_mut(stream);
        let offset = log.next_offset();
        log.append(data);
        Ok(offset)
    }

    pub fn finish_output(
        &mut self,
        token: SessionToken,
        stream: Stream,
    ) -> Result<(), ProtocolError> {
        self.check_token(token)?;
        self.log_mut(stream).eof = true;
        Ok(())
    }

    pub fn read_output(
        &self,
        token: SessionToken,
        stream: Stream,
        offset: u64,
        max_len: usize,
    ) -> Result<ReadOutputResponse, ProtocolError> {
        self.check_token(token)?;
        if max_len > self.max_chunk {
            return Err(ProtocolError::ChunkTooLarge);
        }
        self.log(stream).read(offset, max_len)
    }

    pub fn read_stdout(
        &self,
        token: SessionToken,
        offset: u64,
        max_len: usize,
    ) -> Result<ReadOutputResponse, ProtocolError> {
        self.read_output(token, Stream::Stdout, offset, max_len)
    }

    pub fn read_stderr(
        &self,
        token: SessionToken,
        offset: u64,
        max_len: usize,
    ) -> Result<ReadOutputResponse, ProtocolError> {
        self.read_output(token, Stream::Stderr, offset, max_len)
    }

    pub fn ack_output(
        &mut self,
        token: SessionToken,
        stream: Stream,
        through_offset: u64,
    ) -> Result<(), ProtocolError> {
        self.check_token(token)?;
        self.log_mut(stream).evict_through(through_offset);
        Ok(())
    }

    pub fn write_stdin(
        &mut self,
        token: SessionToken,
        write_id: u64,
        offset: u64,
        data: &[u8],
    ) -> Result<WriteStdinResponse, ProtocolError> {
        self.check_token(token)?;
        if self.stdin_closed {
            return Err(ProtocolError::StdinClosed);
        }
        if data.len() > self.max_chunk {
            return Err(ProtocolError::ChunkTooLarge);
        }
        let expected = self.stdin_next_offset();
        if offset < expected {
            let end = offset as usize + data.len();
            if end <= self.stdin_history.len() && self.stdin_history[offset as usize..end] == *data
            {
                return Ok(WriteStdinResponse {
                    next_offset: expected,
                    disposition: WriteDisposition::Duplicate,
                });
            }
            return Err(ProtocolError::OffsetBehind {
                expected,
                got: offset,
            });
        }
        if offset > expected {
            return Err(ProtocolError::OffsetGap {
                expected,
                got: offset,
            });
        }
        if self.write_ids.contains(&write_id) {
            return Err(ProtocolError::DuplicateWriteId);
        }
        if self.stdin_buffered_bytes() + data.len() > self.stdin_cap {
            return Err(ProtocolError::SlowConsumer {
                retained: self.stdin_buffered_bytes(),
                cap: self.stdin_cap,
            });
        }
        self.write_ids.insert(write_id);
        self.stdin_history.extend_from_slice(data);
        Ok(WriteStdinResponse {
            next_offset: self.stdin_next_offset(),
            disposition: WriteDisposition::Accepted,
        })
    }

    pub fn close_stdin(
        &mut self,
        token: SessionToken,
        offset: u64,
    ) -> Result<WriteStdinResponse, ProtocolError> {
        self.check_token(token)?;
        let expected = self.stdin_next_offset();
        if self.stdin_closed {
            if self.close_stdin_offset == Some(offset) {
                return Ok(WriteStdinResponse {
                    next_offset: expected,
                    disposition: WriteDisposition::Duplicate,
                });
            }
            return Err(ProtocolError::StdinClosed);
        }
        if offset != expected {
            return Err(ProtocolError::OffsetGap {
                expected,
                got: offset,
            });
        }
        self.stdin_closed = true;
        self.close_stdin_offset = Some(offset);
        Ok(WriteStdinResponse {
            next_offset: expected,
            disposition: WriteDisposition::Accepted,
        })
    }

    pub fn consume_stdin(
        &mut self,
        token: SessionToken,
        max_len: usize,
    ) -> Result<Vec<u8>, ProtocolError> {
        self.check_token(token)?;
        let start = self.stdin_delivered_offset as usize;
        let end = start.saturating_add(max_len).min(self.stdin_history.len());
        self.stdin_delivered_offset = end as u64;
        Ok(self.stdin_history[start..end].to_vec())
    }

    pub fn push_control(
        &mut self,
        token: SessionToken,
        seq: u64,
        event: ControlEvent,
    ) -> Result<(), ProtocolError> {
        self.check_token(token)?;
        if seq != self.next_control_seq {
            return Err(ProtocolError::ControlSequenceGap {
                expected: self.next_control_seq,
                got: seq,
            });
        }
        self.next_control_seq += 1;
        self.events.push(OrderedControlEvent { seq, event });
        Ok(())
    }

    pub fn events(&self) -> &[OrderedControlEvent] {
        &self.events
    }

    fn check_token(&self, token: SessionToken) -> Result<(), ProtocolError> {
        if token == self.token {
            Ok(())
        } else {
            Err(ProtocolError::StaleSession)
        }
    }

    fn log(&self, stream: Stream) -> &OutputLog {
        match stream {
            Stream::Stdout => &self.stdout,
            Stream::Stderr => &self.stderr,
        }
    }

    fn log_mut(&mut self, stream: Stream) -> &mut OutputLog {
        match stream {
            Stream::Stdout => &mut self.stdout,
            Stream::Stderr => &mut self.stderr,
        }
    }
}

pub fn pattern_chunk(stream: Stream, absolute_offset: u64, len: usize) -> Vec<u8> {
    let salt = match stream {
        Stream::Stdout => 0x5a_u64,
        Stream::Stderr => 0xa5_u64,
    };
    (0..len)
        .map(|idx| ((absolute_offset + idx as u64).wrapping_mul(31) ^ salt) as u8)
        .collect()
}

pub fn stdin_pattern(absolute_offset: u64, len: usize) -> Vec<u8> {
    (0..len)
        .map(|idx| ((absolute_offset + idx as u64).wrapping_mul(17) ^ 0x3c) as u8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    fn fill_output(session: &mut ChunkedSession, stream: Stream, total: usize) {
        let token = session.token();
        let mut offset = 0_u64;
        while offset < total as u64 {
            let len = DEFAULT_CHUNK.min(total - offset as usize);
            let data = pattern_chunk(stream, offset, len);
            assert_eq!(session.append_output(token, stream, &data), Ok(offset));
            offset += len as u64;
        }
        session.finish_output(token, stream).unwrap();
    }

    fn assert_read_exact(session: &ChunkedSession, stream: Stream, total: usize) {
        let token = session.token();
        let mut offset = 0_u64;
        while offset < total as u64 {
            let response = match stream {
                Stream::Stdout => session.read_stdout(token, offset, DEFAULT_CHUNK),
                Stream::Stderr => session.read_stderr(token, offset, DEFAULT_CHUNK),
            }
            .unwrap();
            assert_eq!(response.offset, offset);
            assert!(!response.data.is_empty());
            assert_eq!(
                response.data,
                pattern_chunk(stream, offset, response.data.len())
            );
            offset += response.data.len() as u64;
        }
        let eof = match stream {
            Stream::Stdout => session.read_stdout(token, total as u64, DEFAULT_CHUNK),
            Stream::Stderr => session.read_stderr(token, total as u64, DEFAULT_CHUNK),
        }
        .unwrap();
        assert!(eof.data.is_empty());
        assert!(eof.eof);
    }

    #[test]
    fn stdout_and_stderr_64_mib_are_byte_exact_by_offset() {
        let total = 64 * MIB;
        let mut session = ChunkedSession::new(1, 140 * MIB, 2 * MIB, DEFAULT_CHUNK);
        fill_output(&mut session, Stream::Stdout, total);
        fill_output(&mut session, Stream::Stderr, total);
        assert_eq!(session.retained_output_bytes(), 128 * MIB);
        assert_read_exact(&session, Stream::Stdout, total);
        assert_read_exact(&session, Stream::Stderr, total);
    }

    #[test]
    fn slow_stdin_16_mib_honors_offsets_duplicates_and_backpressure() {
        let total = 16 * MIB;
        let mut session = ChunkedSession::new(2, 2 * MIB, 512 * KIB, DEFAULT_CHUNK);
        let token = session.token();
        let mut offset = 0_u64;
        let mut write_id = 10_u64;
        let mut delivered = Vec::with_capacity(total);

        while offset < total as u64 {
            let len = DEFAULT_CHUNK.min(total - offset as usize);
            let chunk = stdin_pattern(offset, len);
            match session.write_stdin(token, write_id, offset, &chunk) {
                Ok(response) => {
                    assert_eq!(response.disposition, WriteDisposition::Accepted);
                    assert_eq!(response.next_offset, offset + len as u64);
                    let duplicate = session
                        .write_stdin(token, write_id + 100_000, offset, &chunk)
                        .unwrap();
                    assert_eq!(duplicate.disposition, WriteDisposition::Duplicate);
                    offset += len as u64;
                    write_id += 1;
                }
                Err(ProtocolError::SlowConsumer { retained, cap }) => {
                    assert!(retained <= cap);
                    let consumed = session.consume_stdin(token, 128 * KIB).unwrap();
                    assert!(!consumed.is_empty());
                    delivered.extend_from_slice(&consumed);
                    continue;
                }
                Err(err) => panic!("unexpected WriteStdin error: {err:?}"),
            }
            let consumed = session.consume_stdin(token, 64 * KIB).unwrap();
            delivered.extend_from_slice(&consumed);
        }

        while delivered.len() < total {
            let consumed = session.consume_stdin(token, 256 * KIB).unwrap();
            assert!(!consumed.is_empty());
            delivered.extend_from_slice(&consumed);
        }
        assert_eq!(delivered, stdin_pattern(0, total));

        assert!(matches!(
            session.write_stdin(token, write_id, offset + 1, b"gap"),
            Err(ProtocolError::OffsetGap { .. })
        ));
        assert!(matches!(
            session.write_stdin(token, write_id, 0, b"wrong"),
            Err(ProtocolError::OffsetBehind { .. })
        ));
    }

    #[test]
    fn thirty_second_slow_consumer_is_bounded_and_reports_backpressure() {
        let cap = 2 * MIB;
        let mut session = ChunkedSession::new(3, cap, 2 * MIB, 64 * KIB);
        let token = session.token();
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut offset = 0_u64;
        let mut saw_slow_consumer = false;
        let mut max_retained = 0_usize;

        while Instant::now() < deadline {
            let chunk = pattern_chunk(Stream::Stdout, offset, 64 * KIB);
            match session.append_output(token, Stream::Stdout, &chunk) {
                Ok(start) => {
                    assert_eq!(start, offset);
                    offset += chunk.len() as u64;
                }
                Err(ProtocolError::SlowConsumer { retained, cap }) => {
                    saw_slow_consumer = true;
                    assert!(retained <= cap);
                }
                Err(err) => panic!("unexpected append_output error: {err:?}"),
            }
            max_retained = max_retained.max(session.retained_output_bytes());
            assert!(max_retained <= cap);

            let response = session
                .read_output(token, Stream::Stdout, session.stdout.base_offset, 1024)
                .unwrap();
            if !response.data.is_empty() {
                session
                    .ack_output(
                        token,
                        Stream::Stdout,
                        response.offset + response.data.len() as u64,
                    )
                    .unwrap();
            }
            thread::sleep(Duration::from_millis(25));
        }

        assert!(
            saw_slow_consumer,
            "producer must observe bounded slow-consumer pressure"
        );
        assert!(max_retained <= cap);
    }

    #[test]
    fn four_concurrent_attached_sessions_meet_latency_and_fairness_thresholds() {
        const SESSIONS: usize = 4;
        const TOTAL: usize = 4 * MIB;
        const READ: usize = 32 * KIB;
        const P95_MS: u128 = 25;
        const MAX_MS: u128 = 100;
        const FAIRNESS_BYTES: usize = 128 * KIB;

        let sessions: Vec<_> = (0..SESSIONS)
            .map(|idx| {
                let mut session =
                    ChunkedSession::new(100 + idx as u64, 8 * MIB, 1 * MIB, DEFAULT_CHUNK);
                fill_output(&mut session, Stream::Stdout, TOTAL);
                Arc::new(Mutex::new(session))
            })
            .collect();

        let handles: Vec<_> = sessions
            .into_iter()
            .map(|session| {
                thread::spawn(move || {
                    let token = session.lock().unwrap().token();
                    let mut offset = 0_u64;
                    let mut bytes = 0_usize;
                    let mut latencies = Vec::new();
                    while bytes < TOTAL {
                        let start = Instant::now();
                        let response = session
                            .lock()
                            .unwrap()
                            .read_output(token, Stream::Stdout, offset, READ)
                            .unwrap();
                        let latency = start.elapsed().as_millis();
                        assert_eq!(
                            response.data,
                            pattern_chunk(Stream::Stdout, offset, response.data.len())
                        );
                        offset += response.data.len() as u64;
                        bytes += response.data.len();
                        latencies.push(latency);
                        thread::yield_now();
                    }
                    latencies.sort_unstable();
                    let p95 = latencies[(latencies.len() * 95 / 100).min(latencies.len() - 1)];
                    let max = *latencies.last().unwrap();
                    (bytes, p95, max)
                })
            })
            .collect();

        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.join().unwrap());
        }
        let min_bytes = results.iter().map(|(bytes, _, _)| *bytes).min().unwrap();
        let max_bytes = results.iter().map(|(bytes, _, _)| *bytes).max().unwrap();
        assert!(
            max_bytes - min_bytes <= FAIRNESS_BYTES,
            "session byte skew exceeded threshold"
        );
        for (bytes, p95, max) in results {
            assert_eq!(bytes, TOTAL);
            assert!(
                p95 <= P95_MS,
                "p95 attached read latency {p95}ms > {P95_MS}ms"
            );
            assert!(
                max <= MAX_MS,
                "max attached read latency {max}ms > {MAX_MS}ms"
            );
        }
    }

    #[test]
    fn stale_session_after_restart_is_rejected() {
        let mut session = ChunkedSession::new(4, MIB, MIB, 64 * KIB);
        let old = session.token();
        let new = session.restart();
        assert_ne!(old, new);
        assert_eq!(
            session.append_output(old, Stream::Stdout, b"stale"),
            Err(ProtocolError::StaleSession)
        );
        assert_eq!(session.append_output(new, Stream::Stdout, b"fresh"), Ok(0));
    }

    #[test]
    fn eof_and_tty_ctrl_d_are_distinct() {
        let mut session = ChunkedSession::new(5, MIB, MIB, 64 * KIB);
        let token = session.token();
        assert_eq!(
            session
                .write_stdin(token, 1, 0, &[0x04])
                .unwrap()
                .disposition,
            WriteDisposition::Accepted
        );
        assert_eq!(session.consume_stdin(token, 1).unwrap(), vec![0x04]);
        assert_eq!(
            session.close_stdin(token, 1).unwrap().disposition,
            WriteDisposition::Accepted
        );
        assert_eq!(
            session.close_stdin(token, 1).unwrap().disposition,
            WriteDisposition::Duplicate
        );
        assert_eq!(
            session.write_stdin(token, 2, 1, b"after-eof"),
            Err(ProtocolError::StdinClosed)
        );
    }

    #[test]
    fn resize_ordering_and_signal_exit_mapping_are_explicit() {
        let mut session = ChunkedSession::new(6, MIB, MIB, 64 * KIB);
        let token = session.token();
        session
            .push_control(
                token,
                0,
                ControlEvent::Resize(WindowSize {
                    rows: 40,
                    cols: 120,
                }),
            )
            .unwrap();
        session
            .push_control(token, 1, ControlEvent::Signal(Signal::Int))
            .unwrap();
        session
            .push_control(
                token,
                2,
                ControlEvent::Exit(ExitStatus::from_signal(Signal::Int)),
            )
            .unwrap();
        assert_eq!(
            session.push_control(
                token,
                4,
                ControlEvent::Resize(WindowSize { rows: 24, cols: 80 })
            ),
            Err(ProtocolError::ControlSequenceGap {
                expected: 3,
                got: 4
            })
        );
        assert_eq!(
            session.events(),
            &[
                OrderedControlEvent {
                    seq: 0,
                    event: ControlEvent::Resize(WindowSize {
                        rows: 40,
                        cols: 120
                    }),
                },
                OrderedControlEvent {
                    seq: 1,
                    event: ControlEvent::Signal(Signal::Int),
                },
                OrderedControlEvent {
                    seq: 2,
                    event: ControlEvent::Exit(ExitStatus::Signal {
                        signal: Signal::Int,
                        status_code: 130,
                    }),
                },
            ]
        );
    }
}
