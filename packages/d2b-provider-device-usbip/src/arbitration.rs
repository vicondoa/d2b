//! Device claim arbitration for USBIP.
//!
//! A USB Service used to be the arbiter of its own physical backing: the
//! exclusive/shared ceiling, the claim conflict, and the release all lived in
//! [`UsbipArbitrator`] here, and the host bind followed whatever this table
//! said. That is a second authority beside the graph - a semantic Service
//! deciding which consumer holds a device, over a Core-derived backing token
//! no consumer could be checked against.
//!
//! The `Device` source arbitrates now. This module's converted surface does
//! three things and nothing else:
//!
//! - it *requests* the relationship. [`usbip_service_device_request`] is the
//!   canonical `DeviceBindingRequest` a USB Service's physical backing claim
//!   is: a named capability, one consumer, one stable slot, and the claim mode
//!   the Service declares. The request is the only thing the semantic
//!   resource authors; it grants nothing (AE30).
//! - it *reads* the admitted result. [`AdmittedDeviceClaim`] is a read-only
//!   view of the relationship the `Device` source admitted: the shared
//!   `BindingEvidence`, the named capability and opaque physical authority the
//!   source's trusted inventory resolved, and the operation classes the
//!   admission carries. There is no constructor that takes a declaration, so
//!   nothing in this crate can admit its own claim (AE8).
//! - it *verifies* the bounded leg a helper realizes.
//!   [`BoundDeviceLeg`] is a read-only view of the leg the source bound to
//!   one helper, and [`AdmittedDeviceClaim::verify_helper_leg`] refuses
//!   anything that is not an attenuated realization of this claim's own
//!   reservation (AE27).
//!
//! [`UsbipArbitrator`] stays for the not-yet-cutover production path and is
//! queued for deletion with the rest of the old wiring in U34; nothing in the
//! converted surface below calls it.

use core::fmt;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingContractError, BindingEvidence, BindingKey, BindingLifecycleState,
    BindingRealizationFacet, BindingRealizationSupport, BindingRefusal, BindingSlot,
    DeviceAttachmentMode, DeviceAuthorityKey, DeviceBindingRequest, DeviceClaimRequest,
    DeviceEffectOperation, DeviceFunction, RefusalReason, ResourceRef, ResourceUid,
    SourceReservation, StoreIncarnation, ZoneId,
    device::DeviceArbitration,
};

use crate::busid::PhysicalUsbBackingToken;

/// The stable consumer slot a USB Service's physical backing claim occupies.
///
/// The slot is the relationship's identity (KTD3), so it is a constant of the
/// family's declaration rather than an attachment order or a list index.
pub const USBIP_SERVICE_DEVICE_SLOT: &str = "usb-backing";

/// The named capability a USB Service claims on its backing `Device`.
///
/// The `Device` provider resolves this name against its trusted inventory; the
/// family cannot reach a device node, a bus id, or a serial to widen it.
pub const USBIP_BACKING_FUNCTION: &str = "usb-bus";

/// Build the canonical `DeviceBindingRequest` one USB Service's physical
/// backing claim is.
///
/// The consumer is the Service's own controller `Process`: the Host backend
/// and the per-Network relay are provider-created helpers, so they realize
/// this claim as bounded legs rather than claiming the device themselves.
/// `claim` is the claim mode the Service's declaration asks for; whether the
/// source admits it is the source's decision, not this function's.
///
/// # Errors
///
/// Returns [`BindingContractError`] when the source is not a `Device` or the
/// claim mode is not one the `Device` binding kind admits.
pub fn usbip_service_device_request(
    device_ref: &ResourceRef,
    claim: DeviceClaimRequest,
) -> Result<DeviceBindingRequest, BindingContractError> {
    DeviceBindingRequest::new(
        device_ref.clone(),
        usbip_service_controller_ref(),
        usbip_service_device_slot()?,
        DeviceFunction::parse(USBIP_BACKING_FUNCTION)
            .map_err(|_| BindingContractError::InvalidField)?,
        claim,
        DeviceAttachmentMode::Mediated,
    )
}

/// The one `Process` that consumes a USB Service's `Device` claim.
fn usbip_service_controller_ref() -> ResourceRef {
    ResourceRef::parse(crate::driver::USBIP_SERVICE_CONTROLLER_REF)
        .expect("the USBIP Service controller reference is canonical")
}

/// The stable slot a USB Service's device claim occupies.
fn usbip_service_device_slot() -> Result<BindingSlot, BindingContractError> {
    BindingSlot::parse(USBIP_SERVICE_DEVICE_SLOT).map_err(|_| BindingContractError::InvalidField)
}

