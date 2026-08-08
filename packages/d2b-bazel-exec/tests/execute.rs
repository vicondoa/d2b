use std::{
    fs,
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

use d2b_bazel_exec::provider::verify_provider;
use d2b_bazel_exec::{
    BackendError, ExecutionBackend, ExecutionRequest, HandoffError, LaunchCoordinator,
    MaskSnapshot, SpawnReceipt, StdioPolicy, execute_verified, run_signal_handoff,
};
use d2b_bazel_support::fsops::{Digest, FileSystem, HostFileSystem, OpenFlags, ResolvePolicy};

type PlanObservation = (i32, bool, &'static str);

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
        *self.saw_plan.lock().expect("plan")
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

    fn spawn(
        &self,
        plan: d2b_bazel_exec::execute::LaunchPlan,
    ) -> Result<SpawnReceipt, BackendError> {
        self.events.lock().expect("events").push("spawn");
        *self.saw_plan.lock().expect("plan") = Some((
            plan.private_fd_number(),
            plan.preserves_standard_streams(),
            plan.supervisor().label(),
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
    let result = run_signal_handoff(&coordinator, &backend, || {
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
    let error = run_signal_handoff(
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
    let error = run_signal_handoff(
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
    let error =
        run_signal_handoff(&coordinator, &backend, || backend.spawn).expect_err("spawn failure");
    assert_eq!(error.code(), "D2B-BZLEXEC-PARENT-SPAWN");
    assert_eq!(backend.event_names(), ["capture", "block", "restore"]);

    let coordinator = LaunchCoordinator::new();
    let mut backend = FakeBackend::passing();
    backend.restore = Err(BackendError::Restore);
    let error = run_signal_handoff(&coordinator, &backend, || Ok(SpawnReceipt::started()))
        .expect_err("restore failure after success");
    assert_eq!(error.code(), "D2B-BZLEXEC-PARENT-SIGNAL-HANDOFF");

    let coordinator = LaunchCoordinator::new();
    let mut backend = FakeBackend::passing();
    backend.spawn = Err(BackendError::Spawn);
    backend.restore = Err(BackendError::Restore);
    let error = run_signal_handoff(&coordinator, &backend, || backend.spawn)
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
    let error = run_signal_handoff(
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
        run_signal_handoff(&first_coordinator, &first, || {
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
        let result = run_signal_handoff(&second_coordinator, &second, || {
            second.events.lock().expect("events").push("spawn");
            Ok(SpawnReceipt::started())
        });
        second_started_clone.store(true, Ordering::Release);
        (result, second)
    });
    thread::sleep(Duration::from_millis(20));
    assert!(
        !second_started.load(Ordering::Acquire),
        "second launch cannot pass the process-wide guard"
    );
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

fn verified_temp_file() -> (d2b_bazel_exec::VerifiedExecutable, PathBuf) {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("d2b-bazel-exec-{suffix}"));
    fs::write(&path, b"verified-provider").expect("provider bytes");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("provider mode");
    let filesystem = HostFileSystem::new();
    let parent = filesystem
        .open(
            path.parent().expect("temp parent"),
            OpenFlags::RDONLY | OpenFlags::DIRECTORY | OpenFlags::CLOEXEC,
            ResolvePolicy::Strict,
        )
        .expect("provider parent");
    let provider = filesystem
        .open_provider(
            &parent,
            std::path::Path::new(path.file_name().expect("provider name")),
        )
        .expect("provider descriptor");
    let executable = verify_provider(
        &filesystem,
        provider,
        None,
        Digest::sha256(b"verified-provider"),
    )
    .expect("verified capability");
    (executable, path)
}

#[test]
fn consuming_api_maps_the_same_verified_open_file_description_and_preserves_stdio() {
    let (executable, path) = verified_temp_file();
    let backend = FakeBackend::passing();
    let result = execute_verified(executable, ExecutionRequest::default(), &backend)
        .expect("injected backend");
    assert!(result.helper_started);
    let (private_fd, preserves_stdio, helper) = backend.plan().expect("launch plan");
    assert!(private_fd > 2);
    assert!(preserves_stdio);
    assert_eq!(helper, "d2b-bazel-exec-supervisor");
    fs::remove_file(path).expect("remove provider");
}

#[test]
fn consuming_api_keeps_declared_nondefault_stdio_in_the_plan() {
    let (executable, path) = verified_temp_file();
    let backend = FakeBackend::passing();
    let request = ExecutionRequest {
        stdin: StdioPolicy::Null,
        stdout: StdioPolicy::Inherit,
        stderr: StdioPolicy::Inherit,
    };
    execute_verified(executable, request, &backend).expect("injected backend");
    assert!(!backend.plan().expect("plan").1);
    fs::remove_file(path).expect("remove provider");
}

#[test]
fn private_mapping_keeps_the_verified_open_file_description_after_path_rebind() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("d2b-bazel-exec-rebind-{suffix}"));
    let replacement = path.with_extension("replacement");
    fs::write(&path, b"verified-provider").expect("provider bytes");
    fs::write(&replacement, b"replacement").expect("replacement bytes");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("provider mode");
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o755)).expect("replacement mode");
    let filesystem = HostFileSystem::new();
    let parent = filesystem
        .open(
            path.parent().expect("temp parent"),
            OpenFlags::RDONLY | OpenFlags::DIRECTORY | OpenFlags::CLOEXEC,
            ResolvePolicy::Strict,
        )
        .expect("parent");
    let provider = filesystem
        .open_provider(
            &parent,
            std::path::Path::new(path.file_name().expect("provider name")),
        )
        .expect("provider");
    let before = filesystem
        .fstat(provider.descriptor())
        .expect("provider stat");
    let mapped = provider
        .duplicate_for_mapping()
        .expect("private cloexec mapping");
    fs::rename(&replacement, &path).expect("rebind path");
    let mapped_stat = rustix::fs::fstat(&mapped).expect("mapped stat");
    assert_eq!(mapped_stat.st_ino, before.inode());
    assert!(provider.is_close_on_exec().expect("provider flags"));
    fs::remove_file(path).expect("remove replacement");
}
