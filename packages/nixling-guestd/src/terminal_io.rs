//! Shared guest terminal I/O state machines.
//!
//! This module is pure Rust state and validation logic. PTY allocation,
//! fd ownership, process spawning, and teardown remain in `exec_pty`.

use crate::exec::ExecError;

/// Default terminal geometry applied when a TTY create omits an
/// `initial_terminal_size`. A present size must validate (1..=65535); only an
/// absent size defaults.
pub const DEFAULT_TERMINAL_ROWS: u16 = 24;
pub const DEFAULT_TERMINAL_COLS: u16 = 80;
/// Inclusive bounds for a terminal dimension. Matches the existing wire
/// contract (no new schema bound), so a 0 or out-of-range dimension is rejected
/// rather than silently clamped.
pub const MIN_TERMINAL_DIM: u32 = 1;
pub const MAX_TERMINAL_DIM: u32 = 65535;

/// `VEOF` control byte (Ctrl-D). Injected on `CloseStdin` / `WriteStdin`
/// `close_after` to signal end-of-input to the foreground reader while the PTY
/// master stays open (half-close is modelled as VEOF, never a master close).
pub const VEOF: u8 = 0x04;

/// Validated terminal geometry. Both dimensions are within 1..=65535.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

impl TerminalSize {
    /// The default geometry (24x80) used only when an initial size is absent.
    pub const fn defaulted() -> Self {
        Self {
            rows: DEFAULT_TERMINAL_ROWS,
            cols: DEFAULT_TERMINAL_COLS,
        }
    }

    /// Validate a wire-supplied geometry against the existing 1..=65535 bound.
    pub fn checked(rows: u32, cols: u32) -> Result<Self, ExecError> {
        let valid = |d: u32| (MIN_TERMINAL_DIM..=MAX_TERMINAL_DIM).contains(&d);
        if !valid(rows) || !valid(cols) {
            return Err(ExecError::InvalidTerminalSize);
        }
        Ok(Self {
            rows: rows as u16,
            cols: cols as u16,
        })
    }

    /// Resolve an optional initial size: absent defaults to 24x80, present must
    /// validate (a present 0/out-of-range geometry is rejected, never
    /// defaulted).
    pub fn resolve_initial(initial: Option<(u32, u32)>) -> Result<Self, ExecError> {
        match initial {
            None => Ok(Self::defaulted()),
            Some((rows, cols)) => Self::checked(rows, cols),
        }
    }
}

/// Frozen signal allowlist for foreground terminal process groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtySignal {
    Int,
    Term,
    Hup,
    Quit,
    Winch,
    Usr1,
    Usr2,
    Kill,
    Tstp,
    Cont,
}

impl TtySignal {
    /// Map a wire signal number to the allowlist, rejecting any other value.
    pub fn from_raw(signal: u32) -> Option<Self> {
        // Standard Linux signal numbers; the allowlist is frozen in the
        // guest-control exec reference.
        Some(match signal {
            1 => Self::Hup,
            2 => Self::Int,
            3 => Self::Quit,
            9 => Self::Kill,
            10 => Self::Usr1,
            12 => Self::Usr2,
            15 => Self::Term,
            18 => Self::Cont,
            20 => Self::Tstp,
            28 => Self::Winch,
            _ => return None,
        })
    }

    /// The raw Linux signal number for this allowlisted signal.
    pub fn raw(self) -> i32 {
        match self {
            Self::Hup => 1,
            Self::Int => 2,
            Self::Quit => 3,
            Self::Kill => 9,
            Self::Usr1 => 10,
            Self::Usr2 => 12,
            Self::Term => 15,
            Self::Cont => 18,
            Self::Tstp => 20,
            Self::Winch => 28,
        }
    }
}

/// Pure stdin offset machine for a TTY session. WriteStdin must arrive in
/// monotonic, gap-free offset order; a duplicate/out-of-order offset is
/// rejected, and any write after a close (VEOF) is rejected.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StdinLogic {
    next_offset: u64,
    closed: bool,
}

