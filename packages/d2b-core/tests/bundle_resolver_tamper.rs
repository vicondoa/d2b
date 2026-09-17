//! BundleResolver tamper-resistance integration tests.
//!
//! Each test creates a self-contained fake bundle root inside a
//! `tempfile::TempDir` - the real `/etc/d2b` is never touched.
//!
//! The tests use [`BundleVerifyPolicy`] with the **current process's**
//! uid/gid so that files created without `chown` still pass the owner
//! check.  The "owner = nobody" test (`tamper_owner_wrong_uid`) requires
//! `chown` and is skipped automatically when the process is not root.
//!
//! These bundles are v3 zone-native (`schemaVersion: "v3"`, `bundleVersion: 1`,
//! empty `zones`), so they load via the production zone-native path. The
//! probes below tamper with `bundle.json` itself (identity, ownership, mode,
//! SHA-256 self-hash), which the loader verifies before sibling artifacts.

use d2b_core::bundle_resolver::{BundleResolver, BundleVerifyPolicy};
use d2b_core::error::{BundleError, Error};
use sha2::Digest as _;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use tempfile::TempDir;

// ---------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------

/// Return a [`BundleVerifyPolicy`] whose uid/gid match the running
/// process so test files pass the owner check without `chown`.
fn current_user_policy() -> BundleVerifyPolicy {
    BundleVerifyPolicy {
        required_uid: rustix::process::getuid().as_raw(),
        required_gid: Some(rustix::process::getgid().as_raw()),
        required_mode: 0o640,
    }
}

/// Write `content` to `path` with mode 0o640.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn write_private(path: &Path, content: &[u8]) {
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o640)
        .open(path)
        .expect("create file")
        .write_all(content)
        .expect("write file");
}

/// Compute `"sha256:<hex>"` over `data` - same algorithm as the Rust verifier.
fn sha256_hex(data: &[u8]) -> String {
    let digest: [u8; 32] = sha2::Sha256::digest(data).into();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// Build a minimal but fully-parseable v3 zone-native bundle JSON (without
/// `bundleHash`). Returns the canonical JSON bytes *without* the hash field.
fn minimal_bundle_json_no_hash() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "bundleVersion": 1,
        "schemaVersion": "v3",
        "privilegesPath": "privileges.json",
        "zones": [],
        "artifactHashes": {},
        "generation": {
            "generator": "test",
            "sourceRevision": null,
            "generatedAt": null
        }
    }))
    .expect("bundle json serializes")
}

/// Build bundle JSON with a correct `bundleHash` embedded.
///
/// The verifier re-derives the digest over the serialization with
/// `bundleHash` absent and `artifactHashes` nullified, so the hash input is
/// the pre-hash bytes with `artifactHashes` forced to `null`.
fn bundle_json_with_hash(pre_hash_bytes: &[u8]) -> Vec<u8> {
    let mut value: serde_json::Value =
        serde_json::from_slice(pre_hash_bytes).expect("pre-hash bundle parses");
    let hash = self_hash(&value);
    value
        .as_object_mut()
        .expect("bundle is object")
        .insert("bundleHash".to_owned(), serde_json::Value::String(hash));
    serde_json::to_vec(&value).expect("bundle with hash serializes")
}

/// Compute the verifier-equivalent self-hash: strip `bundleHash` and
/// nullify `artifactHashes`, then SHA-256 the canonical serialization.
fn self_hash(value: &serde_json::Value) -> String {
    let mut preimage = value.clone();
    let obj = preimage.as_object_mut().expect("bundle is object");
    obj.remove("bundleHash");
    if obj.contains_key("artifactHashes") {
        obj.insert("artifactHashes".to_owned(), serde_json::Value::Null);
    }
    sha256_hex(&serde_json::to_vec(&preimage).expect("hash input serializes"))
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn set_mode_to(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set permissions");
}

/// Helper: assert `err` is `BundleTampered` with the given reason slug.
fn assert_tampered(err: &Error, expected_reason: &str) {
    match err {
        Error::Bundle(BundleError::Tampered { reason, .. }) => {
            assert_eq!(
                reason, expected_reason,
                "expected reason={expected_reason:?} but got {reason:?}"
            );
        }
        other => panic!("expected BundleTampered({expected_reason:?}), got {other:?}"),
    }
}

// ---------------------------------------------------------------
// Test 1: symlink at bundle.json → BundleTampered { reason: "symlink" }
// ---------------------------------------------------------------
#[test]
fn tamper_symlink() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");
    let target = dir.path().join("real-bundle.json");

    write_private(&target, &minimal_bundle_json_no_hash());
    std::os::unix::fs::symlink(&target, &bundle_path).expect("create symlink");

    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("symlink should be rejected");
    assert_tampered(&err, "symlink");
}

// ---------------------------------------------------------------
// Test 2: owner = wrong uid → BundleTampered { reason: "owner" }
//
// Requires root (or CAP_CHOWN) to call fchown; skipped otherwise.
// ---------------------------------------------------------------
#[test]
fn tamper_owner_wrong_uid() {
    if rustix::process::getuid().as_raw() != 0 {
        eprintln!("tamper_owner_wrong_uid: skipping - not root (cannot chown)");
        return;
    }

    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");
    write_private(&bundle_path, &minimal_bundle_json_no_hash());

    // Change owner to uid=65534 (nobody) with a direct syscall.
    nix::unistd::chown(&bundle_path, Some(nix::unistd::Uid::from_raw(65534)), None)
        .expect("chown to uid 65534");

    // Use a policy that expects uid=0 so the file fails.
    let policy = BundleVerifyPolicy {
        required_uid: 0,
        required_gid: None,
        required_mode: 0o640,
    };
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("wrong-owner file should be rejected");
    assert_tampered(&err, "owner");
}

