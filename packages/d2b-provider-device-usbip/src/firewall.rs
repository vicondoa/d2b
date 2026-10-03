//! Opaque USBIP firewall and relay effect boundary.
//!
//! The boundary has two shapes. [`UsbipEffectPort`] is the pre-graph seam the
//! not-yet-cutover production path uses: a Core-derived relay authority and a
//! per-Network/per-device projection, with the physical backing decided by
//! this crate's own claim table. U34 deletes it.
//!
//! [`UsbipClaimPort`] is the converted seam. It carries one admitted
//! [`AdmittedDeviceClaim`] into every effect, so a relay, a host bind, or a
//! firewall rule exists only as a bounded realization of a relationship the
//! `Device` source arbitrated, and the projection is additionally fenced
//! against the store incarnation that admission was evaluated
//! under ([`ClaimProjectionFence`]). Releasing the relationship is the
//! source's reservation to give up, so it is a distinct step that runs only
//! after the relay has been stopped and the projection removed.

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingRefusal, RefusalReason, ResourceGeneration, ResourceRef, ResourceUid,
    StoreIncarnation,
};

use crate::arbitration::AdmittedDeviceClaim;

/// Closed direction of one ownership-scoped firewall projection mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallProjectionAction {
    /// Install or converge the resolved projection.
    Apply,
    /// Remove only the resolved projection.
    Remove,
}

impl FirewallProjectionAction {
    /// Parse the exact semantic action spelling.
    pub fn parse(value: &str) -> Result<Self, UsbipEffectError> {
        match value {
            "Apply" => Ok(Self::Apply),
            "Remove" => Ok(Self::Remove),
            _ => Err(UsbipEffectError::UnknownProjectionAction),
        }
    }

    /// Return the exact semantic action spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Apply => "Apply",
            Self::Remove => "Remove",
        }
    }
}

/// Expected resource generations for one projection mutation.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallGenerationFence {
    network_generation: ResourceGeneration,
    service_generation: ResourceGeneration,
}

impl FirewallGenerationFence {
    /// Bind an effect to the exact Network and USB Service generations read by
    /// the controller.
    pub const fn new(
        network_generation: ResourceGeneration,
        service_generation: ResourceGeneration,
    ) -> Self {
        Self {
            network_generation,
            service_generation,
        }
    }

    /// Return the expected Network generation.
    pub const fn network_generation(&self) -> ResourceGeneration {
        self.network_generation
    }

    /// Return the expected USB Service generation.
    pub const fn service_generation(&self) -> ResourceGeneration {
        self.service_generation
    }
}

impl core::fmt::Debug for FirewallGenerationFence {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FirewallGenerationFence(<redacted>)")
    }
}

/// Opaque exact per-Network/per-device projection mutation.
///
/// Core resolves these resource identities through its trusted private bundle.
/// No rule text, ownership marker, interface, address, port, or bus id crosses
/// this boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallProjectionIntent {
    device_uid: ResourceUid,
    network_uid: ResourceUid,
    action: FirewallProjectionAction,
    expected: FirewallGenerationFence,
}

impl FirewallProjectionIntent {
    /// Construct one exact opaque projection mutation.
    pub const fn new(
        device_uid: ResourceUid,
        network_uid: ResourceUid,
        action: FirewallProjectionAction,
        expected: FirewallGenerationFence,
    ) -> Self {
        Self {
            device_uid,
            network_uid,
            action,
            expected,
        }
    }

    /// Borrow the opaque Device identity for the Core adapter.
    pub const fn device_uid(&self) -> &ResourceUid {
        &self.device_uid
    }

    /// Borrow the opaque Network identity for the Core adapter.
    pub const fn network_uid(&self) -> &ResourceUid {
        &self.network_uid
    }

    /// Return the requested closed action.
    pub const fn action(&self) -> FirewallProjectionAction {
        self.action
    }

    /// Borrow the resource-generation fence.
    pub const fn expected(&self) -> &FirewallGenerationFence {
        &self.expected
    }
}

impl core::fmt::Debug for FirewallProjectionIntent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FirewallProjectionIntent")
            .field("action", &self.action)
            .field("expected", &self.expected)
            .finish()
    }
}

/// Opaque adapter-issued token proving ownership of one applied projection.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallToken([u8; 16]);

impl FirewallToken {
    /// Construct a token at the trusted effect adapter boundary.
    pub const fn from_adapter(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl core::fmt::Debug for FirewallToken {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FirewallToken(<redacted>)")
    }
}

/// Opaque ownership-scoped digest returned by the effect adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallDigest([u8; 32]);

impl FirewallDigest {
    /// Construct a digest at the trusted effect adapter boundary.
    pub const fn from_adapter(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl core::fmt::Debug for FirewallDigest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FirewallDigest(<redacted>)")
    }
}

/// Closed successful effect result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallConfirmationKind {
    /// The projection was installed or already matched desired state.
    Applied,
    /// The owned projection was removed.
    Removed,
    /// Ownership was validated and the projection was already absent.
    ValidatedAbsent,
}

