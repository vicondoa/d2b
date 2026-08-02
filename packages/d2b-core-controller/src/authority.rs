//! Zone and Host-global authority admission for scarce resources.
//!
//! Core resolves authored selectors through trusted inventory, derives opaque
//! keys, and admits a claim before any host or VMM effect.  The key, resolved
//! inventory identity, and owner proof have no serialization or display
//! surface.

use std::collections::BTreeMap;

use d2b_contracts::v3::{
    ResourceGeneration, ResourceRef, ResourceUid, UpdateState,
    ifname::IfName,
    network::{
        ExternalNicAdmissionError, ExternalNicAuthorityStatus, ExternalNicClaim, MacvtapMode,
        SharingPolicy, admit_external_nic_claims,
    },
    process::PortProtocol,
    resource_schema::canonical_digest,
};

#[path = "emergency_policy.rs"]
pub mod emergency_policy;
#[path = "quota.rs"]
pub mod quota;

/// Domain tag for the Core-derived external physical-NIC identity.
pub const EXTERNAL_PHYSICAL_NIC_IDENTITY_DOMAIN: &str = "external-physical-nic/v1";
/// Authority class used in the Host-global index.
pub const EXTERNAL_PHYSICAL_NIC_AUTHORITY_CLASS: &str = "external-physical-nic";
/// Domain tag for Core-derived physical USB backing identities.
pub const PHYSICAL_USB_BACKING_IDENTITY_DOMAIN: &str = "physical-usb-backing/v1";
/// Domain tag for Core-derived USBIP relay endpoint identities.
pub const USBIP_NETWORK_RELAY_IDENTITY_DOMAIN: &str = "usbip-network-relay/v1";
const MAX_RESOLVED_NIC_IDENTITY_BYTES: usize = 256;

/// One stable physical-NIC identity resolved from trusted Host inventory.
///
/// This is not an authored interface selector and cannot be serialized into a
/// resource. Core derives the authority key from these private bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedExternalNicIdentity(Vec<u8>);

impl ResolvedExternalNicIdentity {
    /// Record a stable identity returned by the trusted inventory adapter.
    pub fn from_trusted_inventory(bytes: impl Into<Vec<u8>>) -> Result<Self, AuthorityError> {
        let bytes = bytes.into();
        if bytes.is_empty() || bytes.len() > MAX_RESOLVED_NIC_IDENTITY_BYTES {
            return Err(AuthorityError::InvalidTrustedInventoryIdentity);
        }
        Ok(Self(bytes))
    }
}

impl core::fmt::Debug for ResolvedExternalNicIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ResolvedExternalNicIdentity(<redacted>)")
    }
}

/// Trusted Host inventory used to resolve authored interface selectors.
#[derive(Default)]
pub struct TrustedExternalNicInventory {
    entries: BTreeMap<IfName, ResolvedExternalNicIdentity>,
}

impl TrustedExternalNicInventory {
    /// Add one resolver-owned inventory row.
    pub fn insert(
        &mut self,
        selector: IfName,
        identity: ResolvedExternalNicIdentity,
    ) -> Result<(), AuthorityError> {
        if self.entries.insert(selector, identity).is_some() {
            return Err(AuthorityError::DuplicateTrustedInventorySelector);
        }
        Ok(())
    }

    /// Resolve an authored selector without exposing the derived authority key.
    pub fn resolve(
        &self,
        selector: &IfName,
    ) -> Result<ResolvedExternalNicIdentity, AuthorityError> {
        self.entries
            .get(selector)
            .cloned()
            .ok_or(AuthorityError::TrustedInventorySelectorNotFound)
    }
}

impl core::fmt::Debug for TrustedExternalNicInventory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TrustedExternalNicInventory")
            .field("entry_count", &self.entries.len())
            .finish()
    }
}

/// Exact resource identity used to adopt or release one authority holder.
#[derive(Clone, PartialEq, Eq)]
pub struct ExternalNicOwnerProof {
    resource_uid: ResourceUid,
    generation: ResourceGeneration,
}

impl ExternalNicOwnerProof {
    /// Bind an owner proof to an exact resource identity and generation.
    pub const fn new(resource_uid: ResourceUid, generation: ResourceGeneration) -> Self {
        Self {
            resource_uid,
            generation,
        }
    }
}

impl core::fmt::Debug for ExternalNicOwnerProof {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ExternalNicOwnerProof(<redacted>)")
    }
}

/// Complete pre-effect request for one external physical-NIC claim.
pub struct ExternalNicClaimRequest {
    host_uid: ResourceUid,
    identity: ResolvedExternalNicIdentity,
    claim: ExternalNicClaim,
    owner_proof: ExternalNicOwnerProof,
    signed_max_holders: usize,
}

impl ExternalNicClaimRequest {
    /// Construct a request from a trusted inventory result and signed quota.
    pub fn new(
        host_uid: ResourceUid,
        identity: ResolvedExternalNicIdentity,
        claim: ExternalNicClaim,
        owner_proof: ExternalNicOwnerProof,
        signed_max_holders: usize,
    ) -> Result<Self, AuthorityError> {
        if signed_max_holders == 0 || signed_max_holders > u32::MAX as usize {
            return Err(AuthorityError::InvalidSignedHolderLimit);
        }
        Ok(Self {
            host_uid,
            identity,
            claim,
            owner_proof,
            signed_max_holders,
        })
    }
}

impl core::fmt::Debug for ExternalNicClaimRequest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ExternalNicClaimRequest(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ExternalNicAuthorityKey {
    host_uid: ResourceUid,
    opaque_digest: String,
}

impl ExternalNicAuthorityKey {
    fn derive(host_uid: ResourceUid, identity: &ResolvedExternalNicIdentity) -> Self {
        let mut framed = Vec::with_capacity(8 + identity.0.len());
        framed.extend_from_slice(&(identity.0.len() as u64).to_be_bytes());
        framed.extend_from_slice(&identity.0);
        Self {
            host_uid,
            opaque_digest: canonical_digest(EXTERNAL_PHYSICAL_NIC_IDENTITY_DOMAIN, &framed),
        }
    }
}

impl core::fmt::Debug for ExternalNicAuthorityKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ExternalNicAuthorityKey(<redacted>)")
    }
}

#[derive(Clone)]
struct Holder {
    claim: ExternalNicClaim,
    owner_proof: ExternalNicOwnerProof,
}

struct AuthorityEntry {
    holders: Vec<Holder>,
    signed_max_holders: usize,
}

/// Proof that Core admitted a Host-global claim before an external effect.
///
/// The lease is deliberately non-serializable and does not reveal its key or
/// owner proof.
pub struct ExternalNicLease {
    key: ExternalNicAuthorityKey,
    owner_proof: ExternalNicOwnerProof,
}

impl core::fmt::Debug for ExternalNicLease {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ExternalNicLease(<redacted>)")
    }
}

/// Closed effect result retained beside an admitted lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalNicEffectOutcome {
    /// The effect completed and observation confirmed it.
    Confirmed,
    /// The effect may be retried while the authority remains held.
    RetryableFailure,
    /// The effect failed terminally while the authority remains held for drain.
    TerminalFailure,
}

/// Result of gating one host effect on authority admission.
pub struct ExternalNicEffectGate {
    lease: ExternalNicLease,
    outcome: ExternalNicEffectOutcome,
}

impl ExternalNicEffectGate {
    /// Consume the gate into its retained authority lease.
    pub fn into_lease(self) -> ExternalNicLease {
        self.lease
    }

    /// Return the closed effect outcome.
    pub const fn outcome(&self) -> ExternalNicEffectOutcome {
        self.outcome
    }
}

impl core::fmt::Debug for ExternalNicEffectGate {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExternalNicEffectGate")
            .field("lease", &self.lease)
            .field("outcome", &self.outcome)
            .finish()
    }
}

/// Closed result of attempting to close old macvtap and VMM ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalNicCloseOutcome {
    /// Every old holder and FD is confirmed closed.
    Confirmed,
    /// Closure is incomplete, so the authority must remain held.
    RetryableFailure,
}

/// Restart-adoption result for one exact owner proof.
pub enum ExternalNicAdoption {
    /// Exactly one recovered owner matched the indexed claim.
    Adopted(ExternalNicLease),
    /// No matching indexed and observed owner exists.
    Missing,
    /// Recovery found more than one matching owner and effects stay quarantined.
    QuarantinedAmbiguous,
}

impl core::fmt::Debug for ExternalNicAdoption {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Adopted(_) => f.write_str("ExternalNicAdoption::Adopted(<redacted>)"),
            Self::Missing => f.write_str("ExternalNicAdoption::Missing"),
            Self::QuarantinedAmbiguous => f.write_str("ExternalNicAdoption::QuarantinedAmbiguous"),
        }
    }
}

