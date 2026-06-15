//! Shared CLI-contract integration-test harness.
//!
//! Most CLI-contract cases drive the `nixling` binary against static fixtures
//! and need nothing here. A handful of cases (audit / host-check daemon-backed
//! paths) must talk to a real, KVM-free `nixlingd` over `AF_UNIX` +
//! `SO_PEERCRED`. This module spawns such a daemon in `--once` mode with a
//! synthetic config and a caller-chosen test peer identity.
//!
//! The nixlingd binary path is delivered out-of-band via
//! `NIXLING_TEST_NIXLINGD_BIN` (the gated rust-workspace-checks.sh step builds
//! `-p nixlingd` and exports it). `nixling` does NOT depend on `nixlingd`
//! (the static-rust-dependency-direction policy forbids that edge), so daemon
//! cases SKIP cleanly when the env var is unset (e.g. the plain
//! `cargo test --workspace` pass).

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// Returns the built `nixlingd` binary path, or `None` when the daemon-spawn
/// harness is not available (env var unset). Daemon-backed test cases should
/// early-return (skip) when this is `None`.
pub fn nixlingd_bin() -> Option<PathBuf> {
    std::env::var_os("NIXLING_TEST_NIXLINGD_BIN").map(PathBuf::from)
}

/// A test peer identity presented to the daemon via the `NIXLINGD_TEST_PEER_*`
/// env hooks, which stand in for the real `SO_PEERCRED` of the connecting CLI.
pub struct TestPeer {
    pub uid: u32,
    pub gid: u32,
    pub username: &'static str,
    pub groups: &'static str,
}

impl TestPeer {
    /// A launcher-role peer (in `launcherUsers`, not `adminUsers`).
    pub fn launcher() -> Self {
        TestPeer {
            uid: 60003,
            gid: 60003,
            username: "launcher-user",
            groups: "wheel",
        }
    }

    /// An admin-role peer (in `adminUsers`).
    pub fn admin() -> Self {
        TestPeer {
            uid: 60004,
            gid: 60004,
            username: "admin-user",
            groups: "wheel",
        }
    }
}

/// A spawned `nixlingd serve --once` instance plus the temp state it owns.
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
    pub fn wait(mut self) -> std::process::ExitStatus {
        self.child.wait().expect("wait for nixlingd")
    }
}

impl Drop for DaemonOnce {
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

/// Spawn `nixlingd serve --once --test-listen-on <socket>` with a synthetic
/// config presenting `peer` as the connecting identity, and block until the
/// public socket exists. Returns `None` when the daemon-spawn harness is
/// unavailable (so the caller can skip).
///
/// In `--once` mode the daemon accepts exactly one request and then exits, so
/// the caller should run a single `nixling` invocation against
/// `socket_path` and then call [`DaemonOnce::wait`].
pub fn spawn_nixlingd_once(peer: &TestPeer) -> Option<DaemonOnce> {
    let bin = nixlingd_bin()?;

    let tmp = tempfile::tempdir().expect("tempdir");
    let run = tmp.path().join("run");
    let daemon_state_dir = run.join("daemon-state");
    let locks_dir = run.join("locks");
    std::fs::create_dir_all(&daemon_state_dir).expect("mk daemon-state");
    std::fs::create_dir_all(&locks_dir).expect("mk locks");

    let socket_path = run.join("public.sock");
    let state_lock = run.join("daemon.lock");
    let config_json = run.join("config.json");

    let group = primary_group_name();
    let config = serde_json::json!({
        "publicSocketPath": socket_path,
        "brokerSocketPath": run.join("priv.sock"),
        "stateLockPath": state_lock,
        "locksDir": locks_dir,
        "daemonUser": "root",
        "daemonGroup": "root",
        "publicSocketGroup": group,
        "launcherUsers": ["launcher-user"],
        "adminUsers": ["admin-user"],
        "serverVersion": "0.4.0",
        "acceptedClientVersionRange": ">=0.4.0, <0.5.0"
    });
    {
        let mut f = std::fs::File::create(&config_json).expect("write config.json");
        f.write_all(serde_json::to_string_pretty(&config).unwrap().as_bytes())
            .expect("write config bytes");
    }

    let child = Command::new(&bin)
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
        .env("NIXLINGD_TEST_PEER_UID", peer.uid.to_string())
        .env("NIXLINGD_TEST_PEER_GID", peer.gid.to_string())
        .env("NIXLINGD_TEST_PEER_USERNAME", peer.username)
        .env("NIXLINGD_TEST_PEER_GROUPS", peer.groups)
        .env("NIXLING_SKIP_KERNEL_MODULE_CHECK", "1")
        // Quiet the daemon's startup/autostart tracing so it does not pollute
        // test output; assertions over the CLI response give the signal.
        .env("RUST_LOG", "off")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn nixlingd serve --once");

    wait_for_socket(&socket_path, Duration::from_secs(15));

    Some(DaemonOnce {
        child,
        socket_path,
        daemon_state_dir,
        _tmp: tmp,
    })
}

/// Poll until `path` is a socket or the timeout elapses.
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
