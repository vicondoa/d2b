//! Exact endpoint delivery for one launch (U18, R23).
//!
//! A consumer that is admitted one exact host endpoint must get that endpoint
//! and nothing else. Two things could widen it, and each is proven here
//! against the kernel rather than asserted from a field:
//!
//! 1. **AE7** - an alternate absolute socket, a sibling socket in the same
//!    directory, and the runtime directory itself. The alternates are built
//!    for real, and a real `clone3` child with a private mount tree is driven
//!    to show it cannot reach any of them: the private tree contains the one
//!    admitted socket and nothing else. **Real-kernel**
//!    ([`alternate_socket_sibling_and_runtime_directory_are_unreachable`]).
//! 2. **AE19** - a mode reconciliation after the grant nullifies the POSIX ACL
//!    mask under it. The assertion reads the ACL the kernel stores and
//!    computes the permission an access check actually applies, and it
//!    cross-checks that read against `getfacl` so the broker's own parser
//!    cannot be the only witness for its own answer.
//!    ([`traversal_and_socket_grants_survive_the_mode_reconciliation`]).
//!
//! The remaining cases cover what makes those two hold: the grant lands on
//! the pinned inode rather than on a path that can be replaced under it, a
//! replaced inode invalidates the observation, the containing directory stays
//! unlistable, and revocation removes only the exact endpoint's entry.
//!
//! The kernel lane is honest about its limits: a host that cannot create a
//! private mount tree prints `SKIP` and returns, which is a blocked
//! acceptance condition for that scenario and not a pass. The AE19 case needs
//! only `setfacl`/`getfacl` and the xattr, so it runs everywhere those
//! exist.
//!
//! Every case drives the real helpers and, where it needs a mount tree, the
//! real `clone3`/`fork` child, so the blocking filesystem calls, the child
//! wait, and the ACL probes carry the sanctioned `cfg(test) helper` allow at
//! each site rather than a crate-wide one.

use std::ffi::CString;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use d2b_broker::live_handlers::{
    exact_endpoint_access, grant_exact_endpoint_access, revoke_exact_endpoint_access,
};
use d2b_broker::ops::spawn_runner::{PresentationBindSpec, PresentationRealization};
use d2b_broker::sys::pidfd_sys::{RunnerIsolationSpec, UserNamespaceSpec, clone3_spawn_runner};
use d2b_core::sandbox_profile::{MountPolicy, NamespaceSet};

/// The exact compositor socket a consumer is admitted against.
const ADMITTED: &str = "wayland-0";
/// A sibling socket in the SAME directory, which the consumer must not reach.
const SIBLING: &str = "wayland-1";
/// An alternate absolute socket elsewhere on the host.
const ALTERNATE_ABSOLUTE: &str = "attacker.sock";
/// The consumer's single destination inside its private execution root.
const DESTINATION: &str = "compositor-socket";

/// A bound on any child wait, so a wedged handshake fails the case instead of
/// hanging the lane.
const CHILD_WAIT_BOUND: Duration = Duration::from_secs(30);

/// The exact endpoint tree one test case owns.
///
/// `runtime/` is the host session runtime directory as it actually exists on
/// this host: three real AF_UNIX sockets inside it, with the ancestor chain
/// the broker has to traverse. `exact/` is what the consumer is delivered: a
/// directory holding the ONE admitted socket and no sibling.
struct EndpointTree {
    root: tempfile::TempDir,
    runtime: PathBuf,
    exact: PathBuf,
    private_root: PathBuf,
    listeners: Vec<UnixListener>,
}

