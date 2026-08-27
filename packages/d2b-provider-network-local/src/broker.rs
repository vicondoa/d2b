//! Core-owned broker effect adapter for Network reconciliation.
//!
//! The Provider only sees this typed boundary. The concrete Core composition
//! supplies a broker implementation that resolves every opaque intent against
//! its trusted bundle before dispatching a wire operation.

use std::fmt;

use d2b_contracts::types::{BundleOpId, ScopeId, VmId};
use d2b_contracts_broker::broker_wire::{NetworkTapContext, NftablesProjectionAction};
use d2b_contracts_resource::v3::{
    IfName, NetworkIfRole, NetworkProvenance, ResourceBundleGenerationId, ResourceUid,
    network::{AttachmentGenerationFence, AttachmentHandle, NetworkSpec},
};

use crate::controller::{
    FirewallDigest, FirewallIntent, NetworkAdmissionProof, NetworkEffectError, NetworkEffectPort,
};

/// Closed failures returned by the Core-to-broker adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkBrokerError {
    /// The broker transport or socket was unavailable.
    Transport,
    /// The broker rejected the typed request.
    Rejected,
    /// The installed bundle generation no longer matches the request.
    StaleGeneration,
    /// The attachment generation no longer matches the request.
    StaleAttachmentGeneration,
    /// A foreign ownership marker blocked the mutation.
    ForeignOwnership,
    /// A transient host operation should be retried.
    Transient,
    /// The site-level east-west acknowledgement is absent.
    EastWestHostOptInRequired,
    /// A Network operation did not carry root-owned admission evidence.
    NetworkAdmissionRequired,
    /// The Network identity in an intent does not match the admission proof.
    NetworkAdmissionMismatch,
    /// Root-owned Network admission refused the host projection.
    NetworkAdmissionConflict,
    /// Derived interface names collided during host preflight.
    NetworkInterfaceCollision,
    /// Derived route names collided during host preflight.
    NetworkRouteCollision,
}

impl NetworkBrokerError {
    /// Return the stable redacted reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Transport => "network-broker-transport",
            Self::Rejected => "network-broker-rejected",
            Self::StaleGeneration => "stale-projection-generation",
            Self::StaleAttachmentGeneration => "stale-attachment-generation",
            Self::ForeignOwnership => "foreign-nft-rule-preserved",
            Self::Transient => "network-broker-transient",
            Self::EastWestHostOptInRequired => "east-west-host-opt-in-required",
            Self::NetworkAdmissionRequired => "network-admission-required",
            Self::NetworkAdmissionMismatch => "network-admission-mismatch",
            Self::NetworkAdmissionConflict => "network-admission-conflict",
            Self::NetworkInterfaceCollision => "network-interface-collision",
            Self::NetworkRouteCollision => "network-route-collision",
        }
    }
}

impl fmt::Display for NetworkBrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for NetworkBrokerError {}

/// Opaque bundle references and policy resolved by Core for one Network.
#[derive(Clone, PartialEq, Eq)]
pub struct NetworkEffectContext {
    scope_id: ScopeId,
    dnsmasq_vm_id: VmId,
    bridge_intent_ref: BundleOpId,
    bridge_intent_refs: Vec<BundleOpId>,
    projection_intent_ref: BundleOpId,
    nm_intent_ref: BundleOpId,
    hosts_intent_ref: BundleOpId,
    route_intent_refs: Vec<BundleOpId>,
    sysctl_intent_refs: Vec<BundleOpId>,
    expected_generation_id: ResourceBundleGenerationId,
    projection_digest: [u8; 32],
    site_allows_unsafe_east_west: bool,
    host_global_nic_admitted: bool,
    network_admission: Option<NetworkAdmissionProof>,
}

