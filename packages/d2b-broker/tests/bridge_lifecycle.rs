use std::cell::RefCell;

use d2b_contracts_resource::v3::{IfName, NetworkProvenance, ResourceBundleGenerationId, ResourceGeneration, ResourceUid};
use d2b_core::bundle_resolver::ResolvedBridgeIntent;
use d2b_broker::ops::network::{
    BridgeBackend, BridgeReadback, NetworkOpError, bridge_intent_digest, create_bridge,
    delete_bridge,
};

struct FakeBridge {
    state: RefCell<BridgeReadback>,
    events: RefCell<Vec<&'static str>>,
}

impl FakeBridge {
    fn absent() -> Self {
        Self {
            state: RefCell::new(BridgeReadback {
                present: false,
                is_bridge: false,
                mtu: 0,
                stp_disabled: false,
                multicast_snooping_disabled: false,
                ipv6_suppressed: false,
                attached_links: 0,
                ownership_marker: None,
            }),
            events: RefCell::new(Vec::new()),
        }
    }
}

impl BridgeBackend for FakeBridge {
    fn read_bridge(&self, _: &ResolvedBridgeIntent) -> Result<BridgeReadback, NetworkOpError> {
        self.events.borrow_mut().push("read");
        Ok(self.state.borrow().clone())
    }

    fn create_bridge_down(&self, _: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
        self.events.borrow_mut().push("create-down");
        Ok(())
    }

    fn configure_bridge(&self, intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
        self.events.borrow_mut().push("configure-ipv6-off");
        self.state.replace(BridgeReadback {
            present: true,
            is_bridge: true,
            mtu: intent.mtu,
            stp_disabled: intent.stp_disabled,
            multicast_snooping_disabled: intent.multicast_snooping_disabled,
            ipv6_suppressed: intent.ipv6_suppressed,
            attached_links: 0,
            ownership_marker: Some(expected_marker()),
        });
        Ok(())
    }

    fn set_bridge_up(&self, _: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
        self.events.borrow_mut().push("link-up");
        assert!(self.state.borrow().ipv6_suppressed);
        Ok(())
    }

    fn delete_bridge(&self, _: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
        self.events.borrow_mut().push("delete");
        self.state.replace(BridgeReadback {
            present: false,
            is_bridge: false,
            mtu: 0,
            stp_disabled: false,
            multicast_snooping_disabled: false,
            ipv6_suppressed: false,
            attached_links: 0,
            ownership_marker: None,
        });
        Ok(())
    }
}

fn intent() -> ResolvedBridgeIntent {
    ResolvedBridgeIntent {
        intent_id: "network-bridge:223e4567-e89b-42d3-a456-426614174001:323e4567-e89b-42d3-a456-426614174002:aaaaaaaaaaaaaaaa:lan".to_owned(),
        scope_label: "scope".to_owned(),
        bridge_ifname: IfName::new("d2b-b12345678").unwrap(),
        mtu: 1280,
        stp_disabled: true,
        multicast_snooping_disabled: true,
        ipv6_suppressed: true,
        provenance: Some(provenance()),
        ownership_marker: Some(expected_marker()),
    }
}

fn provenance() -> NetworkProvenance {
    NetworkProvenance::new(
        ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
        ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap(),
        ResourceGeneration::new(4).unwrap(),
        ResourceGeneration::new(7).unwrap(),
        ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap(),
    )
}

fn expected_marker() -> String {
    format!(
        "d2b managed: {}",
        d2b_contracts_resource::v3::derive_network_ownership_marker(
            &provenance(),
            "bridge:lan",
        )
    )
}

#[test]
fn create_bridge_applies_ipv6_sysctl() {
    let backend = FakeBridge::absent();
    let expected = intent();
    assert_eq!(
        create_bridge(&backend, &expected).unwrap(),
        bridge_intent_digest(&expected)
    );
    assert_eq!(
        *backend.events.borrow(),
        [
            "read",
            "create-down",
            "configure-ipv6-off",
            "link-up",
            "read"
        ]
    );
    assert!(backend.state.borrow().ipv6_suppressed);
}

#[test]
fn create_bridge_parameters_match_spec() {
    let backend = FakeBridge::absent();
    create_bridge(&backend, &intent()).unwrap();
    let observed = backend.state.borrow().clone();
    assert_eq!(observed.mtu, 1280);
    assert!(observed.stp_disabled);
    assert!(observed.multicast_snooping_disabled);
    assert!(observed.ipv6_suppressed);
}

#[test]
fn delete_bridge_is_idempotent() {
    let backend = FakeBridge::absent();
    let expected = intent();
    assert_eq!(
        delete_bridge(&backend, &expected).unwrap(),
        bridge_intent_digest(&expected)
    );
    assert_eq!(*backend.events.borrow(), ["read"]);
}