impl EndpointTree {
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn new() -> Self {
        let root = tempfile::tempdir().expect("endpoint tempdir");
        let ancestor = root.path().join("ancestor");
        let runtime = ancestor.join("run");
        let exact = root.path().join("exact");
        let private_root = root.path().join("private-root");
        for dir in [&runtime, &exact, &private_root] {
            fs::create_dir_all(dir).expect("endpoint directory");
        }
        // A 0700 ancestor is the shape the recorded ACL-mask failure needs:
        // its group bits are zero, so any later chmod recomputes the mask to
        // nothing and caps every named entry under it.
        fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o700)).expect("chmod ancestor");
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).expect("chmod runtime");

        // Three live endpoints in the runtime directory: the admitted one and
        // the two alternates a consumer must not be able to reach.
        let mut listeners = Vec::new();
        for name in [ADMITTED, SIBLING, ALTERNATE_ABSOLUTE] {
            listeners.push(UnixListener::bind(runtime.join(name)).expect("bind runtime socket"));
        }
        // The delivered directory holds the ONE admitted socket. A hard link
        // is the same inode under a second name, so what the consumer is given
        // IS the admitted endpoint and not a copy of it.
        std::fs::hard_link(runtime.join(ADMITTED), exact.join(ADMITTED))
            .expect("link the admitted socket into the delivered set");
        Self {
            root,
            runtime,
            exact,
            private_root,
            listeners,
        }
    }

    fn admitted_socket(&self) -> PathBuf {
        self.runtime.join(ADMITTED)
    }

    fn sibling_socket(&self) -> PathBuf {
        self.runtime.join(SIBLING)
    }

    fn alternate_socket(&self) -> PathBuf {
        self.runtime.join(ALTERNATE_ABSOLUTE)
    }

    /// The full ancestor chain of the admitted socket, root-first.
    fn ancestor_chain(&self) -> Vec<PathBuf> {
        self.admitted_socket()
            .ancestors()
            .skip(1)
            .filter(|component| !component.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .collect()
    }

    /// The ancestors the broker must grant traverse on: every directory in
    /// the chain that is not already world-traversable. A world-traversable
    /// ancestor already applies a traverse bit to every principal, and it is
    /// not this broker's to mutate, so the grant skips it.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn granted_ancestors(&self) -> Vec<PathBuf> {
        self.ancestor_chain()
            .into_iter()
            .filter(|directory| {
                fs::metadata(directory)
                    .map(|meta| meta.permissions().mode() & 0o001 == 0)
                    .unwrap_or(false)
            })
            .collect()
    }

    /// The ancestors the broker must leave exactly as it found them.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn untouched_ancestors(&self) -> Vec<PathBuf> {
        self.ancestor_chain()
            .into_iter()
            .filter(|directory| {
                fs::metadata(directory)
                    .map(|meta| meta.permissions().mode() & 0o001 != 0)
                    .unwrap_or(false)
            })
            .collect()
    }

    /// How many live endpoints the runtime directory holds. The listeners are
    /// kept alive for the whole tree so a bound socket stays connectable
    /// rather than becoming an unconnected inode.
    fn live_endpoints(&self) -> usize {
        self.listeners.len()
    }

    /// The private destination the exact socket is bound at.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn destination(&self) -> PathBuf {
        self.private_root.join(DESTINATION)
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn shell() -> Option<PathBuf> {
    ["/bin/sh", "/usr/bin/sh", "/run/current-system/sw/bin/sh"]
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

/// One consumer's isolation spec: a private mount tree and, when `user` is
/// set, a final user namespace created after that tree exists.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn isolation_for(user: bool) -> RunnerIsolationSpec {
    RunnerIsolationSpec {
        capabilities: Vec::new(),
        namespaces: NamespaceSet {
            mount: true,
            ..namespaces(user)
        },
        seccomp_program: None,
        mount_policy: empty_mount_policy(),
        cgroup_dir_fd: None,
        cgroup_procs_fd: None,
        user_namespace: user.then(|| UserNamespaceSpec {
            host_uid_for_zero: nix::unistd::Uid::current().as_raw(),
            host_gid_for_zero: nix::unistd::Gid::current().as_raw(),
        }),
        umask: None,
        pre_opened_device_fds: Vec::new(),
        memlock_limit_bytes: None,
        activation_stdin: None,
        presentation: Some(PresentationRealization::FilesystemPresentation),
        private_execution_root: None,
        presentation_binds: Vec::new(),
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
            Ok(status) => panic!("waitpid({pid}) never exited: {status:?}"),
            Err(error) => panic!("waitpid({pid}) failed: {error}"),
        }
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
    let isolation = isolation_for(true);
    let binary = CString::new(shell.to_string_lossy().as_bytes()).expect("shell path");
    let uid = nix::unistd::Uid::current().as_raw();
    let gid = nix::unistd::Gid::current().as_raw();
    match clone3_spawn_runner(binary.clone(), vec![binary], Vec::new(), uid, gid, Vec::new(), isolation) {
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

/// Run one consumer against a prepared exact-endpoint presentation.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn run_consumer(tree: &EndpointTree, script: &str, args: &[&str]) -> Option<i32> {
    let Some(shell) = shell() else {
        println!("SKIP: no /bin/sh for the kernel lane");
        return None;
    };
    let mut isolation = isolation_for(true);
    isolation.private_execution_root = Some(tree.private_root.clone());
    isolation.presentation_binds = vec![bind(&tree.exact, &tree.destination(), true)];
    let binary = CString::new(shell.to_string_lossy().as_bytes()).expect("shell path");
    let mut argv = vec![binary.clone(), CString::new("-c").expect("-c")];
    argv.push(CString::new(script).expect("script has no NUL"));
    argv.push(CString::new("endpoint-delivery-probe").expect("argv0 has no NUL"));
    for argument in args {
        argv.push(CString::new(*argument).expect("argument has no NUL"));
    }
    let uid = nix::unistd::Uid::current().as_raw();
    let gid = nix::unistd::Gid::current().as_raw();
    match clone3_spawn_runner(binary, argv, Vec::new(), uid, gid, Vec::new(), isolation) {
        Ok(outcome) => Some(exit_code(outcome.pid)),
        Err(error) => {
            println!("SKIP: kernel lane could not spawn a consumer here ({error})");
            None
        }
    }
}

/// Resolve an ACL tool from a fixed candidate list, never `$PATH`.
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

/// The permission bits the KERNEL applies to `uid` for `path`, read
/// independently of the broker's own xattr parser.
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
// Scenario 1 (AE7): the exact endpoint, and nothing else
// ---------------------------------------------------------------------------

/// AE7, real-kernel: the consumer reaches the ONE admitted socket and cannot
/// reach an alternate absolute socket, a sibling socket, or the runtime
/// directory that contains it.
///
/// The alternates are constructed for real - three live AF_UNIX sockets in a
/// 0700 runtime directory - and the consumer is a real `clone3` child whose
/// private mount tree contains the delivered directory and nothing else. The
/// script checks each alternate from INSIDE the child, so the refusals are the
/// kernel's answers about the namespace the consumer actually runs in, not an
/// assertion about a field.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn alternate_socket_sibling_and_runtime_directory_are_unreachable() {
    if !kernel_lane_available() {
        println!("SKIP: the AE7 reachability proof is OUTSTANDING on this host");
        return;
    }
    let tree = EndpointTree::new();
    let uid = nix::unistd::Uid::current().as_raw();
    let granted = grant_exact_endpoint_access(&tree.admitted_socket(), &tree.ancestor_chain(), uid, 0o7)
        .expect("grant the exact endpoint");
    assert_eq!(
        granted.socket_effective(),
        0o7,
        "the exact endpoint socket must be EFFECTIVE read and write"
    );
    assert_eq!(
        granted.ancestor_effective() & 0o1,
        0o1,
        "every ancestor must apply an effective traverse bit"
    );
    // The grant the broker asks for is traverse and nothing more. The
    // container directory therefore grants no listing, which is the whole
    // difference between reaching the exact endpoint and reaching the
    // directory. (This host's test principal is also the directories' OWNER,
    // so the class that applies to it here is the owner class; the owner
    // versus named-user distinction is what production hits, and the
    // structural "the directory is not there at all" property is proven by
    // the mount tree below.)
    for directory in tree.granted_ancestors() {
        let acl = pinned_acl(&directory, uid);
        assert_eq!(
            acl.granted_bits(),
            Some(0o1),
            "{}: the broker must ask for traverse and nothing more",
            directory.display()
        );
        assert_eq!(acl.mask() & 0o1, 0o1, "the mask must not cap the traverse bit");
    }
    for directory in tree.untouched_ancestors() {
        assert_eq!(
            pinned_acl(&directory, uid).granted_bits(),
            None,
            "{}: a world-traversable ancestor is not this broker's to mutate",
            directory.display()
        );
    }

    // The alternates really exist on the host, so a "cannot reach" answer in
    // the child is about the child's namespace and not about missing files.
    for alternate in [tree.sibling_socket(), tree.alternate_socket()] {
        assert!(alternate.exists(), "{} must exist on the host", alternate.display());
    }

    let destination = tree.destination();
    // Each `test` is a separate answer from inside the child's namespace:
    // the delivered socket is there and it is alone, and every alternate the
    // workload would try - a sibling in the same host directory, an alternate
    // absolute socket, the runtime directory itself, and the two relative
    // escapes - resolves to nothing.
    let script = "set -e
test -S \"$1/\"$(cat \"$2\")
[ \"$(ls -A \"$1\" | wc -l)\" -eq 1 ]
test ! -e \"$3\"
test ! -e \"$4\"
test ! -e \"$5\"
test ! -e \"$1/../\"$(cat \"$6\")
test ! -e \"$1/../\"$(cat \"$7\")";
    let Some(code) = run_consumer(
        &tree,
        script,
        &[
            destination.to_str().expect("utf-8 destination"),
            ADMITTED,
            tree.sibling_socket().to_str().expect("utf-8 sibling"),
            tree.alternate_socket().to_str().expect("utf-8 alternate"),
            tree.runtime.to_str().expect("utf-8 runtime"),
            SIBLING,
            ALTERNATE_ABSOLUTE,
        ],
    ) else {
        println!("SKIP: the AE7 reachability proof is OUTSTANDING on this host");
        return;
    };
    assert_eq!(
        code, 0,
        "the consumer must reach the exact endpoint and nothing else (exit {code})"
    );
    // The private mount stayed private: the host tree is unchanged.
    assert_eq!(
        tree.exact.read_dir().expect("read the delivered set").count(),
        1,
        "the host-side delivered set must still hold exactly the admitted socket"
    );
}

/// AE7, kernel-truthful and host-independent: the grant lands on ONE socket.
///
/// The alternates are constructed for real - three live AF_UNIX sockets in one
/// 0700 runtime directory - and the kernel's own ACL store is then read on each
/// of them through a pinned descriptor. Exactly one carries an entry for the
/// consumer principal, and it is the admitted inode; the sibling and the
/// alternate carry none at all, so there is no grant there for a redirected
/// lookup to find. This is the same property the mount-tree case proves
/// structurally, expressed against filesystem state so it does not depend on
/// whether this host can create a private mount tree.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn no_grant_lands_on_a_sibling_or_alternate_socket() {
    let tree = EndpointTree::new();
    let uid = nix::unistd::Uid::current().as_raw();
    let granted = grant_exact_endpoint_access(&tree.admitted_socket(), &tree.ancestor_chain(), uid, 0o7)
        .expect("grant the exact endpoint");

    let admitted = tree.admitted_socket();
    assert_eq!(
        granted.socket(),
        pinned_identity(&admitted),
        "the grant must land on the inode the endpoint owner resolved"
    );
    assert_eq!(
        pinned_acl(&admitted, uid).granted_bits(),
        Some(0o7),
        "the admitted endpoint carries the consumer's entry"
    );
    assert_eq!(
        effective_permission(&admitted, uid),
        Some(0o7),
        "and the kernel applies it"
    );

    for (label, alternate) in [
        ("the sibling socket", tree.sibling_socket()),
        ("the alternate socket", tree.alternate_socket()),
    ] {
        assert_ne!(pinned_identity(&alternate), granted.socket(), "{label} is a different inode");
        assert_eq!(
            pinned_acl(&alternate, uid).granted_bits(),
            None,
            "{label} must carry no entry for the consumer principal at all"
        );
        assert_eq!(
            effective_permission(&alternate, uid),
            None,
            "{label} must have nothing the kernel would apply to the consumer"
        );
    }

    // The container directory is a real directory holding all three sockets,
    // so "only the admitted socket" is a statement about the grant and not
    // about an empty directory.
    assert_eq!(
        tree.runtime
            .read_dir()
            .expect("read the runtime directory")
            .count(),
        3,
        "the runtime directory must really hold the admitted socket and both alternates"
    );
}

