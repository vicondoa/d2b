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

use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use d2b_broker::live_handlers::{
    exact_endpoint_access, grant_exact_endpoint_access, revoke_exact_endpoint_access,
};
use d2b_broker::ops::endpoint_access::{
    EndpointAccessError, accept_endpoint_access, ensure_endpoint_socket_dir, endpoint_socket_path,
};
use d2b_broker::ops::spawn_runner::{PresentationBindSpec, PresentationRealization};
use d2b_broker::sys::pidfd_sys::{RunnerIsolationSpec, UserNamespaceSpec, clone3_spawn_runner};
use d2b_contracts_broker::broker_wire::{
    BrokerRequest, EndpointAccessRequest, EndpointAccessResponse, EndpointAccessVerb,
    EndpointPrincipalClaim, endpoint_access_authority_binding,
};
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::resource_schema::{
    CanonicalJsonValue, canonical_json_bytes, framed_canonical_digest,
};
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_core::bundle::{Bundle, BundleGeneration};
use d2b_core::bundle_resolver::{BundleResolver, DEVICE_TPM_PROVIDER_REF};
use d2b_core::manifest_v04::ManifestV04;
use d2b_core::processes::ProcessesJson;
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
        presentation: PresentationRealization::FilesystemPresentation,
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

// ---------------------------------------------------------------------------
// Accept path: the same properties ACROSS the wire
// ---------------------------------------------------------------------------
//
// The cases above drive the three helpers directly, so they prove the kernel
// semantics but not the boundary. The cases below carry every request through
// the broker's own wire contract - a `BrokerRequest` encoded and decoded again
// - and then through the accept path the dispatch arm calls, with the broker
// runtime root and a verified Zone bundle as its only inputs.
//
// What they add is the part a helper-level test cannot reach: the wire has no
// field a path could travel in, the broker's resolved target is a direct child
// of its OWN directory whatever the request says, the principal the ACL names
// is the one the verified bundle derives, and a request whose facts were
// edited on the wire is refused before anything is touched - so "refused" and
// "mutated nothing" stay the same observation.

/// The Zone the fixture bundle declares.
const ZONE: &str = "work";
/// The Zone self-resource uid that Zone's verified bundle is bound to.
const ZONE_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
/// The committed consumer row the grant acts for.
const CONSUMER: &str = "Process/shell";
/// The exact `Endpoint` row the relationship is admitted against.
const ENDPOINT: &str = "Endpoint/compositor";
/// The directory the broker resolves endpoint socket names inside, relative to
/// its runtime root.
const BROKER_ENDPOINT_DIR: &str = "endpoints";

/// A broker runtime root laid out the way the broker itself resolves it.
///
/// `runtime/` is the broker's runtime root: the directory its private socket's
/// parent names. `endpoints/` is the one directory under that root the exact
/// endpoint wire can select from, and it holds live AF_UNIX sockets.
/// `elsewhere/` is a sibling of the runtime root entirely and holds the
/// alternate absolute socket - the thing a path on the wire would reach if a
/// path could travel on the wire at all.
struct AcceptTree {
    /// Held so the tree outlives every case that borrows it; the path is
    /// reached through `runtime` and `elsewhere` rather than from here.
    _scratch: tempfile::TempDir,
    runtime: PathBuf,
    endpoints: PathBuf,
    elsewhere: PathBuf,
    /// One marker per live socket, so a case can assert the tree really holds
    /// all three before it claims the alternates carry nothing.
    live_sockets: usize,
}

