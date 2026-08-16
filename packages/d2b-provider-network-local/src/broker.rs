//! Core-owned broker effect adapter for Network reconciliation.
//!
//! The Provider only sees this typed boundary. The concrete Core composition
//! supplies a broker implementation that resolves every opaque intent against
//! its trusted bundle before dispatching a wire operation.

use std::fmt;

use d2b_contracts::{
    broker_wire::NftablesProjectionAction,
    types::{BundleOpId, ScopeId, VmId},
    v3::{
        ResourceBundleGenerationId, ResourceUid,
        network::{AttachmentGenerationFence, AttachmentHandle, NetworkSpec},
    },
};

use crate::controller::{FirewallDigest, FirewallIntent, NetworkEffectError, NetworkEffectPort};

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
    projection_intent_ref: BundleOpId,
    nm_intent_ref: BundleOpId,
    hosts_intent_ref: BundleOpId,
    route_intent_refs: Vec<BundleOpId>,
    sysctl_intent_refs: Vec<BundleOpId>,
    expected_generation_id: ResourceBundleGenerationId,
    projection_digest: [u8; 32],
    site_allows_unsafe_east_west: bool,
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
        Self {
            scope_id,
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
        }
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
        if spec.isolation().allow_east_west && !self.context.site_allows_unsafe_east_west() {
            return Err(NetworkEffectError::EastWestHostOptInRequired);
        }
        Ok(())
    }

    async fn create_bridges(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.broker
            .create_bridge(&self.context)
            .map_err(map_broker_error)
    }

    async fn apply_sysctls(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.broker
            .apply_sysctls(&self.context)
            .map_err(map_broker_error)
    }

    async fn apply_host_firewall(
        &self,
        intent: &FirewallIntent,
    ) -> Result<FirewallDigest, NetworkEffectError> {
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
        if intent.expected_generation_id() != self.context.expected_generation_id() {
            return Err(NetworkEffectError::StaleConfigurationGeneration);
        }
        self.broker
            .apply_projection(&self.context, NftablesProjectionAction::Remove)
            .map(|_| ())
            .map_err(map_broker_error)
    }

    async fn apply_nm_unmanaged(&self) -> Result<(), NetworkEffectError> {
        self.broker
            .apply_nm_unmanaged(&self.context)
            .map_err(map_broker_error)
    }

    async fn apply_routes(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.broker
            .apply_routes(&self.context)
            .map_err(map_broker_error)
    }

    async fn remove_routes(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.broker
            .remove_routes(&self.context)
            .map_err(map_broker_error)
    }

    async fn update_hosts(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.broker
            .update_hosts(&self.context)
            .map_err(map_broker_error)
    }

    async fn seed_dhcp(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
        self.broker
            .seed_dhcp(&self.context)
            .map_err(map_broker_error)
    }

    async fn delete_persistent_tap(
        &self,
        handle: &AttachmentHandle,
        fence: &AttachmentGenerationFence,
    ) -> Result<(), NetworkEffectError> {
        self.broker
            .delete_persistent_tap(handle, fence)
            .map_err(map_broker_error)
    }

    async fn delete_bridges(&self, _: &ResourceUid) -> Result<(), NetworkEffectError> {
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
        NetworkBrokerError::Transport
        | NetworkBrokerError::Rejected
        | NetworkBrokerError::Transient => NetworkEffectError::Transient,
    }
}
