//! `nixling-exec-runner --serve-exec --slot NN` service mode.
//!
//! This is the per-slot detached supervisor systemd launches as
//! `nixling-exec-<NN>.service`. It reads the SpecCodec spec guestd wrote into
//! the slot dir, spawns the child in its own process group, streams stdout and
//! stderr into the slot's FileRings, and stays resident as supervisor until the
//! child exits or a cancel (control-file sentinel) / optional runtime ceiling
//! fires. It installs NO in-process signal handler (cancellation is polled from
//! the `cancel` control file), so a stop SIGTERM reliably reaches the child.
//!
//! The supervision core is generic over fakeable `Spawner` / `Signaller` /
//! `Clock` / `CancelSource` traits so the cancel/precedence/ceiling matrix is
//! tested deterministically without spawning real processes. Production impls
//! use `std::process` + `rustix` (the binary half of the crate may use rustix;
//! the library half stays dependency-pure).

use std::io::{ErrorKind, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use nixling_exec_runner::atomicio::{atomic_write, read_file_nofollow};
use nixling_exec_runner::filering::FileRing;
use nixling_exec_runner::paths::{RunnerPaths, Stream};
use nixling_exec_runner::record::{StatusPhase, StatusRecord};
use nixling_exec_runner::spec::{ExecSpec, SpecCodec};
use nixling_exec_runner::DETACHED_RETAINED_PER_VM;

/// Drain buffer size (mirrors guestd's PIPE_READ_CHUNK).
const DRAIN_CHUNK: usize = 64 * 1024;

/// Production control-watcher poll interval.
const DEFAULT_POLL: Duration = Duration::from_millis(100);
/// Production child TERM->KILL grace (must stay well under the unit's
/// `TimeoutStopSec` so systemd's backstop SIGKILL never races the status write).
const DEFAULT_GRACE: Duration = Duration::from_secs(5);

/// Stop signals the supervisor may send to the child's process group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopSignal {
    Term,
    Kill,
}

/// Terminal disposition of the direct child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildOutcome {
    Exited(i32),
    Signaled(i32),
}

/// Opaque spawn failure (ENOENT/EACCES/exec-format/relative argv0/...). Carries
/// no payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnFailure;

/// A spawned, supervised child. The supervisor takes the stdout/stderr readers
/// for draining and calls `wait` exactly once to reap the direct child.
pub trait ChildHandle: Send {
    fn pgid(&self) -> i32;
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>>;
    fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>>;
    fn wait(&mut self) -> ChildOutcome;
}

/// Spawns the validated spec's command in its own process group.
pub trait Spawner: Send + Sync {
    fn spawn(&self, spec: &ExecSpec) -> Result<Box<dyn ChildHandle>, SpawnFailure>;
}

/// Signals a child process group (best-effort, idempotent).
pub trait Signaller: Send + Sync {
    fn signal_group(&self, pgid: i32, signal: StopSignal);
}

/// Monotonic millisecond clock (fakeable).
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// Observes the cancel control-file sentinel (fakeable).
pub trait CancelSource: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

/// Fakeable timing for the control watcher.
#[derive(Debug, Clone, Copy)]
pub struct SuperviseConfig {
    pub poll_interval: Duration,
    pub grace: Duration,
}

impl Default for SuperviseConfig {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_POLL,
            grace: DEFAULT_GRACE,
        }
    }
}

/// Outcome of a service-mode run that determines the process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerResult {
    /// The runner did its job (terminal/infra status written). Exit 0; the unit
    /// completes cleanly and guestd reads the status file.
    Done,
    /// The slot dir is so broken the runner could not even write a status file.
    /// Exit non-zero so the unit fails and guestd's reconciliation notices.
    StatusUnwritable,
}

fn write_status(paths: &RunnerPaths, phase: StatusPhase) -> Result<(), ()> {
    atomic_write(&paths.status(), &StatusRecord::new(phase).encode()).map_err(|_| ())
}

fn open_ring(paths: &RunnerPaths, stream: Stream, cap: u64) -> Result<FileRing, ()> {
    FileRing::create(&paths.data(stream), &paths.sidecar(stream), cap).map_err(|_| ())
}

