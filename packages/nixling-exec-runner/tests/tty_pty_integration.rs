//! Real-PTY integration coverage for the `nixling-exec-runner --tty-exec`
//! helper, exercised exactly the way guestd's PTY spawner drives it: allocate a
//! PTY master/slave pair, spawn the helper with the slave on stdin and a
//! `O_CLOEXEC` status pipe on stdout, and assert the controlling-terminal
//! handshake.
//!
//! These assertions are the real-kernel half of the W14 coverage; the guestd
//! runtime state machine (offset machine, control-seq dispatcher, teardown
//! phases) is covered by the fake-driven matrix in `nixling-guestd`.
//!
//! Linux-only: the helper relies on `setsid`/`TIOCSCTTY`/PTY semantics.
#![cfg(target_os = "linux")]

use std::os::fd::OwnedFd;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use rustix::fs::{open, Mode, OFlags};
use rustix::io::{ioctl_fionbio, read};
use rustix::pipe::{pipe_with, PipeFlags};
use rustix::process::{kill_process_group, test_kill_process, Pid, Signal};
use rustix::pty::{grantpt, openpt, ptsname, unlockpt, OpenptFlags};
use rustix::termios::{tcgetsid, tcsetwinsize, Winsize};

/// Allocate a PTY master/slave pair the way the spawner does: master
/// `O_RDWR|O_NOCTTY|O_CLOEXEC`, slave `O_RDWR|O_NOCTTY` (handed to the helper).
fn open_pty() -> (OwnedFd, OwnedFd) {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)
        .expect("openpt master");
    grantpt(&master).expect("grantpt");
    unlockpt(&master).expect("unlockpt");
    let slave_path = ptsname(&master, Vec::new()).expect("ptsname");
    let slave =
        open(&slave_path, OFlags::RDWR | OFlags::NOCTTY, Mode::empty()).expect("open slave");
    (master, slave)
}

/// Drain the master into a persistent accumulator until `needle` is observed or
/// the deadline elapses. Returns whether the needle is present. Using a single
/// growing buffer across phases avoids losing bytes that arrive batched with an
/// earlier needle.
fn drain_until(master: &OwnedFd, acc: &mut String, needle: &str, deadline: Duration) -> bool {
    if acc.contains(needle) {
        return true;
    }
    let start = Instant::now();
    let mut buf = [0u8; 4096];
    while start.elapsed() < deadline {
        match read(master, &mut buf) {
            Ok(0) => return acc.contains(needle), // EOF: all slaves closed.
            Ok(n) => {
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                if acc.contains(needle) {
                    return true;
                }
            }
            Err(err) if err == rustix::io::Errno::AGAIN => sleep(Duration::from_millis(10)),
            Err(err) if err == rustix::io::Errno::IO => return acc.contains(needle),
            Err(_) => return acc.contains(needle),
        }
    }
    acc.contains(needle)
}

/// Poll `tcgetsid(master)` until it resolves (the helper has acquired the slave
/// as its controlling terminal) or the deadline elapses.
fn wait_for_ctty_sid(master: &OwnedFd, deadline: Duration) -> Option<i32> {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if let Ok(pid) = tcgetsid(master) {
            return Some(pid.as_raw_nonzero().get());
        }
        sleep(Duration::from_millis(10));
    }
    None
}

