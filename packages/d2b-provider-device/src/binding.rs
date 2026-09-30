//! Typed `DeviceBinding` admission at the `Device` source.
//!
//! A device capability used to be a `(provider, template)` pair: a closed
//! table in shared code named the device nodes and the Volume class a
//! worker template received, and the launch therefore inherited whatever
//! that row said. That table is a second authority beside the graph, so a
//! template *name* decided a device grant.
//!
//! This module is the Device source's own decision. The source resolves its
//! declared inventory selector into a bounded set of named functions, each
//! backed by an opaque physical authority key the trusted adapter minted, and
//! admits one `DeviceBindingRequest` at a time against those exact facts:
//!
//! - a request may name only a function the trusted inventory resolved, and
//!   only while that function is present, so a capability cannot be spelled
//!   into a declaration the host does not back (R21);
//! - the physical authority comes from the inventory, never from a
//!   device-node path, a serial, or a template name;
//! - exclusive and shared claims are arbitrated once, here, against every
//!   other live relationship on the same authority key, and a relationship
//!   that is revoking or draining keeps holding its claim until release
//!   evidence arrives (AE8);
//! - a helper that realizes a parent's binding takes a bound *leg* of the
//!   parent's reservation instead of a second claim, so an exclusive parent
//!   claim supports its own helper without competing for an allocation
//!   (AE27);
//! - an absence observation revokes or degrades the affected use and leaves
//!   every other owner's claim exactly where it was (R36).
//!
//! The admitted relationship is then materialized as one source-owned
//! `DeviceBinding` row whose desired bytes are the request itself, so a
//! reader and the graph cannot disagree about what was admitted.

use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingAdmission, BindingArbitration, BindingAuthorization, BindingContractError,
    BindingKey, BindingLifecycleState, BindingRealizationFacet, BindingRealizationSupport,
    BindingRefusal, BindingSpecFingerprint, DeviceArbitration, DeviceAuthorityArbitration,
    DeviceAuthorityDescriptor, DeviceAuthorityKey, DeviceBindingRequest, DeviceClaimRequest,
    DeviceEffectOperation, DeviceFunction, DeviceSpec, FreshnessTuple, RefusalReason, ResourceRef,
    ResourceUid, SourceAdmission, SourceReservation, StoreIncarnation, ZoneId,
    admit_binding_request, canonical_json_bytes, framed_canonical_digest,
};

/// The upper bound on named functions one Device source resolves.
///
/// A Device is one physical or emulated device, so its declared capability
/// set is small and closed. The bound keeps one row from becoming a general
/// device-directory grant.
pub const MAX_DEVICE_FUNCTIONS: usize = 8;

/// The domain tag framing one Device binding row name.
const BINDING_ROW_DOMAIN: &str = "d2b:v3:device-binding-row";

/// The domain tag framing one Device source reservation identity.
const RESERVATION_DOMAIN: &str = "d2b:v3:device-source-reservation";

/// Whether the trusted inventory still backs one named function.
///
/// Presence is an observation, never an authority: an absent function is
/// refused, and a present one is still arbitrated against every live claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePresence {
    /// The resolved physical device is present in host inventory.
    Present,
    /// The resolved physical device is gone.
    Absent,
}

/// One named capability the Device source's trusted inventory resolved.
///
/// The entry pairs the name a request may spell with the opaque physical
/// authority that backs it. The host node path, serial, bus id, and PCI slot
/// stay in the trusted inventory that resolved it; none of them is reachable
/// from a declared request.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceInventoryEntry {
    function: DeviceFunction,
    authority: DeviceAuthorityDescriptor,
    presence: DevicePresence,
}

impl DeviceInventoryEntry {
    /// Construct one resolved capability.
    pub fn new(
        function: DeviceFunction,
        authority_key: DeviceAuthorityKey,
        arbitration: DeviceAuthorityArbitration,
        presence: DevicePresence,
    ) -> Self {
        Self {
            function,
            authority: DeviceAuthorityDescriptor::new(authority_key, arbitration),
            presence,
        }
    }

    /// Borrow the named function.
    pub const fn function(&self) -> &DeviceFunction {
        &self.function
    }

    /// Borrow the opaque physical authority descriptor.
    pub const fn authority(&self) -> &DeviceAuthorityDescriptor {
        &self.authority
    }

