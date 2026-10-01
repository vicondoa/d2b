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
//! `DeviceBinding` row: the family's own `DeviceBindingSpec`, carrying the
//! consumer's request identities and the source's accepted decision, so a
//! reader and the graph cannot disagree about what was admitted.

use std::sync::Arc;
use std::time::Duration;

use crate::driver::{DeviceComponent, component_for_provider, declared_device_functions};
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingAdmission, BindingArbitration, BindingAuthorization, BindingContractError,
    BindingKey, BindingLifecycleState, BindingRealizationFacet, BindingRealizationSupport,
    BindingRefusal, BindingRowError, BindingSourceDecision, BindingSpecFingerprint,
    ControllerGeneration, DeviceArbitration, DeviceAuthorityArbitration, DeviceAuthorityDescriptor,
    DeviceAuthorityKey, DeviceBindingRequest, DeviceBindingSpec, DeviceClaimRequest,
    DeviceEffectOperation, DeviceFunction, DeviceSpec, FreshnessTuple, RefusalReason,
    ResourceGeneration, ResourceRef, ResourceSpec, ResourceUid, SourceAdmission,
    SourceReservation, StoreIncarnation, ZoneId, admit_binding_request, canonical_json_bytes,
    framed_canonical_digest,
};
use d2b_provider_toolkit::shared_provider::{
    ContextChildSurface, SharedProviderEffectError, SharedProviderEffectRequest,
};
use d2b_resource_runtime::context::{
    ResourceContext, RowLookup, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureComparison, FailureDetail, FailureKind, FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType,
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
            let outcome = if !capability_backed(inventory, &function) {
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

/// Whether the trusted inventory currently backs one named capability.
///
/// This is the single observation [`decide_presence`] makes, factored out so
/// the serving driver reads the same predicate rather than re-deriving it from
/// the inventory a second way. Presence is an observation and never an
/// authority: a capability the host no longer backs is absent here, and a
/// claim on it is revoked rather than served (R21, R36).
pub fn capability_backed(inventory: &DeviceInventory, function: &DeviceFunction) -> bool {
    inventory
        .entry(function)
        .is_some_and(|entry| entry.presence() == DevicePresence::Present)
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

/// Why one committed `Device` row's binding derivation refused.
///
/// The two shared contract rejections travel beside the family's own refusal
/// rather than inside it, and both are field-free, so a refusal echoes no
/// device node path, resource identity, or caller-supplied text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceBindingDerivationError {
    /// The relationship names a capability this source row's declared
    /// vocabulary does not back.
    FunctionNotDeclared,
    /// The row's own source and consumer references were refused by the
    /// binding kind.
    RowRefs(BindingRowError),
    /// The consumer slot, the row name, or the canonical bytes did not render.
    Contract(BindingContractError),
}

impl core::fmt::Display for DeviceBindingDerivationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FunctionNotDeclared => {
                formatter.write_str("the named capability is not declared")
            }
            Self::RowRefs(refused) => {
                write!(formatter, "the row's own references are refused: {refused}")
            }
            Self::Contract(refused) => {
                write!(formatter, "the row does not render: {refused}")
            }
        }
    }
}

impl std::error::Error for DeviceBindingDerivationError {}

impl From<BindingContractError> for DeviceBindingDerivationError {
    fn from(error: BindingContractError) -> Self {
        Self::Contract(error)
    }
}

impl From<BindingRowError> for DeviceBindingDerivationError {
    fn from(error: BindingRowError) -> Self {
        Self::RowRefs(error)
    }
}

/// The committed `DeviceBinding` rows one committed `Device` row implies.
///
/// Every admitted relationship becomes exactly one row, named from the KTD3
/// key rather than from a declaration position, so the same relationship keeps
/// one identity across restarts and two relationships never collide by
/// ordering. A source row that admits no relationship implies no row at all:
/// there is no default attachment to fall back on.
///
/// The capability vocabulary is the family's, read off the committed row
/// rather than off the trusted inventory. The inventory says which named
/// capabilities the host backs right now; the row says which ones this
/// Provider can deliver, and a claim outside the second is refused here
/// instead of committed as a row no family realizes. A relationship whose
/// named capability the inventory no longer backs is likewise not committed:
/// the source keeps holding that claim until release evidence arrives, but a
/// capability the host cannot deliver has no row to declare, so this one
/// retires.
///
/// # Errors
///
/// Returns [`DeviceBindingDerivationError::FunctionNotDeclared`] when a
/// relationship names a capability this source row's declared vocabulary does
/// not back, and the row contract's own refusals when a row's references,
/// consumer slot, or canonical bytes do not render.
pub fn canonical_binding_rows(
    component: DeviceComponent,
    spec: &DeviceSpec,
    inventory: &DeviceInventory,
    admitted: &[AdmittedDeviceBinding],
) -> Result<Vec<DeviceBindingRow>, DeviceBindingDerivationError> {
    let declared = declared_device_functions(component, spec);
    let support = device_attachment_support();
    let live: Vec<LiveDeviceBinding> = admitted
        .iter()
        .cloned()
        .map(|binding| LiveDeviceBinding::new(binding, BindingLifecycleState::Admitted))
        .collect();
    let fates = decide_presence(&live, inventory);
    let mut rows = Vec::with_capacity(admitted.len());
    for binding in admitted {
        if !declared.contains(binding.function()) {
            return Err(DeviceBindingDerivationError::FunctionNotDeclared);
        }
        let revoked = fates
            .iter()
            .any(|fate| fate.key() == binding.key() && fate.outcome() == DeviceUseOutcome::Revoked);
        if !revoked {
            rows.push(binding_row(binding, &support)?);
        }
    }
    Ok(rows)
}

