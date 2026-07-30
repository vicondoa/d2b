//! `d2b-agentterm` -- an agent-drivable terminal.
//!
//! A PTY wrapper that passes a terminal through to a human unchanged while
//! maintaining a headless VT emulator that an agent can read and drive over a
//! unix socket.
//!
//! # How the pieces fit together
//!
//! ```text
//!         your keystrokes                                agent
//!               |                                          |
//!         /dev/tty (raw)                            unix socket
//!          [tty.rs]                                  [server.rs]
//!               |                                          |
//!               v                                          v
//!    +----------------------------------------------------------+
//!    |  the pump  [pump.rs]                                     |
//!    |                                                          |
//!    |    both input sources merge into ONE queue, then go to    |
//!    |    the PTY master  [pty.rs]                              |
//!    |                                                          |
//!    |    child output forks two ways:                          |
//!    |      - verbatim bytes to your screen                     |
//!    |      - decoded text to the emulator  [session.rs]        |
//!    +----------------------------------------------------------+
//! ```
//!
//! # Reading order
//!
//! If you are new to this crate, read the modules in this order:
//!
//! 1. [`pump`] -- the I/O loop. Everything else is called from there.
//! 2. [`session`] -- the state an agent can observe.
//! 3. [`delta`] -- the "what changed" engine, which is where the real design
//!    content lives.
//! 4. [`server`] and [`protocol`] -- how an agent talks to it.
//!
//! The rest ([`tty`], [`pty`], [`utf8`], [`modes`], [`keys`], [`screen`],
//! [`history`]) are supporting pieces that do one thing each.
//!
//! See `README.md` for the full design rationale, including the two bugs found
//! during development that shaped the delta engine.

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