    /// Borrow the opaque physical authority key.
    pub const fn authority_key(&self) -> &DeviceAuthorityKey {
        self.authority.authority_key()
    }

    /// How the trusted inventory arbitrates this capability on its own.
    pub const fn arbitration(&self) -> DeviceAuthorityArbitration {
        self.authority.arbitration()
    }

    /// Whether the trusted inventory still backs this capability.
    pub const fn presence(&self) -> DevicePresence {
        self.presence
    }
}

impl core::fmt::Debug for DeviceInventoryEntry {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DeviceInventoryEntry")
            .field("function", &self.function)
            .field("arbitration", &self.arbitration())
            .field("presence", &self.presence)
            .finish_non_exhaustive()
    }
}

/// The trusted inventory observation for one `Device` source.
///
/// The set is bounded and its names are unique, so a request either names a
/// capability this source resolved or it names nothing at all; there is no
/// prefix, wildcard, or directory form.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct DeviceInventory {
    entries: Vec<DeviceInventoryEntry>,
}

impl DeviceInventory {
    /// Construct one inventory observation.
    ///
    /// # Errors
    ///
    /// Returns [`BindingContractError::InvalidCollection`] when the set is
    /// empty, exceeds [`MAX_DEVICE_FUNCTIONS`], or names one function twice.
    pub fn new(entries: Vec<DeviceInventoryEntry>) -> Result<Self, BindingContractError> {
        if entries.is_empty() || entries.len() > MAX_DEVICE_FUNCTIONS {
            return Err(BindingContractError::InvalidCollection);
        }
        let mut names: Vec<&DeviceFunction> =
            entries.iter().map(DeviceInventoryEntry::function).collect();
        names.sort_unstable();
        names.dedup();
        if names.len() != entries.len() {
            return Err(BindingContractError::InvalidCollection);
        }
        Ok(Self { entries })
    }

    /// Borrow every resolved capability, in declaration order.
    pub fn entries(&self) -> &[DeviceInventoryEntry] {
        &self.entries
    }

    /// Look one named capability up.
    pub fn entry(&self, function: &DeviceFunction) -> Option<&DeviceInventoryEntry> {
        self.entries
            .iter()
            .find(|entry| entry.function() == function)
    }

    /// Whether this source resolved `function` at all.
    pub fn resolves(&self, function: &DeviceFunction) -> bool {
        self.entry(function).is_some()
    }

    /// The named capabilities the trusted inventory currently backs.
    ///
    /// This is the admitted surface of one Device row: a consumer may reach
    /// one of these and nothing else. A capability the inventory resolved
    /// once and no longer backs is absent from the result, so a worker's
    /// declared shape is refused rather than launched against a device that
    /// has gone.
    pub fn present_functions(&self) -> Vec<DeviceFunction> {
        self.entries
            .iter()
            .filter(|entry| entry.presence() == DevicePresence::Present)
            .map(|entry| entry.function().clone())
            .collect()
    }

    /// Return the same inventory with one function's presence replaced.
    ///
    /// A fresh observation never edits a live one, so a claim admitted
    /// against the previous inventory keeps the evidence it was admitted
    /// with until it is re-admitted.
    ///
    /// # Errors
    ///
    /// Returns [`BindingContractError::InvalidField`] when this inventory
    /// never resolved `function`.
    pub fn with_presence(
        &self,
        function: &DeviceFunction,
        presence: DevicePresence,
    ) -> Result<Self, BindingContractError> {
        let mut entries = self.entries.clone();
        let entry = entries
            .iter_mut()
            .find(|entry| entry.function() == function)
            .ok_or(BindingContractError::InvalidField)?;
        entry.presence = presence;
        Ok(Self { entries })
    }
}

impl core::fmt::Debug for DeviceInventory {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DeviceInventory")
            .field("entries", &self.entries)
            .finish()
    }
}

/// The evidence every admission for one Device source is evaluated against.
///
/// These are the inputs that are not the source's own decision: the selected
/// realization's declared support, the authorization evidence, the dependency
/// revisions the admission is fenced against, and the effect operation
/// classes this source admits. Omitting any of them is not an unchecked
/// admission; the shared evaluator refuses.
#[derive(Clone, Copy)]
pub struct DeviceAdmissionGrant<'a> {
    support: &'a BindingRealizationSupport,
    authorization: &'a BindingAuthorization,
    dependencies: &'a [FreshnessTuple],
    operations: &'a [DeviceEffectOperation],
}

