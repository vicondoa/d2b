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
        let root = std::env::var_os("D2B_VALIDATION_OUTPUT_DIR")
            .map(PathBuf::from)
            .map(|root| root.join("rust-test-scratch/xtask-heavy-gate-cli"))
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-scratch")
            });
        fs::create_dir_all(&root).expect("scratch root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("scratch root mode");
        let path = root.join(format!(
            "{label}-{}-{}",
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

fn build_sigchld_inheritance_helper(scratch: &Scratch) -> PathBuf {
    let source = scratch.0.join("sigchld-inheritance.c");
    let binary = scratch.0.join("sigchld-inheritance");
    fs::write(
        &source,
        br#"#define _GNU_SOURCE
#include <errno.h>
#include <signal.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc < 4) return 2;
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    sigemptyset(&action.sa_mask);
    if (strcmp(argv[1], "ignore") == 0) {
        action.sa_handler = SIG_IGN;
    } else if (strcmp(argv[1], "nocldwait") == 0) {
        action.sa_handler = SIG_DFL;
        action.sa_flags = SA_NOCLDWAIT;
    } else {
        return 3;
    }
    if (sigaction(SIGCHLD, &action, 0) != 0) return 4;

    pid_t probe = fork();
    if (probe < 0) return 5;
    if (probe == 0) _exit(0);
    errno = 0;
    if (waitpid(probe, 0, 0) != -1 || errno != ECHILD) return 6;

    execv(argv[2], &argv[2]);
    return 7;
}
"#,
    )
    .expect("write SIGCHLD inheritance helper");
    let status = Command::new("cc")
        .arg(&source)
        .arg("-O2")
        .arg("-o")
        .arg(&binary)
        .status()
        .expect("compile SIGCHLD inheritance helper");
    assert!(
        status.success(),
        "cc must build the SIGCHLD inheritance helper"
    );
    binary
}

fn launch_gate_with_sigchld_disposition(
    runtime: &Path,
    helper: &Path,
    disposition: &str,
    ready: &Path,
) -> Child {
    let test_binary = std::env::current_exe().expect("test executable");
    Command::new(helper)
        .arg(disposition)
        .arg(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "heavy-gate",
            "--",
            test_binary.to_str().expect("UTF-8 test executable"),
            "heavy_gate_test_child",
            "--exact",
            "--nocapture",
        ])
        .env("XDG_RUNTIME_DIR", runtime)
        .env("D2B_HEAVY_GATE_TEST_ROLE", "quick")
        .env("D2B_HEAVY_GATE_TEST_READY", ready)
        .env("D2B_HEAVY_GATE_TEST_DURATION_MS", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn inherited-SIGCHLD heavy gate")
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
            thread::sleep(duration);
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
        "close-inherited" => {
            nix::unistd::close(raw_fd).expect("close inherited gate FD");
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
fn inherited_sigchld_no_wait_dispositions_are_replaced() {
    let scratch = Scratch::new("sigchld-inheritance");
    let helper = build_sigchld_inheritance_helper(&scratch);
    for disposition in ["ignore", "nocldwait"] {
        let ready = scratch.0.join(format!("{disposition}-ready"));
        let mut wrapper =
            launch_gate_with_sigchld_disposition(&scratch.0, &helper, disposition, &ready);
        wait_for(&ready, Duration::from_secs(2));
        assert!(
            wrapper.wait().expect("wait for heavy gate").success(),
            "heavy gate did not restore waitable children after inherited {disposition}"
        );
    }
}

#[test]
fn inherited_duplicate_holds_slot_after_wrapper_crash() {
    let scratch = Scratch::new("crash");
    let first_ready = scratch.0.join("first-ready");
    let first_done = scratch.0.join("first-done");
    let second_ready = scratch.0.join("second-ready");
    let third_ready = scratch.0.join("third-ready");
    let mut first = launch_gate(
        &scratch.0,
        "group-leader",
        &first_ready,
        900,
        Some(&first_done),
    );
    wait_for(
        &first_ready.with_extension("descendant-ready"),
        Duration::from_secs(2),
    );
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
    wait_for(&first_done, Duration::from_secs(2));
    wait_for(&third_ready, Duration::from_secs(2));
    assert!(third.wait().expect("third wrapper").success());
    assert!(second.wait().expect("second wrapper").success());
}

#[test]
fn wrapper_forwards_termination_to_child_process_group() {
    let scratch = Scratch::new("signal");
    let ready = scratch.0.join("ready");
    let signaled = scratch.0.join("signaled");
    let second_ready = scratch.0.join("second-ready");
    let third_ready = scratch.0.join("third-ready");
    let mut wrapper = launch_gate(&scratch.0, "signal", &ready, 350, Some(&signaled));
    wait_for(&ready, Duration::from_secs(2));
    let mut second = launch_gate(&scratch.0, "sleep", &second_ready, 1_000, None);
    wait_for(&second_ready, Duration::from_secs(2));
    let mut third = launch_gate(&scratch.0, "quick", &third_ready, 0, None);
    send_signal(&wrapper, rustix::process::Signal::Term);
    thread::sleep(Duration::from_millis(150));
    assert!(
        !third_ready.exists(),
        "signal handling released the parent permit before group exit"
    );
    wait_for(&signaled, Duration::from_secs(2));
    assert!(wrapper.wait().expect("wrapper").success());
    assert_eq!(fs::read(signaled).expect("signal marker"), b"signaled");
    wait_for(&third_ready, Duration::from_secs(2));
    assert!(third.wait().expect("third wrapper").success());
    assert!(second.wait().expect("second wrapper").success());
}

#[test]
fn wrapper_parent_descriptor_holds_after_child_closes_duplicate() {
    let scratch = Scratch::new("parent-descriptor");
    let first_ready = scratch.0.join("first-ready");
    let first_done = scratch.0.join("first-done");
    let second_ready = scratch.0.join("second-ready");
    let third_ready = scratch.0.join("third-ready");
    let mut first = launch_gate(
        &scratch.0,
        "close-inherited",
        &first_ready,
        450,
        Some(&first_done),
    );
    wait_for(&first_ready, Duration::from_secs(2));
    let mut second = launch_gate(&scratch.0, "sleep", &second_ready, 1_000, None);
    wait_for(&second_ready, Duration::from_secs(2));
    let mut third = launch_gate(&scratch.0, "quick", &third_ready, 0, None);
    thread::sleep(Duration::from_millis(150));
    assert!(
        !third_ready.exists(),
        "child close released the wrapper parent's original OFD"
    );
    wait_for(&first_done, Duration::from_secs(2));
    assert!(first.wait().expect("first wrapper").success());
    wait_for(&third_ready, Duration::from_secs(2));
    assert!(third.wait().expect("third wrapper").success());
    assert!(second.wait().expect("second wrapper").success());
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
