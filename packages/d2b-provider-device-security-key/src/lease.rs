//! Single-session security-key lease state machine.
//!
//! The pre-graph lease is the Core-admission path: [`SecurityKeyLease::acquire`]
//! and [`SecurityKeyLease::acquire_authorized`] take a `SecurityKeyAdmission`,
//! claim the Host physical backing themselves, and open the hidraw node from
//! that claim. U34 deletes those two methods and
//! [`SecurityKeyEffectPort::claim_physical_backing`] with the rest of the old
//! wiring.
//!
//! The converted path is [`SecurityKeyLease::acquire_bound`]. The security-key
//! Service does not hold a device claim of its own: the `Device` source
//! arbitrated one [`AdmittedDeviceClaim`], the Host relay is a bounded
//! [`BoundDeviceLeg`] of that relationship, and the hidraw open happens only
//! while both hold. The physical authority the lease presents comes from the
//! source's resolved inventory rather than from a caller-supplied token, so a
//! semantic Binding cannot name a device into existence, and teardown stops
//! the relay before the source reservation is handed back.

use core::fmt;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingContractError, BindingEvidence, BindingKey, BindingLifecycleState,
    BindingRealizationFacet, BindingRealizationSupport, BindingRefusal, BindingSlot,
    DeviceAttachmentMode, DeviceAuthorityKey, DeviceBindingRequest, DeviceClaimRequest,
    DeviceEffectOperation, DeviceFunction, RefusalReason, ResourceRef, ResourceUid,
    SourceReservation, StoreIncarnation, ZoneId,
};

use crate::authority::{
    PhysicalAuthorityLease, PhysicalUsbBackingClaim, PhysicalUsbBackingToken, RelayLaunchTicket,
    SecurityKeyAdmission, SecurityKeyEffectError, SecurityKeyEffectPort, SecurityKeyOpenIntent,
};

/// The stable consumer slot a security-key Service's device claim occupies.
pub const SECURITY_KEY_DEVICE_SLOT: &str = "hidraw-device";

/// The named capability a security-key Service claims on its backing `Device`.
pub const SECURITY_KEY_HIDRAW_FUNCTION: &str = "hidraw";

/// The effect operation classes a security-key realization drives.
pub const SECURITY_KEY_RELAY_OPERATIONS: [DeviceEffectOperation; 2] = [
    DeviceEffectOperation::SecurityKeyOpenDevice,
    DeviceEffectOperation::SecurityKeyApplyUdevRules,
];

/// Build the canonical `DeviceBindingRequest` one security-key Service's
/// hidraw claim is.
///
/// The consumer is the Service's own controller `Process`; the Host relay and
/// the in-guest frontend are provider-created helpers that realize this claim
/// as bounded legs. A security-key Binding never claims the device: it requests
/// an Endpoint relationship to the relay, and the Service's exclusive claim is
/// what that relationship rides (AE30).
///
/// # Errors
///
/// Returns [`BindingContractError`] when the source is not a `Device` or the
/// claim mode is not one the `Device` binding kind admits.
pub fn security_key_device_request(
    device_ref: &ResourceRef,
) -> Result<DeviceBindingRequest, BindingContractError> {
    DeviceBindingRequest::new(
        device_ref.clone(),
        security_key_service_controller_ref(),
        BindingSlot::parse(SECURITY_KEY_DEVICE_SLOT).map_err(|_| BindingContractError::InvalidField)?,
        DeviceFunction::parse(SECURITY_KEY_HIDRAW_FUNCTION)
            .map_err(|_| BindingContractError::InvalidField)?,
        DeviceClaimRequest::Exclusive,
        DeviceAttachmentMode::Descriptor,
    )
}

/// The one `Process` that consumes a security-key Service's `Device` claim.
fn security_key_service_controller_ref() -> ResourceRef {
    ResourceRef::parse(crate::driver::SECURITY_KEY_SERVICE_CONTROLLER_REF)
        .expect("the security-key Service controller reference is canonical")
}

