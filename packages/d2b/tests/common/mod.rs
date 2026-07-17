//! Shared CLI-contract integration-test harness.
//!
//! Most CLI-contract cases drive the `d2b` binary against static fixtures
//! and need nothing here. A handful of cases (audit / host-check daemon-backed
//! paths) must talk to a real, KVM-free `d2bd` over `AF_UNIX` +
//! `SO_PEERCRED`. This module spawns such a daemon in `--once` mode with a
//! synthetic config and a caller-chosen test peer identity.
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

/// A test peer identity presented to the daemon via the `D2BD_TEST_PEER_*`
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
    pub fn wait(mut self) -> std::process::ExitStatus {
        self.child.wait().expect("wait for d2bd")
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

fn sha256_digest(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn write_bundle_with_hash(bundle_path: &Path, mut bundle: serde_json::Value) {
    use std::os::unix::fs::PermissionsExt;

    bundle
        .as_object_mut()
        .expect("bundle object")
        .remove("bundleHash");
    let mut canonical_bundle = bundle.clone();
    canonical_bundle
        .as_object_mut()
        .expect("canonical bundle object")
        .insert("artifactHashes".to_owned(), serde_json::Value::Null);
    let canonical = serde_json::to_vec(&canonical_bundle).expect("encode canonical bundle");
    bundle.as_object_mut().expect("bundle object").insert(
        "bundleHash".to_owned(),
        serde_json::Value::String(sha256_digest(&canonical)),
    );
    std::fs::write(
        bundle_path,
        serde_json::to_vec_pretty(&bundle).expect("encode hermetic bundle"),
    )
    .expect("write hermetic bundle");
    std::fs::set_permissions(bundle_path, std::fs::Permissions::from_mode(0o640))
        .expect("chmod hermetic bundle");
}

pub fn refresh_bundle_integrity(destination: &Path, changed_artifacts: &[&str]) {
    let bundle_path = destination.join("bundle.json");
    let mut bundle: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&bundle_path).expect("read bundle"))
            .expect("decode bundle");
    let artifact_hashes = bundle
        .as_object_mut()
        .expect("bundle object")
        .get_mut("artifactHashes")
        .and_then(serde_json::Value::as_object_mut)
        .expect("bundle artifact hashes");
    for artifact in changed_artifacts {
        let bytes =
            std::fs::read(destination.join(artifact)).expect("read changed bundle artifact");
        artifact_hashes.insert(
            (*artifact).to_owned(),
            serde_json::Value::String(sha256_digest(&bytes)),
        );
    }
    write_bundle_with_hash(&bundle_path, bundle);
}

