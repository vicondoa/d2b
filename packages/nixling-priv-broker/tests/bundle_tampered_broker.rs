//! Integration test: tampered bundle → BrokerResponse::Error { kind: "bundle-tampered" }.
//!
//! Verifies that when the broker's `try_load_resolver` path encounters a
//! bundle that fails its tamper-resistance check, the resulting
//! `BrokerResponse` carries `kind: "bundle-tampered"` (not
//! `"Broker.BundleResolverUnavailable"` or any other kind).
//!
//! The test uses the public `probe_bundle_load_response` helper exported
//! from `nixling_priv_broker::runtime`, which exercises the same
//! `try_load_resolver` → `BundleSlot::Tampered` → `BrokerError::BundleTampered`
//! → `into_response()` pipeline as the live `serve` loop.

#[cfg(not(feature = "layer1-bootstrap"))]
mod broker_tampered {
    use nixling_core::bundle_resolver::BundleVerifyPolicy;
    use nixling_ipc::broker_wire::BrokerResponse;
    use nixling_priv_broker::runtime::{
        probe_bundle_load_response, probe_bundle_load_response_with_policy,
    };
    use std::fs;
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;
    use tempfile::TempDir;

    fn current_user_policy() -> BundleVerifyPolicy {
        BundleVerifyPolicy {
            required_uid: rustix::process::getuid().as_raw(),
            required_gid: Some(rustix::process::getgid().as_raw()),
            required_mode: 0o640,
        }
    }

    /// Minimal bundle JSON bytes (no bundleHash — mode check fires first).
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
                "keysDir": "/var/lib/nixling/keys",
                "knownHostsPath": "/var/lib/nixling/known_hosts.nixling",
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

    // ---------------------------------------------------------------
    // Helper: create a bundle dir with bundle.json at mode 0o644 (too
    // permissive — requires 0o640).
    // ---------------------------------------------------------------

    fn make_tampered_bundle_dir() -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        let bundle_path = dir.path().join("bundle.json");
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o644) // too permissive → Tampered { reason: "mode" }
            .open(&bundle_path)
            .expect("create bundle.json")
            .write_all(&minimal_bundle_json())
            .expect("write bundle.json");
        dir
    }

    // ---------------------------------------------------------------
    // Test 1: tampered mode → BrokerResponse::Error { kind: "bundle-tampered" }
    //
    // Uses current_user_policy() so the uid check passes (no root required)
    // and only the mode check fires, producing reason: "mode".
    // ---------------------------------------------------------------

    #[test]
    fn tampered_bundle_returns_bundle_tampered_response() {
        let dir = make_tampered_bundle_dir();
        let bundle_path = dir.path().join("bundle.json");

        // Use current_user_policy (required_mode=0640) so uid matches and
        // only the mode mismatch (0644 ≠ 0640) fires.
        let response = probe_bundle_load_response_with_policy(&bundle_path, &current_user_policy());

        match response {
            BrokerResponse::Error(err) => {
                assert_eq!(
                    err.kind, "bundle-tampered",
                    "expected kind=bundle-tampered, got {:?}",
                    err.kind
                );
                assert!(
                    err.message.contains("integrity checks"),
                    "message should be fail-secure integrity wording; got: {:?}",
                    err.message
                );
                assert!(
                    !err.message.contains("mode"),
                    "message must not leak the raw tamper reason; got: {:?}",
                    err.message
                );
                assert!(
                    err.action.contains("nixos-rebuild switch"),
                    "action should contain remediation guidance; got: {:?}",
                    err.action
                );
            }
            other => panic!("expected BrokerResponse::Error, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // Test 2: absent bundle → BrokerResponse::Error { kind: "Broker.BundleResolverUnavailable" }
    //
    // Confirms the tamper path is distinct from the unavailable path.
    // ---------------------------------------------------------------

    #[test]
    fn absent_bundle_returns_resolver_unavailable_response() {
        let dir = TempDir::new().expect("tempdir");
        let bundle_path = dir.path().join("nonexistent-bundle.json");

        let response = probe_bundle_load_response(&bundle_path);

        match response {
            BrokerResponse::Error(err) => {
                assert_ne!(
                    err.kind, "bundle-tampered",
                    "absent bundle should not produce bundle-tampered; got {:?}",
                    err.kind
                );
            }
            other => panic!("expected BrokerResponse::Error for absent bundle, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // Test 3: symlink at bundle path → bundle-tampered (not unavailable)
    // ---------------------------------------------------------------

    #[test]
    fn symlink_bundle_returns_bundle_tampered_response() {
        let dir = TempDir::new().expect("tempdir");
        let real_path = dir.path().join("real.json");
        let bundle_path = dir.path().join("bundle.json");

        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o640)
            .open(&real_path)
            .expect("create real.json")
            .write_all(&minimal_bundle_json())
            .expect("write real.json");
        std::os::unix::fs::symlink(&real_path, &bundle_path).expect("create symlink");

        let response = probe_bundle_load_response(&bundle_path);

        match response {
            BrokerResponse::Error(err) => {
                assert_eq!(
                    err.kind, "bundle-tampered",
                    "symlink bundle should produce bundle-tampered; got {:?}",
                    err.kind
                );
                assert!(
                    err.message.contains("integrity checks"),
                    "message should be fail-secure integrity wording; got: {:?}",
                    err.message
                );
                assert!(
                    !err.message.contains("symlink"),
                    "message must not leak the raw tamper reason; got: {:?}",
                    err.message
                );
            }
            other => panic!("expected BrokerResponse::Error for symlink bundle, got {other:?}"),
        }
    }
}
