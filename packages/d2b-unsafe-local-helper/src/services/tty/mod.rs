//! Authenticated TTY helper service and frozen composition keys.

pub use crate::tty_exec::{
    CancelOutcome, TransientUserScope, TtyOneShotError, TtyOneShotRequest, TtyOneShotRuntime,
    TtyOneShotService, TtyOneShotSpec, ValidatedTerminal,
};

pub const SERVICE_PACKAGE: &str = "d2b.tty.v2";
pub const ENDPOINT_PURPOSE: &str = "tty-helper";
pub const ENDPOINT_ROLE: &str = "tty-helper";