pub fn build_hermetic_bundle_tree(fixtures: &Path, destination: &Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(destination.join("closures")).expect("mk fixture closures");
    std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o750))
        .expect("chmod fixture directory");
    std::fs::set_permissions(
        destination.join("closures"),
        std::fs::Permissions::from_mode(0o750),
    )
    .expect("chmod fixture closures");
    for entry in std::fs::read_dir(fixtures).expect("read fixture directory") {
        let entry = entry.expect("fixture entry");
        if entry.file_type().expect("fixture type").is_file() {
            let bytes = std::fs::read(entry.path()).expect("read fixture");
            let path = destination.join(entry.file_name());
            std::fs::write(&path, bytes).expect("write fixture");
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640))
                .expect("chmod fixture");
        }
    }
    for entry in std::fs::read_dir(fixtures.join("closures")).expect("read fixture closures") {
        let entry = entry.expect("fixture closure");
        let bytes = std::fs::read(entry.path()).expect("read fixture closure");
        let path = destination.join("closures").join(entry.file_name());
        std::fs::write(&path, bytes).expect("write fixture closure");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640))
            .expect("chmod fixture closure");
    }

    let provider_registry_path = destination.join("provider-registry-v2.json");
    let mut provider_registry: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&provider_registry_path).expect("read provider registry"),
    )
    .expect("decode provider registry");
    let provider_registry_object = provider_registry
        .as_object_mut()
        .expect("provider registry object");
    provider_registry_object.insert(
        "configurationFingerprint".to_owned(),
        serde_json::Value::String("0".repeat(64)),
    );
    provider_registry_object.insert("providers".to_owned(), serde_json::Value::Array(Vec::new()));
    let provider_registry_bytes =
        serde_json::to_vec(&provider_registry).expect("encode empty provider registry");
    std::fs::write(&provider_registry_path, &provider_registry_bytes)
        .expect("write empty provider registry");
    std::fs::set_permissions(
        &provider_registry_path,
        std::fs::Permissions::from_mode(0o640),
    )
    .expect("chmod provider registry");
    let provider_registry_digest = sha256_digest(&provider_registry_bytes);

    let bundle_path = destination.join("bundle.json");
    let mut bundle: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&bundle_path).expect("read copied bundle"))
            .expect("decode copied bundle");
    let object = bundle.as_object_mut().expect("bundle object");
    for field in [
        "allocatorPath",
        "hostPath",
        "privilegesPath",
        "processesPath",
        "providerRegistryV2Path",
        "publicManifestPath",
        "realmControllersPath",
        "realmIdentityPath",
        "realmWorkloadsLauncherV2Path",
        "storagePath",
        "syncPath",
        "unsafeLocalWorkloadsPath",
    ] {
        let name = if field == "publicManifestPath" {
            "manifest.json".to_owned()
        } else {
            object
                .get(field)
                .and_then(serde_json::Value::as_str)
                .and_then(|path| Path::new(path).file_name())
                .expect("bundle artifact filename")
                .to_string_lossy()
                .into_owned()
        };
        object.insert(field.to_owned(), serde_json::Value::String(name));
    }
    let artifact_hashes = object
        .get("artifactHashes")
        .and_then(serde_json::Value::as_object)
        .expect("bundle artifact hashes")
        .iter()
        .map(|(path, digest)| {
            let key = if path.ends_with("/vms.json") {
                "manifest.json".to_owned()
            } else if Path::new(path).is_absolute() {
                Path::new(path)
                    .file_name()
                    .expect("artifact filename")
                    .to_string_lossy()
                    .into_owned()
            } else {
                path.clone()
            };
            let digest = if key == "provider-registry-v2.json" {
                serde_json::Value::String(provider_registry_digest.clone())
            } else {
                digest.clone()
            };
            (key, digest)
        })
        .collect();
    object.insert(
        "artifactHashes".to_owned(),
        serde_json::Value::Object(artifact_hashes),
    );
    write_bundle_with_hash(&bundle_path, bundle);
    d2b_core::bundle_resolver::BundleResolver::load_with_policy(
        &destination.join("bundle.json"),
        &d2b_core::bundle_resolver::BundleVerifyPolicy::for_tests(),
    )
    .expect("validate hermetic bundle");
}

/// Spawn `d2bd serve --once --test-listen-on <socket>` with a synthetic
/// config presenting `peer` as the connecting identity, and block until the
/// public socket exists. Returns `None` when the daemon-spawn harness is
/// unavailable (so the caller can skip).
///
/// In `--once` mode the daemon accepts exactly one request and then exits, so
/// the caller should run a single `d2b` invocation against
/// `socket_path` and then call [`DaemonOnce::wait`].
pub fn spawn_d2bd_once(peer: &TestPeer) -> Option<DaemonOnce> {
    let artifacts_dir = std::env::var_os("D2B_FIXTURES").map(PathBuf::from)?;
    spawn_d2bd_inner(peer, Some(&artifacts_dir), None)
}

/// Spawn `d2bd serve --once` wired to read its bundle/host/closure
/// artifacts from `artifacts_dir` and to drive every `host check` probe from
/// the JSON `fixture_path` (`D2B_HOST_CHECK_FIXTURE`). Used by the
/// daemon-backed `hostCheck` cases migrated from
/// tests/cli-rust-native-host-check.sh.
///
/// `artifacts_dir` must contain a `bundle.json` whose `hostPath` /
/// `processesPath` resolve (relative to the dir) to fixture artifacts that
/// live there too, plus a `closures/` subdir — see
/// `host_check_contract::build_hermetic_bundle_tree`, which rewrites the
/// committed fixture-smoke bundle so the absolute `/etc/d2b/*` paths can
/// never leak the real host's artifacts into the test.
pub fn spawn_d2bd_host_check(
    artifacts_dir: &Path,
    fixture_path: &Path,
    peer: &TestPeer,
) -> Option<DaemonOnce> {
    spawn_d2bd_inner(peer, Some(artifacts_dir), Some(fixture_path))
}