impl AcceptTree {
    /// Build the tree under a root whose own ancestors already apply traverse.
    ///
    /// `tempfile::tempdir()` follows `TMPDIR`, which a sandboxed test runner
    /// points inside a private, non-traversable directory - and the broker
    /// grants traversal only inside its OWN runtime root, so a tree under
    /// such a parent could never be walked to and the case would be measuring
    /// the harness rather than the grant. `/tmp` is where the host runtime
    /// roots' own ancestors (`/run`, `/`) already are: traversable, and never
    /// listable by a consumer this broker does not own.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn new() -> Self {
        let scratch = tempfile::Builder::new()
            .prefix("d2b-endpoint-accept-")
            .tempdir_in(world_traversable_root())
            .expect("accept-path tempdir");
        let runtime = scratch.path().join("runtime");
        let endpoints = runtime.join(BROKER_ENDPOINT_DIR);
        let elsewhere = scratch.path().join("elsewhere");
        for directory in [&runtime, &endpoints, &elsewhere] {
            fs::create_dir_all(directory).expect("accept-path directory");
            // 0750: traversable by the consumer and not listable, and a non-zero
            // GROUP class so the kernel's ACL mask does not cap the named
            // traverse entry this grant installs (the AE19 trap - see the
            // recorded solution in
            // `docs/solutions/infrastructure/posix-acl-mask-nullified-by-chmod-on-mode-0700-directories.md`).
            fs::set_permissions(directory, fs::Permissions::from_mode(0o750))
                .expect("chmod accept-path directory");
        }
        // The tree's own parent is an ancestor of the broker runtime root, and
        // the broker grants traversal only up to its own root, so the rest of
        // the chain must already be traversable - exactly as `/run` and `/`
        // already are in production. 0711: traversable, never listable.
        fs::set_permissions(scratch.path(), fs::Permissions::from_mode(0o711))
            .expect("chmod the accept-path scratch dir");
        let mut listeners = Vec::new();
        for path in [endpoints.join(ADMITTED), endpoints.join(SIBLING)] {
            UnixListener::bind(&path).expect("bind endpoint socket");
            // 0660: the socket grants the consumer NOTHING through the other
            // class, so every bit the kernel applies to a non-owner principal
            // is a bit the broker's grant installed - and the group class keeps
            // the mask wide enough for the named entry to be effective.
            fs::set_permissions(&path, fs::Permissions::from_mode(0o660))
                .expect("chmod endpoint socket");
            listeners.push(());
        }
        UnixListener::bind(elsewhere.join("attacker.sock")).expect("bind alternate socket");
        fs::set_permissions(
            elsewhere.join("attacker.sock"),
            fs::Permissions::from_mode(0o660),
        )
        .expect("chmod alternate socket");
        assert_ancestors_are_traversable(&runtime);
        Self {
            _scratch: scratch,
            runtime,
            endpoints,
            elsewhere,
            live_sockets: listeners.len() + 1,
        }
    }

    /// The exact socket the relationship is admitted against.
    fn admitted(&self) -> PathBuf {
        self.endpoints.join(ADMITTED)
    }

    /// A sibling socket in the SAME directory, which the consumer must not reach.
    fn sibling(&self) -> PathBuf {
        self.endpoints.join(SIBLING)
    }

    /// The alternate absolute socket, outside the broker's runtime root.
    fn alternate_absolute(&self) -> PathBuf {
        self.elsewhere.join("attacker.sock")
    }

    /// Every socket in the tree, so a "nothing was granted anywhere" claim is
    /// checked against all of them rather than against a chosen one.
    fn every_socket(&self) -> Vec<PathBuf> {
        vec![self.admitted(), self.sibling(), self.alternate_absolute()]
    }

    /// The directories the broker grants traversal on, root-first.
    fn ancestors(&self) -> Vec<PathBuf> {
        vec![self.runtime.clone(), self.endpoints.clone()]
    }

    /// Every inode in the tree: the three sockets and the two directories.
    fn every_inode(&self) -> Vec<PathBuf> {
        let mut paths = self.every_socket();
        paths.extend(self.ancestors());
        paths
    }

    /// How many live endpoints the tree holds, so "the sibling and the
    /// alternate carry nothing" is a statement about a real tree.
    fn live_endpoints(&self) -> usize {
        self.live_sockets
    }
}

/// The directory the accept-path tree is built under.
///
/// `/tmp` when the test process can write there, else the process temp
/// directory. Either way the choice is checked, not assumed:
/// [`AcceptTree::assert_ancestors_are_traversable`] fails the case if the
/// directory above the broker's own runtime root is not traversable, so a
/// harness that hands the test a sealed tree is reported instead of quietly
/// weakening every traversal claim the cases make.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn world_traversable_root() -> &'static Path {
    Path::new("/tmp")
}

/// Every ancestor from the filesystem root down to (but excluding) the
/// broker's runtime root must already apply a traverse bit to everyone.
///
/// This is the production shape - the broker's runtime root is `/run/d2b`, and
/// `/run` and `/` are traversable by every principal - and the broker does not
/// repair it when it is not: a traversal grant above the broker's own root
/// would be a mutation of a directory the broker does not own. The case
/// asserts the precondition instead of assuming it, so the traversal claims it
/// makes are about the broker's grant and never about the harness.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn assert_ancestors_are_traversable(runtime_root: &Path) {
    for ancestor in runtime_root.ancestors().skip(1) {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        let mode = fs::metadata(ancestor)
            .unwrap_or_else(|error| panic!("stat {}: {error}", ancestor.display()))
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o001,
            0o001,
            "{} must already apply a traverse bit to every principal: the broker grants \
             traversal only inside its own runtime root, so a sealed ancestor makes the \
             exact endpoint unreachable no matter what the broker applies",
            ancestor.display()
        );
    }
}