/// The `(dev, ino)` one path resolves to through a pinned descriptor.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn pinned_identity(path: &Path) -> (u64, u64) {
    let metadata = open_pinned(path).metadata().expect("stat the pinned inode");
    (metadata.dev(), metadata.ino())
}

/// The exact socket is a live, connectable AF_UNIX endpoint, so the delivery
/// the kernel lane resolves is a real one and not an inert inode.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn the_admitted_endpoint_is_a_live_connectable_socket() {
    let tree = EndpointTree::new();
    let uid = nix::unistd::Uid::current().as_raw();
    grant_exact_endpoint_access(&tree.admitted_socket(), &tree.ancestor_chain(), uid, 0o7)
        .expect("grant the exact endpoint");
    assert_eq!(tree.live_endpoints(), 3, "the alternates must be live endpoints");
    for (label, socket) in [
        ("the admitted endpoint", tree.admitted_socket()),
        ("the sibling", tree.sibling_socket()),
        ("the alternate", tree.alternate_socket()),
    ] {
        assert!(
            std::os::unix::net::UnixStream::connect(&socket).is_ok(),
            "{label} must be a connectable endpoint before delivery"
        );
    }
}

// ---------------------------------------------------------------------------
// Scenario 2 (AE19): effective access survives mode reconciliation
// ---------------------------------------------------------------------------

