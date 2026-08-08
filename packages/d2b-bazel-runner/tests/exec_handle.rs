#[path = "../src/exec_handle.rs"]
mod exec_handle;

use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use d2b_bazel_exec::{
    BackendError, ExecutionBackend, ExecutionRequest, MaskSnapshot, SpawnReceipt, StdioPolicy,
    provider::verify_provider,
};
use d2b_bazel_support::fsops::{Digest, FileSystem, HostFileSystem, OpenFlags, ResolvePolicy};

#[derive(Default)]
struct RecordingBackend {
    events: Mutex<Vec<&'static str>>,
    requests: Mutex<Vec<ExecutionRequest>>,
    descriptor_identities: Mutex<Vec<(u64, u64)>>,
    supervisors: Mutex<Vec<(&'static str, bool)>>,
}

impl RecordingBackend {
    fn events(&self) -> Vec<&'static str> {
        self.events.lock().expect("events").clone()
    }

    fn request(&self) -> Option<ExecutionRequest> {
        self.requests.lock().expect("requests").last().copied()
    }

    fn descriptor_identity(&self) -> Option<(u64, u64)> {
        self.descriptor_identities
            .lock()
            .expect("descriptor identities")
            .last()
            .copied()
    }

    fn supervisor(&self) -> Option<(&'static str, bool)> {
        self.supervisors
            .lock()
            .expect("supervisors")
            .last()
            .copied()
    }
}

impl ExecutionBackend for RecordingBackend {
    fn capture_mask(&self) -> Result<MaskSnapshot, BackendError> {
        self.events.lock().expect("events").push("capture");
        Ok(MaskSnapshot::Test(17))
    }

    fn block_managed(&self) -> Result<(), BackendError> {
        self.events.lock().expect("events").push("block");
        Ok(())
    }

    fn restore_mask(&self, snapshot: MaskSnapshot) -> Result<(), BackendError> {
        assert_eq!(snapshot, MaskSnapshot::Test(17));
        self.events.lock().expect("events").push("restore");
        Ok(())
    }

    fn spawn(
        &self,
        plan: d2b_bazel_exec::execute::LaunchPlan,
    ) -> Result<SpawnReceipt, BackendError> {
        self.events.lock().expect("events").push("spawn");
        let descriptor = plan.private_fd_number();
        assert!(descriptor > 2);
        let identity = fs::metadata(PathBuf::from("/dev/fd").join(descriptor.to_string()))
            .map_err(|_| BackendError::Mapping)?;
        self.descriptor_identities
            .lock()
            .expect("descriptor identities")
            .push((identity.dev(), identity.ino()));
        let request = plan.request();
        self.requests.lock().expect("requests").push(request);
        let supervisor = plan.supervisor();
        self.supervisors
            .lock()
            .expect("supervisors")
            .push((supervisor.label(), supervisor.is_immutable()));
        Ok(SpawnReceipt::started())
    }
}

fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "d2b-bazel-runner-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&path).expect("temporary directory");
    path
}

fn make_executable(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("provider bytes");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("provider mode");
}

fn verify_path(
    path: &Path,
    expected_bytes: &[u8],
) -> (d2b_bazel_exec::VerifiedExecutable, (u64, u64)) {
    let filesystem = HostFileSystem::new();
    let anchor = filesystem
        .open(
            path.parent().expect("provider parent"),
            OpenFlags::RDONLY | OpenFlags::DIRECTORY | OpenFlags::CLOEXEC,
            ResolvePolicy::Strict,
        )
        .expect("provider parent");
    let provider = filesystem
        .open_provider(&anchor, Path::new(path.file_name().expect("provider name")))
        .expect("provider descriptor");
    let metadata = filesystem
        .fstat(provider.descriptor())
        .expect("provider metadata");
    let verified = verify_provider(&filesystem, provider, None, Digest::sha256(expected_bytes))
        .expect("verified provider");
    (verified, (metadata.device(), metadata.inode()))
}

#[test]
fn transfers_original_verified_descriptor_after_path_rebind_and_mutation() {
    let directory = temporary_directory("rebind");
    let provider_path = directory.join("provider");
    let replacement_path = directory.join("replacement");
    make_executable(&provider_path, b"original-provider");
    make_executable(&replacement_path, b"replacement-provider");

    let (verified, original_identity) = verify_path(&provider_path, b"original-provider");
    let replacement_identity = fs::metadata(&replacement_path).expect("replacement metadata");
    fs::rename(&replacement_path, &provider_path).expect("rebind provider path");
    fs::write(&provider_path, b"mutated-replacement").expect("mutate rebound path");
    let rebound_identity = fs::metadata(&provider_path).expect("rebound metadata");
    assert_ne!(
        (rebound_identity.dev(), rebound_identity.ino()),
        original_identity,
        "the declared path must no longer name the verified inode"
    );
    assert_eq!(
        (rebound_identity.dev(), rebound_identity.ino()),
        (replacement_identity.dev(), replacement_identity.ino())
    );

    let backend = RecordingBackend::default();
    exec_handle::execute(verified, ExecutionRequest::default(), &backend)
        .expect("typed consumer execution");

    assert_eq!(
        backend.descriptor_identity(),
        Some(original_identity),
        "the adapter must transfer the verified open file description"
    );
    assert_eq!(backend.events(), ["capture", "block", "spawn", "restore"]);

    fs::remove_file(provider_path).expect("remove rebound provider");
    fs::remove_dir(directory).expect("remove temporary directory");
}

#[test]
fn preserves_declared_stdin_and_distinct_stdout_stderr_through_typed_consumer() {
    let directory = temporary_directory("stdio");
    let provider_path = directory.join("provider");
    make_executable(&provider_path, b"stdio-provider");
    let (verified, _) = verify_path(&provider_path, b"stdio-provider");

    let request = ExecutionRequest {
        stdin: StdioPolicy::Null,
        stdout: StdioPolicy::Inherit,
        stderr: StdioPolicy::Null,
    };
    let backend = RecordingBackend::default();
    let result = exec_handle::execute(verified, request, &backend).expect("typed consumer");

    assert!(result.helper_started);
    assert_eq!(backend.request(), Some(request));
    assert_eq!(
        backend.supervisor(),
        Some(("d2b-bazel-exec-supervisor", true))
    );
    assert_eq!(backend.events(), ["capture", "block", "spawn", "restore"]);

    fs::remove_file(provider_path).expect("remove provider");
    fs::remove_dir(directory).expect("remove temporary directory");
}

#[test]
fn adapter_delegates_once_to_the_typed_consumer_and_not_to_the_helper() {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/exec_handle.rs");
    let source = fs::read_to_string(source_path).expect("adapter source");

    assert_eq!(
        source.matches("d2b_bazel_exec::execute_verified").count(),
        1,
        "the adapter has one exact typed-consumer delegation"
    );
    for forbidden in [
        "Command::new",
        "std::process::Command",
        "IMMUTABLE_SUPERVISOR_PATH",
        "d2b-bazel-exec-supervisor",
        "CARGO_BIN_EXE_",
        "RUNFILES",
        "worktree",
    ] {
        assert!(
            !source.contains(forbidden),
            "the adapter must not contain direct helper or path execution: {forbidden}"
        );
    }
}