impl<'a> DeviceAdmissionGrant<'a> {
    /// Carry the selected realization's support, the grant, the fence, and
    /// the effect operation classes this source admits.
    pub const fn new(
        support: &'a BindingRealizationSupport,
        authorization: &'a BindingAuthorization,
        dependencies: &'a [FreshnessTuple],
        operations: &'a [DeviceEffectOperation],
    ) -> Self {
        Self {
            support,
            authorization,
            dependencies,
            operations,
        }
    }
}

impl core::fmt::Debug for DeviceAdmissionGrant<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DeviceAdmissionGrant")
            .field("operations", &self.operations.len())
            .finish_non_exhaustive()
    }
}

/// The source's own facts one admission is evaluated against.
#[derive(Clone, Copy)]
pub struct DeviceAdmissionSource<'a> {
    zone: &'a ZoneId,
    device_ref: &'a ResourceRef,
    device_uid: &'a ResourceUid,
    spec: &'a DeviceSpec,
    inventory: &'a DeviceInventory,
    grant: &'a DeviceAdmissionGrant<'a>,
}

impl<'a> DeviceAdmissionSource<'a> {
    /// Bind one Device's identity, declared spec, and resolved inventory to
    /// the grant.
    pub const fn new(
        zone: &'a ZoneId,
        device_ref: &'a ResourceRef,
        device_uid: &'a ResourceUid,
        spec: &'a DeviceSpec,
        inventory: &'a DeviceInventory,
        grant: &'a DeviceAdmissionGrant<'a>,
    ) -> Self {
        Self {
            zone,
            device_ref,
            device_uid,
            spec,
            inventory,
            grant,
        }
    }

    /// Borrow the Zone the relationships belong to.
    pub const fn zone(&self) -> &'a ZoneId {
        self.zone
    }

    /// Borrow the exact source reference.
    pub const fn device_ref(&self) -> &'a ResourceRef {
        self.device_ref
    }

    /// Borrow the source's store-assigned identity.
    pub const fn device_uid(&self) -> &'a ResourceUid {
        self.device_uid
    }

    /// Borrow the source's declared spec.
    pub const fn spec(&self) -> &'a DeviceSpec {
        self.spec
    }

    /// Borrow the trusted inventory this source admits against.
    pub const fn inventory(&self) -> &'a DeviceInventory {
        self.inventory
    }

    /// The effect operation classes this source admits.
    pub const fn permitted_operations(&self) -> &'a [DeviceEffectOperation] {
        self.grant.operations
    }
}

impl core::fmt::Debug for DeviceAdmissionSource<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DeviceAdmissionSource")
            .field("device_ref", &self.device_ref)
            .field("arbitration", &self.spec.arbitration())
            .finish_non_exhaustive()
    }
}

/// One admitted device capability use, ready to become one owned row.
///
/// It carries the exact relationship key, the request that was admitted
/// unchanged, the admission the shared evaluator minted, the opaque physical
/// authority the inventory resolved, and the source-owned reservation
/// identity behind them. No device node path, serial, or numeric principal is
/// reachable from it.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedDeviceBinding {
    key: BindingKey,
    request: DeviceBindingRequest,
    admission: BindingAdmission,
    authority_key: DeviceAuthorityKey,
    reservation: SourceReservation,
    operations: Vec<DeviceEffectOperation>,
}

impl AdmittedDeviceBinding {
    /// Borrow the KTD3 identity of this relationship.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Borrow the canonical request, exactly as the consumer authored it.
    pub const fn request(&self) -> &DeviceBindingRequest {
        &self.request
    }

    /// Borrow the admission this relationship was granted.
    pub const fn admission(&self) -> &BindingAdmission {
        &self.admission
    }

    /// Borrow the named capability this claim covers.
    pub const fn function(&self) -> &DeviceFunction {
        self.request.function()
    }

    /// Borrow the opaque physical authority this claim holds.
    pub const fn authority_key(&self) -> &DeviceAuthorityKey {
        &self.authority_key
    }

    /// Borrow the source-owned reservation identity.
    pub const fn reservation(&self) -> &SourceReservation {
        &self.reservation
    }

    /// The effect operation classes this relationship may drive.
    pub fn operations(&self) -> &[DeviceEffectOperation] {
        &self.operations
    }

