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

/// Self-cleaning scratch directory under the cargo target directory, so no
/// test writes into the repository tree. Mirrors the helper in
/// `heavy_gate`'s inline test module, including the 0700 lockdown the guard's
/// root-trust check depends on.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let target = match std::env::var_os("CARGO_TARGET_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("xtask lives inside the workspace root")
                .join("target"),
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

/// Drives the shell re-exec self-guard through bash with a stubbed `cargo`
/// first on PATH, so its fail-closed build and missing-binary paths can be
/// exercised hermetically without a real toolchain. `cargo_stub` is the body
/// of an executable placed first on PATH - exactly the hostile-cargo surface
/// the guard must survive.
fn run_reexec_guard_with_stub_cargo(cargo_stub: &str) -> std::process::Output {
    run_reexec_guard(cargo_stub, &[], false).0
}

fn run_reexec_guard_with_redactor(cargo_stub: &str) -> (std::process::Output, String) {
    run_reexec_guard(cargo_stub, &[], true)
}

/// As above, but injects `extra_env` onto the child bash after the
/// inherited-function strip, so a test can plant a hostile `BASH_FUNC_*` entry
/// and prove the child's function table is still controlled.
fn run_reexec_guard(
    cargo_stub: &str,
    extra_env: &[(&str, &str)],
    plant_redactor: bool,
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
    let helper_src = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/tools/heavy-gate-reexec.sh"
    ))
    .expect("the shipped re-exec helper is readable");
    let helper = tools.join("heavy-gate-reexec.sh");
    fs::write(&helper, helper_src).unwrap();

    let bin = base.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let cargo = bin.join("cargo");
    fs::write(&cargo, cargo_stub.replace("@CHECKOUT@", &checkout)).unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();

    if plant_redactor {
        let target = base.join("packages/target/debug");
        fs::create_dir_all(&target).unwrap();
        let xtask = target.join("xtask");
        fs::copy(env!("CARGO_BIN_EXE_xtask"), &xtask).unwrap();
        fs::set_permissions(&xtask, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let path = match std::env::var("PATH") {
        Ok(existing) => format!("{}:{}", bin.display(), existing),
        Err(_) => bin.display().to_string(),
    };

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
    // Clear the function table too. This is defense in depth after `env_clear`:
    // an explicitly planted BASH_FUNC entry in `extra_env` must not shadow the
    // PATH stub.
    let script = format!(
        "for inherited_fd_path in /proc/self/fd/*; do\n\
           inherited_fd=${{inherited_fd_path##*/}}\n\
           case \"$inherited_fd\" in 0|1|2|*[!0-9]*) ;; *) exec {{inherited_fd}}>&- || true ;; esac\n\
         done\n\
         unset inherited_fd inherited_fd_path\n\
         unset -f cargo rustc 2>/dev/null || true\n\
         . '{helper}'\n\
         d2b_heavy_gate_reexec '{base}' '{base}/tests/tools/entry.sh'\n",
        helper = helper.display(),
        base = base.display(),
    );

    // Start from an empty environment. In addition to heavy-gate state and
    // exported functions, this excludes every bash startup/control channel
    // (`BASH_ENV`, `ENV`, `BASH_XTRACEFD`, `SHELLOPTS`) without relying on an
    // ever-growing denylist. PATH is the only inherited value the harness
    // genuinely needs, and points at the test's cargo stub first.
    let mut command = Command::new("bash");
    command
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", path)
        .env_remove("D2B_HEAVY_GATE_REEXEC_DEPTH");
    // Applied last so a test can plant an entry the parent-env strip above would
    // otherwise remove, isolating the in-script `unset -f` defence.
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
fn reexec_self_guard_fails_closed_and_redacts_when_the_build_fails() {
    // A stub cargo prints the shape of a real compiler error containing this
    // checkout. The previously built, current xtask is planted where the guard
    // expects it, just as a normal incremental build leaves the last good
    // binary available when a subsequent compile fails.
    let stub = "#!/bin/sh\n\
        echo 'error[E0999]: @CHECKOUT@/packages/xtask/src/main.rs build exploded' >&2\n\
        exit 1\n";
    let (out, checkout) = run_reexec_guard_with_redactor(stub);
    assert_eq!(
        out.status.code(),
        Some(70),
        "a failed xtask build must fail closed with exit 70"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("build failed"),
        "the guard must emit a bounded build-failure label: {stderr}"
    );
    assert!(
        stderr.contains("error[E0999]: <repo>/packages/xtask/src/main.rs build exploded"),
        "the redacted compiler diagnostic must remain actionable: {stderr}"
    );
    assert!(
        !stderr.contains(&checkout) && !stderr.contains("/home/"),
        "the diagnostic must contain no absolute checkout path: {stderr}"
    );
}

#[test]
fn reexec_self_guard_fails_closed_when_the_built_binary_is_absent() {
    // A stub cargo that "succeeds" without producing target/debug/xtask -
    // exactly the fake success a hostile PATH could supply alongside no planted
    // binary. With the target dir pinned to this checkout the guard cannot be
    // pointed at a planted xtask, so a missing binary must fail closed rather
    // than proceed unverified.
    let out = run_reexec_guard_with_stub_cargo("#!/bin/sh\nexit 0\n");
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
fn an_inherited_cargo_function_does_not_shadow_the_path_stub() {
    // Regression: a heavy-gate runner may install pinned `cargo`/`rustc`
    // wrappers with `export -f`, which cross into a child bash as BASH_FUNC_*
    // entries. Because shell function resolution precedes PATH, such an
    // inherited `cargo` function would shadow the stub this harness plants on
    // PATH and silently run the real toolchain - the exact defect that let the
    // fail-closed cases pass under `make test-rust`. Prove the child's function
    // table is controlled: the PATH stub must run, not the inherited function.
    let sentinel_scratch = Scratch::new("reexec-sentinel");
    let sentinel = sentinel_scratch.path().join("who");
    let sentinel_display = sentinel.display();

    // The stub records `stub`; a shadowing `cargo` function would record
    // `func`. Exit codes are immaterial (both drive a fail-closed 70) - the
    // sentinel is the discriminator for which `cargo` actually resolved.
    let stub = format!("#!/bin/sh\nprintf stub > '{sentinel_display}'\nexit 1\n");
    // Delivered exactly as bash serialises an exported function.
    let func = format!("() {{ printf func > '{sentinel_display}'; exit 1;\n}}");
    let (out, _) = run_reexec_guard(&stub, &[("BASH_FUNC_cargo%%", func.as_str())], false);

    assert_eq!(
        out.status.code(),
        Some(70),
        "an unbuildable xtask must still fail closed with exit 70"
    );
    let who = fs::read_to_string(&sentinel).unwrap_or_default();
    assert_eq!(
        who, "stub",
        "the PATH stub must run, not the inherited cargo function (ran: {who:?})"
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
         mkdir -p \"$CARGO_TARGET_DIR/debug\"\n\
         cat > \"$CARGO_TARGET_DIR/debug/xtask\" <<'EOF'\n\
         #!/bin/sh\n\
         if [ \"$2\" = verify-slot ]; then\n\
           test -n \"$D2B_HEAVY_GATE_SLOT_FD\" && \
             test -e \"/proc/self/fd/$D2B_HEAVY_GATE_SLOT_FD\" && exit 0\n\
           exit 3\n\
         fi\n\
         printf acquired > '{}'\n\
         exit 66\n\
         EOF\n\
         chmod 755 \"$CARGO_TARGET_DIR/debug/xtask\"\n",
        sentinel.display()
    );
    let fd = planted.as_raw_fd().to_string();
    let out = run_reexec_guard(
        &stub,
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
