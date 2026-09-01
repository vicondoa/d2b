use std::cell::Cell;

use d2b_broker::ops::{
    audit_op::OperationFields,
    network::{
        NetworkOpError, PersistentTapBackend, PersistentTapRealization, attachment_digest,
        delete_persistent_tap, load_persistent_tap_realization, persist_persistent_tap_realization,
    },
};
use d2b_contracts::types::{BundleOpId, RoleId, VmId};
use d2b_contracts_broker::broker_wire::{CreatePersistentTapRequest, DeletePersistentTapRequest};
use d2b_contracts_resource::v3::{
    NetworkIfRole, ResourceBundleGenerationId, ResourceGeneration, ResourceUid,
    derive_network_ifname,
};

struct FakeTap {
    present: Cell<bool>,
    deletes: Cell<usize>,
    foreign_marker: bool,
}

impl PersistentTapBackend for FakeTap {
    fn tap_exists(&self, _: &str) -> Result<bool, NetworkOpError> {
        Ok(self.present.get())
    }

    fn tap_ownership_marker(&self, _: &str) -> Result<Option<String>, NetworkOpError> {
        Ok(self.present.get().then(|| {
            if self.foreign_marker {
                "d2b managed: foreign".to_owned()
            } else {
                realization().ownership_marker
            }
        }))
    }

    fn delete_tap(&self, _: &str) -> Result<(), NetworkOpError> {
        self.deletes.set(self.deletes.get() + 1);
        self.present.set(false);
        Ok(())
    }
}

fn attachment_id() -> ResourceUid {
    ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap()
}

fn zone_uid() -> ResourceUid {
    ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap()
}

fn network_uid() -> ResourceUid {
    ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap()
}