// ---------------------------------------------------------------
// Test 3: mode 0644 → BundleTampered { reason: "mode" }
// ---------------------------------------------------------------
#[test]
fn tamper_mode_too_permissive() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");

    // Write with 0o644 (world-readable, not 0o640).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(&bundle_path)
        .expect("create")
        .write_all(&minimal_bundle_json_no_hash())
        .expect("write");

    let policy = current_user_policy(); // expects 0o640
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("wrong-mode file should be rejected");
    assert_tampered(&err, "mode");
}

// ---------------------------------------------------------------
// Test 4: corrupted file (hash mismatch) → BundleTampered { reason: "hash" }
// ---------------------------------------------------------------
#[test]
fn tamper_hash_mismatch() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");

    // Compute a valid hash over the pre-hash bytes.
    let pre_hash = minimal_bundle_json_no_hash();
    let with_hash = bundle_json_with_hash(&pre_hash);

    // Rewrite so the bundleHash field value is intact but the other content
    // differs - replace the bundleVersion value with a different number.
    let mut value: serde_json::Value = serde_json::from_slice(&with_hash).expect("parse with_hash");
    value["bundleVersion"] = serde_json::json!(99);
    let tampered = serde_json::to_vec(&value).expect("re-serialize tampered");
    write_private(&bundle_path, &tampered);
    set_mode_to(&bundle_path, current_user_policy().required_mode);

    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("corrupted file should be rejected");
    match &err {
        Error::Bundle(BundleError::Tampered { reason, .. }) if reason == "hash" => {}
        Error::Manifest(_) => {} // parse failure on truly corrupted JSON is also acceptable
        other => panic!("expected hash tamper or parse error, got {other:?}"),
    }
}

// ---------------------------------------------------------------
// Test 4b: binary-garbage corruption → hash mismatch or parse error
// ---------------------------------------------------------------
#[test]
fn tamper_truncated() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");

    let pre_hash = minimal_bundle_json_no_hash();
    let with_hash = bundle_json_with_hash(&pre_hash);

    // Truncate to first 10 bytes - definitely unparseable JSON.
    write_private(&bundle_path, &with_hash[..10]);
    set_mode_to(&bundle_path, current_user_policy().required_mode);

    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("truncated file should be rejected");
    match &err {
        Error::Bundle(BundleError::Tampered { reason, .. }) if reason == "hash" => {}
        Error::Manifest(_) => {} // parse failure is also acceptable
        other => panic!("expected tamper or parse error on truncated file, got {other:?}"),
    }
}

// ---------------------------------------------------------------
// Test 5: all-correct file → loads successfully
// ---------------------------------------------------------------
#[test]
fn loads_correct() {
    let dir = TempDir::new().expect("tempdir");
    let policy = current_user_policy();

    // Build bundle.json with correct self-hash.
    let pre_hash = minimal_bundle_json_no_hash();
    let with_hash = bundle_json_with_hash(&pre_hash);

    let bundle_path = dir.path().join("bundle.json");
    write_private(&bundle_path, &with_hash);
    set_mode_to(&bundle_path, policy.required_mode);

    let resolver = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect("all-correct bundle should load without error");

    assert_eq!(resolver.bundle.bundle_version, 1);
    assert_eq!(resolver.bundle.schema_version, "v3");
}

// ---------------------------------------------------------------
// Test 6: missing bundleHash → BundleTampered { reason: "missing-bundle-hash" }
// ---------------------------------------------------------------
#[test]
fn tamper_missing_bundle_hash() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");

    // A bundle without bundleHash must be rejected outright.
    write_private(&bundle_path, &minimal_bundle_json_no_hash());
    set_mode_to(&bundle_path, current_user_policy().required_mode);

    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("bundle without bundleHash should be rejected");
    assert_tampered(&err, "missing-bundle-hash");
}

// ---------------------------------------------------------------
// Test 7: schemaVersion v2 → manifest-version-mismatch (v2 removed)
// ---------------------------------------------------------------
#[test]
fn v2_bundle_rejects_with_manifest_version_mismatch() {
    let dir = TempDir::new().expect("tempdir");
    let bundle_path = dir.path().join("bundle.json");

    // The tamper-resistance hash check runs before the schema check, so the
    // v2 fixture must carry a valid self-hash to reach the version gate.
    let mut value = serde_json::json!({
        "bundleVersion": 1,
        "schemaVersion": "v2",
        "privilegesPath": "privileges.json",
        "zones": [],
        "artifactHashes": {},
        "generation": {
            "generator": "test",
            "sourceRevision": null,
            "generatedAt": null
        }
    });
    let hash = self_hash(&value);
    value
        .as_object_mut()
        .expect("bundle is object")
        .insert("bundleHash".to_owned(), serde_json::Value::String(hash));

    write_private(
        &bundle_path,
        &serde_json::to_vec(&value).expect("v2 json serializes"),
    );
    set_mode_to(&bundle_path, current_user_policy().required_mode);

    let policy = current_user_policy();
    let err = BundleResolver::load_with_policy(&bundle_path, &policy)
        .expect_err("schemaVersion v2 must be rejected by the v3-only loader");
    let message = err.message();
    assert!(
        message.contains("manifest-version-mismatch"),
        "expected manifest-version-mismatch, got {message:?}"
    );
}