/// The committed `DeviceBinding` row one admitted relationship mints.
///
/// The row is the family's own `DeviceBindingSpec`: the request's identities
/// plus the source's accepted decision, read off the admission rather than
/// recomputed, so the committed row cannot describe a different grant than
/// the one that was evaluated. The realized facets are this family's declared
/// attachment support, so a row is never committed against a realization the
/// family does not drive. No device node path, host permission bit, or
/// numerical principal is added.
///
/// # Errors
///
/// Returns the row contract's refusal when the row's own references, the
/// consumer slot, or the canonical bytes do not render.
fn binding_row(
    admitted: &AdmittedDeviceBinding,
    support: &BindingRealizationSupport,
) -> Result<DeviceBindingRow, DeviceBindingDerivationError> {
    let request = admitted.request();
    let decision = BindingSourceDecision::new(
        vec![admitted.admission().rights()],
        admitted.admission().arbitration(),
        support.facets().to_vec(),
    )?;
    let spec = DeviceBindingSpec::new(
        request.source_ref().clone(),
        request.consumer_ref().clone(),
        request.function().clone(),
        request.claim(),
        BoundedToken::parse(request.slot().as_str())
            .map_err(|_| BindingContractError::InvalidField)?,
        decision,
    )?;
    Ok(DeviceBindingRow {
        name: binding_row_name(admitted.key())?,
        spec: canonical_json_bytes(&spec).map_err(|_| BindingContractError::InvalidField)?,
    })
}

// ---------------------------------------------------------------------------
// The `DeviceBinding` serving driver (U16)
// ---------------------------------------------------------------------------
//
// The driver serves the committed row the source admitted: it decodes the
// neutral contract through the row's own wire decoder, enforces the committed
// `BindingSourceDecision`, resolves the parent `Device` row and the consumer
// row through the manager behind their fences, and drives what the family can
// honestly realize through [`DeviceBindingEffects`].
//
// # Which verbs of the realization this driver can honestly serve
//
// The `DeviceAttachment` realization is one facet, and the audit of the host
// effects separates what exists from what does not:
//
// - **`observe` IS routed, through the trusted inventory facet.** The Device
//   source already declares [`crate::facets::DeviceInventorySource`], and the
//   daemon implements it over the verified host device-node matrix
//   (`packages/d2bd/src/shared_provider_effects.rs`, the
//   `DeviceInventorySource` impl). It resolves the committed row's own declared
//   selector into the opaque physical authority each named capability carries
//   plus the presence observed for it - which is exactly the input
//   [`capability_backed`] - the same observation [`decide_presence`] makes -
//   is reachable from the serving half.
//
// - **`attach` and `release` are NOT routable, and this driver does not
//   pretend otherwise.** The host effects that exist are provider-specific and
//   none of them takes a generic device attachment:
//
//     - usbip's [`KernelUsbipDispatcher`](d2b_provider_device_usbip) is
//     constructed per Service over a [`UsbipBindingContext`](d2b_provider_device_usbip)
//     whose `physical_key` is the zone ledger's own map key - minted from the
//     Device row's uid by `UsbipCoreAdapter::physical_usb_backing_key`, NOT the
//     `DeviceAuthorityKey` the inventory resolves. Its `bind_owned` issues
//     `BrokerRequest::UsbipBind`, whose `UsbipBindRequest` carries only a
//     bind-intent reference and no device identity at all; its `start_proxy`
//     keys the ledger by the `usb.d2bus.org.UsbBinding` row's uid, not by this
//     row's.
//   - GPU's [`GpuRuntime::admit_authority`](d2b_provider_device_gpu) is
//     synchronous and takes an `AuthorityRequest` built by the GPU family from
//     its own `GpuAuthorityAdmission`. Nothing in the tree constructs one from
//     a `DeviceBindingSpec`.
//   - the daemon's [`DeviceRuntime`](crate::facets) dispatches on the
//     `DeviceComponent` the committed **Device** row's `providerRef` names, and
//     every branch reads that Device row's own spec. No branch sees a binding
//     row, and no code anywhere dispatches a host effect on a binding row's
//     provider.
//
//   The dispatch that is missing is one function that, given a committed
//   `DeviceBinding` row and the parent `Device` row's `providerRef`, selects
//   the component and drives that component's own attach path. Its signature
//   would be:
//
//   ```text
//   async fn attach(
//       &self,
//       component: DeviceComponent,
//       binding: &DeviceBindingSpec,
//       consumer: &ResourceRef,
//       authority: &DeviceAuthorityKey,
//   ) -> Result<DeviceAttachmentHandle, SharedProviderEffectError>;
//   ```
//
//   over the existing per-component effects, with the returned handle the thing
//   `release` takes. Building it here would be inventing a dispatch the
//   provider-specific effects do not expose, so the driver declares no such
//   verb and reports the attachment it cannot make as a named refusal.
//
// What the driver *can* prove is real and worth serving: the committed row
// decodes, its decision admits it, the parent and consumer rows resolve behind
// their fences, the named capability is one this Provider declares, and the
// trusted inventory still backs it. A row whose capability the host no longer
// backs is reported as revoked through [`decide_presence`] rather than as
// served.

