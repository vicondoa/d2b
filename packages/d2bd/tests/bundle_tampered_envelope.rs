//! P0fu2 integration test: BundleTampered → TypedError::BundleTampered
//! envelope mapping.
//!
//! Verifies the complete mapping chain:
//!   BundleResolver::load_with_policy (tampered file)
//!     → d2b_core::Error::Bundle(BundleError::Tampered { path, reason })
//!       → TypedError::BundleTampered { path, reason }
//!         → ErrorEnvelope { kind: "bundle-tampered", exit_code: 60, … }
//!
//! Uses `load_with_policy` with a current-user policy so the tests run
//! without root: the policy matches the temp files' uid/gid and the tamper
//! is introduced via mode 0o644 (world-readable; policy requires 0o640).

use d2b_core::bundle_resolver::{BundleResolver, BundleVerifyPolicy};
use d2b_core::error::{BundleError, Error as CoreError};
use d2bd::typed_error::TypedError;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use tempfile::TempDir;

mod common;

// ---------------------------------------------------------------
// Helpers (adapted from d2b-core's bundle_resolver_tamper.rs)
// ---------------------------------------------------------------

fn current_user_policy() -> BundleVerifyPolicy {
    BundleVerifyPolicy {
        required_uid: rustix::process::getuid().as_raw(),
        required_gid: Some(rustix::process::getgid().as_raw()),
        required_mode: 0o640,
    }
}

/// Write `content` to `path` with Unix mode `mode`.
fn write_with_mode(path: &std::path::Path, content: &[u8], mode: u32) {
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(path)
        .expect("create file")
        .write_all(content)
        .expect("write file");
}

#[allow(dead_code)]
fn set_mode(path: &std::path::Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set permissions");
}

/// Minimal bundle JSON without a `bundleHash` field (hash check is secondary
/// to the mode check; the mode check fires first so no hash is needed).
fn minimal_bundle_json() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "bundleVersion": 4,
        "schemaVersion": "v2",
        "publicManifestPath": "vms.json",
        "hostPath": "host.json",
        "processesPath": "processes.json",
        "privilegesPath": "privileges.json",
        "closures": [],
        "minijailProfiles": [],
        "managedKeys": {
            "keysDir": "/var/lib/d2b/keys",
            "knownHostsPath": "/var/lib/d2b/known_hosts.d2b",
            "overrides": []
        },
        "generation": {
            "generator": "test",
            "sourceRevision": null,
            "generatedAt": null
        }
    }))
    .expect("bundle json serializes")
}

/// Apply the same mapping that `load_bundle_resolver` in `d2bd/src/lib.rs`
/// uses to convert a `d2b_core::Error` into a `TypedError`.
fn map_core_error(err: CoreError) -> TypedError {
    match err {
        CoreError::Bundle(BundleError::Tampered { path, reason }) => {
            TypedError::BundleTampered { path, reason }
        }
        other => TypedError::InternalIo {
            context: "load bundle resolver".to_owned(),
            detail: other.to_string(),
        },
    }
}

// ---------------------------------------------------------------
// Test: mode 0o644 → kind=bundle-tampered, exit_code=60
// ---------------------------------------------------------------

#[test]
fn tampered_mode_maps_to_bundle_tampered_envelope() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");

    // Write with 0o644 (world-readable — policy expects 0o640).
    write_with_mode(&bundle_path, &minimal_bundle_json(), 0o644);

    let policy = current_user_policy(); // requires 0o640
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("mode-tampered file should be rejected");

    // Step 3: assert the core error shape.
    match &err {
        CoreError::Bundle(BundleError::Tampered { reason, .. }) => {
            assert_eq!(reason, "mode", "expected reason=mode, got {reason:?}");
        }
        other => panic!("expected BundleTampered(mode), got {other:?}"),
    }

    // Step 4: map via the same conversion as `load_bundle_resolver`.
    let typed = map_core_error(err);

    // Step 5: verify the envelope.
    match &typed {
        TypedError::BundleTampered { reason, .. } => {
            assert_eq!(reason, "mode");
        }
        other => panic!("expected TypedError::BundleTampered, got {other:?}"),
    }
    assert_eq!(typed.kind(), "bundle-tampered");
    assert_eq!(typed.exit_code(), 60);
    assert!(
        typed.message().contains("mode"),
        "message should contain reason; got: {:?}",
        typed.message()
    );
}

