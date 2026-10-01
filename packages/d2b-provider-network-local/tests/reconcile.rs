use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use d2b_contracts_resource::v3::{
    ResourceBundleGenerationId, ResourceGeneration, ResourceUid, ZoneId,
    execution_policy::BoundedToken,
    network::{
        AttachmentGenerationFence, AttachmentHandle, DhcpSpec, DnsSpec, Ipv4Cidr, IsolationSpec,
        MdnsSpec, NetworkAttachmentEntry, NetworkSpec, RoutingSpec,
    },
};
use d2b_provider_network_local::{
    NetworkAdmittedConsumer,
    artifact::{ArtifactCatalogEntry, ArtifactKind},
    controller::{
        AttachmentRealization, FinalizerStage, FirewallDigest, FirewallIntent,
        NetworkAdmissionIntent, NetworkAdmissionKey, NetworkConfigContent, NetworkEffectError,
        NetworkEffectPort, NetworkReconciler, NetworkResourcePort, ReconcileInput,
        ReconcileProgress, render_config_with_provenance,
    },
    plan::{ActualState, PlanStep, compute_plan},
};
use d2b_contracts_resource::v3::ControllerGeneration;
use d2b_provider_network_local::{
    NetworkDriverArgs, NetworkEffectFacets, membership_interface, network_binding_descriptor,
    served_network_consumers,
};
use d2b_provider_toolkit::testing::{
    RecordingManagerEndpoint, RecordingRequeue, block_on,
};
use d2b_resource_runtime::context::{ManagerEndpoint, ResourceContext};
use d2b_resource_runtime::identity::{ResourceProvenance, StoredDesiredResource};

#[derive(Clone, Default)]
struct FakePorts {
    inner: Arc<FakePortState>,
}

#[derive(Default)]
struct FakePortState {
    events: Mutex<Vec<&'static str>>,
    effect_error: Mutex<Option<NetworkEffectError>>,
    mdns_values: Mutex<Vec<bool>>,
    firewall_generations: Mutex<Vec<String>>,
    config_content: Mutex<Vec<NetworkConfigContent>>,
}

impl FakePorts {
    /// Take one recorder lock, failing loudly on poisoning.
    ///
    /// The recorded fields are `std::sync::Mutex`: the port methods are
    /// driven by this file's `block_on` harness (plain `#[test]`
    /// functions, with no runtime) and the test bodies read the records
    /// synchronously, so they cannot be awaited async locks. This is the
    /// single acquisition site and it carries the recorded exception.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn recorder<'a, T>(&self, recorder: &'a Mutex<T>) -> MutexGuard<'a, T> {
        recorder
            .lock()
            .expect("a test-support recorder lock is never poisoned")
    }

    fn push(&self, event: &'static str) -> Result<(), NetworkEffectError> {
        self.recorder(&self.inner.events).push(event);
        let mut configured = self.recorder(&self.inner.effect_error);
        if configured.is_some_and(|error| {
            matches!(
                (event, error),
                (
                    "firewall-apply",
                    NetworkEffectError::StaleConfigurationGeneration
                ) | ("tap-delete", NetworkEffectError::StaleAttachmentGeneration)
                    | ("tap-delete", NetworkEffectError::Transient)
            )
        }) {
            return Err(configured.take().expect("configured error exists"));
        }
        Ok(())
    }

    fn events(&self) -> Vec<&'static str> {
        self.recorder(&self.inner.events).clone()
    }

    /// Script the error the next matching effect reports.
    fn script_effect_error(&self, error: Option<NetworkEffectError>) {
        *self.recorder(&self.inner.effect_error) = error;
    }

    /// The captured firewall generation identities, oldest first.
    fn firewall_generations(&self) -> Vec<String> {
        self.recorder(&self.inner.firewall_generations).clone()
    }

    /// The recorded mDNS values, oldest first.
    fn mdns_values(&self) -> Vec<bool> {
        self.recorder(&self.inner.mdns_values).clone()
    }

    /// The config projection the reconciler wrote to the Volume, oldest first.
    fn config_content(&self) -> Vec<NetworkConfigContent> {
        self.recorder(&self.inner.config_content).clone()
    }

    /// The config projection the most recent reconcile pass wrote.
    fn last_config_content(&self) -> NetworkConfigContent {
        let mut content = self.config_content();
        assert!(!content.is_empty(), "a reconcile pass wrote one config");
        content.pop().expect("a config was written")
    }
}

impl NetworkEffectPort for FakePorts {
    async fn validate_policy(&self, spec: &NetworkSpec) -> Result<(), NetworkEffectError> {
        if spec.isolation().allow_east_west {
            Err(NetworkEffectError::EastWestHostOptInRequired)
        } else {
            Ok(())
        }
    }

    async fn create_bridges(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.push("bridges")
    }

    async fn apply_sysctls(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.push("sysctls")
    }

    async fn apply_host_firewall(
        &self,
        intent: &FirewallIntent,
    ) -> Result<FirewallDigest, NetworkEffectError> {
        self.recorder(&self.inner.firewall_generations)
            .push(intent.expected_generation_id().as_str().to_owned());
        self.push("firewall-apply")?;
        Ok(FirewallDigest::new([1; 32]))
    }

    async fn remove_host_firewall(&self, _: &FirewallIntent) -> Result<(), NetworkEffectError> {
        self.push("firewall-remove")
    }

    async fn apply_nm_unmanaged(&self) -> Result<(), NetworkEffectError> {
        self.push("nm")
    }

    async fn apply_routes(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.push("routes")
    }

    async fn remove_routes(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.push("routes-remove")
    }

    async fn update_hosts(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.push("hosts")
    }