    /// Whether this relationship holds the capability alone.
    pub fn holds_exclusive(&self) -> bool {
        self.request.claim() == DeviceClaimRequest::Exclusive
    }

    /// The digest of the exact desired bytes this relationship commits.
    pub fn fingerprint(&self) -> BindingSpecFingerprint {
        self.request.fingerprint()
    }
}

impl core::fmt::Debug for AdmittedDeviceBinding {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AdmittedDeviceBinding")
            .field("request", &self.request)
            .field("rights", &self.admission.rights())
            .field("exclusive", &self.holds_exclusive())
            .finish_non_exhaustive()
    }
}

/// One relationship this source has admitted, with its observed lifecycle.
///
/// The lifecycle is the release evidence: a relationship that is revoking or
/// draining still holds its claim, so a new request is not admitted against
/// an authority that has not been released yet.
#[derive(Clone, PartialEq, Eq)]
pub struct LiveDeviceBinding {
    binding: AdmittedDeviceBinding,
    lifecycle: BindingLifecycleState,
}

impl LiveDeviceBinding {
    /// Pair one admitted relationship with its observed lifecycle.
    pub const fn new(binding: AdmittedDeviceBinding, lifecycle: BindingLifecycleState) -> Self {
        Self {
            binding,
            lifecycle,
        }
    }

    /// Borrow the admitted relationship.
    pub const fn binding(&self) -> &AdmittedDeviceBinding {
        &self.binding
    }

    /// Return the observed lifecycle.
    pub const fn lifecycle(&self) -> BindingLifecycleState {
        self.lifecycle
    }

    /// Return the same relationship with a newly observed lifecycle.
    pub fn with_lifecycle(&self, lifecycle: BindingLifecycleState) -> Self {
        Self {
            binding: self.binding.clone(),
            lifecycle,
        }
    }

    /// Whether this relationship still holds its share of the authority.
    ///
    /// A refused or released relationship holds nothing; every other state,
    /// including an uncertain one, keeps holding until release evidence
    /// arrives. Treating an uncertain claim as free would hand a live device
    /// to a second consumer.
    pub const fn still_holds_claim(&self) -> bool {
        !self.lifecycle.is_terminal()
    }
}

impl core::fmt::Debug for LiveDeviceBinding {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("LiveDeviceBinding")
            .field("binding", &self.binding)
            .field("lifecycle", &self.lifecycle)
            .finish()
    }
}

/// Build one refusal of the shared contract.
const fn refuse(stage: AdmissionStage, reason: RefusalReason) -> BindingRefusal {
    BindingRefusal::new(stage, reason)
}