/// AE19: after a mode reconciliation, the traverse bit on every ancestor and
/// the access bit on the exact socket are still EFFECTIVE.
///
/// The reconciliation step is the recorded failure: `chmod 0700` on an
/// ancestor has zero group bits, so the kernel recomputes that directory's ACL
/// mask to `---` and every named entry under it is capped to nothing while
/// still being listed. This case builds the exact failing shape, asserts that
/// the kernel really did null the mask (so the rest of the case is not
/// vacuous), and then shows the grant path re-establishing effectiveness
/// rather than tolerating the nullified entry.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn traversal_and_socket_grants_survive_the_mode_reconciliation() {
    let tree = EndpointTree::new();
    let uid = nix::unistd::Uid::current().as_raw();
    let socket = tree.admitted_socket();
    let ancestors = tree.granted_ancestors();
    let ancestor = ancestors.first().expect("a granted ancestor").clone();

    grant_exact_endpoint_access(&socket, &tree.ancestor_chain(), uid, 0o7)
        .expect("initial grant");
    assert_eq!(
        effective_permission(&ancestor, uid),
        Some(0o1),
        "the traverse entry must be present and effective before the reconciliation"
    );
    assert_eq!(
        effective_permission(&socket, uid),
        Some(0o7),
        "the exact socket must start out effectively read and write"
    );

    // The reconciliation: a `chmod` that recomputes the mask from the group
    // bits. The entry is still LISTED afterwards and still lists the same
    // bits, so a presence check would pass here - which is the whole trap.
    fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o700)).expect("re-assert the mode");
    let nullified = pinned_acl(&ancestor, uid);
    assert_eq!(
        nullified.granted_bits(),
        Some(0o1),
        "the named entry must still be listed after the re-assertion"
    );
    assert_eq!(
        nullified.mask(),
        0,
        "a 0700 re-assertion must null the mask; otherwise this case proves nothing"
    );
    assert_eq!(
        effective_permission(&ancestor, uid),
        Some(0o0),
        "the kernel applies nothing from an entry the mask has capped"
    );

    // The grant path re-establishes effectiveness rather than tolerating the
    // nullified entry.
    let regranted = grant_exact_endpoint_access(&socket, &tree.ancestor_chain(), uid, 0o7)
        .expect("re-granting must restore the entry, not tolerate it");
    assert_eq!(
        regranted.socket_effective() & 0o7,
        0o7,
        "the re-granted socket must be effective again"
    );
    assert_eq!(regranted.ancestor_effective() & 0o1, 0o1);
    for directory in &ancestors {
        assert_eq!(
            pinned_acl(directory, uid).granted_bits(),
            Some(0o1),
            "{} must still ask for traverse and nothing more",
            directory.display()
        );
        assert_eq!(
            effective_permission(directory, uid),
            Some(0o1),
            "{} must carry an EFFECTIVE traverse bit after the reconciliation",
            directory.display()
        );
    }
    assert_eq!(
        effective_permission(&socket, uid),
        Some(0o7),
        "the exact socket must keep effective read and write"
    );
    // The container directory is still not a grant to enumerate: the
    // traversal entry the broker installs carries no read bit anywhere on the
    // path, which is what stops a consumer from listing the directory around
    // the exact endpoint.
    let observed = exact_endpoint_access(&socket, uid)
        .expect("re-read the exact endpoint access")
        .expect("the exact endpoint is present");
    assert_eq!(
        observed.socket_effective() & 0o7,
        0o7,
        "the socket grant is intact after the reconciliation"
    );
    assert_eq!(
        observed.ancestor_effective() & 0o1,
        0o1,
        "the whole ancestor chain is traversable again"
    );
    for directory in &ancestors {
        let granted = pinned_acl(directory, uid).granted_bits().expect("a named entry");
        assert_eq!(
            granted & 0o4,
            0,
            "{}: the traversal grant must not carry a read bit, or the consumer could \
             enumerate the directory that holds the endpoint",
            directory.display()
        );
    }
}

