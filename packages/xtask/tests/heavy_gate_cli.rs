#![forbid(unsafe_code)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use signal_hook::{consts::SIGTERM, flag};

static NEXT_SCRATCH: AtomicU64 = AtomicU64::new(1);

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root")
            .to_path_buf();
        let path = root.join(format!(
            ".d2b-heavy-gate-cli-{label}-{}-{}",
            std::process::id(),
            NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("scratch");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("scratch mode");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn wait_for(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn launch_gate(
    runtime: &Path,
    role: &str,
    ready: &Path,
    duration_ms: u64,
    done: Option<&Path>,
) -> Child {
    let test_binary = std::env::current_exe().expect("test executable");
    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    command
        .args([
            "heavy-gate",
            "--",
            test_binary.to_str().expect("UTF-8 test executable"),
            "heavy_gate_test_child",
            "--exact",
            "--nocapture",
        ])
        .env("XDG_RUNTIME_DIR", runtime)
        .env("D2B_HEAVY_GATE_TEST_ROLE", role)
        .env("D2B_HEAVY_GATE_TEST_READY", ready)
        .env("D2B_HEAVY_GATE_TEST_DURATION_MS", duration_ms.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(done) = done {
        command.env("D2B_HEAVY_GATE_TEST_DONE", done);
    }
    command.spawn().expect("spawn heavy gate")
}

fn send_signal(child: &Child, signal: rustix::process::Signal) {
    let pid = i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw)
        .expect("child PID");
    rustix::process::kill_process(pid, signal).expect("signal child");
}

#[test]
fn heavy_gate_test_child() {
    let Some(role) = std::env::var_os("D2B_HEAVY_GATE_TEST_ROLE") else {
        return;
    };
    let raw_fd = std::env::var("D2B_HEAVY_GATE_FD")
        .expect("gate FD environment")
        .parse::<i32>()
        .expect("numeric gate FD");
    let flags = fcntl(raw_fd, FcntlArg::F_GETFD).expect("inherited gate FD");
    assert!(
        !FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC),
        "inherited gate FD must survive exec"
    );
    let ready = PathBuf::from(std::env::var_os("D2B_HEAVY_GATE_TEST_READY").expect("ready marker"));
    fs::write(&ready, b"ready").expect("ready marker");
    let duration = Duration::from_millis(
        std::env::var("D2B_HEAVY_GATE_TEST_DURATION_MS")
            .expect("duration")
            .parse()
            .expect("numeric duration"),
    );

    match role.to_str().expect("UTF-8 role") {
        "sleep" => thread::sleep(duration),
        "quick" => {}
        "signal" => {
            let terminated = Arc::new(AtomicBool::new(false));
            let _handler =
                flag::register(SIGTERM, Arc::clone(&terminated)).expect("SIGTERM handler");
            while !terminated.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(5));
            }
            fs::write(
                std::env::var_os("D2B_HEAVY_GATE_TEST_DONE").expect("signal marker"),
                b"signaled",
            )
            .expect("signal marker");
        }
        "group-leader" => {
            let executable = std::env::current_exe().expect("test executable");
            let descendant_ready = ready.with_extension("descendant-ready");
            let descendant = Command::new(executable)
                .args(["heavy_gate_test_child", "--exact", "--nocapture"])
                .env("D2B_HEAVY_GATE_TEST_ROLE", "group-descendant")
                .env("D2B_HEAVY_GATE_TEST_READY", descendant_ready)
                .env(
                    "D2B_HEAVY_GATE_TEST_DURATION_MS",
                    duration.as_millis().to_string(),
                )
                .env(
                    "D2B_HEAVY_GATE_TEST_DONE",
                    std::env::var_os("D2B_HEAVY_GATE_TEST_DONE").expect("done marker"),
                )
                .spawn()
                .expect("spawn descendant");
            std::mem::forget(descendant);
        }
        "group-descendant" => {
            thread::sleep(duration);
            fs::write(
                std::env::var_os("D2B_HEAVY_GATE_TEST_DONE").expect("done marker"),
                b"done",
            )
            .expect("done marker");
        }
        other => panic!("unknown helper role {other}"),
    }
}

#[test]
fn inherited_duplicate_holds_slot_after_wrapper_crash() {
    let scratch = Scratch::new("crash");
    let first_ready = scratch.0.join("first-ready");
    let second_ready = scratch.0.join("second-ready");
    let third_ready = scratch.0.join("third-ready");
    let mut first = launch_gate(&scratch.0, "sleep", &first_ready, 900, None);
    wait_for(&first_ready, Duration::from_secs(2));
    send_signal(&first, rustix::process::Signal::Kill);
    assert!(!first.wait().expect("reap crashed wrapper").success());

    let mut second = launch_gate(&scratch.0, "sleep", &second_ready, 2_000, None);
    wait_for(&second_ready, Duration::from_secs(2));
    let mut third = launch_gate(&scratch.0, "quick", &third_ready, 0, None);
    thread::sleep(Duration::from_millis(300));
    assert!(
        !third_ready.exists(),
        "third gate entered while the crashed wrapper child retained a slot"
    );
    wait_for(&third_ready, Duration::from_secs(2));
    assert!(third.wait().expect("third wrapper").success());
    assert!(second.wait().expect("second wrapper").success());
}

#[test]
fn wrapper_forwards_termination_to_child_process_group() {
    let scratch = Scratch::new("signal");
    let ready = scratch.0.join("ready");
    let signaled = scratch.0.join("signaled");
    let mut wrapper = launch_gate(&scratch.0, "signal", &ready, 0, Some(&signaled));
    wait_for(&ready, Duration::from_secs(2));
    send_signal(&wrapper, rustix::process::Signal::Term);
    assert!(wrapper.wait().expect("wrapper").success());
    assert_eq!(fs::read(signaled).expect("signal marker"), b"signaled");
}

#[test]
fn wrapper_waits_for_complete_process_group_exit() {
    let scratch = Scratch::new("group-wait");
    let ready = scratch.0.join("ready");
    let done = scratch.0.join("done");
    let started = Instant::now();
    let mut wrapper = launch_gate(&scratch.0, "group-leader", &ready, 450, Some(&done));
    wait_for(&ready, Duration::from_secs(2));
    assert!(wrapper.wait().expect("wrapper").success());
    assert!(
        done.exists(),
        "wrapper returned before its descendant exited"
    );
    assert!(started.elapsed() >= Duration::from_millis(400));
}