    async fn seed_dhcp(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.push("dhcp")
    }

    async fn delete_persistent_tap(
        &self,
        _: &AttachmentHandle,
        _: &AttachmentGenerationFence,
    ) -> Result<(), NetworkEffectError> {
        self.push("tap-delete")
    }

    async fn delete_bridges(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.push("bridge-delete")
    }
}

impl NetworkResourcePort for FakePorts {
    async fn upsert_volume_backing(
        &self,
        _: &d2b_contracts_resource::v3::volume::VolumeSpec,
    ) -> Result<(), NetworkEffectError> {
        self.push("volume-upsert")
    }

    async fn upsert_volume_content(
        &self,
        content: &NetworkConfigContent,
    ) -> Result<(), NetworkEffectError> {
        self.recorder(&self.inner.config_content).push(content.clone());
        self.push("volume-write")
    }

    async fn upsert_guest(
        &self,
        _: &d2b_provider_guest::GuestSpec,
    ) -> Result<(), NetworkEffectError> {
        self.push("guest-upsert")
    }

    async fn attach_volume(
        &self,
        _: &d2b_contracts_resource::v3::volume::VolumeAttachment,
    ) -> Result<(), NetworkEffectError> {
        self.push("volume-attach")
    }

    async fn upsert_agent(
        &self,
        _: &d2b_contracts_resource::v3::process::ProcessSpec,
    ) -> Result<(), NetworkEffectError> {
        self.push("agent-upsert")
    }

    async fn reconcile_mdns(&self, enabled: bool) -> Result<(), NetworkEffectError> {
        self.recorder(&self.inner.mdns_values).push(enabled);
        self.push("mdns")
    }

    async fn delete_processes(&self) -> Result<(), NetworkEffectError> {
        self.push("process-delete")
    }

    async fn detach_volume(&self) -> Result<(), NetworkEffectError> {
        self.push("volume-detach")
    }

    async fn delete_guest(&self) -> Result<(), NetworkEffectError> {
        self.push("guest-delete")
    }

    async fn delete_volume(&self) -> Result<(), NetworkEffectError> {
        self.push("volume-delete")
    }
}

fn spec(lan: &str, uplink: &str) -> NetworkSpec {
    NetworkSpec::minimal(
        Ipv4Cidr::parse(lan).unwrap(),
        Ipv4Cidr::parse(uplink).unwrap(),
        BoundedToken::parse("net-vm-base").unwrap(),
    )
    .unwrap()
}

fn east_west_spec(lan: &str, uplink: &str) -> NetworkSpec {
    NetworkSpec::new(
        Ipv4Cidr::parse(lan).unwrap(),
        Ipv4Cidr::parse(uplink).unwrap(),
        None,
        false,
        IsolationSpec {
            allow_east_west: true,
        },
        RoutingSpec::default(),
        DhcpSpec::default(),
        DnsSpec::default(),
        None,
        MdnsSpec::default(),
        None,
        BoundedToken::parse("net-vm-base").unwrap(),
        Vec::new(),
    )
    .unwrap()
}

fn generation() -> ResourceBundleGenerationId {
    ResourceBundleGenerationId::parse(
        "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .unwrap()
}

fn input() -> ReconcileInput {
    let network_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let attachment_uid = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
    let spec = spec("10.20.0.0/24", "192.0.2.0/30");
    let admission = NetworkAdmissionIntent::new(
        NetworkAdmissionKey::new(
            ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap(),
            network_uid.clone(),
            ResourceGeneration::new(4).unwrap(),
            ResourceGeneration::new(7).unwrap(),
            generation(),
        ),
        spec.clone(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
    .proof();
    ReconcileInput {
        spec,
        mdns_enabled: false,
        network_uid: network_uid.clone(),
        network_generation: ResourceGeneration::new(4).unwrap(),
        attachment_generation: ResourceGeneration::new(7).unwrap(),
        installed_generation: generation(),
        admission,
        artifact_catalog: vec![ArtifactCatalogEntry::new(
            BoundedToken::parse("net-vm-base").unwrap(),
            ArtifactKind::NixosSystem,
        )],
        user_ready: true,
        host_memory_budget_available: 8 * 1024 * 1024,
        volume_ready: true,
        guest_ready: true,
        volume_attachment_ready: true,
        workload_fds_closed: true,
        agent_deleted: true,
        mdns_deleted: true,
        volume_attachment_removed: true,
        guest_deleted: true,
        volume_deleted: true,
        attachments: vec![AttachmentRealization {
            handle: AttachmentHandle::new(
                attachment_uid.clone(),
                AttachmentGenerationFence::new(
                    network_uid,
                    ResourceGeneration::new(4).unwrap(),
                    attachment_uid,
                    ResourceGeneration::new(7).unwrap(),
                ),
            ),
            vmm_fd_closed: true,
        }],
    }
}

#[test]
fn reconcile_enforces_effect_and_child_readiness_order() {
    let effects = FakePorts::default();
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources.clone());
    assert_eq!(
        block_on(controller.reconcile(&input())).unwrap(),
        ReconcileProgress::Ready
    );
    assert_eq!(
        effects.events(),
        [
            "bridges",
            "sysctls",
            "firewall-apply",
            "nm",
            "routes",
            "hosts",
            "dhcp",
            "tap-delete",
        ]
    );
    assert_eq!(effects.firewall_generations(), [generation().as_str()]);
    assert_eq!(
        resources.events(),
        [
            "volume-upsert",
            "volume-write",
            "guest-upsert",
            "volume-attach",
            "agent-upsert",
            "mdns",
        ]
    );
}

#[test]
fn guest_and_agent_are_barriered_by_volume_and_attachment_readiness() {
    let effects = FakePorts::default();
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources.clone());
    let mut state = input();
    state.volume_ready = false;
    assert!(matches!(
        block_on(controller.reconcile(&state)).unwrap(),
        ReconcileProgress::Pending(_)
    ));
    assert!(!resources.events().contains(&"guest-upsert"));

    let effects = FakePorts::default();
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources.clone());
    let mut state = input();
    state.guest_ready = false;
    assert!(matches!(
        block_on(controller.reconcile(&state)).unwrap(),
        ReconcileProgress::Pending(_)
    ));
    assert!(!resources.events().contains(&"volume-attach"));

    let effects = FakePorts::default();
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources.clone());
    let mut state = input();
    state.volume_attachment_ready = false;
    assert!(matches!(
        block_on(controller.reconcile(&state)).unwrap(),
        ReconcileProgress::Pending(_)
    ));
    assert!(!resources.events().contains(&"agent-upsert"));
}