impl StdinLogic {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_offset(&self) -> u64 {
        self.next_offset
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Validate a WriteStdin at `offset`. Rejects writes after close and any
    /// non-contiguous offset. Does NOT advance — call [`advance`](Self::advance)
    /// only after the bytes are durably written to the master.
    pub fn admit(&self, offset: u64) -> Result<(), ExecError> {
        if self.closed {
            return Err(ExecError::StdinClosed);
        }
        if offset != self.next_offset {
            return Err(ExecError::StdinOffsetMismatch);
        }
        Ok(())
    }

    /// Advance the offset cursor after `len` bytes were written.
    pub fn advance(&mut self, len: u64) {
        self.next_offset = self.next_offset.saturating_add(len);
    }

    /// Mark stdin closed (idempotent). Returns true if this call performed the
    /// transition (false if it was already closed).
    pub fn close(&mut self) -> bool {
        if self.closed {
            return false;
        }
        self.closed = true;
        true
    }
}

/// Pure control-seq dispatcher shared by resize AND signal.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ControlSeqState {
    last_seq: u64,
}

impl ControlSeqState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    /// Admit a control message at `seq`, requiring strict monotonic increase.
    pub fn admit(&mut self, seq: u64) -> Result<(), ExecError> {
        if seq <= self.last_seq {
            return Err(ExecError::ControlSeqMismatch);
        }
        self.last_seq = seq;
        Ok(())
    }
}

/// Teardown lifecycle for a TTY session: `Running -> Closing -> Terminal`.
/// Entering `Closing` atomically rejects new stdin/control RPCs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtyPhase {
    Running,
    Closing,
    Terminal,
}

/// Outcome of an accepted `WriteStdin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdinWriteOk {
    pub accepted_len: u64,
    pub next_offset: u64,
    pub closed: bool,
}

#[cfg(test)]
mod tests {
    use super::{ControlSeqState, StdinLogic, TerminalSize, TtySignal};

    #[test]
    fn terminal_size_defaults_and_bounds() {
        assert_eq!(
            TerminalSize::resolve_initial(None).unwrap(),
            TerminalSize::defaulted()
        );
        assert_eq!(TerminalSize::defaulted().rows, 24);
        assert_eq!(TerminalSize::defaulted().cols, 80);
        assert!(TerminalSize::resolve_initial(Some((0, 80))).is_err());
        assert!(TerminalSize::resolve_initial(Some((24, 0))).is_err());
        assert!(TerminalSize::resolve_initial(Some((70000, 80))).is_err());
        let ok = TerminalSize::resolve_initial(Some((40, 120))).unwrap();
        assert_eq!((ok.rows, ok.cols), (40, 120));
    }

    #[test]
    fn stdin_offset_machine_rejects_gaps_and_writes_after_close() {
        let mut logic = StdinLogic::new();
        logic.admit(0).unwrap();
        logic.advance(5);
        assert!(logic.admit(0).is_err());
        assert!(logic.admit(6).is_err());
        logic.admit(5).unwrap();
        assert!(logic.close());
        assert!(!logic.close());
        assert!(logic.admit(5).is_err());
    }

    #[test]
    fn control_seq_requires_strict_monotonic_increase() {
        let mut seq = ControlSeqState::new();
        assert_eq!(seq.last_seq(), 0);
        seq.admit(1).unwrap();
        assert_eq!(seq.last_seq(), 1);
        assert!(seq.admit(1).is_err());
        assert!(seq.admit(0).is_err());
        seq.admit(3).unwrap();
        assert_eq!(seq.last_seq(), 3);
    }

    #[test]
    fn tty_signal_allowlist_is_stable() {
        for raw in [1, 2, 3, 9, 10, 12, 15, 18, 20, 28] {
            let sig = TtySignal::from_raw(raw).expect("allowlisted");
            assert_eq!(sig.raw(), raw as i32);
        }
        assert!(TtySignal::from_raw(11).is_none());
        assert!(TtySignal::from_raw(0).is_none());
    }

    #[test]
    fn debug_output_contains_no_terminal_payload() {
        let ok = super::StdinWriteOk {
            accepted_len: 1,
            next_offset: 1,
            closed: false,
        };
        let rendered = format!("{ok:?}");
        assert!(rendered.contains("accepted_len"));
        assert!(!rendered.contains("secret"));
    }
}
