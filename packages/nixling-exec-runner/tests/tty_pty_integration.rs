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
use rustix::process::{kill_process, kill_process_group, test_kill_process, Pid, Signal};
use rustix::pty::{grantpt, openpt, ptsname, unlockpt, OpenptFlags};
use rustix::termios::{tcgetpgrp, tcgetsid, tcsetwinsize, Winsize};

/// Generous wall-clock budget for the signal-/PTY-delivery polls below. The
/// polls break as soon as their condition is met (~1s on an idle machine), so a
/// large ceiling does not slow the happy path — it only keeps the suite green
/// under heavy CI load (e.g. when the rust workspace gate runs concurrently
/// with the Nix eval region). A tight 5s ceiling was racy under that load.
const WAIT: Duration = Duration::from_secs(30);

/// Hard ceiling for the SIGHUP-on-hangup watchdog. The session leader is
/// reaped with a blocking `wait()`, so this is NOT a normal-path budget — it
/// only bounds the pathological case where the leader genuinely ignores SIGHUP
/// (a real bug), escalating to SIGKILL so the test fails loudly instead of
/// hanging. Generous so heavy co-scheduled load never trips it spuriously.
const WATCHDOG: Duration = Duration::from_secs(120);

/// Allocate a PTY master/slave pair the way the spawner does: master AND slave
/// both `O_RDWR|O_NOCTTY|O_CLOEXEC`. The slave is `O_CLOEXEC` (matching the G4
/// production contract) so a concurrent fork/exec elsewhere cannot inherit it
/// and keep the PTY alive; `Stdio::from(slave)` still hands fd0 to the helper
/// because `Command`'s dup2 clears `CLOEXEC` on the duplicate.
fn open_pty() -> (OwnedFd, OwnedFd) {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)
        .expect("openpt master");
    grantpt(&master).expect("grantpt");
    unlockpt(&master).expect("unlockpt");
    let slave_path = ptsname(&master, Vec::new()).expect("ptsname");
    let slave = open(
        &slave_path,
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .expect("open slave");
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
    // SIGWINCH, and backgrounds an in-session child that IGNORES SIGHUP (so it
    // survives the terminal hangup the foreground shell dies from). It stays in
    // the helper-created session, so only the sid-scoped reap — not the SIGHUP
    // and not a foreground-PG kill — can clean it up. A `sleep` loop (rather
    // than a blocking `read`) keeps the shell responsive so a trapped SIGWINCH
    // runs promptly between commands, and a SIGHUP on hangup still terminates it.
    let script = "stty size; \
         trap 'stty size' WINCH; \
         ( trap '' HUP; exec sleep 600 ) & \
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
        while start.elapsed() < WAIT {
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
    let sid = wait_for_ctty_sid(&master, WAIT).expect("controlling-terminal session id");
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
        drain_until(&master, &mut transcript, "READY", WAIT),
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
        drain_until(&master, &mut transcript, "40 120", WAIT),
        "expected post-SIGWINCH winsize 40 120, saw: {transcript:?}"
    );

    // 5. SIGHUP on hangup: dropping the master (the last master reference) closes
    //    the terminal, so the kernel sends SIGHUP to the session leader, which
    //    terminates. Send an explicit SIGHUP to the leader as well (the kernel
    //    terminal-hangup delivery can race). Then poll for the leader to exit
    //    with NO fixed budget for the success path: under heavy co-scheduled
    //    load (the rust gate running concurrently with the Nix eval region) the
    //    leader can be scheduler-starved for tens of seconds, and a wall-clock
    //    deadline would flake. Only after a generous WATCHDOG ceiling — which a
    //    healthy leader never reaches — do we treat a still-alive leader as a
    //    genuine SIGHUP-ignored bug, SIGKILL it to avoid hanging the suite, and
    //    fail the assertion. SIGHUP goes to the leader's PID only (not the
    //    process group), so the HUP-trapping in-session background child checked
    //    in step 6 still survives.
    drop(master);
    if let Some(pid) = Pid::from_raw(child_pid) {
        let _ = kill_process(pid, Signal::Hup);
    }
    let start = Instant::now();
    let mut hup_ignored = false;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if start.elapsed() > WATCHDOG {
                    if let Some(pid) = Pid::from_raw(child_pid) {
                        let _ = kill_process(pid, Signal::Kill);
                    }
                    hup_ignored = true;
                    break;
                }
                sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
    // Reap (the SIGKILL path leaves a just-killed child to collect).
    let _ = child.wait();
    assert!(
        !hup_ignored,
        "session leader did not exit after master hangup (SIGHUP ignored)"
    );
    // Reap the now-exited helper/target so no zombie remains.
    let _ = child.wait();

    // 6. In-session no-orphan: the background child shares the helper-created
    //    session (sid == child_pid) and is NOT a direct child of the helper's
    //    foreground process, so SIGHUP to the session leader does not reach it —
    //    it must still be ALIVE here, BEFORE guestd's reaper runs. This is the
    //    exact condition ProcSessionReaper exists to handle.
    assert!(
        Pid::from_raw(bg_pid)
            .map(|pid| test_kill_process(pid).is_ok())
            .unwrap_or(false),
        "in-session background child {bg_pid} should still be alive after SIGHUP, before the sid-scoped reap"
    );
    // Reap by the SAME sid-scoped logic guestd uses (ProcSessionReaper): scan
    // /proc for every pid whose session id == child_pid and SIGKILL it — not by
    // killing child_pid's process group. This proves the foundation of guestd's
    // no-orphan teardown. A setsid/double-fork escapee would be out of scope
    // (documented trusted-root limitation).
    for _ in 0..50 {
        let pids = pids_in_session(child_pid);
        if pids.is_empty() {
            break;
        }
        for pid in pids {
            if let Some(pid) = Pid::from_raw(pid) {
                let _ = kill_process(pid, Signal::Kill);
            }
        }
        sleep(Duration::from_millis(10));
    }
    let start = Instant::now();
    let mut reaped = false;
    while start.elapsed() < WAIT {
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
        "in-session background child {bg_pid} was not reaped by the sid-scoped sweep"
    );
}