fn bundle_generation() -> ResourceBundleGenerationId {
    ResourceBundleGenerationId::parse(
        "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .unwrap()
}

fn tap_intent_ref() -> BundleOpId {
    BundleOpId::new(d2b_core::bundle_resolver::intent_id_network_tap(
        &zone_uid(),
        &network_uid(),
        &attachment_id(),
        ResourceGeneration::new(4).unwrap(),
        ResourceGeneration::new(7).unwrap(),
        &bundle_generation(),
        "network-attachment",
        "work-vm",
    ))
}

fn tap_ifname() -> String {
    derive_network_ifname(
        &zone_uid(),
        &network_uid(),
        NetworkIfRole::WorkloadGuestTap,
        Some(&attachment_id()),
    )
    .unwrap()
    .as_str()
    .to_owned()
}

fn request(network_generation: u64, attachment_generation: u64) -> DeletePersistentTapRequest {
    DeletePersistentTapRequest {
        attachment_id: attachment_id(),
        expected_zone_uid: zone_uid(),
        expected_network_uid: network_uid(),
        expected_network_generation: ResourceGeneration::new(network_generation).unwrap(),
        expected_attachment_generation: ResourceGeneration::new(attachment_generation).unwrap(),
        expected_bundle_generation: bundle_generation(),
        tracing_span_id: None,
    }
}

fn realization() -> PersistentTapRealization {
    PersistentTapRealization {
        attachment_id: attachment_id().as_str().to_owned(),
        zone_uid: zone_uid().as_str().to_owned(),
        network_uid: network_uid().as_str().to_owned(),
        network_generation: 4,
        attachment_generation: 7,
        bundle_generation: bundle_generation().as_str().to_owned(),
        ifname: tap_ifname(),
        ownership_marker: format!(
            "d2b managed: network:tap:{}:zone:{}:network:{}:generation:4:attachment:7:bundle:{}",
            attachment_id().as_str(),
            zone_uid().as_str(),
            network_uid().as_str(),
            bundle_generation().as_str(),
        ),
        deleted: false,
    }
}

fn create_request() -> CreatePersistentTapRequest {
    CreatePersistentTapRequest {
        role_id: RoleId::new("network-attachment"),
        vm_id: VmId::new("work-vm"),
        bundle_tap_intent_ref: tap_intent_ref(),
        attachment_id: attachment_id(),
        network_generation: ResourceGeneration::new(4).unwrap(),
        attachment_generation: ResourceGeneration::new(7).unwrap(),
        zone_uid: zone_uid(),
        network_uid: network_uid(),
        bundle_generation: bundle_generation(),
        admitted_interface_names: Vec::new(),
        tracing_span_id: None,
    }
}

fn state_dir(test_name: &str) -> std::path::PathBuf {
    let path = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("{test_name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn failed_create_leaves_no_realization_and_retry_is_safe() {
    let root = state_dir("persistent-tap-lifecycle");
    let create = create_request();
    let delete = request(4, 7);
    let ifname = d2b_contracts_resource::v3::IfName::new(tap_ifname()).unwrap();

    // A failed live create has no post-create persistence callback. The
    // realization row must therefore remain absent and a retry must be able
    // to persist the successful outcome normally.
    assert_eq!(
        load_persistent_tap_realization(&root, &delete),
        Err(NetworkOpError::RealizationUnavailable)
    );
    assert!(
        !root
            .join("network-attachments")
            .join(format!("{}.json", attachment_id().as_str()))
            .exists()
    );

    persist_persistent_tap_realization(&root, &create, &ifname).unwrap();
    let loaded = load_persistent_tap_realization(&root, &delete).unwrap();
    assert_eq!(loaded.ifname, ifname.as_str());

    // Replaying the successful post-create persistence is idempotent, so a
    // retry after a lost response does not create a conflicting row.
    persist_persistent_tap_realization(&root, &create, &ifname).unwrap();
    assert_eq!(
        load_persistent_tap_realization(&root, &delete).unwrap(),
        loaded
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn persistence_rejects_a_swapped_tap_identity_before_writing() {
    let root = state_dir("persistent-tap-identity");
    let create = create_request();
    let ifname = d2b_contracts_resource::v3::IfName::new(tap_ifname()).unwrap();
    let swapped = CreatePersistentTapRequest {
        attachment_generation: ResourceGeneration::new(8).unwrap(),
        ..create
    };
    assert_eq!(
        persist_persistent_tap_realization(&root, &swapped, &ifname),
        Err(NetworkOpError::RealizationConflict)
    );
    assert!(
        !root
            .join("network-attachments")
            .join(format!("{}.json", attachment_id().as_str()))
            .exists()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn delete_persistent_tap_pairs_with_create() {
    let create = create_request();
    let delete = request(4, 7);
    let create_json = serde_json::to_value(create).unwrap();
    let delete_json = serde_json::to_value(delete).unwrap();
    assert!(create_json.as_object().unwrap().len() >= 8);
    assert_eq!(delete_json.as_object().unwrap().len(), 7);
    for forbidden in ["ifname", "path", "ownershipMarker"] {
        assert!(!create_json.as_object().unwrap().contains_key(forbidden));
        assert!(!delete_json.as_object().unwrap().contains_key(forbidden));
    }
}

#[test]
fn delete_persistent_tap_absent_is_idempotent_after_ownership_validation() {
    let backend = FakeTap {
        present: Cell::new(false),
        deletes: Cell::new(0),
        foreign_marker: false,
    };
    assert_eq!(
        delete_persistent_tap(&backend, &realization(), &request(4, 7)).unwrap(),
        attachment_digest(attachment_id().as_str())
    );
    assert_eq!(backend.deletes.get(), 0);
}

#[test]
fn delete_persistent_tap_rejects_stale_network_generation() {
    let backend = FakeTap {
        present: Cell::new(true),
        deletes: Cell::new(0),
        foreign_marker: false,
    };
    assert_eq!(
        delete_persistent_tap(&backend, &realization(), &request(3, 7)),
        Err(NetworkOpError::StaleNetworkGeneration)
    );
    assert_eq!(backend.deletes.get(), 0);
}

#[test]
fn delete_persistent_tap_rejects_stale_attachment_generation() {
    let backend = FakeTap {
        present: Cell::new(true),
        deletes: Cell::new(0),
        foreign_marker: false,
    };
    assert_eq!(
        delete_persistent_tap(&backend, &realization(), &request(4, 6)),
        Err(NetworkOpError::StaleAttachmentGeneration)
    );
    assert_eq!(backend.deletes.get(), 0);
}

#[test]
fn delete_persistent_tap_rejects_swapped_network_identity() {
    let backend = FakeTap {
        present: Cell::new(true),
        deletes: Cell::new(0),
        foreign_marker: false,
    };
    let mut request = request(4, 7);
    request.expected_network_uid =
        ResourceUid::parse("423e4567-e89b-42d3-a456-426614174003").unwrap();
    assert_eq!(
        delete_persistent_tap(&backend, &realization(), &request),
        Err(NetworkOpError::ForeignOwnership)
    );
    assert_eq!(backend.deletes.get(), 0);
    assert!(backend.present.get());
}

#[test]
fn delete_persistent_tap_foreign_marker_fails_closed() {
    let backend = FakeTap {
        present: Cell::new(true),
        deletes: Cell::new(0),
        foreign_marker: false,
    };
    let mut foreign = realization();
    foreign.ownership_marker = "foreign marker".to_owned();
    assert_eq!(
        delete_persistent_tap(&backend, &foreign, &request(4, 7)),
        Err(NetworkOpError::ForeignOwnership)
    );
    assert_eq!(backend.deletes.get(), 0);
    assert!(backend.present.get());
}

#[test]
fn delete_persistent_tap_refuses_unmarked_kernel_tap() {
    let backend = FakeTap {
        present: Cell::new(true),
        deletes: Cell::new(0),
        foreign_marker: true,
    };
    assert_eq!(
        delete_persistent_tap(&backend, &realization(), &request(4, 7)),
        Err(NetworkOpError::ForeignOwnership)
    );
    assert_eq!(backend.deletes.get(), 0);
    assert!(backend.present.get());
}

#[test]
fn delete_persistent_tap_request_and_audit_have_no_ifname_or_path() {
    let request_json = serde_json::to_string(&request(4, 7)).unwrap();
    let audit_json = serde_json::to_string(&OperationFields::DeletePersistentTap {
        attachment_digest: attachment_digest(attachment_id().as_str()),
        expected_network_generation: ResourceGeneration::new(4).unwrap(),
        expected_attachment_generation: ResourceGeneration::new(7).unwrap(),
    })
    .unwrap();
    for forbidden in [
        "d2b-t12345678",
        "/sys/class/net",
        "/dev/net/tun",
        "ownership_marker",
    ] {
        assert!(!request_json.contains(forbidden));
        assert!(!audit_json.contains(forbidden));
    }
}
