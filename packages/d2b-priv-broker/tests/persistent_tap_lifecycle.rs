use std::cell::Cell;

use d2b_contracts::{
    broker_wire::{CreatePersistentTapRequest, DeletePersistentTapRequest},
    types::{RoleId, VmId},
    v3::{ResourceGeneration, ResourceUid},
};
use d2b_priv_broker::ops::{
    audit_op::OperationFields,
    network::{
        NetworkOpError, PersistentTapBackend, PersistentTapRealization, attachment_digest,
        delete_persistent_tap,
    },
};

struct FakeTap {
    present: Cell<bool>,
    deletes: Cell<usize>,
}

impl PersistentTapBackend for FakeTap {
    fn tap_exists(&self, _: &str) -> Result<bool, NetworkOpError> {
        Ok(self.present.get())
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

fn request(network_generation: u64, attachment_generation: u64) -> DeletePersistentTapRequest {
    DeletePersistentTapRequest {
        attachment_id: attachment_id(),
        expected_network_generation: ResourceGeneration::new(network_generation).unwrap(),
        expected_attachment_generation: ResourceGeneration::new(attachment_generation).unwrap(),
        tracing_span_id: None,
    }
}

fn realization() -> PersistentTapRealization {
    PersistentTapRealization {
        attachment_id: attachment_id().as_str().to_owned(),
        network_generation: 4,
        attachment_generation: 7,
        ifname: "d2b-t12345678".to_owned(),
        ownership_marker: format!("d2b managed: attachment:{}", attachment_id().as_str()),
    }
}

#[test]
fn delete_persistent_tap_pairs_with_create() {
    let create = CreatePersistentTapRequest {
        role_id: RoleId::new("vmm"),
        vm_id: VmId::new("work-vm"),
        tracing_span_id: None,
    };
    let delete = request(4, 7);
    let create_json = serde_json::to_value(create).unwrap();
    let delete_json = serde_json::to_value(delete).unwrap();
    assert_eq!(create_json.as_object().unwrap().len(), 3);
    assert_eq!(delete_json.as_object().unwrap().len(), 4);
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
    };
    assert_eq!(
        delete_persistent_tap(&backend, &realization(), &request(4, 6)),
        Err(NetworkOpError::StaleAttachmentGeneration)
    );
    assert_eq!(backend.deletes.get(), 0);
}

#[test]
fn delete_persistent_tap_foreign_marker_fails_closed() {
    let backend = FakeTap {
        present: Cell::new(true),
        deletes: Cell::new(0),
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