/// The bounded `Device` reservation leg one helper realizes.
///
/// The `Device` source mints this leg: it names the parent's relationship and
/// reservation, the helper's own identity, the exact capability and physical
/// authority the parent holds, the operation subset the helper may drive, and
/// the store incarnation the admission was fenced against. This trait is a
/// read-only view of that leg rather than a second leg type, so the source's
/// own leg implements it directly and the family holds the graph's leg rather
/// than a copy of it.
pub trait BoundDeviceLeg {
    /// The admitted relationship this leg realizes.
    fn parent_key(&self) -> &BindingKey;

    /// The source-owned reservation this leg rides.
    fn reservation(&self) -> &SourceReservation;

    /// The helper's own resource reference.
    fn helper_ref(&self) -> &ResourceRef;

    /// The helper's own store-assigned identity.
    fn helper_uid(&self) -> &ResourceUid;

    /// The named capability this leg reaches.
    fn function(&self) -> &DeviceFunction;

    /// The exact physical authority this leg reaches.
    fn authority_key(&self) -> &DeviceAuthorityKey;

    /// The operation classes this leg may drive.
    fn operations(&self) -> &[DeviceEffectOperation];

    /// The store incarnation this leg is fenced against.
    fn epoch(&self) -> &StoreIncarnation;

    /// Whether this leg holds a device claim of its own.
    ///
    /// A bounded realization leg never does. A family refuses any leg that
    /// says it does, which is what stops a helper from becoming a competing
    /// allocation against the parent's authority.
    fn holds_claim(&self) -> bool;
}

/// One admitted `Device` relationship this Provider realizes.
///
/// The view is exactly what the `Device` source decided: the shared
/// `BindingEvidence` (relationship key, admission, source-owned reservation,
/// and observed lifecycle), the named capability and opaque physical
/// authority its trusted inventory resolved, and the effect operation classes
/// the admission carries. No device node path, bus id, serial, host permission
/// bit, or numerical principal is reachable from it, and there is no
/// constructor that takes a declaration - the family can only be handed a
/// relationship the source already admitted.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedDeviceClaim {
    evidence: BindingEvidence,
    function: DeviceFunction,
    authority: DeviceAuthorityKey,
    operations: Vec<DeviceEffectOperation>,
}

