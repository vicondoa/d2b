//! Behavioural coverage for the shell re-exec self-guard
//! (`tests/tools/heavy-gate-reexec.sh`).
//!
//! These cases drive the guard through a real `bash` interpreter, because the
//! helper is a bash script (`BASH_SOURCE`, arrays, `local`) that can only be
//! exercised through the shell it targets. They live in `tests/` - not inline
//! in `src/heavy_gate.rs` - so the ADR 0017 no-bash-exec gate stays strict for
//! every production source location while still allowing the helper itself to
//! be tested through bash.

use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Serialise the guard cases against one another. Each spawns a child that
/// inherits this process's descriptor table, and a `fork` briefly keeps
/// close-on-exec descriptors alive in the child, so overlapping spawns could
/// otherwise perturb one another's view of the environment.
static LOCK_STATE: Mutex<()> = Mutex::new(());

fn exclusive() -> MutexGuard<'static, ()> {
    LOCK_STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

static SCRATCH_SEQUENCE: AtomicU32 = AtomicU32::new(0);

/// Self-cleaning scratch directory under the Bazel test directory, so no
/// test writes into the repository tree. Mirrors the helper in
/// `heavy_gate`'s inline test module, including the 0700 lockdown the guard's
/// root-trust check depends on.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let target = match std::env::var_os("TEST_TMPDIR") {
            Some(dir) => PathBuf::from(dir),
            None => repository_root().join(".scratch/heavy-gate-tests"),
        };
        let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = target
            .join("heavy-gate-tests")
            .join(format!("{label}-{}-{sequence}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("scratch directory is creatable");
        // Lock the scratch root down to 0700 so the gate's root-trust check is
        // deterministic regardless of the runner's umask: an owned-but-group-
        // writable root is (correctly) refused by prepare.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("scratch root mode is settable");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn runfile_path(relative: &str) -> PathBuf {
    if let Some(runfiles) = std::env::var_os("RUNFILES_DIR") {
        let candidate = PathBuf::from(runfiles).join("_main").join(relative);
        if candidate.exists() {
            return candidate;
        }
    }
    repository_root().join(relative)
}

fn repository_root() -> PathBuf {
    let mut candidates = Vec::new();
    if let Some(root) = std::env::var_os("D2B_REPO_ROOT") {
        candidates.push(PathBuf::from(root));
    }
    for variable in ["TEST_SRCDIR", "RUNFILES_DIR"] {
        if let Some(base) = std::env::var_os(variable).map(PathBuf::from) {
            candidates.push(base.clone());
            if let Some(workspace) = std::env::var_os("TEST_WORKSPACE") {
                candidates.push(base.join(workspace));
            }
            candidates.push(base.join("_main"));
        }
    }
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir);
    }
    for candidate in candidates {
        let mut path = candidate;
        if path.is_file() {
            path.pop();
        }
        loop {
            if path.join("Cargo.toml").is_file()
                && path.join("BUILD.bazel").is_file()
                && path.join("flake.nix").is_file()
            {
                return path;
            }
            if !path.pop() {
                break;
            }
        }
    }
    panic!("repository root is not discoverable")
}