/// Opaque security-key session identity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecurityKeySessionId([u8; 16]);

impl SecurityKeySessionId {
    /// Construct an ID at the relay/Core boundary.
    pub const fn from_core(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for SecurityKeySessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecurityKeySessionId(<redacted>)")
    }
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
    /// A bounded realization leg never does, and a family refuses any leg that
    /// says it does.
    fn holds_claim(&self) -> bool;
}

/// One admitted `Device` relationship the security-key Service realizes.
///
/// The view is exactly what the `Device` source decided: the shared
/// `BindingEvidence`, the named capability and opaque physical authority its
/// trusted inventory resolved, and the effect operation classes the admission
/// carries. No hidraw path, device node, host permission bit, or numerical
/// principal is reachable from it, and no constructor takes a declaration - a
/// semantic Service or Binding can only be handed a relationship the source
/// already admitted.
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
    /// Returns [`RefusalReason::MandatoryFacetUnsupported`] when the admission
    /// carries no effect operation class.
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
    pub const fn admits_new_use(&self) -> bool {
        self.evidence.state().admits_new_use() && self.evidence.state().proves_effect()
    }

    /// The store incarnation this claim was admitted under.
    ///
    /// Returns `None` for an admission with no dependency fence, which is not an
    /// unchecked admission: every leg check refuses it.
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

    /// Check that this claim is the relationship a security-key Service's own
    /// row declares, in the Zone that row lives in.
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
        let expected_slot = BindingSlot::parse(SECURITY_KEY_DEVICE_SLOT)
            .map_err(|_| {
                BindingRefusal::new(
                    AdmissionStage::Admit,
                    RefusalReason::SourcePolicyRefused,
                )
            })?;
        if self.key().zone() != zone
            || self.key().source_ref() != device_ref
            || self.key().consumer_ref() != &security_key_service_controller_ref()
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
    /// required operation classes, and must hold no claim of its own.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionStage::Reserve`] with
    /// [`RefusalReason::StaleAuthority`] or
    /// [`RefusalReason::ConflictingDeclaration`] when the leg is fenced against
    /// another store incarnation, rides another reservation, or claims the
    /// device itself, and [`AdmissionStage::Authorize`] with
    /// [`RefusalReason::RequiredCapabilityOutsideCeiling`] or
    /// [`RefusalReason::MandatoryFacetUnsupported`] when the leg widens the
    /// claim.
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

    /// The physical backing tuple this claim presents to the effect boundary.
    ///
    /// The token is the source's resolved physical authority, not a
    /// caller-supplied one, so a semantic request cannot point the hidraw open
    /// at a different node than the one the `Device` source arbitrated.
    fn backing_claim(&self, zone: &ZoneId, holder: &ResourceRef) -> PhysicalUsbBackingClaim {
        PhysicalUsbBackingClaim::from_admission(SecurityKeyAdmission::from_core(
            ResourceRef::parse(format!("Zone/{}", zone.as_str()).as_str())
                .expect("a Zone identity renders a canonical reference"),
            self.key().source_uid().clone(),
            holder.clone(),
            PhysicalUsbBackingToken::from_core(*self.authority.as_bytes()),
        ))
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

/// The facts one security-key Service row declares about its admitted claim.
///
/// The lease checks its claim against exactly these - the Zone the row lives
/// in, the backing `Device` it names, the `Guest` whose Binding rides the
/// relationship, and the helper whose bounded leg may realize it - so the
/// acquire entry point takes one admission value rather than a list of
/// independently supplied arguments.
#[derive(Debug, Clone, Copy)]
pub struct SecurityKeyClaimRequest<'a> {
    zone: &'a ZoneId,
    device_ref: &'a ResourceRef,
    holder: &'a ResourceRef,
    helper: &'a ResourceRef,
    helper_uid: &'a ResourceUid,
}