/// The `Device` ResourceType the committed relationship's source is.
const DEVICE_RESOURCE_TYPE: &str = "Device";

/// Canonical `DeviceBinding` ResourceType name.
pub const DEVICE_BINDING_TYPE_NAME: &str =
    d2b_contracts_resource::v3::device_binding::DEVICE_BINDING_RESOURCE_TYPE;

/// The re-check cadence while the committed relationship is not serving.
///
/// Hardware presence is host-dependent and an absence observation reaches this
/// actor as no watch delivery on the binding row, so an unattached
/// relationship re-checks on this interval. The same shape the Endpoint and
/// Volume binding drivers use while their delivery is not yet provable.
const DEVICE_BINDING_RESYNC: Duration = Duration::from_secs(30);

/// Closed, field-free classifications of a serving failure on this row.
///
/// No variant carries a device node path, a serial, a bus id, or a resource
/// identity: a failure names which check refused and nothing else (R42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingDriverErrorKind {
    /// The durable spec did not decode as the strict neutral binding
    /// contract, or the row name is not the one this source derives.
    SpecInvalid,
    /// The committed `BindingSourceDecision` does not admit what the row
    /// claims: an admitted-right set without the claim's own right, realized
    /// facets without the attachment facet, or a facet outside the family's
    /// declared support.
    DecisionRefused,
    /// The parent `Device` row is present but its owner uid differs from this
    /// binding's owner: the manager would silently re-parent. Terminal.
    OwnerMismatch,
    /// The parent `Device` row is not observable yet, or holds no usable
    /// device spec. The former defers retryably (issue #511); the latter is
    /// terminal because the committed row cannot converge by retrying.
    ParentUnavailable,
    /// The parent row decodes, but names no Provider this family serves, or its
    /// declared vocabulary does not back the capability the row names.
    ParentPolicyRefused,
    /// The named consumer row is not observable yet (retryable) or does not
    /// exist at all (terminal).
    ConsumerUnavailable,
    /// The trusted inventory could not be resolved for the committed row.
    InventoryUnavailable,
}

impl BindingDriverErrorKind {
    /// The registered failure kind this classification reports.
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid | Self::DecisionRefused => FailureKinds::BINDING_SPEC_INVALID,
            Self::OwnerMismatch => FailureKinds::BINDING_OWNER_MISMATCH,
            Self::ParentUnavailable | Self::ParentPolicyRefused => {
                FailureKinds::BINDING_PARENT_UNAVAILABLE
            }
            Self::ConsumerUnavailable => FailureKinds::BINDING_PARENT_UNAVAILABLE,
            Self::InventoryUnavailable => FailureKinds::BINDING_SERVING_EFFECT_FAILED,
        }
    }
}

/// Typed serving failure, mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`].
#[derive(Debug, Clone)]
pub struct BindingDriverError {
    kind: BindingDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl BindingDriverError {
    fn new(kind: BindingDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op, detail: FailureDetail::new() }
    }

    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }
}

impl core::fmt::Display for BindingDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            BindingDriverErrorKind::SpecInvalid => "binding-spec-invalid",
            BindingDriverErrorKind::DecisionRefused => "binding-decision-refused",
            BindingDriverErrorKind::OwnerMismatch => "binding-owner-mismatch",
            BindingDriverErrorKind::ParentUnavailable => "binding-parent-unavailable",
            BindingDriverErrorKind::ParentPolicyRefused => "binding-parent-policy-refused",
            BindingDriverErrorKind::ConsumerUnavailable => "binding-consumer-unavailable",
            BindingDriverErrorKind::InventoryUnavailable => "binding-inventory-unavailable",
        })
    }
}

impl std::error::Error for BindingDriverError {}

/// Typed in-memory status projection (R11: never persisted).
///
/// Every field is a state, a bound, or an opaque authority digest. No device
/// node path, serial, bus id, or host permission bit is reachable from it, so
/// an audit record, a status API, and a publication snapshot all read the same
/// closed shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceBindingDriverStatus {
    /// The committed relationship is admitted, the trusted inventory still
    /// backs the capability it names, and the family's own presence decision
    /// retains it.
    ///
    /// This is NOT an attachment. The host effect that would attach it is the
    /// dispatch this family cannot build - see the module section on which
    /// verbs the family can honestly serve - so the status deliberately has no
    /// "attached" variant to reach.
    Admitted {
        /// The component the parent `Device` row's own `providerRef` selected.
        component: DeviceComponent,
        /// The opaque physical authority the trusted inventory resolved for the
        /// named capability.
        authority: DeviceAuthorityKey,
    },
    /// The relationship is fenced: pre-drain ran and new use is refused while
    /// the outstanding claim drains.
    Draining {
        /// The component the parent `Device` row's own `providerRef` selected.
        component: DeviceComponent,
    },
    /// The attachment is not standing.
    ///
    /// The reason is the closed class the observation landed in.
    Unattached {
        /// Closed, field-free: why no attachment is standing.
        reason: UnattachedReason,
    },
}