impl NetworkEffectContext {
    /// Construct one context without accepting raw host locators or payloads.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scope_id: ScopeId,
        dnsmasq_vm_id: VmId,
        bridge_intent_ref: BundleOpId,
        projection_intent_ref: BundleOpId,
        nm_intent_ref: BundleOpId,
        hosts_intent_ref: BundleOpId,
        route_intent_refs: Vec<BundleOpId>,
        sysctl_intent_refs: Vec<BundleOpId>,
        expected_generation_id: ResourceBundleGenerationId,
        projection_digest: [u8; 32],
        site_allows_unsafe_east_west: bool,
    ) -> Self {
        let bridge_intent_refs = vec![bridge_intent_ref.clone()];
        Self {
            scope_id,
            dnsmasq_vm_id,
            bridge_intent_ref,
            bridge_intent_refs,
            projection_intent_ref,
            nm_intent_ref,
            hosts_intent_ref,
            route_intent_refs,
            sysctl_intent_refs,
            expected_generation_id,
            projection_digest,
            site_allows_unsafe_east_west,
            host_global_nic_admitted: false,
            network_admission: None,
        }
    }

    /// Bind an immutable Host-global Network admission proof.
    pub fn with_network_admission(mut self, proof: NetworkAdmissionProof) -> Self {
        self.network_admission = Some(proof);
        self
    }

    /// Add another trusted Network bridge intent to the atomic fabric set.
    pub fn with_additional_bridge_intent(mut self, intent_ref: BundleOpId) -> Self {
        if !self.bridge_intent_refs.iter().any(|current| current == &intent_ref) {
            self.bridge_intent_refs.push(intent_ref);
        }
        self
    }

    /// Mark the context after Core has admitted all requested physical-NIC
    /// claims through the Host-global authority index.
    pub fn with_host_global_nic_admission(mut self) -> Self {
        self.host_global_nic_admitted = true;
        self
    }

    /// Construct a broker context bound to one root-admitted Network.
    #[allow(clippy::too_many_arguments)]
    pub fn for_network(
        proof: NetworkAdmissionProof,
        dnsmasq_vm_id: VmId,
        bridge_intent_ref: BundleOpId,
        projection_intent_ref: BundleOpId,
        nm_intent_ref: BundleOpId,
        hosts_intent_ref: BundleOpId,
        route_intent_refs: Vec<BundleOpId>,
        sysctl_intent_refs: Vec<BundleOpId>,
        expected_generation_id: ResourceBundleGenerationId,
        projection_digest: [u8; 32],
        site_allows_unsafe_east_west: bool,
    ) -> Self {
        let key = proof.key();
        Self::new(
            ScopeId::new(format!(
                "network:{}:{}",
                key.zone_uid().as_str(),
                key.network_uid().as_str()
            )),
            dnsmasq_vm_id,
            bridge_intent_ref,
            projection_intent_ref,
            nm_intent_ref,
            hosts_intent_ref,
            route_intent_refs,
            sysctl_intent_refs,
            expected_generation_id,
            projection_digest,
            site_allows_unsafe_east_west,
        )
        .with_network_admission(proof)
    }

    /// Construct the minimal Core context used by the daemon's aggregate
    /// host-preparation path for the shared NetworkManager projection.
    pub fn for_host_nm(
        scope_id: ScopeId,
        nm_intent_ref: BundleOpId,
        expected_generation_id: ResourceBundleGenerationId,
    ) -> Self {
        Self::new(
            scope_id,
            VmId::new("host"),
            BundleOpId::new("bridge:host"),
            BundleOpId::new("nft-projection:host"),
            nm_intent_ref,
            BundleOpId::new("hosts:host"),
            Vec::new(),
            Vec::new(),
            expected_generation_id,
            [0; 32],
            false,
        )
    }

    /// Borrow the opaque authorization scope.
    pub const fn scope_id(&self) -> &ScopeId {
        &self.scope_id
    }

    /// Borrow the opaque net-VM identity used for DHCP lease seeding.
    pub const fn dnsmasq_vm_id(&self) -> &VmId {
        &self.dnsmasq_vm_id
    }

    /// Borrow the trusted bridge intent reference.
    pub const fn bridge_intent_ref(&self) -> &BundleOpId {
        &self.bridge_intent_ref
    }

    /// Borrow every trusted bridge intent in this Network fabric.
    pub fn bridge_intent_refs(&self) -> &[BundleOpId] {
        &self.bridge_intent_refs
    }

    /// Borrow the trusted projection intent reference.
    pub const fn projection_intent_ref(&self) -> &BundleOpId {
        &self.projection_intent_ref
    }

    /// Borrow the trusted NetworkManager intent reference.
    pub const fn nm_intent_ref(&self) -> &BundleOpId {
        &self.nm_intent_ref
    }

    /// Borrow the trusted hosts-file intent reference.
    pub const fn hosts_intent_ref(&self) -> &BundleOpId {
        &self.hosts_intent_ref
    }

    /// Borrow route intent references.
    pub fn route_intent_refs(&self) -> &[BundleOpId] {
        &self.route_intent_refs
    }

    /// Borrow sysctl intent references.
    pub fn sysctl_intent_refs(&self) -> &[BundleOpId] {
        &self.sysctl_intent_refs
    }

    /// Borrow the immutable installed bundle generation fence.
    pub const fn expected_generation_id(&self) -> &ResourceBundleGenerationId {
        &self.expected_generation_id
    }

    /// Return the trusted projection digest bytes.
    pub const fn projection_digest(&self) -> [u8; 32] {
        self.projection_digest
    }

    /// Return the site-level east-west acknowledgement.
    pub const fn site_allows_unsafe_east_west(&self) -> bool {
        self.site_allows_unsafe_east_west
    }

    /// Return whether Core admitted requested physical-NIC claims.
    pub const fn host_global_nic_admitted(&self) -> bool {
        self.host_global_nic_admitted
    }

    /// Borrow the immutable Network admission proof, when this is a Network
    /// context rather than the host-wide NetworkManager context.
    pub const fn network_admission(&self) -> Option<&NetworkAdmissionProof> {
        self.network_admission.as_ref()
    }

    fn validate_network_uid(&self, network_uid: &ResourceUid) -> Result<(), NetworkBrokerError> {
        self.validate_network_context()?;
        let Some(proof) = self.network_admission() else {
            return Err(NetworkBrokerError::NetworkAdmissionRequired);
        };
        if proof.key().network_uid() != network_uid {
            return Err(NetworkBrokerError::NetworkAdmissionMismatch);
        }
        Ok(())
    }

    fn validate_network_context(&self) -> Result<(), NetworkBrokerError> {
        let Some(proof) = self.network_admission() else {
            return Err(NetworkBrokerError::NetworkAdmissionRequired);
        };
        if self.expected_generation_id() != proof.key().bundle_generation() {
            return Err(NetworkBrokerError::NetworkAdmissionMismatch);
        }
        let expected_scope = format!(
            "network:{}:{}",
            proof.key().zone_uid().as_str(),
            proof.key().network_uid().as_str()
        );
        if self.scope_id.as_str() != expected_scope {
            return Err(NetworkBrokerError::NetworkAdmissionMismatch);
        }
        if self.nm_intent_ref.as_str() != "nm-unmanaged:host" {
            return Err(NetworkBrokerError::NetworkAdmissionMismatch);
        }
        if network_ref_kind(self.projection_intent_ref.as_str()) != Some("network-firewall")
            || self
                .bridge_intent_refs
                .iter()
                .any(|intent| network_ref_kind(intent.as_str()) != Some("network-bridge"))
            || network_ref_kind(self.hosts_intent_ref.as_str()) != Some("network-hosts")
            || self
                .route_intent_refs
                .iter()
                .any(|intent| network_ref_kind(intent.as_str()) != Some("network-route"))
            || self
                .sysctl_intent_refs
                .iter()
                .any(|intent| network_ref_kind(intent.as_str()) != Some("network-sysctl"))
        {
            return Err(NetworkBrokerError::NetworkAdmissionMismatch);
        }
        let expected_name_token = network_ref_token(self.bridge_intent_ref.as_str())
            .ok_or(NetworkBrokerError::NetworkAdmissionMismatch)?;
        let mut refs = self
            .bridge_intent_refs
            .iter()
            .map(BundleOpId::as_str)
            .chain(std::iter::once(self.projection_intent_ref.as_str()))
            .chain(std::iter::once(self.hosts_intent_ref.as_str()))
            .chain(self.route_intent_refs.iter().map(BundleOpId::as_str))
            .chain(self.sysctl_intent_refs.iter().map(BundleOpId::as_str));
        if refs.any(|intent| {
            !network_ref_matches(intent, proof.key().zone_uid(), proof.key().network_uid())
                || network_ref_token(intent) != Some(expected_name_token)
        }) {
            return Err(NetworkBrokerError::NetworkAdmissionMismatch);
        }
        Ok(())
    }

    /// Return the complete immutable provenance carried by this context.
    pub fn provenance(&self) -> Result<NetworkProvenance, NetworkBrokerError> {
        let proof = self
            .network_admission()
            .ok_or(NetworkBrokerError::NetworkAdmissionRequired)?;
        Ok(NetworkProvenance::new(
            proof.key().zone_uid().clone(),
            proof.key().network_uid().clone(),
            proof.key().network_generation(),
            proof.key().attachment_generation(),
            proof.key().bundle_generation().clone(),
        ))
    }

    /// Resolve one TAP identity from this root-admitted Network context.
    pub fn tap_identity(
        &self,
        vm_id: &VmId,
        role_id: &str,
        attachment_id: &ResourceUid,
    ) -> Result<NetworkTapIdentity, NetworkBrokerError> {
        let proof = self
            .network_admission()
            .ok_or(NetworkBrokerError::NetworkAdmissionRequired)?;
        let key = proof.key();
        self.validate_tap_context(
            &NetworkTapContext {
                zone_uid: key.zone_uid().clone(),
                network_uid: key.network_uid().clone(),
                attachment_id: attachment_id.clone(),
                network_generation: key.network_generation(),
                attachment_generation: key.attachment_generation(),
                bundle_generation: key.bundle_generation().clone(),
                admitted_interface_names: proof.intent().interface_names().to_vec(),
            },
            vm_id,
            role_id,
        )
    }

    /// Verify a wire TAP context against this root-admitted Network proof.
    pub fn validate_tap_context(
        &self,
        tap_context: &NetworkTapContext,
        vm_id: &VmId,
        role_id: &str,
    ) -> Result<NetworkTapIdentity, NetworkBrokerError> {
        self.validate_network_context()?;
        let proof = self
            .network_admission()
            .ok_or(NetworkBrokerError::NetworkAdmissionRequired)?;
        let key = proof.key();
        if tap_context.zone_uid != *key.zone_uid()
            || tap_context.network_uid != *key.network_uid()
            || tap_context.network_generation != key.network_generation()
            || tap_context.attachment_generation != key.attachment_generation()
            || tap_context.bundle_generation != *key.bundle_generation()
            || tap_context.admitted_interface_names.as_slice()
                != proof.intent().interface_names()
        {
            return Err(NetworkBrokerError::NetworkAdmissionMismatch);
        }
        let provenance = NetworkProvenance::new(
            tap_context.zone_uid.clone(),
            tap_context.network_uid.clone(),
            tap_context.network_generation,
            tap_context.attachment_generation,
            tap_context.bundle_generation.clone(),
        );
        let identity = resolve_tap_identity(
            &provenance,
            vm_id,
            role_id,
            &tap_context.attachment_id,
        )?;
        if !tap_context
            .admitted_interface_names
            .iter()
            .any(|ifname| ifname == &identity.bridge_ifname)
            || !tap_context
                .admitted_interface_names
                .iter()
                .any(|ifname| ifname == &identity.tap_ifname)
        {
            return Err(NetworkBrokerError::NetworkAdmissionMismatch);
        }
        Ok(identity)
    }
}

