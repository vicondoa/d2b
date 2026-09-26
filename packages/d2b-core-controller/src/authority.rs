//! Zone and Host-global authority admission for scarce resources.
//!
//! Core resolves authored selectors through trusted inventory, derives opaque
//! keys, and admits a claim before any host or VMM effect.  The key, resolved
//! inventory identity, and owner proof have no serialization or display
//! surface.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use d2b_contracts_resource::redacted_debug;
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ResourceGeneration, ResourceRef, ResourceUid, UpdateState,
    is_canonical_digest,
    network::{MacvtapMode, SharingPolicy},
    process::PortProtocol,
    resource_schema::canonical_digest,
};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use std::{collections::hash_map::RandomState, hash::BuildHasher};

/// Domain tag for Core-derived physical USB backing identities.
pub const PHYSICAL_USB_BACKING_IDENTITY_DOMAIN: &str = "physical-usb-backing/v1";
/// Domain tag for Core-derived USBIP relay endpoint identities.
pub const USBIP_NETWORK_RELAY_IDENTITY_DOMAIN: &str = "usbip-network-relay/v1";
/// The one Provider with optional controller cardinality: telemetry may be
/// absent from a Zone, so its claim is AtMostOne instead of ExactlyOne.
const OPTIONAL_PROVIDER_REF: &str = "Provider/observability-otel";
static NEXT_AUTHORITY_INDEX_NONCE: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
pub(crate) fn test_nonce_for_operation(operation_id: &str) -> u64 {
    loop {
        let nonce = RandomState::new().hash_one(operation_id);
        if nonce != 0 {
            return nonce;
        }
    }
}

/// Trusted recovery port for one Core-resolved physical-NIC inventory.
pub trait ExternalNicRecoveryInventory: Send + Sync {
    fn contains_identity(&self, host_uid: &ResourceUid, identity_digest: &str) -> bool;
}

/// Closed, identity-free authority failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityError {
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
    /// The exact owner already has an active reservation for this authority.
    DuplicateActiveReservation,
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
    /// Production admission ran before durable authority proofs were
    /// rehydrated.
    StartupRehydrationRequired,
    /// A reservation was already closed and cannot be reused.
    ReservationClosed,
}

impl AuthorityError {
    /// Return the stable, identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidAuthorityKey => "authority-key-invalid",
            Self::InvalidAuthorityHolderLimit => "authority-holder-limit-invalid",
            Self::InvalidAuthorityRequest => "authority-request-invalid",
            Self::AuthorityOwnerProofMismatch => "authority-owner-proof-mismatch",
            Self::AuthorityArbitrationConflict => "authority-arbitration-conflict",
            Self::DuplicateActiveReservation => "authority-duplicate-active-reservation",
            Self::AuthorityCapacityExceeded => "authority-capacity-exceeded",
            Self::UnknownAuthority => "authority-missing",
            Self::AuthorityCloseUnconfirmed => "authority-close-unconfirmed",
            Self::DuplicateConflict => "duplicate-conflict",
            Self::PhysicalUsbBackingConflict => "physical-usb-backing-conflict",
            Self::UsbipNetworkRelayAuthorityConflict => "usbip-network-relay-authority-conflict",
            Self::InvalidVsockCid => "vsock-cid-invalid",
            Self::InvalidListenerPort => "listener-port-invalid",
            Self::StartupRehydrationRequired => "authority-startup-rehydration-required",
            Self::ReservationClosed => "authority-reservation-closed",
        }
    }
}

impl core::fmt::Display for AuthorityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for AuthorityError {}

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
    /// Return whether this is the forbidden all-zero identity.
    pub fn is_zero(self) -> bool {
        self.0 == [0; 32]
    }

    fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

redacted_debug!(AuthorityDigest);

/// Scope of a Core-owned authority key.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthorityArbitration {
    /// Only one owner may hold this key.
    Exclusive,
    /// Multiple bounded holders may share this key.
    Shared,
    /// Multiple consumers use one multiplexed owner.
    Multiplexed,
}

/// Provider controller cardinality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderCardinality {
    /// The Provider controller is required to exist once.
    ExactlyOne,
    /// The optional Provider may exist zero or one time.
    AtMostOne,
}

/// Exact resource generation proof for an authority owner.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorityOwnerProof {
    resource_ref: Option<ResourceRef>,
    resource_uid: ResourceUid,
    generation: ResourceGeneration,
}

impl AuthorityOwnerProof {
    /// Bind an authority owner to one exact resource generation.
    #[allow(dead_code)]
    pub(crate) const fn new(resource_uid: ResourceUid, generation: ResourceGeneration) -> Self {
        Self {
            resource_ref: None,
            resource_uid,
            generation,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn from_resource_ref(
        resource_ref: ResourceRef,
        resource_uid: ResourceUid,
        generation: ResourceGeneration,
    ) -> Self {
        Self {
            resource_ref: Some(resource_ref),
            resource_uid,
            generation,
        }
    }

    /// Compare two owner proofs without exposing their identities.
    pub fn matches(&self, other: &Self) -> bool {
        self == other
    }
}

redacted_debug!(AuthorityOwnerProof);

/// Durable, non-authorizing representation of one exact owner proof.
///
/// The proof is rehydrated from the resource store or operation ledger. It is
/// never inferred from a status string or a diagnostic projection.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DurableAuthorityOwnerProof {
    #[serde(default)]
    resource_ref: Option<ResourceRef>,
    resource_uid: ResourceUid,
    generation: ResourceGeneration,
}

impl DurableAuthorityOwnerProof {
    /// Bind a durable proof to a resource generation.
    #[allow(dead_code)]
    pub(crate) const fn new(resource_uid: ResourceUid, generation: ResourceGeneration) -> Self {
        Self {
            resource_ref: None,
            resource_uid,
            generation,
        }
    }

    fn from_owner_proof(proof: &AuthorityOwnerProof) -> Self {
        Self {
            resource_ref: proof.resource_ref.clone(),
            resource_uid: proof.resource_uid.clone(),
            generation: proof.generation,
        }
    }

    fn into_owner_proof(self) -> AuthorityOwnerProof {
        AuthorityOwnerProof {
            resource_ref: self.resource_ref,
            resource_uid: self.resource_uid,
            generation: self.generation,
        }
    }

    #[doc(hidden)]
    pub fn resource_ref(&self) -> Option<&ResourceRef> {
        self.resource_ref.as_ref()
    }

    #[doc(hidden)]
    pub fn resource_uid(&self) -> &ResourceUid {
        &self.resource_uid
    }

    #[doc(hidden)]
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }
}

redacted_debug!(DurableAuthorityOwnerProof);

/// Durable authority claim emitted by the authoritative resource/operation
/// store before an effect is dispatched.
///
/// This record contains only typed authority identity and an opaque digest.
/// It carries no host path, interface name, command, socket, or ambient
/// mutation handle.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DurableAuthorityClaim {
    scope: AuthorityScope,
    class: AuthorityClass,
    opaque_digest: String,
    arbitration: AuthorityArbitration,
    max_holders: u32,
    provider_cardinality: Option<ProviderCardinality>,
    owner_proof: DurableAuthorityOwnerProof,
    dependent_guest: Option<ResourceUid>,
}

impl DurableAuthorityClaim {
    #[doc(hidden)]
    pub fn owner_proof(&self) -> &DurableAuthorityOwnerProof {
        &self.owner_proof
    }