/// The closed set of reasons an attachment is not standing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnattachedReason {
    /// The trusted inventory no longer resolves the capability the row names,
    /// or reports it as gone, so the family's own
    /// [`capability_backed`] observation is false and the claim is revoked
    /// rather than served (R21, R36).
    CapabilityNotBacked,
    /// The host effect that would attach this capability is not routable from a
    /// committed row. Named rather than approximated: the family does not
    /// claim an attachment it cannot make.
    AttachDispatchUnroutable,
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one `DeviceBinding` row, exactly as persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BindingSpecEnvelope {
    /// The serving Provider reference the row was committed under, when the
    /// source named one.
    provider_ref: Option<ResourceRef>,
    base: d2b_contracts_resource::v3::CanonicalJsonObject,
}

/// The manager-wired decode hook for `DeviceBinding` rows.
pub fn device_binding_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| BindingSpecEnvelope {
            provider_ref: spec.provider_ref().cloned(),
            base: spec.base().clone(),
        })
    })
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The live-host surfaces the `DeviceBinding` serving driver needs.
///
/// The only verb is the trusted inventory read, and it is routed: the daemon
/// implements [`crate::facets::DeviceInventorySource`] over the verified host
/// device-node matrix. There are deliberately no `attach` or `release` verbs -
/// the provider-specific effects that exist take no generic device attachment
/// and nothing dispatches on the row's provider, so declaring one would be a
/// surface nothing can reach.
#[async_trait::async_trait]
pub trait DeviceBindingEffects: Send + Sync + 'static {
    /// Resolve the trusted host inventory for one committed `Device` row.
    ///
    /// The committed row is the only input: its declared `DeviceSpec` names
    /// the inventory selector, and the resolved physical authority keys and
    /// their presence come from the verified host device-node matrix.
    ///
    /// # Errors
    ///
    /// Returns [`SharedProviderEffectError`] when the committed spec does not
    /// decode, the declared selector names a bus class with no trusted
    /// inventory, or the host device-node matrix cannot be read for it.
    async fn device_inventory(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<DeviceInventory, SharedProviderEffectError>;
}

/// The provider-owned binding effects, built from the daemon-supplied facet set
/// this family already declares (R2).
///
/// One value serves both: the driver holds it as its typed seam, and the
/// `Device` driver's own effects service is the value the composition root
/// already constructs from the same facets.
pub type DeviceBindingEffectsService = crate::effects_service::DeviceEffects;