impl<'a> SecurityKeyClaimRequest<'a> {
    /// Bind one admitted claim to the facts the Service row declares.
    pub const fn new(
        zone: &'a ZoneId,
        device_ref: &'a ResourceRef,
        holder: &'a ResourceRef,
        helper: &'a ResourceRef,
        helper_uid: &'a ResourceUid,
    ) -> Self {
        Self {
            zone,
            device_ref,
            holder,
            helper,
            helper_uid,
        }
    }

    /// The `Guest` whose Binding rides this relationship.
    pub const fn holder(&self) -> &'a ResourceRef {
        self.holder
    }
}

/// The effect boundary one admitted `Device` relationship drives.
///
/// The hidraw open and the teardown both take the admitted claim, so the
/// Provider never claims a device, never names a node, and never decides that
/// it may keep the key: it realizes what the `Device` source admitted and hands
/// the relationship back when the session ends.
pub trait SecurityKeyClaimPort {
    /// Open the exact hidraw node as a bounded leg of the admitted claim.
    fn open_hidraw_leg(
        &mut self,
        claim: &AdmittedDeviceClaim,
        helper: &ResourceRef,
        intent: &SecurityKeyOpenIntent,
    ) -> Result<RelayLaunchTicket, SecurityKeyEffectError>;

    /// Stop the relay that could still reach the key.
    ///
    /// This runs while the reservation is held, never after it is released.
    fn stop_relay_leg(
        &mut self,
        claim: &AdmittedDeviceClaim,
        helper: &ResourceRef,
        ticket: RelayLaunchTicket,
    ) -> Result<(), SecurityKeyEffectError>;

    /// Hand the relationship back so the source can release its reservation.
    fn release_claim(
        &mut self,
        claim: &AdmittedDeviceClaim,
    ) -> Result<(), SecurityKeyEffectError>;
}

/// Lease lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseState {
    /// No session owns the Device.
    Idle,
    /// A Guest request is waiting for the authority.
    AwaitingLease,
    /// The relay has the active physical lease.
    Active,
    /// The session ended normally.
    Completed,
    /// The session was cancelled.
    Cancelled,
    /// The bounded timeout expired.
    Expired,
}

/// Closed lease failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityKeyLeaseError {
    /// The controller already has an active or waiting session.
    SessionConflict,
    /// The physical backing authority rejected the request.
    Effect(SecurityKeyEffectError),
    /// A transition was requested from the wrong state.
    InvalidTransition,
    /// Core-bound Device, Zone, or holder evidence did not match.
    AuthorizationDenied,
    /// The `Device` source refused the relationship, or the leg the helper
    /// presented is not a bounded realization of it.
    ///
    /// The pair names the enforcing stage and the reason under R42; no
    /// resource identity, path, or device detail crosses this boundary.
    ClaimRefused(AdmissionStage, RefusalReason),
}

impl SecurityKeyLeaseError {
    /// Return the stable error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::SessionConflict => "device-claim-conflict",
            Self::Effect(error) => error.code(),
            Self::InvalidTransition => "device-session-invalid-transition",
            Self::AuthorizationDenied => "device-authority-denied",
            Self::ClaimRefused(_, _) => "device-claim-refused",
        }
    }
}

impl fmt::Display for SecurityKeyLeaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for SecurityKeyLeaseError {}

/// Active lease state held by one Device controller.
pub struct SecurityKeyLease {
    holder: ResourceUid,
    backing: Option<PhysicalUsbBackingClaim>,
    authorized_device: Option<ResourceUid>,
    authorized_holder: Option<ResourceRef>,
    state: LeaseState,
    session: Option<SecurityKeySessionId>,
    authority_lease: Option<PhysicalAuthorityLease>,
    relay_ticket: Option<RelayLaunchTicket>,
    claim: Option<AdmittedDeviceClaim>,
    relay_helper: Option<ResourceRef>,
    source_released: bool,
}

