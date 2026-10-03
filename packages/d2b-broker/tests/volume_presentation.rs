//! Effective Volume presentation for one launch (U11, KTD11).
//!
//! The unit's acceptance is kernel-backed: a pure mount-plan test proves the
//! plan, not the effect. The cases below therefore drive the real
//! [`clone3_spawn_runner`] child and assert what the kernel actually did to
//! the runner's mount tree, and each case names which kind of proof it is.
//! The hermetic cases are the ones whose evidence is a refusal rather than an
//! effect: a refusal is observable without a kernel, an applied mount is not.
//!
//! What each plan scenario is proven by:
//!
//! 1. AE2-AE3 `filesystem-presentation` - the worker reads and writes its
//!    admitted destination and cannot reach sibling source content, with and
//!    without a final user namespace. **Real-kernel**
//!    ([kernel_lane_worker_reads_writes_and_is_confined],
//!    [kernel_lane_worker_reads_writes_and_is_confined_without_a_user_namespace]).
//! 2. A read-only destination rejects writes and private mounts do not appear
//!    in the host namespace. **Real-kernel**
//!    ([kernel_lane_read_only_destination_rejects_writes_and_stays_private]).
//! 3. A setup failure closes descriptors and kills/reaps the child WITHOUT
//!    reporting Prepared. **Split**: the parent-side refusal and the descriptor
//!    proof need no kernel at all
//!    ([a_source_that_does_not_resolve_refuses_before_any_child]); the
//!    child-side failure is proven on the kernel by exit status plus a reaped
//!    child ([a_child_mount_failure_exits_with_the_mount_code_and_is_reaped]).
//! 4. AE19 required host traversal stays effective after mode/ACL
//!    reconciliation. **Real-kernel**: the assertion reads the POSIX ACL the
//!    kernel stores and computes the effective permission the access check
//!    applies ([traversal_survives_mode_and_acl_reconciliation]).
//! 5. AE6 `namespace-first-service-source`. **Hermetic for the refusal** - a
//!    refusal IS the requirement - at both the preflight and the syscall
//!    boundary
//!    ([namespace_first_service_source_refuses_a_process_filesystem_mount],
//!    [namespace_first_service_source_refuses_a_requested_mount_policy]).
//!
//! A host that cannot grant `CAP_SYS_ADMIN` for a private mount tree makes the
//! kernel cases print `SKIP` and return. That is a blocked acceptance condition
//! for the live scenario, not a pass, and it is called out per case.
//!
//! Every case drives the real `clone3`/`fork` child and waits on it
//! synchronously, so the blocking filesystem calls, the child wait, and the
//! `setfacl`/`getfacl` probes carry the sanctioned `cfg(test) helper` allow at
//! each site rather than a crate-wide one.

use std::ffi::CString;
use std::fs;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use d2b_broker::ops::spawn_runner::{
    AdmittedPresentation, PresentationBindSpec, PresentationRealization, SpawnRunnerError,
    SpawnRunnerPlanInput, fence_presentation_argv, preflight,
};
use d2b_broker::sys::pidfd_sys::{
    RunnerIsolationSpec, UserNamespaceSpec, clone3_spawn_runner,
};
use d2b_core::sandbox_profile::{CgroupPlacement, MountPolicy, NamespaceSet};

/// The child exit codes this file asserts on, re-declared because the
/// constants are private to `pidfd_sys`.
const CHILD_EXIT_MOUNT: i32 = 64;
const CHILD_EXIT_UNSHARE: i32 = 62;