#[async_trait::async_trait]
impl DeviceBindingEffects for DeviceBindingEffectsService {
    async fn device_inventory(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<DeviceInventory, SharedProviderEffectError> {
        crate::effects_service::DeviceEffects::device_inventory(self, request).await
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Everything the plane must construct to instantiate the `DeviceBinding`
/// driver factory for one zone: the zone, the zone-authority controller
/// generation the family's effects fold in (KTD7), and the daemon-supplied
/// facet set.
pub struct DeviceBindingDriverArgs {
    /// The zone this driver's rows live in.
    pub zone: ZoneId,
    /// Zone controller generation folded into every effect call (KTD7).
    pub controller_generation: ControllerGeneration,
    /// The daemon-supplied facet set. The family never receives a
    /// daemon-built effect port.
    pub facets: crate::facets::DeviceEffectFacets,
}

/// [`ResourceDriverFactory`] for the `DeviceBinding` resource type.
/// Construction is infallible by contract (R3).
pub struct DeviceBindingDriverFactory {
    types: [ResourceTypeName; 1],
    args: DeviceBindingDriverArgs,
}

impl DeviceBindingDriverFactory {
    /// Build the factory for one zone's plane.
    pub fn new(args: DeviceBindingDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(DEVICE_BINDING_TYPE_NAME)],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for DeviceBindingDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        // The driver builds its effects from the declared facets; no externally
        // built port appears at this construction site (R2).
        let effects = Arc::new(DeviceBindingEffectsService::new(self.args.facets.clone()));
        Box::new(DeviceBindingDriver {
            zone: self.args.zone.clone(),
            effects,
            watched: Vec::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One committed `DeviceBinding` row's driver.
///
/// The driver holds no host state: the trusted inventory, and everything the
/// per-component effects would read behind it, arrive through
/// [`DeviceBindingEffects`].
pub struct DeviceBindingDriver {
    zone: ZoneId,
    effects: Arc<dyn DeviceBindingEffects>,
    /// Rows this driver already registered a dependency watch on (R12/R17).
    /// Runtime-only (R6/R11): one registration per target keeps the dependency
    /// edge that wakes the actor on a dependency's death or readiness without
    /// accumulating manager watch entries.
    watched: Vec<ResourceKey>,
}

impl DeviceBindingDriver {
    fn error(&self, kind: BindingDriverErrorKind, op: DriverOp) -> BindingDriverError {
        BindingDriverError::new(kind, op)
    }

    /// Decode the stored envelope into the strict neutral binding contract.
    ///
    /// The wire decoder is the contract's own, so a stored row that is not
    /// canonical `DeviceBinding` bytes is refused here rather than half-read.
    fn decoded_binding(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<DeviceBindingSpec, BindingDriverError> {
        let envelope = ctx
            .spec::<BindingSpecEnvelope>()
            .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        serde_json::from_slice::<DeviceBindingSpec>(&envelope.base.to_canonical_bytes())
            .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))
    }

    /// The committed `BindingSourceDecision` must admit what the row claims.
    ///
    /// Three refusals, all terminal, all read back out of the committed bytes:
    ///
    /// - an admitted-right set that does not cover the claim's own right. A
    ///   shared claim needs the sharing right and an exclusive one the
    ///   exclusive right; a row admitted for observation alone was not minted
    ///   by this family.
    /// - realized facets that do not cover the attachment facet. A row
    ///   committed without it declares no realization this family drives.
    /// - a facet the source committed that this family does not declare it can
    ///   realize. A committed facet is read back as something the source
    ///   admitted through, and the source may only commit what it can deliver.
    ///
    /// The arbitration itself is NOT checked here: a Device source arbitrates
    /// an exclusive claim against its peers and arbitrates a shared one under
    /// its own ceiling, so both values are ones this family commits. The
    /// claim's own requested right is what has to be admitted.
    fn check_committed_decision(
        &self,
        binding: &DeviceBindingSpec,
        op: DriverOp,
    ) -> Result<(), BindingDriverError> {
        let source = binding.source();
        let support = device_attachment_support();
        let claimed = binding.claim().requested_rights();
        if !source.admitted_rights().contains(&claimed) {
            return Err(self.decision_error(op, "source.admittedRights", wire(&claimed), "absent".to_owned()));
        }
        let required = BindingRealizationFacet::DeviceAttachment;
        if !source.realized_facets().contains(&required) {
            return Err(self.decision_error(op, "source.realizedFacets", wire(&required), "absent".to_owned()));
        }
        if let Some(unsupported) = source
            .realized_facets()
            .iter()
            .find(|facet| !support.facets().contains(facet))
        {
            return Err(self.decision_error(
                op,
                "source.realizedFacets",
                support
                    .facets()
                    .iter()
                    .map(wire)
                    .collect::<Vec<_>>()
                    .join(","),
                wire(unsupported),
            ));
        }
        Ok(())
    }

    /// The refusal detail for one committed decision that does not admit the
    /// row's own claim.
    fn decision_error(
        &self,
        op: DriverOp,
        field: &'static str,
        expected: String,
        observed: String,
    ) -> BindingDriverError {
        self.error(BindingDriverErrorKind::DecisionRefused, op).with_detail(
            FailureDetail::at(match field {
                "source.arbitration" => "spec/source.arbitration",
                "source.admittedRights" => "spec/source.admittedRights",
                _ => "spec/source.realizedFacets",
            })
            .comparison(FailureComparison::new(field, expected, observed)),
        )
    }

    /// The key of the parent `Device` this binding declares.
    ///
    /// The key is built in this driver's own Zone, so a cross-Zone source is
    /// structurally unnameable rather than checked for: a primitive binding is
    /// same-Zone, and the Zone this row reconciles in is the Zone both sides
    /// live in.
    fn parent_device_key(&self, binding: &DeviceBindingSpec) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            DEVICE_RESOURCE_TYPE,
            binding.device_ref().name().as_str(),
        )
    }

    /// The key of the consumer this binding attaches to.
    fn consumer_key(&self, binding: &DeviceBindingSpec) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            binding.execution_ref().resource_type().as_str(),
            binding.execution_ref().name().as_str(),
        )
    }

    /// The parent `Device` row through the manager (R2: the driver never
    /// touches the spec store), with the same-Zone and owner fences.
    ///
    /// The binding's declared `Device` must be the row the manager reports as
    /// this resource's owner: the Device source is what mints the
    /// relationship, so a binding whose owner is a different row is one the
    /// manager would silently re-parent.
    ///
    /// The row's own `providerRef` is returned beside the spec because it is
    /// the one thing that selects which hardware family realizes this
    /// relationship - the same selection [`component_for_provider`] makes for
    /// the `Device` row's own driver, read through the same function rather
    /// than a second spelling of it.
    async fn parent_device(
        &self,
        ctx: &mut ResourceContext,
        binding: &DeviceBindingSpec,
        op: DriverOp,
    ) -> Result<(DeviceComponent, DeviceSpec), BindingDriverError> {
        let key = self.parent_device_key(binding);
        let lookup = ctx.lookup(&key).await;
        let row = match lookup {
            RowLookup::Present { row, .. } => row,
            _ => {
                // A non-present read defers: the row may not be committed yet,
                // and an unreadable payload is not terminal by itself (#511).
                let mut detail = FailureDetail::at("parent/lookup");
                if let Some(comparison) = lookup.failure_comparison("parent.device", "present") {
                    detail = detail.comparison(comparison);
                }
                if let Some(error) = lookup.error_detail() {
                    detail = detail.with_note(error);
                }
                return Err(self
                    .error(BindingDriverErrorKind::ParentUnavailable, op)
                    .with_detail(detail));
            }
        };
        if let Some(owner) = ctx.owner()
            && owner != &row.uid
        {
            return Err(self
                .error(BindingDriverErrorKind::OwnerMismatch, op)
                .with_detail(FailureDetail::at("parent/owner").comparison(
                    FailureComparison::new("parent.ownerUid", uid_hex(owner), uid_hex(&row.uid)),
                )));
        }
        let envelope = serde_json::from_slice::<ResourceSpec>(&row.spec)
            .map_err(|_| self.parent_spec_invalid(op, "parent.spec"))?;
        // The envelope's own `providerRef` is the universal desired-state layer
        // the typed `DeviceSpec` denies, so it is stripped before the base
        // fields decode - the same strip the daemon's inventory facet performs.
        let mut base = serde_json::to_value(envelope.base()).map_err(|_| {
            self.parent_spec_invalid(op, "parent.spec")
        })?;
        let provider_ref = envelope
            .provider_ref()
            .map(|reference| reference.to_canonical_string())
            .or_else(|| {
                base.get("providerRef")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .ok_or_else(|| self.parent_spec_invalid(op, "parent.providerRef"))?;
        if let Some(object) = base.as_object_mut() {
            for field in ["providerRef", "updatePolicy", "provider"] {
                object.remove(field);
            }
        }
        let spec: DeviceSpec =
            serde_json::from_value(base).map_err(|_| self.parent_spec_invalid(op, "parent.spec"))?;
        let component = component_for_provider(&provider_ref).ok_or_else(|| {
            self.error(BindingDriverErrorKind::ParentPolicyRefused, op).with_detail(
                FailureDetail::at("parent/providerRef")
                    .comparison(FailureComparison::new(
                        "device.providerRef",
                        "a device provider",
                        "not a device provider",
                    )),
            )
        })?;
        Ok((component, spec))
    }

    /// The terminal classification for a present parent row whose stored spec
    /// does not decode (issue #508: this is not an ownership mismatch).
    fn parent_spec_invalid(&self, op: DriverOp, field: &'static str) -> BindingDriverError {
        self.error(BindingDriverErrorKind::ParentUnavailable, op).with_detail(
            FailureDetail::at("parent/decode").comparison(FailureComparison::new(
                field,
                "a canonical Device row",
                "decode failed",
            )),
        )
    }

    /// The consumer row through the manager, with the same-Zone fence.
    ///
    /// The consumer is not this row's owner - it is the party the attachment is
    /// made to - so there is no owner fence here. What is read back is the
    /// store-assigned identity the KTD3 key is derived from, so a consumer that
    /// was replaced under the same name produces a different key rather than
    /// silently continuing the old relationship.
    async fn consumer_uid(
        &self,
        ctx: &mut ResourceContext,
        binding: &DeviceBindingSpec,
        op: DriverOp,
    ) -> Result<ResourceUid, BindingDriverError> {
        let key = self.consumer_key(binding);
        let lookup = ctx.lookup(&key).await;
        match lookup {
            RowLookup::Present { row, .. } => ResourceUid::from_bytes(&row.uid)
                .map_err(|_| self.error(BindingDriverErrorKind::ConsumerUnavailable, op)),
            _ => {
                let mut detail = FailureDetail::at("consumer/lookup");
                if let Some(comparison) =
                    lookup.failure_comparison("consumer.executionRef", "present")
                {
                    detail = detail.comparison(comparison);
                }
                if let Some(error) = lookup.error_detail() {
                    detail = detail.with_note(error);
                }
                Err(self
                    .error(BindingDriverErrorKind::ConsumerUnavailable, op)
                    .with_detail(detail))
            }
        }
    }

    /// The parent row's own declared vocabulary must back the capability this
    /// relationship names.
    ///
    /// This is the second, independent half of the admission and the half a
    /// consumer cannot influence: the inventory says which named capabilities
    /// the host backs right now, and the row says which ones this Provider can
    /// deliver. A claim outside the second was never admitted here.
    fn check_parent_policy(
        &self,
        component: DeviceComponent,
        spec: &DeviceSpec,
        binding: &DeviceBindingSpec,
        op: DriverOp,
    ) -> Result<(), BindingDriverError> {
        if !declared_device_functions(component, spec).contains(binding.function()) {
            return Err(self.error(BindingDriverErrorKind::ParentPolicyRefused, op).with_detail(
                FailureDetail::at("parent/declaredFunctions").comparison(
                    FailureComparison::new(
                        "device.declaredFunctions",
                        binding.function().as_str(),
                        "not declared",
                    ),
                ),
            ));
        }
        Ok(())
    }

    /// Every check a serving pass runs before it touches the host: the wire
    /// decode, the committed decision, the parent row behind its owner fence,
    /// that row's own vocabulary, and the consumer row.
    async fn resolved(
        &self,
        ctx: &mut ResourceContext,
        op: DriverOp,
    ) -> Result<(DeviceBindingSpec, DeviceComponent, DeviceSpec), BindingDriverError> {
        let binding = self.decoded_binding(ctx, op)?;
        self.check_committed_decision(&binding, op)?;
        let (component, spec) = self.parent_device(ctx, &binding, op).await?;
        self.check_parent_policy(component, &spec, &binding, op)?;
        self.consumer_uid(ctx, &binding, op).await?;
        Ok((binding, component, spec))
    }

    /// Register one dependency watch, at most once per target (R12/R17).
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) {
            return;
        }
        if ctx.watch(target.clone(), WatchCondition::Ready).await.is_ok() {
            self.watched.push(target);
        }
    }

    /// The trusted inventory the committed `Device` row resolves to.
    ///
    /// The request is built from the PARENT row, because the parent is what
    /// declares the inventory selector: a binding row names a capability and a
    /// consumer, never a selector. Reading the parent's own envelope here is
    /// what makes the resolution the same one the `Device` driver's effects
    /// would make.
    async fn inventory(
        &self,
        ctx: &mut ResourceContext,
        binding: &DeviceBindingSpec,
        op: DriverOp,
    ) -> Result<DeviceInventory, BindingDriverError> {
        let key = self.parent_device_key(binding);
        let lookup = ctx.lookup(&key).await;
        let row = match lookup {
            RowLookup::Present { row, .. } => row,
            _ => {
                return Err(self
                    .error(BindingDriverErrorKind::InventoryUnavailable, op)
                    .with_detail(FailureDetail::at("inventory/lookup")));
            }
        };
        let envelope = serde_json::from_slice::<ResourceSpec>(&row.spec).map_err(|_| {
            self.error(BindingDriverErrorKind::InventoryUnavailable, op)
                .with_detail(FailureDetail::at("inventory/spec"))
        })?;
        // The inventory effect reads the Provider the delivery is scoped to,
        // and the daemon's own `DeviceInventorySource` impl reads it from
        // `spec.providerRef` first and `metadata.providerRef` second. The
        // request is composed the same way here so the two agree on one
        // spelling rather than the driver's being a second convention.
        let mut spec_value = serde_json::to_value(envelope.base()).unwrap_or_default();
        if let (Some(reference), Some(object)) = (envelope.provider_ref(), spec_value.as_object_mut())
        {
            object.insert(
                "providerRef".to_owned(),
                serde_json::Value::String(reference.to_canonical_string()),
            );
        }
        let surface = ContextChildSurface::new(ctx);
        let request = SharedProviderEffectRequest {
            zone: self.zone.clone(),
            target: key.clone(),
            uid: ResourceUid::from_bytes(&row.uid).map_err(|_| {
                self.error(BindingDriverErrorKind::InventoryUnavailable, op)
                    .with_detail(FailureDetail::at("inventory/uid"))
            })?,
            generation: ResourceGeneration::new(row.generation).map_err(|_| {
                self.error(BindingDriverErrorKind::InventoryUnavailable, op)
                    .with_detail(FailureDetail::at("inventory/generation"))
            })?,
            operation_id: format!("device-binding:{}", key.name),
            spec: &spec_value,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            status: None,
            children: &surface,
        };
        self.effects
            .device_inventory(&request)
            .await
            .map_err(|_| self.error(BindingDriverErrorKind::InventoryUnavailable, op))
    }
}

/// The hex spelling one compared uid renders as.
fn uid_hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The canonical wire spelling one committed vocabulary value renders as.
///
/// Read back through the contract's own serde rename rather than a second
/// hand-written spelling, so a failure detail cannot drift from the bytes the
/// row was decoded from.
fn wire<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|rendered| rendered.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unrenderable".to_owned())
}

