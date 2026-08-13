#![allow(dead_code)]

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use tempfile::{Builder, TempDir};

pub const HELLO_FRAME: &str =
    r#"{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":[]}"#;

pub fn d2bd_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_d2bd"))
}

#[derive(Debug, Clone)]
pub struct TestPeer {
    /// Synthetic values used only by the explicit forged-environment test.
    pub uid: u32,
    /// Synthetic values used only by the explicit forged-environment test.
    pub gid: u32,
    /// Synthetic values used only by the explicit forged-environment test.
    pub username: &'static str,
    /// Synthetic values used only by the explicit forged-environment test.
    pub groups: &'static str,
    kind: TestPeerKind,
}

#[derive(Debug, Clone, Copy)]
enum TestPeerKind {
    Launcher,
    Admin,
    Deny,
    Daemon,
}

impl TestPeer {
    pub fn launcher() -> Self {
        Self {
            uid: 60003,
            gid: 60003,
            username: "launcher-user",
            groups: "wheel",
            kind: TestPeerKind::Launcher,
        }
    }

    pub fn admin() -> Self {
        Self {
            uid: 60004,
            gid: 60004,
            username: "admin-user",
            groups: "wheel",
            kind: TestPeerKind::Admin,
        }
    }

    pub fn deny(uid: u32, username: &'static str, groups: &'static str) -> Self {
        Self {
            uid,
            gid: uid,
            username,
            groups,
            kind: TestPeerKind::Deny,
        }
    }

    pub fn daemon() -> Self {
        Self {
            uid: 0,
            gid: 0,
            username: "daemon-user",
            groups: "root",
            kind: TestPeerKind::Daemon,
        }
    }
}

pub struct DaemonFixture {
    tmp: TempDir,
    root_dir: PathBuf,
    pub run_dir: PathBuf,
    pub socket_path: PathBuf,
    pub broker_socket_path: PathBuf,
    pub state_lock_path: PathBuf,
    pub locks_dir: PathBuf,
    pub daemon_state_dir: PathBuf,
    pub config_path: PathBuf,
}

impl DaemonFixture {
    pub fn new(prefix: &str) -> Self {
        let parent = PathBuf::from("target/d2bd-integration-tests");
        fs::create_dir_all(&parent).expect("create d2bd integration temp parent");
        let tmp = Builder::new()
            .prefix(prefix)
            .tempdir_in(&parent)
            .expect("create d2bd integration tempdir");
        let root_dir = relative_to_cwd(tmp.path());
        let run_dir = root_dir.join("run");
        let socket_path = run_dir.join("public.sock");
        let broker_socket_path = run_dir.join("priv.sock");
        let state_lock_path = run_dir.join("daemon.lock");
        let locks_dir = run_dir.join("locks");
        let daemon_state_dir = tmp.path().join("state");
        let config_path = tmp.path().join("config.json");

        fs::create_dir_all(&run_dir).expect("create run dir");
        fs::create_dir_all(&locks_dir).expect("create locks dir");
        fs::create_dir_all(&daemon_state_dir).expect("create daemon state dir");
        fs::set_permissions(&run_dir, fs::Permissions::from_mode(0o755)).expect("chmod run dir");

        Self {
            tmp,
            root_dir,
            run_dir,
            socket_path,
            broker_socket_path,
            state_lock_path,
            locks_dir,
            daemon_state_dir,
            config_path,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root_dir
    }

    pub fn reset_runtime_endpoints(&self) {
        remove_file_if_present(&self.socket_path);
        remove_file_if_present(&self.state_lock_path);
        fs::create_dir_all(&self.locks_dir).expect("ensure locks dir");
    }

    pub fn write_config(&self, launcher_users: &[&str], admin_users: &[&str]) {
        write_daemon_config(self, launcher_users, admin_users);
    }
}

fn relative_to_cwd(path: &Path) -> PathBuf {
    let cwd = std::env::current_dir().expect("current dir");
    path.strip_prefix(&cwd).unwrap_or(path).to_path_buf()
}

fn remove_file_if_present(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => panic!("remove {}: {err}", path.display()),
    }
}

pub fn primary_group_name() -> String {
    let gid = nix::unistd::getgid();
    nix::unistd::Group::from_gid(gid)
        .ok()
        .flatten()
        .map(|group| group.name)
        .unwrap_or_else(|| gid.to_string())
}

pub fn lifecycle_group_name() -> String {
    nix::unistd::Group::from_name("d2b")
        .ok()
        .flatten()
        .map(|group| group.name)
        .unwrap_or_else(primary_group_name)
}

