//! Shared CLI-side terminal seams.
//!
//! The exec client is currently the only consumer. These traits split the
//! terminal FSM from exec-specific public wire envelopes so future interactive
//! adapters can drive the same host terminal machinery without copying it.

use std::io;

/// One owner-connection terminal round trip.
pub trait TerminalTransport {
    type Op;
    type Response;
    type Error;

    fn round_trip(&mut self, op: &Self::Op) -> Result<Self::Response, Self::Error>;
}

/// Host-side terminal I/O used by an attached terminal FSM.
pub trait TerminalHostIo {
    /// Read available stdin bytes. Implementations must be non-blocking and
    /// return `WouldBlock` when no bytes are ready.
    fn read_stdin(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    fn write_stdout(&mut self, data: &[u8]) -> io::Result<()>;
    fn write_stderr(&mut self, data: &[u8]) -> io::Result<()>;
    fn window_size(&self) -> Option<(u32, u32)>;
}

/// Host signal/event source used by an attached terminal FSM.
pub trait TerminalSignalSource {
    type Signal;

    fn drain(&mut self) -> Vec<Self::Signal>;
}
