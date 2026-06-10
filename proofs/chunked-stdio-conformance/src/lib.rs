#![forbid(unsafe_code)]

//! Executable conformance model for nixling's selected Kata-style
//! chunked stdio guest-control protocol.
//!
//! The crate intentionally uses only `std`: it is a protocol proof, not
//! a production implementation. The tests exercise byte offsets,
//! idempotent stdin writes, slow-consumer caps, stale session detection,
//! terminal control ordering, and attached-session fairness thresholds.

use std::collections::HashMap;

pub const KIB: usize = 1024;
pub const MIB: usize = 1024 * KIB;
pub const DEFAULT_CHUNK: usize = 256 * KIB;
pub const MAX_DEDUPE_ENTRIES: usize = 1024;

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
    OffsetGap {
        expected: u64,
        got: u64,
    },
    OffsetBehind {
        expected: u64,
        got: u64,
    },
    OutputEvicted {
        earliest: u64,
        got: u64,
    },
    StdinClosed,
    SlowConsumer {
        retained: usize,
        cap: usize,
    },
    DuplicateWriteId,
    RequestIdConflict,
    ControlSequenceGap {
        expected: u64,
        got: u64,
    },
    ReceiveMessageTooLarge,
    DecodeBudgetExhausted {
        used: usize,
        requested: usize,
        cap: usize,
    },
    EmptyStdinData,
    StdinRequestInFlight,
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
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrderedControlEvent {
    pub seq: u64,
    pub event: ControlEvent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StdinEndpoint {
    Pipe,
    Pty,
}

#[derive(Clone, Debug)]
struct OutputLog {
    base_offset: u64,
    bytes: Vec<u8>,
    eof: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AcceptedWrite {
    offset: u64,
    len: usize,
    hash: u64,
    close_after: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AcceptedCloseAfter {
    offset: u64,
    len: usize,
    hash: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AcceptedControl {
    seq: u64,
    event: ControlEvent,
}

#[derive(Clone, Debug)]
struct StdinQueue {
    base_offset: u64,
    bytes: Vec<u8>,
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

impl StdinQueue {
    fn new() -> Self {
        Self {
            base_offset: 0,
            bytes: Vec::new(),
        }
    }

    fn next_offset(&self) -> u64 {
        self.base_offset + self.bytes.len() as u64
    }

    fn buffered_bytes(&self) -> usize {
        self.bytes.len()
    }

    fn append(&mut self, data: &[u8]) {
        self.bytes.extend_from_slice(data);
    }

    fn consume(&mut self, max_len: usize) -> Vec<u8> {
        let end = max_len.min(self.bytes.len());
        let consumed: Vec<_> = self.bytes.drain(0..end).collect();
        self.base_offset += consumed.len() as u64;
        consumed
    }
}

#[derive(Clone, Debug)]
pub struct DecodedByteBudget {
    cap: usize,
    used: usize,
}

impl DecodedByteBudget {
    pub fn new(cap: usize) -> Self {
        Self { cap, used: 0 }
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn try_acquire(&mut self, len: usize) -> Result<(), ProtocolError> {
        if len > self.cap {
            return Err(ProtocolError::ReceiveMessageTooLarge);
        }
        if self.used + len > self.cap {
            return Err(ProtocolError::DecodeBudgetExhausted {
                used: self.used,
                requested: len,
                cap: self.cap,
            });
        }
        self.used += len;
        Ok(())
    }

    pub fn release(&mut self, len: usize) {
        self.used = self
            .used
            .checked_sub(len)
            .expect("decoded budget release exceeds acquired bytes");
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
    stdin: StdinQueue,
    stdin_endpoint: StdinEndpoint,
    stdin_closed: bool,
    child_stdin_closed: bool,
    stdin_inflight: bool,
    close_stdin_request: Option<(u64, u64)>,
    close_after_request: Option<(u64, AcceptedCloseAfter)>,
    writes: HashMap<u64, AcceptedWrite>,
    controls: HashMap<u64, AcceptedControl>,
    next_control_seq: u64,
    events: Vec<OrderedControlEvent>,
    terminal_status: Option<ExitStatus>,
}

impl ChunkedSession {
    pub fn new(session_id: u64, output_cap: usize, stdin_cap: usize, max_chunk: usize) -> Self {
        Self::with_stdin_endpoint(
            session_id,
            output_cap,
            stdin_cap,
            max_chunk,
            StdinEndpoint::Pipe,
        )
    }

    pub fn with_stdin_endpoint(
        session_id: u64,
        output_cap: usize,
        stdin_cap: usize,
        max_chunk: usize,
        stdin_endpoint: StdinEndpoint,
    ) -> Self {
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
            stdin: StdinQueue::new(),
            stdin_endpoint,
            stdin_closed: false,
            child_stdin_closed: false,
            stdin_inflight: false,
            close_stdin_request: None,
            close_after_request: None,
            writes: HashMap::new(),
            controls: HashMap::new(),
            // ExecCreate reports control_seq=0. The first post-create
            // mutation must therefore use control_seq=1.
            next_control_seq: 1,
            events: Vec::new(),
            terminal_status: None,
        }
    }

    pub fn token(&self) -> SessionToken {
        self.token
    }

    pub fn restart(&mut self) -> SessionToken {
        self.token.generation += 1;
        self.stdin_inflight = false;
        self.token
    }

    pub fn retained_output_bytes(&self) -> usize {
        self.stdout.bytes.len() + self.stderr.bytes.len()
    }

    pub fn stdin_next_offset(&self) -> u64 {
        self.stdin.next_offset()
    }

    pub fn stdin_buffered_bytes(&self) -> usize {
        self.stdin.buffered_bytes()
    }

    pub fn stdin_dedupe_entries(&self) -> usize {
        self.writes.len()
    }

    pub fn child_stdin_closed(&self) -> bool {
        self.child_stdin_closed
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
        self.write_stdin_with_close_after(token, write_id, offset, data, false)
    }

    pub fn write_stdin_close_after(
        &mut self,
        token: SessionToken,
        write_id: u64,
        offset: u64,
        data: &[u8],
    ) -> Result<WriteStdinResponse, ProtocolError> {
        self.write_stdin_with_close_after(token, write_id, offset, data, true)
    }

    fn write_stdin_with_close_after(
        &mut self,
        token: SessionToken,
        write_id: u64,
        offset: u64,
        data: &[u8],
        close_after: bool,
    ) -> Result<WriteStdinResponse, ProtocolError> {
        self.check_token(token)?;
        if self.stdin_inflight {
            return Err(ProtocolError::StdinRequestInFlight);
        }
        if data.is_empty() {
            return Err(ProtocolError::EmptyStdinData);
        }
        self.stdin_inflight = true;
        let result = self.write_stdin_admitted(token, write_id, offset, data, close_after);
        self.stdin_inflight = false;
        result
    }

    fn write_stdin_admitted(
        &mut self,
        token: SessionToken,
        write_id: u64,
        offset: u64,
        data: &[u8],
        close_after: bool,
    ) -> Result<WriteStdinResponse, ProtocolError> {
        self.check_token(token)?;
        if data.len() > self.max_chunk {
            return Err(ProtocolError::ChunkTooLarge);
        }
        let expected = self.stdin_next_offset();
        if let Some(previous) = self.writes.get(&write_id) {
            if previous.offset == offset
                && previous.hash == Self::hash(data)
                && previous.close_after == close_after
            {
                return Ok(WriteStdinResponse {
                    next_offset: expected,
                    disposition: WriteDisposition::Duplicate,
                });
            }
            return Err(ProtocolError::RequestIdConflict);
        }
        if let Some((previous_id, previous)) = &self.close_after_request {
            if *previous_id == write_id {
                if previous.offset == offset
                    && previous.hash == Self::hash(data)
                    && previous.len == data.len()
                    && close_after
                {
                    return Ok(WriteStdinResponse {
                        next_offset: expected,
                        disposition: WriteDisposition::Duplicate,
                    });
                }
                return Err(ProtocolError::RequestIdConflict);
            }
        }
        if self.stdin_closed {
            return Err(ProtocolError::StdinClosed);
        }
        if offset < expected {
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
        if self.stdin_buffered_bytes() + data.len() > self.stdin_cap {
            return Err(ProtocolError::SlowConsumer {
                retained: self.stdin_buffered_bytes(),
                cap: self.stdin_cap,
            });
        }
        self.writes.insert(
            write_id,
            AcceptedWrite {
                offset,
                len: data.len(),
                hash: Self::hash(data),
                close_after,
            },
        );
        self.evict_stdin_dedupe_to_limit();
        self.stdin.append(data);
        if close_after {
            self.stdin_closed = true;
            let next = self.stdin_next_offset();
            self.close_stdin_request = Some((write_id, next));
            self.close_after_request = Some((
                write_id,
                AcceptedCloseAfter {
                    offset,
                    len: data.len(),
                    hash: Self::hash(data),
                },
            ));
            self.child_stdin_closed =
                self.stdin_endpoint == StdinEndpoint::Pipe && self.stdin_buffered_bytes() == 0;
        }
        Ok(WriteStdinResponse {
            next_offset: self.stdin_next_offset(),
            disposition: WriteDisposition::Accepted,
        })
    }

    pub fn close_stdin(
        &mut self,
        token: SessionToken,
        request_id: u64,
        offset: u64,
    ) -> Result<WriteStdinResponse, ProtocolError> {
        self.check_token(token)?;
        if self.stdin_inflight {
            return Err(ProtocolError::StdinRequestInFlight);
        }
        let expected = self.stdin_next_offset();
        if self.stdin_closed {
            if self
                .close_after_request
                .as_ref()
                .is_some_and(|(id, _)| *id == request_id)
            {
                return Err(ProtocolError::RequestIdConflict);
            }
            if self.close_stdin_request == Some((request_id, offset)) {
                return Ok(WriteStdinResponse {
                    next_offset: expected,
                    disposition: WriteDisposition::Duplicate,
                });
            }
            if self
                .close_stdin_request
                .is_some_and(|(id, _)| id == request_id)
            {
                return Err(ProtocolError::RequestIdConflict);
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
        self.child_stdin_closed =
            self.stdin_endpoint == StdinEndpoint::Pipe && self.stdin_buffered_bytes() == 0;
        self.close_stdin_request = Some((request_id, offset));
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
        let consumed = self.stdin.consume(max_len);
        self.evict_stdin_dedupe();
        if self.stdin_closed
            && self.stdin_endpoint == StdinEndpoint::Pipe
            && self.stdin_buffered_bytes() == 0
        {
            self.child_stdin_closed = true;
        }
        Ok(consumed)
    }

    pub fn begin_stdin_request(&mut self, token: SessionToken) -> Result<(), ProtocolError> {
        self.check_token(token)?;
        if self.stdin_inflight {
            return Err(ProtocolError::StdinRequestInFlight);
        }
        self.stdin_inflight = true;
        Ok(())
    }

    pub fn finish_stdin_request(&mut self, token: SessionToken) -> Result<(), ProtocolError> {
        self.check_token(token)?;
        self.stdin_inflight = false;
        Ok(())
    }

    pub fn push_control(
        &mut self,
        token: SessionToken,
        request_id: u64,
        seq: u64,
        event: ControlEvent,
    ) -> Result<WriteDisposition, ProtocolError> {
        self.check_token(token)?;
        if let Some(previous) = self.controls.get(&request_id) {
            if previous.seq == seq && previous.event == event {
                return Ok(WriteDisposition::Duplicate);
            }
            return Err(ProtocolError::RequestIdConflict);
        }
        if seq != self.next_control_seq {
            return Err(ProtocolError::ControlSequenceGap {
                expected: self.next_control_seq,
                got: seq,
            });
        }
        self.next_control_seq += 1;
        self.controls
            .insert(request_id, AcceptedControl { seq, event });
        self.evict_control_dedupe_to_limit();
        self.events.push(OrderedControlEvent { seq, event });
        Ok(WriteDisposition::Accepted)
    }

    pub fn events(&self) -> &[OrderedControlEvent] {
        &self.events
    }

    pub fn set_terminal_status(
        &mut self,
        token: SessionToken,
        status: ExitStatus,
    ) -> Result<(), ProtocolError> {
        self.check_token(token)?;
        self.terminal_status = Some(status);
        Ok(())
    }

    pub fn terminal_status(&self) -> Option<ExitStatus> {
        self.terminal_status
    }

    pub fn visible_terminal_status_after_output_drain(&self) -> Option<ExitStatus> {
        if self.retained_output_bytes() == 0 {
            self.terminal_status
        } else {
            None
        }
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

    fn hash(data: &[u8]) -> u64 {
        // Deterministic non-cryptographic proof hash. The production design uses
        // a real payload hash in the request-id dedupe ring.
        data.iter().fold(0xcbf29ce484222325_u64, |acc, byte| {
            (acc ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
    }

    fn evict_stdin_dedupe(&mut self) {
        let earliest = self.stdin.base_offset;
        self.writes
            .retain(|_, accepted| accepted.offset + accepted.len as u64 > earliest);
        self.evict_stdin_dedupe_to_limit();
    }

    fn evict_stdin_dedupe_to_limit(&mut self) {
        while self.writes.len() > MAX_DEDUPE_ENTRIES {
            let Some(oldest_id) = self
                .writes
                .iter()
                .min_by_key(|(_, accepted)| accepted.offset)
                .map(|(id, _)| *id)
            else {
                break;
            };
            self.writes.remove(&oldest_id);
        }
    }

    fn evict_control_dedupe_to_limit(&mut self) {
        while self.controls.len() > MAX_DEDUPE_ENTRIES {
            let Some(oldest_id) = self
                .controls
                .iter()
                .min_by_key(|(_, accepted)| accepted.seq)
                .map(|(id, _)| *id)
            else {
                break;
            };
            self.controls.remove(&oldest_id);
        }
    }

    pub fn receive_message(max_recv: usize, payload_len: usize) -> Result<Vec<u8>, ProtocolError> {
        if payload_len > max_recv {
            return Err(ProtocolError::ReceiveMessageTooLarge);
        }
        Ok(vec![0u8; payload_len])
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
        let mut max_buffered = 0_usize;

        while offset < total as u64 {
            let len = DEFAULT_CHUNK.min(total - offset as usize);
            let chunk = stdin_pattern(offset, len);
            match session.write_stdin(token, write_id, offset, &chunk) {
                Ok(response) => {
                    assert_eq!(response.disposition, WriteDisposition::Accepted);
                    assert_eq!(response.next_offset, offset + len as u64);
                    let duplicate = session
                        .write_stdin(token, write_id, offset, &chunk)
                        .unwrap();
                    assert_eq!(duplicate.disposition, WriteDisposition::Duplicate);
                    assert_eq!(
                        session.write_stdin(token, write_id, offset, b"different"),
                        Err(ProtocolError::RequestIdConflict)
                    );
                    assert!(matches!(
                        session.write_stdin(token, write_id + 100_000, offset, &chunk),
                        Err(ProtocolError::OffsetBehind { .. })
                    ));
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
            max_buffered = max_buffered.max(session.stdin_buffered_bytes());
            assert!(max_buffered <= 512 * KIB);
            let consumed = session.consume_stdin(token, 64 * KIB).unwrap();
            delivered.extend_from_slice(&consumed);
            max_buffered = max_buffered.max(session.stdin_buffered_bytes());
            assert!(max_buffered <= 512 * KIB);
        }

        while delivered.len() < total {
            let consumed = session.consume_stdin(token, 256 * KIB).unwrap();
            assert!(!consumed.is_empty());
            delivered.extend_from_slice(&consumed);
            max_buffered = max_buffered.max(session.stdin_buffered_bytes());
            assert!(max_buffered <= 512 * KIB);
        }
        assert_eq!(delivered, stdin_pattern(0, total));
        assert_eq!(session.stdin_buffered_bytes(), 0);

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
    fn partial_child_writes_drain_queue_without_duplicate_or_lost_stdin() {
        for (name, endpoint, write_steps) in [
            (
                "pipe",
                StdinEndpoint::Pipe,
                &[8 * KIB, 16 * KIB, 4 * KIB][..],
            ),
            ("pty", StdinEndpoint::Pty, &[1024, 4096, 2048][..]),
        ] {
            let mut session =
                ChunkedSession::with_stdin_endpoint(20, MIB, 64 * KIB, 64 * KIB, endpoint);
            let token = session.token();
            let first = stdin_pattern(0, 64 * KIB);
            let second = stdin_pattern(64 * KIB as u64, 64 * KIB);
            session.write_stdin(token, 1, 0, &first).unwrap();

            let first_partial = session.consume_stdin(token, write_steps[0]).unwrap();
            assert_eq!(
                first_partial,
                stdin_pattern(0, first_partial.len()),
                "{name} first partial write must preserve prefix bytes"
            );
            assert_eq!(
                session.write_stdin(token, 2, 64 * KIB as u64, &second),
                Err(ProtocolError::SlowConsumer {
                    retained: 64 * KIB - write_steps[0],
                    cap: 64 * KIB,
                }),
                "{name} must reject a retryable second chunk while the bounded queue is occupied"
            );

            let mut delivered = first_partial;
            let mut step = 1;
            while delivered.len() < 64 * KIB {
                let remaining = 64 * KIB - delivered.len();
                let consumed = session
                    .consume_stdin(token, write_steps[step % write_steps.len()].min(remaining))
                    .unwrap();
                assert!(!consumed.is_empty(), "{name} partial writer stalled");
                delivered.extend_from_slice(&consumed);
                assert!(session.stdin_buffered_bytes() <= 64 * KIB);
                step += 1;
            }
            assert_eq!(delivered, first, "{name} delivered bytes changed");
            assert_eq!(session.stdin_buffered_bytes(), 0);

            session
                .write_stdin(token, 2, 64 * KIB as u64, &second)
                .unwrap();
            assert_eq!(
                session.close_stdin(token, 3, 128 * KIB as u64),
                Ok(WriteStdinResponse {
                    next_offset: 128 * KIB as u64,
                    disposition: WriteDisposition::Accepted,
                }),
                "{name} close_after ordering must observe both accepted chunks"
            );
            assert!(
                !session.child_stdin_closed(),
                "{name} child stdin must stay open until queued bytes drain"
            );
            assert_eq!(session.consume_stdin(token, 64 * KIB).unwrap(), second);
            if endpoint == StdinEndpoint::Pipe {
                assert!(
                    session.child_stdin_closed(),
                    "{name} child stdin must close only after queued bytes drain"
                );
            } else {
                assert!(
                    !session.child_stdin_closed(),
                    "{name} protocol close must not synthesize PTY EOF/HUP"
                );
                session
                    .append_output(token, Stream::Stdout, b"after-close")
                    .unwrap();
                assert_eq!(
                    session.read_stdout(token, 0, 64 * KIB).unwrap().data,
                    b"after-close"
                );
            }
        }
    }

    #[test]
    fn write_stdin_close_after_is_atomic_and_endpoint_specific() {
        for (name, endpoint) in [("pipe", StdinEndpoint::Pipe), ("pty", StdinEndpoint::Pty)] {
            let mut session =
                ChunkedSession::with_stdin_endpoint(21, MIB, 64 * KIB, 64 * KIB, endpoint);
            let token = session.token();
            let final_chunk = stdin_pattern(0, 64 * KIB);
            assert_eq!(
                session.write_stdin_close_after(token, 10, 0, &final_chunk),
                Ok(WriteStdinResponse {
                    next_offset: 64 * KIB as u64,
                    disposition: WriteDisposition::Accepted,
                }),
                "{name} close_after must accept the final chunk atomically"
            );
            assert_eq!(
                session.write_stdin_close_after(token, 10, 0, &final_chunk),
                Ok(WriteStdinResponse {
                    next_offset: 64 * KIB as u64,
                    disposition: WriteDisposition::Duplicate,
                }),
                "{name} close_after lost-response retry must be idempotent"
            );
            assert_eq!(
                session.write_stdin_close_after(token, 10, 0, b"different"),
                Err(ProtocolError::RequestIdConflict),
                "{name} close_after mismatched duplicate must be rejected"
            );
            assert_eq!(
                session.write_stdin(token, 11, 64 * KIB as u64, b"after-close"),
                Err(ProtocolError::StdinClosed),
                "{name} close_after must reject later host input"
            );
            assert_eq!(
                session.close_stdin(token, 10, 64 * KIB as u64),
                Err(ProtocolError::RequestIdConflict),
                "{name} close_after request id is scoped to the WriteStdin RPC shape"
            );
            assert!(
                !session.child_stdin_closed(),
                "{name} child stdin must not close before queued final bytes drain"
            );
            assert_eq!(
                session.consume_stdin(token, 64 * KIB).unwrap(),
                final_chunk,
                "{name} close_after must deliver the final chunk exactly once"
            );
            assert_eq!(
                session.write_stdin_close_after(token, 10, 0, &final_chunk),
                Ok(WriteStdinResponse {
                    next_offset: 64 * KIB as u64,
                    disposition: WriteDisposition::Duplicate,
                }),
                "{name} close_after lost-response retry after drain must stay idempotent"
            );
            if endpoint == StdinEndpoint::Pipe {
                assert!(
                    session.child_stdin_closed(),
                    "{name} pipe stdin must half-close after the final chunk drains"
                );
            } else {
                assert!(
                    !session.child_stdin_closed(),
                    "{name} PTY close_after must not synthesize EOF/HUP"
                );
                session
                    .append_output(token, Stream::Stdout, b"pty-output-after-close")
                    .unwrap();
                assert_eq!(
                    session.read_stdout(token, 0, 64 * KIB).unwrap().data,
                    b"pty-output-after-close"
                );
            }
        }

        let mut backpressured = ChunkedSession::new(22, MIB, 8 * KIB, 64 * KIB);
        let token = backpressured.token();
        let too_large_for_queue = stdin_pattern(0, 16 * KIB);
        assert_eq!(
            backpressured.write_stdin_close_after(token, 1, 0, &too_large_for_queue),
            Err(ProtocolError::SlowConsumer {
                retained: 0,
                cap: 8 * KIB,
            })
        );
        assert_eq!(
            backpressured.write_stdin(token, 2, 0, b"x"),
            Ok(WriteStdinResponse {
                next_offset: 1,
                disposition: WriteDisposition::Accepted,
            }),
            "failed close_after must leave stdin open and offset unchanged"
        );
    }

    #[test]
    fn deterministic_slow_consumer_stress_is_bounded_and_reports_backpressure() {
        let cap = 2 * MIB;
        let mut session = ChunkedSession::new(3, cap, 2 * MIB, 64 * KIB);
        let token = session.token();
        let mut offset = 0_u64;
        let mut stderr_offset = 0_u64;
        let mut max_retained = 0_usize;
        let mut slow_consumer_errors = 0_usize;

        for _ in 0..1200 {
            let (stream, cursor) = if offset <= stderr_offset {
                (Stream::Stdout, &mut offset)
            } else {
                (Stream::Stderr, &mut stderr_offset)
            };
            let chunk = pattern_chunk(stream, *cursor, 64 * KIB);
            match session.append_output(token, stream, &chunk) {
                Ok(start) => {
                    assert_eq!(start, *cursor);
                    *cursor += chunk.len() as u64;
                }
                Err(ProtocolError::SlowConsumer { retained, cap }) => {
                    slow_consumer_errors += 1;
                    assert!(retained <= cap);
                }
                Err(err) => panic!("unexpected append_output error: {err:?}"),
            }
            max_retained = max_retained.max(session.retained_output_bytes());
            assert!(max_retained <= cap);
        }
        assert!(
            slow_consumer_errors > 0,
            "active producer must observe bounded slow-consumer pressure"
        );
        assert!(max_retained <= cap);
    }

    #[test]
    fn four_concurrent_attached_sessions_progress_fairly_under_round_robin_scheduler() {
        const SESSIONS: usize = 4;
        const TOTAL: usize = 4 * MIB;
        const READ: usize = 32 * KIB;
        const FAIRNESS_BYTES: u64 = (READ * (SESSIONS - 1)) as u64;

        let mut sessions: Vec<_> = (0..SESSIONS)
            .map(|idx| {
                let mut session =
                    ChunkedSession::new(100 + idx as u64, 8 * MIB, MIB, DEFAULT_CHUNK);
                fill_output(&mut session, Stream::Stdout, TOTAL);
                session
            })
            .collect();
        let tokens: Vec<_> = sessions.iter().map(ChunkedSession::token).collect();
        let mut offsets = [0_u64; SESSIONS];
        let mut read_turns = 0_usize;

        while offsets.iter().any(|offset| *offset < TOTAL as u64) {
            for idx in 0..SESSIONS {
                if offsets[idx] >= TOTAL as u64 {
                    continue;
                }
                let response = sessions[idx]
                    .read_output(tokens[idx], Stream::Stdout, offsets[idx], READ)
                    .unwrap();
                assert_eq!(
                    response.data,
                    pattern_chunk(Stream::Stdout, offsets[idx], response.data.len())
                );
                offsets[idx] += response.data.len() as u64;
                sessions[idx]
                    .ack_output(tokens[idx], Stream::Stdout, offsets[idx])
                    .unwrap();
                read_turns += 1;

                let min = *offsets.iter().min().unwrap();
                let max = *offsets.iter().max().unwrap();
                assert!(
                    max - min <= FAIRNESS_BYTES,
                    "session byte skew exceeded deterministic round-robin bound"
                );
            }
        }

        assert_eq!(read_turns, SESSIONS * (TOTAL / READ));
        for (idx, session) in sessions.iter().enumerate() {
            assert_eq!(offsets[idx], TOTAL as u64);
            assert_eq!(session.retained_output_bytes(), 0);
        }
    }

    #[test]
    fn mixed_attached_load_has_bounded_deterministic_service_gaps() {
        const MAX_TURN_GAP: u64 = 4;
        const INTERACTIVE_OPS: u64 = 240;
        const HEALTH_OPS: u64 = 240;

        let mut slow_output = ChunkedSession::new(200, 8 * MIB, 512 * KIB, DEFAULT_CHUNK);
        fill_output(&mut slow_output, Stream::Stdout, 2 * MIB);
        let slow_token = slow_output.token();
        let mut slow_offset = 0_u64;

        let mut blocked_stdin = ChunkedSession::new(201, MIB, 64 * KIB, 64 * KIB);
        let blocked_token = blocked_stdin.token();
        let mut blocked_offset = 0_u64;
        let mut blocked_write_id = 1_u64;
        let mut blocked_slow = 0_usize;

        let mut interactive = ChunkedSession::new(202, MIB, MIB, 64 * KIB);
        let interactive_token = interactive.token();
        let mut interactive_ops = 0_u64;
        let mut health_ops = 0_u64;
        let mut turn = 0_u64;
        let mut last_interactive_turn = 0_u64;
        let mut last_health_turn = 0_u64;
        let mut max_interactive_gap = 0_u64;
        let mut max_health_gap = 0_u64;

        while slow_offset < 2 * MIB as u64
            || blocked_write_id <= 64
            || interactive_ops < INTERACTIVE_OPS
            || health_ops < HEALTH_OPS
        {
            turn += 1;
            if health_ops < HEALTH_OPS {
                if last_health_turn != 0 {
                    max_health_gap = max_health_gap.max(turn - last_health_turn);
                }
                last_health_turn = turn;
                health_ops += 1;
            }

            turn += 1;
            if interactive_ops < INTERACTIVE_OPS {
                if last_interactive_turn != 0 {
                    max_interactive_gap = max_interactive_gap.max(turn - last_interactive_turn);
                }
                last_interactive_turn = turn;
                let offset = interactive_ops;
                let byte = [offset as u8];
                interactive
                    .write_stdin(interactive_token, offset + 1, offset, &byte)
                    .unwrap();
                assert_eq!(
                    interactive.consume_stdin(interactive_token, 1).unwrap(),
                    byte
                );
                interactive
                    .append_output(interactive_token, Stream::Stdout, &byte)
                    .unwrap();
                let response = interactive
                    .read_stdout(interactive_token, offset, 1)
                    .unwrap();
                assert_eq!(response.data, byte);
                interactive
                    .ack_output(interactive_token, Stream::Stdout, offset + 1)
                    .unwrap();
                interactive_ops += 1;
            }

            turn += 1;
            if slow_offset < 2 * MIB as u64 {
                let response = slow_output
                    .read_stdout(slow_token, slow_offset, 32 * KIB)
                    .unwrap();
                assert_eq!(
                    response.data,
                    pattern_chunk(Stream::Stdout, slow_offset, response.data.len())
                );
                slow_offset += response.data.len() as u64;
                slow_output
                    .ack_output(slow_token, Stream::Stdout, slow_offset)
                    .unwrap();
            }

            turn += 1;
            if blocked_write_id <= 64 {
                let chunk = stdin_pattern(blocked_offset, 16 * KIB);
                match blocked_stdin.write_stdin(
                    blocked_token,
                    blocked_write_id,
                    blocked_offset,
                    &chunk,
                ) {
                    Ok(_) => blocked_offset += chunk.len() as u64,
                    Err(ProtocolError::SlowConsumer { .. }) => blocked_slow += 1,
                    Err(err) => panic!("unexpected stdin result: {err:?}"),
                }
                blocked_write_id += 1;
            }
        }

        assert_eq!(slow_offset, 2 * MIB as u64);
        assert!(blocked_slow > 0);
        assert_eq!(interactive_ops, INTERACTIVE_OPS);
        assert_eq!(health_ops, HEALTH_OPS);
        assert!(max_interactive_gap <= MAX_TURN_GAP);
        assert!(max_health_gap <= MAX_TURN_GAP);
    }

    #[test]
    fn receive_cap_and_effective_chunk_limit_are_distinct() {
        let max_recv = MIB + 16 * KIB;
        assert_eq!(
            ChunkedSession::receive_message(max_recv, max_recv)
                .unwrap()
                .len(),
            max_recv
        );
        assert_eq!(
            ChunkedSession::receive_message(max_recv, max_recv + 1),
            Err(ProtocolError::ReceiveMessageTooLarge)
        );

        let mut session = ChunkedSession::new(7, MIB, 2 * MIB, 64 * KIB);
        let token = session.token();
        assert_eq!(
            session.write_stdin(token, 1, 0, b""),
            Err(ProtocolError::EmptyStdinData)
        );
        assert_eq!(session.stdin_next_offset(), 0);
        assert_eq!(session.stdin_dedupe_entries(), 0);

        let bounded_but_too_large = vec![0u8; 64 * KIB + 1];
        assert_eq!(
            session.write_stdin(token, 1, 0, &bounded_but_too_large),
            Err(ProtocolError::ChunkTooLarge)
        );
        assert_eq!(session.stdin_next_offset(), 0);
        assert_eq!(session.stdin_buffered_bytes(), 0);
    }

    #[test]
    fn decoded_byte_budget_and_stdin_permit_bound_concurrent_write_fan_in() {
        let max_recv = MIB;
        let mut budget = DecodedByteBudget::new(4 * max_recv);
        for _ in 0..4 {
            budget.try_acquire(max_recv).unwrap();
        }
        assert_eq!(budget.used(), 4 * max_recv);
        assert_eq!(
            budget.try_acquire(1),
            Err(ProtocolError::DecodeBudgetExhausted {
                used: 4 * max_recv,
                requested: 1,
                cap: 4 * max_recv,
            })
        );
        budget.release(max_recv);
        budget.try_acquire(max_recv).unwrap();
        assert_eq!(budget.used(), 4 * max_recv);

        let mut session = ChunkedSession::new(8, MIB, 2 * max_recv, 64 * KIB);
        let token = session.token();
        session.begin_stdin_request(token).unwrap();
        assert_eq!(
            session.begin_stdin_request(token),
            Err(ProtocolError::StdinRequestInFlight)
        );
        let first = stdin_pattern(0, 64 * KIB);
        assert_eq!(
            session.write_stdin(token, 1, 0, &first),
            Err(ProtocolError::StdinRequestInFlight)
        );
        assert_eq!(session.stdin_next_offset(), 0);
        assert_eq!(session.stdin_buffered_bytes(), 0);
        assert_eq!(session.stdin_dedupe_entries(), 0);
        session.finish_stdin_request(token).unwrap();

        session.write_stdin(token, 1, 0, &first).unwrap();

        session.begin_stdin_request(token).unwrap();
        assert_eq!(
            session.write_stdin(token, 2, 64 * KIB as u64, &first),
            Err(ProtocolError::StdinRequestInFlight)
        );
        assert_eq!(
            session.close_stdin(token, 99, 64 * KIB as u64),
            Err(ProtocolError::StdinRequestInFlight)
        );
        session.finish_stdin_request(token).unwrap();
        let second = stdin_pattern(64 * KIB as u64, 64 * KIB);
        session
            .write_stdin(token, 2, 64 * KIB as u64, &second)
            .unwrap();
        assert_eq!(session.stdin_next_offset(), 128 * KIB as u64);
        assert_eq!(
            session.consume_stdin(token, 128 * KIB).unwrap().len(),
            128 * KIB
        );
        assert_eq!(session.stdin_buffered_bytes(), 0);
    }

    #[test]
    fn stdin_dedupe_metadata_is_bounded() {
        let mut session = ChunkedSession::new(9, MIB, 2 * MAX_DEDUPE_ENTRIES, 1);
        let token = session.token();
        for idx in 0..(MAX_DEDUPE_ENTRIES + 128) {
            let data = [(idx % 251) as u8];
            session
                .write_stdin(token, idx as u64 + 1, idx as u64, &data)
                .unwrap();
            assert!(session.stdin_dedupe_entries() <= MAX_DEDUPE_ENTRIES);
        }
        assert_eq!(
            session.stdin_next_offset(),
            (MAX_DEDUPE_ENTRIES + 128) as u64
        );
        assert!(session.stdin_dedupe_entries() <= MAX_DEDUPE_ENTRIES);
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
    fn restart_releases_stale_stdin_permit() {
        let mut session = ChunkedSession::new(10, MIB, MIB, 64 * KIB);
        let old = session.token();
        session.begin_stdin_request(old).unwrap();
        let new = session.restart();
        assert_eq!(
            session.finish_stdin_request(old),
            Err(ProtocolError::StaleSession)
        );
        assert_eq!(
            session.write_stdin(new, 1, 0, b"x"),
            Ok(WriteStdinResponse {
                next_offset: 1,
                disposition: WriteDisposition::Accepted,
            })
        );
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
            session.close_stdin(token, 99, 1).unwrap().disposition,
            WriteDisposition::Accepted
        );
        assert_eq!(
            session.close_stdin(token, 99, 1).unwrap().disposition,
            WriteDisposition::Duplicate
        );
        assert_eq!(
            session.close_stdin(token, 99, 2),
            Err(ProtocolError::RequestIdConflict)
        );
        assert_eq!(
            session.close_stdin(token, 100, 1),
            Err(ProtocolError::StdinClosed)
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
        assert_eq!(
            session.push_control(
                token,
                10,
                1,
                ControlEvent::Resize(WindowSize {
                    rows: 40,
                    cols: 120,
                }),
            ),
            Ok(WriteDisposition::Accepted)
        );
        assert_eq!(
            session.push_control(
                token,
                10,
                1,
                ControlEvent::Resize(WindowSize {
                    rows: 40,
                    cols: 120,
                }),
            ),
            Ok(WriteDisposition::Duplicate)
        );
        assert_eq!(
            session.push_control(
                token,
                10,
                1,
                ControlEvent::Resize(WindowSize {
                    rows: 41,
                    cols: 120
                }),
            ),
            Err(ProtocolError::RequestIdConflict)
        );
        assert_eq!(
            session.push_control(token, 11, 2, ControlEvent::Signal(Signal::Int)),
            Ok(WriteDisposition::Accepted)
        );
        assert_eq!(
            session.push_control(token, 11, 2, ControlEvent::Signal(Signal::Int)),
            Ok(WriteDisposition::Duplicate)
        );
        assert_eq!(
            session.push_control(token, 11, 2, ControlEvent::Signal(Signal::Term)),
            Err(ProtocolError::RequestIdConflict)
        );
        assert_eq!(
            session.push_control(token, 12, 3, ControlEvent::Cancel),
            Ok(WriteDisposition::Accepted)
        );
        assert_eq!(
            session.push_control(token, 12, 3, ControlEvent::Cancel),
            Ok(WriteDisposition::Duplicate)
        );
        assert_eq!(
            session.push_control(
                token,
                12,
                3,
                ControlEvent::Resize(WindowSize { rows: 1, cols: 1 }),
            ),
            Err(ProtocolError::RequestIdConflict)
        );
        session
            .append_output(token, Stream::Stdout, b"tail")
            .unwrap();
        session
            .set_terminal_status(token, ExitStatus::from_signal(Signal::Int))
            .unwrap();
        assert_eq!(
            session.visible_terminal_status_after_output_drain(),
            None,
            "terminal status must not be visible while output remains retained"
        );
        assert_eq!(
            session.push_control(
                token,
                13,
                5,
                ControlEvent::Resize(WindowSize { rows: 24, cols: 80 })
            ),
            Err(ProtocolError::ControlSequenceGap {
                expected: 4,
                got: 5
            })
        );
        assert_eq!(
            session.events(),
            &[
                OrderedControlEvent {
                    seq: 1,
                    event: ControlEvent::Resize(WindowSize {
                        rows: 40,
                        cols: 120
                    }),
                },
                OrderedControlEvent {
                    seq: 2,
                    event: ControlEvent::Signal(Signal::Int),
                },
                OrderedControlEvent {
                    seq: 3,
                    event: ControlEvent::Cancel,
                },
            ]
        );
        assert_eq!(
            session.terminal_status(),
            Some(ExitStatus::Signal {
                signal: Signal::Int,
                status_code: 130,
            })
        );
        session.ack_output(token, Stream::Stdout, 4).unwrap();
        assert_eq!(
            session.visible_terminal_status_after_output_drain(),
            Some(ExitStatus::Signal {
                signal: Signal::Int,
                status_code: 130,
            })
        );
    }
}