    /// Rehydrate the in-memory request owned by the authority index.
    fn into_request(self) -> Result<AuthorityRequest, AuthorityError> {
        if !valid_resource_uid(&self.owner_proof.resource_uid)
            || self
                .dependent_guest
                .as_ref()
                .is_some_and(|uid| !valid_resource_uid(uid))
        {
            return Err(AuthorityError::InvalidAuthorityRequest);
        }
        AuthorityRequest::new(
            self.scope,
            self.class,
            self.opaque_digest,
            self.arbitration,
            usize::try_from(self.max_holders)
                .map_err(|_| AuthorityError::InvalidAuthorityHolderLimit)?,
            self.provider_cardinality,
            self.owner_proof.into_owner_proof(),
            self.dependent_guest,
        )
    }
}

/// Non-authorizing durable representation of one external physical-NIC claim.
///
/// The identity digest is a storage key, not an admission capability. Core
/// validates these rows and consumes them into a private recovery receipt
/// before the index can use them.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DurableExternalNicClaim {
    host_uid: ResourceUid,
    identity_digest: String,
    zone_uid: ResourceUid,
    macvtap_mode: MacvtapMode,
    sharing_policy: SharingPolicy,
    signed_max_holders: u32,
    owner_proof: DurableAuthorityOwnerProof,
}

impl core::fmt::Debug for DurableExternalNicClaim {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DurableExternalNicClaim")
            .field("has_identity_digest", &true)
            .field("macvtap_mode", &self.macvtap_mode)
            .field("sharing_policy", &self.sharing_policy)
            .field("signed_max_holders", &self.signed_max_holders)
            .field("owner_proof", &self.owner_proof)
            .finish()
    }
}

impl DurableExternalNicClaim {
    #[doc(hidden)]
    pub fn owner_proof(&self) -> &DurableAuthorityOwnerProof {
        &self.owner_proof
    }

    #[doc(hidden)]
    pub fn host_uid(&self) -> &ResourceUid {
        &self.host_uid
    }

    #[doc(hidden)]
    pub fn identity_digest(&self) -> &str {
        &self.identity_digest
    }
}

/// Persisted authority row variant. This is deliberately storage data only;
/// it is never accepted directly by the authority index.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AuthorityStorageClaim {
    Generic(DurableAuthorityClaim),
    ExternalNic(DurableExternalNicClaim),
}

impl core::fmt::Debug for AuthorityStorageClaim {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Generic(_) => "AuthorityStorageClaim::Generic(<redacted>)",
            Self::ExternalNic(_) => "AuthorityStorageClaim::ExternalNic(<redacted>)",
        })
    }
}

/// Persisted lifecycle state for one authority operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthorityOperationState {
    Pending,
    EffectConfirmed,
    EffectRetryable,
    EffectTerminal,
    Closing,
    Closed,
    Released,
}

/// Non-authorizing operation row stored by the Zone persistence adapter.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorityStorageOperation {
    pub operation_id: String,
    pub claim: AuthorityStorageClaim,
    pub state: AuthorityOperationState,
    pub claim_digest: String,
    pub store_binding_digest: String,
}

impl core::fmt::Debug for AuthorityStorageOperation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AuthorityStorageOperation")
            .field("operation_id", &"<opaque>")
            .field("claim", &self.claim)
            .field("state", &self.state)
            .finish()
    }
}

/// Private receipt proving that durable authority rows were loaded from the
/// trusted persistence adapter and centrally validated.
pub struct AuthorityRecoveryReceipt {
    operations: Vec<AuthorityStorageOperation>,
    seen_operation_ids: BTreeSet<String>,
    capabilities: BTreeMap<String, crate::authority_persistence::AuthorityOperationCapability>,
}

impl core::fmt::Debug for AuthorityRecoveryReceipt {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AuthorityRecoveryReceipt")
            .field("operation_count", &self.operations.len())
            .field("seen_operation_count", &self.seen_operation_ids.len())
            .field("capability_count", &self.capabilities.len())
            .finish()
    }
}

impl core::fmt::Debug for DurableAuthorityClaim {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DurableAuthorityClaim")
            .field("scope", &self.scope)
            .field("class", &self.class)
            .field("arbitration", &self.arbitration)
            .field("max_holders", &self.max_holders)
            .field(
                "has_provider_cardinality",
                &self.provider_cardinality.is_some(),
            )
            .field("owner_proof", &self.owner_proof)
            .field("has_dependent_guest", &self.dependent_guest.is_some())
            .finish()
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
        if !valid_authority_digest(&opaque_digest) {
            return Err(AuthorityError::InvalidAuthorityKey);
        }
        Ok(Self {
            scope,
            class,
            opaque_digest,
        })
    }
}

fn valid_authority_digest(value: &str) -> bool {
    is_canonical_digest(value)
}

#[doc(hidden)]
pub fn claim_digest(claim: &AuthorityStorageClaim) -> Result<String, AuthorityError> {
    let bytes = serde_json::to_vec(claim).map_err(|_| AuthorityError::InvalidAuthorityRequest)?;
    let canonical =
        CanonicalJsonValue::parse(&bytes).map_err(|_| AuthorityError::InvalidAuthorityRequest)?;
    let bytes = d2b_contracts_resource::v3::canonical_json_bytes(&canonical)
        .map_err(|_| AuthorityError::InvalidAuthorityRequest)?;
    Ok(canonical_digest("d2b:authority-claim/v1", &bytes))
}

fn valid_resource_uid(uid: &ResourceUid) -> bool {
    !uid.to_canonical_string().is_empty()
}