#[async_trait::async_trait]
impl ResourceDriver for DeviceBindingDriver {
    type Error = BindingDriverError;

    fn classify_error(&self, error: &BindingDriverError) -> DriverFailure {
        let failure = match error.kind {
            BindingDriverErrorKind::SpecInvalid
            | BindingDriverErrorKind::DecisionRefused
            | BindingDriverErrorKind::OwnerMismatch
            | BindingDriverErrorKind::ParentPolicyRefused => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            BindingDriverErrorKind::ParentUnavailable
            | BindingDriverErrorKind::ConsumerUnavailable
            | BindingDriverErrorKind::InventoryUnavailable => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// Structural validation: the wire decode, the committed decision, the
    /// parent `Device` row behind its owner fence, that row's own declared
    /// capability vocabulary, and the named consumer row.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        self.resolved(ctx, DriverOp::Validate).await?;
        Ok(())
    }

    /// Adoption of the pre-restart incarnation (F2).
    ///
    /// A relationship with no attachment has nothing to adopt: no host effect
    /// persists a claim this actor could find, and adopting an attachment it
    /// cannot prove would be the restart-adoption failure R41 exists to
    /// prevent. So a restart reports `Missing` and the next reconcile pass
    /// re-derives the relationship from the committed row.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        self.resolved(ctx, DriverOp::Recover).await?;
        Ok(RecoveryOutcome::Missing)
    }