/// Closed, identity-free authority failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityError {
    /// Trusted inventory returned an absent or oversized stable identity.
    InvalidTrustedInventoryIdentity,
    /// The trusted inventory contains the same selector twice.
    DuplicateTrustedInventorySelector,
    /// The authored selector did not resolve in trusted inventory.
    TrustedInventorySelectorNotFound,
    /// The signed quota is zero or cannot be represented in bounded status.
    InvalidSignedHolderLimit,
    /// Claim compatibility or isolation admission failed.
    Admission(ExternalNicAdmissionError),
    /// A lease no longer names an indexed claim.
    UnknownClaim,
    /// A lease does not match the indexed owner proof.
    OwnerProofMismatch,
    /// Macvtap or VMM ownership was not confirmed closed.
    AttachmentCloseUnconfirmed,
    /// A Core-derived generic authority key is empty or zero.
    InvalidAuthorityKey,
    /// A generic authority holder limit is outside bounded status range.
    InvalidAuthorityHolderLimit,
    /// A generic authority request does not match its closed class.
    InvalidAuthorityRequest,
    /// A generic lease does not match its indexed owner proof.
    AuthorityOwnerProofMismatch,
    /// A request changes the arbitration mode of an incumbent authority.
    AuthorityArbitrationConflict,
    /// A bounded shared authority has no remaining holder slot.
    AuthorityCapacityExceeded,
    /// A generic authority is not present in the index.
    UnknownAuthority,
    /// A generic effect was not confirmed closed.
    AuthorityCloseUnconfirmed,
    /// An incumbent owns the exact authority key.
    DuplicateConflict,
    /// A second USB or security-key claimant owns one physical USB backing.
    PhysicalUsbBackingConflict,
    /// A second owner attempted one Network USBIP relay Endpoint.
    UsbipNetworkRelayAuthorityConflict,
    /// A vsock CID is outside the nonzero allocation range.
    InvalidVsockCid,
    /// A fixed listener port is zero.
    InvalidListenerPort,
}

impl AuthorityError {
    /// Return the stable, identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidTrustedInventoryIdentity => "invalid-trusted-inventory-identity",
            Self::DuplicateTrustedInventorySelector => "duplicate-trusted-inventory-selector",
            Self::TrustedInventorySelectorNotFound => "trusted-inventory-selector-not-found",
            Self::InvalidSignedHolderLimit => "invalid-signed-holder-limit",
            Self::Admission(reason) => reason.code(),
            Self::UnknownClaim => "external-physical-nic-claim-missing",
            Self::OwnerProofMismatch => "external-physical-nic-owner-proof-mismatch",
            Self::AttachmentCloseUnconfirmed => "external-physical-nic-close-unconfirmed",
            Self::InvalidAuthorityKey => "authority-key-invalid",
            Self::InvalidAuthorityHolderLimit => "authority-holder-limit-invalid",
            Self::InvalidAuthorityRequest => "authority-request-invalid",
            Self::AuthorityOwnerProofMismatch => "authority-owner-proof-mismatch",
            Self::AuthorityArbitrationConflict => "authority-arbitration-conflict",
            Self::AuthorityCapacityExceeded => "authority-capacity-exceeded",
            Self::UnknownAuthority => "authority-missing",
            Self::AuthorityCloseUnconfirmed => "authority-close-unconfirmed",
            Self::DuplicateConflict => "duplicateConflict",
            Self::PhysicalUsbBackingConflict => "physical-usb-backing-conflict",
            Self::UsbipNetworkRelayAuthorityConflict => "usbip-network-relay-authority-conflict",
            Self::InvalidVsockCid => "vsock-cid-invalid",
            Self::InvalidListenerPort => "listener-port-invalid",
        }
    }
}

impl core::fmt::Display for AuthorityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for AuthorityError {}

impl From<ExternalNicAdmissionError> for AuthorityError {
    fn from(value: ExternalNicAdmissionError) -> Self {
        Self::Admission(value)
    }
}

fn conflict_for_class(class: AuthorityClass) -> AuthorityError {
    match class {
        AuthorityClass::PhysicalUsbBacking => AuthorityError::PhysicalUsbBackingConflict,
        AuthorityClass::UsbipNetworkRelay => AuthorityError::UsbipNetworkRelayAuthorityConflict,
        _ => AuthorityError::DuplicateConflict,
    }
}

/// Opaque identity produced by a trusted Core inventory or allocator.
///
/// The authority index accepts this value only at a Core adapter boundary.
/// Provider code may compare it, but it cannot derive an authority key from a
/// host path, selector, bus id, or other implementation detail.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthorityDigest([u8; 32]);

impl AuthorityDigest {
    /// Construct a digest returned by the trusted Core adapter.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Return whether this is the forbidden all-zero identity.
    pub fn is_zero(self) -> bool {
        self.0 == [0; 32]
    }

    fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl core::fmt::Debug for AuthorityDigest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuthorityDigest(<redacted>)")
    }
}

/// Scope of a Core-owned authority key.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthorityScope {
    /// A key shared by all resources in one Zone only.
    Zone(ResourceUid),
    /// A key shared by every Zone on one Host.
    Host(ResourceUid),
}

impl core::fmt::Debug for AuthorityScope {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Zone(_) => "AuthorityScope::Zone(<redacted>)",
            Self::Host(_) => "AuthorityScope::Host(<redacted>)",
        })
    }
}

/// Closed authority classes admitted by the core index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthorityClass {
    /// Provider controller cardinality in one Zone.
    Provider,
    /// Quota scope authority in one Zone.
    Quota,
    /// EmergencyPolicy scope authority in one Zone.
    EmergencyPolicy,
    /// Whole-GPU or VFIO authority.
    GpuFullDevice,
    /// Render-node-only GPU authority.
    GpuRenderNode,
    /// Per-Guest swtpm state and tamper marker.
    GuestSwtpm,
    /// Physical host TPM authority.
    PhysicalTpm,
    /// Core-derived physical USB backing.
    PhysicalUsbBacking,
    /// Host-global usbip kernel module.
    UsbipHost,
    /// Per-Network USBIP relay Endpoint.
    UsbipNetworkRelay,
    /// Host-shared `/dev/kvm` grant authority.
    Kvm,
    /// Host-shared `/dev/vhost-vsock` grant authority.
    VhostVsock,
    /// Globally unique vsock CID.
    VsockCid,
    /// Fixed host listener port Endpoint.
    FixedListenerPort,
    /// Host Nix store authority.
    HostStore,
    /// Per-Guest store-view writer.
    GuestStoreViewWriter,
    /// Zone-local Network TAP or bridge.
    NetworkTapBridge,
}

impl AuthorityClass {
    /// Return the stable internal class label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider-controller",
            Self::Quota => "quota-scope",
            Self::EmergencyPolicy => "emergency-policy-scope",
            Self::GpuFullDevice => "gpu-full-device",
            Self::GpuRenderNode => "gpu-render-node",
            Self::GuestSwtpm => "guest-swtpm",
            Self::PhysicalTpm => "physical-tpm",
            Self::PhysicalUsbBacking => "physical-usb-backing",
            Self::UsbipHost => "usbip-host",
            Self::UsbipNetworkRelay => "usbip-network-relay",
            Self::Kvm => "kvm",
            Self::VhostVsock => "vhost-vsock",
            Self::VsockCid => "vsock-cid",
            Self::FixedListenerPort => "fixed-listener-port",
            Self::HostStore => "host-store",
            Self::GuestStoreViewWriter => "guest-store-view-writer",
            Self::NetworkTapBridge => "network-tap-bridge",
        }
    }
}

/// Arbitration policy for one authority class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthorityArbitration {
    /// Only one owner may hold this key.
    Exclusive,
    /// Multiple bounded holders may share this key.
    Shared,
    /// Multiple consumers use one multiplexed owner.
    Multiplexed,
}

/// Provider controller cardinality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderCardinality {
    /// The Provider controller is required to exist once.
    ExactlyOne,
    /// The optional Provider may exist zero or one time.
    AtMostOne,
}

/// Exact resource generation proof for an authority owner.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorityOwnerProof {
    resource_uid: ResourceUid,
    generation: ResourceGeneration,
}

impl AuthorityOwnerProof {
    /// Bind an authority owner to one exact resource generation.
    pub const fn new(resource_uid: ResourceUid, generation: ResourceGeneration) -> Self {
        Self {
            resource_uid,
            generation,
        }
    }

    /// Compare two owner proofs without exposing their identities.
    pub fn matches(&self, other: &Self) -> bool {
        self == other
    }
}