#[test]
fn stale_configuration_generation_requeues_without_following_effects() {
    let effects = FakePorts::default();
    effects.script_effect_error(Some(NetworkEffectError::StaleConfigurationGeneration));
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources.clone());
    assert!(matches!(
        block_on(controller.reconcile(&input())).unwrap(),
        ReconcileProgress::Requeue(_)
    ));
    assert_eq!(effects.events(), ["bridges", "sysctls", "firewall-apply"]);
    assert!(resources.events().is_empty());
}

#[test]
fn finalizer_never_deletes_bridge_before_tap_and_children() {
    let effects = FakePorts::default();
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources.clone());
    assert_eq!(
        block_on(controller.finalize(&input())).unwrap(),
        FinalizerStage::Complete
    );
    assert_eq!(
        effects.events(),
        [
            "tap-delete",
            "firewall-remove",
            "routes-remove",
            "bridge-delete"
        ]
    );

    let effects = FakePorts::default();
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources.clone());
    let mut waiting = input();
    waiting.attachments[0].vmm_fd_closed = false;
    assert_eq!(
        block_on(controller.finalize(&waiting)).unwrap(),
        FinalizerStage::WorkloadFdClosure
    );
    assert!(effects.events().is_empty());
}

#[test]
fn admission_mismatch_and_host_budget_block_before_effects() {
    let effects = FakePorts::default();
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources.clone());
    let mut conflicting = input();
    conflicting.spec = spec("10.30.0.0/24", "198.51.100.0/30");
    assert_eq!(
        block_on(controller.reconcile(&conflicting)),
        Err(NetworkEffectError::NetworkAdmissionMismatch)
    );
    assert!(effects.events().is_empty());

    let effects = FakePorts::default();
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources.clone());
    let mut exhausted = input();
    exhausted.host_memory_budget_available = 1024;
    assert_eq!(
        block_on(controller.reconcile(&exhausted)),
        Err(NetworkEffectError::HostMemoryBudgetExceeded)
    );
    assert!(effects.events().is_empty());
}

#[test]
fn stale_attachment_admission_refuses_before_effects() {
    let effects = FakePorts::default();
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources);
    let mut stale = input();
    let network_uid = stale.network_uid.clone();
    let attachment_uid = stale.attachments[0].handle.opaque_id().clone();
    stale.attachments[0].handle = AttachmentHandle::new(
        attachment_uid.clone(),
        AttachmentGenerationFence::new(
            network_uid,
            stale.network_generation,
            attachment_uid,
            ResourceGeneration::new(6).unwrap(),
        ),
    );
    assert_eq!(
        block_on(controller.reconcile(&stale)),
        Err(NetworkEffectError::NetworkAdmissionMismatch)
    );
    assert!(effects.events().is_empty());
}

#[test]
fn user_readiness_and_mdns_toggle_are_explicit() {
    let effects = FakePorts::default();
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources.clone());
    let mut waiting = input();
    waiting.user_ready = false;
    assert!(matches!(
        block_on(controller.reconcile(&waiting)).unwrap(),
        ReconcileProgress::Pending(_)
    ));
    assert!(effects.events().is_empty());

    let mut enabled = input();
    enabled.mdns_enabled = true;
    assert_eq!(
        block_on(controller.reconcile(&enabled)).unwrap(),
        ReconcileProgress::Ready
    );
    assert_eq!(resources.mdns_values(), [true]);
}

#[test]
fn east_west_requires_the_site_opt_in_before_any_effect() {
    let effects = FakePorts::default();
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources);
    let mut state = input();
    state.spec = east_west_spec("10.20.0.0/24", "192.0.2.0/30");
    assert_eq!(
        block_on(controller.reconcile(&state)),
        Err(NetworkEffectError::EastWestHostOptInRequired)
    );
    assert!(effects.events().is_empty());
}

#[test]
fn transient_tap_delete_retains_finalizer_stage_for_retry() {
    let effects = FakePorts::default();
    effects.script_effect_error(Some(NetworkEffectError::Transient));
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects.clone(), resources);
    assert_eq!(
        block_on(controller.finalize(&input())).unwrap(),
        FinalizerStage::PersistentTaps
    );
    assert!(!effects.events().contains(&"bridge-delete"));
}