// ---------------------------------------------------------------
// Test: symlink → kind=bundle-tampered, exit_code=60
// ---------------------------------------------------------------

#[test]
fn tampered_symlink_maps_to_bundle_tampered_envelope() {
    let dir = TempDir::new().expect("tempdir");
    let real_path = dir.path().join("real-bundle.json");
    let bundle_path = dir.path().join("bundle.json");

    write_with_mode(&real_path, &minimal_bundle_json(), 0o640);
    std::os::unix::fs::symlink(&real_path, &bundle_path).expect("create symlink");

    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("symlink bundle.json should be rejected");

    let typed = map_core_error(err);

    assert_eq!(typed.kind(), "bundle-tampered");
    assert_eq!(typed.exit_code(), 60);
    assert!(
        typed.message().contains("symlink"),
        "message should contain reason; got: {:?}",
        typed.message()
    );
}

// ---------------------------------------------------------------
// Test: non-Bundle error maps to InternalIo (not BundleTampered)
// ---------------------------------------------------------------

#[test]
fn non_bundle_error_maps_to_internal_io() {
    let missing = PathBuf::from("/nonexistent/path/bundle.json");
    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&missing, &policy)
        .expect_err("missing file should produce an error");

    let typed = map_core_error(err);

    // Should NOT be BundleTampered — missing file is an I/O error.
    assert_ne!(
        typed.kind(),
        "bundle-tampered",
        "missing-file error should not map to bundle-tampered"
    );
}

// ---------------------------------------------------------------
// Test: remediation text contains canonical operator guidance
// ---------------------------------------------------------------

#[test]
fn bundle_tampered_remediation_contains_rebuild_guidance() {
    let typed = TypedError::BundleTampered {
        path: PathBuf::from("/var/lib/d2b/current-bundle/bundle.json"),
        reason: "mode".to_owned(),
    };
    let remediation = typed.remediation();
    assert!(
        remediation.contains("nixos-rebuild switch"),
        "remediation should mention nixos-rebuild switch; got: {remediation:?}"
    );
    assert!(
        remediation.contains("0640"),
        "remediation should mention expected mode 0640; got: {remediation:?}"
    );
}

// ---------------------------------------------------------------
// Test: to_envelope round-trip
// ---------------------------------------------------------------

#[test]
fn bundle_tampered_to_envelope_round_trip() {
    let typed = TypedError::BundleTampered {
        path: PathBuf::from("/var/lib/d2b/current-bundle/bundle.json"),
        reason: "hash".to_owned(),
    };
    // `to_envelope` calls `log_raw_detail` (which emits a tracing event) and
    // builds the public envelope.  The tracing subscriber isn't initialised
    // here so the event is silently discarded — that's fine for a unit test.
    let envelope = typed.to_envelope();
    assert_eq!(envelope.kind, "bundle-tampered");
    assert_eq!(envelope.exit_code, 60);
    assert!(
        envelope.message.contains("hash"),
        "envelope message should contain reason; got: {:?}",
        envelope.message
    );
}

#[test]
fn daemon_refuses_a_tampered_bundle_during_provider_registry_startup() {
    let fixture = common::DaemonFixture::new("bundle-tampered-daemon.");
    let username = common::current_username();
    fixture.write_config(&[&username], &[&username]);

    let artifacts_dir = fixture.root().join("artifacts");
    fs::create_dir_all(&artifacts_dir).expect("create artifacts dir");
    let bundle_path = artifacts_dir.join("bundle.json");
    write_with_mode(&bundle_path, &minimal_bundle_json(), 0o644);

    let mut config: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.config_path).expect("read daemon config"))
            .expect("daemon config is JSON");
    config["artifacts"] = serde_json::json!({
        "publicManifestPath": artifacts_dir.join("vms.json"),
        "bundlePath": bundle_path,
        "hostPath": artifacts_dir.join("host.json"),
        "processesPath": artifacts_dir.join("processes.json"),
        "closuresDir": artifacts_dir.join("closures")
    });
    fs::write(
        &fixture.config_path,
        serde_json::to_vec_pretty(&config).expect("serialize daemon config"),
    )
    .expect("rewrite daemon config with test artifact paths");

    let server = common::spawn_d2bd_serve_expect_startup_failure(&fixture);
    let status = server.wait();

    assert!(
        !status.success(),
        "d2bd must fail startup before serving a tampered provider bundle"
    );
}
