use crate::terminal_session::{
    OutputStreamSel, ReadOutputOutcome, TerminalBackend, TerminalKind, WaitOutcome,
    WriteStdinOutcome,
};
use async_trait::async_trait;
use d2b_contracts::{
    terminal_wire::{TerminalStatus, TerminalStream},
    unsafe_local_wire::{
        HelperFailureCode, HelperTerminalAttachmentClosed, HelperTerminalChunkBase64,
        HelperTerminalControl, HelperTerminalReadOutput, HelperTerminalRequest,
        HelperTerminalResize, HelperTerminalResponse, HelperTerminalWait, HelperTerminalWriteStdin,
        MAX_UNSAFE_LOCAL_TERMINAL_CHUNK_BYTES, MAX_UNSAFE_LOCAL_TERMINAL_FRAME_SIZE,
        decode_unsafe_local_terminal_frame, encode_unsafe_local_terminal_frame,
    },
};
use parking_lot::Mutex;
use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    os::fd::OwnedFd,
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

const MAX_PENDING_TERMINAL_REQUESTS: usize = 32;
const MAX_ABANDONED_TERMINAL_REQUESTS: usize = MAX_PENDING_TERMINAL_REQUESTS * 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsafeLocalTerminalError {
    Bounds,
    Capacity,
    Timeout,
    Closed,
    Protocol,
    ResponseMismatch,
    OutputGap,
    OffsetMismatch,
    InvalidSize,
    Unsupported,
    Rejected(HelperFailureCode),
}

type PendingSender = mpsc::SyncSender<Result<HelperTerminalResponse, UnsafeLocalTerminalError>>;

struct ClientState {
    pending: Mutex<HashMap<u64, PendingSender>>,
    abandoned: Mutex<VecDeque<u64>>,
    abandoned_evicted_through: AtomicU64,
    closed: AtomicBool,
}

impl ClientState {
    fn close(&self, error: UnsafeLocalTerminalError) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let pending = std::mem::take(&mut *self.pending.lock());
        for (_, sender) in pending {
            let _ = sender.try_send(Err(error));
        }
    }

    fn abandon(&self, request_id: u64) {
        let mut abandoned = self.abandoned.lock();
        if abandoned.len() >= MAX_ABANDONED_TERMINAL_REQUESTS
            && let Some(evicted) = abandoned.pop_front()
        {
            self.abandoned_evicted_through
                .fetch_max(evicted, Ordering::AcqRel);
        }
        abandoned.push_back(request_id);
    }

    fn consume_abandoned(&self, request_id: u64) -> bool {
        let mut abandoned = self.abandoned.lock();
        let Some(index) = abandoned.iter().position(|value| *value == request_id) else {
            return false;
        };
        abandoned.remove(index);
        true
    }

    fn was_evicted_abandoned(&self, request_id: u64) -> bool {
        request_id <= self.abandoned_evicted_through.load(Ordering::Acquire)
    }
}

pub struct UnsafeLocalTerminalClient {
    writer: Mutex<UnixStream>,
    state: Arc<ClientState>,
    next_request_id: AtomicU64,
}

impl std::fmt::Debug for UnsafeLocalTerminalClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnsafeLocalTerminalClient")
            .field("closed", &self.state.closed.load(Ordering::Acquire))
            .field("pending_count", &self.state.pending.lock().len())
            .finish()
    }
}

impl UnsafeLocalTerminalClient {
    pub fn new(fd: OwnedFd) -> Result<Self, UnsafeLocalTerminalError> {
        let stream = UnixStream::from(fd);
        stream
            .set_read_timeout(None)
            .and_then(|()| stream.set_write_timeout(None))
            .map_err(|_| UnsafeLocalTerminalError::Closed)?;
        let reader = stream
            .try_clone()
            .map_err(|_| UnsafeLocalTerminalError::Closed)?;
        let state = Arc::new(ClientState {
            pending: Mutex::new(HashMap::new()),
            abandoned: Mutex::new(VecDeque::new()),
            abandoned_evicted_through: AtomicU64::new(0),
            closed: AtomicBool::new(false),
        });
        let reader_state = Arc::clone(&state);
        std::thread::Builder::new()
            .name("d2b-unsafe-local-terminal-reader".to_owned())
            .spawn(move || terminal_reader(reader, reader_state))
            .map_err(|_| UnsafeLocalTerminalError::Closed)?;
        Ok(Self {
            writer: Mutex::new(stream),
            state,
            next_request_id: AtomicU64::new(1),
        })
    }