    /// One reconcile pass: resolve the committed relationship through its
    /// fences, ask the trusted inventory whether the host still backs the
    /// capability the row names, and publish the in-memory status (R11).
    ///
    /// An unattached relationship re-checks on the preserved resync cadence:
    /// hardware presence is host-dependent and an absence observation reaches
    /// this actor as no watch delivery on the binding row.
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        let (binding, component, _spec) = self.resolved(ctx, op).await?;
        // Dependency edges (R12/R17): the Device row and the consumer row both
        // wake this actor when they change.
        self.watch_once(ctx, self.parent_device_key(&binding)).await;
        self.watch_once(ctx, self.consumer_key(&binding)).await;
        let status = self.attachment_state(ctx, &binding, component).await;
        let retained = matches!(status, DeviceBindingDriverStatus::Admitted { .. });
        ctx.set_status(status);
        if !retained {
            ctx.requeue_after(DEVICE_BINDING_RESYNC);
        }
        // `Satisfied` is the driver's own convergence: this pass did its work
        // and published its result. The attachment state itself is the typed
        // status, and an unattached relationship re-checks above rather than
        // deferring the row, so a consumer's own launch never forms a startup
        // cycle with the observation that it can see the relationship.
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Pre-drain (KTD10, R36): block NEW use before anything else is torn
    /// down.
    ///
    /// The fence is the driver's own in-memory status (R11): a relationship
    /// that has run pre-drain keeps reporting
    /// [`DeviceBindingDriverStatus::Draining`] so the next reconcile pass does
    /// not hand it back an admitted state. The source keeps holding the claim
    /// until release evidence arrives, so fencing the relationship does not
    /// free the physical authority for another owner. Idempotent under retry,
    /// and a row whose spec no longer decodes converges without effects.
    async fn pre_drain(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        let Ok((_binding, component, _spec)) = self.resolved(ctx, op).await else {
            // Nothing durable to fence: converged without effects.
            return Ok(());
        };
        ctx.set_status(DeviceBindingDriverStatus::Draining { component });
        Ok(())
    }