/// Parse a labelled decimal pid (`<label><pid>`) from the accumulated
/// transcript, e.g. `SHELL:` or `CHILD:`.
fn parse_labelled_pid(transcript: &str, label: &str) -> Option<i32> {
    transcript
        .lines()
        .find_map(|line| line.trim().strip_prefix(label))
        .and_then(|pid| pid.trim().parse().ok())
}

/// Poll `tcgetpgrp(master)` until the terminal's foreground process group moves
/// off `session_leader` (job control's `tcsetpgrp` has placed a child group in
/// the foreground) or the deadline elapses.
fn wait_for_fg_pgrp_off(master: &OwnedFd, session_leader: i32, deadline: Duration) -> Option<i32> {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if let Ok(pgid) = tcgetpgrp(master) {
            let pgid = pgid.as_raw_nonzero().get();
            if pgid != session_leader {
                return Some(pgid);
            }
        }
        sleep(Duration::from_millis(10));
    }
    None
}

#[test]
fn tty_signal_follows_the_foreground_process_group_at_delivery_time() {
    // ExecSignal resolves its target via `tcgetpgrp(master)` AT DELIVERY TIME
    // (exec_pty.rs `ProcPtyControl::signal_foreground`), so a signal follows the
    // job-control shell's *current* foreground process group rather than the
    // session leader's group. Prove that end to end against a real PTY + a real
    // job-control shell:
    //
    //   1. The helper execs `/bin/sh` with `set -m` (job control on). The shell
    //      is the session leader; its process group == the session id.
    //   2. The shell runs a FOREGROUND child in its OWN process group (job
    //      control's `tcsetpgrp`), moving the terminal's foreground PG OFF the
    //      session leader and ONTO the child's group.
    //   3. Delivering the allowlisted signal exactly as guestd does —
    //      `tcgetpgrp(master)` then `kill_process_group` — reaches the CHILD (a
    //      different PG than the session leader). Had delivery used the session
    //      leader's PG, the child (different PG) would never receive it.
    let bin = env!("CARGO_BIN_EXE_nixling-exec-runner");
    let (master, slave) = open_pty();
    // CLOEXEC status pipe (helper protocol). We do not need the failure byte
    // here; success is observed via the controlling-terminal handshake below.
    let (status_r, status_w) = pipe_with(PipeFlags::CLOEXEC).expect("status pipe");

    // `set -m`: each foreground external command runs in its own PG and is made
    // the terminal's foreground PG. The child reports its pid, traps SIGUSR1
    // (allowlisted: signal 10), prints CHILDREADY, then loops. On SIGUSR1 it
    // prints GOTUSR1 and exits, after which the shell prints SHELLRESUMED.
    let script = "set -m; \
         echo \"SHELL:$$\"; \
         sh -c 'echo \"CHILD:$$\"; trap \"echo GOTUSR1; exit 7\" USR1; echo CHILDREADY; while :; do sleep 1; done'; \
         echo SHELLRESUMED";

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
        // Safe fd handoff mirroring the production spawner.
        .stdin(Stdio::from(slave))
        .stdout(Stdio::from(status_w))
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn --tty-exec helper");
    let helper_pid = child.id() as i32;
    // The status read end closes on a successful exec; we keep it open (drop at
    // end of scope) so a possible failure byte has somewhere to go.
    ioctl_fionbio(&master, true).expect("master O_NONBLOCK");

    // The helper exec'd /bin/sh and bound the slave as its controlling terminal:
    // the session id resolves to the helper/shell pid (setsid made pid == sid).
    let sid = wait_for_ctty_sid(&master, WAIT).expect("controlling-terminal session id");
    assert_eq!(sid, helper_pid, "the shell is the session leader");

    // The shell prints SHELL:<pid> and the foreground child prints CHILD:<pid>
    // then CHILDREADY. The child is a DIFFERENT process from the shell.
    let mut transcript = String::new();
    assert!(
        drain_until(&master, &mut transcript, "CHILDREADY", WAIT),
        "foreground child did not become ready, saw: {transcript:?}"
    );
    let shell_pid = parse_labelled_pid(&transcript, "SHELL:")
        .unwrap_or_else(|| panic!("expected SHELL:<pid>, saw: {transcript:?}"));
    let child_pid = parse_labelled_pid(&transcript, "CHILD:")
        .unwrap_or_else(|| panic!("expected CHILD:<pid>, saw: {transcript:?}"));
    assert_eq!(
        shell_pid, sid,
        "the shell pid is the session leader / session id"
    );
    assert_ne!(
        child_pid, shell_pid,
        "the foreground child is a distinct process"
    );

    // The terminal's foreground PG moved OFF the session leader and ONTO the
    // child's own process group (job control). This is the precondition the
    // delivery-time `tcgetpgrp` exists to track.
    let fg_pgrp = wait_for_fg_pgrp_off(&master, sid, WAIT)
        .expect("foreground PG must move off the session leader");
    assert_ne!(
        fg_pgrp, sid,
        "the foreground PG must not be the session leader's group at delivery time"
    );
    assert_eq!(
        fg_pgrp, child_pid,
        "the foreground PG should be the child's own process group"
    );

    // Deliver SIGUSR1 EXACTLY as guestd's signal_foreground does: resolve the
    // foreground PG via tcgetpgrp(master) at delivery time, then signal that
    // group. The resolution must reflect the CURRENT foreground (the child),
    // not a value captured at session start.
    let pgid = tcgetpgrp(&master).expect("tcgetpgrp at delivery time");
    assert_eq!(
        pgid.as_raw_nonzero().get(),
        child_pid,
        "delivery-time tcgetpgrp must resolve to the child's foreground group"
    );
    let sig = Signal::from_raw(10).expect("SIGUSR1 is a valid signal");
    kill_process_group(pgid, sig).expect("deliver SIGUSR1 to the foreground PG");

    // The child (in the foreground PG, distinct from the session leader)
    // received SIGUSR1, ran its trap, and exited — and only then does the shell
    // resume. Observing both markers proves the signal reached the child's group
    // and NOT merely the session leader.
    assert!(
        drain_until(&master, &mut transcript, "GOTUSR1", WAIT),
        "SIGUSR1 was not delivered to the foreground child, saw: {transcript:?}"
    );
    assert!(
        drain_until(&master, &mut transcript, "SHELLRESUMED", WAIT),
        "shell did not resume after the foreground child exited, saw: {transcript:?}"
    );

    // Teardown: hang up the terminal and sid-scoped reap any survivors, exactly
    // as guestd's no-orphan teardown would.
    drop(master);
    drop(status_r);
    for _ in 0..50 {
        let pids = pids_in_session(sid);
        if pids.is_empty() {
            break;
        }
        for pid in pids {
            if let Some(pid) = Pid::from_raw(pid) {
                let _ = kill_process(pid, Signal::Kill);
            }
        }
        sleep(Duration::from_millis(10));
    }
    let _ = child.wait();
}

