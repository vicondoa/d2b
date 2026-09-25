//! Public Volume status stays free of paths, policy ids, and numeric identity.

use d2b_core::test_support::block_on;
use d2b_provider_volume_local::testing::{ScriptedPort, fixtures};
use d2b_provider_volume_local::{VolumeLocalController, VolumeLocalProfile};

/// Fragments that must never appear in a public Volume status document.
const FORBIDDEN_STATUS_FRAGMENTS: [&str; 19] = [
    "pid",
    "pidfd",
    "unit",
    "invocation",
    "cgroup",
    "path",
    "argv",
    "command",
    "binary",
    "env",
    "sourcepolicyid",
    "state-root",
    "uid",
    "gid",
    "socket",
    "acl",
    "marker",
    "sync.lock",
    "gcroots",
];

#[test]
fn public_status_carries_no_path_policy_id_or_numeric_identity() {
    let port = ScriptedPort::empty();
    let controller = VolumeLocalController::new(VolumeLocalProfile::shipped(), &port, &port);
    let report =
        block_on(controller.reconcile(
            &fixtures::volume_uid(),
            &fixtures::store_view_volume(),
            None,
            None,
        ))
        .expect("reconcile succeeds");
    let rendered = serde_json::to_string(&report)
        .expect("status serializes")
        .to_ascii_lowercase();
    for fragment in FORBIDDEN_STATUS_FRAGMENTS {
        assert!(
            !rendered.contains(fragment),
            "public status carries the forbidden fragment {fragment}"
        );
    }
    assert!(rendered.contains("volume-local"));
}