impl AdmittedDeviceClaim {
    /// Wrap one admitted relationship as this family's realization view.
    ///
    /// # Errors
    ///
    /// Returns [`RefusalReason::MandatoryFacetUnsupported`] when the
    /// admission carries no effect operation class: a relationship the family
    /// could not drive is not a relationship it may realize.
    pub fn new(
        evidence: BindingEvidence,
        function: DeviceFunction,
        authority: DeviceAuthorityKey,
        operations: Vec<DeviceEffectOperation>,
    ) -> Result<Self, BindingRefusal> {
        let mut operations = operations;
        operations.sort_unstable();
        operations.dedup();
        if operations.is_empty() {
            return Err(BindingRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::MandatoryFacetUnsupported,
            ));
        }
        Ok(Self {
            evidence,
            function,
            authority,
            operations,
        })
    }

    /// Borrow the shared admitted-relationship evidence.
    pub const fn evidence(&self) -> &BindingEvidence {
        &self.evidence
    }

    /// Borrow the KTD3 identity of this relationship.
    pub const fn key(&self) -> &BindingKey {
        self.evidence.key()
    }

    /// Borrow the source-owned reservation this claim holds.
    pub const fn reservation(&self) -> &SourceReservation {
        self.evidence.reservation()
    }

    /// The named capability the trusted inventory resolved for this claim.
    pub const fn function(&self) -> &DeviceFunction {
        &self.function
    }

    /// The opaque physical authority the trusted inventory resolved.
    pub const fn authority_key(&self) -> &DeviceAuthorityKey {
        &self.authority
    }

    /// The effect operation classes this relationship may drive.
    pub fn operations(&self) -> &[DeviceEffectOperation] {
        &self.operations
    }

    /// The lifecycle the relationship is currently observed in.
    pub const fn state(&self) -> BindingLifecycleState {
        self.evidence.state()
    }

    /// Whether the relationship still admits new use.
    ///
    /// A revoking, draining, or unproven relationship is not free for another
    /// consumer either; what this decides is whether *this* consumer may keep
    /// driving effects, and a relationship whose effectiveness cannot be
    /// proven is not that.
    pub const fn admits_new_use(&self) -> bool {
        self.evidence.state().admits_new_use() && self.evidence.state().proves_effect()
    }

    /// The store incarnation this claim was admitted under.
    ///
    /// Returns `None` for an admission that carried no dependency fence,
    /// which is not an unchecked admission: every leg check refuses it.
    pub fn epoch(&self) -> Option<&StoreIncarnation> {
        self.evidence
            .admission()
            .dependencies()
            .first()
            .map(|dependency| dependency.store_incarnation())
    }

    /// Whether this claim drives one operation class.
    pub fn covers(&self, operation: DeviceEffectOperation) -> bool {
        self.operations.contains(&operation)
    }

    /// Check that this claim is the relationship a USB Service's own row
    /// declares, in the Zone that row lives in.
    ///
    /// A claim minted for another Zone, for another backing `Device`, or for
    /// another consumer is refused here, before any relay, host bind, or
    /// firewall effect is attempted.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionStage::Admit`] with
    /// [`RefusalReason::SourcePolicyRefused`] when the Zone, the source, the
    /// consumer, or the slot is not the one this Service declares, and
    /// [`AdmissionStage::Authorize`] with [`RefusalReason::StaleAuthority`]
    /// when the relationship no longer admits new use.
    pub fn verify_service_claim(
        &self,
        zone: &ZoneId,
        device_ref: &ResourceRef,
    ) -> Result<(), BindingRefusal> {
        let expected_slot = usbip_service_device_slot().map_err(|_| {
            BindingRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::SourcePolicyRefused,
            )
        })?;
        if self.key().zone() != zone
            || self.key().source_ref() != device_ref
            || self.key().consumer_ref() != &usbip_service_controller_ref()
            || self.key().slot() != &expected_slot
        {
            return Err(BindingRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::SourcePolicyRefused,
            ));
        }
        if !self.admits_new_use() {
            return Err(BindingRefusal::new(
                AdmissionStage::Authorize,
                RefusalReason::StaleAuthority,
            ));
        }
        Ok(())
    }

    /// Check that `leg` is a bounded realization of *this* claim.
    ///
    /// The leg must name this relationship and ride this claim's own
    /// reservation, must name this helper, must reach the capability and
    /// physical authority this claim holds, must be fenced against the store
    /// incarnation the claim was admitted under, must drive at least the
    /// required operation classes, and must hold no claim of its own. Anything
    /// else is refused: a leg is an explicitly attenuated realization, not a
    /// second allocation and not a different source.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionStage::Reserve`] with a
    /// [`RefusalReason::StaleAuthority`] or
    /// [`RefusalReason::ConflictingDeclaration`] refusal when the leg is
    /// fenced against another store incarnation, rides another reservation,
    /// or claims the device itself, and [`AdmissionStage::Authorize`] with a
    /// [`RefusalReason::RequiredCapabilityOutsideCeiling`] or
    /// [`RefusalReason::MandatoryFacetUnsupported`] refusal when the leg
    /// widens the claim.
    pub fn verify_helper_leg<L: BoundDeviceLeg + ?Sized>(
        &self,
        helper_ref: &ResourceRef,
        helper_uid: &ResourceUid,
        required: &[DeviceEffectOperation],
        leg: &L,
    ) -> Result<(), BindingRefusal> {
        if leg.holds_claim() {
            return Err(BindingRefusal::new(
                AdmissionStage::Reserve,
                RefusalReason::ConflictingDeclaration,
            ));
        }
        if leg.parent_key() != self.key() || leg.reservation() != self.reservation() {
            return Err(BindingRefusal::new(
                AdmissionStage::Reserve,
                RefusalReason::StaleAuthority,
            ));
        }
        if self.epoch() != Some(leg.epoch()) {
            return Err(BindingRefusal::new(
                AdmissionStage::Reserve,
                RefusalReason::StaleAuthority,
            ));
        }
        if leg.helper_ref() != helper_ref || leg.helper_uid() != helper_uid {
            return Err(BindingRefusal::new(
                AdmissionStage::Authorize,
                RefusalReason::SourcePolicyRefused,
            ));
        }
        if leg.function() != self.function() || leg.authority_key() != self.authority_key() {
            return Err(BindingRefusal::new(
                AdmissionStage::Authorize,
                RefusalReason::RequiredCapabilityOutsideCeiling,
            ));
        }
        if required.is_empty()
            || required
                .iter()
                .any(|operation| !self.covers(*operation) || !leg.operations().contains(operation))
        {
            return Err(BindingRefusal::new(
                AdmissionStage::Authorize,
                RefusalReason::MandatoryFacetUnsupported,
            ));
        }
        Ok(())
    }

    /// The realization facet this family's relationship depends on.
    pub const fn required_facet() -> BindingRealizationFacet {
        BindingRealizationFacet::DeviceAttachment
    }

    /// The realization support one USBIP relationship is admitted against.
    ///
    /// Every device binding is delivered as a verified descriptor or a mediated
    /// attachment, so the closed support set is the single attachment facet: a
    /// family cannot widen its realization by naming a different one.
    ///
    /// # Errors
    ///
    /// Never returns an error; the single attachment facet is a valid support
    /// set by construction.
    pub fn support() -> Result<BindingRealizationSupport, BindingContractError> {
        BindingRealizationSupport::new(vec![AdmittedDeviceClaim::required_facet()])
    }
}