    fn next_request_id(&self) -> u64 {
        loop {
            let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
            if request_id != 0 {
                return request_id;
            }
        }
    }

    fn round_trip(
        &self,
        build: impl FnOnce(u64) -> Result<HelperTerminalRequest, UnsafeLocalTerminalError>,
        timeout: Duration,
    ) -> Result<HelperTerminalResponse, UnsafeLocalTerminalError> {
        if self.state.closed.load(Ordering::Acquire) {
            return Err(UnsafeLocalTerminalError::Closed);
        }
        let request_id = self.next_request_id();
        let request = build(request_id)?;
        request.validate_bounds().map_err(map_helper_failure_code)?;
        let frame = encode_unsafe_local_terminal_frame(&request)
            .map_err(|_| UnsafeLocalTerminalError::Bounds)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        {
            let mut pending = self.state.pending.lock();
            if self.state.closed.load(Ordering::Acquire) {
                return Err(UnsafeLocalTerminalError::Closed);
            }
            if pending.len() >= MAX_PENDING_TERMINAL_REQUESTS {
                return Err(UnsafeLocalTerminalError::Capacity);
            }
            if pending.insert(request_id, sender).is_some() {
                return Err(UnsafeLocalTerminalError::Protocol);
            }
        }

        if self.writer.lock().write_all(&frame).is_err() {
            self.state.pending.lock().remove(&request_id);
            self.state.close(UnsafeLocalTerminalError::Closed);
            let _ = self.writer.lock().shutdown(std::net::Shutdown::Both);
            return Err(UnsafeLocalTerminalError::Closed);
        }
        match receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if self.state.pending.lock().remove(&request_id).is_some() {
                    self.state.abandon(request_id);
                    Err(UnsafeLocalTerminalError::Timeout)
                } else {
                    receiver
                        .recv()
                        .unwrap_or(Err(UnsafeLocalTerminalError::Closed))
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(UnsafeLocalTerminalError::Closed),
        }
    }

    pub fn close_attachment(
        &self,
        control_sequence: u64,
        timeout: Duration,
    ) -> Result<HelperTerminalAttachmentClosed, UnsafeLocalTerminalError> {
        let response = self.round_trip(
            |request_id| {
                Ok(HelperTerminalRequest::CloseAttachment(
                    HelperTerminalControl {
                        request_id,
                        control_sequence,
                    },
                ))
            },
            timeout,
        )?;
        match response {
            HelperTerminalResponse::CloseAttachment(response)
                if response.control_sequence == control_sequence =>
            {
                Ok(response.result)
            }
            HelperTerminalResponse::Rejected(rejected) => {
                Err(map_helper_failure_code(rejected.code))
            }
            _ => Err(UnsafeLocalTerminalError::ResponseMismatch),
        }
    }
}

impl Drop for UnsafeLocalTerminalClient {
    fn drop(&mut self) {
        self.state.close(UnsafeLocalTerminalError::Closed);
        let _ = self.writer.get_mut().shutdown(std::net::Shutdown::Both);
    }
}

fn terminal_reader(mut stream: UnixStream, state: Arc<ClientState>) {
    loop {
        let response = match read_response(&mut stream) {
            Ok(response) => response,
            Err(error) => {
                state.close(error);
                let _ = stream.shutdown(std::net::Shutdown::Both);
                return;
            }
        };
        let request_id = response.request_id();
        if let Some(sender) = state.pending.lock().remove(&request_id) {
            let _ = sender.try_send(Ok(response));
        } else if !state.consume_abandoned(request_id) && !state.was_evicted_abandoned(request_id) {
            state.close(UnsafeLocalTerminalError::Protocol);
            let _ = stream.shutdown(std::net::Shutdown::Both);
            return;
        }
    }
}