impl core::fmt::Debug for AuthorityOwnerProof {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuthorityOwnerProof(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AuthorityKey {
    scope: AuthorityScope,
    class: AuthorityClass,
    opaque_digest: String,
}

impl AuthorityKey {
    fn new(
        scope: AuthorityScope,
        class: AuthorityClass,
        opaque_digest: String,
    ) -> Result<Self, AuthorityError> {
        if opaque_digest.is_empty() {
            return Err(AuthorityError::InvalidAuthorityKey);
        }
        Ok(Self {
            scope,
            class,
            opaque_digest,
        })
    }
}

impl core::fmt::Debug for AuthorityKey {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuthorityKey(<redacted>)")
    }
}

fn framed_digest(domain: &str, parts: &[&[u8]]) -> String {
    let size = parts
        .iter()
        .map(|part| core::mem::size_of::<u64>() + part.len())
        .sum();
    let mut framed = Vec::with_capacity(size);
    for part in parts {
        framed.extend_from_slice(&(part.len() as u64).to_be_bytes());
        framed.extend_from_slice(part);
    }
    canonical_digest(domain, &framed)
}

fn uid_digest(domain: &str, uid: &ResourceUid) -> String {
    let rendered = uid.to_canonical_string();
    framed_digest(domain, &[rendered.as_bytes()])
}

fn class_digest(class: AuthorityClass) -> String {
    framed_digest(class.as_str(), &[class.as_str().as_bytes()])
}

fn port_protocol_tag(protocol: PortProtocol) -> u8 {
    match protocol {
        PortProtocol::Tcp => 1,
        PortProtocol::Udp => 2,
        PortProtocol::Sctp => 3,
    }
}

/// A typed request for one Core-owned authority.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorityRequest {
    key: AuthorityKey,
    owner_proof: AuthorityOwnerProof,
    arbitration: AuthorityArbitration,
    max_holders: usize,
    provider_cardinality: Option<ProviderCardinality>,
    dependent_guest: Option<ResourceUid>,
}

impl AuthorityRequest {
    /// Build a Provider controller cardinality claim.
    pub fn provider(
        zone_uid: ResourceUid,
        provider_ref: ResourceRef,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        let cardinality = if provider_ref.to_canonical_string() == "Provider/observability-otel" {
            ProviderCardinality::AtMostOne
        } else {
            ProviderCardinality::ExactlyOne
        };
        Self::provider_with_cardinality(zone_uid, provider_ref, cardinality, owner_proof)
    }

    /// Build a Provider claim with an explicit closed cardinality.
    pub fn provider_with_cardinality(
        zone_uid: ResourceUid,
        provider_ref: ResourceRef,
        provider_cardinality: ProviderCardinality,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(AuthorityError::InvalidAuthorityRequest);
        }
        let rendered = provider_ref.to_canonical_string();
        Self::new(
            AuthorityScope::Zone(zone_uid),
            AuthorityClass::Provider,
            framed_digest("provider-cardinality/v1", &[rendered.as_bytes()]),
            AuthorityArbitration::Exclusive,
            1,
            Some(provider_cardinality),
            owner_proof,
            None,
        )
    }

    /// Build the one Quota scope claim for a Zone.
    pub fn quota(
        zone_uid: ResourceUid,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        let digest = uid_digest("quota-scope/v1", &zone_uid);
        Self::new(
            AuthorityScope::Zone(zone_uid),
            AuthorityClass::Quota,
            digest,
            AuthorityArbitration::Exclusive,
            1,
            None,
            owner_proof,
            None,
        )
    }

    /// Build the one EmergencyPolicy scope claim for a Zone.
    pub fn emergency_policy(
        zone_uid: ResourceUid,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        let digest = uid_digest("emergency-policy-scope/v1", &zone_uid);
        Self::new(
            AuthorityScope::Zone(zone_uid),
            AuthorityClass::EmergencyPolicy,
            digest,
            AuthorityArbitration::Exclusive,
            1,
            None,
            owner_proof,
            None,
        )
    }

    /// Build an exclusive full-device GPU claim.
    pub fn gpu_full_device(
        host_uid: ResourceUid,
        backing: AuthorityDigest,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        Self::hardware(
            host_uid,
            AuthorityClass::GpuFullDevice,
            backing,
            AuthorityArbitration::Exclusive,
            1,
            owner_proof,
            None,
        )
    }

    /// Build a bounded shared render-node claim.
    pub fn gpu_render_node(
        host_uid: ResourceUid,
        backing: AuthorityDigest,
        max_holders: usize,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        Self::hardware(
            host_uid,
            AuthorityClass::GpuRenderNode,
            backing,
            AuthorityArbitration::Shared,
            max_holders,
            owner_proof,
            None,
        )
    }

    /// Build the exclusive per-Guest swtpm state claim.
    pub fn guest_swtpm(
        host_uid: ResourceUid,
        guest_uid: ResourceUid,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        let digest = uid_digest("guest-swtpm/v1", &guest_uid);
        Self::new(
            AuthorityScope::Host(host_uid),
            AuthorityClass::GuestSwtpm,
            digest,
            AuthorityArbitration::Exclusive,
            1,
            None,
            owner_proof,
            Some(guest_uid),
        )
    }

    /// Build the exclusive physical TPM claim.
    pub fn physical_tpm(
        host_uid: ResourceUid,
        backing: AuthorityDigest,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        Self::hardware(
            host_uid,
            AuthorityClass::PhysicalTpm,
            backing,
            AuthorityArbitration::Exclusive,
            1,
            owner_proof,
            None,
        )
    }

    /// Build the Core-derived physical USB backing claim.
    pub fn physical_usb_backing(
        host_uid: ResourceUid,
        backing: AuthorityDigest,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        Self::hardware(
            host_uid,
            AuthorityClass::PhysicalUsbBacking,
            backing,
            AuthorityArbitration::Exclusive,
            1,
            owner_proof,
            None,
        )
    }

    /// Build the host-global usbip kernel-module claim.
    pub fn usbip_host_module(
        host_uid: ResourceUid,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        Self::new(
            AuthorityScope::Host(host_uid),
            AuthorityClass::UsbipHost,
            class_digest(AuthorityClass::UsbipHost),
            AuthorityArbitration::Exclusive,
            1,
            None,
            owner_proof,
            None,
        )
    }

    /// Build a Core-derived per-Network USBIP relay Endpoint claim.
    pub fn usbip_network_relay(
        host_uid: ResourceUid,
        network_uid: ResourceUid,
        signed_policy_port_digest: AuthorityDigest,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        if signed_policy_port_digest.is_zero() {
            return Err(AuthorityError::InvalidAuthorityKey);
        }
        let network = network_uid.to_canonical_string();
        let policy = signed_policy_port_digest.as_bytes();
        let digest = framed_digest(
            USBIP_NETWORK_RELAY_IDENTITY_DOMAIN,
            &[network.as_bytes(), &policy],
        );
        Self::new(
            AuthorityScope::Host(host_uid),
            AuthorityClass::UsbipNetworkRelay,
            digest,
            AuthorityArbitration::Multiplexed,
            1,
            None,
            owner_proof,
            None,
        )
    }

    /// Build the host-shared `/dev/kvm` grant authority claim.
    pub fn kvm(
        host_uid: ResourceUid,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        Self::shared_host_grant(AuthorityClass::Kvm, host_uid, owner_proof)
    }

    /// Build the host-shared `/dev/vhost-vsock` grant authority claim.
    pub fn vhost_vsock(
        host_uid: ResourceUid,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        Self::shared_host_grant(AuthorityClass::VhostVsock, host_uid, owner_proof)
    }

    /// Build a globally unique vsock CID claim.
    pub fn vsock_cid(
        host_uid: ResourceUid,
        cid: u32,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        if cid == 0 {
            return Err(AuthorityError::InvalidVsockCid);
        }
        let digest = framed_digest("vsock-cid/v1", &[&cid.to_be_bytes()]);
        Self::new(
            AuthorityScope::Host(host_uid),
            AuthorityClass::VsockCid,
            digest,
            AuthorityArbitration::Exclusive,
            1,
            None,
            owner_proof,
            None,
        )
    }

    /// Build a fixed listener Endpoint claim.
    pub fn fixed_listener_port(
        host_uid: ResourceUid,
        port: u16,
        protocol: PortProtocol,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        if port == 0 {
            return Err(AuthorityError::InvalidListenerPort);
        }
        let digest = framed_digest(
            "fixed-listener-port/v1",
            &[&port.to_be_bytes(), &[port_protocol_tag(protocol)]],
        );
        Self::new(
            AuthorityScope::Host(host_uid),
            AuthorityClass::FixedListenerPort,
            digest,
            AuthorityArbitration::Exclusive,
            1,
            None,
            owner_proof,
            None,
        )
    }

    /// Build the Host Nix store authority claim.
    pub fn host_store(
        host_uid: ResourceUid,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        Self::new(
            AuthorityScope::Host(host_uid),
            AuthorityClass::HostStore,
            class_digest(AuthorityClass::HostStore),
            AuthorityArbitration::Shared,
            1,
            None,
            owner_proof,
            None,
        )
    }

    /// Build the exclusive per-Guest store-view writer claim.
    pub fn guest_store_view_writer(
        host_uid: ResourceUid,
        guest_uid: ResourceUid,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        let digest = uid_digest("guest-store-view-writer/v1", &guest_uid);
        Self::new(
            AuthorityScope::Host(host_uid),
            AuthorityClass::GuestStoreViewWriter,
            digest,
            AuthorityArbitration::Exclusive,
            1,
            None,
            owner_proof,
            Some(guest_uid),
        )
    }

    /// Build a Zone-local Network TAP/bridge claim.
    pub fn network_tap_bridge(
        zone_uid: ResourceUid,
        network_uid: ResourceUid,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        let digest = uid_digest("network-tap-bridge/v1", &network_uid);
        Self::new(
            AuthorityScope::Zone(zone_uid),
            AuthorityClass::NetworkTapBridge,
            digest,
            AuthorityArbitration::Exclusive,
            1,
            None,
            owner_proof,
            None,
        )
    }

    /// Return the closed authority class.
    pub const fn class(&self) -> AuthorityClass {
        self.key.class
    }

    /// Return the requested arbitration.
    pub const fn arbitration(&self) -> AuthorityArbitration {
        self.arbitration
    }

    /// Return the bounded requested holder limit.
    pub const fn max_holders(&self) -> usize {
        self.max_holders
    }

    /// Return Provider cardinality when this is a Provider claim.
    pub const fn provider_cardinality(&self) -> Option<ProviderCardinality> {
        self.provider_cardinality
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        scope: AuthorityScope,
        class: AuthorityClass,
        opaque_digest: String,
        arbitration: AuthorityArbitration,
        max_holders: usize,
        provider_cardinality: Option<ProviderCardinality>,
        owner_proof: AuthorityOwnerProof,
        dependent_guest: Option<ResourceUid>,
    ) -> Result<Self, AuthorityError> {
        if max_holders == 0 || max_holders > u32::MAX as usize {
            return Err(AuthorityError::InvalidAuthorityHolderLimit);
        }
        if class != AuthorityClass::Provider && provider_cardinality.is_some() {
            return Err(AuthorityError::InvalidAuthorityRequest);
        }
        Ok(Self {
            key: AuthorityKey::new(scope, class, opaque_digest)?,
            owner_proof,
            arbitration,
            max_holders,
            provider_cardinality,
            dependent_guest,
        })
    }

    fn hardware(
        host_uid: ResourceUid,
        class: AuthorityClass,
        backing: AuthorityDigest,
        arbitration: AuthorityArbitration,
        max_holders: usize,
        owner_proof: AuthorityOwnerProof,
        dependent_guest: Option<ResourceUid>,
    ) -> Result<Self, AuthorityError> {
        if backing.is_zero() {
            return Err(AuthorityError::InvalidAuthorityKey);
        }
        let bytes = backing.as_bytes();
        let digest = if class == AuthorityClass::PhysicalUsbBacking {
            framed_digest(PHYSICAL_USB_BACKING_IDENTITY_DOMAIN, &[&bytes])
        } else {
            framed_digest(class.as_str(), &[&bytes])
        };
        Self::new(
            AuthorityScope::Host(host_uid),
            class,
            digest,
            arbitration,
            max_holders,
            None,
            owner_proof,
            dependent_guest,
        )
    }

    fn shared_host_grant(
        class: AuthorityClass,
        host_uid: ResourceUid,
        owner_proof: AuthorityOwnerProof,
    ) -> Result<Self, AuthorityError> {
        Self::new(
            AuthorityScope::Host(host_uid),
            class,
            class_digest(class),
            AuthorityArbitration::Shared,
            1,
            None,
            owner_proof,
            None,
        )
    }
}