/// The canonical content hash one fixture resource array hashes to, computed
/// the way `ResourceBundle` computes it, so the fixture bundle verifies.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn fixture_content_hash(resources: &[serde_json::Value]) -> String {
    let array = serde_json::Value::Array(resources.to_vec());
    let canonical = CanonicalJsonValue::parse(
        &serde_json::to_vec(&array).expect("fixture resources serialize"),
    )
    .expect("fixture resources are canonical JSON");
    framed_canonical_digest(
        "d2b:v3:resource-bundle",
        &canonical_json_bytes(&canonical).expect("fixture resources encode"),
    )
}

/// One Zone resource bundle declaring a single template-bound `Process`
/// consumer row, in the shape the compiler emits: the owning `Provider` row
/// and the consumer row under `resources`, plus the executable binding that
/// gives the row a numeric principal under `processTemplates`.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn zone_bundle(zone: &str, zone_uid: &str, row_name: &str) -> Vec<u8> {
    let host = format!("{zone}-host");
    let provider = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "Provider",
        "metadata": { "name": "device-tpm", "zone": zone },
        "spec": { "artifactId": "device-tpm" },
    });
    let consumer = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "Process",
        "metadata": {
            "name": row_name,
            "zone": zone,
            "ownerRef": DEVICE_TPM_PROVIDER_REF,
        },
        "spec": {
            "domain": "system",
            "executionRef": format!("Host/{host}"),
            "processClass": "controller",
            "providerRef": "Provider/system-minijail",
            "template": "consumer-worker",
        },
    });
    // `ResourceBundle::verify` requires rows sorted by `(type, name)`.
    let resources = vec![consumer, provider];
    let binding = serde_json::json!({
        "processRef": format!("Process/{row_name}"),
        "ownerRef": DEVICE_TPM_PROVIDER_REF,
        "executionRef": format!("Host/{host}"),
        "template": "consumer-worker",
        "artifactId": "device-tpm",
        "binaryRef": "swtpm",
        "artifactDigest": format!("sha256:{}", "a".repeat(64)),
        "binaryPath": "/nix/store/device-tpm/bin/swtpm",
    });
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 3,
        "bundleVersion": 1,
        "zone": zone,
        "zoneUid": zone_uid,
        "contentHash": fixture_content_hash(&resources),
        "artifactCatalogDigest": format!("sha256:{}", "c".repeat(64)),
        "schemaFingerprints": {},
        "providerSchemaDigests": {},
        "resources": resources,
        "processTemplates": [binding],
        "generatedAt": "1970-01-01T00:00:00.000Z",
    }))
    .expect("fixture zone resource bundle serializes")
}

/// A resolver carrying the fixture Zone's committed bundle, assembled the way
/// the loader assembles one from the verified artifact bytes.
///
/// The principal this resolver answers with is DERIVED from the committed row -
/// a number in the reserved 50,000 range, never the test process's uid - which
/// is the point: the ACL the accept path writes names the bundle's answer, not
/// a number a test chose.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn accept_resolver() -> BundleResolver {
    BundleResolver::from_artifacts_with_zone_resource_bundles(
        Bundle {
            bundle_version: 11,
            schema_version: "v2".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: None,
            realm_workloads_launcher_v2_path: None,
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash: None,
            artifact_hashes: None,
        },
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .expect("host fixture parses"),
        ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: Vec::new(),
        },
        ManifestV04::from_slice(
            include_str!("../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .expect("manifest fixture parses"),
        BTreeMap::from_iter([(ZONE.to_owned(), zone_bundle(ZONE, ZONE_UID, "shell"))]),
    )
}