fn read_response(
    stream: &mut UnixStream,
) -> Result<HelperTerminalResponse, UnsafeLocalTerminalError> {
    let mut prefix = [0u8; 4];
    stream
        .read_exact(&mut prefix)
        .map_err(|_| UnsafeLocalTerminalError::Closed)?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length == 0 || length > MAX_UNSAFE_LOCAL_TERMINAL_FRAME_SIZE {
        return Err(UnsafeLocalTerminalError::Bounds);
    }
    let mut frame = Vec::with_capacity(length + 4);
    frame.extend_from_slice(&prefix);
    frame.resize(length + 4, 0);
    stream
        .read_exact(&mut frame[4..])
        .map_err(|_| UnsafeLocalTerminalError::Protocol)?;
    decode_unsafe_local_terminal_frame(&frame).map_err(|_| UnsafeLocalTerminalError::Protocol)
}

fn map_helper_failure_code(code: HelperFailureCode) -> UnsafeLocalTerminalError {
    match code {
        HelperFailureCode::QueueFull => UnsafeLocalTerminalError::Capacity,
        HelperFailureCode::Timeout => UnsafeLocalTerminalError::Timeout,
        HelperFailureCode::TerminalOutputGap => UnsafeLocalTerminalError::OutputGap,
        HelperFailureCode::TerminalOffsetMismatch => UnsafeLocalTerminalError::OffsetMismatch,
        HelperFailureCode::TerminalClosed => UnsafeLocalTerminalError::Closed,
        HelperFailureCode::InvalidTerminalSize => UnsafeLocalTerminalError::InvalidSize,
        HelperFailureCode::InvalidRequest => UnsafeLocalTerminalError::Bounds,
        other => UnsafeLocalTerminalError::Rejected(other),
    }
}

fn response_or_rejection(
    response: HelperTerminalResponse,
) -> Result<HelperTerminalResponse, UnsafeLocalTerminalError> {
    if let HelperTerminalResponse::Rejected(rejected) = response {
        Err(map_helper_failure_code(rejected.code))
    } else {
        Ok(response)
    }
}

#[async_trait]
impl TerminalBackend for UnsafeLocalTerminalClient {
    type Error = UnsafeLocalTerminalError;

    async fn write_stdin(
        &self,
        offset: u64,
        data: Vec<u8>,
        eof: bool,
        timeout: Duration,
    ) -> Result<WriteStdinOutcome, Self::Error> {
        if data.len() as u64 > MAX_UNSAFE_LOCAL_TERMINAL_CHUNK_BYTES {
            return Err(UnsafeLocalTerminalError::Bounds);
        }
        let encoded = HelperTerminalChunkBase64::new(d2b_core::base64_codec::encode(&data))
            .map_err(map_helper_failure_code)?;
        let response = response_or_rejection(self.round_trip(
            |request_id| {
                Ok(HelperTerminalRequest::WriteStdin(
                    HelperTerminalWriteStdin {
                        request_id,
                        offset,
                        chunk_base64: encoded,
                        eof,
                    },
                ))
            },
            timeout,
        )?)?;
        let HelperTerminalResponse::WriteStdin(response) = response else {
            return Err(UnsafeLocalTerminalError::ResponseMismatch);
        };
        if response.result.accepted_len > data.len() as u64
            || offset.checked_add(response.result.accepted_len) != Some(response.result.next_offset)
        {
            return Err(UnsafeLocalTerminalError::OffsetMismatch);
        }
        Ok(WriteStdinOutcome {
            accepted_len: response.result.accepted_len,
            next_offset: response.result.next_offset,
            backpressured: response.result.backpressured,
            stdin_closed: response.result.stdin_closed,
        })
    }