impl core::fmt::Debug for AuthorityRequest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AuthorityRequest")
            .field("class", &self.key.class)
            .field("arbitration", &self.arbitration)
            .field("max_holders", &self.max_holders)
            .field(
                "has_provider_cardinality",
                &self.provider_cardinality.is_some(),
            )
            .finish()
    }
}

/// Proof that a generic authority was admitted before an effect.
pub struct AuthorityLease {
    key: AuthorityKey,
    owner_proof: AuthorityOwnerProof,
}

impl AuthorityLease {
    /// Return the class held by this lease.
    pub const fn class(&self) -> AuthorityClass {
        self.key.class
    }
}

impl core::fmt::Debug for AuthorityLease {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuthorityLease(<redacted>)")
    }
}

/// Closed effect outcome retained with an admitted generic lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityEffectOutcome {
    /// The effect completed and observation confirmed it.
    Confirmed,
    /// The effect can be retried while the lease remains held.
    RetryableFailure,
    /// The effect failed terminally while the lease remains held for drain.
    TerminalFailure,
}

/// Result of gating one generic host or Zone effect on admission.
pub struct AuthorityEffectGate {
    lease: AuthorityLease,
    outcome: AuthorityEffectOutcome,
}

impl AuthorityEffectGate {
    /// Consume the gate into its retained lease.
    pub fn into_lease(self) -> AuthorityLease {
        self.lease
    }

    /// Return the closed effect outcome.
    pub const fn outcome(&self) -> AuthorityEffectOutcome {
        self.outcome
    }
}

impl core::fmt::Debug for AuthorityEffectGate {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AuthorityEffectGate")
            .field("lease", &self.lease)
            .field("outcome", &self.outcome)
            .finish()
    }
}

/// Result of closing an old generic authority-backed effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityCloseOutcome {
    /// The old effect is confirmed closed.
    Confirmed,
    /// The old effect remains held for retry.
    RetryableFailure,
}

/// Restart-adoption result for a generic authority.
pub enum AuthorityAdoption {
    /// Exactly one recovered owner matched the indexed holder.
    Adopted(AuthorityLease),
    /// No indexed or observed owner matched.
    Missing,
    /// More than one observed owner matched, so the effect is quarantined.
    QuarantinedAmbiguous,
}

impl core::fmt::Debug for AuthorityAdoption {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Adopted(_) => "AuthorityAdoption::Adopted(<redacted>)",
            Self::Missing => "AuthorityAdoption::Missing",
            Self::QuarantinedAmbiguous => "AuthorityAdoption::QuarantinedAmbiguous",
        })
    }
}

/// Bounded public observation for one generic authority key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityStatus {
    available: bool,
    holder_count: u32,
    max_holders: u32,
    arbitration: AuthorityArbitration,
    update_currency: UpdateState,
}

impl AuthorityStatus {
    /// Whether a compatible holder can currently be admitted.
    pub const fn available(self) -> bool {
        self.available
    }

    /// Return the current bounded holder count.
    pub const fn holder_count(self) -> u32 {
        self.holder_count
    }

    /// Return the configured bounded holder limit.
    pub const fn max_holders(self) -> u32 {
        self.max_holders
    }

    /// Return the closed arbitration policy.
    pub const fn arbitration(self) -> AuthorityArbitration {
        self.arbitration
    }

    /// Return the status currency.
    pub const fn update_currency(self) -> UpdateState {
        self.update_currency
    }
}

#[derive(Clone)]
struct GenericHolder {
    owner_proof: AuthorityOwnerProof,
    dependent_guest: Option<ResourceUid>,
}

struct GenericAuthorityEntry {
    holders: Vec<GenericHolder>,
    arbitration: AuthorityArbitration,
    max_holders: usize,
    dependent_guest: Option<ResourceUid>,
}

/// Core-owned Host-global external physical-NIC authority index.
#[derive(Default)]
pub struct HostGlobalAuthorityIndex {
    authorities: BTreeMap<AuthorityKey, GenericAuthorityEntry>,
    external_nics: BTreeMap<ExternalNicAuthorityKey, AuthorityEntry>,
}

impl HostGlobalAuthorityIndex {
    /// Admit one typed authority before invoking any host or Zone effect.
    pub fn admit_authority(
        &mut self,
        request: AuthorityRequest,
    ) -> Result<AuthorityLease, AuthorityError> {
        let key = request.key.clone();
        if let Some(entry) = self.authorities.get_mut(&key) {
            if let Some(holder) = entry
                .holders
                .iter()
                .find(|holder| holder.owner_proof == request.owner_proof)
            {
                if holder.dependent_guest != request.dependent_guest
                    || entry.arbitration != request.arbitration
                {
                    return Err(AuthorityError::AuthorityOwnerProofMismatch);
                }
                return Ok(AuthorityLease {
                    key,
                    owner_proof: request.owner_proof,
                });
            }
            if entry.arbitration != request.arbitration {
                return Err(AuthorityError::AuthorityArbitrationConflict);
            }
            let holder_limit = entry.max_holders.min(request.max_holders);
            if entry.holders.len() >= holder_limit {
                return Err(
                    if entry.arbitration == AuthorityArbitration::Shared && holder_limit > 1 {
                        AuthorityError::AuthorityCapacityExceeded
                    } else {
                        conflict_for_class(request.class())
                    },
                );
            }
            entry.max_holders = holder_limit;
            entry.holders.push(GenericHolder {
                owner_proof: request.owner_proof.clone(),
                dependent_guest: request.dependent_guest,
            });
        } else {
            self.authorities.insert(
                key.clone(),
                GenericAuthorityEntry {
                    holders: vec![GenericHolder {
                        owner_proof: request.owner_proof.clone(),
                        dependent_guest: request.dependent_guest.clone(),
                    }],
                    arbitration: request.arbitration,
                    max_holders: request.max_holders,
                    dependent_guest: request.dependent_guest,
                },
            );
        }
        Ok(AuthorityLease {
            key,
            owner_proof: request.owner_proof,
        })
    }