/// Admit one canonical consumer request against this Device source.
///
/// `held` is every relationship this source has already admitted, across all
/// consumers and all named capabilities, so the arbitration decision sees
/// its peers rather than only the request in hand. Re-admitting a
/// relationship already in `held` is idempotent: the same key is not a
/// conflict with itself.
///
/// # Errors
///
/// Returns the first refusal of one source-side admission path: a request
/// that names another source, a capability the trusted inventory did not
/// resolve, a capability the inventory no longer backs, an exclusive claim on
/// an authority this source does not arbitrate exclusively, an exclusive
/// claim another live relationship already holds, a shared claim past the
/// declared holder ceiling, a source that admits no effect operation class, a
/// presentation the selected realization cannot enforce, or a fence the
/// shared evaluator rejects.
pub fn admit_device_request(
    source: &DeviceAdmissionSource<'_>,
    consumer_uid: &ResourceUid,
    request: &DeviceBindingRequest,
    held: &[LiveDeviceBinding],
) -> Result<AdmittedDeviceBinding, BindingRefusal> {
    if request.source_ref() != source.device_ref {
        return Err(refuse(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused,
        ));
    }
    // Exact capabilities: the name a request spells has to be one this
    // source's trusted inventory resolved, and it has to still be backed.
    let Some(entry) = source.inventory().entry(request.function()) else {
        return Err(refuse(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused,
        ));
    };
    if entry.presence() != DevicePresence::Present {
        return Err(refuse(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused,
        ));
    }
    let key = request
        .key(
            source.zone().clone(),
            source.device_uid().clone(),
            consumer_uid.clone(),
        )
        .map_err(|_| {
            refuse(
                AdmissionStage::Admit,
                RefusalReason::SourcePolicyRefused,
            )
        })?;

    // Arbitration is the source's own single decision, and it is taken over
    // the physical authority the inventory resolved rather than over the
    // function name, so two capabilities that share one physical node
    // arbitrate against each other. A relationship that is revoking or
    // draining still holds its claim: release evidence gates reassignment.
    let peers = held
        .iter()
        .filter(|live| live.still_holds_claim() && live.binding().key() != &key)
        .filter(|live| live.binding().authority_key() == entry.authority_key());
    let arbitration = match request.claim() {
        DeviceClaimRequest::Exclusive => {
            if entry.arbitration() != DeviceAuthorityArbitration::Exclusive
                || source.spec().arbitration() == DeviceArbitration::Shared
            {
                return Err(refuse(
                    AdmissionStage::Admit,
                    RefusalReason::SourcePolicyRefused,
                ));
            }
            if peers.clone().next().is_some() {
                return Err(refuse(
                    AdmissionStage::Reserve,
                    RefusalReason::ConflictingDeclaration,
                ));
            }
            BindingArbitration::Exclusive
        }
        DeviceClaimRequest::Shared => {
            if source.spec().arbitration() == DeviceArbitration::Exclusive
                && peers.clone().next().is_some()
            {
                return Err(refuse(
                    AdmissionStage::Reserve,
                    RefusalReason::ConflictingDeclaration,
                ));
            }
            let ceiling = source.spec().max_concurrent_claims() as usize;
            if peers.clone().count() >= ceiling {
                return Err(refuse(
                    AdmissionStage::Reserve,
                    RefusalReason::ConflictingDeclaration,
                ));
            }
            BindingArbitration::Shared
        }
    };

    let decision =
        SourceAdmission::new(key.clone(), vec![request.requested_rights()], arbitration).map_err(
            |_| {
                refuse(
                    AdmissionStage::Admit,
                    RefusalReason::SourcePolicyRefused,
                )
            },
        )?;
    let admission = admit_binding_request(
        &key,
        request.requested_rights(),
        request.required_facets(),
        source.grant.authorization,
        &decision,
        source.grant.support,
        source.grant.dependencies,
    )?;
    let operations = permitted_operations(source)?;
    let reservation = source_reservation(source, &key)?;
    Ok(AdmittedDeviceBinding {
        key,
        request: request.clone(),
        admission,
        authority_key: entry.authority_key().clone(),
        reservation,
        operations,
    })
}

/// Admit a batch of canonical requests through the one source-side path.
///
/// Each admitted relationship joins the set the next one is judged against,
/// so arbitration is decided across the whole batch in declaration order
/// rather than per request.
///
/// # Errors
///
/// Returns the first refusal of the batch. An admitted prefix is not
/// committed: the caller owns what it commits, and a partial batch is never
/// half-applied by this function.
pub fn admit_device_requests(
    source: &DeviceAdmissionSource<'_>,
    requests: &[(ResourceUid, DeviceBindingRequest)],
) -> Result<Vec<AdmittedDeviceBinding>, BindingRefusal> {
    let mut admitted: Vec<AdmittedDeviceBinding> = Vec::with_capacity(requests.len());
    for (index, (consumer_uid, request)) in requests.iter().enumerate() {
        let live: Vec<LiveDeviceBinding> = admitted[..index]
            .iter()
            .cloned()
            .map(|binding| LiveDeviceBinding::new(binding, BindingLifecycleState::Admitted))
            .collect();
        admitted.push(admit_device_request(source, consumer_uid, request, &live)?);
    }
    Ok(admitted)
}

fn permitted_operations(
    source: &DeviceAdmissionSource<'_>,
) -> Result<Vec<DeviceEffectOperation>, BindingRefusal> {
    let mut operations = source.permitted_operations().to_vec();
    operations.sort_unstable();
    operations.dedup();
    if operations.is_empty() {
        return Err(refuse(
            AdmissionStage::Admit,
            RefusalReason::MandatoryFacetUnsupported,
        ));
    }
    Ok(operations)
}

