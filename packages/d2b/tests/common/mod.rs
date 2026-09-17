//! Shared CLI-contract integration-test harness.
//!
//! Most CLI-contract cases drive the `d2b` binary against static fixtures
//! and need nothing here. A handful of cases (audit daemon-backed paths) must
//! talk to a real, KVM-free `d2bd` over `AF_UNIX` + `SO_PEERCRED`. This
//! module spawns such a daemon in `--once` mode with a synthetic config and a
//! caller-chosen test peer identity.
//!
//! The d2bd binary path is delivered out-of-band via
//! `D2B_TEST_D2BD_BIN` (the gated rust-workspace-checks.sh step builds
//! `-p d2bd` and exports it). `d2b` does NOT depend on `d2bd`
//! (the static-rust-dependency-direction policy forbids that edge), so daemon
//! cases SKIP cleanly when the env var is unset (e.g. the plain
//! `cargo test --workspace` pass).

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// Returns the built `d2bd` binary path, or `None` when the daemon-spawn
/// harness is not available (env var unset). Daemon-backed test cases should
/// early-return (skip) when this is `None`.
pub fn d2bd_bin() -> Option<PathBuf> {
    std::env::var_os("D2B_TEST_D2BD_BIN").map(PathBuf::from)
}

/// A test peer role used to compile a hermetic daemon config for the real
/// process that opens the public socket.
pub struct TestPeer {
    admin: bool,
}

impl TestPeer {
    /// A launcher-role peer (in `launcherUsers`, not `adminUsers`).
    pub fn launcher() -> Self {
        TestPeer { admin: false }
    }

    /// An admin-role peer (in `adminUsers`).
    pub fn admin() -> Self {
        TestPeer { admin: true }
    }
}

/// A spawned `d2bd serve --once` instance plus the temp state it owns.
/// Dropping the guard kills the daemon if it is still running and removes the
/// temp dir.
pub struct DaemonOnce {
    pub child: Child,
    pub socket_path: PathBuf,
    pub daemon_state_dir: PathBuf,
    _tmp: TempDir,
}

impl DaemonOnce {
    /// Wait for the daemon process to exit (it serves a single request in
    /// `--once` mode) and return its exit status.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    pub fn wait(mut self) -> std::process::ExitStatus {
        self.child.wait().expect("wait for d2bd")
    }
}

impl Drop for DaemonOnce {
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn drop(&mut self) {
        // Best-effort: if --once already returned this is a no-op.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn primary_group_name() -> String {
    let gid = nix::unistd::getgid();
    nix::unistd::Group::from_gid(gid)
        .ok()
        .flatten()
        .map(|g| g.name)
        .unwrap_or_else(|| gid.to_string())
}

fn lifecycle_group_name() -> String {
    nix::unistd::Group::from_name("d2b")
        .ok()
        .flatten()
        .map(|group| group.name)
        .unwrap_or_else(primary_group_name)
}

fn current_username() -> String {
    nix::unistd::User::from_uid(nix::unistd::getuid())
        .ok()
        .flatten()
        .map(|user| user.name)
        .unwrap_or_else(|| "d2b-test-user".to_owned())
}

/// Spawn `d2bd serve --once --test-listen-on <socket>` with a hermetic config
/// classifying the real test process, and block until the public socket
/// exists. Returns `None` when the daemon-spawn harness is unavailable (so
/// the caller can skip).
///
/// In `--once` mode the daemon accepts exactly one request and then exits, so
/// the caller should run a single `d2b` invocation against
/// `socket_path` and then call [`DaemonOnce::wait`].
pub fn spawn_d2bd_once(peer: &TestPeer) -> Option<DaemonOnce> {
    spawn_d2bd_inner(peer)
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn spawn_d2bd_inner(peer: &TestPeer) -> Option<DaemonOnce> {
    let bin = d2bd_bin()?;

    let tmp = tempfile::tempdir().expect("tempdir");
    let run = tmp.path().join("run");
    let daemon_state_dir = run.join("daemon-state");
    let locks_dir = run.join("locks");
    std::fs::create_dir_all(&daemon_state_dir).expect("mk daemon-state");
    std::fs::create_dir_all(&locks_dir).expect("mk locks");
    // The state-lock parent (`run`) must be uid/gid-owned by the invoking user
    // and mode 0755/0750 for `--allow-unprivileged-runtime-dir` lock-parent
    // validation; pin it explicitly rather than relying on the process umask.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).expect("chmod run dir");

    let socket_path = run.join("public.sock");
    let state_lock = run.join("daemon.lock");
    let config_json = run.join("config.json");

    let username = current_username();
    let group = lifecycle_group_name();
    // Keep optional config inputs inside the test-owned run tree. Host-installed
    // realm files may still contain fields removed by the current contract.
    let config = serde_json::json!({
        "publicSocketPath": socket_path,
        "brokerSocketPath": run.join("priv.sock"),
        "stateLockPath": state_lock,
        "locksDir": locks_dir,
        "daemonUser": "root",
        "daemonGroup": "root",
        "publicSocketGroup": group,
        "launcherUsers": [&username],
        "adminUsers": if peer.admin {
            serde_json::json!([&username])
        } else {
            serde_json::json!([])
        },
        "serverVersion": "0.4.0",
        "acceptedClientVersionRange": ">=0.4.0, <0.5.0",
        "artifacts": {
            "publicManifestPath": run.join("manifest.json"),
            "bundlePath": run.join("bundle.json"),
            "hostPath": run.join("host.json"),
            "processesPath": run.join("processes.json"),
            "closuresDir": run.join("closures")
        },
        "gatewayConfigPath": run.join("gateway.json"),
        "realmControllersConfigPath": run.join("realm-controllers.json"),
        "realmIdentityConfigPath": run.join("realm-identity.json")
    });
    {
        let mut f = std::fs::File::create(&config_json).expect("write config.json");
        f.write_all(serde_json::to_string_pretty(&config).unwrap().as_bytes())
            .expect("write config bytes");
    }

    let mut command = Command::new(&bin);
    command
        .args(["serve", "--config"])
        .arg(&config_json)
        .arg("--test-listen-on")
        .arg(&socket_path)
        .arg("--state-lock")
        .arg(&state_lock)
        .arg("--locks-dir")
        .arg(&locks_dir)
        .arg("--daemon-state-dir")
        .arg(&daemon_state_dir)
        .args([
            "--once",
            "--allow-unprivileged-runtime-dir",
            "--no-drop-privileges",
        ])
        // The daemon's startup kernel-module gate reads the real /proc/modules;
        // bypass it so the daemon starts on any host.
        .env("D2B_SKIP_KERNEL_MODULE_CHECK", "1")
        // Quiet the daemon's startup/autostart tracing so it does not pollute
        // test output; assertions over the CLI response give the signal.
        .env("RUST_LOG", "off")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = command.spawn().expect("spawn d2bd serve --once");

    wait_for_socket(&socket_path, Duration::from_secs(15));

    Some(DaemonOnce {
        child,
        socket_path,
        daemon_state_dir,
        _tmp: tmp,
    })
}

/// Poll until `path` is a socket or the timeout elapses.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
pub fn wait_for_socket(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for socket: {}", path.display());
}
