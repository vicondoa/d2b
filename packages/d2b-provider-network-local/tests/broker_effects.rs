use std::{
    future::Future,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use d2b_contracts::{
    broker_wire::NftablesProjectionAction,
    types::{BundleOpId, ScopeId, VmId},
    v3::{
        ResourceBundleGenerationId, ResourceUid,
        execution_policy::BoundedToken,
        network::{
            AttachmentGenerationFence, AttachmentHandle, Ipv4Cidr, IsolationSpec, NetworkSpec,
        },
    },
};
use d2b_provider_network_local::{
    broker::{BrokerNetworkEffectPort, NetworkBroker, NetworkBrokerError, NetworkEffectContext},
    controller::{FirewallDigest, FirewallIntent, NetworkEffectError, NetworkEffectPort},
};

#[derive(Clone, Default)]
struct RecordingBroker {
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl RecordingBroker {
    fn record(&self, event: &'static str) {
        self.events.lock().unwrap().push(event);
    }

    fn events(&self) -> Vec<&'static str> {
        self.events.lock().unwrap().clone()
    }
}

impl NetworkBroker for RecordingBroker {
    fn create_bridge(&self, _: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        self.record("create-bridge");
        Ok(())
    }

    fn delete_bridge(&self, _: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        self.record("delete-bridge");
        Ok(())
    }

    fn apply_projection(
        &self,
        _: &NetworkEffectContext,
        action: NftablesProjectionAction,
    ) -> Result<FirewallDigest, NetworkBrokerError> {
        self.record(match action {
            NftablesProjectionAction::Apply => "projection-apply",
            NftablesProjectionAction::Remove => "projection-remove",
        });
        Ok(FirewallDigest::new([7; 32]))
    }

    fn apply_nm_unmanaged(&self, _: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        self.record("nm-unmanaged");
        Ok(())
    }

    fn apply_routes(&self, _: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        self.record("routes");
        Ok(())
    }

    fn apply_sysctls(&self, _: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        self.record("sysctls");
        Ok(())
    }

    fn update_hosts(&self, _: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        self.record("hosts");
        Ok(())
    }

    fn seed_dhcp(&self, _: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        self.record("dhcp");
        Ok(())
    }

    fn delete_persistent_tap(
        &self,
        _: &AttachmentHandle,
        _: &AttachmentGenerationFence,
    ) -> Result<(), NetworkBrokerError> {
        self.record("tap-delete");
        Ok(())
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn context(site_allows_unsafe_east_west: bool) -> NetworkEffectContext {
    NetworkEffectContext::new(
        ScopeId::new("env:work"),
        VmId::new("sys-work-net"),
        BundleOpId::new("bridge:env:work"),
        BundleOpId::new("nft-projection:env:work"),
        BundleOpId::new("nm-unmanaged:host"),
        BundleOpId::new("hosts:host"),
        vec![BundleOpId::new("route:env:work:0")],
        vec![BundleOpId::new("sysctl:env:work:bridge")],
        ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap(),
        [7; 32],
        site_allows_unsafe_east_west,
    )
}

fn network_spec(allow_east_west: bool) -> NetworkSpec {
    NetworkSpec::new(
        Ipv4Cidr::parse("10.20.0.0/24").unwrap(),
        Ipv4Cidr::parse("192.0.2.0/30").unwrap(),
        None,
        false,
        IsolationSpec { allow_east_west },
        Default::default(),
        Default::default(),
        Default::default(),
        None,
        Default::default(),
        None,
        BoundedToken::parse("net-vm-base").unwrap(),
        Vec::new(),
    )
    .unwrap()
}

#[test]
fn broker_port_requires_site_opt_in_before_dispatch() {
    let broker = RecordingBroker::default();
    let port = BrokerNetworkEffectPort::new(broker.clone(), context(false));
    assert_eq!(
        block_on(port.validate_policy(&network_spec(true))),
        Err(NetworkEffectError::EastWestHostOptInRequired)
    );
    assert!(broker.events().is_empty());
}

#[test]
fn broker_port_maps_host_effects_to_typed_broker_calls() {
    let broker = RecordingBroker::default();
    let port = BrokerNetworkEffectPort::new(broker.clone(), context(true));
    let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let generation = ResourceBundleGenerationId::parse(
        "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .unwrap();
    let firewall = FirewallIntent::new(uid.clone(), generation);
    let attachment_uid = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
    let handle = AttachmentHandle::new(
        attachment_uid.clone(),
        AttachmentGenerationFence::new(
            uid.clone(),
            d2b_contracts::v3::ResourceGeneration::new(4).unwrap(),
            attachment_uid,
            d2b_contracts::v3::ResourceGeneration::new(7).unwrap(),
        ),
    );

    block_on(port.create_bridges(&uid)).unwrap();
    block_on(port.apply_sysctls(&uid)).unwrap();
    let _ = block_on(port.apply_host_firewall(&firewall)).unwrap();
    block_on(port.apply_nm_unmanaged()).unwrap();
    block_on(port.apply_routes(&uid)).unwrap();
    block_on(port.update_hosts(&uid)).unwrap();
    block_on(port.seed_dhcp(&uid)).unwrap();
    block_on(port.delete_persistent_tap(&handle, handle.generation_fence())).unwrap();
    block_on(port.remove_host_firewall(&firewall)).unwrap();
    block_on(port.delete_bridges(&uid)).unwrap();

    assert_eq!(
        broker.events(),
        [
            "create-bridge",
            "sysctls",
            "projection-apply",
            "nm-unmanaged",
            "routes",
            "hosts",
            "dhcp",
            "tap-delete",
            "projection-remove",
            "delete-bridge",
        ]
    );
}