#[test]
fn tty_helper_establishes_session_ctty_winsize_winch_and_hangup() {
    let bin = env!("CARGO_BIN_EXE_nixling-exec-runner");
    let (master, slave) = open_pty();
    // CLOEXEC status pipe: the write end closes on a successful exec (EOF), and
    // the helper never leaks it into the target (proves the CLOEXEC handoff).
    let (status_r, status_w) = pipe_with(PipeFlags::CLOEXEC).expect("status pipe");

    // The target prints its initial winsize, then re-prints it on every
    // SIGWINCH, and backgrounds an in-session child whose pid it announces. A
    // `sleep` loop (rather than a blocking `read`) keeps the shell responsive so
    // a trapped SIGWINCH runs promptly between commands, and a SIGHUP on hangup
    // still terminates it.
    let script = "stty size; \
         trap 'stty size' WINCH; \
         sleep 600 & \
         echo \"BG:$!\"; \
         echo READY; \
         while :; do sleep 1; done";

    let mut child = Command::new(bin)
        .args([
            "--tty-exec",
            "--rows",
            "30",
            "--cols",
            "100",
            "--",
            "/bin/sh",
            "-c",
            script,
        ])
        // Safe fd handoff, mirroring the production spawner: slave on stdin,
        // status pipe on stdout, stderr to null. No pass_fds / pre_exec.
        .stdin(Stdio::from(slave))
        .stdout(Stdio::from(status_w))
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn --tty-exec helper");
    let child_pid = child.id() as i32;

    // The master is non-blocking so reads never wedge the test.
    ioctl_fionbio(&master, true).expect("master O_NONBLOCK");

    // 1. Success handshake: the status pipe reaches EOF (no failure byte) once
    //    the helper exec's the target. A non-empty read would be a typed
    //    failure byte (setup/exec failure).
    {
        let status = status_r;
        ioctl_fionbio(&status, true).expect("status O_NONBLOCK");
        let start = Instant::now();
        let mut handshake: Option<std::io::Result<usize>> = None;
        let mut byte = [0u8; 1];
        while start.elapsed() < Duration::from_secs(5) {
            match read(&status, &mut byte) {
                Ok(n) => {
                    handshake = Some(Ok(n));
                    break;
                }
                Err(err) if err == rustix::io::Errno::AGAIN => sleep(Duration::from_millis(10)),
                Err(err) => {
                    handshake = Some(Err(err.into()));
                    break;
                }
            }
        }
        match handshake {
            Some(Ok(0)) => {} // EOF == exec succeeded.
            other => panic!("expected status-pipe EOF on success, got {other:?}"),
        }
        // `status` drops here, closing the status reader.
    }

    // 2. Session leader + controlling terminal: tcgetsid(master) resolves to the
    //    child's pid (setsid made pid == sid; TIOCSCTTY bound the slave).
    let sid = wait_for_ctty_sid(&master, Duration::from_secs(5))
        .expect("controlling-terminal session id");
    assert_eq!(
        sid, child_pid,
        "session leader pid must equal the spawned helper/target pid"
    );

    // 3. Initial winsize applied by the helper's tcsetwinsize, and the
    //    in-session background child's pid: the script prints `stty size`
    //    ("rows cols"), then "BG:<pid>", then "READY". A single accumulator
    //    keeps every byte regardless of how reads batch.
    let mut transcript = String::new();
    assert!(
        drain_until(&master, &mut transcript, "READY", Duration::from_secs(5)),
        "expected READY from target, saw: {transcript:?}"
    );
    assert!(
        transcript.contains("30 100"),
        "expected initial winsize 30 100, saw: {transcript:?}"
    );
    let bg_pid: i32 = transcript
        .lines()
        .find_map(|line| line.trim().strip_prefix("BG:"))
        .and_then(|pid| pid.trim().parse().ok())
        .unwrap_or_else(|| panic!("expected BG:<pid> line, saw: {transcript:?}"));

    // 4. SIGWINCH: resizing the master delivers SIGWINCH to the foreground
    //    process group; the trap re-runs `stty size` with the new geometry.
    tcsetwinsize(
        &master,
        Winsize {
            ws_row: 40,
            ws_col: 120,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )
    .expect("resize master");
    assert!(
        drain_until(&master, &mut transcript, "40 120", Duration::from_secs(5)),
        "expected post-SIGWINCH winsize 40 120, saw: {transcript:?}"
    );

    // 5. SIGHUP on hangup: dropping the master (the last master reference) closes
    //    the terminal, so the kernel sends SIGHUP to the session leader, which
    //    terminates.
    drop(master);
    let start = Instant::now();
    let mut hung_up = false;
    while start.elapsed() < Duration::from_secs(5) {
        match child.try_wait() {
            Ok(Some(_status)) => {
                hung_up = true;
                break;
            }
            Ok(None) => sleep(Duration::from_millis(10)),
            Err(_) => break,
        }
    }
    assert!(hung_up, "session leader did not exit after master hangup");
    // Reap the now-exited helper/target so no zombie remains.
    let _ = child.wait();

    // 6. In-session no-orphan: the whole session (sid == child_pid) is reapable
    //    as a unit. The background child shares the session; SIGKILL by session
    //    process group reaps it, demonstrating the foundation guestd's
    //    ProcSessionReaper relies on (sid-scoped kill). A setsid escapee would
    //    be out of scope (documented trusted-root limitation).
    if let Some(pid) = Pid::from_raw(child_pid) {
        let _ = kill_process_group(pid, Signal::Kill);
    }
    let start = Instant::now();
    let mut reaped = false;
    while start.elapsed() < Duration::from_secs(5) {
        // test_kill_process is kill(pid, 0): Err(ESRCH) once the pid is gone.
        if let Some(pid) = Pid::from_raw(bg_pid) {
            if test_kill_process(pid).is_err() {
                reaped = true;
                break;
            }
        }
        sleep(Duration::from_millis(10));
    }
    assert!(
        reaped,
        "in-session background child {bg_pid} was not reaped with the session"
    );
}