impl fmt::Debug for AdmittedDeviceClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedDeviceClaim")
            .field("function", &self.function)
            .field("state", &self.state())
            .field("operations", &self.operations)
            .finish_non_exhaustive()
    }
}

/// Closed USBIP Device claim failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipClaimError {
    /// The backing authority did not match the expected Device.
    PhysicalBackingConflict,
    /// An exclusive Device already has a claimant.
    ClaimConflict,
    /// The configured claim ceiling was reached.
    MaxClaimsExceeded,
    /// The requested arbitration and claim mode disagree.
    ArbitrationViolation,
}

impl UsbipClaimError {
    /// Return the stable Device error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::PhysicalBackingConflict => "physical-usb-backing-conflict",
            Self::ClaimConflict => "device-claim-conflict",
            Self::MaxClaimsExceeded => "device-claim-max-exceeded",
            Self::ArbitrationViolation => "device-arbitration-violation",
        }
    }
}

impl fmt::Display for UsbipClaimError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for UsbipClaimError {}

/// A bounded USBIP claimant record.
#[derive(Clone, PartialEq, Eq)]
pub struct UsbipClaim {
    holder: ResourceUid,
    backing: PhysicalUsbBackingToken,
}

impl UsbipClaim {
    /// Borrow the opaque holder identity.
    pub const fn holder(&self) -> &ResourceUid {
        &self.holder
    }

    /// Borrow the Core-derived backing token.
    pub const fn backing(&self) -> &PhysicalUsbBackingToken {
        &self.backing
    }
}

impl fmt::Debug for UsbipClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UsbipClaim(<redacted>)")
    }
}

/// Exclusive/shared Device claim arbiter.
///
/// This table is the pre-graph path: it decides a claim conflict from a
/// Core-derived backing token without consulting any admitted relationship, so
/// a semantic Service arbitrates its own device. The converted surface is
/// [`AdmittedDeviceClaim`]; U34 deletes this type and its production caller
/// with the rest of the old wiring.
pub struct UsbipArbitrator {
    arbitration: DeviceArbitration,
    max_claims: u32,
    backing: PhysicalUsbBackingToken,
    claims: Vec<UsbipClaim>,
}

impl UsbipArbitrator {
    /// Construct an arbiter after validating the Device claim ceiling.
    ///
    /// # Errors
    ///
    /// Returns [`UsbipClaimError::ArbitrationViolation`] when the claim
    /// ceiling is outside `1..=16` or an exclusive arbitration requests a
    /// ceiling other than one.
    pub fn new(
        arbitration: DeviceArbitration,
        max_claims: u32,
        backing: PhysicalUsbBackingToken,
    ) -> Result<Self, UsbipClaimError> {
        if !(1..=16).contains(&max_claims)
            || (arbitration == DeviceArbitration::Exclusive && max_claims != 1)
        {
            return Err(UsbipClaimError::ArbitrationViolation);
        }
        Ok(Self {
            arbitration,
            max_claims,
            backing,
            claims: Vec::new(),
        })
    }

    /// Admit one claimant before any bind, module, firewall, or relay effect.
    pub fn claim(
        &mut self,
        holder: ResourceUid,
        backing: PhysicalUsbBackingToken,
    ) -> Result<(), UsbipClaimError> {
        if backing != self.backing {
            return Err(UsbipClaimError::PhysicalBackingConflict);
        }
        if self.claims.iter().any(|claim| claim.holder == holder) {
            return Ok(());
        }
        if self.arbitration == DeviceArbitration::Exclusive && !self.claims.is_empty() {
            return Err(UsbipClaimError::ClaimConflict);
        }
        if self.claims.len() >= self.max_claims as usize {
            return Err(UsbipClaimError::MaxClaimsExceeded);
        }
        self.claims.push(UsbipClaim { holder, backing });
        Ok(())
    }

    /// Release one exact claimant.
    pub fn release(&mut self, holder: &ResourceUid) -> bool {
        let before = self.claims.len();
        self.claims.retain(|claim| &claim.holder != holder);
        before != self.claims.len()
    }