/// Drives the shell re-exec self-guard through bash with a declared xtask
/// artifact. `xtask_stub` is the body of that artifact, allowing the
/// fail-closed paths to be exercised without a build at test runtime.
fn run_reexec_guard(
    xtask_stub: Option<&str>,
    extra_env: &[(&str, &str)],
) -> (std::process::Output, String) {
    let _guard = exclusive();
    let scratch = Scratch::new("reexec-guard");
    let base = scratch.path();
    let checkout = base.display().to_string();

    // Reproduce the minimal checkout layout the guard resolves from
    // BASH_SOURCE: <base>/tests/tools/heavy-gate-reexec.sh and <base>/packages/.
    // The guard derives root/packages from the helper's own location, never
    // from a supplied variable.
    let tools = base.join("tests/tools");
    fs::create_dir_all(&tools).unwrap();
    fs::create_dir_all(base.join("packages")).unwrap();
    let helper_src = fs::read_to_string(runfile_path("tests/tools/heavy-gate-reexec.sh"))
        .expect("the shipped re-exec helper is readable");
    let helper = tools.join("heavy-gate-reexec.sh");
    fs::write(&helper, helper_src).unwrap();

    if let Some(xtask_stub) = xtask_stub {
        let target = base.join("bazel-bin/packages/xtask");
        fs::create_dir_all(&target).unwrap();
        let xtask = target.join("xtask");
        fs::write(&xtask, xtask_stub.replace("@CHECKOUT@", &checkout)).unwrap();
        fs::set_permissions(&xtask, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let path = std::env::var("PATH").unwrap_or_default();

    // The entrypoint argument is read only on the acquire (exec) path, which
    // these fail-closed cases never reach, so a nonexistent path is fine. Paths
    // carry no shell metacharacters, but quote defensively.
    //
    // Close every inherited descriptor above stderr before sourcing the helper.
    // The suite itself may run under heavy-gate, whose slot descriptor
    // deliberately survives exec. Clearing only its environment advertisement
    // would leave a planted marker able to rediscover that descriptor. Bash's
    // `{var}>&-` form closes the numeric descriptor without eval.
    //
    let script = format!(
        "for inherited_fd_path in /proc/self/fd/*; do\n\
           inherited_fd=${{inherited_fd_path##*/}}\n\
           case \"$inherited_fd\" in 0|1|2|*[!0-9]*) ;; *) exec {{inherited_fd}}>&- || true ;; esac\n\
         done\n\
         unset inherited_fd inherited_fd_path\n\
         . '{helper}'\n\
         d2b_heavy_gate_reexec '{base}' '{base}/tests/tools/entry.sh'\n",
        helper = helper.display(),
        base = base.display(),
    );

    // Start from an empty environment. In addition to heavy-gate state and
    // exported functions, this excludes every bash startup/control channel
    // (`BASH_ENV`, `ENV`, `BASH_XTRACEFD`, `SHELLOPTS`) without relying on an
    // ever-growing denylist. PATH is the only inherited value the harness
    // genuinely needs.
    let mut command = Command::new("bash");
    command
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", path)
        .env_remove("D2B_HEAVY_GATE_REEXEC_DEPTH");
    // Applied last so the hostile function is present when the production
    // helper runs.
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command
        .stdin(Stdio::null())
        .output()
        .expect("bash runs the sourced self-guard");
    (output, checkout)
}

#[test]
fn reexec_self_guard_fails_closed_when_the_built_binary_is_absent() {
    let out = run_reexec_guard(None, &[]).0;
    assert_eq!(
        out.status.code(),
        Some(70),
        "a successful build that yields no binary must fail closed with exit 70"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unavailable"),
        "the guard must emit a bounded missing-binary label: {stderr}"
    );
    assert!(
        !stderr.contains("/debug/xtask"),
        "the binary path (username-bearing) must not be disclosed: {stderr}"
    );
}

#[test]
fn inherited_gate_state_and_descriptor_do_not_authorise_the_child() {
    let planted_scratch = Scratch::new("reexec-planted-fd");
    let planted = fs::File::create(planted_scratch.path().join("slot")).unwrap();
    nix::fcntl::fcntl(
        planted.as_raw_fd(),
        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
    )
    .expect("the planted descriptor is inheritable");
    let sentinel = planted_scratch.path().join("acquired");

    // The fake verifier succeeds only if the advertised descriptor survived
    // into it. A clean child sees the planted environment but no descriptor,
    // returns the real verifier's "unheld" status, and reaches the acquisition
    // branch, where the sentinel is written.
    let stub = format!(
        "#!/bin/sh\n\
         if [ \"$2\" = verify-slot ]; then\n\
           test -n \"$D2B_HEAVY_GATE_SLOT_FD\" && \
             test -e \"/proc/self/fd/$D2B_HEAVY_GATE_SLOT_FD\" && exit 0\n\
           exit 3\n\
         fi\n\
         printf acquired > '{}'\n\
         exit 66\n\
         ",
        sentinel.display()
    );
    let fd = planted.as_raw_fd().to_string();
    let (out, _checkout) = run_reexec_guard(
        Some(&stub),
        &[
            ("D2B_HEAVY_GATE", "1"),
            ("D2B_HEAVY_GATE_SLOT", "0"),
            ("D2B_HEAVY_GATE_SLOT_FD", fd.as_str()),
            ("ENV", "/does/not/exist"),
            ("BASH_ENV", "/does/not/exist"),
            ("BASH_XTRACEFD", "9"),
            ("SHELLOPTS", "xtrace"),
        ],
    );

    assert_eq!(
        out.status.code(),
        Some(66),
        "the clean child must reject inherited gate state and try to acquire"
    );
    assert_eq!(
        fs::read_to_string(&sentinel).unwrap_or_default(),
        "acquired",
        "the planted descriptor must not let verify-slot report a held slot"
    );
}