fn spawn_drain(
    mut reader: Box<dyn Read + Send>,
    ring: Arc<Mutex<FileRing>>,
    done: std::sync::mpsc::Sender<()>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; DRAIN_CHUNK];
        let mut lost = false;
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if !lost {
                        let mut guard = ring.lock().expect("ring poisoned");
                        if guard.append(&buf[..n]).is_err() {
                            // Write error (e.g. /run full): mark the stream lost
                            // and keep draining so the child never blocks on a
                            // stuck log writer.
                            let _ = guard.mark_lost();
                            lost = true;
                        }
                    }
                }
                Err(ref err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        {
            let mut guard = ring.lock().expect("ring poisoned");
            let _ = guard.mark_eof();
        }
        // Signal clean completion so the supervisor can bound its wait. A send
        // failure (supervisor already moved on) is benign.
        let _ = done.send(());
    })
}

/// Wait up to `grace` for the drain threads to finish naturally. In the normal
/// case the direct child's exit closed the only pipe write-ends, so the drains
/// EOF and finish at once (capturing all output + a clean stream EOF). If a
/// leaked descendant inherited a pipe write-end the drains can block
/// indefinitely; this bounded wait then returns so the terminal status is
/// still published and the runner never hangs. Returns the number of drains
/// that finished within the grace.
fn await_drains_bounded(
    done_rx: &std::sync::mpsc::Receiver<()>,
    expected: usize,
    grace: Duration,
) -> usize {
    let deadline = std::time::Instant::now() + grace;
    let mut finished = 0;
    while finished < expected {
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            break;
        };
        match done_rx.recv_timeout(remaining) {
            Ok(()) => finished += 1,
            Err(_) => break,
        }
    }
    finished
}

/// Supervise one detached exec. Writes spawn-failed/infra-failed/terminal
/// status as appropriate. Generic over the spawn/signal/clock/cancel traits.
#[allow(clippy::too_many_arguments)]
pub fn supervise(
    spec: &ExecSpec,
    paths: &RunnerPaths,
    spawner: &dyn Spawner,
    signaller: Arc<dyn Signaller>,
    clock: Arc<dyn Clock>,
    cancel: Arc<dyn CancelSource>,
    cfg: &SuperviseConfig,
) -> RunnerResult {
    // Reserve both log rings up front; a failure here is an infra failure
    // (guestd treats the infra-failed status as a create error and cleans up).
    let stdout_ring = match open_ring(paths, Stream::Stdout, spec.stdout_log_cap) {
        Ok(ring) => Arc::new(Mutex::new(ring)),
        Err(()) => return finish_infra_failed(paths),
    };
    let stderr_ring = match open_ring(paths, Stream::Stderr, spec.stderr_log_cap) {
        Ok(ring) => Arc::new(Mutex::new(ring)),
        Err(()) => return finish_infra_failed(paths),
    };

    let mut child = match spawner.spawn(spec) {
        Ok(child) => child,
        Err(SpawnFailure) => {
            // Legitimate terminal exec: retained, exit 0.
            return match write_status(paths, StatusPhase::SpawnFailed) {
                Ok(()) => RunnerResult::Done,
                Err(()) => RunnerResult::StatusUnwritable,
            };
        }
    };

    let pgid = child.pgid();
    let start_ms = clock.now_ms();
    let ceiling_ms = spec.max_runtime_sec.saturating_mul(1_000);

    if write_status(paths, StatusPhase::Started).is_err() {
        // The child is live but we cannot publish `started`; tear it down and
        // fail the unit so reconciliation cleans up.
        signaller.signal_group(pgid, StopSignal::Kill);
        let _ = child.wait();
        return RunnerResult::StatusUnwritable;
    }

    // Drain stdout/stderr concurrently so the child never blocks on a full pipe.
    // Each drain signals clean completion on `done_tx`; the supervisor uses it
    // to bound its post-reap wait so a leaked descendant holding a pipe
    // write-end can never stall the terminal status (F4).
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let mut drains = Vec::new();
    if let Some(reader) = child.take_stdout() {
        drains.push(spawn_drain(reader, Arc::clone(&stdout_ring), done_tx.clone()));
    }
    if let Some(reader) = child.take_stderr() {
        drains.push(spawn_drain(reader, Arc::clone(&stderr_ring), done_tx.clone()));
    }
    let drain_count = drains.len();
    // Drop our own sender so the channel disconnects once every drain thread
    // has finished (lets `recv_timeout` observe disconnect promptly).
    drop(done_tx);

    let reaped = Arc::new(AtomicBool::new(false));
    let cancel_requested = Arc::new(AtomicBool::new(false));

    let watcher = spawn_watcher(
        pgid,
        start_ms,
        ceiling_ms,
        *cfg,
        Arc::clone(&reaped),
        Arc::clone(&cancel_requested),
        Arc::clone(&signaller),
        Arc::clone(&clock),
        Arc::clone(&cancel),
    );

    // The supervisor is the single reaper.
    let outcome = child.wait();
    reaped.store(true, Ordering::SeqCst);

    // The watcher exits as soon as it observes `reaped`; join it so
    // `cancel_requested` is final before we decide the terminal phase.
    let _ = watcher.join();

    // Decide the terminal phase NOW, before any (possibly unbounded) wait on the
    // drains. Exactly-once terminal status: cancellation (sentinel or ceiling)
    // wins iff it was requested before the child was reaped; otherwise the
    // natural exit status wins.
    let phase = if cancel_requested.load(Ordering::SeqCst) {
        StatusPhase::Cancelled
    } else {
        match outcome {
            ChildOutcome::Exited(code) => StatusPhase::Exited(code),
            ChildOutcome::Signaled(signal) => StatusPhase::Signaled(signal),
        }
    };

    // Best-effort: give the drains a bounded grace to flush their tails and mark
    // EOF. In the normal case the child's exit closed the only write-ends and
    // they finish immediately. If a leaked descendant inherited a write-end the
    // drains may block forever — we stop waiting and publish the terminal status
    // anyway. Any still-blocked drain thread is detached (dropping its
    // `JoinHandle`) and reclaimed at process exit; the streams simply lack a
    // clean EOF, which readers already tolerate.
    let _finished = await_drains_bounded(&done_rx, drain_count, cfg.grace);
    drop(drains);

    match write_status(paths, phase) {
        Ok(()) => RunnerResult::Done,
        Err(()) => RunnerResult::StatusUnwritable,
    }
}