    /// Admit one typed authority and run an effect only after admission.
    pub fn admit_authority_before_effect(
        &mut self,
        request: AuthorityRequest,
        effect: impl FnOnce(&AuthorityLease) -> AuthorityEffectOutcome,
    ) -> Result<AuthorityEffectGate, AuthorityError> {
        let lease = self.admit_authority(request)?;
        let outcome = effect(&lease);
        Ok(AuthorityEffectGate { lease, outcome })
    }

    /// Return a bounded observation for an admitted generic authority.
    pub fn authority_status(&self, request: &AuthorityRequest) -> Option<AuthorityStatus> {
        let entry = self.authorities.get(&request.key)?;
        Some(AuthorityStatus {
            available: entry.holders.len() < entry.max_holders,
            holder_count: entry.holders.len() as u32,
            max_holders: entry.max_holders as u32,
            arbitration: entry.arbitration,
            update_currency: UpdateState::Current,
        })
    }

    /// Adopt exactly one recovered owner proof after restart.
    pub fn adopt_authority(
        &self,
        request: &AuthorityRequest,
        recovered_owner_proofs: &[AuthorityOwnerProof],
    ) -> AuthorityAdoption {
        let Some(entry) = self.authorities.get(&request.key) else {
            return AuthorityAdoption::Missing;
        };
        let matching_observations = recovered_owner_proofs
            .iter()
            .filter(|proof| *proof == &request.owner_proof)
            .count();
        if matching_observations > 1 {
            return AuthorityAdoption::QuarantinedAmbiguous;
        }
        let indexed = entry
            .holders
            .iter()
            .any(|holder| holder.owner_proof == request.owner_proof);
        if matching_observations == 1 && indexed {
            AuthorityAdoption::Adopted(AuthorityLease {
                key: request.key.clone(),
                owner_proof: request.owner_proof.clone(),
            })
        } else {
            AuthorityAdoption::Missing
        }
    }

    /// Close an old effect before releasing its generic authority.
    pub fn close_then_release_authority(
        &mut self,
        lease: &AuthorityLease,
        close: impl FnOnce() -> AuthorityCloseOutcome,
    ) -> Result<(), AuthorityError> {
        if close() != AuthorityCloseOutcome::Confirmed {
            return Err(AuthorityError::AuthorityCloseUnconfirmed);
        }
        self.release_authority(lease)
    }

    /// Drain an old effect and admit its replacement without an overlap.
    pub fn replace_authority_after_close(
        &mut self,
        lease: &AuthorityLease,
        replacement: AuthorityRequest,
        close: impl FnOnce() -> AuthorityCloseOutcome,
    ) -> Result<AuthorityLease, AuthorityError> {
        self.close_then_release_authority(lease, close)?;
        self.admit_authority(replacement)
    }

    /// Release one exact generic authority holder.
    pub fn release_authority(&mut self, lease: &AuthorityLease) -> Result<(), AuthorityError> {
        let entry = self
            .authorities
            .get_mut(&lease.key)
            .ok_or(AuthorityError::UnknownAuthority)?;
        let holder = entry
            .holders
            .iter()
            .position(|holder| holder.owner_proof == lease.owner_proof)
            .ok_or(AuthorityError::AuthorityOwnerProofMismatch)?;
        entry.holders.remove(holder);
        if entry.holders.is_empty() {
            self.authorities.remove(&lease.key);
        }
        Ok(())
    }

    /// Drain all authority leases dependent on a stopped Guest.
    pub fn drain_guest(&mut self, host_uid: &ResourceUid, guest_uid: &ResourceUid) -> usize {
        let keys = self
            .authorities
            .iter()
            .filter(|(key, entry)| {
                matches!(&key.scope, AuthorityScope::Host(host) if host == host_uid)
                    && (entry.dependent_guest.as_ref() == Some(guest_uid)
                        || entry
                            .holders
                            .iter()
                            .any(|holder| holder.dependent_guest.as_ref() == Some(guest_uid)))
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let mut drained = 0;
        for key in keys {
            if let Some(entry) = self.authorities.remove(&key) {
                drained += entry.holders.len();
            }
        }
        drained
    }

    /// Admit the claim, then and only then invoke one host effect.
    pub fn admit_before_effect(
        &mut self,
        request: ExternalNicClaimRequest,
        effect: impl FnOnce(&ExternalNicLease) -> ExternalNicEffectOutcome,
    ) -> Result<ExternalNicEffectGate, AuthorityError> {
        let lease = self.admit(request)?;
        let outcome = effect(&lease);
        Ok(ExternalNicEffectGate { lease, outcome })
    }

    /// Return the bounded public observation for one resolved authority.
    pub fn external_nic_status(
        &self,
        host_uid: ResourceUid,
        identity: &ResolvedExternalNicIdentity,
    ) -> Option<ExternalNicAuthorityStatus> {
        let key = ExternalNicAuthorityKey::derive(host_uid, identity);
        let entry = self.external_nics.get(&key)?;
        let all_multiplexable = entry.holders.iter().all(|holder| {
            holder.claim.macvtap_mode() == MacvtapMode::Bridge
                && holder.claim.sharing_policy() == SharingPolicy::Multiplexed
        });
        let arbitration = if all_multiplexable {
            SharingPolicy::Multiplexed
        } else {
            SharingPolicy::Exclusive
        };
        Some(ExternalNicAuthorityStatus::new(
            all_multiplexable && entry.holders.len() < entry.signed_max_holders,
            entry.holders.len() as u32,
            0,
            arbitration,
            UpdateState::Current,
        ))
    }

    /// Adopt only one exact recovered owner; duplicate observations quarantine.
    pub fn adopt(
        &self,
        host_uid: ResourceUid,
        identity: &ResolvedExternalNicIdentity,
        owner_proof: &ExternalNicOwnerProof,
        recovered_owner_proofs: &[ExternalNicOwnerProof],
    ) -> ExternalNicAdoption {
        let key = ExternalNicAuthorityKey::derive(host_uid, identity);
        let Some(entry) = self.external_nics.get(&key) else {
            return ExternalNicAdoption::Missing;
        };
        if recovered_owner_proofs
            .iter()
            .filter(|proof| *proof == owner_proof)
            .count()
            > 1
        {
            return ExternalNicAdoption::QuarantinedAmbiguous;
        }
        let observed = recovered_owner_proofs
            .iter()
            .filter(|proof| *proof == owner_proof)
            .count()
            == 1;
        let indexed = entry
            .holders
            .iter()
            .any(|holder| &holder.owner_proof == owner_proof);
        if observed && indexed {
            ExternalNicAdoption::Adopted(ExternalNicLease {
                key,
                owner_proof: owner_proof.clone(),
            })
        } else {
            ExternalNicAdoption::Missing
        }
    }

    /// Close the old attachment before releasing its authority claim.
    pub fn close_then_release(
        &mut self,
        lease: &ExternalNicLease,
        close: impl FnOnce() -> ExternalNicCloseOutcome,
    ) -> Result<(), AuthorityError> {
        if close() != ExternalNicCloseOutcome::Confirmed {
            return Err(AuthorityError::AttachmentCloseUnconfirmed);
        }
        self.release(lease)
    }

    /// Drain and release an old claim before admitting a disruptive replacement.
    pub fn replace_after_close(
        &mut self,
        lease: &ExternalNicLease,
        replacement: ExternalNicClaimRequest,
        close: impl FnOnce() -> ExternalNicCloseOutcome,
    ) -> Result<ExternalNicLease, AuthorityError> {
        self.close_then_release(lease, close)?;
        self.admit(replacement)
    }

    fn admit(
        &mut self,
        request: ExternalNicClaimRequest,
    ) -> Result<ExternalNicLease, AuthorityError> {
        let key = ExternalNicAuthorityKey::derive(request.host_uid, &request.identity);
        if let Some(entry) = self.external_nics.get_mut(&key) {
            if let Some(holder) = entry
                .holders
                .iter()
                .find(|holder| holder.owner_proof == request.owner_proof)
            {
                if holder.claim == request.claim {
                    return Ok(ExternalNicLease {
                        key,
                        owner_proof: request.owner_proof,
                    });
                }
                return Err(AuthorityError::OwnerProofMismatch);
            }
            let signed_limit = entry.signed_max_holders.min(request.signed_max_holders);
            let mut claims: Vec<ExternalNicClaim> = entry
                .holders
                .iter()
                .map(|holder| holder.claim.clone())
                .collect();
            claims.push(request.claim.clone());
            admit_external_nic_claims(&claims, signed_limit)?;
            entry.signed_max_holders = signed_limit;
            entry.holders.push(Holder {
                claim: request.claim,
                owner_proof: request.owner_proof.clone(),
            });
        } else {
            admit_external_nic_claims(
                core::slice::from_ref(&request.claim),
                request.signed_max_holders,
            )?;
            self.external_nics.insert(
                key.clone(),
                AuthorityEntry {
                    holders: vec![Holder {
                        claim: request.claim,
                        owner_proof: request.owner_proof.clone(),
                    }],
                    signed_max_holders: request.signed_max_holders,
                },
            );
        }
        Ok(ExternalNicLease {
            key,
            owner_proof: request.owner_proof,
        })
    }

    fn release(&mut self, lease: &ExternalNicLease) -> Result<(), AuthorityError> {
        let entry = self
            .external_nics
            .get_mut(&lease.key)
            .ok_or(AuthorityError::UnknownClaim)?;
        let holder = entry
            .holders
            .iter()
            .position(|holder| holder.owner_proof == lease.owner_proof)
            .ok_or(AuthorityError::OwnerProofMismatch)?;
        entry.holders.remove(holder);
        if entry.holders.is_empty() {
            self.external_nics.remove(&lease.key);
        }
        Ok(())
    }
}

impl core::fmt::Debug for HostGlobalAuthorityIndex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HostGlobalAuthorityIndex")
            .field(
                "authority_count",
                &(self.external_nics.len() + self.authorities.len()),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).unwrap()
    }