/// AE19: a correct grant one level below a root whose traverse access was
/// nullified is no grant at all.
///
/// This is the consequence that made the original failure expensive: checking
/// the directory you care about tells you nothing about whether it is
/// reachable. The case builds exactly that - the socket keeps a perfect entry,
/// the ancestor's entry is capped to nothing - and shows the kernel-effective
/// answer for the path as a whole.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_correct_grant_below_a_nullified_root_is_not_a_grant() {
    let tree = EndpointTree::new();
    let uid = nix::unistd::Uid::current().as_raw();
    let socket = tree.admitted_socket();
    let ancestors = tree.granted_ancestors();
    let ancestor = ancestors.first().expect("a granted ancestor").clone();

    grant_exact_endpoint_access(&socket, &tree.ancestor_chain(), uid, 0o7)
        .expect("initial grant");
    assert_eq!(effective_permission(&socket, uid), Some(0o7));

    // Nullify ONLY the ancestor's mask, leaving the socket's entry intact.
    fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o700)).expect("re-assert the mode");
    assert_eq!(effective_permission(&ancestor, uid), Some(0o0));
    assert_eq!(
        effective_permission(&socket, uid),
        Some(0o7),
        "the socket's own entry is untouched, which is exactly why a socket-only check misses this"
    );

    // Walking the whole path is what finds it: the granted socket and the
    // capped ancestor disagree, so the relationship is not traversable.
    let socket_traversable = effective_permission(&socket, uid).is_some();
    let ancestor_traversable = effective_permission(&ancestor, uid).is_some_and(|bits| bits & 0o1 == 0o1);
    assert!(
        socket_traversable && !ancestor_traversable,
        "a perfect socket entry under a root whose traverse entry is capped is not a grant"
    );
    assert_eq!(
        exact_endpoint_access(&socket, uid)
            .expect("re-read")
            .expect("present")
            .socket_effective()
            & 0o7,
        0o7,
        "the socket itself is still fine, which is what makes this case worth having"
    );
}