    async fn read_output(
        &self,
        stream: OutputStreamSel,
        offset: u64,
        max_len: u64,
        wait: bool,
        timeout_ms: u64,
        timeout: Duration,
    ) -> Result<ReadOutputOutcome, Self::Error> {
        if max_len == 0 || max_len > MAX_UNSAFE_LOCAL_TERMINAL_CHUNK_BYTES {
            return Err(UnsafeLocalTerminalError::Bounds);
        }
        let response = response_or_rejection(self.round_trip(
            |request_id| {
                Ok(HelperTerminalRequest::ReadOutput(
                    HelperTerminalReadOutput {
                        request_id,
                        stream: match stream {
                            OutputStreamSel::Stdout => TerminalStream::Stdout,
                            OutputStreamSel::Stderr => TerminalStream::Stderr,
                        },
                        cursor: offset,
                        max_len,
                        wait,
                        timeout_ms,
                    },
                ))
            },
            timeout,
        )?)?;
        let HelperTerminalResponse::ReadOutput(response) = response else {
            return Err(UnsafeLocalTerminalError::ResponseMismatch);
        };
        let data = d2b_core::base64_codec::decode(response.result.data_base64.as_str())
            .map_err(|_| UnsafeLocalTerminalError::Protocol)?;
        let expected_next = offset
            .checked_add(response.result.dropped_bytes)
            .and_then(|cursor| cursor.checked_add(data.len() as u64))
            .ok_or(UnsafeLocalTerminalError::Protocol)?;
        if data.len() as u64 > max_len || response.result.next_cursor != expected_next {
            return Err(if response.result.dropped_bytes > 0 {
                UnsafeLocalTerminalError::OutputGap
            } else {
                UnsafeLocalTerminalError::OffsetMismatch
            });
        }
        Ok(ReadOutputOutcome {
            data,
            next_offset: response.result.next_cursor,
            eof: response.result.eof,
            dropped_bytes: response.result.dropped_bytes,
            truncated: response.result.truncated,
            timed_out: response.result.timed_out,
        })
    }

    async fn signal(
        &self,
        _control_seq: u64,
        _signo: u32,
        _timeout: Duration,
    ) -> Result<(), Self::Error> {
        Err(UnsafeLocalTerminalError::Unsupported)
    }

    async fn resize(
        &self,
        control_seq: u64,
        rows: u32,
        cols: u32,
        timeout: Duration,
    ) -> Result<(), Self::Error> {
        let response = response_or_rejection(self.round_trip(
            |request_id| {
                Ok(HelperTerminalRequest::Resize(HelperTerminalResize {
                    request_id,
                    control_sequence: control_seq,
                    rows,
                    cols,
                }))
            },
            timeout,
        )?)?;
        match response {
            HelperTerminalResponse::Resize(response)
                if response.control_sequence == control_seq && response.result.delivered =>
            {
                Ok(())
            }
            _ => Err(UnsafeLocalTerminalError::ResponseMismatch),
        }
    }

    async fn wait(&self, timeout_ms: u64, timeout: Duration) -> Result<WaitOutcome, Self::Error> {
        let response = response_or_rejection(self.round_trip(
            |request_id| {
                Ok(HelperTerminalRequest::Wait(HelperTerminalWait {
                    request_id,
                    timeout_ms,
                }))
            },
            timeout,
        )?)?;
        let HelperTerminalResponse::Wait(response) = response else {
            return Err(UnsafeLocalTerminalError::ResponseMismatch);
        };
        Ok(WaitOutcome {
            running: response.result.running,
            terminal: response.result.terminal_status.map(|status| match status {
                TerminalStatus::Exited { code } => TerminalKind::Exited(code),
                TerminalStatus::Signaled { signal } => TerminalKind::Signaled(signal),
                TerminalStatus::Error { .. } => TerminalKind::Error("terminal-error"),
            }),
        })
    }