    fn identity(value: &[u8]) -> ResolvedExternalNicIdentity {
        ResolvedExternalNicIdentity::from_trusted_inventory(value).unwrap()
    }

    fn proof(value: &str, generation: u64) -> ExternalNicOwnerProof {
        ExternalNicOwnerProof::new(uid(value), ResourceGeneration::new(generation).unwrap())
    }

    fn authority_proof(value: &str, generation: u64) -> AuthorityOwnerProof {
        AuthorityOwnerProof::new(uid(value), ResourceGeneration::new(generation).unwrap())
    }

    fn digest(byte: u8) -> AuthorityDigest {
        AuthorityDigest::from_core([byte; 32])
    }

    fn request(
        host: &ResourceUid,
        nic: &ResolvedExternalNicIdentity,
        zone: &ResourceUid,
        owner: ExternalNicOwnerProof,
        mode: MacvtapMode,
        policy: SharingPolicy,
        limit: usize,
    ) -> ExternalNicClaimRequest {
        ExternalNicClaimRequest::new(
            host.clone(),
            nic.clone(),
            ExternalNicClaim::new(zone.clone(), mode, policy),
            owner,
            limit,
        )
        .unwrap()
    }

    #[test]
    fn two_selectors_resolving_to_one_nic_share_one_host_global_key() {
        let mut inventory = TrustedExternalNicInventory::default();
        let resolved = identity(b"stable-inventory-identity");
        inventory
            .insert(IfName::parse("eno1").unwrap(), resolved.clone())
            .unwrap();
        inventory
            .insert(IfName::parse("uplink0").unwrap(), resolved.clone())
            .unwrap();
        let first = inventory.resolve(&IfName::parse("eno1").unwrap()).unwrap();
        let second = inventory
            .resolve(&IfName::parse("uplink0").unwrap())
            .unwrap();
        let host = uid("123e4567-e89b-42d3-a456-426614174000");
        assert_eq!(
            ExternalNicAuthorityKey::derive(host.clone(), &first),
            ExternalNicAuthorityKey::derive(host, &second)
        );
    }

    #[test]
    fn cross_zone_bridge_rejection_is_distinct_and_runs_no_effect() {
        let host = uid("123e4567-e89b-42d3-a456-426614174000");
        let work = uid("223e4567-e89b-42d3-a456-426614174001");
        let personal = uid("323e4567-e89b-42d3-a456-426614174002");
        let nic = identity(b"one-physical-nic");
        let mut index = HostGlobalAuthorityIndex::default();
        let first = request(
            &host,
            &nic,
            &work,
            proof("423e4567-e89b-42d3-a456-426614174003", 1),
            MacvtapMode::Bridge,
            SharingPolicy::Multiplexed,
            8,
        );
        index
            .admit_before_effect(first, |_| ExternalNicEffectOutcome::Confirmed)
            .unwrap();

        let mut effects = 0;
        let second = request(
            &host,
            &nic,
            &personal,
            proof("523e4567-e89b-42d3-a456-426614174004", 1),
            MacvtapMode::Bridge,
            SharingPolicy::Exclusive,
            1,
        );
        let error = index
            .admit_before_effect(second, |_| {
                effects += 1;
                ExternalNicEffectOutcome::Confirmed
            })
            .unwrap_err();
        assert_eq!(
            error,
            AuthorityError::Admission(ExternalNicAdmissionError::ExternalPhysicalNicCrossZoneL2)
        );
        assert_eq!(error.code(), "external-physical-nic-cross-zone-l2");
        assert_eq!(effects, 0);
    }

    #[test]
    fn same_zone_compatible_bridge_multiplex_obeys_the_signed_limit() {
        let host = uid("123e4567-e89b-42d3-a456-426614174000");
        let zone = uid("223e4567-e89b-42d3-a456-426614174001");
        let nic = identity(b"one-physical-nic");
        let mut index = HostGlobalAuthorityIndex::default();
        for owner in [
            "323e4567-e89b-42d3-a456-426614174002",
            "423e4567-e89b-42d3-a456-426614174003",
        ] {
            index
                .admit_before_effect(
                    request(
                        &host,
                        &nic,
                        &zone,
                        proof(owner, 1),
                        MacvtapMode::Bridge,
                        SharingPolicy::Multiplexed,
                        2,
                    ),
                    |_| ExternalNicEffectOutcome::Confirmed,
                )
                .unwrap();
        }
        let status = index.external_nic_status(host, &nic).unwrap();
        assert_eq!(status.holder_count(), 2);
        assert_eq!(status.arbitration(), SharingPolicy::Multiplexed);
        assert!(!status.available());
    }

    #[test]
    fn exclusive_mixed_and_non_bridge_claims_report_the_general_conflict() {
        let host = uid("123e4567-e89b-42d3-a456-426614174000");
        let zone = uid("223e4567-e89b-42d3-a456-426614174001");
        for (first_mode, first_policy, next_mode, next_policy) in [
            (
                MacvtapMode::Bridge,
                SharingPolicy::Exclusive,
                MacvtapMode::Bridge,
                SharingPolicy::Multiplexed,
            ),
            (
                MacvtapMode::Private,
                SharingPolicy::Exclusive,
                MacvtapMode::Private,
                SharingPolicy::Exclusive,
            ),
        ] {
            let nic = identity(b"one-physical-nic");
            let mut index = HostGlobalAuthorityIndex::default();
            index
                .admit_before_effect(
                    request(
                        &host,
                        &nic,
                        &zone,
                        proof("323e4567-e89b-42d3-a456-426614174002", 1),
                        first_mode,
                        first_policy,
                        8,
                    ),
                    |_| ExternalNicEffectOutcome::Confirmed,
                )
                .unwrap();
            let error = index
                .admit_before_effect(
                    request(
                        &host,
                        &nic,
                        &zone,
                        proof("423e4567-e89b-42d3-a456-426614174003", 1),
                        next_mode,
                        next_policy,
                        8,
                    ),
                    |_| ExternalNicEffectOutcome::Confirmed,
                )
                .unwrap_err();
            assert_eq!(
                error,
                AuthorityError::Admission(ExternalNicAdmissionError::ExternalPhysicalNicConflict)
            );
        }

        let nic = identity(b"cross-zone-exclusive-nic");
        let mut index = HostGlobalAuthorityIndex::default();
        index
            .admit_before_effect(
                request(
                    &host,
                    &nic,
                    &zone,
                    proof("323e4567-e89b-42d3-a456-426614174002", 1),
                    MacvtapMode::Passthru,
                    SharingPolicy::Exclusive,
                    1,
                ),
                |_| ExternalNicEffectOutcome::Confirmed,
            )
            .unwrap();
        let mut effects = 0;
        let error = index
            .admit_before_effect(
                request(
                    &host,
                    &nic,
                    &uid("523e4567-e89b-42d3-a456-426614174004"),
                    proof("423e4567-e89b-42d3-a456-426614174003", 1),
                    MacvtapMode::Passthru,
                    SharingPolicy::Exclusive,
                    1,
                ),
                |_| {
                    effects += 1;
                    ExternalNicEffectOutcome::Confirmed
                },
            )
            .unwrap_err();
        assert_eq!(
            error,
            AuthorityError::Admission(ExternalNicAdmissionError::ExternalPhysicalNicConflict)
        );
        assert_eq!(effects, 0);
    }