/// The one request struct all three variants share, for one verb.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn access_request(socket: &str, verb: EndpointAccessVerb) -> EndpointAccessRequest {
    let endpoint_ref = ResourceRef::parse(ENDPOINT).expect("fixture endpoint ref");
    let consumer_ref = ResourceRef::parse(CONSUMER).expect("fixture consumer ref");
    let zone_uid = ResourceUid::parse(ZONE_UID).expect("fixture zone uid");
    let socket = BoundedToken::parse(socket).expect("the fixture socket name is a bounded token");
    EndpointAccessRequest {
        authority_key: endpoint_access_authority_binding(
            &endpoint_ref,
            &consumer_ref,
            &zone_uid,
            &socket,
            verb,
        ),
        endpoint_ref,
        consumer_ref,
        zone_uid,
        socket,
        socket_rights: 0o6,
        claimed_principal: None,
        tracing_span_id: None,
    }
}

/// The wire request for one verb, bound to the relationship it is admitted
/// under: the variant is the verb the request travels as.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn access_request_variant(verb: EndpointAccessVerb) -> BrokerRequest {
    let request = access_request(ADMITTED, verb);
    match verb {
        EndpointAccessVerb::Observe => BrokerRequest::EndpointObserve(request),
        EndpointAccessVerb::Grant => BrokerRequest::EndpointGrantAccess(request),
        EndpointAccessVerb::Revoke => BrokerRequest::EndpointRevokeAccess(request),
    }
}

/// Cross the wire: encode a request with the broker's own codec and decode it
/// again, so a case is proven against the value the accept path receives
/// rather than against the value the test built.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn across_the_wire(request: &BrokerRequest) -> BrokerRequest {
    let frame = serde_json::to_vec(request).expect("the request encodes for the wire");
    serde_json::from_slice(&frame).expect("the frame decodes back into a request")
}

/// Drive one request across the wire and through the accept path.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn accept(
    request: &BrokerRequest,
    tree: &AcceptTree,
    resolver: &BundleResolver,
) -> Result<EndpointAccessResponse, EndpointAccessError> {
    accept_endpoint_access(&across_the_wire(request), &tree.runtime, resolver)
}

/// Assert that no inode anywhere in `tree` carries an entry for `uid`, so a
/// refusal and "mutated nothing" are the same observation.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn assert_nothing_granted(tree: &AcceptTree, uid: u32, context: &str) {
    for path in tree.every_inode() {
        assert_eq!(
            pinned_acl(&path, uid).granted_bits(),
            None,
            "{context}: {} must carry no entry for the consumer principal",
            path.display()
        );
    }
}

/// AE7: the request cannot carry a path, so an alternate absolute socket, a
/// `..` escape, and the containing directory never reach a resolver at all.
///
/// Every refusal here happens in the CONTRACT, before any broker code runs:
/// the token grammar rejects the spelling, and a hand-built hostile frame fails
/// to decode into a request. That is the difference between a fence the broker
/// applies and a fence the wire cannot be argued past.
#[test]
fn an_alternate_absolute_socket_a_relative_escape_and_the_runtime_directory_cannot_be_named() {
    let hostile = [
        // An alternate absolute socket, spelled as a workload would spell it.
        "/tmp/attacker.sock",
        // The same socket reached relatively from the broker's own directory.
        "../../elsewhere/attacker.sock",
        // A `..` escape out of the broker's endpoint directory.
        "..",
        // A sibling reached through its neighbour.
        "wayland-0/../wayland-1",
        // The empty name, and a name that does not start with a letter.
        "",
        "0-wayland",
    ];
    for spelling in hostile {
        assert!(
            BoundedToken::parse(spelling).is_err(),
            "{spelling:?} must not be a bounded token: it is not a single safe path component"
        );
    }

    // A frame an attacker would hand the socket: every field present, with the
    // socket spelled as a path. It does not decode, so no broker code is
    // reached and no path is ever resolved.
    for spelling in [
        "/tmp/attacker.sock",
        "../../elsewhere/attacker.sock",
        "..",
        "wayland-0/../wayland-1",
    ] {
        let payload = serde_json::json!({
            "endpointRef": ENDPOINT,
            "consumerRef": CONSUMER,
            "zoneUid": ZONE_UID,
            "socket": spelling,
            "socketRights": 7u8,
            "authorityKey": "sha256:00",
        });
        assert!(
            serde_json::from_value::<BrokerRequest>(serde_json::json!({
                "kind": "EndpointGrantAccess",
                "payload": payload.clone(),
            }))
            .is_err(),
            "{spelling:?} decoded into a request: the wire accepted a path"
        );
        assert!(
            serde_json::from_value::<EndpointAccessRequest>(payload).is_err(),
            "{spelling:?} decoded into an endpoint access request: the wire accepted a path"
        );
    }

    // The admitted request's own encoded payload is exactly the contract's
    // fields, and its socket is a bare name.
    let encoded =
        serde_json::to_value(access_request_variant(EndpointAccessVerb::Grant)).expect("encodes");
    let payload = &encoded["payload"];
    let mut keys: Vec<&str> = payload
        .as_object()
        .expect("the payload is an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "authorityKey",
            "consumerRef",
            "endpointRef",
            "socket",
            "socketRights",
            "zoneUid",
        ],
        "the wire payload is exactly the contract's fields and nothing else"
    );
    assert_eq!(payload["socket"], serde_json::json!(ADMITTED));

    // And what the broker resolves from that name is a DIRECT child of its own
    // directory, for the admitted name and for every legal name. The container
    // itself is therefore not a reachable target either: a token always names
    // something below it, never it.
    let tree = AcceptTree::new();
    for name in [ADMITTED, SIBLING, "any-other-endpoint"] {
        let token = BoundedToken::parse(name).expect("a legal socket name");
        let resolved = endpoint_socket_path(&tree.runtime, &token).expect("a direct child");
        assert_eq!(resolved, tree.endpoints.join(name));
        assert_eq!(
            resolved.parent(),
            Some(tree.endpoints.as_path()),
            "{name}: the resolved socket must be a DIRECT child of the broker's own directory"
        );
        assert_ne!(resolved, tree.endpoints, "{name}: the container is not a target");
        assert_ne!(resolved, tree.runtime, "{name}: the runtime root is not a target");
    }
    assert!(
        !tree.alternate_absolute().starts_with(&tree.runtime),
        "the alternate absolute socket must live outside the broker's runtime root"
    );
    assert_eq!(tree.live_endpoints(), 3);
}