/// Successful, ownership-scoped firewall confirmation.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallConfirmation {
    kind: FirewallConfirmationKind,
    token: Option<FirewallToken>,
    digest: Option<FirewallDigest>,
}

impl FirewallConfirmation {
    /// Confirm an applied projection and return the retained token and digest.
    pub const fn applied(token: FirewallToken, digest: FirewallDigest) -> Self {
        Self {
            kind: FirewallConfirmationKind::Applied,
            token: Some(token),
            digest: Some(digest),
        }
    }

    /// Confirm removal of the exact owned projection.
    pub const fn removed() -> Self {
        Self {
            kind: FirewallConfirmationKind::Removed,
            token: None,
            digest: None,
        }
    }

    /// Confirm idempotent, ownership-validated absence.
    pub const fn validated_absent() -> Self {
        Self {
            kind: FirewallConfirmationKind::ValidatedAbsent,
            token: None,
            digest: None,
        }
    }

    /// Return the closed result kind.
    pub const fn kind(&self) -> FirewallConfirmationKind {
        self.kind
    }

    /// Consume the confirmation into an applied token and digest.
    pub fn into_applied(self) -> Option<(FirewallToken, FirewallDigest)> {
        match (self.kind, self.token, self.digest) {
            (FirewallConfirmationKind::Applied, Some(token), Some(digest)) => Some((token, digest)),
            _ => None,
        }
    }
}

impl core::fmt::Debug for FirewallConfirmation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FirewallConfirmation")
            .field("kind", &self.kind)
            .field("has_token", &self.token.is_some())
            .field("has_digest", &self.digest.is_some())
            .finish()
    }
}

/// Ownership-scoped firewall observation.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallObservation {
    matches_expected: bool,
    digest: FirewallDigest,
}

impl FirewallObservation {
    /// Construct one projection-only observation.
    pub const fn new(matches_expected: bool, digest: FirewallDigest) -> Self {
        Self {
            matches_expected,
            digest,
        }
    }

    /// Whether the exact USBIP ownership projection matches desired state.
    pub const fn matches_expected(&self) -> bool {
        self.matches_expected
    }

    /// Borrow the opaque projection digest.
    pub const fn digest(&self) -> &FirewallDigest {
        &self.digest
    }
}

impl core::fmt::Debug for FirewallObservation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FirewallObservation")
            .field("matches_expected", &self.matches_expected)
            .field("digest", &self.digest)
            .finish()
    }
}

/// Opaque lease on the Core-derived per-Network relay Endpoint authority.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayAuthorityLease([u8; 16]);

impl RelayAuthorityLease {
    /// Construct a lease at the trusted Core authority adapter boundary.
    pub const fn from_adapter(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl core::fmt::Debug for RelayAuthorityLease {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RelayAuthorityLease(<redacted>)")
    }
}

/// The fence one admitted-relationship projection mutation is measured
/// against.
///
/// The generations are the resource facts a projection converges with; the
/// store incarnation is the authority fact. A claim admitted before a store
/// replacement is not the same authority as one admitted after it, so a
/// projection written under the previous incarnation is refused instead of
/// being re-applied over a relationship the source has since re-decided.
#[derive(Clone, PartialEq, Eq)]
pub struct ClaimProjectionFence {
    network_generation: ResourceGeneration,
    service_generation: ResourceGeneration,
    store: StoreIncarnation,
}

impl ClaimProjectionFence {
    /// Bind one projection mutation to the claim's own evidence.
    ///
    /// # Errors
    ///
    /// Returns [`RefusalReason::StaleAuthority`] when the claim was admitted
    /// under a different store incarnation, which is the case where older
    /// evidence must not decide this projection any more.
    pub fn new(
        claim: &AdmittedDeviceClaim,
        network_generation: ResourceGeneration,
        service_generation: ResourceGeneration,
        store: &StoreIncarnation,
    ) -> Result<Self, BindingRefusal> {
        if claim.epoch() != Some(store) {
            return Err(BindingRefusal::new(
                AdmissionStage::Reserve,
                RefusalReason::StaleAuthority,
            ));
        }
        Ok(Self {
            network_generation,
            service_generation,
            store: store.clone(),
        })
    }

    /// Return the expected Network generation.
    pub const fn network_generation(&self) -> ResourceGeneration {
        self.network_generation
    }

    /// Return the expected USB Service generation.
    pub const fn service_generation(&self) -> ResourceGeneration {
        self.service_generation
    }

    /// Return the store incarnation this projection is fenced against.
    pub const fn store(&self) -> &StoreIncarnation {
        &self.store
    }
}

impl core::fmt::Debug for ClaimProjectionFence {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ClaimProjectionFence(<redacted>)")
    }
}

