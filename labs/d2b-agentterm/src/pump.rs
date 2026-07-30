//! The I/O pump.
//!
//! A single task owns the terminal, the PTY and the session state, so the
//! ordering of everything the child sees is decided in one place.
//!
//! The critical property is that human keystrokes and agent-injected bytes
//! merge into **one** queue. Two independent writers to a PTY master can
//! interleave mid-escape-sequence, which would turn an injected arrow key and a
//! simultaneous keypress into a corrupt third sequence.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

use crate::pty::Pty;
use crate::server::{self, PumpCommand};
use crate::session::Session;
use crate::tty::{RawTty, TtySize};

/// Size of each read from the PTY and the terminal.
const READ_BUF: usize = 64 * 1024;

/// How the session should be started.
#[derive(Debug, Clone)]
pub struct PumpConfig {
    pub command: Vec<String>,
    /// Explicit size, or `None` to follow the attached terminal.
    pub size: Option<TtySize>,
    pub scrollback: usize,
    pub socket: PathBuf,
    /// Print the socket path on startup.
    pub announce: bool,
}

/// Run a session to completion, returning the child's exit status.
pub async fn run(config: PumpConfig) -> anyhow::Result<i32> {
    let tty = RawTty::open()?;

    // Follow the real terminal unless overridden. The human's window is the
    // authority; an explicit --size is a starting point, not a lock.
    let size = match config.size {
        Some(size) => size,
        None => tty.size().unwrap_or(TtySize::new(80, 24)),
    };

    let mut env = HashMap::new();
    env.insert("TERM".to_string(), "xterm-256color".to_string());

    let pty = Pty::spawn(&config.command, size, &env)?;

    let session: server::SharedSession = Arc::new(Mutex::new(Session::new(
        size,
        config.scrollback,
        pty.child_pid(),
    )));

    let listener = server::bind(&config.socket)?;
    let (command_tx, mut command_rx) = mpsc::unbounded_channel::<PumpCommand>();

    let server_task = tokio::spawn(server::serve(
        listener,
        Arc::clone(&session),
        command_tx.clone(),
    ));

    if config.announce {
        // Written to stderr so it cannot corrupt a redirected stdout, and
        // before raw-mode output begins in earnest.
        eprintln!(
            "d2b-agentterm: socket {} ({}x{})\r",
            config.socket.display(),
            size.cols,
            size.rows
        );
    }

    let mut winch = signal(SignalKind::window_change())?;

    // Keeps a fresh delta baseline while the screen is idle. Checkpoints are
    // otherwise only taken when output arrives, so a settled screen would be
    // compared against an ever-older baseline and report changes that already
    // happened long before the requested window.
    let mut checkpoint_tick = tokio::time::interval(crate::history::DEFAULT_CHECKPOINT_INTERVAL);
    checkpoint_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut pty_buf = vec![0u8; READ_BUF];
    let mut tty_buf = vec![0u8; READ_BUF];
    // Bytes queued toward the child, and toward the human's screen.
    let mut to_child: Vec<u8> = Vec::new();
    let mut to_screen: Vec<u8> = Vec::new();

    let exit_status = loop {
        tokio::select! {
            // Child output: to the human verbatim, and to the emulator decoded.
            result = pty.read(&mut pty_buf) => {
                match result {
                    Ok(0) => break pty.wait().unwrap_or(0),
                    Ok(n) => {
                        if let Ok(mut s) = session.lock() {
                            s.feed_output(&pty_buf[..n]);
                        }
                        to_screen.extend_from_slice(&pty_buf[..n]);
                    }
                    Err(err) => {
                        eprintln!("d2b-agentterm: pty read failed: {err}\r");
                        break pty.try_wait().unwrap_or(1);
                    }
                }
            }

            // Human keystrokes.
            result = tty.read(&mut tty_buf) => {
                match result {
                    Ok(0) => {
                        // The terminal closed. Ask the child to finish.
                        pty.hangup();
                        break pty.wait().unwrap_or(0);
                    }
                    Ok(n) => to_child.extend_from_slice(&tty_buf[..n]),
                    Err(err) => {
                        eprintln!("d2b-agentterm: tty read failed: {err}\r");
                        pty.hangup();
                        break pty.wait().unwrap_or(1);
                    }
                }
            }

            // Drain queued input into the child.
            result = pty.write(&to_child), if !to_child.is_empty() => {
                match result {
                    Ok(n) => { to_child.drain(..n); }
                    Err(err) => {
                        eprintln!("d2b-agentterm: pty write failed: {err}\r");
                        break pty.try_wait().unwrap_or(1);
                    }
                }
            }

            // Drain queued output onto the screen.
            result = tty.write(&to_screen), if !to_screen.is_empty() => {
                match result {
                    Ok(n) => { to_screen.drain(..n); }
                    Err(err) => {
                        eprintln!("d2b-agentterm: tty write failed: {err}\r");
                        break pty.try_wait().unwrap_or(1);
                    }
                }
            }

            // Agent requests.
            Some(command) = command_rx.recv() => {
                match command {
                    PumpCommand::Input(bytes) => to_child.extend_from_slice(&bytes),
                    PumpCommand::Resize(size) => apply_resize(&pty, &session, size),
                }
            }

            _ = checkpoint_tick.tick() => {
                if let Ok(mut s) = session.lock() {
                    s.checkpoint_now();
                }
            }

            // The window changed. This is the path ht omits entirely: without
            // the TIOCSWINSZ inside apply_resize the child never learns its new
            // size and a full-screen TUI renders at its startup dimensions
            // forever.
            _ = winch.recv() => {
                if let Ok(size) = tty.size() {
                    apply_resize(&pty, &session, size);
                }
            }
        }
    };

    // Flush whatever the child produced last, so the human's screen matches the
    // emulator's final state.
    if !to_screen.is_empty() {
        let _ = tty.write(&to_screen).await;
    }
    if let Ok(mut s) = session.lock() {
        s.finish_output();
        s.set_exit_status(exit_status);
    }

    server_task.abort();
    let _ = std::fs::remove_file(&config.socket);

    Ok(exit_status)
}

/// Apply a size change to the child, the emulator, and the recorded state.
///
/// Order matters: the kernel first, so the child's `SIGWINCH` handler reads the
/// new size, then the emulator, so a subsequent snapshot agrees with what the
/// child is about to draw.
fn apply_resize(pty: &Pty, session: &server::SharedSession, size: TtySize) {
    if let Err(err) = pty.resize(size) {
        eprintln!("d2b-agentterm: resize failed: {err}\r");
        return;
    }

    if let Ok(mut s) = session.lock() {
        s.resize(size);
    }
}