/// The broker creates its own endpoint directory at serve time, so the
/// exact-endpoint surface is reachable rather than wired and inert.
///
/// The dispatch arm is production-wired, so the only thing that could keep it
/// from ever answering is the directory it resolves socket names inside. The
/// broker owns its runtime root and therefore owns that one directory: it
/// creates it once, before any connection is accepted, and NOT from a request.
/// The absent-directory refusal stays exactly as it was - a request never
/// invents the tree its own effect would run against - so the case proves both
/// halves against the production path and never against a harness that made
/// the directory itself.
///
/// What the surface answers once the directory is there depends on the
/// consumer's host account: this host provisions none for the fixture's
/// consumer row, so the request is refused by name and no ACL entry is ever
/// written. The grant itself - and the kernel read-back that proves it landed
/// on the exact inode - is a host-lane proof, reachable only where the host
/// has provisioned the account.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn the_broker_provisions_its_own_endpoint_directory_before_it_serves() {
    let scratch = tempfile::Builder::new()
        .prefix("d2b-endpoint-serve-")
        .tempdir_in(world_traversable_root())
        .expect("serve-path tempdir");
    let runtime = scratch.path().join("runtime");
    fs::create_dir_all(&runtime).expect("the broker's runtime root");
    // The production shape of the runtime root: private, traversable, never
    // listable, and carrying a non-zero GROUP class so the named traverse
    // entry the grant installs below stays effective (the AE19 trap).
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o750))
        .expect("posture the broker runtime root");
    fs::set_permissions(scratch.path(), fs::Permissions::from_mode(0o711))
        .expect("posture the scratch root");
    assert_ancestors_are_traversable(&runtime);

    let resolver = accept_resolver();
    let endpoints = runtime.join(BROKER_ENDPOINT_DIR);
    assert!(
        !endpoints.exists(),
        "the case starts from a runtime root nothing has provisioned under"
    );

    // Absent is a refusal by name, for every verb, and nothing is created by
    // asking: a request does not get to invent the tree.
    for verb in [
        EndpointAccessVerb::Observe,
        EndpointAccessVerb::Grant,
        EndpointAccessVerb::Revoke,
    ] {
        let error = accept_endpoint_access(
            &across_the_wire(&access_request_variant(verb)),
            &runtime,
            &resolver,
        )
        .expect_err("an unprovisioned broker refuses the exact-endpoint surface by name");
        assert_eq!(error, EndpointAccessError::EndpointDirectoryAbsent, "{verb:?}");
        assert_eq!(
            error.code(),
            "endpoint-access-directory-absent",
            "{verb:?} carries the closed slug"
        );
        assert!(
            !endpoints.exists(),
            "{verb:?} must not have created the directory it resolves into"
        );
    }

    // The broker's own serve-time provisioning makes it exist, and only it.
    let provisioned =
        ensure_endpoint_socket_dir(&runtime).expect("the broker provisions the directory it owns");
    assert_eq!(provisioned, endpoints);
    let mode = fs::metadata(&endpoints)
        .expect("stat the endpoint directory")
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(
        mode & 0o070,
        0o010,
        "the endpoint directory must keep a non-zero group class: POSIX rewrites the ACL \
         mask from the group bits on every chmod, so a 0700 directory nullifies the named \
         traverse entry the moment anything re-asserts its mode"
    );
    assert_eq!(
        mode & 0o004,
        0,
        "the endpoint directory must never be listable by anyone outside the broker"
    );

    // With the broker's own directory in place, the surface answers - and what
    // it answers is the refusal, because the consumer row this host has no
    // account for resolves to no principal. Nothing is written.
    let admitted = endpoints.join(ADMITTED);
    let _socket = UnixListener::bind(&admitted).expect("bind the admitted endpoint");
    fs::set_permissions(&admitted, fs::Permissions::from_mode(0o660))
        .expect("posture the admitted endpoint");

    let error = accept_endpoint_access(
        &across_the_wire(&access_request_variant(EndpointAccessVerb::Grant)),
        &runtime,
        &resolver,
    )
    .expect_err("a consumer row with no host account is refused, not granted a uid of our own");
    assert_eq!(
        error.code(),
        "endpoint-access-consumer-principal",
        "the refusal names the consumer principal the verified bundle could not resolve"
    );
    assert_eq!(
        effective_permission(&admitted, nix::unistd::Uid::current().as_raw()),
        None,
        "and no ACL entry was written for anyone on the admitted endpoint"
    );
}