pub fn current_username() -> String {
    nix::unistd::User::from_uid(nix::unistd::getuid())
        .ok()
        .flatten()
        .map(|user| user.name)
        .unwrap_or_else(|| "d2b-test-user".to_owned())
}

pub fn write_daemon_config(fixture: &DaemonFixture, launcher_users: &[&str], admin_users: &[&str]) {
    write_daemon_config_with_artifacts(fixture, launcher_users, admin_users, None);
}

pub fn write_daemon_config_with_artifacts(
    fixture: &DaemonFixture,
    launcher_users: &[&str],
    admin_users: &[&str],
    artifacts: Option<serde_json::Value>,
) {
    let mut config = serde_json::json!({
        "publicSocketPath": path_string(&fixture.socket_path),
        "brokerSocketPath": path_string(&fixture.broker_socket_path),
        "stateLockPath": path_string(&fixture.state_lock_path),
        "locksDir": path_string(&fixture.locks_dir),
        "daemonUser": "root",
        "daemonGroup": "root",
        "publicSocketGroup": primary_group_name(),
        "launcherUsers": launcher_users,
        "adminUsers": admin_users,
        "serverVersion": "0.4.0",
        "acceptedClientVersionRange": ">=0.4.0, <0.5.0",
        "gatewayConfigPath": path_string(&fixture.root().join("gateway.json"))
    });
    if let Some(artifacts) = artifacts {
        config
            .as_object_mut()
            .expect("daemon config JSON object")
            .insert("artifacts".to_owned(), artifacts);
    }
    let mut file = fs::File::create(&fixture.config_path).expect("create daemon config");
    file.write_all(
        serde_json::to_string_pretty(&config)
            .expect("serialize daemon config")
            .as_bytes(),
    )
    .expect("write daemon config");
}

