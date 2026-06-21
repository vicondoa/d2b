//! Shared daemon-side terminal session DTOs and backend seams.
//!
//! Exec remains the only backend in this wave. These types isolate terminal
//! output, stdin, and wait semantics from exec-specific public wire envelopes so
//! later shell work can reuse the same worker vocabulary.

use std::time::Duration;

use async_trait::async_trait;

/// Output stream selector handed to a terminal backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStreamSel {
    Stdout,
    Stderr,
}

/// Outcome of a terminal stdin write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteStdinOutcome {
    pub accepted_len: u64,
    pub next_offset: u64,
    pub backpressured: bool,
    pub stdin_closed: bool,
}

/// Outcome of a terminal output read. `Debug` is redacted so a stray `{:?}` can
/// never leak terminal bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct ReadOutputOutcome {
    pub data: Vec<u8>,
    pub next_offset: u64,
    pub eof: bool,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub timed_out: bool,
}

impl std::fmt::Debug for ReadOutputOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadOutputOutcome")
            .field("data_len", &self.data.len())
            .field("next_offset", &self.next_offset)
            .field("eof", &self.eof)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .field("timed_out", &self.timed_out)
            .finish()
    }
}

/// Terminal disposition of a backend process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalKind {
    Exited(i32),
    Signaled(u32),
    Error(&'static str),
}

/// Outcome of a terminal wait poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitOutcome {
    pub running: bool,
    pub terminal: Option<TerminalKind>,
}

/// Backend seam for one established terminal session. The daemon worker owns
/// scheduling/cancellation; backends own transport-specific RPCs.
#[async_trait]
pub trait TerminalBackend: Send + Sync {
    type Error;

    async fn write_stdin(
        &self,
        offset: u64,
        data: Vec<u8>,
        eof: bool,
        timeout: Duration,
    ) -> Result<WriteStdinOutcome, Self::Error>;

    async fn read_output(
        &self,
        stream: OutputStreamSel,
        offset: u64,
        max_len: u64,
        wait: bool,
        timeout_ms: u64,
        timeout: Duration,
    ) -> Result<ReadOutputOutcome, Self::Error>;

    async fn signal(
        &self,
        control_seq: u64,
        signo: u32,
        timeout: Duration,
    ) -> Result<(), Self::Error>;

    async fn resize(
        &self,
        control_seq: u64,
        rows: u32,
        cols: u32,
        timeout: Duration,
    ) -> Result<(), Self::Error>;

    async fn wait(&self, timeout_ms: u64, timeout: Duration) -> Result<WaitOutcome, Self::Error>;

    async fn close_stdin(&self, offset: u64, timeout: Duration) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::ReadOutputOutcome;
    use super::{OutputStreamSel, TerminalBackend, TerminalKind, WaitOutcome, WriteStdinOutcome};
    use async_trait::async_trait;
    use std::time::Duration;

    #[test]
    fn read_output_debug_redacts_terminal_bytes() {
        let outcome = ReadOutputOutcome {
            data: b"secret-terminal-output".to_vec(),
            next_offset: 22,
            eof: false,
            dropped_bytes: 0,
            truncated: false,
            timed_out: false,
        };
        let rendered = format!("{outcome:?}");
        assert!(!rendered.contains("secret-terminal-output"));
        assert!(rendered.contains("data_len"));
    }

    struct FakeBackend;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeError {
        Closed,
    }

    #[async_trait]
    impl TerminalBackend for FakeBackend {
        type Error = FakeError;

        async fn write_stdin(
            &self,
            offset: u64,
            data: Vec<u8>,
            eof: bool,
            _timeout: Duration,
        ) -> Result<WriteStdinOutcome, Self::Error> {
            Ok(WriteStdinOutcome {
                accepted_len: data.len() as u64,
                next_offset: offset + data.len() as u64,
                backpressured: false,
                stdin_closed: eof,
            })
        }

        async fn read_output(
            &self,
            stream: OutputStreamSel,
            offset: u64,
            _max_len: u64,
            _wait: bool,
            _timeout_ms: u64,
            _timeout: Duration,
        ) -> Result<ReadOutputOutcome, Self::Error> {
            assert_eq!(stream, OutputStreamSel::Stdout);
            Ok(ReadOutputOutcome {
                data: Vec::new(),
                next_offset: offset,
                eof: true,
                dropped_bytes: 0,
                truncated: false,
                timed_out: false,
            })
        }

        async fn signal(
            &self,
            _control_seq: u64,
            _signo: u32,
            _timeout: Duration,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn resize(
            &self,
            _control_seq: u64,
            _rows: u32,
            _cols: u32,
            _timeout: Duration,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn wait(
            &self,
            _timeout_ms: u64,
            _timeout: Duration,
        ) -> Result<WaitOutcome, Self::Error> {
            Ok(WaitOutcome {
                running: false,
                terminal: Some(TerminalKind::Exited(0)),
            })
        }

        async fn close_stdin(&self, _offset: u64, _timeout: Duration) -> Result<(), Self::Error> {
            Err(FakeError::Closed)
        }
    }

    #[tokio::test]
    async fn terminal_backend_trait_covers_shared_ops_without_exec_wire() {
        let backend = FakeBackend;
        let write = backend
            .write_stdin(3, b"abc".to_vec(), true, Duration::from_millis(1))
            .await
            .expect("write succeeds");
        assert_eq!(write.next_offset, 6);
        assert!(write.stdin_closed);

        let read = backend
            .read_output(
                OutputStreamSel::Stdout,
                10,
                1024,
                true,
                50,
                Duration::from_millis(60),
            )
            .await
            .expect("read succeeds");
        assert_eq!(read.next_offset, 10);
        assert!(read.eof);

        assert!(matches!(
            backend
                .wait(0, Duration::from_millis(1))
                .await
                .expect("wait succeeds")
                .terminal,
            Some(TerminalKind::Exited(0))
        ));
        assert_eq!(
            backend
                .close_stdin(6, Duration::from_millis(1))
                .await
                .expect_err("close is scripted failure"),
            FakeError::Closed
        );
    }
}