#[test]
fn finalizer_removes_volume_attachment_before_guest_and_volume() {
    let effects = FakePorts::default();
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(effects, resources.clone());
    let mut state = input();
    state.volume_attachment_removed = false;
    state.guest_deleted = false;
    state.volume_deleted = false;
    assert_eq!(
        block_on(controller.finalize(&state)).unwrap(),
        FinalizerStage::VolumeAttachment
    );
    assert_eq!(resources.events(), ["volume-detach"]);

    state.volume_attachment_removed = true;
    assert_eq!(
        block_on(controller.finalize(&state)).unwrap(),
        FinalizerStage::Guest
    );
    assert_eq!(resources.events(), ["volume-detach", "guest-delete"]);

    state.guest_deleted = true;
    assert_eq!(
        block_on(controller.finalize(&state)).unwrap(),
        FinalizerStage::Volume
    );
    assert_eq!(
        resources.events(),
        ["volume-detach", "guest-delete", "volume-delete"]
    );
}

#[test]
fn matching_mdns_state_does_not_schedule_a_second_effect() {
    let plan = compute_plan(
        &spec("10.20.0.0/24", "192.0.2.0/30"),
        true,
        ActualState {
            bridges_ready: true,
            sysctls_ready: true,
            firewall_ready: true,
            volume_ready: true,
            guest_ready: true,
            attachment_ready: true,
            agent_ready: true,
            mdns_matches: true,
        },
    );

    assert!(!plan.steps().contains(&PlanStep::ReconcileMdns));
}

#[test]
fn network_runner_is_the_only_scheduler_and_watches_config_as_dependency() {
    let contract = d2b_provider_network_local::controller::network_runner_contract();
    assert_eq!(contract.resource_type(), "Network");
    assert_eq!(contract.finalizer(), "network.d2bus.org/fabric-cleanup");
    assert!(contract.watched_configuration_is_dependency());
    assert!((30..=60).contains(&contract.repair_interval_secs()));
}

// ---------------------------------------------------------------------------
// The live render reads the committed membership, not a caller argument.
// ---------------------------------------------------------------------------

const ZONE_UID: &str = "323e4567-e89b-42d3-a456-426614174002";
const FRONTEND_UID: &str = "423e4567-e89b-42d3-a456-426614174003";
const BACKEND_UID: &str = "523e4567-e89b-42d3-a456-426614174004";
const UNATTACHED_UID: &str = "623e4567-e89b-42d3-a456-426614174005";

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("canonical resource uid")
}

fn reference(value: &str) -> d2b_contracts_resource::v3::ResourceRef {
    d2b_contracts_resource::v3::ResourceRef::parse(value).expect("resource reference")
}

/// One committed `Network` row whose attachments are its execution targets.
fn committed_network_row(attached: &[&str]) -> NetworkSpec {
    NetworkSpec::new(
        Ipv4Cidr::parse("10.20.0.0/24").unwrap(),
        Ipv4Cidr::parse("192.0.2.0/30").unwrap(),
        None,
        false,
        IsolationSpec::default(),
        RoutingSpec::default(),
        DhcpSpec::default(),
        DnsSpec::default(),
        None,
        MdnsSpec::default(),
        None,
        BoundedToken::parse("net-vm-base").unwrap(),
        attached
            .iter()
            .enumerate()
            .map(|(index, target)| {
                NetworkAttachmentEntry::new(reference(target), index as u8 + 2, None)
                    .expect("declared attachment")
            })
            .collect(),
    )
    .expect("committed network row")
}

/// One committed `NetworkBinding` relationship: the consumer's own request.
fn committed_relationship(
    consumer: &str,
    consumer_uid: &str,
    ports: Vec<d2b_contracts_resource::v3::PortSpec>,
    allow_egress: bool,
) -> NetworkAdmittedConsumer {
    let request = d2b_contracts_resource::v3::NetworkBindingRequest::new(
        reference("Network/lan"),
        reference(consumer),
        d2b_contracts_resource::v3::BindingSlot::parse("fabric").expect("bounded slot"),
        d2b_contracts_resource::v3::NetworkMembership::new(ports, allow_egress)
            .expect("membership policy"),
        d2b_contracts_resource::v3::NetworkPresentation::shared_fabric(),
    )
    .expect("committed network binding request");
    NetworkAdmittedConsumer::new(reference(consumer), uid(consumer_uid), request)
        .expect("committed relationship")
}

fn tcp_port(number: u16, purpose: &str) -> d2b_contracts_resource::v3::PortSpec {
    d2b_contracts_resource::v3::PortSpec::new(
        number,
        d2b_contracts_resource::v3::PortProtocol::Tcp,
        purpose,
    )
    .expect("declared port")
}

/// The production input a committed row and its committed relationships make.
fn committed_input(
    spec: NetworkSpec,
    relationships: Vec<NetworkAdmittedConsumer>,
) -> ReconcileInput {
    let mut state = input();
    let consumers = relationships
        .iter()
        .map(|relationship| relationship.consumer_uid().clone())
        .collect();
    state.spec = spec.clone();
    state.admission = NetworkAdmissionIntent::new(
        NetworkAdmissionKey::new(
            uid(ZONE_UID),
            state.network_uid.clone(),
            state.network_generation,
            state.attachment_generation,
            generation(),
        ),
        spec,
        consumers,
        relationships,
    )
    .expect("root-owned host intent")
    .proof();
    state
}