fn finish_infra_failed(paths: &RunnerPaths) -> RunnerResult {
    match write_status(paths, StatusPhase::InfraFailed) {
        Ok(()) => RunnerResult::Done,
        Err(()) => RunnerResult::StatusUnwritable,
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_watcher(
    pgid: i32,
    start_ms: u64,
    ceiling_ms: u64,
    cfg: SuperviseConfig,
    reaped: Arc<AtomicBool>,
    cancel_requested: Arc<AtomicBool>,
    signaller: Arc<dyn Signaller>,
    clock: Arc<dyn Clock>,
    cancel: Arc<dyn CancelSource>,
) -> JoinHandle<()> {
    let poll = cfg.poll_interval.max(Duration::from_millis(1));
    let grace_steps = {
        let grace = cfg.grace.as_millis().max(1);
        let poll_ms = poll.as_millis().max(1);
        grace.div_ceil(poll_ms) as u32
    };
    std::thread::spawn(move || loop {
        if reaped.load(Ordering::SeqCst) {
            return;
        }
        let ceiling_hit =
            ceiling_ms > 0 && clock.now_ms().saturating_sub(start_ms) >= ceiling_ms;
        if cancel.is_cancelled() || ceiling_hit {
            cancel_requested.store(true, Ordering::SeqCst);
            signaller.signal_group(pgid, StopSignal::Term);
            for _ in 0..grace_steps {
                if reaped.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(poll);
            }
            if !reaped.load(Ordering::SeqCst) {
                signaller.signal_group(pgid, StopSignal::Kill);
            }
            return;
        }
        std::thread::sleep(poll);
    })
}

/// Entry point for `--serve-exec --slot NN`. Returns the process exit code.
pub fn main_service(slot: u32) -> i32 {
    if slot >= DETACHED_RETAINED_PER_VM as u32 {
        eprintln!("nixling-exec-runner: slot out of range");
        return 64;
    }
    let paths = RunnerPaths::for_slot(slot);

    if validate_slot_dir(&paths).is_err() {
        // We cannot trust the slot dir; do not attempt to write status there.
        eprintln!("nixling-exec-runner: slot directory validation failed");
        return 71;
    }

    let spec = match read_spec(&paths) {
        Ok(spec) => spec,
        Err(()) => {
            return match write_status(&paths, StatusPhase::InfraFailed) {
                Ok(()) => 0,
                Err(()) => 71,
            };
        }
    };

    let signaller: Arc<dyn Signaller> = Arc::new(production::RustixSignaller);
    let clock: Arc<dyn Clock> = Arc::new(production::MonotonicClock::new());
    let cancel: Arc<dyn CancelSource> =
        Arc::new(production::FileCancelSource::new(paths.cancel()));
    let cfg = SuperviseConfig::default();

    match supervise(
        &spec,
        &paths,
        &production::StdSpawner,
        signaller,
        clock,
        cancel,
        &cfg,
    ) {
        RunnerResult::Done => 0,
        RunnerResult::StatusUnwritable => 71,
    }
}

fn read_spec(paths: &RunnerPaths) -> Result<ExecSpec, ()> {
    let bytes = read_file_nofollow(&paths.spec()).map_err(|_| ())?;
    SpecCodec::decode(&bytes).map_err(|_| ())
}

/// Validate the slot dir is a real, root-owned directory reached without
/// traversing a symlink (dir-fd `O_NOFOLLOW` openat on each component).
fn validate_slot_dir(paths: &RunnerPaths) -> Result<(), ()> {
    use rustix::fs::{fstat, open, openat, Mode, OFlags};

    let dir_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let base = open(paths.base(), dir_flags, Mode::empty()).map_err(|_| ())?;
    let base_stat = fstat(&base).map_err(|_| ())?;
    if base_stat.st_uid != 0 {
        return Err(());
    }
    let slot = openat(&base, paths.slot_dir_name(), dir_flags, Mode::empty()).map_err(|_| ())?;
    let slot_stat = fstat(&slot).map_err(|_| ())?;
    if slot_stat.st_uid != 0 {
        return Err(());
    }
    Ok(())
}

mod production {
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    use rustix::process::{kill_process_group, Pid, Signal};

    use super::{ChildHandle, ChildOutcome, Clock, CancelSource, Signaller, SpawnFailure, Spawner, StopSignal};
    use std::io::Read;

    use nixling_exec_runner::spec::ExecSpec;

    pub struct StdSpawner;

    impl Spawner for StdSpawner {
        fn spawn(&self, spec: &ExecSpec) -> Result<Box<dyn ChildHandle>, SpawnFailure> {
            // No PATH lookup: argv[0] must be absolute.
            let program = spec.argv.first().ok_or(SpawnFailure)?;
            if !Path::new(program).is_absolute() {
                return Err(SpawnFailure);
            }
            let mut cmd = Command::new(program);
            cmd.args(&spec.argv[1..])
                .env_clear()
                .envs(spec.env.iter().map(|entry| (&entry.key, &entry.value)))
                .current_dir(spec.cwd.as_deref().unwrap_or("/"))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                // New process group with the child as leader (pgid == pid).
                .process_group(0);
            let mut child = cmd.spawn().map_err(|_| SpawnFailure)?;
            let pgid = child.id() as i32;
            let stdout = child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>);
            let stderr = child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>);
            Ok(Box::new(StdChild {
                child: Some(child),
                pgid,
                stdout,
                stderr,
            }))
        }
    }

    struct StdChild {
        child: Option<Child>,
        pgid: i32,
        stdout: Option<Box<dyn Read + Send>>,
        stderr: Option<Box<dyn Read + Send>>,
    }

    impl ChildHandle for StdChild {
        fn pgid(&self) -> i32 {
            self.pgid
        }

        fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
            self.stdout.take()
        }

        fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>> {
            self.stderr.take()
        }

        fn wait(&mut self) -> ChildOutcome {
            match self.child.as_mut().map(Child::wait) {
                Some(Ok(status)) => {
                    if let Some(code) = status.code() {
                        ChildOutcome::Exited(code)
                    } else if let Some(signal) = status.signal() {
                        ChildOutcome::Signaled(signal)
                    } else {
                        ChildOutcome::Exited(-1)
                    }
                }
                _ => ChildOutcome::Exited(-1),
            }
        }
    }

    pub struct RustixSignaller;

    impl Signaller for RustixSignaller {
        fn signal_group(&self, pgid: i32, signal: StopSignal) {
            // The group persists while the (possibly zombie) leader is
            // unreaped, so the PGID cannot be reused under this signal; the
            // supervisor stops signalling once it reaps the leader.
            if let Some(pid) = Pid::from_raw(pgid) {
                let sig = match signal {
                    StopSignal::Term => Signal::Term,
                    StopSignal::Kill => Signal::Kill,
                };
                let _ = kill_process_group(pid, sig);
            }
        }
    }

    pub struct MonotonicClock {
        start: Instant,
    }

    impl MonotonicClock {
        pub fn new() -> Self {
            Self {
                start: Instant::now(),
            }
        }
    }

    impl Clock for MonotonicClock {
        fn now_ms(&self) -> u64 {
            self.start.elapsed().as_millis() as u64
        }
    }

    pub struct FileCancelSource {
        path: PathBuf,
    }

    impl FileCancelSource {
        pub fn new(path: PathBuf) -> Self {
            Self { path }
        }
    }

    impl CancelSource for FileCancelSource {
        fn is_cancelled(&self) -> bool {
            // symlink_metadata does not follow a symlink at the final
            // component; presence of the sentinel is the cancel signal.
            std::fs::symlink_metadata(&self.path).is_ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::{Condvar, Mutex as StdMutex};

    use nixling_exec_runner::filering::FileRingReader;
    use nixling_exec_runner::RunnerEnv;

    fn scratch_slot() -> (PathBuf, RunnerPaths) {
        let base = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join(format!(
            "runner-svc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = RunnerPaths::new(&dir, 3);
        std::fs::create_dir_all(paths.slot_dir()).unwrap();
        (dir, paths)
    }

    fn spec(argv0: &str, max_runtime_sec: u64) -> ExecSpec {
        ExecSpec {
            argv: vec![argv0.to_owned()],
            cwd: None,
            env: Vec::<RunnerEnv>::new(),
            stdout_log_cap: 64 * 1024,
            stderr_log_cap: 64 * 1024,
            max_runtime_sec,
        }
    }

    fn fast_cfg() -> SuperviseConfig {
        SuperviseConfig {
            poll_interval: Duration::from_millis(1),
            grace: Duration::from_millis(8),
        }
    }

    fn read_phase(paths: &RunnerPaths) -> StatusPhase {
        let bytes = read_file_nofollow(&paths.status()).unwrap();
        StatusRecord::decode(&bytes).unwrap().phase
    }

    // ---- Fakes -----------------------------------------------------------

    #[derive(Default)]
    struct FakeState {
        signals: Vec<StopSignal>,
        exit: Option<ChildOutcome>,
        /// When set, receiving this signal causes the child to exit.
        die_on: Option<StopSignal>,
        die_outcome: Option<ChildOutcome>,
    }

    struct FakeProc {
        state: StdMutex<FakeState>,
        cv: Condvar,
    }

    impl FakeProc {
        fn new(die_on: Option<StopSignal>) -> Arc<Self> {
            Arc::new(Self {
                state: StdMutex::new(FakeState {
                    die_on,
                    die_outcome: Some(ChildOutcome::Signaled(15)),
                    ..FakeState::default()
                }),
                cv: Condvar::new(),
            })
        }

        fn set_exit(&self, outcome: ChildOutcome) {
            let mut state = self.state.lock().unwrap();
            if state.exit.is_none() {
                state.exit = Some(outcome);
            }
            self.cv.notify_all();
        }

        fn signals(&self) -> Vec<StopSignal> {
            self.state.lock().unwrap().signals.clone()
        }
    }

    struct FakeChild {
        proc: Arc<FakeProc>,
        stdout: Option<Box<dyn Read + Send>>,
        stderr: Option<Box<dyn Read + Send>>,
    }

    impl ChildHandle for FakeChild {
        fn pgid(&self) -> i32 {
            4242
        }
        fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
            self.stdout.take()
        }
        fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>> {
            self.stderr.take()
        }
        fn wait(&mut self) -> ChildOutcome {
            let mut state = self.proc.state.lock().unwrap();
            while state.exit.is_none() {
                state = self.proc.cv.wait(state).unwrap();
            }
            state.exit.unwrap()
        }
    }

    struct FakeSpawner {
        proc: Arc<FakeProc>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        fail: bool,
    }

    impl Spawner for FakeSpawner {
        fn spawn(&self, _spec: &ExecSpec) -> Result<Box<dyn ChildHandle>, SpawnFailure> {
            if self.fail {
                return Err(SpawnFailure);
            }
            Ok(Box::new(FakeChild {
                proc: Arc::clone(&self.proc),
                stdout: Some(Box::new(Cursor::new(self.stdout.clone()))),
                stderr: Some(Box::new(Cursor::new(self.stderr.clone()))),
            }))
        }
    }

    struct FakeSignaller {
        proc: Arc<FakeProc>,
    }

    impl Signaller for FakeSignaller {
        fn signal_group(&self, _pgid: i32, signal: StopSignal) {
            let mut state = self.proc.state.lock().unwrap();
            state.signals.push(signal);
            if state.die_on == Some(signal) && state.exit.is_none() {
                state.exit = state.die_outcome;
                self.proc.cv.notify_all();
            }
        }
    }

    struct FixedClock {
        now: Arc<AtomicU64>,
    }
    use std::sync::atomic::AtomicU64;

    impl Clock for FixedClock {
        fn now_ms(&self) -> u64 {
            self.now.load(Ordering::SeqCst)
        }
    }

    struct FlagCancel {
        flag: Arc<AtomicBool>,
    }

    impl CancelSource for FlagCancel {
        fn is_cancelled(&self) -> bool {
            self.flag.load(Ordering::SeqCst)
        }
    }

    /// Spawner whose child stdout is the read end of a real pipe. The test
    /// keeps the write end open to simulate a leaked descendant that inherited
    /// the pipe, so the drain thread can never observe EOF.
    struct PipeStdoutSpawner {
        proc: Arc<FakeProc>,
        read_fd: StdMutex<Option<std::os::fd::OwnedFd>>,
    }

    impl Spawner for PipeStdoutSpawner {
        fn spawn(&self, _spec: &ExecSpec) -> Result<Box<dyn ChildHandle>, SpawnFailure> {
            let fd = self
                .read_fd
                .lock()
                .unwrap()
                .take()
                .expect("PipeStdoutSpawner spawned more than once");
            let file = std::fs::File::from(fd);
            Ok(Box::new(FakeChild {
                proc: Arc::clone(&self.proc),
                stdout: Some(Box::new(file)),
                stderr: None,
            }))
        }
    }

    // ---- Tests -----------------------------------------------------------

    #[test]
    fn natural_exit_captures_output_and_records_exit_code() {
        let (dir, paths) = scratch_slot();
        let proc = FakeProc::new(None);
        proc.set_exit(ChildOutcome::Exited(7));
        let spawner = FakeSpawner {
            proc: Arc::clone(&proc),
            stdout: b"hello stdout".to_vec(),
            stderr: b"hello stderr".to_vec(),
            fail: false,
        };
        let result = supervise(
            &spec("/bin/true", 0),
            &paths,
            &spawner,
            Arc::new(FakeSignaller { proc: Arc::clone(&proc) }),
            Arc::new(FixedClock { now: Arc::new(AtomicU64::new(0)) }),
            Arc::new(FlagCancel { flag: Arc::new(AtomicBool::new(false)) }),
            &fast_cfg(),
        );
        assert_eq!(result, RunnerResult::Done);
        assert_eq!(read_phase(&paths), StatusPhase::Exited(7));

        let out = FileRingReader::open(&paths.data(Stream::Stdout), &paths.sidecar(Stream::Stdout))
            .unwrap()
            .read(0, 1024)
            .unwrap();
        assert_eq!(out.data, b"hello stdout");
        assert!(out.eof);
        let err = FileRingReader::open(&paths.data(Stream::Stderr), &paths.sidecar(Stream::Stderr))
            .unwrap()
            .read(0, 1024)
            .unwrap();
        assert_eq!(err.data, b"hello stderr");
        assert!(proc.signals().is_empty(), "no signals on a natural exit");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn signaled_child_records_signal() {
        let (dir, paths) = scratch_slot();
        let proc = FakeProc::new(None);
        proc.set_exit(ChildOutcome::Signaled(9));
        let spawner = FakeSpawner {
            proc: Arc::clone(&proc),
            stdout: Vec::new(),
            stderr: Vec::new(),
            fail: false,
        };
        supervise(
            &spec("/bin/true", 0),
            &paths,
            &spawner,
            Arc::new(FakeSignaller { proc: Arc::clone(&proc) }),
            Arc::new(FixedClock { now: Arc::new(AtomicU64::new(0)) }),
            Arc::new(FlagCancel { flag: Arc::new(AtomicBool::new(false)) }),
            &fast_cfg(),
        );
        assert_eq!(read_phase(&paths), StatusPhase::Signaled(9));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spawn_failure_is_terminal_and_retained() {
        let (dir, paths) = scratch_slot();
        let proc = FakeProc::new(None);
        let spawner = FakeSpawner {
            proc,
            stdout: Vec::new(),
            stderr: Vec::new(),
            fail: true,
        };
        let result = supervise(
            &spec("/bin/true", 0),
            &paths,
            &spawner,
            Arc::new(FakeSignaller {
                proc: FakeProc::new(None),
            }),
            Arc::new(FixedClock { now: Arc::new(AtomicU64::new(0)) }),
            Arc::new(FlagCancel { flag: Arc::new(AtomicBool::new(false)) }),
            &fast_cfg(),
        );
        assert_eq!(result, RunnerResult::Done);
        assert_eq!(read_phase(&paths), StatusPhase::SpawnFailed);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cancel_sentinel_terminates_and_records_cancelled() {
        let (dir, paths) = scratch_slot();
        // The child ignores TERM and only dies on KILL, proving TERM is sent
        // before the KILL backstop.
        let proc = FakeProc::new(Some(StopSignal::Kill));
        let spawner = FakeSpawner {
            proc: Arc::clone(&proc),
            stdout: Vec::new(),
            stderr: Vec::new(),
            fail: false,
        };
        let flag = Arc::new(AtomicBool::new(true));
        supervise(
            &spec("/bin/true", 0),
            &paths,
            &spawner,
            Arc::new(FakeSignaller { proc: Arc::clone(&proc) }),
            Arc::new(FixedClock { now: Arc::new(AtomicU64::new(0)) }),
            Arc::new(FlagCancel { flag }),
            &fast_cfg(),
        );
        assert_eq!(read_phase(&paths), StatusPhase::Cancelled);
        let signals = proc.signals();
        assert_eq!(signals.first(), Some(&StopSignal::Term));
        assert!(
            signals.contains(&StopSignal::Kill),
            "KILL backstop fires when TERM is ignored: {signals:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cancel_when_child_terms_promptly_does_not_kill() {
        let (dir, paths) = scratch_slot();
        // The child dies on TERM, so no KILL backstop is needed.
        let proc = FakeProc::new(Some(StopSignal::Term));
        let spawner = FakeSpawner {
            proc: Arc::clone(&proc),
            stdout: Vec::new(),
            stderr: Vec::new(),
            fail: false,
        };
        supervise(
            &spec("/bin/true", 0),
            &paths,
            &spawner,
            Arc::new(FakeSignaller { proc: Arc::clone(&proc) }),
            Arc::new(FixedClock { now: Arc::new(AtomicU64::new(0)) }),
            Arc::new(FlagCancel { flag: Arc::new(AtomicBool::new(true)) }),
            &fast_cfg(),
        );
        assert_eq!(read_phase(&paths), StatusPhase::Cancelled);
        assert_eq!(proc.signals(), vec![StopSignal::Term]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn runtime_ceiling_cancels_when_clock_advances() {
        let (dir, paths) = scratch_slot();
        let proc = FakeProc::new(Some(StopSignal::Term));
        let spawner = FakeSpawner {
            proc: Arc::clone(&proc),
            stdout: Vec::new(),
            stderr: Vec::new(),
            fail: false,
        };
        let now = Arc::new(AtomicU64::new(0));
        // Advance the clock past the 1s ceiling after a brief delay.
        let now_writer = Arc::clone(&now);
        let bump = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            now_writer.store(2_000, Ordering::SeqCst);
        });
        supervise(
            &spec("/bin/true", 1),
            &paths,
            &spawner,
            Arc::new(FakeSignaller { proc: Arc::clone(&proc) }),
            Arc::new(FixedClock { now: Arc::clone(&now) }),
            Arc::new(FlagCancel { flag: Arc::new(AtomicBool::new(false)) }),
            &fast_cfg(),
        );
        bump.join().unwrap();
        assert_eq!(read_phase(&paths), StatusPhase::Cancelled);
        assert!(proc.signals().contains(&StopSignal::Term));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_ceiling_does_not_cancel_a_long_running_child() {
        let (dir, paths) = scratch_slot();
        let proc = FakeProc::new(None);
        let spawner = FakeSpawner {
            proc: Arc::clone(&proc),
            stdout: Vec::new(),
            stderr: Vec::new(),
            fail: false,
        };
        let now = Arc::new(AtomicU64::new(0));
        // Let the watcher poll several times with a huge clock, then exit
        // naturally; max_runtime_sec=0 means no ceiling ever fires.
        let proc_exit = Arc::clone(&proc);
        let bump = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            proc_exit.set_exit(ChildOutcome::Exited(0));
        });
        now.store(u64::MAX / 2, Ordering::SeqCst);
        supervise(
            &spec("/bin/true", 0),
            &paths,
            &spawner,
            Arc::new(FakeSignaller { proc: Arc::clone(&proc) }),
            Arc::new(FixedClock { now }),
            Arc::new(FlagCancel { flag: Arc::new(AtomicBool::new(false)) }),
            &fast_cfg(),
        );
        bump.join().unwrap();
        assert_eq!(read_phase(&paths), StatusPhase::Exited(0));
        assert!(proc.signals().is_empty(), "no ceiling => no signals");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn infra_failure_when_slot_dir_missing() {
        let base = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join(format!("runner-missing-{}", std::process::id()));
        // Intentionally do NOT create the slot dir; ring create fails.
        let paths = RunnerPaths::new(&dir, 5);
        let proc = FakeProc::new(None);
        let spawner = FakeSpawner {
            proc: Arc::clone(&proc),
            stdout: Vec::new(),
            stderr: Vec::new(),
            fail: false,
        };
        let result = supervise(
            &spec("/bin/true", 0),
            &paths,
            &spawner,
            Arc::new(FakeSignaller { proc }),
            Arc::new(FixedClock { now: Arc::new(AtomicU64::new(0)) }),
            Arc::new(FlagCancel { flag: Arc::new(AtomicBool::new(false)) }),
            &fast_cfg(),
        );
        // No status file could be written (no dir) => unit must fail.
        assert_eq!(result, RunnerResult::StatusUnwritable);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn terminal_status_published_even_with_lingering_pipe_holder() {
        let (dir, paths) = scratch_slot();
        // A real pipe: the read end becomes the child's stdout, and we hold the
        // write end open for the duration of supervise() to mimic a leaked
        // descendant that inherited the child's stdout write-end. The drain
        // thread can never observe EOF, so an unbounded drain join would hang
        // forever and the terminal status would never be published (F4).
        let (read_fd, write_fd) = rustix::pipe::pipe().expect("pipe");
        let proc = FakeProc::new(None);
        // The direct child exits immediately; only the leaked pipe lingers.
        proc.set_exit(ChildOutcome::Exited(0));
        let spawner = PipeStdoutSpawner {
            proc: Arc::clone(&proc),
            read_fd: StdMutex::new(Some(read_fd)),
        };
        let start = std::time::Instant::now();
        let result = supervise(
            &spec("/bin/true", 0),
            &paths,
            &spawner,
            Arc::new(FakeSignaller {
                proc: Arc::clone(&proc),
            }),
            Arc::new(FixedClock {
                now: Arc::new(AtomicU64::new(0)),
            }),
            Arc::new(FlagCancel {
                flag: Arc::new(AtomicBool::new(false)),
            }),
            &fast_cfg(),
        );
        // The bounded drain wait must let supervise return promptly even though
        // the pipe write-end is still held open.
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "supervise hung on a lingering pipe holder"
        );
        assert_eq!(result, RunnerResult::Done);
        assert_eq!(read_phase(&paths), StatusPhase::Exited(0));
        // Releasing the write end lets the detached drain thread finish cleanly.
        drop(write_fd);
        std::fs::remove_dir_all(&dir).ok();
    }
}