#[test]
fn delete_bridge_never_cascades_attached_tap() {
    let backend = FakeBridge::absent();
    backend.state.replace(BridgeReadback {
        present: true,
        is_bridge: true,
        mtu: 1280,
        stp_disabled: true,
        multicast_snooping_disabled: true,
        ipv6_suppressed: true,
        attached_links: 1,
        ownership_marker: Some(expected_marker()),
    });
    assert_eq!(
        delete_bridge(&backend, &intent()),
        Err(NetworkOpError::BridgeNotEmpty)
    );
    assert_eq!(*backend.events.borrow(), ["read"]);
    assert_eq!(backend.state.borrow().attached_links, 1);
}

#[test]
fn bridge_name_prefix_is_not_ownership_proof() {
    let backend = FakeBridge::absent();
    let mut trusted = intent();
    trusted.bridge_ifname = IfName::new("br-foreign").unwrap();
    assert_eq!(
        create_bridge(&backend, &trusted).unwrap(),
        bridge_intent_digest(&trusted)
    );
    assert_eq!(
        *backend.events.borrow(),
        [
            "read",
            "create-down",
            "configure-ipv6-off",
            "link-up",
            "read"
        ]
    );
}

#[test]
fn unmarked_existing_bridge_is_foreign_and_unchanged() {
    let backend = FakeBridge::absent();
    backend.state.replace(BridgeReadback {
        present: true,
        is_bridge: true,
        mtu: 1280,
        stp_disabled: true,
        multicast_snooping_disabled: true,
        ipv6_suppressed: true,
        attached_links: 0,
        ownership_marker: None,
    });
    let before = backend.state.borrow().clone();
    assert_eq!(
        create_bridge(&backend, &intent()),
        Err(NetworkOpError::ForeignOwnership)
    );
    assert_eq!(*backend.state.borrow(), before);
    assert_eq!(*backend.events.borrow(), ["read"]);
}

#[test]
fn matching_bridge_marker_allows_adoption_without_mutation() {
    let backend = FakeBridge::absent();
    backend.state.replace(BridgeReadback {
        present: true,
        is_bridge: true,
        mtu: 1280,
        stp_disabled: true,
        multicast_snooping_disabled: true,
        ipv6_suppressed: true,
        attached_links: 0,
        ownership_marker: Some(expected_marker()),
    });
    let before = backend.state.borrow().clone();
    assert_eq!(
        create_bridge(&backend, &intent()).unwrap(),
        bridge_intent_digest(&intent())
    );
    assert_eq!(*backend.state.borrow(), before);
    assert_eq!(*backend.events.borrow(), ["read"]);
}

#[test]
fn matching_bridge_marker_with_parameter_drift_is_unchanged() {
    let backend = FakeBridge::absent();
    backend.state.replace(BridgeReadback {
        present: true,
        is_bridge: true,
        mtu: 1400,
        stp_disabled: true,
        multicast_snooping_disabled: true,
        ipv6_suppressed: true,
        attached_links: 0,
        ownership_marker: Some(expected_marker()),
    });
    let before = backend.state.borrow().clone();
    assert_eq!(
        create_bridge(&backend, &intent()),
        Err(NetworkOpError::BridgeParameterMismatch)
    );
    assert_eq!(*backend.state.borrow(), before);
    assert_eq!(*backend.events.borrow(), ["read"]);
}

#[test]
fn mismatched_existing_bridge_refuses_create_without_mutation() {
    let backend = FakeBridge::absent();
    backend.state.replace(BridgeReadback {
        present: true,
        is_bridge: true,
        mtu: 1280,
        stp_disabled: true,
        multicast_snooping_disabled: true,
        ipv6_suppressed: true,
        attached_links: 0,
        ownership_marker: Some("d2b managed: foreign".to_owned()),
    });
    let before = backend.state.borrow().clone();
    assert_eq!(
        create_bridge(&backend, &intent()),
        Err(NetworkOpError::ForeignOwnership)
    );
    assert_eq!(*backend.state.borrow(), before);
    assert_eq!(*backend.events.borrow(), ["read"]);
}

#[test]
fn mismatched_existing_bridge_refuses_delete_without_mutation() {
    let backend = FakeBridge::absent();
    backend.state.replace(BridgeReadback {
        present: true,
        is_bridge: true,
        mtu: 1280,
        stp_disabled: true,
        multicast_snooping_disabled: true,
        ipv6_suppressed: true,
        attached_links: 0,
        ownership_marker: Some("d2b managed: foreign".to_owned()),
    });
    let before = backend.state.borrow().clone();
    assert_eq!(
        delete_bridge(&backend, &intent()),
        Err(NetworkOpError::ForeignOwnership)
    );
    assert_eq!(*backend.state.borrow(), before);
}