/// The live reconcile render carries a committed Network row's membership.
///
/// The proven path is the production one: a committed `Network` row plus its
/// committed `NetworkBinding` relationships build the host admission, the
/// admission travels into the reconciler's own `ReconcileInput`, and the
/// reconciler writes the config into the config Volume. Nothing here calls the
/// renderer.
#[test]
fn the_live_render_carries_a_committed_membership() {
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(FakePorts::default(), resources.clone());
    let rendered = |state: &ReconcileInput| {
        let _ = block_on(controller.reconcile(state)).expect("committed network reconciles");
        resources.last_config_content()
    };

    let committed = committed_input(
        committed_network_row(&["Guest/frontend", "Guest/backend"]),
        vec![
            committed_relationship(
                "Guest/frontend",
                FRONTEND_UID,
                vec![tcp_port(443, "https")],
                false,
            ),
            committed_relationship(
                "Guest/backend",
                BACKEND_UID,
                vec![tcp_port(8443, "admin")],
                true,
            ),
        ],
    );
    let with_memberships = rendered(&committed);

    let dnsmasq = String::from_utf8(with_memberships.dnsmasq.clone()).expect("dnsmasq bytes");
    assert!(dnsmasq.contains("consumer=Guest/backend"), "{dnsmasq}");
    assert!(dnsmasq.contains("consumer=Guest/frontend"), "{dnsmasq}");
    assert!(dnsmasq.contains("egress=allow"), "{dnsmasq}");
    assert!(dnsmasq.contains("egress=deny"), "{dnsmasq}");

    let provenance = committed.admission.key().provenance();
    let fabric = |consumer_uid: &str| {
        d2b_provider_network_local::membership_interface(&provenance, &uid(consumer_uid))
            .expect("membership interface")
    };
    let attachments = String::from_utf8(with_memberships.attachments.clone()).expect("attachments");
    assert_eq!(
        attachments,
        format!(
            "[2,3,member={},member={}]",
            fabric(FRONTEND_UID).as_str(),
            fabric(BACKEND_UID).as_str()
        ),
        "each committed consumer holds its own fabric interface"
    );

    // The shared fabric's own projection is realized once for both consumers.
    let without_memberships = rendered(&committed_input(committed_network_row(&["Guest/frontend"]), Vec::new()));
    assert_eq!(
        with_memberships.nftables, without_memberships.nftables,
        "memberships do not fork the Network's single firewall slot"
    );
    assert_eq!(with_memberships.routing, without_memberships.routing);
    assert_ne!(
        with_memberships.digest(),
        without_memberships.digest(),
        "the consumer policies are part of what the config commits"
    );
}

/// A Network row that declares no membership renders exactly what it renders
/// today: the fabric alone, byte for byte.
#[test]
fn a_network_row_without_a_membership_renders_todays_bytes() {
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(FakePorts::default(), resources.clone());
    let spec = committed_network_row(&["Guest/frontend"]);
    let state = committed_input(spec.clone(), Vec::new());
    let _ = block_on(controller.reconcile(&state)).expect("committed network reconciles");
    let rendered = resources.last_config_content();

    assert_eq!(
        rendered,
        render_config_with_provenance(&spec, &state.admission.key().provenance())
            .expect("the fabric-only render"),
        "an attached row with no committed membership renders the fabric alone"
    );
    assert_eq!(rendered.dnsmasq, b"lan=10.20.0.0/24\n");
    assert_eq!(rendered.attachments, b"[2]");
}

/// A committed membership naming a consumer the committed `Network` row does
/// not attach renders nothing, and the row that does attach still renders.
#[test]
fn a_membership_for_a_consumer_the_row_does_not_attach_does_not_render() {
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(FakePorts::default(), resources.clone());
    let attached_only = committed_input(
        committed_network_row(&["Guest/frontend"]),
        vec![committed_relationship(
            "Guest/frontend",
            FRONTEND_UID,
            vec![tcp_port(443, "https")],
            false,
        )],
    );
    let _ = block_on(controller.reconcile(&attached_only)).expect("committed network reconciles");
    let baseline = resources.last_config_content();

    let with_unattached = committed_input(
        committed_network_row(&["Guest/frontend"]),
        vec![
            committed_relationship(
                "Guest/frontend",
                FRONTEND_UID,
                vec![tcp_port(443, "https")],
                false,
            ),
            committed_relationship(
                "Guest/elsewhere",
                UNATTACHED_UID,
                vec![tcp_port(9000, "smuggled")],
                true,
            ),
        ],
    );
    let _ = block_on(controller.reconcile(&with_unattached)).expect("committed network reconciles");
    let rendered = resources.config_content().remove(1);

    let dnsmasq = String::from_utf8(rendered.dnsmasq.clone()).expect("dnsmasq bytes");
    assert!(!dnsmasq.contains("Guest/elsewhere"), "{dnsmasq}");
    assert!(!dnsmasq.contains("9000"), "{dnsmasq}");
    assert_eq!(
        rendered, baseline,
        "a source row implies no relationship for a consumer it does not declare"
    );
}

/// A committed membership naming a consumer the host admission never admitted
/// is refused at admission, before the reconciler exists: the render can never
/// describe traffic policy for a consumer that holds no fabric tap.
#[test]
fn a_membership_for_an_unadmitted_consumer_is_refused_at_admission() {
    let spec = committed_network_row(&["Guest/frontend"]);
    let smuggled = committed_relationship(
        "Guest/elsewhere",
        UNATTACHED_UID,
        vec![tcp_port(9000, "smuggled")],
        true,
    );
    assert_eq!(
        NetworkAdmissionIntent::new(
            NetworkAdmissionKey::new(
                uid(ZONE_UID),
                uid("123e4567-e89b-42d3-a456-426614174000"),
                ResourceGeneration::new(4).unwrap(),
                ResourceGeneration::new(7).unwrap(),
                generation(),
            ),
            spec,
            Vec::new(),
            vec![smuggled],
        )
        .expect_err("a committed membership for an unadmitted consumer is refused"),
        NetworkEffectError::NetworkAdmissionMismatch
    );
}

// ---------------------------------------------------------------------------
// Production composition: committed Network row -> committed NetworkBinding rows
// ---------------------------------------------------------------------------