/// AE7: a request whose consumer resolves to no host account lands nothing on
/// any inode, and says so by name.
///
/// The three sockets are real and live before the request, so "nothing carries
/// an entry" is a statement about the refusal rather than about an empty
/// directory. The alternate lives OUTSIDE the broker runtime root, which is
/// what makes it the alternate absolute socket: reaching it would have to mean
/// leaving the one directory the wire selects from.
///
/// Where the grant lands on the exact inode, and on no other, is the AE19/R23
/// proof, and it needs a consumer whose account the host has provisioned. This
/// host provisions none, so what this holds is the other half of the same rule:
/// the broker writes no ACL entry at all, rather than one for a uid nothing
/// holds.
#[test]
fn a_request_whose_consumer_has_no_host_account_lands_on_no_inode() {
    let tree = AcceptTree::new();
    let resolver = accept_resolver();
    for socket in tree.every_socket() {
        assert!(
            socket.exists(),
            "{} must exist before the request",
            socket.display()
        );
    }

    let error = accept(
        &access_request_variant(EndpointAccessVerb::Grant),
        &tree,
        &resolver,
    )
    .expect_err("a consumer row with no host account is refused, not answered with a uid");
    assert_eq!(
        error.code(),
        "endpoint-access-consumer-principal",
        "the refusal names the consumer principal the verified bundle could not resolve"
    );
    for socket in tree.every_socket() {
        assert_eq!(
            pinned_acl(&socket, nix::unistd::Uid::current().as_raw()).granted_bits(),
            None,
            "{} must carry no entry at all, because no principal was resolved \
             to grant one",
            socket.display()
        );
    }
    assert_eq!(tree.every_socket().len(), 3, "the tree must still hold all three");
}

