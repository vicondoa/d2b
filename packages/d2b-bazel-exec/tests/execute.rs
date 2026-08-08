#![cfg(feature = "test-support")]

use std::{
    ffi::OsString,
    fs::{self, File},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use d2b_bazel_exec::{
    ExecutionRequest, HandoffError, LaunchCoordinator, MaskSnapshot, ProtocolError, StdioPolicy,
    decode_exec_error, test_support,
};
use test_support::{BackendError, ExecutionBackend, LaunchPlan, SpawnReceipt};

type PlanObservation = (i32, bool, &'static str, Vec<OsString>);

#[derive(Clone)]
struct FakeBackend {
    events: Arc<Mutex<Vec<&'static str>>>,
    capture: Result<MaskSnapshot, BackendError>,
    block: Result<(), BackendError>,
    restore: Result<(), BackendError>,
    spawn: Result<SpawnReceipt, BackendError>,
    entered: Option<mpsc::Sender<()>>,
    release: Option<Arc<Barrier>>,
    saw_plan: Arc<Mutex<Option<PlanObservation>>>,
}

impl FakeBackend {
    fn passing() -> Self {
        Self {
            capture: Ok(MaskSnapshot::Test(7)),
            block: Ok(()),
            restore: Ok(()),
            spawn: Ok(SpawnReceipt::started()),
            ..Self::default()
        }
    }

    fn event_names(&self) -> Vec<&'static str> {
        self.events.lock().expect("events").clone()
    }

    fn plan(&self) -> Option<PlanObservation> {
        self.saw_plan.lock().expect("plan").clone()
    }
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            capture: Err(BackendError::Capture),
            block: Err(BackendError::Block),
            restore: Err(BackendError::Restore),
            spawn: Err(BackendError::Spawn),
            entered: None,
            release: None,
            saw_plan: Arc::new(Mutex::new(None)),
        }
    }
}

impl ExecutionBackend for FakeBackend {
    fn capture_mask(&self) -> Result<MaskSnapshot, BackendError> {
        self.events.lock().expect("events").push("capture");
        self.capture
    }

    fn block_managed(&self) -> Result<(), BackendError> {
        self.events.lock().expect("events").push("block");
        self.block
    }

    fn restore_mask(&self, _snapshot: MaskSnapshot) -> Result<(), BackendError> {
        self.events.lock().expect("events").push("restore");
        self.restore
    }

    fn spawn(&self, plan: LaunchPlan) -> Result<SpawnReceipt, BackendError> {
        self.events.lock().expect("events").push("spawn");
        *self.saw_plan.lock().expect("plan") = Some((
            plan.private_fd_number(),
            plan.preserves_standard_streams(),
            plan.supervisor().label(),
            plan.request().target_argv.clone(),
        ));
        if let Some(entered) = &self.entered {
            entered.send(()).expect("spawn entered");
        }
        if let Some(release) = &self.release {
            release.wait();
        }
        self.spawn
    }
}

#[test]
fn successful_signal_handoff_restores_before_unlock() {
    let coordinator = LaunchCoordinator::new();
    let backend = FakeBackend::passing();
    let result = test_support::run_signal_handoff(&coordinator, &backend, || {
        backend.events.lock().expect("events").push("closure");
        Ok(SpawnReceipt::started())
    })
    .expect("handoff");
    assert!(result.helper_started());
    assert_eq!(
        backend.event_names(),
        ["capture", "block", "closure", "restore"]
    );
}

#[test]
fn capture_and_block_failures_refuse_spawn_and_restore_after_capture() {
    let coordinator = LaunchCoordinator::new();
    let mut backend = FakeBackend::passing();
    backend.capture = Err(BackendError::Capture);
    let error = test_support::run_signal_handoff(
        &coordinator,
        &backend,
        || -> Result<SpawnReceipt, BackendError> {
            panic!("capture failure must not spawn");
        },
    )
    .expect_err("capture failure");
    assert_eq!(error.code(), "D2B-BZLEXEC-PARENT-SIGNAL-HANDOFF");
    assert_eq!(backend.event_names(), ["capture"]);

    let coordinator = LaunchCoordinator::new();
    let mut backend = FakeBackend::passing();
    backend.block = Err(BackendError::Block);
    let error = test_support::run_signal_handoff(
        &coordinator,
        &backend,
        || -> Result<SpawnReceipt, BackendError> {
            panic!("block failure must not spawn");
        },
    )
    .expect_err("block failure");
    assert_eq!(error.code(), "D2B-BZLEXEC-PARENT-SIGNAL-HANDOFF");
    assert_eq!(backend.event_names(), ["capture", "block", "restore"]);
}