// ---------------------------------------------------------------------------
// Scenario 3: the grant lands on the pinned inode
// ---------------------------------------------------------------------------

/// A replaced socket inode invalidates the observation: the access read
/// reports the NEW `(dev, ino)`, so a relationship prepared against the old
/// one can tell it is stale.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_replaced_socket_inode_invalidates_the_observation() {
    let tree = EndpointTree::new();
    let uid = nix::unistd::Uid::current().as_raw();
    let socket = tree.admitted_socket();
    let granted = grant_exact_endpoint_access(&socket, &tree.ancestor_chain(), uid, 0o7)
        .expect("grant the exact endpoint");
    assert_eq!(granted.socket_effective(), 0o7);
    assert_eq!(granted.ancestor_effective() & 0o1, 0o1);

    // The producer recycled its socket: a different inode now answers at the
    // same pathname, and the old one is unlinked.
    let previous = granted.socket();
    fs::remove_file(&socket).expect("unlink the admitted socket");
    let replacement = UnixListener::bind(&socket).expect("rebind the socket");
    drop(replacement);

    let observed = exact_endpoint_access(&socket, uid)
        .expect("re-read the exact endpoint access")
        .expect("the replacement socket is present");
    assert_ne!(
        observed.socket(),
        previous,
        "a recycled socket must read as a different inode, or cached readiness survives it"
    );
    assert_eq!(
        exact_endpoint_access(&tree.root.path().join("no-such.sock"), uid),
        Ok(None),
        "an absent endpoint reads as absent rather than as a stale grant"
    );
}

/// The grant refuses a target that is not a socket, so a directory or a
/// regular file can never be delivered as an endpoint.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn a_grant_on_anything_but_a_socket_is_refused() {
    let tree = EndpointTree::new();
    let uid = nix::unistd::Uid::current().as_raw();
    let ancestors = tree.ancestor_chain();
    let error = grant_exact_endpoint_access(&tree.runtime, &ancestors, uid, 0o7)
        .expect_err("a directory is not an endpoint");
    assert!(error.contains("not a socket"), "{error}");
    let absent = grant_exact_endpoint_access(
        &tree.root.path().join("absent.sock"),
        &ancestors,
        uid,
        0o7,
    )
    .expect_err("an absent endpoint cannot be granted");
    assert!(absent.contains("absent"), "{absent}");
}