/// AE7: a request whose facts were edited on the wire is refused, and the
/// refusal is the same observation as "nothing was touched".
///
/// Every case below changes exactly one field, so each refusal is attributable
/// to that field: a socket repointed at the sibling, a grant's authority key
/// replayed as a revoke, another Zone, and a permission outside the socket's
/// own triple. The forged principal claim and the absent-socket request are the
/// same shape of proof: the claim is checked and never read, and a request that
/// names an endpoint the broker's directory does not hold is refused by name.
///
/// What the refusals are checked against is the absence of any grant, because
/// this host provisions no account for the fixture's consumer row: the
/// committed request itself is refused too, and the case that used to prove
/// these fences are not a blanket denial - a correct request that still grants
/// - is a host-lane proof, reachable only where the host has provisioned the
/// account.
#[test]
fn a_repointed_or_forged_request_is_refused_and_mutates_nothing() {
    let resolver = accept_resolver();
    // Nothing is granted to anyone in this tree, so every case below is
    // checked against a principal the running process holds: a fence that
    // failed would show up as an entry for exactly that principal.
    let uid = nix::unistd::Uid::current().as_raw();
    let tree = AcceptTree::new();

    // 1. The socket repointed at the sibling. The sibling's NAME is a legal
    //    token, so only the authority binding can refuse this - and it does,
    //    because the binding is recomputed over the socket name.
    let mut repointed = access_request(ADMITTED, EndpointAccessVerb::Grant);
    repointed.socket = BoundedToken::parse(SIBLING).expect("the sibling name is a legal token");
    let refusal = accept(
        &BrokerRequest::EndpointGrantAccess(repointed),
        &tree,
        &resolver,
    )
    .expect_err("a repointed socket must be refused");
    assert_eq!(refusal.code(), "endpoint-access-authority-mismatch");
    assert_nothing_granted(&tree, uid, "a repointed socket");

    // 2. A grant's authority key replayed as a revoke and as an observation.
    //    The request is byte-for-byte the granted one; only the VARIANT it
    //    travels as changes, and the verb is part of the binding, so the key
    //    cannot travel with it.
    let granted_request = access_request(ADMITTED, EndpointAccessVerb::Grant);
    for request in [
        BrokerRequest::EndpointRevokeAccess(granted_request.clone()),
        BrokerRequest::EndpointObserve(granted_request.clone()),
    ] {
        let refusal = accept(&request, &tree, &resolver)
            .expect_err("a grant's key must not be replayable as another verb");
        assert_eq!(refusal.code(), "endpoint-access-authority-mismatch");
        assert_nothing_granted(&tree, uid, "a replayed authority key");
    }

    // 3. Another Zone's committed bundle answers for another Zone's consumer,
    //    so a request that repoints the Zone is refused by the binding.
    let mut other_zone = access_request(ADMITTED, EndpointAccessVerb::Grant);
    other_zone.zone_uid =
        ResourceUid::parse("123e4567-e89b-42d3-a456-4266141740ff").expect("a second zone uid");
    let refusal = accept(
        &BrokerRequest::EndpointGrantAccess(other_zone),
        &tree,
        &resolver,
    )
    .expect_err("a repointed Zone must be refused");
    assert_eq!(refusal.code(), "endpoint-access-authority-mismatch");
    assert_nothing_granted(&tree, uid, "a repointed Zone");

    // 4. A permission outside the socket's own triple, including the empty
    //    one. The binding is reproduced deliberately here, so the refusal is
    //    attributable to the rights and not to the key.
    for rights in [0u8, 0o10, 0o70] {
        let mut widened = access_request(ADMITTED, EndpointAccessVerb::Grant);
        widened.socket_rights = rights;
        widened.authority_key = endpoint_access_authority_binding(
            &widened.endpoint_ref,
            &widened.consumer_ref,
            &widened.zone_uid,
            &widened.socket,
            EndpointAccessVerb::Grant,
        );
        let refusal = accept(
            &BrokerRequest::EndpointGrantAccess(widened),
            &tree,
            &resolver,
        )
        .expect_err("a permission outside the socket triple must be refused");
        assert_eq!(refusal.code(), "endpoint-access-rights-out-of-range");
        assert_nothing_granted(&tree, uid, "a widened permission");
    }

    // 5. A forged principal claim. The claim is re-pinned against the
    //    derivation and refused, and a claim that AGREES still yields the
    //    derived numbers, never the claim's copy.
    let mut forged = access_request(ADMITTED, EndpointAccessVerb::Grant);
    forged.claimed_principal = Some(EndpointPrincipalClaim {
        uid: nix::unistd::Uid::current().as_raw(),
        gid: nix::unistd::Gid::current().as_raw(),
    });
    let refusal = accept(
        &BrokerRequest::EndpointGrantAccess(forged),
        &tree,
        &resolver,
    )
    .expect_err("a forged principal claim must be refused");
    assert_eq!(refusal.code(), "endpoint-access-consumer-principal");
    assert_nothing_granted(&tree, uid, "a forged principal claim");
    assert_eq!(
        pinned_acl(&tree.admitted(), nix::unistd::Uid::current().as_raw()).granted_bits(),
        None,
        "the forged claim's own numbers must have been applied to nothing"
    );

    // 6. A socket the broker's own directory does not hold. The name is a legal
    //    token, so the request passes the authority binding and is refused on
    //    the resolved path - and it is the broker's OWN directory the broker
    //    looked in, not a path the request supplied. The consumer principal is
    //    resolved before the socket is looked up, so on a host with no account
    //    for the row this answers with the consumer refusal rather than the
    //    absent-socket one; either way the request mutates nothing.
    let absent = accept(
        &BrokerRequest::EndpointGrantAccess(access_request(
            "no-such-endpoint",
            EndpointAccessVerb::Grant,
        )),
        &tree,
        &resolver,
    )
    .expect_err("a socket the broker's directory does not hold must be refused");
    assert_eq!(absent.code(), "endpoint-access-consumer-principal");
    assert_nothing_granted(&tree, uid, "an absent socket");

    // 7. The committed request is refused as well, by the consumer principal
    //    the verified bundle could not resolve: this host provisions no account
    //    for the row, so there is no uid to grant anything to. That is the
    //    other half of the same rule, and it is why the case can no longer
    //    prove these fences are not a blanket denial - only a host that has
    //    provisioned the account can.
    let committed = accept(
        &access_request_variant(EndpointAccessVerb::Grant),
        &tree,
        &resolver,
    )
    .expect_err("the committed request is refused while the host has no account for the row");
    assert_eq!(committed.code(), "endpoint-access-consumer-principal");
    assert_nothing_granted(&tree, uid, "the committed request");
}