#[test]
fn spawn_and_restore_failures_remain_typed_after_each_spawn_outcome() {
    let coordinator = LaunchCoordinator::new();
    let mut backend = FakeBackend::passing();
    backend.spawn = Err(BackendError::Spawn);
    let error = test_support::run_signal_handoff(&coordinator, &backend, || backend.spawn)
        .expect_err("spawn failure");
    assert_eq!(error.code(), "D2B-BZLEXEC-PARENT-SPAWN");
    assert_eq!(backend.event_names(), ["capture", "block", "restore"]);

    let coordinator = LaunchCoordinator::new();
    let mut backend = FakeBackend::passing();
    backend.restore = Err(BackendError::Restore);
    let error =
        test_support::run_signal_handoff(&coordinator, &backend, || Ok(SpawnReceipt::started()))
            .expect_err("restore failure after success");
    assert_eq!(error.code(), "D2B-BZLEXEC-PARENT-SIGNAL-HANDOFF");

    let coordinator = LaunchCoordinator::new();
    let mut backend = FakeBackend::passing();
    backend.spawn = Err(BackendError::Spawn);
    backend.restore = Err(BackendError::Restore);
    let error = test_support::run_signal_handoff(&coordinator, &backend, || backend.spawn)
        .expect_err("restore failure after spawn failure");
    assert_eq!(error.code(), "D2B-BZLEXEC-PARENT-SIGNAL-HANDOFF");
}

#[test]
fn poisoned_process_guard_refuses_before_capture() {
    let coordinator = Arc::new(LaunchCoordinator::new());
    let poisoned = Arc::clone(&coordinator);
    let _ = thread::spawn(move || {
        let _ = std::panic::catch_unwind(|| poisoned.poison_for_test());
    })
    .join();
    let backend = FakeBackend::passing();
    let error = test_support::run_signal_handoff(
        &coordinator,
        &backend,
        || -> Result<SpawnReceipt, BackendError> {
            panic!("poisoned guard must not spawn");
        },
    )
    .expect_err("poisoned guard");
    assert_eq!(error, HandoffError::GuardPoisoned);
    assert!(backend.event_names().is_empty());
}

#[test]
fn overlapping_launches_share_the_guard_and_restore_before_the_second_capture() {
    let coordinator = Arc::new(LaunchCoordinator::new());
    let (entered_tx, entered_rx) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let first = FakeBackend {
        entered: Some(entered_tx),
        release: Some(Arc::clone(&release)),
        ..FakeBackend::passing()
    };
    let second_started = Arc::new(AtomicBool::new(false));
    let second_started_clone = Arc::clone(&second_started);
    let second = FakeBackend {
        events: Arc::new(Mutex::new(Vec::new())),
        ..FakeBackend::passing()
    };
    let first_coordinator = Arc::clone(&coordinator);
    let first_thread = thread::spawn(move || {
        test_support::run_signal_handoff(&first_coordinator, &first, || {
            first.events.lock().expect("events").push("spawn");
            first
                .entered
                .as_ref()
                .expect("entered channel")
                .send(())
                .expect("spawn entered");
            first.release.as_ref().expect("release barrier").wait();
            Ok(SpawnReceipt::started())
        })
    });
    entered_rx.recv().expect("first launch entered spawn");

    let second_coordinator = Arc::clone(&coordinator);
    let second_thread = thread::spawn(move || {
        let result = test_support::run_signal_handoff(&second_coordinator, &second, || {
            second.events.lock().expect("events").push("spawn");
            Ok(SpawnReceipt::started())
        });
        second_started_clone.store(true, Ordering::Release);
        (result, second)
    });
    thread::sleep(Duration::from_millis(20));
    assert!(!second_started.load(Ordering::Acquire));
    release.wait();
    first_thread
        .join()
        .expect("first launch")
        .expect("first result");
    let (_, second) = second_thread.join().expect("second launch");
    assert_eq!(
        second.event_names(),
        ["capture", "block", "spawn", "restore"]
    );
}

