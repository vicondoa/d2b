use std::cell::{Cell, RefCell};

use d2b_core::{bundle_resolver::ResolvedBridgeIntent, host::IfName};
use d2b_priv_broker::ops::network::{
    BridgeBackend, BridgeReadback, NetworkOpError, bridge_intent_digest, create_bridge,
    delete_bridge,
};

struct FakeBridge {
    state: Cell<BridgeReadback>,
    events: RefCell<Vec<&'static str>>,
}

impl FakeBridge {
    fn absent() -> Self {
        Self {
            state: Cell::new(BridgeReadback {
                present: false,
                is_bridge: false,
                mtu: 0,
                stp_disabled: false,
                multicast_snooping_disabled: false,
                ipv6_suppressed: false,
                attached_links: 0,
            }),
            events: RefCell::new(Vec::new()),
        }
    }
}

impl BridgeBackend for FakeBridge {
    fn read_bridge(&self, _: &ResolvedBridgeIntent) -> Result<BridgeReadback, NetworkOpError> {
        self.events.borrow_mut().push("read");
        Ok(self.state.get())
    }

    fn create_bridge_down(&self, _: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
        self.events.borrow_mut().push("create-down");
        Ok(())
    }

    fn configure_bridge(&self, intent: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
        self.events.borrow_mut().push("configure-ipv6-off");
        self.state.set(BridgeReadback {
            present: true,
            is_bridge: true,
            mtu: intent.mtu,
            stp_disabled: intent.stp_disabled,
            multicast_snooping_disabled: intent.multicast_snooping_disabled,
            ipv6_suppressed: intent.ipv6_suppressed,
            attached_links: 0,
        });
        Ok(())
    }

    fn set_bridge_up(&self, _: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
        self.events.borrow_mut().push("link-up");
        assert!(self.state.get().ipv6_suppressed);
        Ok(())
    }

    fn delete_bridge(&self, _: &ResolvedBridgeIntent) -> Result<(), NetworkOpError> {
        self.events.borrow_mut().push("delete");
        self.state.set(BridgeReadback {
            present: false,
            is_bridge: false,
            mtu: 0,
            stp_disabled: false,
            multicast_snooping_disabled: false,
            ipv6_suppressed: false,
            attached_links: 0,
        });
        Ok(())
    }
}

fn intent() -> ResolvedBridgeIntent {
    ResolvedBridgeIntent {
        intent_id: "bridge-opaque".to_owned(),
        scope_label: "scope".to_owned(),
        bridge_ifname: IfName::new("d2b-b12345678").unwrap(),
        mtu: 1280,
        stp_disabled: true,
        multicast_snooping_disabled: true,
        ipv6_suppressed: true,
    }
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
    assert!(backend.state.get().ipv6_suppressed);
}

#[test]
fn create_bridge_parameters_match_spec() {
    let backend = FakeBridge::absent();
    create_bridge(&backend, &intent()).unwrap();
    let observed = backend.state.get();
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
    backend.state.set(BridgeReadback {
        present: true,
        is_bridge: true,
        mtu: 1280,
        stp_disabled: true,
        multicast_snooping_disabled: true,
        ipv6_suppressed: true,
        attached_links: 1,
    });
    assert_eq!(
        delete_bridge(&backend, &intent()),
        Err(NetworkOpError::BridgeNotEmpty)
    );
    assert_eq!(*backend.events.borrow(), ["read"]);
    assert_eq!(backend.state.get().attached_links, 1);
}
