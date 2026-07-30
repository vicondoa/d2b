//! The I/O pump: the heart of the program.
//!
//! A single task owns the terminal, the PTY and the session state, so the
//! ordering of everything the child sees is decided in one place.
//!
//! # The one invariant that matters
//!
//! Human keystrokes and agent-injected bytes merge into **one** queue
//! (`to_child`). Two independent writers to a PTY master can interleave
//! mid-escape-sequence: an agent injecting `ESC [ A` while you press a key can
//! produce `ESC [ x A`, which is a different sequence entirely. One queue, one
//! writer, no interleaving.
//!
//! # The second invariant
//!
//! The human's screen receives the child's bytes **verbatim**. Only the
//! emulator sees decoded text. So anything `avt` does not model -- sixel
//! graphics, kitty images, an obscure private sequence -- still renders
//! correctly for you, because it is never round-tripped through the emulator on
//! the way to your screen.

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
///
/// 64 KiB is comfortably larger than a full-screen redraw of a big terminal
/// (a 200x60 screen with attributes is well under 64 KiB), so a single redraw
/// usually arrives in one read and produces one coherent emulator update.
const READ_BUF: usize = 64 * 1024;

/// How the session should be started.
#[derive(Debug, Clone)]
pub struct PumpConfig {
    /// argv of the program to run. Element 0 is resolved through `PATH`.
    pub command: Vec<String>,
    /// Explicit size, or `None` to follow the attached terminal.
    pub size: Option<TtySize>,
    /// Lines of scrollback the emulator retains.
    pub scrollback: usize,
    /// Where to bind the agent socket.
    pub socket: PathBuf,
    /// Print the socket path on startup.
    pub announce: bool,
}