/// The effect boundary one admitted `Device` relationship drives.
///
/// Every method takes the admitted claim, so an implementation privately
/// resolves the relationship's source reservation, ownership marker, host
/// module, listener, and firewall intent from the graph rather than from a
/// resource identity the Provider chose. The Provider receives no broker DTO,
/// host identity, rule text, or device path, and it holds no device claim: the
/// `Device` source reserved the capability and this port only realizes it.
pub trait UsbipClaimPort {
    /// Start the per-Network relay as a bounded leg of the admitted claim.
    ///
    /// A second owner fails here, before any listener or firewall effect.
    fn start_relay_leg(
        &mut self,
        claim: &AdmittedDeviceClaim,
        helper: &ResourceRef,
        network_uid: &ResourceUid,
        fence: &ClaimProjectionFence,
    ) -> Result<RelayAuthorityLease, UsbipEffectError>;

    /// Apply or remove the exact resolved ownership projection.
    fn mutate_claim_firewall(
        &mut self,
        claim: &AdmittedDeviceClaim,
        network_uid: &ResourceUid,
        action: FirewallProjectionAction,
        fence: &ClaimProjectionFence,
        retained_token: Option<&FirewallToken>,
    ) -> Result<FirewallConfirmation, UsbipEffectError>;

    /// Observe only the exact projection the retained token represents.
    fn observe_claim_firewall(
        &mut self,
        claim: &AdmittedDeviceClaim,
        network_uid: &ResourceUid,
        fence: &ClaimProjectionFence,
        token: &FirewallToken,
    ) -> Result<FirewallObservation, UsbipEffectError>;

    /// Stop the relay leg while the reservation is still held.
    ///
    /// The relay has to be down before the relationship is handed back: a
    /// live listener holding a released reservation is the same use-after-free
    /// as a mount that outlives its volume.
    fn stop_relay_leg(
        &mut self,
        claim: &AdmittedDeviceClaim,
        helper: &ResourceRef,
    ) -> Result<(), UsbipEffectError>;

    /// Hand the relationship back so the source can release its reservation.
    ///
    /// This is the last step of teardown: it runs only after the projection is
    /// removed or validated absent and the relay leg is stopped.
    fn release_claim(&mut self, claim: &AdmittedDeviceClaim) -> Result<(), UsbipEffectError>;
}

/// Closed effect failures with no caller-controlled payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipEffectError {
    /// Device and Network do not belong to one Zone.
    WrongZone,
    /// The dependency was not Ready.
    NetworkNotReady,
    /// The dependency assignment no longer matches this controller.
    StaleAssignment,
    /// A second owner attempted to create the Network relay authority.
    RelayAuthorityConflict,
    /// Effect may be retried with all authority retained.
    Transient,
    /// The installed generation differs; dependencies must be refreshed.
    FirewallGenerationMismatch,
    /// A foreign ownership marker blocked safe mutation.
    FirewallForeignConflict,
    /// The effect adapter rejected the request terminally.
    EffectRejected,
    /// A caller attempted a value outside the closed action set.
    UnknownProjectionAction,
    /// The `Device` source refused the relationship, or the leg the helper
    /// presented is not a bounded realization of it.
    ///
    /// The pair names the enforcing stage and the reason under R42; no
    /// resource identity, path, or device detail crosses this boundary.
    ClaimRefused(AdmissionStage, RefusalReason),
}

impl UsbipEffectError {
    /// Return the stable closed error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::WrongZone => "wrong-zone",
            Self::NetworkNotReady => "network-not-ready",
            Self::StaleAssignment => "usbip-assignment-stale",
            Self::RelayAuthorityConflict => "usbip-network-relay-authority-conflict",
            Self::Transient => "transient",
            Self::FirewallGenerationMismatch => "firewall-generation-mismatch",
            Self::FirewallForeignConflict => "firewall-foreign-conflict",
            Self::EffectRejected => "effect-rejected",
            Self::UnknownProjectionAction => "unknown-projection-action",
            Self::ClaimRefused(_, _) => "device-claim-refused",
        }
    }
}

impl core::fmt::Display for UsbipEffectError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for UsbipEffectError {}

/// Injected semantic boundary implemented by Core.
///
/// Implementations privately resolve the shared projection request, installed
/// generation, ownership marker, host module, listener, and firewall intent.
/// The Provider receives no broker DTO or host identity.
pub trait UsbipEffectPort {
    /// Acquire or share the one multiplexed relay Endpoint authority for a
    /// Network. A second owner fails before listener or firewall effects.
    fn acquire_relay(
        &mut self,
        network_uid: &ResourceUid,
    ) -> Result<RelayAuthorityLease, UsbipEffectError>;

    /// Apply or remove the exact resolved ownership projection.
    fn mutate_firewall(
        &mut self,
        intent: &FirewallProjectionIntent,
        retained_token: Option<&FirewallToken>,
    ) -> Result<FirewallConfirmation, UsbipEffectError>;

    /// Observe only the exact USBIP projection represented by the token.
    fn observe_firewall(
        &mut self,
        intent: &FirewallProjectionIntent,
        token: &FirewallToken,
    ) -> Result<FirewallObservation, UsbipEffectError>;

    /// Release a relay authority after the last projection removal is confirmed.
    fn release_relay(&mut self, lease: RelayAuthorityLease) -> Result<(), UsbipEffectError>;
}