/// Mirror of guestd's `ProcSessionReaper` enumeration: every pid in `/proc`
/// whose session id (field 6 of `/proc/<pid>/stat`, the 4th field after the
/// final `)`) equals `sid`.
fn pids_in_session(sid: i32) -> Vec<i32> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Ok(pid) = name.parse::<i32>() else {
            continue;
        };
        if session_of(pid) == Some(sid) {
            out.push(pid);
        }
    }
    out
}

/// Parse the session id from `/proc/<pid>/stat` (the `comm` field may contain
/// spaces/parens, so fields are read after the final `)`).
fn session_of(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rparen = stat.rfind(')')?;
    let rest = &stat[rparen + 1..];
    // After ')': state(0) ppid(1) pgrp(2) session(3) ...
    rest.split_whitespace().nth(3).and_then(|s| s.parse().ok())
}

/// Drive the helper exactly as the spawner does and return the single status
/// byte it writes on a setup/exec failure (or `None` if it reached EOF, i.e.
/// exec succeeded). Used by the G5 failure-handshake tests.
fn run_helper_expect_status_byte(args: &[&str]) -> Option<u8> {
    let bin = env!("CARGO_BIN_EXE_nixling-exec-runner");
    let (master, slave) = open_pty();
    let (status_r, status_w) = pipe_with(PipeFlags::CLOEXEC).expect("status pipe");
    let mut full_args = vec!["--tty-exec"];
    full_args.extend_from_slice(args);
    let mut child = Command::new(bin)
        .args(&full_args)
        .stdin(Stdio::from(slave))
        .stdout(Stdio::from(status_w))
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn --tty-exec helper");
    // Keep the master alive so the slave end stays open while the helper runs.
    ioctl_fionbio(&status_r, true).expect("status O_NONBLOCK");
    let start = Instant::now();
    let mut result: Option<u8> = None;
    let mut byte = [0u8; 1];
    while start.elapsed() < WAIT {
        match read(&status_r, &mut byte) {
            Ok(0) => break, // EOF: exec succeeded (no failure byte).
            Ok(_) => {
                result = Some(byte[0]);
                break;
            }
            Err(err) if err == rustix::io::Errno::AGAIN => sleep(Duration::from_millis(5)),
            Err(_) => break,
        }
    }
    let _ = child.wait();
    drop(master);
    result
}

#[test]
fn tty_helper_reports_typed_byte_when_target_missing() {
    // ENOENT target: setsid/TIOCSCTTY/winsize/dup2 succeed, then execve fails.
    // The helper must write the Exec failure byte (5), NOT reach a bare EOF that
    // guestd would misread as a successful exec.
    let byte = run_helper_expect_status_byte(&[
        "--rows",
        "24",
        "--cols",
        "80",
        "--",
        "/nonexistent/nixling-tty-target",
    ]);
    assert_eq!(
        byte,
        Some(5),
        "missing target must yield the Exec failure byte (5), got {byte:?}"
    );
}

#[test]
fn tty_helper_reports_typed_byte_on_bad_args() {
    // Malformed invocation (relative argv): parsing fails before any dup2, so the
    // helper writes the Args failure byte (6) over the still-attached status fd.
    let byte = run_helper_expect_status_byte(&["--rows", "24", "--cols", "80", "--", "relative"]);
    assert_eq!(
        byte,
        Some(6),
        "malformed args must yield the Args failure byte (6), got {byte:?}"
    );
}