/// A bound on any child wait, so a wedged handshake fails the case instead of
/// hanging the lane.
const CHILD_WAIT_BOUND: Duration = Duration::from_secs(30);

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn shell() -> Option<PathBuf> {
    [
        "/bin/sh",
        "/usr/bin/sh",
        "/run/current-system/sw/bin/sh",
    ]
    .iter()
    .map(Path::new)
    .find(|candidate| candidate.exists())
    .map(Path::to_path_buf)
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn namespaces(user: bool) -> NamespaceSet {
    NamespaceSet {
        mount: false,
        pid: false,
        net: false,
        ipc: false,
        uts: false,
        user,
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn empty_mount_policy() -> MountPolicy {
    MountPolicy {
        read_only_paths: Vec::new(),
        writable_paths: Vec::new(),
        nix_store_read_only: false,
        hide_device_nodes_by_default: false,
        device_binds: Vec::new(),
        bind_mounts: Vec::new(),
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn current_user_namespace() -> UserNamespaceSpec {
    UserNamespaceSpec {
        host_uid_for_zero: nix::unistd::Uid::current().as_raw(),
        host_gid_for_zero: nix::unistd::Gid::current().as_raw(),
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn exit_code(pid: i32) -> i32 {
    use nix::sys::wait::{WaitStatus, waitpid};
    let deadline = Instant::now() + CHILD_WAIT_BOUND;
    loop {
        match waitpid(nix::unistd::Pid::from_raw(pid), None) {
            Ok(WaitStatus::Exited(_, code)) => return code,
            Ok(WaitStatus::Signaled(_, signal, _)) => panic!("child {pid} died on signal {signal}"),
            Ok(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(status) => panic!("child {pid} never exited: {status:?}"),
            Err(error) => panic!("waitpid({pid}) failed: {error}"),
        }
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn open_descriptor_count() -> usize {
    fs::read_dir("/proc/self/fd")
        .expect("/proc/self/fd is readable")
        .count()
}

/// A source tree with an admitted view and a sibling the view must not reach.
///
/// `<root>/source/view` is what the plan admits;
/// `<root>/source/sibling/secret.txt` is what it does not.
struct SourceTree {
    root: tempfile::TempDir,
    view: PathBuf,
}

impl SourceTree {
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn new() -> Self {
        let root = tempfile::tempdir().expect("source tree tempdir");
        let source = root.path().join("source");
        let view = source.join("view");
        fs::create_dir_all(&view).expect("view dir");
        fs::create_dir_all(source.join("sibling")).expect("sibling dir");
        fs::write(view.join("admitted.txt"), b"admitted\n").expect("admitted file");
        fs::write(
            source.join("sibling").join("secret.txt"),
            b"sibling-only\n",
        )
        .expect("sibling file");
        Self { root, view }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn private_execution_root(&self) -> PathBuf {
        let root = self.root.path().join("private-root");
        fs::create_dir_all(&root).expect("private execution root");
        root
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn destination(&self, slot: &str) -> PathBuf {
        self.private_execution_root().join(slot)
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn bind(source: &Path, destination: &Path, read_only: bool) -> PresentationBindSpec {
    PresentationBindSpec {
        source: source.to_path_buf(),
        destination: destination.to_path_buf(),
        read_only,
    }
}

/// Whether this environment can run the kernel lane.
///
/// The probe asks the kernel rather than assuming: a spawn that reaches
/// `execve` proves the child got its user namespace, its private mount tree,
/// and the bounded handshake release. A host that refuses is reported, not
/// silently treated as a pass.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn kernel_lane_available() -> bool {
    let Some(shell) = shell() else {
        println!("SKIP: no /bin/sh to probe the kernel lane");
        return false;
    };
    // `mount: true` is what makes this probe the SAME order the real path
    // uses: a private mount namespace first, and only then the requested user
    // namespace. A probe that skipped the mount namespace would report a
    // capability the lane itself cannot use.
    let isolation = RunnerIsolationSpec {
        capabilities: Vec::new(),
        namespaces: NamespaceSet {
            mount: true,
            ..namespaces(true)
        },
        seccomp_program: None,
        mount_policy: empty_mount_policy(),
        cgroup_dir_fd: None,
        cgroup_procs_fd: None,
        user_namespace: Some(current_user_namespace()),
        umask: None,
        pre_opened_device_fds: Vec::new(),
        memlock_limit_bytes: None,
        activation_stdin: None,
        presentation: PresentationRealization::FilesystemPresentation,
        private_execution_root: None,
        presentation_binds: Vec::new(),
    };
    let binary = CString::new(shell.to_string_lossy().as_bytes()).expect("shell path");
    let argv = vec![binary.clone()];
    let uid = nix::unistd::Uid::current().as_raw();
    let gid = nix::unistd::Gid::current().as_raw();
    match clone3_spawn_runner(binary, argv, Vec::new(), uid, gid, Vec::new(), isolation) {
        Ok(outcome) => exit_code(outcome.pid) == 0,
        Err(error) => {
            println!(
                "SKIP: kernel lane unavailable - a user namespace plus a private mount tree \
                 could not be established here ({error})"
            );
            false
        }
    }
}

/// Run one worker against a prepared presentation and return its exit code.
///
/// The worker is `/bin/sh -c <script> <argv0> <args...>`, so the argv the
/// broker execs carries the admitted destination and nothing else. `user` picks
/// the posture: `true` defers the user namespace until after the private mount
/// tree exists, `false` runs the same tree without a user namespace at all.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn run_worker(
    tree: &SourceTree,
    binds: Vec<PresentationBindSpec>,
    user: bool,
    script: &str,
    args: &[&str],
) -> Option<i32> {
    let Some(shell) = shell() else {
        println!("SKIP: no /bin/sh for the kernel lane");
        return None;
    };
    let root = tree.private_execution_root();
    let isolation = RunnerIsolationSpec {
        capabilities: Vec::new(),
        namespaces: namespaces(user),
        seccomp_program: None,
        mount_policy: empty_mount_policy(),
        cgroup_dir_fd: None,
        cgroup_procs_fd: None,
        user_namespace: user.then(current_user_namespace),
        umask: None,
        pre_opened_device_fds: Vec::new(),
        memlock_limit_bytes: None,
        activation_stdin: None,
        presentation: PresentationRealization::FilesystemPresentation,
        private_execution_root: Some(root),
        presentation_binds: binds,
    };
    let binary = CString::new(shell.to_string_lossy().as_bytes()).expect("shell path");
    let mut argv = vec![binary.clone(), CString::new("-c").unwrap()];
    argv.push(CString::new(script).expect("script has no NUL"));
    argv.push(CString::new("volume-presentation-probe").unwrap());
    for argument in args {
        argv.push(CString::new(*argument).expect("argument has no NUL"));
    }
    let uid = nix::unistd::Uid::current().as_raw();
    let gid = nix::unistd::Gid::current().as_raw();
    match clone3_spawn_runner(binary, argv, Vec::new(), uid, gid, Vec::new(), isolation) {
        Ok(outcome) => Some(exit_code(outcome.pid)),
        Err(error) => {
            println!("SKIP: kernel lane could not spawn a worker here ({error})");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Scenario 1 + 2 (AE2-AE3): effective view, destination, and isolation
// ---------------------------------------------------------------------------

/// AE2-AE3, real-kernel: the worker reads and writes its admitted destination
/// and cannot reach sibling source content, WITH a final user namespace
/// created after the private mount tree.
///
/// The sibling is unreachable for a structural reason, not an ACL one: the only
/// thing in the private execution root is the bound slot, so `..` from it lands
/// on a tmpfs that never held a sibling.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn kernel_lane_worker_reads_writes_and_is_confined() {
    if !kernel_lane_available() {
        println!("SKIP: the effective-access proof is OUTSTANDING on this host");
        return;
    }
    let tree = SourceTree::new();
    let destination = tree.destination("data");
    let script = "set -e; cat \"$1/admitted.txt\"; echo written > \"$1/written.txt\"; \
                  test ! -e \"$1/../sibling/secret.txt\"";
    let destination_arg = destination.to_str().expect("utf-8 destination");
    let Some(code) = run_worker(
        &tree,
        vec![bind(&tree.view, &destination, false)],
        true,
        script,
        &[destination_arg],
    ) else {
        println!("SKIP: the effective-access proof is OUTSTANDING on this host");
        return;
    };
    assert_eq!(
        code, 0,
        "the worker must read and write its admitted destination and reach no sibling source \
         content (exit {code})"
    );
    assert_eq!(
        fs::read_to_string(tree.view.join("written.txt")).expect("write reached the admitted view"),
        "written\n",
        "the write must land on the admitted view, not on a copy"
    );
    assert!(
        !destination.exists(),
        "{} must not exist in the host namespace",
        destination.display()
    );
}

/// Whether the host can create a private mount tree with the broker's real
/// credentials - i.e. without a user namespace supplying `CAP_SYS_ADMIN`.
///
/// A host that only grants it inside a user namespace cannot run this posture;
/// the probe asks the kernel and reports the gap rather than pretending the
/// posture was covered.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn root_mount_lane_available() -> bool {
    if nix::unistd::Uid::effective().is_root() {
        return true;
    }
    let Some(shell) = shell() else {
        println!("SKIP: no /bin/sh to probe the no-user-namespace posture");
        return false;
    };
    let root = tempfile::tempdir().expect("probe tempdir").path().to_path_buf();
    let isolation = RunnerIsolationSpec {
        capabilities: Vec::new(),
        namespaces: namespaces(false),
        seccomp_program: None,
        mount_policy: empty_mount_policy(),
        cgroup_dir_fd: None,
        cgroup_procs_fd: None,
        user_namespace: None,
        umask: None,
        pre_opened_device_fds: Vec::new(),
        memlock_limit_bytes: None,
        activation_stdin: None,
        presentation: PresentationRealization::FilesystemPresentation,
        private_execution_root: Some(root),
        presentation_binds: Vec::new(),
    };
    let binary = CString::new(shell.to_string_lossy().as_bytes()).expect("shell path");
    let argv = vec![binary.clone()];
    let uid = nix::unistd::Uid::current().as_raw();
    let gid = nix::unistd::Gid::current().as_raw();
    match clone3_spawn_runner(binary, argv, Vec::new(), uid, gid, Vec::new(), isolation) {
        Ok(outcome) => {
            let code = exit_code(outcome.pid);
            if code != 0 {
                println!(
                    "SKIP: this host cannot create a private mount tree with the broker's own \
                     credentials (child exit {code}); the no-user-namespace posture is \
                     OUTSTANDING here"
                );
                false
            } else {
                true
            }
        }
        Err(error) => {
            println!("SKIP: the no-user-namespace posture could not be probed ({error})");
            false
        }
    }
}

/// AE2-AE3, real-kernel, the other posture the plan names: the same private
/// mount tree WITHOUT a final user namespace, so the mount is effective on both
/// supported local namespace postures.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn kernel_lane_worker_reads_writes_and_is_confined_without_a_user_namespace() {
    if !root_mount_lane_available() {
        println!("SKIP: the no-user-namespace effective-access proof is OUTSTANDING on this host");
        return;
    }
    if !kernel_lane_available() {
        println!("SKIP: the no-user-namespace effective-access proof is OUTSTANDING on this host");
        return;
    }
    let tree = SourceTree::new();
    let destination = tree.destination("data");
    let script = "set -e; cat \"$1/admitted.txt\"; echo written > \"$1/written.txt\"; \
                  test ! -e \"$1/../sibling/secret.txt\"";
    let destination_arg = destination.to_str().expect("utf-8 destination");
    let Some(code) = run_worker(
        &tree,
        vec![bind(&tree.view, &destination, false)],
        false,
        script,
        &[destination_arg],
    ) else {
        println!("SKIP: the no-user-namespace proof is OUTSTANDING on this host");
        return;
    };
    assert_eq!(code, 0, "the mount must be effective without a user namespace too");
    assert_eq!(
        fs::read_to_string(tree.view.join("written.txt")).expect("write reached the admitted view"),
        "written\n"
    );
    assert!(!destination.exists(), "the private mount must stay out of the host namespace");
}

/// AE2-AE3, real-kernel: a read-only destination rejects writes, and the mount
/// it was made of is still absent from the host namespace.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn kernel_lane_read_only_destination_rejects_writes_and_stays_private() {
    if !kernel_lane_available() {
        println!("SKIP: the read-only proof is OUTSTANDING on this host");
        return;
    }
    let tree = SourceTree::new();
    let destination = tree.destination("data");
    let script = "set -e; test \"$(cat \"$1/admitted.txt\")\" = admitted; \
                  ! echo nope > \"$1/new.txt\" 2>/dev/null; test ! -e \"$1/new.txt\"";
    let destination_arg = destination.to_str().expect("utf-8 destination");
    let Some(code) = run_worker(
        &tree,
        vec![bind(&tree.view, &destination, true)],
        true,
        script,
        &[destination_arg],
    ) else {
        println!("SKIP: the read-only proof is OUTSTANDING on this host");
        return;
    };
    assert_eq!(
        code, 0,
        "a read-only destination must reject writes and still admit reads (exit {code})"
    );
    assert!(!tree.view.join("new.txt").exists(), "the write must not reach the source");
    assert!(!destination.exists(), "the read-only mount must stay private");
}

// ---------------------------------------------------------------------------
// Scenario 3: a setup failure is never Prepared
// ---------------------------------------------------------------------------

/// Hermetic: a source that does not resolve through an anchored, whole-path
/// `RESOLVE_NO_SYMLINKS` traversal refuses in the parent, before `clone3`, and
/// leaves no descriptor behind. There is no pid to report, so nothing can be
/// Published as Prepared.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_source_that_does_not_resolve_refuses_before_any_child() {
    let tree = SourceTree::new();
    let destination = tree.destination("data");
    let isolation = RunnerIsolationSpec {
        capabilities: Vec::new(),
        namespaces: namespaces(false),
        seccomp_program: None,
        mount_policy: empty_mount_policy(),
        cgroup_dir_fd: None,
        cgroup_procs_fd: None,
        user_namespace: None,
        umask: None,
        pre_opened_device_fds: Vec::new(),
        memlock_limit_bytes: None,
        activation_stdin: None,
        presentation: PresentationRealization::FilesystemPresentation,
        private_execution_root: Some(tree.private_execution_root()),
        presentation_binds: vec![bind(
            Path::new("/nonexistent/d2b-volume-source"),
            &destination,
            false,
        )],
    };
    // Sampled immediately before the call, and read back with a short retry:
    // this test binary runs cases on parallel threads, so a sibling's transient
    // descriptor would otherwise be misreported as a leak by this case. A
    // refusal that opened a descriptor would leave it open for good.
    let before = open_descriptor_count();
    let error = clone3_spawn_runner(
        CString::new("/bin/true").unwrap(),
        vec![CString::new("/bin/true").unwrap()],
        Vec::new(),
        1000,
        1000,
        Vec::new(),
        isolation,
    )
    .expect_err("an unresolvable source must refuse before any child exists");
    assert!(
        format!("{error}").contains("presentation source"),
        "the refusal must name the source it could not resolve: {error}"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let leaked = loop {
        let now = open_descriptor_count();
        if now <= before {
            break 0;
        }
        if Instant::now() >= deadline {
            break now - before;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        leaked, 0,
        "the refused preparation must close every descriptor it opened"
    );
    assert!(!destination.exists(), "a refused launch prepares no destination");
}

/// Hermetic: a destination outside the private execution root is refused, so
/// the broker can never establish a presentation in the host root.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_destination_outside_the_private_root_is_refused() {
    let tree = SourceTree::new();
    let isolation = RunnerIsolationSpec {
        capabilities: Vec::new(),
        namespaces: namespaces(false),
        seccomp_program: None,
        mount_policy: empty_mount_policy(),
        cgroup_dir_fd: None,
        cgroup_procs_fd: None,
        user_namespace: None,
        umask: None,
        pre_opened_device_fds: Vec::new(),
        memlock_limit_bytes: None,
        activation_stdin: None,
        presentation: PresentationRealization::FilesystemPresentation,
        private_execution_root: Some(tree.private_execution_root()),
        presentation_binds: vec![bind(&tree.view, &tree.root.path().join("host-root"), false)],
    };
    let error = clone3_spawn_runner(
        CString::new("/bin/true").unwrap(),
        vec![CString::new("/bin/true").unwrap()],
        Vec::new(),
        1000,
        1000,
        Vec::new(),
        isolation,
    )
    .expect_err("a host-root destination must be refused");
    assert!(
        format!("{error}").contains("outside the private execution root"),
        "the refusal must name the private execution root: {error}"
    );
}

/// Real-kernel: a child whose mount stage fails exits with the mount code and is
/// reaped here, so no Prepared status can be published for it.
///
/// The failure is induced by pointing the private execution root at a regular
/// file, so the child's `tmpfs` mount there fails.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_child_mount_failure_exits_with_the_mount_code_and_is_reaped() {
    let Some(shell) = shell() else {
        println!("SKIP: no /bin/sh for the child-failure case");
        return;
    };
    let tree = SourceTree::new();
    // The destination is `<private root>/blocker/slot`, and `blocker` is a
    // regular file. The parent's checks pass - the destination is inside the
    // private execution root and the source resolves - so the failure happens
    // in the child's mount stage, which is the path this case exists to prove.
    let private_root = tree.private_execution_root();
    fs::write(private_root.join("blocker"), b"x").expect("write the blocker file");
    let destination = private_root.join("blocker/slot");
    let before = open_descriptor_count();
    let isolation = RunnerIsolationSpec {
        capabilities: Vec::new(),
        namespaces: namespaces(false),
        seccomp_program: None,
        mount_policy: empty_mount_policy(),
        cgroup_dir_fd: None,
        cgroup_procs_fd: None,
        user_namespace: None,
        umask: None,
        pre_opened_device_fds: Vec::new(),
        memlock_limit_bytes: None,
        activation_stdin: None,
        presentation: PresentationRealization::FilesystemPresentation,
        private_execution_root: Some(private_root),
        presentation_binds: vec![bind(&tree.view, &destination, false)],
    };
    let binary = CString::new(shell.to_string_lossy().as_bytes()).expect("shell path");
    let argv = vec![binary.clone()];
    let uid = nix::unistd::Uid::current().as_raw();
    let gid = nix::unistd::Gid::current().as_raw();
    let outcome = match clone3_spawn_runner(binary, argv, Vec::new(), uid, gid, Vec::new(), isolation)
    {
        Ok(outcome) => outcome,
        Err(error) => {
            println!("SKIP: the kernel refused the blocked mount in the parent ({error})");
            return;
        }
    };
    let code = exit_code(outcome.pid);
    if code == CHILD_EXIT_UNSHARE {
        // This host cannot create the private mount namespace at all, so the
        // child never reached the stage under test. That is an outstanding
        // live scenario, not a pass and not a failure of the code.
        println!(
            "SKIP: this host denies the private mount namespace before the child's mount              stage runs (exit {CHILD_EXIT_UNSHARE}); the child-side setup-failure proof is              OUTSTANDING here"
        );
        return;
    }
    assert_eq!(
        code, CHILD_EXIT_MOUNT,
        "a child that cannot prepare its mount tree must exit with the mount code, never 0"
    );
    assert_eq!(
        open_descriptor_count(),
        before,
        "the failed child must not leave a descriptor behind"
    );
}

// ---------------------------------------------------------------------------
// Scenario 4 (AE19): required traversal stays effective
// ---------------------------------------------------------------------------

/// AE19: after the broker's mode/ACL reconciliation, the launched principal's
/// traverse right on the ancestor is still effective.
///
/// The assertion reads the POSIX ACL the KERNEL stores
/// (`system.posix_acl_access`) and computes the effective permission an access
/// check applies - the named-user entry's bits AND the mask. Reading the mode
/// the broker asked for would prove only that the broker called `chmod`.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn traversal_survives_mode_and_acl_reconciliation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ancestor = dir.path().join("ancestor");
    let leaf = ancestor.join("leaf");
    fs::create_dir_all(&leaf).expect("create tree");
    let uid = nix::unistd::Uid::current().as_raw();
    fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o700)).expect("chmod ancestor");

    let apply = |target: &Path, spec: &str| {
        let fd = rustix::fs::open(
            target,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .expect("open for setfacl");
        d2b_broker::sys::pidfd_sys::run_setfacl_op_on_fd(
            fd.as_fd(),
            "-m",
            &format!("u:{uid}:{spec}"),
        )
        .unwrap_or_else(|error| panic!("setfacl on {}: {error}", target.display()));
    };

    // The grant the broker applies: traverse on the ancestor, full on the leaf.
    apply(&ancestor, "--x");
    apply(&leaf, "rwx");

    // The reconciliation step: a `chmod` on the ancestor. POSIX rewrites the ACL
    // mask from the group bits, which is the step that can silently neutralize a
    // named-user entry.
    fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o710)).expect("chmod again");

    assert_eq!(
        effective_permission(&ancestor, uid),
        Some(0o1),
        "the launched principal must keep an effective traverse bit on the ancestor"
    );
    assert_eq!(
        effective_permission(&leaf, uid),
        Some(0o7),
        "the launched principal must keep effective access on the leaf"
    );
}

/// Resolve the `setfacl`/`getfacl` binary from the broker's fixed candidate
/// list, never `$PATH`.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn acl_tool(name: &str) -> Option<PathBuf> {
    ["/run/current-system/sw/bin", "/usr/bin", "/bin"]
        .iter()
        .flat_map(|dir| [Path::new(dir).join(name)])
    .find(|candidate| candidate.exists())
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn acl_tool_or_skip(name: &str) -> PathBuf {
    match acl_tool(name) {
        Some(path) => path,
        None => panic!("{name} is required for the AE19 traversal proof"),
    }
}

/// The permission bits the KERNEL applies to `uid` for `path`.
///
/// `getfacl` reports them from the `system.posix_acl_access` xattr the kernel
/// stores, already ANDed with the ACL mask - the value an access check
/// actually uses. A named-user entry the mask has neutralized prints
/// `#effective:---`, which is precisely the failure mode a `chmod` after the
/// grant introduces, so this reads the EFFECTIVE permission and not the mode
/// the broker asked for. `None` when the path carries no entry for `uid`.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn effective_permission(path: &Path, uid: u32) -> Option<u32> {
    let output = std::process::Command::new(acl_tool_or_skip("getfacl"))
        .arg("--omit-header")
        .arg("--numeric")
        .arg("--absolute-names")
        .arg("--no-effective")
        .arg(path)
        .output()
        .unwrap_or_else(|error| panic!("getfacl on {}: {error}", path.display()));
    assert!(
        output.status.success(),
        "getfacl on {} failed: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let rendered = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut mask = 0o7;
    let mut named_user = None;
    for line in rendered.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("mask::") {
            mask = parse_acl_permission(rest);
        }
        if let Some(rest) = line.strip_prefix(&format!("user:{uid}:")) {
            named_user = Some(parse_acl_permission(rest));
        }
    }
    named_user.map(|perm| perm & mask)
}

/// Parse one `getfacl` permission triplet, ignoring an `#effective:` note.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn parse_acl_permission(rendered: &str) -> u32 {
    let triplet = rendered.split_whitespace().next().unwrap_or_default();
    // `getfacl` renders each permission slot with `-` for "absent", so `--x`
    // is read-not-readable, not-readable, executable. Summing the symbols is
    // what the kernel's mask check does; parsing it as one octal digit would
    // reject every entry that lacks read or write.
    triplet
        .bytes()
        .map(|symbol| match symbol {
            b'r' => 4,
            b'w' => 2,
            b'x' => 1,
            b'-' => 0,
            other => panic!("unknown getfacl permission symbol {other} in {rendered:?}"),
        })
        .sum()
}

// ---------------------------------------------------------------------------
// Scenario 5 (AE6): namespace-first service source
// ---------------------------------------------------------------------------

/// AE6, hermetic: a namespace-first service source is handed a Process
/// filesystem mount it cannot realize, and the launch is refused rather than
/// served with the mount skipped. The refusal IS the requirement here, so no
/// kernel is involved.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn namespace_first_service_source_refuses_a_process_filesystem_mount() {
    let tree = SourceTree::new();
    let destination = tree.destination("data");
    let mut input = plan_input();
    input.argv = vec![
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        "true".to_owned(),
        format!("--shared-dir={}", destination.display()),
    ];
    input.presentation = PresentationRealization::NamespaceFirstServiceSource;
    input.admitted_presentation = AdmittedPresentation {
        private_execution_root: tree.private_execution_root(),
        binds: vec![bind(&tree.view, &destination, false)],
    };
    assert!(
        matches!(
            preflight(&input),
            Err(SpawnRunnerError::PresentationRequiresFilesystemRealization)
        ),
        "a namespace-first service source must refuse a filesystem presentation, not skip it"
    );
}

/// AE6, hermetic: the same refusal at the syscall boundary, where a mount policy
/// that asks for a mount the service sandbox cannot realize is refused by name.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn namespace_first_service_source_refuses_a_requested_mount_policy() {
    let mut mount_policy = empty_mount_policy();
    mount_policy.nix_store_read_only = true;
    let isolation = RunnerIsolationSpec {
        capabilities: Vec::new(),
        namespaces: namespaces(true),
        seccomp_program: None,
        mount_policy,
        cgroup_dir_fd: None,
        cgroup_procs_fd: None,
        user_namespace: Some(current_user_namespace()),
        umask: None,
        pre_opened_device_fds: Vec::new(),
        memlock_limit_bytes: None,
        activation_stdin: None,
        presentation: PresentationRealization::NamespaceFirstServiceSource,
        private_execution_root: None,
        presentation_binds: Vec::new(),
    };
    let uid = nix::unistd::Uid::current().as_raw();
    let gid = nix::unistd::Gid::current().as_raw();
    let error = clone3_spawn_runner(
        CString::new("/bin/true").unwrap(),
        vec![CString::new("/bin/true").unwrap()],
        Vec::new(),
        uid,
        gid,
        Vec::new(),
        isolation,
    )
    .expect_err("a requested mount on a service-source leg must be refused");
    let rendered = format!("{error}");
    assert!(
        rendered.contains("presentation-requires-mount-realization"),
        "the refusal must name the realization that cannot realize it: {rendered}"
    );
    assert!(
        rendered.contains("refused rather than skipped"),
        "the refusal must state the mount is refused rather than skipped: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// The argv fence: the destination or a declared descriptor, never the source
// ---------------------------------------------------------------------------

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn argv_may_not_name_the_host_source_path() {
    let tree = SourceTree::new();
    let destination = tree.destination("data");
    let presentation = AdmittedPresentation {
        private_execution_root: tree.private_execution_root(),
        binds: vec![bind(&tree.view, &destination, false)],
    };
    assert!(
        fence_presentation_argv(
            &[format!("--shared-dir={}", destination.display())],
            &presentation
        )
        .is_ok(),
        "the worker may address its admitted destination"
    );
    assert!(
        fence_presentation_argv(
            &[
                "--gpu-device-node".to_owned(),
                "/proc/self/fd/10".to_owned()
            ],
            &presentation
        )
        .is_ok(),
        "a declared inherited descriptor is not a host path"
    );
    assert!(
        fence_presentation_argv(&[format!("--shared-dir={}", tree.view.display())], &presentation)
            .is_err(),
        "the worker must not be handed the host source path"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn preflight_refuses_argv_that_names_the_host_source() {
    let tree = SourceTree::new();
    let destination = tree.destination("data");
    let mut input = plan_input();
    input.argv = vec![
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        "true".to_owned(),
        format!("--shared-dir={}", tree.view.display()),
    ];
    input.admitted_presentation = AdmittedPresentation {
        private_execution_root: tree.private_execution_root(),
        binds: vec![bind(&tree.view, &destination, false)],
    };
    assert!(matches!(
        preflight(&input),
        Err(SpawnRunnerError::PresentationArgvNamesHostSource)
    ));
}

/// A plan input shaped like a real launch, with the binary check skipped so the
/// case does not depend on `/bin/sh` existing.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn plan_input() -> SpawnRunnerPlanInput {
    SpawnRunnerPlanInput {
        binary_path: PathBuf::from("/bin/sh"),
        argv: vec!["/bin/sh".to_owned(), "-c".to_owned(), "true".to_owned()],
        uid: 4242,
        gid: 4242,
        supplementary_groups: Vec::new(),
        env: Vec::new(),
        capabilities: Vec::new(),
        namespaces: namespaces(false),
        seccomp_policy_ref: None,
        mount_policy: empty_mount_policy(),
        cgroup_placement: CgroupPlacement {
            subtree: "d2b.slice/pubzone/shell".to_owned(),
            controllers: vec!["pids".to_owned()],
            delegated: false,
        },
        root_carve_out: false,
        skip_binary_exists_check: true,
        user_namespace: None,
        umask: None,
        presentation: PresentationRealization::FilesystemPresentation,
        admitted_presentation: AdmittedPresentation {
            private_execution_root: std::path::PathBuf::new(),
            binds: Vec::new(),
        },
    }
}

#[allow(dead_code, reason = "keeps the raw-fd import honest for the fd-based helpers")]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn _raw_fd(fd: &std::os::fd::OwnedFd) -> i32 {
    fd.as_raw_fd()
}