    #[test]
    fn restart_adopts_one_exact_owner_and_quarantines_ambiguity() {
        let host = uid("123e4567-e89b-42d3-a456-426614174000");
        let zone = uid("223e4567-e89b-42d3-a456-426614174001");
        let nic = identity(b"one-physical-nic");
        let owner = proof("323e4567-e89b-42d3-a456-426614174002", 4);
        let mut index = HostGlobalAuthorityIndex::default();
        index
            .admit_before_effect(
                request(
                    &host,
                    &nic,
                    &zone,
                    owner.clone(),
                    MacvtapMode::Bridge,
                    SharingPolicy::Exclusive,
                    1,
                ),
                |_| ExternalNicEffectOutcome::Confirmed,
            )
            .unwrap();
        assert!(matches!(
            index.adopt(host.clone(), &nic, &owner, core::slice::from_ref(&owner)),
            ExternalNicAdoption::Adopted(_)
        ));
        assert!(matches!(
            index.adopt(host, &nic, &owner, &[owner.clone(), owner.clone()]),
            ExternalNicAdoption::QuarantinedAmbiguous
        ));
    }

    #[test]
    fn update_and_delete_release_only_after_attachment_close() {
        let host = uid("123e4567-e89b-42d3-a456-426614174000");
        let zone = uid("223e4567-e89b-42d3-a456-426614174001");
        let nic = identity(b"one-physical-nic");
        let mut index = HostGlobalAuthorityIndex::default();
        let gate = index
            .admit_before_effect(
                request(
                    &host,
                    &nic,
                    &zone,
                    proof("323e4567-e89b-42d3-a456-426614174002", 1),
                    MacvtapMode::Bridge,
                    SharingPolicy::Exclusive,
                    1,
                ),
                |_| ExternalNicEffectOutcome::Confirmed,
            )
            .unwrap();
        let lease = gate.into_lease();
        assert_eq!(
            index.close_then_release(&lease, || ExternalNicCloseOutcome::RetryableFailure),
            Err(AuthorityError::AttachmentCloseUnconfirmed)
        );
        assert!(index.external_nic_status(host.clone(), &nic).is_some());

        let adopted = match index.adopt(
            host.clone(),
            &nic,
            &proof("323e4567-e89b-42d3-a456-426614174002", 1),
            &[proof("323e4567-e89b-42d3-a456-426614174002", 1)],
        ) {
            ExternalNicAdoption::Adopted(lease) => lease,
            other => panic!("expected adoption, got {other:?}"),
        };
        let mut closed = false;
        let replacement = request(
            &host,
            &nic,
            &zone,
            proof("423e4567-e89b-42d3-a456-426614174003", 2),
            MacvtapMode::Bridge,
            SharingPolicy::Exclusive,
            1,
        );
        let replacement_lease = index
            .replace_after_close(&adopted, replacement, || {
                closed = true;
                ExternalNicCloseOutcome::Confirmed
            })
            .unwrap();
        assert!(closed);
        index
            .close_then_release(&replacement_lease, || ExternalNicCloseOutcome::Confirmed)
            .unwrap();
        assert!(index.external_nic_status(host, &nic).is_none());
    }

    #[test]
    fn provider_cardinality_is_zone_local_and_effects_are_fail_closed() {
        let mut index = HostGlobalAuthorityIndex::default();
        let provider = ResourceRef::parse("Provider/system-core").unwrap();
        let zone = uid("123e4567-e89b-42d3-a456-426614174000");
        let first = AuthorityRequest::provider(
            zone.clone(),
            provider.clone(),
            authority_proof("223e4567-e89b-42d3-a456-426614174001", 1),
        )
        .unwrap();
        assert_eq!(
            first.provider_cardinality(),
            Some(ProviderCardinality::ExactlyOne)
        );
        index
            .admit_authority_before_effect(first, |_| AuthorityEffectOutcome::Confirmed)
            .unwrap();

        let mut effects = 0;
        let duplicate = AuthorityRequest::provider(
            zone,
            provider,
            authority_proof("323e4567-e89b-42d3-a456-426614174002", 1),
        )
        .unwrap();
        assert_eq!(
            index
                .admit_authority_before_effect(duplicate, |_| {
                    effects += 1;
                    AuthorityEffectOutcome::Confirmed
                })
                .unwrap_err()
                .code(),
            "duplicateConflict"
        );
        assert_eq!(effects, 0);

        let other_zone = AuthorityRequest::provider(
            uid("423e4567-e89b-42d3-a456-426614174003"),
            ResourceRef::parse("Provider/observability-otel").unwrap(),
            authority_proof("523e4567-e89b-42d3-a456-426614174004", 1),
        )
        .unwrap();
        assert_eq!(
            other_zone.provider_cardinality(),
            Some(ProviderCardinality::AtMostOne)
        );
        index.admit_authority(other_zone).unwrap();
    }

    #[test]
    fn host_global_hardware_matrix_cannot_be_bypassed_by_zone_or_private_class() {
        let host = uid("623e4567-e89b-42d3-a456-426614174005");
        let mut index = HostGlobalAuthorityIndex::default();

        let gpu = AuthorityRequest::gpu_full_device(
            host.clone(),
            digest(1),
            authority_proof("723e4567-e89b-42d3-a456-426614174006", 1),
        )
        .unwrap();
        index.admit_authority(gpu).unwrap();
        let gpu_duplicate = AuthorityRequest::gpu_full_device(
            host.clone(),
            digest(1),
            authority_proof("823e4567-e89b-42d3-a456-426614174007", 1),
        )
        .unwrap();
        assert_eq!(
            index.admit_authority(gpu_duplicate).unwrap_err(),
            AuthorityError::DuplicateConflict
        );

        let render_first = AuthorityRequest::gpu_render_node(
            host.clone(),
            digest(2),
            2,
            authority_proof("923e4567-e89b-42d3-a456-426614174008", 1),
        )
        .unwrap();
        let render_second = AuthorityRequest::gpu_render_node(
            host.clone(),
            digest(2),
            2,
            authority_proof("a23e4567-e89b-42d3-a456-426614174009", 1),
        )
        .unwrap();
        index.admit_authority(render_first).unwrap();
        index.admit_authority(render_second).unwrap();
        let render_third = AuthorityRequest::gpu_render_node(
            host.clone(),
            digest(2),
            2,
            authority_proof("b23e4567-e89b-42d3-a456-426614174010", 1),
        )
        .unwrap();
        assert_eq!(
            index.admit_authority(render_third).unwrap_err(),
            AuthorityError::AuthorityCapacityExceeded
        );

        let guest = uid("c23e4567-e89b-42d3-a456-426614174011");
        let swtpm = AuthorityRequest::guest_swtpm(
            host.clone(),
            guest.clone(),
            authority_proof("d23e4567-e89b-42d3-a456-426614174012", 1),
        )
        .unwrap();
        index.admit_authority(swtpm).unwrap();
        let swtpm_duplicate = AuthorityRequest::guest_swtpm(
            host.clone(),
            guest.clone(),
            authority_proof("e23e4567-e89b-42d3-a456-426614174013", 1),
        )
        .unwrap();
        assert_eq!(
            index.admit_authority(swtpm_duplicate).unwrap_err(),
            AuthorityError::DuplicateConflict
        );
        let other_guest = AuthorityRequest::guest_swtpm(
            host.clone(),
            uid("f23e4567-e89b-42d3-a456-426614174014"),
            authority_proof("a33e4567-e89b-42d3-a456-426614174015", 1),
        )
        .unwrap();
        index.admit_authority(other_guest).unwrap();

        let physical_tpm = AuthorityRequest::physical_tpm(
            host.clone(),
            digest(3),
            authority_proof("b33e4567-e89b-42d3-a456-426614174016", 1),
        )
        .unwrap();
        index.admit_authority(physical_tpm).unwrap();
        let physical_tpm_duplicate = AuthorityRequest::physical_tpm(
            host.clone(),
            digest(3),
            authority_proof("c33e4567-e89b-42d3-a456-426614174017", 1),
        )
        .unwrap();
        assert_eq!(
            index.admit_authority(physical_tpm_duplicate).unwrap_err(),
            AuthorityError::DuplicateConflict
        );

        let usb = AuthorityRequest::physical_usb_backing(
            host.clone(),
            digest(4),
            authority_proof("d33e4567-e89b-42d3-a456-426614174018", 1),
        )
        .unwrap();
        index.admit_authority(usb).unwrap();
        let usb_loser = AuthorityRequest::physical_usb_backing(
            host.clone(),
            digest(4),
            authority_proof("e33e4567-e89b-42d3-a456-426614174019", 1),
        )
        .unwrap();
        assert_eq!(
            index.admit_authority(usb_loser).unwrap_err(),
            AuthorityError::PhysicalUsbBackingConflict
        );

        let module = AuthorityRequest::usbip_host_module(
            host.clone(),
            authority_proof("f33e4567-e89b-42d3-a456-426614174020", 1),
        )
        .unwrap();
        index.admit_authority(module).unwrap();
        let module_duplicate = AuthorityRequest::usbip_host_module(
            host.clone(),
            authority_proof("a43e4567-e89b-42d3-a456-426614174021", 1),
        )
        .unwrap();
        assert_eq!(
            index.admit_authority(module_duplicate).unwrap_err(),
            AuthorityError::DuplicateConflict
        );

        let network = uid("b43e4567-e89b-42d3-a456-426614174022");
        let relay = AuthorityRequest::usbip_network_relay(
            host.clone(),
            network.clone(),
            digest(5),
            authority_proof("c43e4567-e89b-42d3-a456-426614174023", 1),
        )
        .unwrap();
        index.admit_authority(relay).unwrap();
        let relay_duplicate = AuthorityRequest::usbip_network_relay(
            host.clone(),
            network,
            digest(5),
            authority_proof("d43e4567-e89b-42d3-a456-426614174024", 1),
        )
        .unwrap();
        assert_eq!(
            index.admit_authority(relay_duplicate).unwrap_err(),
            AuthorityError::UsbipNetworkRelayAuthorityConflict
        );

        for (request, duplicate) in [
            (
                AuthorityRequest::kvm(
                    host.clone(),
                    authority_proof("e43e4567-e89b-42d3-a456-426614174025", 1),
                )
                .unwrap(),
                AuthorityRequest::kvm(
                    host.clone(),
                    authority_proof("f43e4567-e89b-42d3-a456-426614174026", 1),
                )
                .unwrap(),
            ),
            (
                AuthorityRequest::vhost_vsock(
                    host.clone(),
                    authority_proof("a53e4567-e89b-42d3-a456-426614174027", 1),
                )
                .unwrap(),
                AuthorityRequest::vhost_vsock(
                    host.clone(),
                    authority_proof("b53e4567-e89b-42d3-a456-426614174028", 1),
                )
                .unwrap(),
            ),
            (
                AuthorityRequest::vsock_cid(
                    host.clone(),
                    42,
                    authority_proof("c53e4567-e89b-42d3-a456-426614174029", 1),
                )
                .unwrap(),
                AuthorityRequest::vsock_cid(
                    host.clone(),
                    42,
                    authority_proof("d53e4567-e89b-42d3-a456-426614174030", 1),
                )
                .unwrap(),
            ),
            (
                AuthorityRequest::fixed_listener_port(
                    host.clone(),
                    3240,
                    PortProtocol::Tcp,
                    authority_proof("e53e4567-e89b-42d3-a456-426614174031", 1),
                )
                .unwrap(),
                AuthorityRequest::fixed_listener_port(
                    host.clone(),
                    3240,
                    PortProtocol::Tcp,
                    authority_proof("f53e4567-e89b-42d3-a456-426614174032", 1),
                )
                .unwrap(),
            ),
        ] {
            index.admit_authority(request).unwrap();
            assert_eq!(
                index.admit_authority(duplicate).unwrap_err(),
                AuthorityError::DuplicateConflict
            );
        }
    }