/// One committed row in the manager recorder the production driver reads.
fn committed_row(
    zone: &str,
    type_name: &str,
    name: &str,
    uid: ResourceUid,
    spec: serde_json::Value,
) -> StoredDesiredResource {
    StoredDesiredResource {
        key: d2b_resource_runtime::identity::ResourceKey::new(zone, type_name, name),
        uid: manager_uid(&uid),
        generation: 3,
        owner_uid: None,
        provenance: ResourceProvenance::Resource,
        deleting: false,
        spec: serde_json::to_vec(&spec).expect("canonical row spec"),
        metadata: Vec::new(),
        created_at: 0,
    }
}

/// The manager's 16-byte durable identity for one committed resource uid.
///
/// The manager stores a 16-byte uid while the contract spells an identity as
/// 32 bytes, so the fixture derives the stored half from the committed one
/// rather than carrying a second hand-written identifier.
fn manager_uid(uid: &ResourceUid) -> [u8; 16] {
    let mut digits: Vec<u8> = uid
        .as_str()
        .as_bytes()
        .iter()
        .copied()
        .filter(|byte| *byte != b'-')
        .collect();
    digits.truncate(32);
    let mut slot = [0u8; 16];
    for (index, pair) in digits.chunks_exact(2).enumerate() {
        slot[index] = (hex_digit(pair[0]) << 4) | hex_digit(pair[1]);
    }
    slot
}

/// One hexadecimal digit's value.
fn hex_digit(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => panic!("a canonical uid is lowercase hexadecimal"),
    }
}

/// The effects runtime the production driver delegates its host effects to.
///
/// The producing pass this test proves is the driver's child surface, which
/// runs BEFORE the typed effect, so the runtime here only has to answer: the
/// stub reports the row as not converged rather than reaching for a host.
struct PendingRuntime;

#[async_trait::async_trait]
impl d2b_provider_network_local::NetworkRuntime for PendingRuntime {
    async fn bundle(&self) -> Arc<d2b_core::bundle_resolver::BundleResolver> {
        unreachable!("the host effect is not reached by the producing pass")
    }

    fn broker_socket_path(&self) -> &Path {
        Path::new("/nonexistent/d2b-test-broker.sock")
    }

    fn caller_role(&self) -> d2b_contracts_broker::broker_wire::BrokerCallerRole {
        d2b_contracts_broker::broker_wire::BrokerCallerRole::AdminUid { uid: 0 }
    }

    async fn reconcile_network(
        &self,
        _request: &d2b_provider_toolkit::SharedProviderEffectRequest<'_>,
    ) -> Result<
        d2b_provider_toolkit::SharedProviderEffectOutcome,
        d2b_provider_toolkit::SharedProviderEffectError,
    > {
        Ok(d2b_provider_toolkit::SharedProviderEffectOutcome {
            phase: d2b_provider_toolkit::SharedProviderEffectPhase::Pending,
            resource_projection: None,
        })
    }

    async fn finalize_network(
        &self,
        _request: &d2b_provider_toolkit::SharedProviderEffectRequest<'_>,
    ) -> Result<d2b_provider_toolkit::SharedProviderFinalize, d2b_provider_toolkit::SharedProviderEffectError>
    {
        Ok(d2b_provider_toolkit::SharedProviderFinalize::Complete)
    }
}

/// Drive the production Network driver over one committed Network row and
/// return the `NetworkBinding` rows the pass committed.
fn produce_committed_binding_rows(
    zone: &str,
    attached: &[&str],
) -> Vec<StoredDesiredResource> {
    let network_uid = uid("223e4567-e89b-42d3-a456-426614174001");
    let zone_uid = uid(ZONE_UID);
    let mut rows = vec![
        committed_row(zone, "Zone", zone, zone_uid, serde_json::json!({})),
        committed_row(
            zone,
            "Guest",
            "frontend",
            uid(FRONTEND_UID),
            guest_row_spec(attached, "frontend"),
        ),
        committed_row(
            zone,
            "Guest",
            "backend",
            uid(BACKEND_UID),
            guest_row_spec(attached, "backend"),
        ),
    ];
    let mut network_spec = serde_json::to_value(committed_network_row(attached))
        .expect("committed Network base spec");
    network_spec
        .as_object_mut()
        .expect("a JSON object spec")
        .insert(
            "providerRef".to_owned(),
            serde_json::Value::String("Provider/network-local".to_owned()),
        );
    rows.push(committed_row(
        zone,
        "Network",
        "lan",
        network_uid,
        network_spec.clone(),
    ));

    let manager = Arc::new(
        RecordingManagerEndpoint::new()
            .with_zone(zone)
            .with_owner_uid(manager_uid(&uid("223e4567-e89b-42d3-a456-426614174001")))
            .with_rows(rows),
    );
    let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
    let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
    let driver_row = committed_row(
        zone,
        "Network",
        "lan",
        uid("223e4567-e89b-42d3-a456-426614174001"),
        network_spec.clone(),
    );
    let decoder = d2b_provider_toolkit::shared_provider_spec_decoder();
    let mut ctx = ResourceContext::new(
        driver_row,
        decoder,
        Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
        Arc::new(RecordingRequeue::default()),
        effects_tx,
        notify_tx,
    );
    let descriptor = d2b_provider_network_local::network_descriptor(NetworkDriverArgs {
        zone: ZoneId::parse("dev").expect("a zone"),
        controller_generation: ControllerGeneration::new(1).expect("generation"),
        facets: NetworkEffectFacets {
            runtime: Arc::new(PendingRuntime),
        },
    });
    block_on(async {
        let mut driver = descriptor.factory.create(ctx.key()).await;
        // The producing pass is the driver's child surface, which runs ahead of
        // the typed effect, so the rows are committed whether or not the row
        // itself has converged.
        let _ = driver.reconcile(&mut ctx).await;
    });
    manager
        .rows()
        .into_iter()
        .filter(|row| row.key.type_name == "NetworkBinding")
        .collect()
}