fn spawn_d2bd_inner(
    peer: &TestPeer,
    artifacts_dir: Option<&Path>,
    fixture_path: Option<&Path>,
) -> Option<DaemonOnce> {
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
    let artifacts_dir = artifacts_dir.map(|dir| {
        if fixture_path.is_none() {
            let destination = run.join("artifacts");
            build_hermetic_bundle_tree(dir, &destination);
            destination
        } else {
            dir.to_path_buf()
        }
    });

    let group = primary_group_name();
    let mut config = serde_json::json!({
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
        "acceptedClientVersionRange": ">=0.4.0, <0.5.0",
        "gatewayConfigPath": run.join("gateway.json"),
        "realmControllersConfigPath": run.join("realm-controllers.json"),
        "realmIdentityConfigPath": run.join("realm-identity.json"),
        "artifacts": {
            "publicManifestPath": run.join("manifest.json"),
            "bundlePath": run.join("bundle.json"),
            "hostPath": run.join("host.json"),
            "processesPath": run.join("processes.json"),
            "closuresDir": run.join("closures")
        }
    });
    if let Some(dir) = artifacts_dir.as_deref() {
        config.as_object_mut().unwrap().insert(
            "artifacts".to_owned(),
            serde_json::json!({
                "publicManifestPath": dir.join("manifest.json"),
                "bundlePath": dir.join("bundle.json"),
                "hostPath": dir.join("host.json"),
                "processesPath": dir.join("processes.json"),
                "closuresDir": dir.join("closures"),
            }),
        );
    }
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
        .env("D2BD_TEST_PEER_UID", peer.uid.to_string())
        .env("D2BD_TEST_PEER_GID", peer.gid.to_string())
        .env("D2BD_TEST_PEER_USERNAME", peer.username)
        .env("D2BD_TEST_PEER_GROUPS", peer.groups)
        // The daemon's startup kernel-module gate reads the real /proc/modules
        // (NOT the host-check fixture); bypass it so the daemon starts on any
        // host. The host-check dispatch itself still runs entirely from
        // D2B_HOST_CHECK_FIXTURE.
        .env("D2B_SKIP_KERNEL_MODULE_CHECK", "1")
        // Quiet the daemon's startup/autostart tracing so it does not pollute
        // test output; assertions over the CLI response give the signal.
        .env("RUST_LOG", "off")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(fixture) = fixture_path {
        command.env("D2B_HOST_CHECK_FIXTURE", fixture);
    }
    let child = command.spawn().expect("spawn d2bd serve --once");

    wait_for_socket(&socket_path, Duration::from_secs(15));

    Some(DaemonOnce {
        child,
        socket_path,
        daemon_state_dir,
        _tmp: tmp,
    })
}

/// Drive one daemon `hostCheck` round-trip through the bundled `d2bd
/// test-client` (the daemon binary's own subcommand) and return the parsed
/// `hostCheckResponse`.
///
/// The client opens a single `AF_UNIX`/`SOCK_SEQPACKET` connection, sends a
/// `hello` frame followed by `{"type":"hostCheck","strict":<strict>}`, and
/// prints one JSON line per response frame. The LAST line is the
/// `hostCheckResponse`. Panics if the harness binary is unavailable — callers
/// that obtained a [`DaemonOnce`] from [`spawn_d2bd_host_check`] already
/// know `d2bd_bin()` is `Some`.
pub fn daemon_host_check_response(socket_path: &Path, strict: bool) -> serde_json::Value {
    let bin = d2bd_bin().expect("d2bd test-client binary");
    let host_check_frame = format!("{{\"type\":\"hostCheck\",\"strict\":{strict}}}");
    let out = Command::new(&bin)
        .arg("test-client")
        .arg("--socket")
        .arg(socket_path)
        .arg("--frame-json")
        .arg(r#"{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":[]}"#)
        .arg("--frame-json")
        .arg(&host_check_frame)
        .output()
        .expect("spawn d2bd test-client");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let last = stdout
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or_else(|| {
            panic!(
                "d2bd test-client produced no response line; stderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
    serde_json::from_str(last)
        .unwrap_or_else(|err| panic!("hostCheckResponse was not valid JSON: {err}\nline: {last}"))
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
