//! `d2b-agentterm` -- an agent-drivable terminal.
//!
//! A PTY wrapper that passes a terminal through to a human unchanged while
//! maintaining a headless VT emulator that an agent can read and drive over a
//! unix socket.
//!
//! See `DESIGN.md` for the architecture and `README.md` for usage.

pub mod cli;
pub mod delta;
pub mod history;
pub mod keys;
pub mod modes;
pub mod protocol;
pub mod pump;
pub mod screen;
pub mod server;
pub mod session;
pub mod utf8;

// The two modules permitted to use `unsafe`, quarantined here so the exemption
// is visible in one place rather than scattered across the tree.
//
// `pty` needs forkpty/execvpe and the TIOCSWINSZ ioctl; `tty` needs the
// TIOCGWINSZ ioctl and raw read/write against a registered descriptor. The
// crate-level lint is `deny` rather than `forbid` precisely so these two can
// opt back in; `forbid` cannot be locally overridden.
#[allow(unsafe_code)]
pub mod pty;
#[allow(unsafe_code)]
pub mod tty;