/// A committed Guest row whose own execution policy attaches this Network.
fn guest_row_spec(attached: &[&str], name: &str) -> serde_json::Value {
    let _ = name;
    serde_json::json!({
        "providerRef": "Provider/guest-cloud-hypervisor",
        "networkAttachments": attached
            .iter()
            .map(|target| serde_json::json!({"networkRef": target, "default": false}))
            .collect::<Vec<_>>(),
    })
}

/// The production composition carries a committed Network row's membership
/// into the rendered config.
///
/// The whole chain is the production one: the committed `Network` row is driven
/// by this crate's own declared driver, whose producing pass commits the
/// `NetworkBinding` rows the row's own attachments imply; those committed rows
/// are then served back into the relationships the host admission consumes; and
/// the reconciler writes the config the membership policy reaches. A render over
/// an empty relationship list would still be green, so the assertions below are
/// on the committed rows themselves and on the bytes they render.
#[test]
fn a_committed_network_row_carries_its_membership_into_the_rendered_config() {
    let resources = FakePorts::default();
    let controller = NetworkReconciler::new(FakePorts::default(), resources.clone());
    let committed = produce_committed_binding_rows("dev", &["Guest/frontend", "Guest/backend"]);

    assert_eq!(
        committed.len(),
        2,
        "one committed NetworkBinding row per execution target the row attaches"
    );
    let committed_ref = reference("Network/lan");
    let mut relationships = Vec::new();
    for row in &committed {
        let decoded: d2b_contracts_resource::v3::NetworkBindingSpec =
            serde_json::from_slice(&row.spec).expect("a committed NetworkBinding row");
        assert_eq!(
            decoded.network_ref(),
            &committed_ref,
            "the committed row names the Network that minted it"
        );
        relationships.push(d2b_provider_network_local::CommittedNetworkBinding::new(
            BoundedToken::parse(&row.key.name).expect("a bounded row name"),
            row.spec.clone(),
            consumer_uid(row),
        ));
    }

    // The serving half reads the committed rows back into the relationships
    // the host admission consumes.
    let served = served_network_consumers(
        &ZoneId::parse("dev").expect("a zone"),
        &committed_ref,
        &uid("223e4567-e89b-42d3-a456-426614174001"),
        &committed_network_row(&["Guest/frontend", "Guest/backend"]),
        &relationships,
    )
    .expect("the committed rows serve as the relationships they were derived from");
    assert_eq!(served.len(), 2, "each committed row serves one relationship");

    let mut state = input();
    state.spec = committed_network_row(&["Guest/frontend", "Guest/backend"]);
    state.admission = NetworkAdmissionIntent::new(
        NetworkAdmissionKey::new(
            uid(ZONE_UID),
            state.network_uid.clone(),
            state.network_generation,
            state.attachment_generation,
            generation(),
        ),
        state.spec.clone(),
        served.iter().map(|entry| entry.consumer_uid().clone()).collect(),
        served.clone(),
    )
    .expect("the served relationships are admitted onto the fabric")
    .proof();
    assert_eq!(
        state.admission.intent().memberships().len(),
        2,
        "each committed relationship renders one consumer policy"
    );

    block_on(controller.reconcile(&state)).expect("committed network reconciles");
    let rendered = resources.last_config_content();
    let dnsmasq = String::from_utf8(rendered.dnsmasq.clone()).expect("dnsmasq bytes");
    assert!(
        dnsmasq.contains("consumer=Guest/frontend"),
        "the committed Host consumer's membership is in the live config: {dnsmasq}"
    );
    assert!(
        dnsmasq.contains("consumer=Guest/backend"),
        "the committed Guest consumer's membership is in the live config: {dnsmasq}"
    );
    let attachments = String::from_utf8(rendered.attachments.clone()).expect("attachments");
    let provenance = state.admission.key().provenance();
    assert!(
        attachments.contains(&format!(
            "member={}",
            membership_interface(&provenance, &uid(FRONTEND_UID))
                .expect("membership interface")
                .as_str()
        )),
        "each committed consumer holds its own derived fabric interface: {attachments}"
    );
}

/// A Network row that attaches nothing commits no relationship and renders the
/// fabric alone, so a render over an empty relationship list is never what a
/// committed row produces.
#[test]
fn a_committed_network_row_that_attaches_nothing_renders_no_membership() {
    let committed = produce_committed_binding_rows("dev", &[]);
    assert!(
        committed.is_empty(),
        "zero committed attachments implies zero committed relationships"
    );
    let served = served_network_consumers(
        &ZoneId::parse("dev").expect("a zone"),
        &reference("Network/lan"),
        &uid("223e4567-e89b-42d3-a456-426614174001"),
        &committed_network_row(&[]),
        &[],
    )
    .expect("an empty committed row set serves no relationship");
    assert!(served.is_empty());
}