impl SecurityKeyLease {
    /// Construct an idle Device lease.
    pub fn new(holder: ResourceUid, backing: PhysicalUsbBackingClaim) -> Self {
        Self {
            holder,
            backing: Some(backing),
            authorized_device: None,
            authorized_holder: None,
            state: LeaseState::Idle,
            session: None,
            authority_lease: None,
            relay_ticket: None,
            claim: None,
            relay_helper: None,
            source_released: false,
        }
    }

    /// Construct a lease bound to one Core-admitted Device and holder.
    pub fn new_authorized(
        device_uid: ResourceUid,
        admission: SecurityKeyAdmission,
    ) -> Result<Self, SecurityKeyLeaseError> {
        if admission.device_uid() != &device_uid
            || admission.zone_ref().resource_type().as_str() != "Zone"
            || admission.holder_ref().resource_type().as_str() != "Guest"
        {
            tracing::warn!(
                device = %device_uid.to_canonical_string(),
                reason = "admission device, zone, or holder binding mismatch",
                "security-key authorized lease construction refused",
            );
            return Err(SecurityKeyLeaseError::AuthorizationDenied);
        }
        let holder = device_uid.clone();
        let authorized_holder = admission.holder_ref().clone();
        Ok(Self {
            holder,
            backing: Some(admission.into_claim()),
            authorized_device: Some(device_uid),
            authorized_holder: Some(authorized_holder),
            state: LeaseState::Idle,
            session: None,
            authority_lease: None,
            relay_ticket: None,
            claim: None,
            relay_helper: None,
            source_released: false,
        })
    }

    /// Return the current lifecycle state.
    pub const fn state(&self) -> LeaseState {
        self.state
    }

    /// Borrow the opaque holder identity.
    pub const fn holder(&self) -> &ResourceUid {
        &self.holder
    }

    /// Borrow the active session ID, if present.
    pub const fn session(&self) -> Option<&SecurityKeySessionId> {
        self.session.as_ref()
    }

    /// Borrow the admitted `Device` claim this lease realizes, once one has
    /// been accepted.
    pub const fn claim(&self) -> Option<&AdmittedDeviceClaim> {
        self.claim.as_ref()
    }

    /// Borrow the helper whose bounded leg realizes the retained claim.
    pub const fn relay_helper(&self) -> Option<&ResourceRef> {
        self.relay_helper.as_ref()
    }

    /// Whether the source reservation has been handed back.
    pub const fn source_released(&self) -> bool {
        self.source_released
    }