fn source_reservation(
    source: &DeviceAdmissionSource<'_>,
    key: &BindingKey,
) -> Result<SourceReservation, BindingRefusal> {
    let bytes = canonical_json_bytes(key).map_err(|_| {
        refuse(
            AdmissionStage::Reserve,
            RefusalReason::SourcePolicyRefused,
        )
    })?;
    let digest = framed_canonical_digest(RESERVATION_DOMAIN, &bytes);
    let reservation_id = BoundedToken::parse(format!("dev-res-{}", &digest[7..31])).map_err(|_| {
        refuse(
            AdmissionStage::Reserve,
            RefusalReason::SourcePolicyRefused,
        )
    })?;
    Ok(SourceReservation::new(
        source.zone().clone(),
        source.device_uid().clone(),
        reservation_id,
    ))
}

/// One helper's bound leg of a parent's reservation.
///
/// A provider-created helper realizes the parent's relationship; it does not
/// take a second device claim. The leg names the parent's reservation, the
/// helper's own identity, the exact capability and physical authority the
/// parent holds, a permitted operation subset, and the broker epoch the
/// admission was fenced against. It cannot introduce another source, widen
/// the parent's operations, or outlive the parent's revocation.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceHelperLeg {
    parent_key: BindingKey,
    reservation: SourceReservation,
    helper_ref: ResourceRef,
    helper_uid: ResourceUid,
    function: DeviceFunction,
    authority_key: DeviceAuthorityKey,
    operations: Vec<DeviceEffectOperation>,
    epoch: StoreIncarnation,
}

impl DeviceHelperLeg {
    /// Bind one helper to a parent's admitted reservation.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the helper names no permitted operation, asks
    /// for an operation class the parent's admission does not carry, or is
    /// fenced against a different broker epoch than the parent.
    pub fn bind(
        parent: &AdmittedDeviceBinding,
        helper_ref: ResourceRef,
        helper_uid: ResourceUid,
        operations: &[DeviceEffectOperation],
        epoch: &StoreIncarnation,
    ) -> Result<Self, BindingRefusal> {
        if operations.is_empty() {
            return Err(refuse(
                AdmissionStage::Authorize,
                RefusalReason::MandatoryFacetUnsupported,
            ));
        }
        if operations
            .iter()
            .any(|operation| !parent.operations().contains(operation))
        {
            return Err(refuse(
                AdmissionStage::Authorize,
                RefusalReason::RequiredCapabilityOutsideCeiling,
            ));
        }
        let parent_dependencies = parent.admission().dependencies();
        if parent_dependencies.is_empty() || parent_dependencies[0].store_incarnation() != epoch {
            return Err(refuse(
                AdmissionStage::Reserve,
                RefusalReason::StaleAuthority,
            ));
        }
        let mut operations = operations.to_vec();
        operations.sort_unstable();
        operations.dedup();
        Ok(Self {
            parent_key: parent.key().clone(),
            reservation: parent.reservation().clone(),
            helper_ref,
            helper_uid,
            function: parent.function().clone(),
            authority_key: parent.authority_key().clone(),
            operations,
            epoch: epoch.clone(),
        })
    }

    /// Borrow the parent relationship this leg realizes.
    pub const fn parent_key(&self) -> &BindingKey {
        &self.parent_key
    }

    /// Borrow the parent reservation identity this leg uses.
    pub const fn reservation(&self) -> &SourceReservation {
        &self.reservation
    }

    /// Borrow the helper's own resource reference.
    pub const fn helper_ref(&self) -> &ResourceRef {
        &self.helper_ref
    }

    /// Borrow the helper's store-assigned identity.
    pub const fn helper_uid(&self) -> &ResourceUid {
        &self.helper_uid
    }

    /// Borrow the capability this leg reaches.
    pub const fn function(&self) -> &DeviceFunction {
        &self.function
    }

    /// Borrow the exact physical authority this leg reaches.
    pub const fn authority_key(&self) -> &DeviceAuthorityKey {
        &self.authority_key
    }

    /// The operation subset this leg may drive.
    pub fn operations(&self) -> &[DeviceEffectOperation] {
        &self.operations
    }

    /// Borrow the broker epoch this leg is fenced against.
    pub const fn epoch(&self) -> &StoreIncarnation {
        &self.epoch
    }

    /// Whether this leg holds a claim of its own.
    ///
    /// It never does: a leg is an explicitly attenuated realization of the
    /// parent's reservation, not a second allocation against the same
    /// physical authority.
    pub const fn holds_claim(&self) -> bool {
        false
    }