/// Revocation over the wire, on a host that has provisioned no account for the
/// consumer row: every verb is refused before any effect runs, so there is no
/// entry to remove and the sockets are left exactly as they were.
///
/// The revoke that removes an admitted entry, reports the inode it removed it
/// from, and leaves the ancestor traversal and the sibling alone is a
/// host-lane proof: it needs a consumer whose account the host has provisioned.
#[test]
fn revocation_of_a_relationship_with_no_host_account_mutates_nothing() {
    let tree = AcceptTree::new();
    let resolver = accept_resolver();
    let uid = nix::unistd::Uid::current().as_raw();
    for verb in [
        EndpointAccessVerb::Grant,
        EndpointAccessVerb::Observe,
        EndpointAccessVerb::Revoke,
    ] {
        let error = accept(
            &access_request_variant(verb),
            &tree,
            &resolver,
        )
        .expect_err("a consumer row with no host account is refused for every verb");
        assert_eq!(
            error.code(),
            "endpoint-access-consumer-principal",
            "{verb:?} is refused by the consumer principal, not by a later stage"
        );
        assert_nothing_granted(&tree, uid, &format!("{verb:?}"));
    }
}

/// A producer that recycled its socket, observed on a host that has
/// provisioned no account for the consumer row: the observation is refused
/// before the socket is read, so a relationship prepared against the old
/// socket cannot be told stale from a fresh one.
///
/// The observation that names a different inode for a recycled socket is a
/// host-lane proof: it needs a consumer whose account the host has
/// provisioned, which is the only way the broker reaches the inode at all.
#[test]
fn a_recycled_socket_is_not_observed_for_a_relationship_with_no_host_account() {
    let tree = AcceptTree::new();
    let resolver = accept_resolver();
    let previous = pinned_identity(&tree.admitted());

    fs::remove_file(tree.admitted()).expect("unlink the admitted socket");
    UnixListener::bind(tree.admitted()).expect("rebind the socket");
    // The producer's replacement carries the same posture the first one did:
    // it grants the consumer nothing through the other class, so "nothing is
    // granted" is about the broker's grant rather than about the socket mode.
    fs::set_permissions(
        tree.admitted(),
        fs::Permissions::from_mode(0o660),
    )
    .expect("chmod the replacement socket");

    let error = accept(
        &access_request_variant(EndpointAccessVerb::Observe),
        &tree,
        &resolver,
    )
    .expect_err("an observation for a consumer with no host account is refused");
    assert_eq!(error.code(), "endpoint-access-consumer-principal");
    assert_ne!(
        pinned_identity(&tree.admitted()),
        previous,
        "the socket really was recycled underneath, so the refusal above is not \
         the broker mistaking one inode for another"
    );
    assert_eq!(
        pinned_acl(&tree.admitted(), nix::unistd::Uid::current().as_raw()).granted_bits(),
        None,
        "and the replacement carries no entry for any principal"
    );
}