    /// Start a session as a bounded realization of one admitted `Device` claim.
    ///
    /// This is the converted acquire path. It takes no caller-supplied physical
    /// token: the hidraw open is driven by the claim the `Device` source
    /// arbitrated and by the leg that claim bound to the Host relay, both of
    /// which are checked here before the port is called. A semantic Binding
    /// therefore cannot bypass the source's arbitration - there is no path in
    /// this crate that claims the device - and a relay that is not a bounded leg
    /// of the claim cannot open the node at all (AE8, AE27, AE30).
    ///
    /// The `Guest` in `admitted` is the one whose Binding rides this
    /// relationship; it is carried into the open request so the effect boundary
    /// can attribute the session, and it never selects the device.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityKeyLeaseError::ClaimRefused`] when the claim or the leg
    /// is refused, [`SecurityKeyLeaseError::SessionConflict`] when the lease
    /// already holds an active or unfinished session,
    /// [`SecurityKeyLeaseError::InvalidTransition`] when the claim is presented
    /// in a terminal state, and [`SecurityKeyLeaseError::Effect`] when the
    /// bounded open fails.
    pub fn acquire_bound<L, P>(
        &mut self,
        session: SecurityKeySessionId,
        admitted: &SecurityKeyClaimRequest<'_>,
        leg: &L,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError>
    where
        L: BoundDeviceLeg + ?Sized,
        P: SecurityKeyClaimPort,
    {
        let SecurityKeyClaimRequest {
            zone,
            device_ref,
            holder,
            helper,
            helper_uid,
        } = *admitted;
        let claim = self.claim.clone().ok_or_else(|| {
            tracing::warn!(
                reason = "no admitted device claim is retained",
                "security-key bounded acquire refused: claim missing",
            );
            SecurityKeyLeaseError::ClaimRefused(
                AdmissionStage::Admit,
                RefusalReason::SourcePolicyRefused,
            )
        })?;
        claim
            .verify_service_claim(zone, device_ref)
            .map_err(|refusal| self.claim_refused(refusal))?;
        claim
            .verify_helper_leg(helper, helper_uid, &SECURITY_KEY_RELAY_OPERATIONS, leg)
            .map_err(|refusal| self.claim_refused(refusal))?;
        if self.relay_helper.as_ref() != Some(helper) {
            return Err(self.claim_refused(BindingRefusal::new(
                AdmissionStage::Authorize,
                RefusalReason::SourcePolicyRefused,
            )));
        }
        if !matches!(
            self.state,
            LeaseState::Idle | LeaseState::Completed | LeaseState::Cancelled | LeaseState::Expired
        ) || self.session.is_some()
        {
            tracing::warn!(
                device = %self.holder.to_canonical_string(),
                reason = "lease already holds an active or unfinished session",
                "security-key bounded acquire refused: session conflict",
            );
            return Err(SecurityKeyLeaseError::SessionConflict);
        }
        self.state = LeaseState::AwaitingLease;
        let backing = claim.backing_claim(zone, holder);
        let intent =
            SecurityKeyOpenIntent::from_core(claim.key().source_uid().clone(), session, backing.clone());
        let ticket = match port.open_hidraw_leg(&claim, helper, &intent) {
            Ok(ticket) => ticket,
            Err(error) => {
                // The claim belongs to the source, so a failed open consumes
                // nothing: the reservation is still held and stays this
                // Service's, and the lease is immediately retryable.
                self.state = LeaseState::Idle;
                tracing::warn!(
                    device = %self.holder.to_canonical_string(),
                    error = %error,
                    "security-key bounded hidraw open failed",
                );
                return Err(SecurityKeyLeaseError::Effect(error));
            }
        };
        self.session = Some(session);
        self.relay_ticket = Some(ticket);
        self.authorized_device = Some(claim.key().source_uid().clone());
        self.authorized_holder = Some(holder.clone());
        self.backing = Some(backing);
        self.state = LeaseState::Active;
        Ok(())
    }

    /// Complete the active bounded session: stop the relay, then release the
    /// claim.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityKeyLeaseError::InvalidTransition`] when no bounded
    /// session is active and [`SecurityKeyLeaseError::Effect`] when the relay
    /// stop or the claim release does not confirm.
    pub fn complete_bound<P: SecurityKeyClaimPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        self.finish_bound(LeaseState::Completed, port)
    }