    #[test]
    fn host_store_guest_writer_and_zone_network_authorities_have_exact_scopes() {
        let host = uid("a63e4567-e89b-42d3-a456-426614174033");
        let guest = uid("b63e4567-e89b-42d3-a456-426614174034");
        let zone = uid("c63e4567-e89b-42d3-a456-426614174035");
        let network = uid("d63e4567-e89b-42d3-a456-426614174036");
        let mut index = HostGlobalAuthorityIndex::default();

        index
            .admit_authority(
                AuthorityRequest::host_store(
                    host.clone(),
                    authority_proof("e63e4567-e89b-42d3-a456-426614174037", 1),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            index
                .admit_authority(
                    AuthorityRequest::host_store(
                        host.clone(),
                        authority_proof("f63e4567-e89b-42d3-a456-426614174038", 1),
                    )
                    .unwrap()
                )
                .unwrap_err(),
            AuthorityError::DuplicateConflict
        );

        let writer = AuthorityRequest::guest_store_view_writer(
            host.clone(),
            guest.clone(),
            authority_proof("a73e4567-e89b-42d3-a456-426614174039", 1),
        )
        .unwrap();
        index.admit_authority(writer).unwrap();
        assert_eq!(
            index.drain_guest(&host, &guest),
            1,
            "Guest stop drains its dependent writer lease"
        );

        let network_authority = AuthorityRequest::network_tap_bridge(
            zone.clone(),
            network.clone(),
            authority_proof("b73e4567-e89b-42d3-a456-426614174040", 1),
        )
        .unwrap();
        index.admit_authority(network_authority).unwrap();
        let same_zone_same_network = AuthorityRequest::network_tap_bridge(
            zone.clone(),
            network.clone(),
            authority_proof("c73e4567-e89b-42d3-a456-426614174041", 1),
        )
        .unwrap();
        assert_eq!(
            index.admit_authority(same_zone_same_network).unwrap_err(),
            AuthorityError::DuplicateConflict
        );
        let other_zone = AuthorityRequest::network_tap_bridge(
            uid("d73e4567-e89b-42d3-a456-426614174042"),
            network,
            authority_proof("e73e4567-e89b-42d3-a456-426614174043", 1),
        )
        .unwrap();
        index.admit_authority(other_zone).unwrap();
        let same_zone = AuthorityRequest::network_tap_bridge(
            zone,
            uid("f73e4567-e89b-42d3-a456-426614174044"),
            authority_proof("a83e4567-e89b-42d3-a456-426614174045", 1),
        )
        .unwrap();
        index.admit_authority(same_zone).unwrap();
    }

    #[test]
    fn generic_adoption_close_and_effect_order_are_fail_closed() {
        let host = uid("a83e4567-e89b-42d3-a456-426614174045");
        let owner = authority_proof("b83e4567-e89b-42d3-a456-426614174046", 2);
        let request =
            AuthorityRequest::vsock_cid(host, 77, owner.clone()).expect("valid CID request");
        let mut index = HostGlobalAuthorityIndex::default();
        let mut effects = 0;
        let gate = index
            .admit_authority_before_effect(request.clone(), |_| {
                effects += 1;
                AuthorityEffectOutcome::Confirmed
            })
            .unwrap();
        assert_eq!(effects, 1);
        assert_eq!(gate.outcome(), AuthorityEffectOutcome::Confirmed);
        assert!(matches!(
            index.adopt_authority(&request, core::slice::from_ref(&owner)),
            AuthorityAdoption::Adopted(_)
        ));
        assert!(matches!(
            index.adopt_authority(&request, &[owner.clone(), owner.clone()]),
            AuthorityAdoption::QuarantinedAmbiguous
        ));

        let lease = gate.into_lease();
        assert_eq!(
            index.close_then_release_authority(&lease, || {
                AuthorityCloseOutcome::RetryableFailure
            }),
            Err(AuthorityError::AuthorityCloseUnconfirmed)
        );
        assert!(
            index
                .authority_status(&request)
                .expect("retained after failed close")
                .holder_count()
                == 1
        );
        index
            .close_then_release_authority(&lease, || AuthorityCloseOutcome::Confirmed)
            .unwrap();
        assert!(index.authority_status(&request).is_none());
    }

    #[test]
    fn generic_authority_diagnostics_are_redacted_and_input_bounds_are_closed() {
        let canary = uid("c83e4567-e89b-42d3-a456-426614174047");
        let request = AuthorityRequest::physical_usb_backing(
            canary.clone(),
            digest(9),
            authority_proof("d83e4567-e89b-42d3-a456-426614174048", 1),
        )
        .unwrap();
        let rendered = format!("{:?} {:?} {:?}", digest(9), request, canary);
        assert!(!rendered.contains("c83e4567-e89b-42d3-a456-426614174047"));
        assert!(!rendered.contains("9"));
        assert_eq!(
            AuthorityRequest::vsock_cid(
                canary.clone(),
                0,
                authority_proof("e83e4567-e89b-42d3-a456-426614174049", 1),
            )
            .unwrap_err(),
            AuthorityError::InvalidVsockCid
        );
        assert_eq!(
            AuthorityRequest::fixed_listener_port(
                canary,
                0,
                PortProtocol::Tcp,
                authority_proof("f83e4567-e89b-42d3-a456-426614174050", 1),
            )
            .unwrap_err(),
            AuthorityError::InvalidListenerPort
        );
    }

    #[test]
    fn diagnostics_never_expose_identity_digest_host_or_owner_values() {
        let identity_canary = b"private-hardware-identity";
        let host_canary = "123e4567-e89b-42d3-a456-426614174000";
        let owner_canary = "223e4567-e89b-42d3-a456-426614174001";
        let nic = identity(identity_canary);
        let owner = proof(owner_canary, 1);
        let key = ExternalNicAuthorityKey::derive(uid(host_canary), &nic);
        let rendered = format!("{nic:?} {owner:?} {key:?}");
        for canary in [
            String::from_utf8(identity_canary.to_vec()).unwrap(),
            host_canary.to_owned(),
            owner_canary.to_owned(),
            key.opaque_digest.clone(),
        ] {
            assert!(!rendered.contains(&canary));
        }
    }
}