fn temporary_provider(label: &str, bytes: &[u8]) -> (File, PathBuf) {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("d2b-bazel-exec-{label}-{suffix}"));
    fs::write(&path, bytes).expect("provider bytes");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("provider mode");
    (File::open(&path).expect("provider fd"), path)
}

#[test]
fn consuming_api_maps_an_open_file_and_passes_target_argv() {
    let (file, path) = temporary_provider("plan", b"verified-provider");
    let executable = test_support::verified_file(file);
    let backend = FakeBackend::passing();
    let request = ExecutionRequest {
        stdin: StdioPolicy::Null,
        stdout: StdioPolicy::Inherit,
        stderr: StdioPolicy::Inherit,
        target_argv: vec![OsString::from("target"), OsString::from("--closed")],
    };
    let result = test_support::execute_verified_with_backend(executable, request, &backend)
        .expect("injected backend");
    assert!(result.helper_started);
    let (private_fd, preserves_stdio, helper, argv) = backend.plan().expect("launch plan");
    assert!(private_fd > 2);
    assert!(!preserves_stdio);
    assert_eq!(helper, "d2b-bazel-exec-supervisor");
    assert_eq!(
        argv,
        vec![OsString::from("target"), OsString::from("--closed")]
    );
    fs::remove_file(path).expect("remove provider");
}

#[test]
fn consuming_api_keeps_the_open_file_description_after_path_rebind() {
    let (file, path) = temporary_provider("rebind", b"verified-provider");
    let replacement = path.with_extension("replacement");
    fs::write(&replacement, b"replacement").expect("replacement bytes");
    let executable = test_support::verified_file(file);
    fs::rename(&replacement, &path).expect("rebind path");

    let backend = FakeBackend::passing();
    test_support::execute_verified_with_backend(executable, ExecutionRequest::default(), &backend)
        .expect("injected backend");
    assert!(backend.plan().expect("launch plan").0 > 2);
    fs::remove_file(path).expect("remove replacement");
}

#[test]
fn production_adapter_rejects_the_fixture_terminal_frame_without_false_success() {
    if option_env!("D2B_BAZEL_EXEC_SUPERVISOR").is_some() {
        let target = std::env::var_os("D2B_REAL_EXEC_TARGET").expect("real target contract");
        let executable = test_support::verified_file(File::open(target).expect("target"));
        let error = d2b_bazel_exec::execute_verified(
            executable,
            ExecutionRequest {
                target_argv: vec![OsString::from("false")],
                ..ExecutionRequest::default()
            },
        )
        .expect_err("malformed terminal frame must remain typed");
        assert_eq!(
            error,
            d2b_bazel_exec::HandoffError::Protocol(d2b_bazel_exec::ProtocolError::InvalidLength)
        );
        return;
    }
    let (file, path) = temporary_provider("missing-helper", b"verified-provider");
    let executable = test_support::verified_file(file);
    let error = d2b_bazel_exec::execute_verified(executable, ExecutionRequest::default())
        .expect_err("local builds must refuse without the Nix identity");
    assert_eq!(error.code(), "D2B-BZLEXEC-PARENT-HELPER-IDENTITY");
    fs::remove_file(path).expect("remove provider");
}

#[test]
fn unknown_child_error_codes_are_rejected_without_widening_the_enum() {
    let unknown = [b'D', b'2', b'B', b'E', 1, 1, 0, 9];
    assert_eq!(
        decode_exec_error(&unknown, true),
        Err(ProtocolError::UnknownChildCode)
    );
    let reserved = [b'D', b'2', b'B', b'E', 1, 1, 1, 1];
    assert_eq!(
        decode_exec_error(&reserved, true),
        Err(ProtocolError::ExecErrorUnknown)
    );
}

#[test]
fn empty_target_argv_is_rejected_before_helper_spawn() {
    let (file, path) = temporary_provider("empty-argv", b"verified-provider");
    let executable = test_support::verified_file(file);
    let error = d2b_bazel_exec::execute_verified(
        executable,
        ExecutionRequest {
            target_argv: Vec::new(),
            ..ExecutionRequest::default()
        },
    )
    .expect_err("empty target argv");
    assert_eq!(
        error,
        d2b_bazel_exec::HandoffError::Backend(BackendError::TargetArguments)
    );
    fs::remove_file(path).expect("remove provider");
}