redacted_debug!(AuthorityKey);

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
        let cardinality = if provider_ref.to_canonical_string() == OPTIONAL_PROVIDER_REF {
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

    /// Build a GPU claim from Core-resolved opaque identity bytes.
    ///
    /// This is the narrow adapter used by the daemon's typed GPU Provider
    /// port. The caller supplies only identities and generation evidence
    /// already resolved by Core; no host locator or caller-selected
    /// authority class is accepted.
    pub fn gpu_from_core(
        host_uid: ResourceUid,
        owner_ref: ResourceRef,
        owner_uid: ResourceUid,
        owner_generation: ResourceGeneration,
        backing: [u8; 32],
        render_node_only: bool,
        max_holders: usize,
    ) -> Result<Self, AuthorityError> {
        let owner_proof =
            AuthorityOwnerProof::from_resource_ref(owner_ref, owner_uid, owner_generation);
        let backing = AuthorityDigest(backing);
        if render_node_only {
            Self::gpu_render_node(host_uid, backing, max_holders, owner_proof)
        } else {
            Self::gpu_full_device(host_uid, backing, owner_proof)
        }
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

    /// Return the typed durable owner record that must be committed before an
    /// external effect is dispatched.
    pub fn durable_claim(&self) -> DurableAuthorityClaim {
        DurableAuthorityClaim {
            scope: self.key.scope.clone(),
            class: self.key.class,
            opaque_digest: self.key.opaque_digest.clone(),
            arbitration: self.arbitration,
            max_holders: self.max_holders as u32,
            provider_cardinality: self.provider_cardinality,
            owner_proof: DurableAuthorityOwnerProof::from_owner_proof(&self.owner_proof),
            dependent_guest: self.dependent_guest.clone(),
        }
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
        if !valid_resource_uid(&owner_proof.resource_uid)
            || dependent_guest
                .as_ref()
                .is_some_and(|uid| !valid_resource_uid(uid))
        {
            return Err(AuthorityError::InvalidAuthorityRequest);
        }
        if class != AuthorityClass::Provider && provider_cardinality.is_some() {
            return Err(AuthorityError::InvalidAuthorityRequest);
        }
        Self::validate_authority_combination(
            &scope,
            class,
            arbitration,
            max_holders,
            provider_cardinality,
            dependent_guest.as_ref(),
        )?;
        Ok(Self {
            key: AuthorityKey::new(scope, class, opaque_digest)?,
            owner_proof,
            arbitration,
            max_holders,
            provider_cardinality,
            dependent_guest,
        })
    }

    fn validate_authority_combination(
        scope: &AuthorityScope,
        class: AuthorityClass,
        arbitration: AuthorityArbitration,
        max_holders: usize,
        provider_cardinality: Option<ProviderCardinality>,
        dependent_guest: Option<&ResourceUid>,
    ) -> Result<(), AuthorityError> {
        let host_scoped = matches!(scope, AuthorityScope::Host(_));
        let zone_scoped = matches!(scope, AuthorityScope::Zone(_));
        match class {
            AuthorityClass::Provider => {
                if !zone_scoped
                    || arbitration != AuthorityArbitration::Exclusive
                    || max_holders != 1
                    || provider_cardinality.is_none()
                    || dependent_guest.is_some()
                {
                    return Err(AuthorityError::InvalidAuthorityRequest);
                }
            }
            AuthorityClass::Quota
            | AuthorityClass::EmergencyPolicy
            | AuthorityClass::NetworkTapBridge => {
                if !zone_scoped
                    || arbitration != AuthorityArbitration::Exclusive
                    || max_holders != 1
                    || provider_cardinality.is_some()
                    || dependent_guest.is_some()
                {
                    return Err(AuthorityError::InvalidAuthorityRequest);
                }
            }
            AuthorityClass::GuestSwtpm | AuthorityClass::GuestStoreViewWriter => {
                if !host_scoped
                    || arbitration != AuthorityArbitration::Exclusive
                    || max_holders != 1
                    || provider_cardinality.is_some()
                    || dependent_guest.is_none()
                {
                    return Err(AuthorityError::InvalidAuthorityRequest);
                }
            }
            AuthorityClass::GpuRenderNode => {
                if !host_scoped
                    || arbitration != AuthorityArbitration::Shared
                    || max_holders < 2
                    || provider_cardinality.is_some()
                    || dependent_guest.is_some()
                {
                    return Err(AuthorityError::InvalidAuthorityRequest);
                }
            }
            AuthorityClass::PhysicalUsbBacking
            | AuthorityClass::PhysicalTpm
            | AuthorityClass::GpuFullDevice
            | AuthorityClass::UsbipNetworkRelay
            | AuthorityClass::VsockCid
            | AuthorityClass::FixedListenerPort => {
                if !host_scoped
                    || arbitration == AuthorityArbitration::Shared
                    || max_holders != 1
                    || provider_cardinality.is_some()
                    || dependent_guest.is_some()
                {
                    return Err(AuthorityError::InvalidAuthorityRequest);
                }
            }
            AuthorityClass::UsbipHost => {
                if !host_scoped
                    || arbitration != AuthorityArbitration::Exclusive
                    || max_holders != 1
                    || provider_cardinality.is_some()
                    || dependent_guest.is_some()
                {
                    return Err(AuthorityError::InvalidAuthorityRequest);
                }
            }
            AuthorityClass::Kvm | AuthorityClass::VhostVsock => {
                if !host_scoped
                    || arbitration != AuthorityArbitration::Shared
                    || max_holders != 1
                    || provider_cardinality.is_some()
                    || dependent_guest.is_some()
                {
                    return Err(AuthorityError::InvalidAuthorityRequest);
                }
            }
            AuthorityClass::HostStore => {
                if !host_scoped
                    || arbitration != AuthorityArbitration::Shared
                    || max_holders != 1
                    || provider_cardinality.is_some()
                    || dependent_guest.is_some()
                {
                    return Err(AuthorityError::InvalidAuthorityRequest);
                }
            }
        }
        Ok(())
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
    arbitration: AuthorityArbitration,
    max_holders: usize,
    provider_cardinality: Option<ProviderCardinality>,
    dependent_guest: Option<ResourceUid>,
    token: u128,
    operation_id: Option<String>,
}

impl AuthorityLease {
    /// Return the class held by this lease.
    pub const fn class(&self) -> AuthorityClass {
        self.key.class
    }

    /// Return the opaque token for a typed Core adapter.
    pub const fn token_bytes(&self) -> [u8; 16] {
        self.token.to_be_bytes()
    }
}

redacted_debug!(AuthorityLease);

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
#[allow(clippy::large_enum_variant)]
pub enum AuthorityAdoption {
    /// Exactly one recovered owner matched the indexed holder.
    Adopted(AuthorityLease),
    /// No indexed or observed owner matched.
    Missing,
    /// More than one observed owner matched, so the effect is quarantined.
    QuarantinedAmbiguous,
}

/// Outcome of replaying one active authority operation after restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorityRecoveryResolution {
    /// The authoritative effect was observed and its reservation was adopted.
    ObservedAndAdopted,
    /// The effect was not found and the durable operation was resolved closed.
    ObservedClosed,
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
    token: u128,
    operation_id: Option<String>,
    owner_proof: AuthorityOwnerProof,
    max_holders: usize,
    dependent_guest: Option<ResourceUid>,
}

struct GenericAuthorityEntry {
    holders: Vec<GenericHolder>,
    arbitration: AuthorityArbitration,
    max_holders: usize,
    provider_cardinality: Option<ProviderCardinality>,
    dependent_guest: Option<ResourceUid>,
}

/// Core-owned Host-global external physical-NIC authority index.
pub struct HostGlobalAuthorityIndex {
    authorities: BTreeMap<AuthorityKey, GenericAuthorityEntry>,
    rehydrated: bool,
    unresolved_operations: BTreeSet<String>,
    quarantined_operations: BTreeSet<String>,
    seen_operation_ids: BTreeSet<String>,
    recovery_capabilities:
        BTreeMap<String, crate::authority_persistence::AuthorityOperationCapability>,
    next_token: AtomicU64,
    instance_nonce: u64,
    ready_epoch: u64,
    runtime_epoch: Arc<AtomicU64>,
}

impl Default for HostGlobalAuthorityIndex {
    fn default() -> Self {
        Self {
            authorities: BTreeMap::new(),
            rehydrated: false,
            unresolved_operations: BTreeSet::new(),
            quarantined_operations: BTreeSet::new(),
            seen_operation_ids: BTreeSet::new(),
            recovery_capabilities: BTreeMap::new(),
            next_token: AtomicU64::new(1),
            instance_nonce: NEXT_AUTHORITY_INDEX_NONCE.fetch_add(1, Ordering::Relaxed),
            runtime_epoch: Arc::new(AtomicU64::new(1)),
            ready_epoch: 1,
        }
    }
}

impl HostGlobalAuthorityIndex {
    /// Construct an explicitly ready pure in-memory index for unit tests.
    #[doc(hidden)]
    #[cfg(any(test, feature = "test-support"))]
    pub fn new_for_tests_ready() -> Self {
        Self {
            authorities: BTreeMap::new(),
            rehydrated: true,
            unresolved_operations: BTreeSet::new(),
            quarantined_operations: BTreeSet::new(),
            seen_operation_ids: BTreeSet::new(),
            recovery_capabilities: BTreeMap::new(),
            next_token: AtomicU64::new(1),
            instance_nonce: NEXT_AUTHORITY_INDEX_NONCE.fetch_add(1, Ordering::Relaxed),
            runtime_epoch: Arc::new(AtomicU64::new(1)),
            ready_epoch: 1,
        }
    }

    /// Construct the production gate before durable owner proofs are loaded.
    pub fn new_unrehydrated() -> Self {
        Self::default()
    }

    /// Invalidate process-local authority readiness before a restart relist.
    pub fn invalidate_for_restart(&mut self) {
        let epoch = self.runtime_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.authorities.clear();
        self.rehydrated = false;
        self.unresolved_operations.clear();
        self.quarantined_operations.clear();
        self.recovery_capabilities.clear();
        self.ready_epoch = epoch;
    }

    pub(crate) fn restart_epoch_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.runtime_epoch)
    }

    fn issue_token(&self) -> u128 {
        let sequence = self.next_token.fetch_add(1, Ordering::Relaxed);
        ((self.instance_nonce as u128) << 64) | sequence as u128
    }

    fn reserve_operation_id(&mut self, operation_id: &str) -> Result<(), AuthorityError> {
        if operation_id.is_empty()
            || operation_id.len() > 512
            || operation_id.bytes().any(|byte| byte.is_ascii_control())
            || !self.seen_operation_ids.insert(operation_id.to_owned())
        {
            return Err(AuthorityError::InvalidAuthorityRequest);
        }
        Ok(())
    }

    fn quarantine_operation_id(&mut self, operation_id: &str) {
        self.quarantined_operations.insert(operation_id.to_owned());
    }

    fn validate_recovery_operations(
        operations: &[AuthorityStorageOperation],
        expected_store_binding_digest: Option<&str>,
    ) -> Result<(), AuthorityError> {
        let mut seen = BTreeSet::new();
        let mut generic_keys = BTreeSet::new();

        for operation in operations {
            if operation.operation_id.is_empty()
                || operation.operation_id.len() > 512
                || operation
                    .operation_id
                    .bytes()
                    .any(|byte| byte.is_ascii_control())
                || !seen.insert(operation.operation_id.clone())
            {
                return Err(AuthorityError::InvalidAuthorityRequest);
            }
            let expected_claim_digest = claim_digest(&operation.claim)?;
            if operation.claim_digest != expected_claim_digest
                || !valid_authority_digest(&operation.claim_digest)
                || !valid_authority_digest(&operation.store_binding_digest)
                || expected_store_binding_digest
                    .is_some_and(|expected| expected != operation.store_binding_digest)
            {
                return Err(AuthorityError::InvalidAuthorityRequest);
            }
            match &operation.claim {
                AuthorityStorageClaim::Generic(claim) => {
                    let request = claim.clone().into_request()?;
                    let owner = (
                        request.owner_proof.resource_uid.to_canonical_string(),
                        request.owner_proof.generation.get(),
                    );
                    if !generic_keys.insert((
                        request.key.clone(),
                        owner,
                        request.arbitration,
                        request.max_holders,
                        request.provider_cardinality,
                        request.dependent_guest.clone(),
                    )) {
                        return Err(AuthorityError::InvalidAuthorityRequest);
                    }
                }
                // A persisted external physical-NIC operation row has no
                // controller-side admission path anymore; refuse it fail-closed.
                AuthorityStorageClaim::ExternalNic(_) => {
                    return Err(AuthorityError::InvalidAuthorityRequest);
                }
            }
        }
        Ok(())
    }

    /// Rehydrate one production index from a private trusted-store receipt.
    pub fn rehydrate(receipt: AuthorityRecoveryReceipt) -> Result<Self, AuthorityError> {
        let mut index = Self::new_unrehydrated();
        Self::validate_recovery_operations(&receipt.operations, None)?;
        index.seen_operation_ids = receipt.seen_operation_ids;
        index.recovery_capabilities = receipt.capabilities;
        for operation in receipt.operations {
            match operation.claim {
                AuthorityStorageClaim::Generic(claim) => {
                    let request = claim.into_request()?;
                    if matches!(
                        operation.state,
                        AuthorityOperationState::Closed | AuthorityOperationState::Released
                    ) {
                        continue;
                    }
                    index.admit_authority_inner_with_operation(
                        request,
                        Some(operation.operation_id.clone()),
                    )?;
                    index.unresolved_operations.insert(operation.operation_id);
                }
                // A persisted external physical-NIC operation row has no
                // controller-side admission path anymore; refuse it fail-closed.

                AuthorityStorageClaim::ExternalNic(_) => {
                    return Err(AuthorityError::InvalidAuthorityRequest);
                }
            }
        }
        index.rehydrated = true;
        index.ready_epoch = index.runtime_epoch.load(Ordering::Acquire);
        Ok(index)
    }

    pub(crate) fn recovery_receipt_from_operations_with_prepared_capabilities(
        operations: Vec<AuthorityStorageOperation>,
        expected_store_binding_digest: Option<&str>,
        prepared_operations: BTreeMap<
            String,
            crate::authority_persistence::PreparedAuthorityOperation,
        >,
    ) -> Result<AuthorityRecoveryReceipt, AuthorityError> {
        Self::validate_recovery_operations(&operations, expected_store_binding_digest)?;
        let active_operation_ids = operations
            .iter()
            .filter(|operation| {
                !matches!(
                    operation.state,
                    AuthorityOperationState::Closed | AuthorityOperationState::Released
                )
            })
            .map(|operation| operation.operation_id.clone())
            .collect::<BTreeSet<_>>();
        if prepared_operations.keys().any(|operation_id| {
            !operations
                .iter()
                .any(|operation| &operation.operation_id == operation_id)
        }) || prepared_operations.len() != active_operation_ids.len()
            || !active_operation_ids
                .iter()
                .all(|operation_id| prepared_operations.contains_key(operation_id))
        {
            return Err(AuthorityError::InvalidAuthorityRequest);
        }
        let capabilities = prepared_operations
            .into_iter()
            .map(|(operation_id, prepared)| {
                let operation = operations
                    .iter()
                    .find(|operation| operation.operation_id == operation_id)
                    .ok_or(AuthorityError::InvalidAuthorityRequest)?;
                if !prepared.matches_operation(operation) {
                    return Err(AuthorityError::InvalidAuthorityRequest);
                }
                let capability =
                    crate::authority_persistence::AuthorityOperationCapability::from_prepared(
                        &operation_id,
                        prepared,
                    )
                    .map_err(|_| AuthorityError::InvalidAuthorityRequest)?;
                Ok((operation_id, capability))
            })
            .collect::<Result<BTreeMap<_, _>, AuthorityError>>()?;
        Ok(AuthorityRecoveryReceipt {
            seen_operation_ids: operations
                .iter()
                .map(|operation| operation.operation_id.clone())
                .collect(),
            operations,
            capabilities,
        })
    }

    /// Whether the startup reservation barrier has completed.
    pub const fn is_rehydrated(&self) -> bool {
        self.rehydrated
    }

    /// Whether every recovered active operation has been observed and resolved.
    pub fn is_ready_for_readiness(&self) -> bool {
        self.rehydrated
            && self.unresolved_operations.is_empty()
            && self.quarantined_operations.is_empty()
            && self.ready_epoch == self.runtime_epoch.load(Ordering::Acquire)
    }

    /// Resolve one recovered operation after the authoritative runtime has
    /// observed its effect or quarantined it.
    pub(crate) fn resolve_recovered_operation(
        &mut self,
        operation_id: &str,
        resolution: AuthorityRecoveryResolution,
    ) -> Result<(), AuthorityError> {
        if !self.seen_operation_ids.contains(operation_id) {
            return Err(AuthorityError::UnknownAuthority);
        }
        self.unresolved_operations.remove(operation_id);
        if matches!(resolution, AuthorityRecoveryResolution::ObservedClosed) {
            let generic_keys = self
                .authorities
                .iter()
                .filter(|(_, entry)| {
                    entry
                        .holders
                        .iter()
                        .any(|holder| holder.operation_id.as_deref() == Some(operation_id))
                })
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            for key in generic_keys {
                if let Some(entry) = self.authorities.get_mut(&key) {
                    entry
                        .holders
                        .retain(|holder| holder.operation_id.as_deref() != Some(operation_id));
                    if entry.holders.is_empty() {
                        self.authorities.remove(&key);
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn take_recovery_capability(
        &mut self,
        operation_id: &str,
    ) -> Option<crate::authority_persistence::AuthorityOperationCapability> {
        self.recovery_capabilities.remove(operation_id)
    }

    pub(crate) fn restore_recovery_capability(
        &mut self,
        operation_id: String,
        capability: crate::authority_persistence::AuthorityOperationCapability,
    ) {
        self.recovery_capabilities.insert(operation_id, capability);
    }

    pub(crate) fn quarantine_recovered_operation(&mut self, operation_id: &str) {
        self.quarantined_operations.insert(operation_id.to_owned());
    }

    /// Snapshot generic typed claims for durable store handoff.
    pub fn durable_claims(&self) -> Vec<DurableAuthorityClaim> {
        self.authorities
            .iter()
            .flat_map(|(key, entry)| {
                entry.holders.iter().map(|holder| DurableAuthorityClaim {
                    scope: key.scope.clone(),
                    class: key.class,
                    opaque_digest: key.opaque_digest.clone(),
                    arbitration: entry.arbitration,
                    max_holders: entry.max_holders as u32,
                    provider_cardinality: entry.provider_cardinality,
                    owner_proof: DurableAuthorityOwnerProof::from_owner_proof(&holder.owner_proof),
                    dependent_guest: holder.dependent_guest.clone(),
                })
            })
            .collect()
    }

    /// Admit one typed authority before invoking any host or Zone effect.
    pub fn admit_authority(
        &mut self,
        request: AuthorityRequest,
    ) -> Result<AuthorityLease, AuthorityError> {
        if !self.is_ready_for_readiness() {
            return Err(AuthorityError::StartupRehydrationRequired);
        }
        self.admit_authority_inner(request)
    }

    fn admit_authority_inner(
        &mut self,
        request: AuthorityRequest,
    ) -> Result<AuthorityLease, AuthorityError> {
        self.admit_authority_inner_with_operation(request, None)
    }

    fn admit_authority_inner_with_operation(
        &mut self,
        request: AuthorityRequest,
        operation_id: Option<String>,
    ) -> Result<AuthorityLease, AuthorityError> {
        let key = request.key.clone();
        let token = self.issue_token();
        if let Some(entry) = self.authorities.get_mut(&key) {
            if entry
                .holders
                .iter()
                .any(|holder| holder.owner_proof == request.owner_proof)
            {
                return Err(AuthorityError::DuplicateActiveReservation);
            }
            if entry.arbitration != request.arbitration {
                return Err(AuthorityError::AuthorityArbitrationConflict);
            }
            if entry.provider_cardinality != request.provider_cardinality {
                return Err(AuthorityError::AuthorityOwnerProofMismatch);
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
                token,
                operation_id: operation_id.clone(),
                owner_proof: request.owner_proof.clone(),
                max_holders: request.max_holders,
                dependent_guest: request.dependent_guest.clone(),
            });
        } else {
            self.authorities.insert(
                key.clone(),
                GenericAuthorityEntry {
                    holders: vec![GenericHolder {
                        token,
                        operation_id: operation_id.clone(),
                        owner_proof: request.owner_proof.clone(),
                        max_holders: request.max_holders,
                        dependent_guest: request.dependent_guest.clone(),
                    }],
                    arbitration: request.arbitration,
                    max_holders: request.max_holders,
                    provider_cardinality: request.provider_cardinality,
                    dependent_guest: request.dependent_guest.clone(),
                },
            );
        }
        Ok(AuthorityLease {
            key,
            owner_proof: request.owner_proof,
            arbitration: request.arbitration,
            max_holders: request.max_holders,
            provider_cardinality: request.provider_cardinality,
            dependent_guest: request.dependent_guest,
            token,
            operation_id,
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
        let indexed = entry.holders.iter().find(|holder| {
            holder.owner_proof == request.owner_proof
                && holder.max_holders == request.max_holders
                && holder.dependent_guest == request.dependent_guest
                && entry.arbitration == request.arbitration
                && entry.provider_cardinality == request.provider_cardinality
        });
        if matching_observations == 1
            && let Some(holder) = indexed
        {
            AuthorityAdoption::Adopted(AuthorityLease {
                key: request.key.clone(),
                owner_proof: request.owner_proof.clone(),
                arbitration: entry.arbitration,
                max_holders: holder.max_holders,
                provider_cardinality: entry.provider_cardinality,
                dependent_guest: holder.dependent_guest.clone(),
                token: holder.token,
                operation_id: holder.operation_id.clone(),
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
            .position(|holder| {
                holder.token == lease.token
                    && holder.owner_proof == lease.owner_proof
                    && holder.dependent_guest == lease.dependent_guest
                    && holder.max_holders == lease.max_holders
                    && entry.arbitration == lease.arbitration
                    && entry.provider_cardinality == lease.provider_cardinality
                    && lease.operation_id.as_ref().is_none_or(|operation_id| {
                        holder.operation_id.as_ref() == Some(operation_id)
                    })
            })
            .ok_or(AuthorityError::AuthorityOwnerProofMismatch)?;
        entry.holders.remove(holder);
        if entry.holders.is_empty() {
            self.authorities.remove(&lease.key);
        }
        Ok(())
    }

    /// Confirm closure of every authority-backed effect before releasing
    /// leases owned by a finalized Guest.
    pub fn close_then_drain_guest(
        &mut self,
        host_uid: &ResourceUid,
        guest_uid: &ResourceUid,
        mut close: impl FnMut(&AuthorityLease) -> AuthorityCloseOutcome,
    ) -> Result<usize, AuthorityError> {
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
        let leases = keys
            .iter()
            .flat_map(|key| {
                self.authorities.get(key).into_iter().flat_map(|entry| {
                    entry.holders.iter().map(|holder| AuthorityLease {
                        key: key.clone(),
                        owner_proof: holder.owner_proof.clone(),
                        arbitration: entry.arbitration,
                        max_holders: holder.max_holders,
                        provider_cardinality: entry.provider_cardinality,
                        dependent_guest: holder.dependent_guest.clone(),
                        token: holder.token,
                        operation_id: holder.operation_id.clone(),
                    })
                })
            })
            .collect::<Vec<_>>();
        for lease in &leases {
            if close(lease) != AuthorityCloseOutcome::Confirmed {
                return Err(AuthorityError::AuthorityCloseUnconfirmed);
            }
        }
        let mut drained = 0;
        for key in keys {
            if let Some(entry) = self.authorities.remove(&key) {
                drained += entry.holders.len();
            }
        }
        Ok(drained)
    }

}

/// Error returned by an asynchronous authority reservation dispatch.
#[derive(Debug, PartialEq, Eq)]
pub enum AuthorityReservationError<E> {
    /// The reservation was already closed.
    Closed,
    /// The broker/effect adapter failed before confirmation.
    Effect(E),
    /// The durable authority owner could not be updated.
    Persistence(crate::authority_persistence::AuthorityPersistenceError),
}

/// A Host-global reservation retained across an asynchronous effect call.
///
/// The lease is inserted into the shared index before `dispatch` is awaited.
/// A caller must invoke [`Self::close_then_release`] only after the effect
/// owner confirms closure; dropping this value does not guess whether a host
/// effect ran.
#[must_use = "an authority reservation must remain owned until effect closure is confirmed"]
pub struct AuthorityReservation {
    index: Arc<tokio::sync::Mutex<HostGlobalAuthorityIndex>>,
    lease: Option<AuthorityLease>,
    outcome: Option<AuthorityEffectOutcome>,
    persistence: Option<Arc<dyn crate::authority_persistence::AuthorityPersistence>>,
    capability: Option<crate::authority_persistence::AuthorityOperationCapability>,
    close_recorded: bool,
}

impl AuthorityReservation {
    /// Reserve one authority before starting an asynchronous effect.
    pub async fn reserve(
        index: Arc<tokio::sync::Mutex<HostGlobalAuthorityIndex>>,
        request: AuthorityRequest,
    ) -> Result<Self, AuthorityError> {
        let lease = index.lock().await.admit_authority(request)?;
        Ok(Self {
            index,
            lease: Some(lease),
            outcome: None,
            persistence: None,
            capability: None,
            close_recorded: false,
        })
    }

    /// Reserve one authority and durably write its pending owner before any
    /// effect dispatch.
    pub async fn reserve_durable(
        index: Arc<tokio::sync::Mutex<HostGlobalAuthorityIndex>>,
        persistence: Arc<dyn crate::authority_persistence::AuthorityPersistence>,
        operation_id: impl Into<String>,
        request: AuthorityRequest,
    ) -> Result<Self, AuthorityReservationError<AuthorityError>> {
        let operation_id = operation_id.into();
        let claim = AuthorityStorageClaim::Generic(request.durable_claim());
        let lease = {
            let mut guard = index.lock().await;
            guard
                .reserve_operation_id(&operation_id)
                .map_err(AuthorityReservationError::Effect)?;
            guard
                .admit_authority_inner_with_operation(request, Some(operation_id.clone()))
                .map_err(AuthorityReservationError::Effect)?
        };
        let prepared = match persistence.prepare(&operation_id, &claim).await {
            Ok(prepared) => prepared,
            Err(error @ crate::authority_persistence::AuthorityPersistenceError::CommitUnknown) => {
                index.lock().await.quarantine_operation_id(&operation_id);
                return Err(AuthorityReservationError::Persistence(error));
            }
            Err(error) => {
                let _ = index.lock().await.release_authority(&lease);
                return Err(AuthorityReservationError::Persistence(error));
            }
        };
        let capability =
            match crate::authority_persistence::AuthorityOperationCapability::from_prepared(
                &operation_id,
                prepared,
            ) {
                Ok(capability) => capability,
                Err(error) => {
                    let _ = index.lock().await.release_authority(&lease);
                    return Err(AuthorityReservationError::Persistence(error));
                }
            };
        Ok(Self {
            index,
            lease: Some(lease),
            outcome: None,
            persistence: Some(persistence),
            capability: Some(capability),
            close_recorded: false,
        })
    }

    /// Dispatch a typed effect while retaining the reservation across await.
    pub async fn dispatch<F, Fut, E>(
        &mut self,
        dispatch: F,
    ) -> Result<AuthorityEffectOutcome, AuthorityReservationError<E>>
    where
        F: FnOnce(&AuthorityLease) -> Fut,
        Fut: Future<Output = Result<AuthorityEffectOutcome, E>>,
    {
        let Some(lease) = self.lease.as_ref() else {
            return Err(AuthorityReservationError::Closed);
        };
        let outcome = dispatch(lease)
            .await
            .map_err(AuthorityReservationError::Effect)?;
        self.outcome = Some(outcome);
        if let Some(persistence) = &self.persistence {
            let state = match outcome {
                AuthorityEffectOutcome::Confirmed => AuthorityOperationState::EffectConfirmed,
                AuthorityEffectOutcome::RetryableFailure => {
                    AuthorityOperationState::EffectRetryable
                }
                AuthorityEffectOutcome::TerminalFailure => AuthorityOperationState::EffectTerminal,
            };
            let capability =
                self.capability
                    .as_ref()
                    .ok_or(AuthorityReservationError::Persistence(
                        crate::authority_persistence::AuthorityPersistenceError::StateInvalid,
                    ))?;
            persistence
                .record_effect(capability, state)
                .await
                .map_err(AuthorityReservationError::Persistence)?;
        }
        Ok(outcome)
    }

    /// Close the host effect and release the reservation only on confirmation.
    pub async fn close_then_release(
        &mut self,
        close: impl FnOnce() -> AuthorityCloseOutcome,
    ) -> Result<(), AuthorityError> {
        let lease = self
            .lease
            .as_ref()
            .ok_or(AuthorityError::ReservationClosed)?;
        if let Some(persistence) = &self.persistence {
            if close() != AuthorityCloseOutcome::Confirmed {
                if let Some(capability) = self.capability.as_ref() {
                    let _ = persistence
                        .record_effect(capability, AuthorityOperationState::EffectRetryable)
                        .await;
                }
                return Err(AuthorityError::AuthorityCloseUnconfirmed);
            }
            let capability = self
                .capability
                .as_ref()
                .ok_or(AuthorityError::ReservationClosed)?;
            if !self.close_recorded {
                persistence
                    .record_close(capability)
                    .await
                    .map_err(|_| AuthorityError::AuthorityCloseUnconfirmed)?;
                self.close_recorded = true;
            }
            persistence
                .release(capability)
                .await
                .map_err(|_| AuthorityError::AuthorityCloseUnconfirmed)?;
            self.index.lock().await.release_authority(lease)?;
            self.lease = None;
        } else {
            self.index
                .lock()
                .await
                .close_then_release_authority(lease, close)?;
        }
        self.lease = None;
        Ok(())
    }

    /// Return the most recent broker/effect outcome, if dispatch has run.
    pub const fn outcome(&self) -> Option<AuthorityEffectOutcome> {
        self.outcome
    }
}

impl core::fmt::Debug for AuthorityReservation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AuthorityReservation")
            .field("has_lease", &self.lease.is_some())
            .field("outcome", &self.outcome)
            .finish()
    }
}

impl core::fmt::Debug for HostGlobalAuthorityIndex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HostGlobalAuthorityIndex")
            .field("authority_count", &self.authorities.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority_persistence::{
        AuthorityFuture, AuthorityPersistence, AuthorityRecoveryData, PreparedAuthorityOperation,
    };
    use std::sync::Mutex;

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).unwrap()
    }

fn authority_proof(value: &str, generation: u64) -> AuthorityOwnerProof {
        AuthorityOwnerProof::new(uid(value), ResourceGeneration::new(generation).unwrap())
    }

    fn digest(byte: u8) -> AuthorityDigest {
        AuthorityDigest([byte; 32])
    }

    #[derive(Default)]
    struct RecordingPersistence {
        states: Mutex<Vec<AuthorityOperationState>>,
    }

    impl AuthorityPersistence for RecordingPersistence {
        // R11 inventory note: sanctioned "cfg(test) helper" exception; the
        // std Mutex capture is short and synchronous, dropped before any await.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn prepare<'a>(
            &'a self,
            operation_id: &'a str,
            _claim: &'a AuthorityStorageClaim,
        ) -> AuthorityFuture<'a, PreparedAuthorityOperation> {
            self.states
                .lock()
                .unwrap()
                .push(AuthorityOperationState::Pending);
            Box::pin(async {
                PreparedAuthorityOperation::new(
                    operation_id.to_owned(),
                    "sha256:".to_owned() + &"1".repeat(64),
                    test_nonce_for_operation(operation_id),
                )
            })
        }

        // R11 inventory note: sanctioned "cfg(test) helper" exception (see above).
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn record_effect<'a>(
            &'a self,
            _capability: &'a crate::authority_persistence::AuthorityOperationCapability,
            state: AuthorityOperationState,
        ) -> AuthorityFuture<'a, ()> {
            self.states.lock().unwrap().push(state);
            Box::pin(async { Ok(()) })
        }

        // R11 inventory note: sanctioned "cfg(test) helper" exception (see above).
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn record_close<'a>(
            &'a self,
            _capability: &'a crate::authority_persistence::AuthorityOperationCapability,
        ) -> AuthorityFuture<'a, ()> {
            self.states
                .lock()
                .unwrap()
                .push(AuthorityOperationState::Closing);
            Box::pin(async { Ok(()) })
        }

        // R11 inventory note: sanctioned "cfg(test) helper" exception (see above).
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn release<'a>(
            &'a self,
            _capability: &'a crate::authority_persistence::AuthorityOperationCapability,
        ) -> AuthorityFuture<'a, ()> {
            self.states
                .lock()
                .unwrap()
                .push(AuthorityOperationState::Released);
            Box::pin(async { Ok(()) })
        }

        fn recover<'a>(&'a self) -> AuthorityFuture<'a, AuthorityRecoveryData> {
            Box::pin(async { Ok(AuthorityRecoveryData::new(Vec::new(), BTreeMap::new())) })
        }
    }

    #[test]
    fn provider_cardinality_is_zone_local_and_effects_are_fail_closed() {
        let mut index = HostGlobalAuthorityIndex::new_for_tests_ready();
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
            "duplicate-conflict"
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

    /// The index refuses an admission before any effect runs until the
    /// startup barrier completes. This is the index's own guard, distinct
    /// from the process-level stage barrier in `main`: the index is shared
    /// into the controller admission surface, so a caller outside the
    /// startup path reaches this refusal too, and a claim on a host
    /// resource whose pre-restart owner set was never loaded must not
    /// succeed.
    #[test]
    fn admission_waits_for_the_same_startup_barrier_before_any_effect() {
        let host = uid("623e4567-e89b-42d3-a456-426614174005");
        let claim = || {
            AuthorityRequest::gpu_full_device(
                host.clone(),
                digest(1),
                authority_proof("723e4567-e89b-42d3-a456-426614174006", 1),
            )
            .unwrap()
        };

        let mut unrehydrated = HostGlobalAuthorityIndex::new_unrehydrated();
        assert!(!unrehydrated.is_ready_for_readiness());
        let mut effects = 0;
        assert_eq!(
            unrehydrated
                .admit_authority_before_effect(claim(), |_| {
                    effects += 1;
                    AuthorityEffectOutcome::Confirmed
                })
                .unwrap_err(),
            AuthorityError::StartupRehydrationRequired
        );
        assert_eq!(effects, 0);
        assert_eq!(
            AuthorityError::StartupRehydrationRequired.code(),
            "authority-startup-rehydration-required"
        );

        // A restart relist invalidates an index that has already
        // admitted claims, and the same barrier refuses again until
        // rehydration loads the durable owner proofs.
        let mut live = HostGlobalAuthorityIndex::new_for_tests_ready();
        live.admit_authority(claim()).unwrap();
        live.invalidate_for_restart();
        let mut effects = 0;
        assert_eq!(
            live.admit_authority_before_effect(claim(), |_| {
                effects += 1;
                AuthorityEffectOutcome::Confirmed
            })
            .unwrap_err(),
            AuthorityError::StartupRehydrationRequired
        );
        assert_eq!(effects, 0);
    }

    #[test]
    fn host_global_hardware_matrix_cannot_be_bypassed_by_zone_or_private_class() {
        let host = uid("623e4567-e89b-42d3-a456-426614174005");
        let mut index = HostGlobalAuthorityIndex::new_for_tests_ready();

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
        let zone = uid("c63e4567-e89b-42d3-a456-426614174035");
        let network = uid("d63e4567-e89b-42d3-a456-426614174036");
        let mut index = HostGlobalAuthorityIndex::new_for_tests_ready();

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
        let mut index = HostGlobalAuthorityIndex::new_for_tests_ready();
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
    fn duplicate_same_owner_reservations_are_rejected_without_aliasing_leases() {
        let host = uid("f93e4567-e89b-42d3-a456-426614174060");
        let owner = authority_proof("a04e4567-e89b-42d3-a456-426614174061", 1);
        let authority_request = AuthorityRequest::vsock_cid(host, 95, owner).unwrap();
        let mut index = HostGlobalAuthorityIndex::new_for_tests_ready();
        let first = index.admit_authority(authority_request.clone()).unwrap();
        assert_eq!(
            index
                .admit_authority(authority_request.clone())
                .unwrap_err(),
            AuthorityError::DuplicateActiveReservation
        );
        index.release_authority(&first).unwrap();
        assert!(index.authority_status(&authority_request).is_none());
    }

    #[test]
    fn lease_tokens_cannot_cross_restart_boundaries() {
        let request = AuthorityRequest::kvm(
            uid("a14e4567-e89b-42d3-a456-426614174067"),
            authority_proof("b14e4567-e89b-42d3-a456-426614174068", 1),
        )
        .unwrap();
        let mut before_restart = HostGlobalAuthorityIndex::new_for_tests_ready();
        let stale = before_restart.admit_authority(request.clone()).unwrap();
        let mut after_restart = HostGlobalAuthorityIndex::new_for_tests_ready();
        let current = after_restart.admit_authority(request).unwrap();
        assert_eq!(
            after_restart.release_authority(&stale).unwrap_err(),
            AuthorityError::AuthorityOwnerProofMismatch
        );
        after_restart.release_authority(&current).unwrap();
    }

    // R11 inventory note: tokio Mutex::lock().await resolves to the banned
    // Runtime::block_on bridge in clippy 1.97; sanctioned "cfg(test) helper".
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reservation_stays_held_across_async_effect_and_closes_before_release() {
        let host = uid("f83e4567-e89b-42d3-a456-426614174050");
        let owner = authority_proof("a93e4567-e89b-42d3-a456-426614174051", 1);
        let request = AuthorityRequest::vsock_cid(host, 91, owner).unwrap();
        let index = std::sync::Arc::new(tokio::sync::Mutex::new(
            HostGlobalAuthorityIndex::new_for_tests_ready(),
        ));
        let mut reservation = AuthorityReservation::reserve(index.clone(), request.clone())
            .await
            .unwrap();

        let competing = AuthorityRequest::vsock_cid(
            uid("f83e4567-e89b-42d3-a456-426614174050"),
            91,
            authority_proof("b93e4567-e89b-42d3-a456-426614174052", 1),
        )
        .unwrap();
        assert_eq!(
            index.lock().await.admit_authority(competing).unwrap_err(),
            AuthorityError::DuplicateConflict
        );

        reservation
            .dispatch(|_| async {
                tokio::task::yield_now().await;
                Ok::<_, ()>(AuthorityEffectOutcome::Confirmed)
            })
            .await
            .unwrap();
        reservation
            .close_then_release(|| AuthorityCloseOutcome::Confirmed)
            .await
            .unwrap();
        assert!(index.lock().await.authority_status(&request).is_none());
    }

    // R11 inventory note: tokio Mutex::lock().await resolves to the banned
    // Runtime::block_on bridge in clippy 1.97; sanctioned "cfg(test) helper".
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn failed_reservation_close_keeps_lease_for_successful_retry() {
        let host = uid("d93e4567-e89b-42d3-a456-426614174053");
        let request = AuthorityRequest::vsock_cid(
            host.clone(),
            93,
            authority_proof("e93e4567-e89b-42d3-a456-426614174054", 1),
        )
        .unwrap();
        let index = std::sync::Arc::new(tokio::sync::Mutex::new(
            HostGlobalAuthorityIndex::new_for_tests_ready(),
        ));
        let mut reservation = AuthorityReservation::reserve(index.clone(), request.clone())
            .await
            .unwrap();
        reservation
            .dispatch(|_| async { Ok::<_, ()>(AuthorityEffectOutcome::Confirmed) })
            .await
            .unwrap();
        assert_eq!(
            reservation
                .close_then_release(|| AuthorityCloseOutcome::RetryableFailure)
                .await,
            Err(AuthorityError::AuthorityCloseUnconfirmed)
        );
        assert_eq!(
            index
                .lock()
                .await
                .authority_status(&request)
                .expect("failed close retains owner")
                .holder_count(),
            1
        );
        reservation
            .close_then_release(|| AuthorityCloseOutcome::Confirmed)
            .await
            .unwrap();
        assert!(index.lock().await.authority_status(&request).is_none());
    }

    // R11 inventory note: clippy 1.97 double-reports the std Mutex::lock()
    // capture chain as both Mutex::lock and the banned Runtime::block_on
    // bridge; sanctioned "cfg(test) helper".
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn durable_reservation_records_pending_effect_close_and_release() {
        let host = uid("f93e4567-e89b-42d3-a456-426614174055");
        let request = AuthorityRequest::vsock_cid(
            host,
            94,
            authority_proof("a04e4567-e89b-42d3-a456-426614174056", 1),
        )
        .unwrap();
        let index = std::sync::Arc::new(tokio::sync::Mutex::new(
            HostGlobalAuthorityIndex::new_for_tests_ready(),
        ));
        let persistence = std::sync::Arc::new(RecordingPersistence::default());
        let mut reservation = AuthorityReservation::reserve_durable(
            index,
            persistence.clone(),
            "authority-operation",
            request,
        )
        .await
        .unwrap();
        reservation
            .dispatch(|_| async { Ok::<_, ()>(AuthorityEffectOutcome::Confirmed) })
            .await
            .unwrap();
        reservation
            .close_then_release(|| AuthorityCloseOutcome::Confirmed)
            .await
            .unwrap();
        assert_eq!(
            *persistence.states.lock().unwrap(),
            vec![
                AuthorityOperationState::Pending,
                AuthorityOperationState::EffectConfirmed,
                AuthorityOperationState::Closing,
                AuthorityOperationState::Released,
            ]
        );
    }

    #[test]
    fn dependent_authority_requires_close_before_finalizer_release() {
        let host = uid("c83e4567-e89b-42d3-a456-426614174047");
        let guest = uid("d83e4567-e89b-42d3-a456-426614174048");
        let request = AuthorityRequest::guest_store_view_writer(
            host.clone(),
            guest.clone(),
            authority_proof("e83e4567-e89b-42d3-a456-426614174049", 1),
        )
        .unwrap();
        let mut index = HostGlobalAuthorityIndex::new_for_tests_ready();
        index.admit_authority(request.clone()).unwrap();

        assert_eq!(
            index
                .close_then_drain_guest(&host, &guest, |_| AuthorityCloseOutcome::RetryableFailure),
            Err(AuthorityError::AuthorityCloseUnconfirmed)
        );
        assert_eq!(index.authority_status(&request).unwrap().holder_count(), 1);
        assert_eq!(
            index
                .close_then_drain_guest(&host, &guest, |_| AuthorityCloseOutcome::Confirmed)
                .unwrap(),
            1
        );
        assert!(index.authority_status(&request).is_none());
    }

    fn operation_row(
        operation_id: &str,
        request: &AuthorityRequest,
    ) -> (AuthorityStorageOperation, PreparedAuthorityOperation) {
        let claim = AuthorityStorageClaim::Generic(request.durable_claim());
        let claim_digest = claim_digest(&claim).unwrap();
        let store_binding_digest = "sha256:".to_owned() + &"1".repeat(64);
        let operation = AuthorityStorageOperation {
            operation_id: operation_id.to_owned(),
            claim,
            state: AuthorityOperationState::Pending,
            claim_digest,
            store_binding_digest: store_binding_digest.clone(),
        };
        let prepared = PreparedAuthorityOperation::new(
            operation_id.to_owned(),
            store_binding_digest,
            test_nonce_for_operation(operation_id),
        )
        .unwrap();
        (operation, prepared)
    }

    #[test]
    fn recovery_receipt_rejects_tampered_claim_digest() {
        let request = AuthorityRequest::vsock_cid(
            uid("a73e4567-e89b-42d3-a456-426614174080"),
            79,
            authority_proof("b73e4567-e89b-42d3-a456-426614174081", 1),
        )
        .unwrap();
        let (mut operation, prepared) = operation_row("recovery-tampered-digest", &request);
        let mut tampered: Vec<char> = operation.claim_digest.chars().collect();
        tampered[63] = if tampered[63] == '0' { '1' } else { '0' };
        operation.claim_digest = tampered.into_iter().collect();
        assert!(matches!(
            HostGlobalAuthorityIndex::recovery_receipt_from_operations_with_prepared_capabilities(
                vec![operation],
                None,
                BTreeMap::from([("recovery-tampered-digest".to_owned(), prepared)]),
            ),
            Err(AuthorityError::InvalidAuthorityRequest)
        ));
    }

    #[test]
    fn recovery_receipt_rejects_prepared_set_missing_active_operation() {
        let request = AuthorityRequest::vsock_cid(
            uid("c73e4567-e89b-42d3-a456-426614174082"),
            81,
            authority_proof("d73e4567-e89b-42d3-a456-426614174083", 1),
        )
        .unwrap();
        let (operation, _) = operation_row("recovery-missing-prepared", &request);
        assert!(matches!(
            HostGlobalAuthorityIndex::recovery_receipt_from_operations_with_prepared_capabilities(
                vec![operation],
                None,
                BTreeMap::new(),
            ),
            Err(AuthorityError::InvalidAuthorityRequest)
        ));
    }

    #[test]
    fn recovery_receipt_rejects_duplicate_operation_id() {
        let host = uid("e73e4567-e89b-42d3-a456-426614174084");
        let first = AuthorityRequest::vsock_cid(
            host.clone(),
            83,
            authority_proof("f73e4567-e89b-42d3-a456-426614174085", 1),
        )
        .unwrap();
        let second = AuthorityRequest::vsock_cid(
            host,
            84,
            authority_proof("a83e4567-e89b-42d3-a456-426614174086", 1),
        )
        .unwrap();
        let (first_operation, _) = operation_row("recovery-duplicate-id", &first);
        let (second_operation, _) = operation_row("recovery-duplicate-id", &second);
        assert!(matches!(
            HostGlobalAuthorityIndex::recovery_receipt_from_operations_with_prepared_capabilities(
                vec![first_operation, second_operation],
                None,
                BTreeMap::new(),
            ),
            Err(AuthorityError::InvalidAuthorityRequest)
        ));
    }

    #[test]
    fn rehydrate_round_trip_admits_recovered_operation_and_reaches_readiness() {
        let owner = authority_proof("c83e4567-e89b-42d3-a456-426614174088", 1);
        let request = AuthorityRequest::vsock_cid(
            uid("b83e4567-e89b-42d3-a456-426614174087"),
            86,
            owner.clone(),
        )
        .unwrap();
        let (operation, prepared) = operation_row("recovery-round-trip", &request);
        let receipt =
            HostGlobalAuthorityIndex::recovery_receipt_from_operations_with_prepared_capabilities(
                vec![operation],
                None,
                BTreeMap::from([("recovery-round-trip".to_owned(), prepared)]),
            )
            .unwrap();
        let mut index = HostGlobalAuthorityIndex::rehydrate(receipt).unwrap();
        assert!(index.is_rehydrated());
        assert_eq!(index.authority_status(&request).unwrap().holder_count(), 1);
        assert!(matches!(
            index.adopt_authority(&request, core::slice::from_ref(&owner)),
            AuthorityAdoption::Adopted(_)
        ));
        assert!(!index.is_ready_for_readiness());
        index
            .resolve_recovered_operation(
                "recovery-round-trip",
                AuthorityRecoveryResolution::ObservedAndAdopted,
            )
            .unwrap();
        assert!(index.is_ready_for_readiness());
    }
}
