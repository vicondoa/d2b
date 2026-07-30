//! Named views, right intersection, and sharing admission.

use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_contracts::v3::volume::{AttachmentAccess, VolumeSpec};
use d2b_provider_volume_local::testing::fixtures;
use d2b_provider_volume_local::{VolumeLocalError, admit_access, admit_attachments, resolve_view};

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("valid fixture token")
}

fn spec_with_attachments(attachments: serde_json::Value) -> VolumeSpec {
    serde_json::from_value(serde_json::json!({
        "source": {
            "executionRef": "Host/host-system",
            "settings": { "kind": "local-path", "sourcePolicyId": "state-root" },
        },
        "kind": "durable",
        "layout": [],
        "views": {
            "controller": { "path": "", "rights": ["read", "write", "traverse"] },
            "reader": { "path": "data", "rights": ["read", "traverse"] },
        },
        "attachments": attachments,
    }))
    .expect("conformant fixture Volume spec")
}

fn attachment(execution_ref: &str, view: &str, access: &str) -> serde_json::Value {
    serde_json::json!({
        "executionRef": execution_ref,
        "transport": "virtiofs",
        "view": view,
        "access": access,
        "mountPath": "/state",
    })
}

#[test]
fn a_view_that_is_not_declared_is_rejected() {
    let spec = fixtures::state_volume();
    assert_eq!(
        resolve_view(&spec, &token("absent")).unwrap_err(),
        VolumeLocalError::ViewNotFound
    );
    assert!(resolve_view(&spec, &token("controller")).is_ok());
}

#[test]
fn write_access_requires_the_view_to_grant_the_write_right() {
    let spec = spec_with_attachments(serde_json::json!([]));
    let reader = resolve_view(&spec, &token("reader")).expect("declared view");
    assert!(admit_access(reader, AttachmentAccess::ReadOnly).is_ok());
    assert_eq!(
        admit_access(reader, AttachmentAccess::ReadWrite).unwrap_err(),
        VolumeLocalError::ViewRightsInsufficient
    );
    let controller = resolve_view(&spec, &token("controller")).expect("declared view");
    assert!(admit_access(controller, AttachmentAccess::ReadWrite).is_ok());
}

#[test]
fn many_readers_share_one_volume() {
    let spec = spec_with_attachments(serde_json::json!([
        attachment("Guest/work-vm", "reader", "read-only"),
        attachment("Guest/personal-vm", "reader", "read-only"),
        attachment("Host/host-system", "controller", "read-write"),
    ]));
    let plans = admit_attachments(&spec, false).expect("admitted");
    assert_eq!(plans.len(), 3);
    assert_eq!(
        plans
            .iter()
            .filter(|plan| plan.access == AttachmentAccess::ReadWrite)
            .count(),
        1
    );
}

#[test]
fn a_second_simultaneous_writer_is_rejected() {
    let spec = spec_with_attachments(serde_json::json!([attachment(
        "Guest/work-vm",
        "controller",
        "read-write"
    ),]));
    assert!(admit_attachments(&spec, false).is_ok());

    // The base contract rejects two `read-write` attachments outright, so
    // the second-writer case reaches the Provider only as `shared-write`.
    let shared = spec_with_attachments(serde_json::json!([
        attachment("Guest/work-vm", "controller", "read-write"),
        attachment("Guest/personal-vm", "controller", "shared-write"),
    ]));
    assert_eq!(
        admit_attachments(&shared, false).unwrap_err(),
        VolumeLocalError::SharedWriteUnsupported
    );
    assert!(admit_attachments(&shared, true).is_ok());
}

#[test]
fn the_shipped_provider_does_not_declare_shared_write() {
    use d2b_provider_volume_local::VolumeLocalProfile;
    assert!(!VolumeLocalProfile::shipped().supports_shared_write());
}

#[test]
fn every_admitted_attachment_keeps_its_typed_reference_and_view() {
    let spec = fixtures::attached_state_volume();
    let plans = admit_attachments(&spec, false).expect("admitted");
    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0].execution_ref.to_canonical_string(),
        "Guest/work-vm"
    );
    assert_eq!(plans[0].view.as_str(), "controller");
    assert_eq!(plans[0].access, AttachmentAccess::ReadWrite);
}