    /// Whether this leg drives one operation class.
    pub fn covers(&self, operation: DeviceEffectOperation) -> bool {
        self.operations.contains(&operation)
    }
}

/// The source's real leg satisfies the USBIP family's read-only view.
///
/// The family declares its own `BoundDeviceLeg` because a family cannot
/// depend on this crate - that is a hard package cycle, which cargo rejects
/// even as a dev-dependency. Writing the implementation here instead lets the
/// family hold the graph's own leg rather than a copy of it, so the graph's
/// authority is what the family verifies and no second leg type exists to
/// drift from it.
impl d2b_provider_device_usbip::BoundDeviceLeg for DeviceHelperLeg {
    fn parent_key(&self) -> &BindingKey {
        &self.parent_key
    }

    fn reservation(&self) -> &SourceReservation {
        &self.reservation
    }

    fn helper_ref(&self) -> &ResourceRef {
        &self.helper_ref
    }

    fn helper_uid(&self) -> &ResourceUid {
        &self.helper_uid
    }

    fn function(&self) -> &DeviceFunction {
        &self.function
    }

    fn authority_key(&self) -> &DeviceAuthorityKey {
        &self.authority_key
    }

    fn operations(&self) -> &[DeviceEffectOperation] {
        &self.operations
    }

    fn epoch(&self) -> &StoreIncarnation {
        &self.epoch
    }

    fn holds_claim(&self) -> bool {
        false
    }
}

/// The source's real leg satisfies the security-key family's read-only view.
///
/// See the USBIP implementation above: each family declares its own view to
/// avoid the package cycle, and this is the single place the source's leg
/// satisfies both of them.
impl d2b_provider_device_security_key::BoundDeviceLeg for DeviceHelperLeg {
    fn parent_key(&self) -> &BindingKey {
        &self.parent_key
    }

    fn reservation(&self) -> &SourceReservation {
        &self.reservation
    }

    fn helper_ref(&self) -> &ResourceRef {
        &self.helper_ref
    }

    fn helper_uid(&self) -> &ResourceUid {
        &self.helper_uid
    }

    fn function(&self) -> &DeviceFunction {
        &self.function
    }

    fn authority_key(&self) -> &DeviceAuthorityKey {
        &self.authority_key
    }

    fn operations(&self) -> &[DeviceEffectOperation] {
        &self.operations
    }

    fn epoch(&self) -> &StoreIncarnation {
        &self.epoch
    }

    fn holds_claim(&self) -> bool {
        false
    }
}

impl core::fmt::Debug for DeviceHelperLeg {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DeviceHelperLeg")
            .field("helper_ref", &self.helper_ref)
            .field("function", &self.function)
            .field("operations", &self.operations)
            .finish_non_exhaustive()
    }
}

/// The safe outcome one observation demands of one relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceUseOutcome {
    /// The relationship's use continues exactly as it was.
    Retained,
    /// Current use must stop: the capability is gone, so the claim cannot be
    /// proven effective any more.
    Revoked,
    /// The relationship exists but its effectiveness cannot be proven, so it
    /// is treated as unusable without being declared released.
    Degraded,
}

/// What one observation makes of one relationship.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceBindingFate {
    key: BindingKey,
    function: DeviceFunction,
    outcome: DeviceUseOutcome,
}

impl DeviceBindingFate {
    /// Borrow the relationship this decision is about.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Borrow the capability the decision is about.
    pub const fn function(&self) -> &DeviceFunction {
        &self.function
    }

    /// Return the safe outcome this observation demands.
    pub const fn outcome(&self) -> DeviceUseOutcome {
        self.outcome
    }
}