/// A committed row that stops attaching a consumer retires that consumer's
/// relationship, so its membership leaves the served set rather than lingering
/// on a row the source no longer declares.
#[test]
fn a_relationship_the_committed_row_no_longer_attaches_is_not_served() {
    let committed = produce_committed_binding_rows("dev", &["Guest/frontend", "Guest/backend"]);
    let backend = committed
        .iter()
        .find(|row| {
            serde_json::from_slice::<d2b_contracts_resource::v3::NetworkBindingSpec>(&row.spec)
                .map(|decoded| decoded.execution_ref() == &reference("Guest/backend"))
                .unwrap_or(false)
        })
        .expect("the backend's committed relationship");
    // The owning Network row now attaches only the frontend, so the backend's
    // committed row names a consumer the source row does not declare.
    let served = served_network_consumers(
        &ZoneId::parse("dev").expect("a zone"),
        &reference("Network/lan"),
        &uid("223e4567-e89b-42d3-a456-426614174001"),
        &committed_network_row(&["Guest/frontend"]),
        &[d2b_provider_network_local::CommittedNetworkBinding::new(
            BoundedToken::parse(&backend.key.name).expect("a bounded row name"),
            backend.spec.clone(),
            consumer_uid(backend),
        )],
    )
    .expect("the unattached relationship is skipped, not refused");
    assert!(
        served.is_empty(),
        "a membership whose consumer the committed Network row does not attach is not served"
    );
}

/// The consumer identity one committed relationship row was derived for.
///
/// The row name is a digest of the relationship's committed identities, so the
/// consumer a committed row names is read back off its own committed bytes and
/// matched against the committed identity the producing pass derived it from.
/// That pairing is the same proof the serving half makes before it serves a
/// row: a mismatched identity yields a different name.
fn consumer_uid(row: &StoredDesiredResource) -> ResourceUid {
    let decoded: d2b_contracts_resource::v3::NetworkBindingSpec =
        serde_json::from_slice(&row.spec).expect("a committed NetworkBinding row");
    match decoded.execution_ref().name().as_str() {
        "frontend" => uid(FRONTEND_UID),
        "backend" => uid(BACKEND_UID),
        other => panic!("no committed consumer identity is declared for {other}"),
    }
}

/// The serving driver is registered for the committed relationship type and
/// decides exactly the rows the producing pass minted.
///
/// The row the producing pass committed is accepted; a row whose name is not
/// the name this source derives for its own identities is refused. That is the
/// whole contract of the serving half: a committed relationship is served only
/// when the boundary can prove the source minted it.
#[test]
fn the_serving_driver_serves_exactly_the_rows_the_producing_pass_minted() {
    let zone = "dev";
    let network_uid = uid("223e4567-e89b-42d3-a456-426614174001");
    let committed = produce_committed_binding_rows(zone, &["Guest/frontend"]);
    let minted = committed.first().expect("the producing pass committed a row");
    let descriptor = network_binding_descriptor(
        d2b_provider_network_local::NetworkBindingDriverArgs {
            zone: ZoneId::parse(zone).expect("a zone"),
        },
    );
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        "NetworkBinding"
    );
    assert!(!descriptor.exportable);
    assert_eq!(
        descriptor.factory.resource_types(),
        &[d2b_resource_runtime::identity::ResourceTypeName::new("NetworkBinding")],
    );
    assert!(
        descriptor.operations.is_empty()
            && descriptor.creations.is_empty()
            && descriptor.startup.is_empty()
            && descriptor.services.is_empty(),
        "a relationship owns no operation, no child, no startup step, and no service"
    );

    let served = move |name: &str, spec: Vec<u8>| {
        let mut rows = vec![
            committed_row(zone, "Zone", zone, uid(ZONE_UID), serde_json::json!({})),
            committed_row(
                zone,
                "Guest",
                "frontend",
                uid(FRONTEND_UID),
                guest_row_spec(&["Guest/frontend"], "frontend"),
            ),
        ];
        let mut network_spec =
            serde_json::to_value(committed_network_row(&["Guest/frontend"]))
                .expect("committed Network base spec");
        network_spec
            .as_object_mut()
            .expect("a JSON object spec")
            .insert(
                "providerRef".to_owned(),
                serde_json::Value::String("Provider/network-local".to_owned()),
            );
        rows.push(committed_row(
            zone,
            "Network",
            "lan",
            network_uid.clone(),
            network_spec,
        ));
        let mut relationship = committed_row(
            zone,
            "NetworkBinding",
            name,
            uid("823e4567-e89b-42d3-a456-426614174009"),
            serde_json::from_slice(&minted.spec).expect("the minted row's base"),
        );
        relationship.owner_uid = Some(manager_uid(&network_uid));
        relationship.spec = spec;
        rows.push(relationship);
        let manager = Arc::new(
            RecordingManagerEndpoint::new()
                .with_zone(zone)
                .with_owner_uid(manager_uid(&network_uid))
                .with_rows(rows),
        );
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut driver_row = committed_row(
            zone,
            "NetworkBinding",
            name,
            uid("823e4567-e89b-42d3-a456-426614174009"),
            serde_json::from_slice(&minted.spec).expect("the minted row's base"),
        );
        driver_row.owner_uid = Some(manager_uid(&network_uid));
        driver_row.spec = serde_json::to_vec(&d2b_contracts_resource::v3::ResourceSpec::new(
            None,
            None,
            d2b_contracts_resource::v3::CanonicalJsonObject::parse(&minted.spec)
                .expect("the minted row's canonical bytes"),
            None,
        )
        .expect("the stored envelope"))
        .expect("the envelope's canonical bytes");
        let mut ctx = ResourceContext::new(
            driver_row,
            descriptor.decoder.clone(),
            Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            Arc::new(RecordingRequeue::default()),
            effects_tx,
            notify_tx,
        );
        block_on(async {
            let mut driver = descriptor.factory.create(ctx.key()).await;
            driver.validate(&mut ctx).await
        })
    };

    let minted_result = served(&minted.key.name, minted.spec.clone());
    assert!(
        minted_result.is_ok(),
        "the row the producing pass minted is served: {minted_result:?}"
    );
    assert!(
        served("net-binding-not-the-derived-name", minted.spec.clone()).is_err(),
        "a row whose name is not the name this source derives is refused"
    );
}