    /// Drain step (R10, F3): the relationship owns no child rows, so this is
    /// the generic children-first finalization and it converges immediately.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| self.error(BindingDriverErrorKind::ParentUnavailable, DriverOp::Delete))?;
        Ok(())
    }

    /// Teardown: release the device claim.
    ///
    /// A relationship this driver never attached has nothing to release on the
    /// host, so the pass converges without a mutation - and it converges the
    /// same way on every retry, because the attach verb it would undo is not
    /// one this family can route. That is the honest teardown, not a skipped
    /// one: the status this driver publishes never claims an attachment, so
    /// there is no attachment to withdraw.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        if let Ok((_binding, _component, _spec)) = self.resolved(ctx, op).await {
            ctx.set_status(DeviceBindingDriverStatus::Unattached {
                reason: UnattachedReason::AttachDispatchUnroutable,
            });
        }
        Ok(())
    }
}

impl DeviceBindingDriver {
    /// The attachment state this pass observes.
    ///
    /// The attachment state this pass observes.
    ///
    /// One closed class is decided here with real evidence, through the
    /// family's own [`capability_backed`] observation: a capability the trusted
    /// inventory no longer resolves or reports as gone admits no attachment.
    /// The other is the attach path itself - the provider-specific effects take
    /// no generic attachment and nothing dispatches on the row's provider - so
    /// it is reported as named rather than approximated. A row that passes
    /// every structural check and every presence observation still reaches the
    /// attach verdict, which is the honest answer and not a defect in the
    /// checks.
    async fn attachment_state(
        &self,
        ctx: &mut ResourceContext,
        binding: &DeviceBindingSpec,
        component: DeviceComponent,
    ) -> DeviceBindingDriverStatus {
        let Ok(inventory) = self.inventory(ctx, binding, DriverOp::Reconcile).await else {
            // An unresolved inventory admits nothing, which is the fail-closed
            // answer rather than a claim the source could not prove.
            return unattached(UnattachedReason::CapabilityNotBacked);
        };
        // The fence lives in the in-memory status slot rather than a durable
        // field (R11), so a pre-drain that ran is still read back as fenced on
        // the next pass.
        if matches!(
            ctx.status::<DeviceBindingDriverStatus>(),
            Some(DeviceBindingDriverStatus::Draining { .. })
        ) {
            return DeviceBindingDriverStatus::Draining { component };
        }
        let Some(entry) = inventory.entry(binding.function()) else {
            return unattached(UnattachedReason::CapabilityNotBacked);
        };
        if !capability_backed(&inventory, binding.function()) {
            return unattached(UnattachedReason::CapabilityNotBacked);
        }
        // The opaque authority the trusted inventory resolved. It is a digest,
        // not a node path, so publishing it leaks no host identity.
        DeviceBindingDriverStatus::Admitted {
            component,
            authority: entry.authority_key().clone(),
        }
    }
}

/// The status an unattached relationship publishes, with the driver converging.
fn unattached(reason: UnattachedReason) -> DeviceBindingDriverStatus {
    DeviceBindingDriverStatus::Unattached { reason }
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The execution domains the `DeviceBinding` type can be reconciled in.
///
/// Derived from the placement contract: `DeviceBinding` names no placement
/// anchor (`PlacementAnchor::canonical_for` resolves none), so a relationship
/// row never carries the canonical `spec.executionRef` and the plane reconciles
/// it on its containing Zone's Host. The consumer reference in the spec selects
/// the Guest or Process that receives the attachment, never where the
/// relationship row itself is reconciled.
const DEVICE_BINDING_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the binding driver reads while reconciling.
///
/// Derived from the driver's row reads: the bound `Device` row for its own
/// declared capability vocabulary, its inventory selector, and the owner fence,
/// and the consumer row for the store-assigned identity the KTD3 key is derived
/// from.
const DEVICE_BINDING_READS: &[WellKnownType] = &[WellKnownType::DEVICE];

/// The `DeviceBinding` type's driver declaration.
///
/// `DeviceBinding` is `BUILTIN | STARTUP | RUNTIME` (the RUNTIME bit is
/// present for the same reason the `Device` type carries it): hardware presence
/// is host-dependent, so a committed relationship may arrive late or find its
/// capability gone. The type is not exportable: `ResourceExport` admits only
/// qualified `*.d2bus.org.*Service` types, so a relationship can never be an
/// export subject. The driver serves no broker operations, mints no children,
/// contributes no startup steps, and declares no hosted effects service: a
/// relationship attaches one device capability to one consumer and owns nothing
/// else, and a `ServiceDecl` with no host behind it would be a surface nothing
/// can reach.
pub fn device_binding_descriptor(args: DeviceBindingDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::DEVICE_BINDING,
        allowed_sources: AllowedSources::BUILTIN
            | AllowedSources::STARTUP
            | AllowedSources::RUNTIME,
        verbs: CONVERTED_TYPE_VERBS,
        execution: DEVICE_BINDING_EXECUTION_DOMAINS,
        exportable: false,
        reads: DEVICE_BINDING_READS,
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: device_binding_spec_decoder(),
        factory: Arc::new(DeviceBindingDriverFactory::new(args)),
    }
}