impl core::fmt::Debug for DeviceBindingFate {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DeviceBindingFate")
            .field("function", &self.function)
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

/// Decide what a source's live relationships must do after one observation.
///
/// The decision is scoped to the capabilities this source resolved: a
/// capability the inventory no longer backs, or that the observation reports
/// as gone, is revoked; a capability that is still backed keeps its use
/// exactly as it was. A relationship whose own effect cannot be proven stays
/// degraded rather than being read as granted use, and nothing here frees the
/// physical authority for another owner: reassignment still waits for the
/// revoked relationship's release evidence.
pub fn decide_presence(
    held: &[LiveDeviceBinding],
    inventory: &DeviceInventory,
) -> Vec<DeviceBindingFate> {
    held.iter()
        .map(|live| {
            let function = live.binding().function().clone();
            let backed = inventory
                .entry(&function)
                .is_some_and(|entry| entry.presence() == DevicePresence::Present);
            let outcome = if !backed {
                DeviceUseOutcome::Revoked
            } else if !live.lifecycle().proves_effect() {
                DeviceUseOutcome::Degraded
            } else {
                DeviceUseOutcome::Retained
            };
            DeviceBindingFate {
                key: live.binding().key().clone(),
                function,
                outcome,
            }
        })
        .collect()
}

/// The safe outcome one helper leg gets from its parent's decision.
///
/// A leg cannot outlive the parent's revocation: it is the parent's
/// reservation, so a revoked or degraded parent withdraws it too, while a
/// parent whose use continues leaves the leg exactly as it was. A leg whose
/// parent this decision says nothing about stays degraded rather than being
/// assumed usable.
pub fn leg_outcome(leg: &DeviceHelperLeg, fates: &[DeviceBindingFate]) -> DeviceUseOutcome {
    fates
        .iter()
        .find(|fate| fate.key() == leg.parent_key())
        .map_or(DeviceUseOutcome::Degraded, DeviceBindingFate::outcome)
}

/// The realization support one Device family declares.
///
/// Every device binding is delivered as a verified device descriptor or a
/// mediated attachment, so the closed support set is the single attachment
/// facet. The attachment *mode* is the source's admission decision, not a
/// separate realization capability, so a family cannot widen it by naming a
/// different facet.
pub fn device_attachment_support() -> BindingRealizationSupport {
    BindingRealizationSupport::new(vec![BindingRealizationFacet::DeviceAttachment])
        .expect("the single attachment facet is a valid realization support")
}

/// The deterministic row name the source mints for one relationship.
///
/// The name derives from the relationship's committed identities - Zone,
/// source, consumer, kind, and the stable slot - never from a declaration
/// index or an attachment order, so reordering declarations never churns
/// identities and two relationships cannot collide by position.
///
/// # Errors
///
/// Returns [`BindingContractError::InvalidField`] when the derived row name
/// is not a bounded token.
pub fn binding_row_name(key: &BindingKey) -> Result<BoundedToken, BindingContractError> {
    let bytes = canonical_json_bytes(key).map_err(|_| BindingContractError::InvalidField)?;
    let digest = framed_canonical_digest(BINDING_ROW_DOMAIN, &bytes);
    BoundedToken::parse(format!("dev-binding-{}", &digest[7..31]))
        .map_err(|_| BindingContractError::InvalidField)
}

/// One source-owned `DeviceBinding` row the source mints for one admitted
/// relationship.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceBindingRow {
    name: BoundedToken,
    spec: Vec<u8>,
}

impl DeviceBindingRow {
    /// Borrow the deterministic row name.
    pub const fn name(&self) -> &BoundedToken {
        &self.name
    }

    /// Borrow the canonical desired bytes committed as the row's spec.
    pub fn spec(&self) -> &[u8] {
        &self.spec
    }
}

impl core::fmt::Debug for DeviceBindingRow {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DeviceBindingRow")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// The committed `DeviceBinding` row one admitted relationship mints.
///
/// The row's desired bytes are the canonical request, so the durable record
/// of the relationship is the declaration the consumer authored rather than
/// a second description of it. No device node path, host permission bit, or
/// numerical principal is added.
///
/// # Errors
///
/// Returns [`BindingContractError::InvalidField`] when the request does not
/// render canonical bytes or the derived row name is not a bounded token.
pub fn canonical_binding_row(
    admitted: &AdmittedDeviceBinding,
) -> Result<DeviceBindingRow, BindingContractError> {
    let spec =
        canonical_json_bytes(admitted.request()).map_err(|_| BindingContractError::InvalidField)?;
    Ok(DeviceBindingRow {
        name: binding_row_name(admitted.key())?,
        spec,
    })
}

/// Read one committed `DeviceBinding` row back as the request it declares.
///
/// # Errors
///
/// Returns [`BindingContractError::InvalidField`] when the committed bytes do
/// not decode as a canonical device binding request.
pub fn parsed_consumer_request(spec: &[u8]) -> Result<DeviceBindingRequest, BindingContractError> {
    serde_json::from_slice(spec).map_err(|_| BindingContractError::InvalidField)
}