/// Exact bridge/TAP names and opaque reference for one admitted attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkTapIdentity {
    /// Opaque bundle reference that binds the full provenance tuple.
    pub intent_ref: BundleOpId,
    /// UID-derived bridge interface name.
    pub bridge_ifname: IfName,
    /// UID- and attachment-derived TAP interface name.
    pub tap_ifname: IfName,
}

/// Derive one TAP identity from the complete admitted Network provenance.
pub fn resolve_tap_identity(
    provenance: &NetworkProvenance,
    vm_id: &VmId,
    role_id: &str,
    attachment_id: &ResourceUid,
) -> Result<NetworkTapIdentity, NetworkBrokerError> {
    let canonical_role_id = d2b_core::bundle_resolver::canonical_tap_role_id(role_id);
    let is_net_vm = vm_id.as_str()
        == d2b_contracts_resource::v3::derive_network_child_name(
            provenance.network_uid(),
            "vm",
        );
    let (bridge_role, tap_role, attachment) = if is_net_vm {
        (NetworkIfRole::LanBridge, NetworkIfRole::NetVmLanTap, None)
    } else {
        match canonical_role_id {
            "net-vm-lan" => (NetworkIfRole::LanBridge, NetworkIfRole::NetVmLanTap, None),
            "uplink" => (
                NetworkIfRole::UplinkBridge,
                NetworkIfRole::NetVmUplinkTap,
                None,
            ),
            "ch" | "qemu-media" | "workload-lan" | "network-attachment" | "runner-lan" => (
                NetworkIfRole::LanBridge,
                NetworkIfRole::WorkloadGuestTap,
                Some(attachment_id),
            ),
            _ => return Err(NetworkBrokerError::NetworkAdmissionMismatch),
        }
    };
    let bridge_ifname = d2b_contracts_resource::v3::derive_network_ifname(
        provenance.zone_uid(),
        provenance.network_uid(),
        bridge_role,
        None,
    )
    .map_err(|_| NetworkBrokerError::NetworkAdmissionMismatch)?;
    let tap_ifname = d2b_contracts_resource::v3::derive_network_ifname(
        provenance.zone_uid(),
        provenance.network_uid(),
        tap_role,
        attachment,
    )
    .map_err(|_| NetworkBrokerError::NetworkAdmissionMismatch)?;
    let intent_ref = BundleOpId::new(d2b_core::bundle_resolver::intent_id_network_tap(
        provenance.zone_uid(),
        provenance.network_uid(),
        attachment_id,
        provenance.network_generation(),
        provenance.attachment_generation(),
        provenance.bundle_generation(),
        canonical_role_id,
        vm_id.as_str(),
    ));
    Ok(NetworkTapIdentity {
        intent_ref,
        bridge_ifname,
        tap_ifname,
    })
}