/// Run a session to completion, returning the child's exit status.
pub async fn run(config: PumpConfig) -> anyhow::Result<i32> {
    // Take the terminal into raw mode first. `RawTty` restores it on drop and
    // via a panic hook, so from here on every exit path is covered.
    let tty = RawTty::open()?;

    // Follow the real terminal unless overridden. The human's window is the
    // authority; an explicit --size is a starting point, not a lock, and the
    // first SIGWINCH will override it.
    let size = match config.size {
        Some(size) => size,
        None => tty.size().unwrap_or(TtySize::new(80, 24)),
    };

    // `avt` emulates a broadly xterm-compatible terminal, so tell the child
    // that is what it has. Without this the child inherits whatever TERM the
    // outer terminal advertises, which may promise capabilities the emulator
    // does not implement.
    let mut env = HashMap::new();
    env.insert("TERM".to_string(), "xterm-256color".to_string());

    let pty = Pty::spawn(&config.command, size, &env)?;

    // The session is the agent-visible state. It is shared with the socket
    // server behind a mutex; see the module docs in `server` for why that is
    // safe (the lock is never held across an await).
    let session: server::SharedSession = Arc::new(Mutex::new(Session::new(
        size,
        config.scrollback,
        pty.child_pid(),
    )));

    let listener = server::bind(&config.socket)?;

    // Agent requests that mutate the child arrive here rather than being
    // applied by the socket task directly, which is what keeps the single
    // writer invariant intact.
    let (command_tx, mut command_rx) = mpsc::unbounded_channel::<PumpCommand>();

    let server_task = tokio::spawn(server::serve(
        listener,
        Arc::clone(&session),
        command_tx.clone(),
    ));

    if config.announce {
        // Written to stderr so it cannot corrupt a redirected stdout. The
        // trailing \r is needed because the terminal is already in raw mode,
        // where \n alone moves down without returning to column 0.
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
    // happened long before the requested window. See README part 3.
    let mut checkpoint_tick = tokio::time::interval(crate::history::DEFAULT_CHECKPOINT_INTERVAL);
    // If the loop is busy, skip missed ticks rather than firing a burst of
    // catch-up checkpoints the moment it frees up.
    checkpoint_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Read scratch buffers, reused across iterations to avoid per-read allocation.
    let mut pty_buf = vec![0u8; READ_BUF];
    let mut tty_buf = vec![0u8; READ_BUF];

    // Bytes queued toward the child, and toward the human's screen. These exist
    // because a write may be partially accepted; the remainder stays queued and
    // the corresponding select arm stays enabled until it drains.
    let mut to_child: Vec<u8> = Vec::new();
    let mut to_screen: Vec<u8> = Vec::new();

    let exit_status = loop {
        tokio::select! {
            // --- child output -------------------------------------------
            // Forks two ways: verbatim to the human, decoded to the emulator.
            result = pty.read(&mut pty_buf) => {
                match result {
                    // Zero means the child closed the PTY, i.e. it exited.
                    // `Pty::read` already translates the EIO that a PTY master
                    // reports instead of EOF.
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

            // --- human keystrokes ---------------------------------------
            result = tty.read(&mut tty_buf) => {
                match result {
                    Ok(0) => {
                        // The terminal closed underneath us. Ask the child to
                        // finish rather than leaving it orphaned on a dead PTY.
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

            // --- drain queued input into the child ----------------------
            // The guard keeps this arm disabled while the queue is empty, so
            // select does not spin on a write that has nothing to write.
            result = pty.write(&to_child), if !to_child.is_empty() => {
                match result {
                    // A short write is normal; drain only what was accepted.
                    Ok(n) => { to_child.drain(..n); }
                    Err(err) => {
                        eprintln!("d2b-agentterm: pty write failed: {err}\r");
                        break pty.try_wait().unwrap_or(1);
                    }
                }
            }

            // --- drain queued output onto the screen --------------------
            result = tty.write(&to_screen), if !to_screen.is_empty() => {
                match result {
                    Ok(n) => { to_screen.drain(..n); }
                    Err(err) => {
                        eprintln!("d2b-agentterm: tty write failed: {err}\r");
                        break pty.try_wait().unwrap_or(1);
                    }
                }
            }

            // --- agent requests -----------------------------------------
            // Injected input joins the same queue as human keystrokes, which
            // is the single-writer invariant described in the module docs.
            Some(command) = command_rx.recv() => {
                match command {
                    PumpCommand::Input(bytes) => to_child.extend_from_slice(&bytes),
                    PumpCommand::Resize(size) => apply_resize(&pty, &session, size),
                }
            }

            // --- idle checkpoint ----------------------------------------
            _ = checkpoint_tick.tick() => {
                if let Ok(mut s) = session.lock() {
                    s.checkpoint_now();
                }
            }

            // --- window resize ------------------------------------------
            // This is the path `ht` omits entirely: without the TIOCSWINSZ
            // inside apply_resize the child never learns its new size and a
            // full-screen TUI renders at its startup dimensions forever.
            _ = winch.recv() => {
                if let Ok(size) = tty.size() {
                    apply_resize(&pty, &session, size);
                }
            }
        }
    };

    // Flush whatever the child produced last, so the human's screen matches the
    // emulator's final state rather than losing the last frame.
    if !to_screen.is_empty() {
        let _ = tty.write(&to_screen).await;
    }
    if let Ok(mut s) = session.lock() {
        // Emit any truncated trailing UTF-8 character rather than dropping it.
        s.finish_output();
        s.set_exit_status(exit_status);
    }

    // Stop accepting agent connections and remove the socket file, so a later
    // session at the same path is not blocked by a stale entry.
    server_task.abort();
    let _ = std::fs::remove_file(&config.socket);

    Ok(exit_status)
}

/// Apply a size change to the child, the emulator, and the recorded state.
///
/// Order matters. The kernel goes first, so the child's `SIGWINCH` handler
/// reads the new size when it wakes; then the emulator, so a snapshot taken
/// afterwards agrees with what the child is about to draw. Doing it the other
/// way round leaves a window where the emulator claims a size the child has not
/// been told about yet.
fn apply_resize(pty: &Pty, session: &server::SharedSession, size: TtySize) {
    if let Err(err) = pty.resize(size) {
        // If the kernel refused, do not update the emulator either -- an
        // emulator that disagrees with the child is worse than one that is
        // merely stale.
        eprintln!("d2b-agentterm: resize failed: {err}\r");
        return;
    }

    if let Ok(mut s) = session.lock() {
        s.resize(size);
    }
}