    /// Cancel the active bounded session: stop the relay, then release the
    /// claim.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityKeyLeaseError::InvalidTransition`] when no bounded
    /// session is active and [`SecurityKeyLeaseError::Effect`] when the relay
    /// stop or the claim release does not confirm.
    pub fn cancel_bound<P: SecurityKeyClaimPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        self.finish_bound(LeaseState::Cancelled, port)
    }

    /// Expire the active bounded session: stop the relay, then release the
    /// claim.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityKeyLeaseError::InvalidTransition`] when no bounded
    /// session is active and [`SecurityKeyLeaseError::Effect`] when the relay
    /// stop or the claim release does not confirm.
    pub fn expire_bound<P: SecurityKeyClaimPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        self.finish_bound(LeaseState::Expired, port)
    }

    /// Start a session, claiming physical authority before opening hidraw.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityKeyLeaseError::SessionConflict`] when the lease
    /// already holds an active or unfinished session,
    /// [`SecurityKeyLeaseError::AuthorizationDenied`] when no physical
    /// backing claim remains, [`SecurityKeyLeaseError::Effect`] when the
    /// physical backing claim or hidraw open fails, and
    /// [`SecurityKeyLeaseError::InvalidTransition`] when the authority lease
    /// disappears during cleanup.
    pub fn acquire<P: SecurityKeyEffectPort>(
        &mut self,
        session: SecurityKeySessionId,
        device_uid: ResourceUid,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        if !matches!(
            self.state,
            LeaseState::Idle | LeaseState::Completed | LeaseState::Cancelled | LeaseState::Expired
        ) || self.session.is_some()
        {
            tracing::warn!(
                device = %self.holder.to_canonical_string(),
                reason = "lease already holds an active or unfinished session",
                "security-key session acquire rejected: session conflict",
            );
            return Err(SecurityKeyLeaseError::SessionConflict);
        }
        let backing = self
            .backing
            .clone()
            .ok_or(SecurityKeyLeaseError::AuthorizationDenied)
            .inspect_err(|_| {
                tracing::warn!(
                    device = %self.holder.to_canonical_string(),
                    reason = "admission evidence consumed; no physical backing claim remains",
                    "security-key session acquire refused: authorization denied",
                );
            })?;
        self.state = LeaseState::AwaitingLease;
        let authority_lease = match port.claim_physical_backing(&backing) {
            Ok(lease) => lease,
            Err(error) => {
                self.state = LeaseState::Idle;
                tracing::warn!(
                    device = %self.holder.to_canonical_string(),
                    error = %error,
                    "security-key physical backing claim failed",
                );
                return Err(SecurityKeyLeaseError::Effect(error));
            }
        };
        self.authority_lease = Some(authority_lease);
        let intent = SecurityKeyOpenIntent::from_core(device_uid, session, backing.clone());
        let relay_ticket = match port.open_hidraw(&intent) {
            Ok(ticket) => ticket,
            Err(error) => {
                let authority = self
                    .authority_lease
                    .as_ref()
                    .cloned()
                    .ok_or(SecurityKeyLeaseError::InvalidTransition)?;
                tracing::warn!(
                    device = %self.holder.to_canonical_string(),
                    error = %error,
                    "security-key hidraw open failed during session start",
                );
                if let Err(release_error) = port.release_physical_backing(authority) {
                    // Keep the authority lease and remain non-reacquirable
                    // until Core confirms its release. Reacquiring here
                    // would permit two owners after a partial cleanup.
                    self.state = LeaseState::AwaitingLease;
                    tracing::warn!(
                        device = %self.holder.to_canonical_string(),
                        error = %release_error,
                        "security-key physical backing release failed after open failure",
                    );
                    return Err(SecurityKeyLeaseError::Effect(release_error));
                }
                self.authority_lease = None;
                self.state = LeaseState::Idle;
                return Err(SecurityKeyLeaseError::Effect(error));
            }
        };
        self.session = Some(session);
        self.relay_ticket = Some(relay_ticket);
        self.state = LeaseState::Active;
        Ok(())
    }

    /// Start a session after rechecking the exact Core Device and holder
    /// binding. The check happens before any physical claim or hidraw open.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityKeyLeaseError::AuthorizationDenied`] when the
    /// device or holder binding differs from the admission, and the same
    /// errors as [`Self::acquire`] otherwise.
    pub fn acquire_authorized<P: SecurityKeyEffectPort>(
        &mut self,
        session: SecurityKeySessionId,
        device_uid: ResourceUid,
        holder: &ResourceRef,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        if self.authorized_device.as_ref() != Some(&device_uid)
            || self.authorized_holder.as_ref() != Some(holder)
        {
            tracing::warn!(
                device = %device_uid.to_canonical_string(),
                holder = %holder.to_canonical_string(),
                reason = "device or holder binding differs from the admission",
                "security-key authorized session acquire refused",
            );
            return Err(SecurityKeyLeaseError::AuthorizationDenied);
        }
        self.acquire(session, device_uid, port)
    }

    /// Replace consumed admission evidence with a fresh Core admission.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityKeyLeaseError::AuthorizationDenied`] when the lease
    /// is not in a terminal state, still holds a session or authority lease,
    /// or the admission does not match the device, Zone, or Guest holder.
    pub fn rebind_authorized(
        &mut self,
        device_uid: ResourceUid,
        admission: SecurityKeyAdmission,
    ) -> Result<(), SecurityKeyLeaseError> {
        if !matches!(
            self.state,
            LeaseState::Completed | LeaseState::Cancelled | LeaseState::Expired
        ) || self.session.is_some()
            || self.authority_lease.is_some()
            || admission.device_uid() != &device_uid
            || admission.zone_ref().resource_type().as_str() != "Zone"
            || admission.holder_ref().resource_type().as_str() != "Guest"
        {
            tracing::warn!(
                device = %device_uid.to_canonical_string(),
                reason = "lease state, session, authority, or admission binding mismatch",
                "security-key admission rebind refused",
            );
            return Err(SecurityKeyLeaseError::AuthorizationDenied);
        }
        let holder = admission.holder_ref().clone();
        self.holder = device_uid.clone();
        self.backing = Some(admission.into_claim());
        self.authorized_device = Some(device_uid);
        self.authorized_holder = Some(holder);
        self.state = LeaseState::Idle;
        Ok(())
    }

    /// Complete the active session and release its authority.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityKeyLeaseError::InvalidTransition`] when no session
    /// is active and [`SecurityKeyLeaseError::Effect`] when the physical
    /// backing release fails.
    pub fn complete<P: SecurityKeyEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        self.finish(LeaseState::Completed, port)
    }

    /// Cancel the active session and release its authority.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityKeyLeaseError::InvalidTransition`] when no session
    /// is active and [`SecurityKeyLeaseError::Effect`] when the physical
    /// backing release fails.
    pub fn cancel<P: SecurityKeyEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        self.finish(LeaseState::Cancelled, port)
    }

    /// Expire the active session and release its authority.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityKeyLeaseError::InvalidTransition`] when no session
    /// is active and [`SecurityKeyLeaseError::Effect`] when the physical
    /// backing release fails.
    pub fn expire<P: SecurityKeyEffectPort>(
        &mut self,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        self.finish(LeaseState::Expired, port)
    }

    /// Accept the one `Device` claim this Service realizes, in the Zone the
    /// Service row lives in.
    ///
    /// The claim is the source's reservation, not a grant taken here. What is
    /// refused is a claim for another Zone, for another backing `Device`, for a
    /// relationship this Service does not declare, one admitted under another
    /// store incarnation, a revoking or unproven one, and a second claim while
    /// another is still retained - so a reappearing device cannot be adopted
    /// under a foreign owner's evidence while a session is live (AE27, R41).
    ///
    /// # Errors
    ///
    /// Returns [`SecurityKeyLeaseError::ClaimRefused`] naming the enforcing
    /// stage and reason for every one of those cases.
    pub fn admit_relay_claim(
        &mut self,
        zone: &ZoneId,
        device_ref: &ResourceRef,
        store: &StoreIncarnation,
        helper: &ResourceRef,
        claim: &AdmittedDeviceClaim,
    ) -> Result<(), SecurityKeyLeaseError> {
        claim
            .verify_service_claim(zone, device_ref)
            .map_err(|refusal| self.claim_refused(refusal))?;
        if claim.epoch() != Some(store) {
            return Err(self.claim_refused(BindingRefusal::new(
                AdmissionStage::Reserve,
                RefusalReason::StaleAuthority,
            )));
        }
        if self.claim.as_ref().is_some_and(|current| current != claim) {
            tracing::warn!(
                device = %self.holder.to_canonical_string(),
                reason = "another device claim is still retained",
                "security-key claim refused: conflicting relationship",
            );
            return Err(SecurityKeyLeaseError::ClaimRefused(
                AdmissionStage::Reserve,
                RefusalReason::ConflictingDeclaration,
            ));
        }
        self.claim = Some(claim.clone());
        self.relay_helper = Some(helper.clone());
        self.holder = claim.key().source_uid().clone();
        Ok(())
    }

    /// Stop the relay that could still reach the key, then hand the claim back.
    ///
    /// The order is the contract: the reservation stays held until the relay is
    /// down, so there is no window in which a stopped claim is still serving
    /// traffic or a live relay is holding a released one.
    fn finish_bound<P: SecurityKeyClaimPort>(
        &mut self,
        terminal: LeaseState,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        if self.state != LeaseState::Active {
            tracing::debug!(
                device = %self.holder.to_canonical_string(),
                state = ?self.state,
                reason = "bounded finish requested outside the Active phase",
                "security-key bounded session finish refused",
            );
            return Err(SecurityKeyLeaseError::InvalidTransition);
        }
        let Some(claim) = self.claim.clone() else {
            return Err(SecurityKeyLeaseError::InvalidTransition);
        };
        let helper = self
            .relay_helper
            .clone()
            .ok_or(SecurityKeyLeaseError::InvalidTransition)?;
        if let Some(ticket) = self.relay_ticket.take() {
            port.stop_relay_leg(&claim, &helper, ticket).map_err(|error| {
                tracing::warn!(
                    device = %self.holder.to_canonical_string(),
                    error = %error,
                    "security-key relay stop failed during session finish",
                );
                SecurityKeyLeaseError::Effect(error)
            })?;
        }
        port.release_claim(&claim).map_err(|error| {
            tracing::warn!(
                device = %self.holder.to_canonical_string(),
                error = %error,
                "security-key claim release failed during session finish",
            );
            SecurityKeyLeaseError::Effect(error)
        })?;
        self.claim = None;
        self.relay_helper = None;
        self.relay_ticket = None;
        self.session = None;
        self.authority_lease = None;
        self.backing = None;
        self.authorized_device = None;
        self.authorized_holder = None;
        self.source_released = true;
        self.state = terminal;
        Ok(())
    }

    /// Record one claim refusal and report it as this lease's failure.
    fn claim_refused(&mut self, refusal: BindingRefusal) -> SecurityKeyLeaseError {
        tracing::warn!(
            device = %self.holder.to_canonical_string(),
            stage = ?refusal.stage(),
            reason = ?refusal.reason(),
            "security-key device claim refused",
        );
        SecurityKeyLeaseError::ClaimRefused(refusal.stage(), refusal.reason())
    }

    fn finish<P: SecurityKeyEffectPort>(
        &mut self,
        terminal: LeaseState,
        port: &mut P,
    ) -> Result<(), SecurityKeyLeaseError> {
        if self.state != LeaseState::Active {
            tracing::debug!(
                device = %self.holder.to_canonical_string(),
                state = ?self.state,
                reason = "finish requested outside the Active phase",
                "security-key session finish refused",
            );
            return Err(SecurityKeyLeaseError::InvalidTransition);
        }
        let authority = self
            .authority_lease
            .as_ref()
            .cloned()
            .ok_or(SecurityKeyLeaseError::InvalidTransition)?;
        port.release_physical_backing(authority)
            .map_err(|error| {
                tracing::warn!(
                    device = %self.holder.to_canonical_string(),
                    error = %error,
                    "security-key physical backing release failed during session finish",
                );
                SecurityKeyLeaseError::Effect(error)
            })?;
        self.authority_lease = None;
        self.relay_ticket = None;
        self.session = None;
        // Core admission evidence is single-use. A later session must carry
        // a fresh admission rather than replaying the prior physical claim.
        if self.authorized_device.is_some() {
            self.backing = None;
            self.authorized_device = None;
            self.authorized_holder = None;
        }
        self.state = terminal;
        Ok(())
    }
}

impl fmt::Debug for SecurityKeyLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecurityKeyLease")
            .field("holder", &"<redacted>")
            .field("backing", &self.backing)
            .field("state", &self.state)
            .field("session", &self.session)
            .field("has_claim", &self.claim.is_some())
            .finish()
    }
}