fn network_ref_token(intent: &str) -> Option<&str> {
    intent.split(':').nth(3).filter(|value| {
        value.len() == 16 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn network_ref_kind(intent: &str) -> Option<&str> {
    match intent.split(':').next()? {
        "network-bridge"
        | "network-firewall"
        | "network-hosts"
        | "network-route"
        | "network-sysctl" => intent.split(':').next(),
        _ => None,
    }
}

fn network_ref_matches(
    intent: &str,
    zone_uid: &ResourceUid,
    network_uid: &ResourceUid,
) -> bool {
    let fields = intent.split(':').collect::<Vec<_>>();
    matches!(
        fields.first().copied(),
        Some("network-bridge")
            | Some("network-firewall")
            | Some("network-hosts")
            | Some("network-route")
            | Some("network-sysctl")
            | Some("network-marker")
    ) && fields.get(1).and_then(|value| ResourceUid::parse((*value).to_owned()).ok())
        == Some(zone_uid.clone())
        && fields.get(2).and_then(|value| ResourceUid::parse((*value).to_owned()).ok())
            == Some(network_uid.clone())
        && fields.get(3).is_some_and(|value| {
            value.len() == 16 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

impl fmt::Debug for NetworkEffectContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NetworkEffectContext(<redacted>)")
    }
}

/// Broker operations needed by the Network effect port.
pub trait NetworkBroker: Send + Sync {
    /// Ensure the Network bridge exists.
    fn create_bridge(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError>;
    /// Remove the Network bridge after child links are gone.
    fn delete_bridge(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError>;
    /// Apply or remove one ownership-scoped Network projection.
    fn apply_projection(
        &self,
        context: &NetworkEffectContext,
        action: NftablesProjectionAction,
    ) -> Result<FirewallDigest, NetworkBrokerError>;
    /// Apply the trusted NetworkManager unmanaged projection.
    fn apply_nm_unmanaged(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError>;
    /// Apply all trusted route intents for the Network.
    fn apply_routes(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError>;
    /// Remove all trusted route intents for the Network.
    fn remove_routes(&self, _context: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        Ok(())
    }
    /// Apply all trusted IPv6 suppression sysctls for the Network.
    fn apply_sysctls(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError>;
    /// Apply the trusted `/etc/hosts` projection.
    fn update_hosts(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError>;
    /// Seed the Network's DHCP state.
    fn seed_dhcp(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError>;
    /// Delete one generation-fenced persistent TAP.
    fn delete_persistent_tap(
        &self,
        context: &NetworkEffectContext,
        handle: &AttachmentHandle,
        fence: &AttachmentGenerationFence,
    ) -> Result<(), NetworkBrokerError>;
}

/// Production Network effect port backed by a Core broker adapter.
pub struct BrokerNetworkEffectPort<B> {
    broker: B,
    context: NetworkEffectContext,
}

impl<B> BrokerNetworkEffectPort<B> {
    /// Bind a broker implementation to one Core-resolved Network context.
    pub const fn new(broker: B, context: NetworkEffectContext) -> Self {
        Self { broker, context }
    }

    /// Borrow the bound context.
    pub const fn context(&self) -> &NetworkEffectContext {
        &self.context
    }

    /// Consume the adapter and return the broker implementation.
    pub fn into_broker(self) -> B {
        self.broker
    }
}

impl<B: NetworkBroker> NetworkEffectPort for BrokerNetworkEffectPort<B> {
    async fn validate_policy(&self, spec: &NetworkSpec) -> Result<(), NetworkEffectError> {
        self.context
            .validate_network_context()
            .map_err(map_broker_error)?;
        let proof = self
            .context
            .network_admission()
            .ok_or(NetworkEffectError::NetworkAdmissionRequired)?;
        if self.context.expected_generation_id() != proof.key().bundle_generation() {
            return Err(NetworkEffectError::NetworkAdmissionMismatch);
        }
        if spec.isolation().allow_east_west && !self.context.site_allows_unsafe_east_west() {
            return Err(NetworkEffectError::EastWestHostOptInRequired);
        }
        if spec.external_attachment().is_some() && !self.context.host_global_nic_admitted() {
            return Err(NetworkEffectError::ExternalNicAuthorityRequired);
        }
        Ok(())
    }

    async fn create_bridges(&self, network_uid: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.context
            .validate_network_uid(network_uid)
            .map_err(map_broker_error)?;
        self.broker
            .create_bridge(&self.context)
            .map_err(map_broker_error)
    }

    async fn apply_sysctls(&self, network_uid: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.context
            .validate_network_uid(network_uid)
            .map_err(map_broker_error)?;
        self.broker
            .apply_sysctls(&self.context)
            .map_err(map_broker_error)
    }

    async fn apply_host_firewall(
        &self,
        intent: &FirewallIntent,
    ) -> Result<FirewallDigest, NetworkEffectError> {
        self.context
            .validate_network_uid(intent.network_uid())
            .map_err(map_broker_error)?;
        let Some(proof) = self.context.network_admission() else {
            return Err(NetworkEffectError::NetworkAdmissionRequired);
        };
        if intent.zone_uid() != Some(proof.key().zone_uid())
            || intent.network_generation() != Some(proof.key().network_generation())
            || intent.attachment_generation() != Some(proof.key().attachment_generation())
        {
            return Err(NetworkEffectError::NetworkAdmissionMismatch);
        }
        if intent.expected_generation_id() != self.context.expected_generation_id() {
            return Err(NetworkEffectError::StaleConfigurationGeneration);
        }
        self.broker
            .apply_projection(&self.context, NftablesProjectionAction::Apply)
            .map_err(map_broker_error)
    }

    async fn remove_host_firewall(
        &self,
        intent: &FirewallIntent,
    ) -> Result<(), NetworkEffectError> {
        self.context
            .validate_network_uid(intent.network_uid())
            .map_err(map_broker_error)?;
        let Some(proof) = self.context.network_admission() else {
            return Err(NetworkEffectError::NetworkAdmissionRequired);
        };
        if intent.zone_uid() != Some(proof.key().zone_uid())
            || intent.network_generation() != Some(proof.key().network_generation())
            || intent.attachment_generation() != Some(proof.key().attachment_generation())
        {
            return Err(NetworkEffectError::NetworkAdmissionMismatch);
        }
        if intent.expected_generation_id() != self.context.expected_generation_id() {
            return Err(NetworkEffectError::StaleConfigurationGeneration);
        }
        self.broker
            .apply_projection(&self.context, NftablesProjectionAction::Remove)
            .map(|_| ())
            .map_err(map_broker_error)
    }

    async fn apply_nm_unmanaged(&self) -> Result<(), NetworkEffectError> {
        if self.context.network_admission().is_some() {
            self.context
                .validate_network_context()
                .map_err(map_broker_error)?;
        }
        self.broker
            .apply_nm_unmanaged(&self.context)
            .map_err(map_broker_error)
    }

    async fn apply_routes(&self, network_uid: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.context
            .validate_network_uid(network_uid)
            .map_err(map_broker_error)?;
        self.broker
            .apply_routes(&self.context)
            .map_err(map_broker_error)
    }

    async fn remove_routes(&self, network_uid: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.context
            .validate_network_uid(network_uid)
            .map_err(map_broker_error)?;
        self.broker
            .remove_routes(&self.context)
            .map_err(map_broker_error)
    }

    async fn update_hosts(&self, network_uid: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.context
            .validate_network_uid(network_uid)
            .map_err(map_broker_error)?;
        self.broker
            .update_hosts(&self.context)
            .map_err(map_broker_error)
    }

    async fn seed_dhcp(&self, network_uid: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.context
            .validate_network_uid(network_uid)
            .map_err(map_broker_error)?;
        self.broker
            .seed_dhcp(&self.context)
            .map_err(map_broker_error)
    }

    async fn delete_persistent_tap(
        &self,
        handle: &AttachmentHandle,
        fence: &AttachmentGenerationFence,
    ) -> Result<(), NetworkEffectError> {
        self.context
            .validate_network_context()
            .map_err(map_broker_error)?;
        let Some(proof) = self.context.network_admission() else {
            return Err(NetworkEffectError::NetworkAdmissionRequired);
        };
        if handle.opaque_id() != fence.attachment_uid()
            || fence.network_uid() != proof.key().network_uid()
            || fence.network_generation() != proof.key().network_generation()
            || fence.attachment_generation() != proof.key().attachment_generation()
        {
            return Err(NetworkEffectError::NetworkAdmissionMismatch);
        }
        self.broker
            .delete_persistent_tap(&self.context, handle, fence)
            .map_err(map_broker_error)
    }

    async fn delete_bridges(&self, network_uid: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.context
            .validate_network_uid(network_uid)
            .map_err(map_broker_error)?;
        self.broker
            .delete_bridge(&self.context)
            .map_err(map_broker_error)
    }
}

fn map_broker_error(error: NetworkBrokerError) -> NetworkEffectError {
    match error {
        NetworkBrokerError::StaleGeneration => NetworkEffectError::StaleConfigurationGeneration,
        NetworkBrokerError::StaleAttachmentGeneration => {
            NetworkEffectError::StaleAttachmentGeneration
        }
        NetworkBrokerError::ForeignOwnership => NetworkEffectError::ForeignOwnership,
        NetworkBrokerError::EastWestHostOptInRequired => {
            NetworkEffectError::EastWestHostOptInRequired
        }
        NetworkBrokerError::NetworkAdmissionRequired => {
            NetworkEffectError::NetworkAdmissionRequired
        }
        NetworkBrokerError::NetworkAdmissionMismatch => {
            NetworkEffectError::NetworkAdmissionMismatch
        }
        NetworkBrokerError::NetworkAdmissionConflict => {
            NetworkEffectError::NetworkAdmissionConflict
        }
        NetworkBrokerError::NetworkInterfaceCollision => {
            NetworkEffectError::NetworkInterfaceCollision
        }
        NetworkBrokerError::NetworkRouteCollision => NetworkEffectError::NetworkRouteCollision,
        NetworkBrokerError::Transport | NetworkBrokerError::Transient => {
            NetworkEffectError::Transient
        }
        NetworkBrokerError::Rejected => NetworkEffectError::InvalidState,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        sync::{Arc, Mutex},
        task::{Context, Poll, Waker},
    };

    use super::*;
    use d2b_contracts::types::{BundleOpId, VmId};
    use d2b_contracts_broker::broker_wire::NftablesProjectionAction;
    use d2b_contracts_resource::v3::{
    ResourceBundleGenerationId,
    ResourceUid,
    execution_policy::BoundedToken,
    ifname::IfName,
    network::{AttachmentGenerationFence, AttachmentHandle, EgressSpec, ExternalAttachmentMode, ExternalAttachmentSpec, ExternalIpv4Spec, Ipv4Cidr, IsolationSpec, MacvtapMode, NetworkSpec, SharingPolicy},
};
    use crate::controller::{NetworkAdmissionIntent, NetworkAdmissionKey};

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

        fn remove_routes(&self, _: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
            self.record("routes-remove");
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
            _: &NetworkEffectContext,
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
        let bundle_generation = ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let network_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let zone_uid = ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap();
        let attachment_uid =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
        let admission = NetworkAdmissionIntent::new(
            NetworkAdmissionKey::new(
                zone_uid.clone(),
                network_uid.clone(),
                d2b_contracts_resource::v3::ResourceGeneration::new(4).unwrap(),
                d2b_contracts_resource::v3::ResourceGeneration::new(7).unwrap(),
                bundle_generation.clone(),
            ),
            network_spec(false),
            vec![attachment_uid],
        )
        .unwrap()
        .proof();
        NetworkEffectContext::for_network(
            admission,
            VmId::new("net-network-vm"),
            BundleOpId::new(d2b_core::bundle_resolver::intent_id_network_bridge_uids(
                &zone_uid,
                &network_uid,
                "work-net",
                false,
            )),
            BundleOpId::new(d2b_core::bundle_resolver::intent_id_network_projection_uids(
                &zone_uid,
                &network_uid,
                "work-net",
            )),
            BundleOpId::new("nm-unmanaged:host"),
            BundleOpId::new(d2b_core::bundle_resolver::intent_id_network_hosts_uids(
                &zone_uid,
                &network_uid,
                "work-net",
            )),
            vec![BundleOpId::new(
                d2b_core::bundle_resolver::intent_id_network_route_uids(
                    &zone_uid,
                    &network_uid,
                    "work-net",
                    0,
                ),
            )],
            vec![BundleOpId::new(
                d2b_core::bundle_resolver::intent_id_network_sysctl_uids(
                    &zone_uid,
                    &network_uid,
                    "work-net",
                    "lan",
                    "disable-ipv6",
                ),
            )],
            bundle_generation,
            [7; 32],
            site_allows_unsafe_east_west,
        )
    }

    #[test]
    fn provider_tap_identity_is_uid_bound_and_refuses_swapped_attachment() {
        let context = context(false);
        let attachment_id =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
        let identity = context
            .tap_identity(&VmId::new("work-vm"), "ch", &attachment_id)
            .expect("admitted workload TAP identity");
        assert_eq!(
            identity.bridge_ifname,
            crate::ifname::derive_network_ifname(
                context.network_admission().unwrap().key().zone_uid(),
                context.network_admission().unwrap().key().network_uid(),
                NetworkIfRole::LanBridge,
                None,
            )
            .unwrap()
        );
        assert_eq!(
            identity.tap_ifname,
            crate::ifname::derive_network_ifname(
                context.network_admission().unwrap().key().zone_uid(),
                context.network_admission().unwrap().key().network_uid(),
                NetworkIfRole::WorkloadGuestTap,
                Some(&attachment_id),
            )
            .unwrap()
        );
        assert_eq!(
            identity.intent_ref.as_str(),
            d2b_core::bundle_resolver::intent_id_network_tap(
                context.network_admission().unwrap().key().zone_uid(),
                context.network_admission().unwrap().key().network_uid(),
                &attachment_id,
                context
                    .network_admission()
                    .unwrap()
                    .key()
                    .network_generation(),
                context
                    .network_admission()
                    .unwrap()
                    .key()
                    .attachment_generation(),
                context
                    .network_admission()
                    .unwrap()
                    .key()
                    .bundle_generation(),
                "ch",
                "work-vm",
            )
        );
        let swapped_attachment =
            ResourceUid::parse("423e4567-e89b-42d3-a456-426614174003").unwrap();
        assert_eq!(
            context.tap_identity(&VmId::new("work-vm"), "ch", &swapped_attachment),
            Err(NetworkBrokerError::NetworkAdmissionMismatch)
        );
    }

    #[test]
    fn provider_rejects_swapped_tap_context_before_effects() {
        let context = context(false);
        let key = context.network_admission().unwrap().key();
        let valid = NetworkTapContext {
            zone_uid: key.zone_uid().clone(),
            network_uid: key.network_uid().clone(),
            attachment_id: ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap(),
            network_generation: key.network_generation(),
            attachment_generation: key.attachment_generation(),
            bundle_generation: key.bundle_generation().clone(),
            admitted_interface_names: context
                .network_admission()
                .unwrap()
                .intent()
                .interface_names()
                .to_vec(),
        };
        assert!(
            context
                .validate_tap_context(&valid, &VmId::new("work-vm"), "ch")
                .is_ok()
        );
        let cases = [
            NetworkTapContext {
                network_uid: ResourceUid::parse("423e4567-e89b-42d3-a456-426614174003").unwrap(),
                ..valid.clone()
            },
            NetworkTapContext {
                attachment_id: ResourceUid::parse(
                    "523e4567-e89b-42d3-a456-426614174004",
                )
                .unwrap(),
                ..valid.clone()
            },
            NetworkTapContext {
                network_generation: d2b_contracts_resource::v3::ResourceGeneration::new(5)
                    .unwrap(),
                ..valid.clone()
            },
            NetworkTapContext {
                attachment_generation: d2b_contracts_resource::v3::ResourceGeneration::new(8)
                    .unwrap(),
                ..valid.clone()
            },
            NetworkTapContext {
                bundle_generation: ResourceBundleGenerationId::parse(
                    "sha256:1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                )
                .unwrap(),
                ..valid.clone()
            },
        ];
        for swapped in cases {
            assert_eq!(
                context.validate_tap_context(&swapped, &VmId::new("work-vm"), "ch"),
                Err(NetworkBrokerError::NetworkAdmissionMismatch)
            );
        }
    }

    #[test]
    fn tap_context_requires_the_live_admitted_interface_set() {
        let context = context(false);
        let proof = context.network_admission().unwrap();
        let key = proof.key();
        let attachment_id =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
        let valid = NetworkTapContext {
            zone_uid: key.zone_uid().clone(),
            network_uid: key.network_uid().clone(),
            attachment_id,
            network_generation: key.network_generation(),
            attachment_generation: key.attachment_generation(),
            bundle_generation: key.bundle_generation().clone(),
            admitted_interface_names: proof.intent().interface_names().to_vec(),
        };
        assert!(
            context
                .validate_tap_context(&valid, &VmId::new("work-vm"), "ch")
                .is_ok()
        );

        let mut missing_tap = valid.clone();
        missing_tap.admitted_interface_names.pop();
        assert_eq!(
            context.validate_tap_context(&missing_tap, &VmId::new("work-vm"), "ch"),
            Err(NetworkBrokerError::NetworkAdmissionMismatch)
        );

        let mut wrong_generation = valid;
        wrong_generation.network_generation =
            d2b_contracts_resource::v3::ResourceGeneration::new(5).unwrap();
        assert_eq!(
            context.validate_tap_context(&wrong_generation, &VmId::new("work-vm"), "ch"),
            Err(NetworkBrokerError::NetworkAdmissionMismatch)
        );
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

    fn external_network_spec() -> NetworkSpec {
        let external = ExternalAttachmentSpec::new(
            ExternalAttachmentMode::Macvtap,
            IfName::parse("eno1").unwrap(),
            MacvtapMode::Bridge,
            SharingPolicy::Exclusive,
            None,
            ExternalIpv4Spec::default(),
            EgressSpec::default(),
            Vec::new(),
        )
        .unwrap();
        NetworkSpec::new(
            Ipv4Cidr::parse("10.20.0.0/24").unwrap(),
            Ipv4Cidr::parse("192.0.2.0/30").unwrap(),
            None,
            false,
            IsolationSpec::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Some(external),
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
    fn broker_port_requires_host_global_nic_admission_before_dispatch() {
        let broker = RecordingBroker::default();
        let port = BrokerNetworkEffectPort::new(broker.clone(), context(true));
        let error = block_on(port.validate_policy(&external_network_spec())).unwrap_err();
        assert_eq!(error.code(), "external-nic-authority-required");
        assert!(broker.events().is_empty());

        let admitted = BrokerNetworkEffectPort::new(
            broker.clone(),
            context(true).with_host_global_nic_admission(),
        );
        assert!(block_on(admitted.validate_policy(&external_network_spec())).is_ok());
        assert!(broker.events().is_empty());
    }

    #[test]
    fn broker_port_rejects_swapped_network_intent_refs() {
        let broker = RecordingBroker::default();
        let mut swapped = context(true);
        swapped.projection_intent_ref = BundleOpId::new(
            "network-firewall:323e4567-e89b-42d3-a456-426614174002:423e4567-e89b-42d3-a456-426614174003:other-net",
        );
        let port = BrokerNetworkEffectPort::new(broker.clone(), swapped);
        assert_eq!(
            block_on(port.validate_policy(&network_spec(false))),
            Err(NetworkEffectError::NetworkAdmissionMismatch)
        );
        assert!(broker.events().is_empty());
    }

    #[test]
    fn broker_port_rejects_swapped_network_name_hints() {
        let broker = RecordingBroker::default();
        let mut swapped = context(true);
        swapped.route_intent_refs[0] = BundleOpId::new(
            "network-route:323e4567-e89b-42d3-a456-426614174002:123e4567-e89b-42d3-a456-426614174000:aaaaaaaaaaaaaaaa:0",
        );
        let port = BrokerNetworkEffectPort::new(broker.clone(), swapped);
        assert_eq!(
            block_on(port.validate_policy(&network_spec(false))),
            Err(NetworkEffectError::NetworkAdmissionMismatch)
        );
        assert!(broker.events().is_empty());
    }

    #[test]
    fn broker_port_refuses_swapped_bridge_and_stale_bundle_before_effects() {
        let broker = RecordingBroker::default();
        let mut swapped_bridge = context(true);
        swapped_bridge.bridge_intent_ref = BundleOpId::new(
            "network-bridge:323e4567-e89b-42d3-a456-426614174002:423e4567-e89b-42d3-a456-426614174003:aaaaaaaaaaaaaaaa:lan",
        );
        let port = BrokerNetworkEffectPort::new(broker.clone(), swapped_bridge);
        let network_uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        assert_eq!(
            block_on(port.create_bridges(&network_uid)),
            Err(NetworkEffectError::NetworkAdmissionMismatch)
        );
        assert!(broker.events().is_empty());

        let mut stale_bundle = context(true);
        stale_bundle.expected_generation_id = ResourceBundleGenerationId::parse(
            "sha256:1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let port = BrokerNetworkEffectPort::new(broker.clone(), stale_bundle);
        assert_eq!(
            block_on(port.apply_routes(&network_uid)),
            Err(NetworkEffectError::NetworkAdmissionMismatch)
        );
        assert!(broker.events().is_empty());
    }

    #[test]
    fn broker_port_refuses_mixed_network_intent_kinds_before_effects() {
        let broker = RecordingBroker::default();
        let mut mixed = context(true);
        mixed.route_intent_refs[0] = mixed.projection_intent_ref.clone();
        let port = BrokerNetworkEffectPort::new(broker.clone(), mixed);
        let network_uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        assert_eq!(
            block_on(port.apply_routes(&network_uid)),
            Err(NetworkEffectError::NetworkAdmissionMismatch)
        );
        assert!(broker.events().is_empty());
    }

    #[test]
    fn broker_port_refuses_swapped_sysctl_hosts_and_firewall_refs_before_effects() {
        let network_uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let foreign_network =
            ResourceUid::parse("423e4567-e89b-42d3-a456-426614174003").unwrap();

        let broker = RecordingBroker::default();
        let mut swapped_sysctl = context(true);
        swapped_sysctl.sysctl_intent_refs[0] = BundleOpId::new(format!(
            "network-sysctl:323e4567-e89b-42d3-a456-426614174002:{}:aaaaaaaaaaaaaaaa:lan:disable-ipv6",
            foreign_network.as_str()
        ));
        let port = BrokerNetworkEffectPort::new(broker.clone(), swapped_sysctl);
        assert_eq!(
            block_on(port.apply_sysctls(&network_uid)),
            Err(NetworkEffectError::NetworkAdmissionMismatch)
        );
        assert!(broker.events().is_empty());

        let broker = RecordingBroker::default();
        let mut swapped_hosts = context(true);
        swapped_hosts.hosts_intent_ref = BundleOpId::new(format!(
            "network-hosts:323e4567-e89b-42d3-a456-426614174002:{}:aaaaaaaaaaaaaaaa",
            foreign_network.as_str()
        ));
        let port = BrokerNetworkEffectPort::new(broker.clone(), swapped_hosts);
        assert_eq!(
            block_on(port.update_hosts(&network_uid)),
            Err(NetworkEffectError::NetworkAdmissionMismatch)
        );
        assert!(broker.events().is_empty());

        let broker = RecordingBroker::default();
        let mut swapped_firewall = context(true);
        swapped_firewall.projection_intent_ref = BundleOpId::new(format!(
            "network-firewall:323e4567-e89b-42d3-a456-426614174002:{}:aaaaaaaaaaaaaaaa",
            foreign_network.as_str()
        ));
        let port = BrokerNetworkEffectPort::new(broker.clone(), swapped_firewall);
        assert_eq!(
            block_on(port.validate_policy(&network_spec(false))),
            Err(NetworkEffectError::NetworkAdmissionMismatch)
        );
        assert!(broker.events().is_empty());
    }

    #[test]
    fn broker_port_rejects_unfenced_firewall_intent() {
        let broker = RecordingBroker::default();
        let port = BrokerNetworkEffectPort::new(broker.clone(), context(true));
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let generation = ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        assert_eq!(
            block_on(port.apply_host_firewall(&FirewallIntent::new(uid, generation))),
            Err(NetworkEffectError::NetworkAdmissionMismatch)
        );
        assert!(broker.events().is_empty());
    }

    #[test]
    fn broker_port_maps_host_effects_to_typed_broker_calls() {
        let broker = RecordingBroker::default();
        let bound_context = context(true);
        let admission = bound_context.network_admission().unwrap().clone();
        let port = BrokerNetworkEffectPort::new(broker.clone(), bound_context);
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let generation = ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let firewall = FirewallIntent::from_admission(&admission, generation);
        let attachment_uid = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
        let handle = AttachmentHandle::new(
            attachment_uid.clone(),
            AttachmentGenerationFence::new(
                uid.clone(),
                d2b_contracts_resource::v3::ResourceGeneration::new(4).unwrap(),
                attachment_uid,
                d2b_contracts_resource::v3::ResourceGeneration::new(7).unwrap(),
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
        block_on(port.remove_routes(&uid)).unwrap();
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
                "routes-remove",
                "delete-bridge",
            ]
        );
    }
}