pub fn set_public_socket_group(fixture: &DaemonFixture, group: &str) {
    let bytes = fs::read(&fixture.config_path).expect("read daemon config");
    let mut config: serde_json::Value =
        serde_json::from_slice(&bytes).expect("parse daemon config");
    config["publicSocketGroup"] = serde_json::Value::String(group.to_owned());
    fs::write(
        &fixture.config_path,
        serde_json::to_vec_pretty(&config).expect("serialize daemon config"),
    )
    .expect("write daemon config");
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub struct SpawnedProcess {
    child: Option<Child>,
}

impl SpawnedProcess {
    pub fn from_child(child: Child) -> Self {
        Self { child: Some(child) }
    }

    pub fn id(&self) -> u32 {
        self.child.as_ref().expect("process is live").id()
    }

    pub fn wait(mut self) -> ExitStatus {
        self.child
            .take()
            .expect("process already consumed")
            .wait()
            .expect("wait for process")
    }

    pub fn kill_and_wait(mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn wait_timeout(mut self, timeout: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            let child = self.child.as_mut().expect("process already consumed");
            if let Some(status) = child.try_wait().expect("poll process") {
                self.child.take();
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for SpawnedProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn spawn_d2bd_serve(
    fixture: &DaemonFixture,
    peer: &TestPeer,
    once: bool,
    state_restore_report: Option<&Path>,
) -> SpawnedProcess {
    spawn_d2bd_serve_inner(fixture, peer, once, state_restore_report, true)
}

pub fn spawn_d2bd_serve_with_forged_peer_env(
    fixture: &DaemonFixture,
    peer: &TestPeer,
    once: bool,
    state_restore_report: Option<&Path>,
) -> SpawnedProcess {
    spawn_d2bd_serve_inner(fixture, peer, once, state_restore_report, false)
}

fn spawn_d2bd_serve_inner(
    fixture: &DaemonFixture,
    peer: &TestPeer,
    once: bool,
    state_restore_report: Option<&Path>,
    configure_peer: bool,
) -> SpawnedProcess {
    fixture.reset_runtime_endpoints();
    if configure_peer {
        configure_real_peer(fixture, peer);
    }
    let mut command = Command::new(d2bd_bin());
    command
        .arg("serve")
        .arg("--config")
        .arg(&fixture.config_path)
        .arg("--test-listen-on")
        .arg(&fixture.socket_path)
        .arg("--state-lock")
        .arg(&fixture.state_lock_path)
        .arg("--locks-dir")
        .arg(&fixture.locks_dir)
        .arg("--daemon-state-dir")
        .arg(&fixture.daemon_state_dir);
    if once {
        command.arg("--once");
    }
    command
        .arg("--allow-unprivileged-runtime-dir")
        .arg("--no-drop-privileges")
        .env("D2B_SKIP_KERNEL_MODULE_CHECK", "1")
        .env("RUST_LOG", "off")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !configure_peer {
        command
            .env("D2BD_TEST_PEER_UID", peer.uid.to_string())
            .env("D2BD_TEST_PEER_GID", peer.gid.to_string())
            .env("D2BD_TEST_PEER_USERNAME", peer.username)
            .env("D2BD_TEST_PEER_GROUPS", peer.groups);
    }
    if let Some(report) = state_restore_report {
        command.arg("--test-state-restore-report").arg(report);
    }
    let child = command.spawn().expect("spawn d2bd serve");
    wait_for_socket(&fixture.socket_path, Duration::from_secs(15));
    SpawnedProcess::from_child(child)
}

fn configure_real_peer(fixture: &DaemonFixture, peer: &TestPeer) {
    let bytes = fs::read(&fixture.config_path).expect("read daemon config");
    let mut config: serde_json::Value =
        serde_json::from_slice(&bytes).expect("parse daemon config");
    let username = current_username();
    let lifecycle_group = lifecycle_group_name();
    let configured_admin = config
        .get("adminUsers")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|users| {
            users.iter().any(|value| {
                value.as_str() == Some(peer.username) || value.as_str() == Some(username.as_str())
            })
        });
    config["publicSocketGroup"] = serde_json::Value::String(lifecycle_group);
    match peer.kind {
        TestPeerKind::Launcher => {
            config["launcherUsers"] = serde_json::json!([username]);
            config["adminUsers"] = if configured_admin {
                serde_json::json!([current_username()])
            } else {
                serde_json::json!([])
            };
        }
        TestPeerKind::Admin => {
            config["launcherUsers"] = serde_json::json!([username]);
            config["adminUsers"] = serde_json::json!([username]);
        }
        TestPeerKind::Deny => {
            config["launcherUsers"] = serde_json::json!(["unrelated-test-user"]);
            config["adminUsers"] = serde_json::json!([]);
            config["publicSocketGroup"] = serde_json::Value::String("d2b-test-denied".to_owned());
        }
        TestPeerKind::Daemon => {
            config["daemonUser"] = serde_json::Value::String(username.clone());
            config["launcherUsers"] = serde_json::json!([username]);
            config["adminUsers"] = serde_json::json!([username]);
        }
    }
    fs::write(
        &fixture.config_path,
        serde_json::to_vec_pretty(&config).expect("serialize daemon config"),
    )
    .expect("write daemon config");
}

pub fn spawn_lock_only(config: &Path, state_lock: &Path, hold_seconds: u64) -> SpawnedProcess {
    let child = Command::new(d2bd_bin())
        .arg("lock-only")
        .arg("--config")
        .arg(config)
        .arg("--state-lock")
        .arg(state_lock)
        .arg("--allow-unprivileged-runtime-dir")
        .arg("--hold-seconds")
        .arg(hold_seconds.to_string())
        .env("RUST_LOG", "off")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn d2bd lock-only");
    SpawnedProcess::from_child(child)
}

pub fn test_client(socket: &Path, frames: &[&str]) -> (i32, String) {
    let mut command = Command::new(d2bd_bin());
    command.arg("test-client").arg("--socket").arg(socket);
    for frame in frames {
        command.arg("--frame-json").arg(frame);
    }
    let output = command.output().expect("spawn d2bd test-client");
    (status_code(&output.status), combined_output(&output))
}

pub fn run_lock_only(config: &Path, state_lock: &Path, _locks_dir: &Path) -> (i32, String) {
    let output = Command::new(d2bd_bin())
        .arg("lock-only")
        .arg("--config")
        .arg(config)
        .arg("--state-lock")
        .arg(state_lock)
        .arg("--allow-unprivileged-runtime-dir")
        .arg("--hold-seconds")
        .arg("1")
        .env("RUST_LOG", "off")
        .output()
        .expect("run d2bd lock-only");
    (status_code(&output.status), combined_output(&output))
}

fn status_code(status: &ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}

fn combined_output(output: &std::process::Output) -> String {
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    combined
}

pub fn wait_for_socket(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if fs::metadata(path)
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for socket: {}", path.display());
}

pub fn wait_for_file(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for file: {}", path.display());
}

pub fn assert_contains(haystack: &str, needle: &str, context: &str) {
    assert!(
        haystack.contains(needle),
        "{context}: missing {needle:?} in output:\n{haystack}"
    );
}

pub fn last_non_empty_line(output: &str) -> &str {
    output
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_else(|| panic!("no non-empty line in output:\n{output}"))
}