    async fn close_stdin(&self, control_seq: u64, timeout: Duration) -> Result<(), Self::Error> {
        let response = response_or_rejection(self.round_trip(
            |request_id| {
                Ok(HelperTerminalRequest::CloseStdin(HelperTerminalControl {
                    request_id,
                    control_sequence: control_seq,
                }))
            },
            timeout,
        )?)?;
        match response {
            HelperTerminalResponse::CloseStdin(response)
                if response.control_sequence == control_seq && response.result.stdin_closed =>
            {
                Ok(())
            }
            _ => Err(UnsafeLocalTerminalError::ResponseMismatch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::{
        terminal_wire::{TerminalControlResult, TerminalWriteStdinResult},
        unsafe_local_wire::{
            HelperTerminalControlResponse, HelperTerminalOperationResult,
            HelperTerminalReadOutputResult,
        },
    };

    fn client_pair() -> (Arc<UnsafeLocalTerminalClient>, UnixStream) {
        let (client, peer) = UnixStream::pair().unwrap();
        (
            Arc::new(UnsafeLocalTerminalClient::new(client.into()).unwrap()),
            peer,
        )
    }

    fn read_request(peer: &mut UnixStream) -> HelperTerminalRequest {
        let mut prefix = [0u8; 4];
        peer.read_exact(&mut prefix).unwrap();
        let length = u32::from_le_bytes(prefix) as usize;
        let mut frame = Vec::from(prefix);
        frame.resize(length + 4, 0);
        peer.read_exact(&mut frame[4..]).unwrap();
        decode_unsafe_local_terminal_frame(&frame).unwrap()
    }

    fn write_response(peer: &mut UnixStream, response: &HelperTerminalResponse) {
        peer.write_all(&encode_unsafe_local_terminal_frame(response).unwrap())
            .unwrap();
    }

    #[test]
    fn debug_is_redacted_and_reader_close_wakes_pending() {
        let (client, peer) = client_pair();
        assert!(!format!("{client:?}").contains("fd"));
        drop(peer);
        let error = futures_lite_block_on(client.wait(0, Duration::from_secs(1))).unwrap_err();
        assert_eq!(error, UnsafeLocalTerminalError::Closed);
    }

    #[test]
    fn multiplexes_out_of_order_write_and_resize_replies() {
        let (client, mut peer) = client_pair();
        let write_client = Arc::clone(&client);
        let write = std::thread::spawn(move || {
            futures_lite_block_on(write_client.write_stdin(
                0,
                b"abc".to_vec(),
                false,
                Duration::from_secs(2),
            ))
        });
        let resize_client = Arc::clone(&client);
        let resize = std::thread::spawn(move || {
            futures_lite_block_on(resize_client.resize(7, 40, 120, Duration::from_secs(2)))
        });
        let first = read_request(&mut peer);
        let second = read_request(&mut peer);
        let (write_id, resize_id) = match (&first, &second) {
            (HelperTerminalRequest::WriteStdin(write), HelperTerminalRequest::Resize(resize)) => {
                (write.request_id, resize.request_id)
            }
            (HelperTerminalRequest::Resize(resize), HelperTerminalRequest::WriteStdin(write)) => {
                (write.request_id, resize.request_id)
            }
            _ => panic!("unexpected requests"),
        };
        write_response(
            &mut peer,
            &HelperTerminalResponse::Resize(HelperTerminalControlResponse {
                request_id: resize_id,
                control_sequence: 7,
                result: TerminalControlResult { delivered: true },
            }),
        );
        write_response(
            &mut peer,
            &HelperTerminalResponse::WriteStdin(HelperTerminalOperationResult {
                request_id: write_id,
                result: TerminalWriteStdinResult {
                    accepted_len: 3,
                    next_offset: 3,
                    backpressured: false,
                    stdin_closed: false,
                },
            }),
        );
        assert!(resize.join().unwrap().is_ok());
        assert_eq!(write.join().unwrap().unwrap().next_offset, 3);
    }

    #[test]
    fn output_cursor_and_gap_are_validated() {
        let (client, mut peer) = client_pair();
        let reader_client = Arc::clone(&client);
        let read = std::thread::spawn(move || {
            futures_lite_block_on(reader_client.read_output(
                OutputStreamSel::Stdout,
                4,
                32,
                false,
                0,
                Duration::from_secs(2),
            ))
        });
        let request = read_request(&mut peer);
        let request_id = request.request_id();
        write_response(
            &mut peer,
            &HelperTerminalResponse::ReadOutput(HelperTerminalOperationResult {
                request_id,
                result: HelperTerminalReadOutputResult {
                    data_base64: HelperTerminalChunkBase64::new(d2b_core::base64_codec::encode(
                        b"ok",
                    ))
                    .unwrap(),
                    next_cursor: 9,
                    eof: false,
                    dropped_bytes: 3,
                    truncated: true,
                    timed_out: false,
                },
            }),
        );
        let outcome = read.join().unwrap().unwrap();
        assert_eq!(outcome.data, b"ok");
        assert_eq!(outcome.dropped_bytes, 3);
        assert_eq!(outcome.next_offset, 9);
    }

    #[test]
    fn concurrent_long_read_does_not_block_stdin_write() {
        let (client, mut peer) = client_pair();
        let read_client = Arc::clone(&client);
        let read = std::thread::spawn(move || {
            futures_lite_block_on(read_client.read_output(
                OutputStreamSel::Stdout,
                0,
                32,
                true,
                1_000,
                Duration::from_secs(2),
            ))
        });
        let write_client = Arc::clone(&client);
        let write = std::thread::spawn(move || {
            futures_lite_block_on(write_client.write_stdin(
                0,
                b"x".to_vec(),
                false,
                Duration::from_secs(2),
            ))
        });
        let first = read_request(&mut peer);
        let second = read_request(&mut peer);
        let mut read_id = None;
        let mut write_id = None;
        for request in [first, second] {
            match request {
                HelperTerminalRequest::ReadOutput(value) => read_id = Some(value.request_id),
                HelperTerminalRequest::WriteStdin(value) => write_id = Some(value.request_id),
                _ => panic!("unexpected terminal request"),
            }
        }
        write_response(
            &mut peer,
            &HelperTerminalResponse::WriteStdin(HelperTerminalOperationResult {
                request_id: write_id.unwrap(),
                result: TerminalWriteStdinResult {
                    accepted_len: 1,
                    next_offset: 1,
                    backpressured: false,
                    stdin_closed: false,
                },
            }),
        );
        assert_eq!(write.join().unwrap().unwrap().next_offset, 1);
        write_response(
            &mut peer,
            &HelperTerminalResponse::ReadOutput(HelperTerminalOperationResult {
                request_id: read_id.unwrap(),
                result: HelperTerminalReadOutputResult {
                    data_base64: HelperTerminalChunkBase64::new("").unwrap(),
                    next_cursor: 0,
                    eof: false,
                    dropped_bytes: 0,
                    truncated: false,
                    timed_out: true,
                },
            }),
        );
        assert!(read.join().unwrap().unwrap().timed_out);
    }

    #[test]
    fn malformed_reader_frame_wakes_pending_and_bounds_fail_before_write() {
        let (client, mut peer) = client_pair();
        assert_eq!(
            futures_lite_block_on(client.resize(1, 0, 80, Duration::from_millis(10))),
            Err(UnsafeLocalTerminalError::InvalidSize)
        );

        let wait_client = Arc::clone(&client);
        let wait = std::thread::spawn(move || {
            futures_lite_block_on(wait_client.wait(100, Duration::from_secs(2)))
        });
        let _ = read_request(&mut peer);
        peer.write_all(&[1, 0, 0, 0, b'{']).unwrap();
        assert_eq!(
            wait.join().unwrap(),
            Err(UnsafeLocalTerminalError::Protocol)
        );
    }

    #[test]
    fn response_for_evicted_abandoned_id_does_not_close_session() {
        let (client, mut peer) = client_pair();
        for request_id in 1..=(MAX_ABANDONED_TERMINAL_REQUESTS as u64 + 1) {
            client.state.abandon(request_id);
        }
        write_response(
            &mut peer,
            &HelperTerminalResponse::Wait(HelperTerminalOperationResult {
                request_id: 1,
                result: d2b_contracts::terminal_wire::TerminalWaitResult {
                    running: true,
                    terminal_status: None,
                },
            }),
        );
        std::thread::sleep(Duration::from_millis(20));
        assert!(!client.state.closed.load(Ordering::Acquire));

        let next_client = Arc::clone(&client);
        let next = std::thread::spawn(move || {
            futures_lite_block_on(next_client.wait(0, Duration::from_secs(2)))
        });
        let request = read_request(&mut peer);
        write_response(
            &mut peer,
            &HelperTerminalResponse::Wait(HelperTerminalOperationResult {
                request_id: request.request_id(),
                result: d2b_contracts::terminal_wire::TerminalWaitResult {
                    running: true,
                    terminal_status: None,
                },
            }),
        );
        assert!(next.join().unwrap().unwrap().running);
    }

    fn futures_lite_block_on<F: std::future::Future>(future: F) -> F::Output {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(future)
    }
}