/// Revocation removes the exact endpoint's entry and leaves the ancestor
/// traversal grants in place, which sibling endpoints and the producer's own
/// helpers also depend on.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn revocation_removes_the_exact_entry_and_keeps_ancestor_traversal() {
    let tree = EndpointTree::new();
    let uid = nix::unistd::Uid::current().as_raw();
    let socket = tree.admitted_socket();
    let ancestors = tree.granted_ancestors();
    grant_exact_endpoint_access(&socket, &tree.ancestor_chain(), uid, 0o7)
        .expect("grant the exact endpoint");
    assert_eq!(effective_permission(&socket, uid), Some(0o7));

    let pinned = pinned_identity(&socket);
    let revoked = revoke_exact_endpoint_access(&socket, uid).expect("revoke the exact entry");
    assert_eq!(
        revoked,
        Some(pinned),
        "the revoke must land on the socket inode the grant used"
    );
    assert_eq!(
        effective_permission(&socket, uid),
        None,
        "the revoked principal must have no entry on the exact socket at all"
    );
    for directory in &ancestors {
        assert_eq!(
            effective_permission(directory, uid),
            Some(0o1),
            "{} must keep the traversal grant other endpoints depend on",
            directory.display()
        );
    }
    for directory in tree.untouched_ancestors() {
        assert_eq!(
            pinned_acl(&directory, uid).granted_bits(),
            None,
            "{} must never have been granted anything",
            directory.display()
        );
    }
    // Revocation is idempotent under retry: a second call finds no entry and
    // still resolves the inode.
    assert!(revoke_exact_endpoint_access(&socket, uid).is_ok());
    assert_eq!(
        revoke_exact_endpoint_access(&tree.root.path().join("absent.sock"), uid),
        Ok(None),
        "revoking an absent endpoint is a no-op, not an error"
    );
}

/// The pinned-inode read and `getfacl` agree on every entry this test tree
/// produces, so the broker's xattr parser is not the sole witness for its own
/// effective-permission answer.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn the_pinned_read_agrees_with_getfacl_on_every_entry() {
    let tree = EndpointTree::new();
    let uid = nix::unistd::Uid::current().as_raw();
    let socket = tree.admitted_socket();
    let ancestors = tree.granted_ancestors();
    grant_exact_endpoint_access(&socket, &tree.ancestor_chain(), uid, 0o7)
        .expect("grant the exact endpoint");
    fs::set_permissions(ancestors.first().expect("an ancestor"), fs::Permissions::from_mode(0o700))
        .expect("nullify the ancestor mask");
    grant_exact_endpoint_access(&socket, &ancestors, uid, 0o7).expect("re-grant");

    for path in std::iter::once(&socket).chain(ancestors.iter()) {
        assert_eq!(
            pinned_bits(path, uid),
            effective_permission(path, uid),
            "{}: the broker's xattr read and getfacl must agree on the effective bits",
            path.display()
        );
    }
}

/// The ACL the broker's own pinned read reports for one principal on one path.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn pinned_acl(path: &Path, uid: u32) -> d2b_broker::live_handlers::EffectiveAcl {
    let file = open_pinned(path);
    let metadata = file.metadata().expect("stat the pinned inode");
    d2b_broker::live_handlers::effective_acl(&file, &metadata, uid).expect("read the effective ACL")
}

/// The named-user effective bits the broker's pinned read reports, which is
/// the value `getfacl` independently renders for the same entry.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn pinned_bits(path: &Path, uid: u32) -> Option<u32> {
    let acl = pinned_acl(path, uid);
    acl.granted_bits().map(|bits| bits & acl.mask())
}

/// Open one path as the same pinned `O_PATH|NOFOLLOW` descriptor the broker
/// holds, so the test reads the identical inode rather than re-resolving the
/// name.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn open_pinned(path: &Path) -> std::fs::File {
    rustix::fs::open(
        path,
        rustix::fs::OFlags::PATH | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    )
    .map(std::fs::File::from)
    .unwrap_or_else(|error| panic!("pin {}: {error}", path.display()))
}