    /// Return the current claimant count.
    pub const fn claim_count(&self) -> usize {
        self.claims.len()
    }

    /// Borrow the bounded claimant list.
    pub fn claims(&self) -> &[UsbipClaim] {
        &self.claims
    }
}

impl fmt::Debug for UsbipArbitrator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UsbipArbitrator")
            .field("arbitration", &self.arbitration)
            .field("max_claims", &self.max_claims)
            .field("claim_count", &self.claims.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).unwrap()
    }

    fn backing() -> PhysicalUsbBackingToken {
        PhysicalUsbBackingToken::from_core([7; 32])
    }

    #[test]
    fn constructor_rejects_ceilings_outside_one_to_sixteen() {
        let rejected = [
            (DeviceArbitration::Shared, 0u32),
            (DeviceArbitration::Shared, 17),
            (DeviceArbitration::Exclusive, 0),
            (DeviceArbitration::Exclusive, 2),
        ];
        for (arbitration, max_claims) in rejected {
            assert_eq!(
                UsbipArbitrator::new(arbitration, max_claims, backing()).unwrap_err(),
                UsbipClaimError::ArbitrationViolation,
                "ceiling {max_claims} under {arbitration:?} must be rejected",
            );
        }
        for (arbitration, max_claims) in [
            (DeviceArbitration::Shared, 1u32),
            (DeviceArbitration::Shared, 16),
            (DeviceArbitration::Exclusive, 1),
        ] {
            assert!(
                UsbipArbitrator::new(arbitration, max_claims, backing()).is_ok(),
                "ceiling {max_claims} under {arbitration:?} must be accepted",
            );
        }
    }

    #[test]
    fn claim_paths_follow_the_arbitration_mode_and_ceiling() {
        let table = [
            (
                DeviceArbitration::Exclusive,
                1u32,
                Ok(()),
                Err(UsbipClaimError::ClaimConflict),
                Err(UsbipClaimError::ClaimConflict),
            ),
            (
                DeviceArbitration::Shared,
                1u32,
                Ok(()),
                Err(UsbipClaimError::MaxClaimsExceeded),
                Err(UsbipClaimError::MaxClaimsExceeded),
            ),
            (
                DeviceArbitration::Shared,
                2u32,
                Ok(()),
                Ok(()),
                Err(UsbipClaimError::MaxClaimsExceeded),
            ),
        ];
        for (arbitration, ceiling, first, second, third) in table {
            let mut arbiter = UsbipArbitrator::new(arbitration, ceiling, backing()).unwrap();
            assert_eq!(arbiter.claim(uid("123e4567-e89b-42d3-a456-426614174000"), backing()), first);
            assert_eq!(arbiter.claim(uid("223e4567-e89b-42d3-a456-426614174001"), backing()), second);
            assert_eq!(arbiter.claim(uid("323e4567-e89b-42d3-a456-426614174002"), backing()), third);
        }
    }

    #[test]
    fn same_holder_reclaim_is_idempotent() {
        let mut arbiter = UsbipArbitrator::new(DeviceArbitration::Shared, 2, backing()).unwrap();
        let holder = uid("123e4567-e89b-42d3-a456-426614174000");
        assert_eq!(arbiter.claim(holder.clone(), backing()), Ok(()));
        assert_eq!(arbiter.claim(holder, backing()), Ok(()));
        assert_eq!(arbiter.claim_count(), 1);
    }

    #[test]
    fn release_removes_exactly_the_named_claimant_and_frees_the_slot() {
        let mut arbiter = UsbipArbitrator::new(DeviceArbitration::Shared, 2, backing()).unwrap();
        let first = uid("123e4567-e89b-42d3-a456-426614174000");
        let second = uid("223e4567-e89b-42d3-a456-426614174001");
        arbiter.claim(first.clone(), backing()).unwrap();
        arbiter.claim(second.clone(), backing()).unwrap();
        assert!(arbiter.release(&first));
        assert_eq!(arbiter.claim_count(), 1);
        assert!(!arbiter.release(&first));
        assert_eq!(arbiter.claim(first, backing()), Ok(()));
        assert_eq!(arbiter.claim_count(), 2);
    }

    #[test]
    fn claim_rejects_a_mismatched_backing_token() {
        let mut arbiter = UsbipArbitrator::new(DeviceArbitration::Shared, 1, backing()).unwrap();
        assert_eq!(
            arbiter.claim(
                uid("123e4567-e89b-42d3-a456-426614174000"),
                PhysicalUsbBackingToken::from_core([9; 32]),
            ),
            Err(UsbipClaimError::PhysicalBackingConflict)
        );
        assert_eq!(arbiter.claim_count(), 0);
    }
}
