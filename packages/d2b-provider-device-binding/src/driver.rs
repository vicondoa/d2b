//! The `DeviceBinding` resource driver: the v3 `ResourceDriver` conversion
//! of the row side of the Device source's admitted device relationship.
//!
//! The Device source decides a device claim once, against its trusted
//! inventory (see `d2b-provider-device`'s binding module), and materializes
//! the decision as one `DeviceBinding` row whose desired bytes are the
//! consumer's own request. This driver is the other half: it realizes that
//! row into the host's device mediation and gives the claim back on teardown.
//!
//! What that means concretely, per verb:
//!
//! - `validate` decodes the stored envelope and the canonical
//!   `DeviceBindingRequest` strictly - unknown fields, a source that is not a
//!   `Device`, a consumer kind the binding kind does not admit, and a claim
//!   whose right it does not admit are all refused terminal, because the
//!   shared contract's own constructor is what refuses them - and applies the
//!   owner fence against the parent `Device` row the binding names: a Device
//!   that has been deleted, re-created under another owner, or replaced by a
//!   row that does not decode leaves a claim that can no longer be proven.
//! - `observe` (recover) asks the mediation whether the exact realization this
//!   row declares is still in place. An attachment that survived the restart
//!   is adopted as is: the drive is idempotent, so nothing is claimed twice
//!   and the consumer never sees a second attachment. A realization that did
//!   not survive is reported missing so the actor drives it again.
//! - `reconcile` resolves the parent `Device`, watches it, drives the
//!   realization - the claim and its attachment - through the driver effect
//!   port, then publishes the fenced readiness projection and the in-memory
//!   status. A claim the trusted adapter refuses as unauthorized or stale is
//!   terminal and drives nothing further; a claim conflict, an operational
//!   mediation failure, and a parent that is not observable yet all defer and
//!   retry.
//! - `finalize_binding` (pre-drain) blocks new use by removing the attachment
//!   the consumer was holding, before the generic children-first finalization
//!   runs.
//! - teardown on delete observes the drain gate BEFORE anything is released -
//!   a consumer that still holds the attachment keeps the durable deleting
//!   mark and the claim - and otherwise tears down attachment-first: the
//!   attachment goes before the device slot is released, so a second consumer
//!   can never receive the capability while a stale attachment still exists.
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only) plus the fenced
//!   `status.resource` projection.
//!
//! Conversion mapping:
//! - `describe` -> [`binding_descriptor`] registration under `DeviceBinding`.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`].
//! - `binding_children` readiness -> [`ResourceDriver::reconcile`].
//! - `finalize_binding` drain -> [`ResourceDriver::pre_drain`].
//! - teardown on delete -> [`ResourceDriver::delete`].
//!
//! The family mints no child ([`DEVICE_BINDING_CREATIONS`] is empty): the
//! realized attachment is a mediation over the Device provider's own trusted
//! inventory, whose worker rows the Zone bundle declares and whose Endpoints
//! the Device driver declares, so a binding row that minted an attachment
//! surface of its own would be a second authority over the same realization.
//! It reads exactly one row - the parent `Device` it names
//! ([`DEVICE_BINDING_READS`]) - and takes no authority from it: the physical
//! authority key, the presence of the named capability, and every device-side
//! realization stay with the Device provider's trusted inventory. Everything
//! else the driver needs from outside arrives through the driver effect port
//! ([`DeviceBindingDriverEffects`]), whose production implementation is
//! served by this crate itself ([`crate::effects_service`]) over the declared
//! facets the composition root supplies; the daemon hosts the family's
//! declared effects service
//! ([`crate::effects_service::DEVICE_BINDING_EFFECTS_SERVICE`]) per zone from
//! the family's registered factory.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::{
    BindingRealizationFacet, CanonicalJsonObject, ResourceGeneration, ResourceRef, ResourceSpec,
    ResourceUid, ZoneRevision,
    binding::BindingKind,
    device::DeviceSpec,
    device_binding::{DEVICE_BINDING_RESOURCE_TYPE, DeviceBindingSpec},
};
use d2b_resource_runtime::context::{
    ResourceContext, RowLookup, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, ChildCreation, DriverDescriptor, WellKnownType,
};

use crate::effects_service::DEVICE_BINDING_EFFECTS_SERVICE;
use crate::facets::{
    DeviceAttachment, DeviceBindingEffectFacets, DeviceBindingFence, DeviceEstablishOutcome,
    DeviceRefusal,
};
use crate::row_readers::BindingReadiness;

/// The `DeviceBinding` ResourceType name.
pub const DEVICE_BINDING_TYPE_NAME: &str = DEVICE_BINDING_RESOURCE_TYPE;

/// The children this driver may create.
///
/// The family mints none. The realized attachment is a mediation over the
/// Device provider's own trusted inventory, whose worker rows the Zone bundle
/// declares and whose Endpoints the Device driver declares; a binding row
/// that minted an attachment surface of its own would be a second authority
/// over the same realization.
pub const DEVICE_BINDING_CREATIONS: &[ChildCreation] = &[];

/// The resource types this driver reads while reconciling.
///
/// The parent `Device` the row names. A driver that reconciles a device
/// attachment without reading the device it attaches is the same class of
/// defect as one that guesses the row's shape: a Device that has been deleted,
/// re-created under another owner, or replaced by a row that does not decode
/// leaves a binding whose claim can no longer be proven, and the row refuses
/// rather than keep realizing it. The named capability itself is still
/// resolved against the Device provider's trusted inventory through the effect
/// port, never from a field on this side.
pub const DEVICE_BINDING_READS: &[WellKnownType] = &[WellKnownType::DEVICE];

/// The ResourceType of the row this driver reads: the source Device every
/// device binding names. The spelling is the binding contract's own, so the
/// driver cannot drift from the kind it keys its relationships by.
const DEVICE_TYPE: &str = BindingKind::Device.source_resource_type();

/// The execution domains a `DeviceBinding` row is reconciled in.
///
/// Derived from the placement contract: `DeviceBinding` names no placement
/// anchor, so a binding row never carries the canonical `spec.executionRef`
/// and the plane reconciles it on its containing Zone's Host. The request's
/// consumer reference names who the capability reaches, never where the
/// binding row itself is reconciled.
const DEVICE_BINDING_EXECUTION_DOMAINS: &[&str] = &["host"];

/// Preserved resync cadence while the realized attachment is not yet serving:
/// the pass re-checks on the interval a non-converged provider row uses, so a
/// readiness the port cannot observe yet converges without depending on a
/// watch delivery.
const DEVICE_BINDING_RESYNC: Duration = Duration::from_secs(5);

/// The stable provider reason a not-yet-serving attachment reports.
const REASON_NOT_READY: &str = "device-attachment-not-ready";

// ---------------------------------------------------------------------------
// Driver error
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingDriverErrorKind {
    /// The durable spec did not decode as the strict neutral binding
    /// contract.
    SpecInvalid,
    /// The trusted mediation refused the attachment as unauthorized or
    /// stale: no retry of this row can converge.
    Refused,
    /// The capability is not available to this row yet - a live exclusive
    /// claim elsewhere, or a consumer that still holds the attachment the
    /// teardown is draining.
    NotYet,
    /// The mediation adapter could not complete a drive or a release.
    MediationFailed,
    /// The parent Device row is not observable yet: the manager answered
    /// `Absent` (the row may simply not be committed yet) or could not answer
    /// at all. Retryable by contract (issue #511).
    ParentUnavailable,
    /// The Device row the binding names is present but its owner uid differs
    /// from this binding's owner: the manager would silently re-parent.
    OwnerMismatch,
    /// The parent Device row is present but is not a usable Device row (its
    /// uid or stored spec does not decode).
    ParentSpecInvalid,
    /// The row's committed source decision admits another right than the
    /// claim this row carries.
    RightsNotAdmitted,
    /// The row's committed source decision never claimed device attachment,
    /// so its realized facets do not cover what this row drives.
    FacetNotAdmitted,
}

impl BindingDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::SpecInvalid
            | Self::Refused
            | Self::OwnerMismatch
            | Self::ParentSpecInvalid
            | Self::RightsNotAdmitted
            | Self::FacetNotAdmitted => FailureClass::Terminal,
            Self::NotYet | Self::MediationFailed | Self::ParentUnavailable => FailureClass::Retryable,
        }
    }

    /// The registered failure kind this classification reports (issue #508).
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid => FailureKinds::BINDING_SPEC_INVALID,
            // The binding family's registered kinds carry the volume
            // wording, so the two classifications with no binding-family
            // entry are the registered generic ones - which is exactly what an
            // admission the trusted adapter refused and a capability that is
            // not available to this row yet are.
            Self::Refused => FailureKinds::DRIVER_REFUSED,
            Self::NotYet => FailureKinds::DRIVER_NOT_YET,
            Self::MediationFailed => FailureKinds::BINDING_SERVING_EFFECT_FAILED,
            Self::ParentUnavailable => FailureKinds::BINDING_PARENT_UNAVAILABLE,
            Self::OwnerMismatch => FailureKinds::BINDING_OWNER_MISMATCH,
            Self::ParentSpecInvalid => FailureKinds::BINDING_PARENT_SPEC_INVALID,
            Self::RightsNotAdmitted => FailureKinds::DRIVER_REFUSED,
            Self::FacetNotAdmitted => FailureKinds::DRIVER_REFUSED,
        }
    }
}

/// Typed driver failure; mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`] (R13, issue
/// #508).
#[derive(Debug, Clone)]
pub(crate) struct BindingDriverError {
    kind: BindingDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl BindingDriverError {
    fn new(kind: BindingDriverErrorKind, op: DriverOp) -> Self {
        Self {
            kind,
            op,
            detail: FailureDetail::new(),
        }
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
            BindingDriverErrorKind::Refused => "driver-refused",
            BindingDriverErrorKind::NotYet => "driver-not-yet",
            BindingDriverErrorKind::MediationFailed => "binding-serving-effect-failed",
            BindingDriverErrorKind::ParentUnavailable => "binding-parent-unavailable",
            BindingDriverErrorKind::OwnerMismatch => "binding-owner-mismatch",
            BindingDriverErrorKind::ParentSpecInvalid => "binding-parent-spec-invalid",
            BindingDriverErrorKind::RightsNotAdmitted => "binding-rights-not-admitted",
            BindingDriverErrorKind::FacetNotAdmitted => "binding-facet-not-admitted",
        })
    }
}

impl std::error::Error for BindingDriverError {}

/// Typed in-memory status projection (R11: never persisted). Carries the
/// exact attachment the pass drove, so a reader and the graph cannot disagree
/// about which relationship this row is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BindingDriverStatus {
    /// The attachment is realized and the pass observed its readiness.
    Realized {
        attachment: DeviceAttachment,
        /// The realization was already in place (this pass changed nothing on
        /// the host); the manager requeues otherwise.
        converged: bool,
        ready: bool,
    },
    /// The exact pre-restart realization was still in place and was adopted.
    Recovered {
        attachment: DeviceAttachment,
        ready: bool,
    },
    /// A terminal refusal (old `failed_binding_result`): the stable provider
    /// reason stays visible in memory while the actor publishes the Failed
    /// phase (KTD5).
    Rejected { reason: &'static str },
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one `DeviceBinding` row, exactly as persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BindingSpecEnvelope {
    provider_ref: Option<ResourceRef>,
    base: CanonicalJsonObject,
}

/// The manager-wired decode hook for `DeviceBinding` rows.
///
/// The row's base layer is the strict [`DeviceBindingSpec`] the graph
/// publishes, so this decodes the shared contract's own type rather than a
/// copy of it: there is no second field set in this crate that could drift
/// from the graph's. Every row field the driver reads is reached through
/// [`DeviceAttachment`]'s accessors, so the row contract is named in exactly
/// one place - [`DeviceBindingDriver::attachment`].
pub fn binding_spec_decoder() -> Arc<dyn SpecDecoder> {
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

/// The provider-facing realization surface the binding driver needs.
///
/// The production implementation is this crate's own effects service (U6),
/// built from the declared facets the composition root supplies; test doubles
/// implement the same seam (R4). The driver never touches the host: it asks
/// for the claim, the readiness, the drain evidence, and the two releases,
/// and it refuses whatever the trusted adapter refuses.
#[async_trait::async_trait]
pub trait DeviceBindingDriverEffects: Send + Sync + 'static {
    /// Realize the claim and the attachment this row declares, or report the
    /// exact realization is already in place.
    ///
    /// # Errors
    ///
    /// Returns the [`DeviceRefusal`] the trusted adapter refused with.
    async fn establish(
        &self,
        attachment: &DeviceAttachment,
    ) -> Result<DeviceEstablishOutcome, DeviceRefusal>;

    /// Whether the realized attachment reaches its consumer now.
    async fn attachment_ready(&self, attachment: &DeviceAttachment) -> bool;

    /// Whether the consumer still holds the realized attachment: the drain
    /// gate the teardown reads before anything is released.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the observation could not complete; the caller
    /// fails closed.
    async fn attachment_held(&self, attachment: &DeviceAttachment) -> Result<bool, String>;

    /// Remove the realized attachment the consumer was holding.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the attachment could not be removed. One that was
    /// never realized answers `Ok(())`.
    async fn release_attachment(&self, attachment: &DeviceAttachment) -> Result<(), String>;

    /// Release the consumer's device slot, handing the claim back to the
    /// source that arbitrated it.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the slot could not be released. One that is already
    /// free answers `Ok(())`.
    async fn release_slot(&self, attachment: &DeviceAttachment) -> Result<(), String>;
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Everything the plane must construct to instantiate the binding driver
/// factory for one zone.
///
/// The zone is not factory wiring: the row's own key is the zone authority,
/// so every attachment this driver drives names the zone the row lives in.
pub struct DeviceBindingDriverArgs {
    /// The daemon-supplied facet set the family's effects are built from
    /// (R2): the claim-and-attach drive, the two release steps, and the two
    /// observations. The family never receives a daemon-built effect port.
    pub facets: DeviceBindingEffectFacets,
}

/// [`ResourceDriverFactory`] for the `DeviceBinding` resource type.
/// Construction is infallible by contract (R3).
pub(crate) struct DeviceBindingDriverFactory {
    types: [ResourceTypeName; 1],
    args: DeviceBindingDriverArgs,
}

impl DeviceBindingDriverFactory {
    pub(crate) fn new(args: DeviceBindingDriverArgs) -> Self {
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
        // The driver builds its effects from the declared facets; no
        // externally built port appears at this construction site (R2).
        Box::new(DeviceBindingDriver::new(Arc::new(
            crate::effects_service::DeviceBindingEffectsService::new(self.args.facets.clone()),
        )))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One `DeviceBinding` resource's driver.
#[derive(Clone)]
pub(crate) struct DeviceBindingDriver {
    effects: Arc<dyn DeviceBindingDriverEffects>,
    /// Targets this driver already registered a dependency watch on (R12/R17).
    /// Runtime-only (R6/R11): one registration per target keeps the dependency
    /// edge that wakes the actor on dependency death or readiness without
    /// accumulating manager watch entries.
    watched: Vec<ResourceKey>,
}

impl DeviceBindingDriver {
    pub(crate) fn new(effects: Arc<dyn DeviceBindingDriverEffects>) -> Self {
        Self {
            effects,
            watched: Vec::new(),
        }
    }

    /// The key of the parent Device this binding names.
    fn device_key(
        &self,
        ctx: &ResourceContext,
        attachment: &DeviceAttachment,
    ) -> ResourceKey {
        ResourceKey::new(
            ctx.key().zone.as_str(),
            DEVICE_TYPE,
            attachment.device().name().as_str(),
        )
    }

    /// The parent `Device` row this binding names, read through the manager
    /// (R2: the driver never touches the spec store).
    ///
    /// The declared Device must be the resource the manager reports as this
    /// row's owner when the row declares one - a child cannot silently change
    /// owner (R8) - and it must be a usable Device row. A parent that is not
    /// observable yet defers (issue #511): the row may simply not be committed
    /// yet, which is not terminal evidence against the binding.
    async fn parent_device(
        &self,
        ctx: &mut ResourceContext,
        attachment: &DeviceAttachment,
        op: DriverOp,
    ) -> Result<(), BindingDriverError> {
        let key = self.device_key(ctx, attachment);
        let lookup = ctx.lookup(&key).await;
        let row = match lookup {
            RowLookup::Present { row, .. } => row,
            _ => {
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
        ResourceUid::from_bytes(&row.uid)
            .map_err(|_| self.parent_spec_invalid(op, "parent.uid"))?;
        let envelope = serde_json::from_slice::<ResourceSpec>(&row.spec)
            .map_err(|_| self.parent_spec_invalid(op, "parent.spec"))?;
        serde_json::from_slice::<DeviceSpec>(&envelope.base().to_canonical_bytes())
            .map_err(|_| self.parent_spec_invalid(op, "parent.spec"))?;
        Ok(())
    }

    /// The terminal classification for a present parent row whose stored
    /// identity or spec does not decode: the committed rows cannot converge by
    /// retrying (issue #508: this is not an ownership mismatch).
    fn parent_spec_invalid(&self, op: DriverOp, field: &'static str) -> BindingDriverError {
        self.error(BindingDriverErrorKind::ParentSpecInvalid, op)
            .with_detail(
                FailureDetail::at("parent/decode").comparison(FailureComparison::new(
                    field,
                    "a canonical Device row",
                    "decode failed",
                )),
            )
    }

    /// Register one dependency watch (R12/R17) exactly once per target.
    ///
    /// Best-effort by design: a dependency that is still an unconverted row
    /// has no actor to watch yet, and the resync requeue re-evaluates those
    /// rows until they are served.
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) {
            return;
        }
        if ctx.watch(target.clone(), WatchCondition::Ready).await.is_ok() {
            self.watched.push(target);
        }
    }

    fn error(&self, kind: BindingDriverErrorKind, op: DriverOp) -> BindingDriverError {
        BindingDriverError::new(kind, op)
    }

    /// The committed request plus the fence it is realized under, built once
    /// per pass from the row's own identity.
    ///
    /// The fence's observation revision is the row revision the manager plane
    /// publishes - the row generation, which the manager maps onto the wire
    /// revision: the new store carries no separate Zone revision, so a fence
    /// pinned to any other ordinal would either claim an observation the row
    /// never held or fall behind the row it describes (KTD8). The read side
    /// keeps the guard - a fence ahead of the row's own revision, under
    /// another uid, or under another generation is never current.
    fn attachment(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<DeviceAttachment, BindingDriverError> {
        let envelope = ctx
            .spec::<BindingSpecEnvelope>()
            .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        let spec = serde_json::from_slice::<DeviceBindingSpec>(&envelope.base.to_canonical_bytes())
            .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        self.require_admitted_decision(&spec, op)?;
        let uid = ResourceUid::from_bytes(ctx.uid())
            .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        let generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| self.error(BindingDriverErrorKind::SpecInvalid, op))?;
        Ok(DeviceAttachment::new(
            ctx.key().clone(),
            envelope.provider_ref.clone(),
            spec,
            DeviceBindingFence::new(uid, generation, ZoneRevision::new(generation.get())),
        ))
    }

    /// The row's committed source decision must admit what this family drives.
    ///
    /// A binding row carries the source's accepted decision, not only the
    /// relationship: a row whose decision admits another right than the claim
    /// it declares, or whose realization never claimed device attachment, is a
    /// row this driver must not drive. An unread decision would leave the
    /// field inert, so the check reads committed facts on every verb rather
    /// than only where an attachment is first attached.
    fn require_admitted_decision(
        &self,
        spec: &DeviceBindingSpec,
        op: DriverOp,
    ) -> Result<(), BindingDriverError> {
        let decision = spec.source();
        let requested = spec.claim().requested_rights();
        if !decision.admitted_rights().contains(&requested) {
            return Err(self
                .error(BindingDriverErrorKind::RightsNotAdmitted, op)
                .with_detail(FailureDetail::at("spec/source-decision").comparison(
                    FailureComparison::new(
                        "source.admittedRights",
                        format!("{requested:?}"),
                        "another right set",
                    ),
                )));
        }
        if !decision
            .realized_facets()
            .contains(&BindingRealizationFacet::DeviceAttachment)
        {
            return Err(self
                .error(BindingDriverErrorKind::FacetNotAdmitted, op)
                .with_detail(FailureDetail::at("spec/source-decision").comparison(
                    FailureComparison::new(
                        "source.realizedFacets",
                        "device-attachment",
                        "another facet set",
                    ),
                )));
        }
        Ok(())
    }

    /// Record one refusal in this pass's in-memory status (old
    /// `failed_binding_result`) and return the typed failure: the actor
    /// publishes the Failed phase, and the stable provider reason stays
    /// visible instead of collapsing into a generic error (KTD5).
    fn refused(
        &self,
        ctx: &mut ResourceContext,
        refusal: &DeviceRefusal,
        op: DriverOp,
    ) -> BindingDriverError {
        ctx.set_status(BindingDriverStatus::Rejected {
            reason: refusal.reason(),
        });
        let kind = match refusal {
            DeviceRefusal::Unauthorized { .. } | DeviceRefusal::Stale { .. } => {
                BindingDriverErrorKind::Refused
            }
            DeviceRefusal::MediationFailed { .. } => BindingDriverErrorKind::MediationFailed,
            DeviceRefusal::Conflicted { .. } => BindingDriverErrorKind::NotYet,
        };
        self.error(kind, op).with_detail(
            FailureDetail::at("attachment/establish").comparison(FailureComparison::new(
                "attachment.realized",
                "realized",
                refusal.reason(),
            ))
            .with_note(refusal.detail()),
        )
    }

    /// Drive the realization, mapping the adapter's refusal onto the driver's
    /// classification.
    async fn establish(
        &self,
        ctx: &mut ResourceContext,
        attachment: &DeviceAttachment,
        op: DriverOp,
    ) -> Result<DeviceEstablishOutcome, BindingDriverError> {
        match self.effects.establish(attachment).await {
            Ok(outcome) => Ok(outcome),
            Err(refusal) => {
                if !refusal.is_terminal() {
                    tracing::warn!(
                        binding = %attachment.binding().name,
                        reason = refusal.reason(),
                        detail = refusal.detail(),
                        "the device mediation could not realize this attachment yet",
                    );
                }
                Err(self.refused(ctx, &refusal, op))
            }
        }
    }

    /// The retryable classification of a failed release, naming which step
    /// stopped the teardown.
    fn released(&self, error: String, stage: &'static str, op: DriverOp) -> BindingDriverError {
        self.error(BindingDriverErrorKind::MediationFailed, op)
            .with_detail(
                FailureDetail::at(stage)
                    .comparison(FailureComparison::new(
                        "attachment.released",
                        "released",
                        "release failed",
                    ))
                    .with_note(error),
            )
    }

    /// The fenced readiness this pass publishes for the row.
    fn readiness(
        &self,
        attachment: &DeviceAttachment,
        ready: bool,
        reason: Option<&str>,
    ) -> BindingReadiness {
        BindingReadiness::new(ready, attachment.fence().clone(), reason)
    }
}

/// The hex spelling one compared uid renders as (issue #508).
fn uid_hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[async_trait::async_trait]
impl ResourceDriver for DeviceBindingDriver {
    type Error = BindingDriverError;

    fn classify_error(&self, error: &BindingDriverError) -> DriverFailure {
        let failure = match error.kind {
            BindingDriverErrorKind::SpecInvalid
            | BindingDriverErrorKind::Refused
            | BindingDriverErrorKind::OwnerMismatch
            | BindingDriverErrorKind::ParentSpecInvalid
            | BindingDriverErrorKind::RightsNotAdmitted
            | BindingDriverErrorKind::FacetNotAdmitted => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            BindingDriverErrorKind::NotYet | BindingDriverErrorKind::ParentUnavailable => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            BindingDriverErrorKind::MediationFailed => {
                DriverFailure::error(error.op, error.kind.failure_kind(), error.kind.class())
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// Spec decode against the canonical contract, plus the parent fence:
    /// unknown fields, a source that is not a `Device`, a consumer kind this
    /// binding kind does not admit, and a claim whose right it does not admit
    /// are refused terminal, and so is a Device row this binding declares but
    /// cannot own.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Validate;
        let attachment = self.attachment(ctx, op)?;
        self.parent_device(ctx, &attachment, op).await?;
        Ok(())
    }

    /// Owned-realization adoption on restart (F2): the exact realization this
    /// row declares is adopted when the mediation reports it already in
    /// place. The drive is idempotent, so adoption never takes a second claim
    /// and never attaches the consumer twice; a realization that did not
    /// survive the restart is reported missing so the actor drives it.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let op = DriverOp::Recover;
        let attachment = self.attachment(ctx, op)?;
        // A binding whose Device is gone is not an adopted realization: the
        // claim it holds can no longer be proven against a source that is not
        // there.
        self.parent_device(ctx, &attachment, op).await?;
        if self.establish(ctx, &attachment, op).await? == DeviceEstablishOutcome::Realized {
            return Ok(RecoveryOutcome::Missing);
        }
        let ready = self.effects.attachment_ready(&attachment).await;
        ctx.set_status(BindingDriverStatus::Recovered { attachment, ready });
        Ok(RecoveryOutcome::Adopted)
    }

    /// One reconcile pass: decode the committed request, drive the realization
    /// through the effect port, and publish the fenced readiness projection
    /// plus the in-memory status (R11).
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        let attachment = self.attachment(ctx, op)?;
        self.parent_device(ctx, &attachment, op).await?;
        // Dependency edge (R12/R17): a Device change wakes this actor.
        self.watch_once(ctx, self.device_key(ctx, &attachment)).await;
        let outcome = self.establish(ctx, &attachment, op).await?;
        // Readiness is the mediation's own observation of the realized
        // attachment: a binding the port cannot observe is not ready, and
        // reports itself so rather than claiming a capability it could not
        // prove.
        let ready = self.effects.attachment_ready(&attachment).await;
        ctx.set_status(BindingDriverStatus::Realized {
            attachment: attachment.clone(),
            converged: !outcome.mutated(),
            ready,
        });
        ctx.set_status_projection(
            self.readiness(&attachment, ready, (!ready).then_some(REASON_NOT_READY))
                .to_projection(),
        );
        if outcome.mutated() || !ready {
            // The realization changed this pass, or the attachment is not
            // serving yet: re-check on the preserved resync cadence so a
            // readiness the port cannot observe yet converges without
            // depending on a watch delivery.
            ctx.requeue_after(DEVICE_BINDING_RESYNC);
        }
        // `Satisfied` is the *driver's* convergence - this pass did its work -
        // not the serving readiness, which is the fenced projection set
        // above. Deferring the row while its attachment is still coming up
        // would publish `Pending` for the consumer the attachment reaches.
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Pre-drain (KTD10, R6, R36): block new use by removing the attachment
    /// the consumer was holding, before the generic children-first
    /// finalization runs. Idempotent under retry; a row that never realized
    /// its attachment answers `Ok(())`.
    async fn pre_drain(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        let attachment = self.attachment(ctx, op)?;
        self.effects
            .release_attachment(&attachment)
            .await
            .map_err(|error| self.released(error, "pre-drain/attachment", op))
    }

    /// Teardown with the drain gate and the preserved attachment-first
    /// ordering: the consumer's hold on the attachment is observed BEFORE
    /// anything is released - a held attachment keeps the durable deleting
    /// mark and the claim, and the pass retries instead of yanking a
    /// capability out from under a live consumer. Otherwise the attachment
    /// goes first and the device slot is released last, so the capability
    /// becomes assignable again only once nothing still holds it. Idempotent
    /// under retry (R10): a row that never realized its attachment releases
    /// nothing and converges.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        let attachment = self.attachment(ctx, op)?;
        if self
            .effects
            .attachment_held(&attachment)
            .await
            .unwrap_or(false)
        {
            return Err(self.error(BindingDriverErrorKind::NotYet, op).with_detail(
                FailureDetail::at("delete/attachment").comparison(FailureComparison::new(
                    "attachment.held",
                    "released",
                    "the consumer still holds it",
                )),
            ));
        }
        self.effects
            .release_attachment(&attachment)
            .await
            .map_err(|error| self.released(error, "delete/attachment", op))?;
        self.effects
            .release_slot(&attachment)
            .await
            .map_err(|error| self.released(error, "delete/slot", op))
    }
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The `DeviceBinding` type's driver declaration.
///
/// `DeviceBinding` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane cannot
/// serve the converted binding shapes without it, so it must be registered
/// before the plane opens. The type is not exportable: `ResourceExport`
/// admits only qualified `*.d2bus.org.*Service` types, so a binding can never
/// be an export subject. The driver serves no broker operations and
/// contributes no startup steps; the family declares no child creation (see
/// [`DEVICE_BINDING_CREATIONS`]) and reads no row (see
/// [`DEVICE_BINDING_READS`]), and the declaration carries the family's
/// declared effects service
/// ([`crate::effects_service::DEVICE_BINDING_EFFECTS_SERVICE`]), which the
/// daemon hosts per zone from the family's registered factory.
pub fn binding_descriptor(args: DeviceBindingDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::DEVICE_BINDING,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: DEVICE_BINDING_EXECUTION_DOMAINS,
        exportable: false,
        reads: DEVICE_BINDING_READS,
        operations: &[],
        creations: DEVICE_BINDING_CREATIONS,
        startup: &[],
        services: &[DEVICE_BINDING_EFFECTS_SERVICE],
        decoder: binding_spec_decoder(),
        factory: Arc::new(DeviceBindingDriverFactory::new(args)),
    }
}

// ---------------------------------------------------------------------------
// Tests: the driver verbs over a scripted mediation port and a recording
// manager endpoint with one shared ordered log (the drain gate and the
// attachment-first release are asserted as the two record them).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use d2b_contracts_resource::v3::device::DEVICE_RESOURCE_TYPE;
    use d2b_contracts_resource::v3::device_binding::DeviceClaimRequest;
    use d2b_contracts_resource::v3::{ResourceGeneration, ResourceUid, ZoneRevision};
    use d2b_provider_toolkit::testing::fakes::{RecordingManagerEndpoint, RecordingRequeue};
    use d2b_resource_runtime::context::ResourceContext;
    use d2b_resource_runtime::driver::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{FailureClass, FailureKinds};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};

    use super::{
        BindingDriverStatus, DeviceBindingDriverArgs, DeviceBindingDriverFactory, REASON_NOT_READY,
        binding_spec_decoder,
    };
    use crate::row_readers::BindingReadiness;
    use crate::test_support::{FakeAttachmentEffects, ScriptedRefusal};

    // -- fixtures ------------------------------------------------------------

    /// The canonical minimal Device spec the parent row carries.
    fn device_row(uid: [u8; 16], owner_uid: Option<[u8; 16]>) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", DEVICE_RESOURCE_TYPE, "gpu0"),
            uid,
            generation: 2,
            owner_uid,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: serde_json::to_vec(
                &d2b_contracts_resource::v3::device::DeviceSpec::emulated_exclusive(),
            )
            .expect("the canonical Device spec serializes"),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    /// The exact desired bytes the Device source commits for one admitted
    /// relationship: the canonical row spec inside the spec-store envelope.
    fn binding_row() -> StoredDesiredResource {
        let spec = serde_json::json!({
            "providerRef": "Provider/device-gpu",
            "deviceRef": "Device/gpu0",
            "executionRef": "Process/render",
            "slot": "gpu0",
            "function": "render",
            "claim": "exclusive",
            "source": {
                "admittedRights": ["exclusive"],
                "arbitration": "exclusive",
                "realizedFacets": ["device-attachment"],
            },
        });
        StoredDesiredResource {
            key: binding_key(),
            uid: [0x42; 16],
            generation: 1,
            // The Device source materializes the row as an owned child of the
            // Device it names, so the owner fence is live for this row.
            owner_uid: Some(DEVICE_UID),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: spec.to_string().into_bytes(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    /// The uid every fixture's parent Device row carries, and the owner uid
    /// the binding row declares.
    const DEVICE_UID: [u8; 16] = [0x21; 16];

    fn binding_key() -> ResourceKey {
        ResourceKey::new("work", "DeviceBinding", "dev-binding-000000000000000000000000")
    }

    /// The manager double every fixture reads the parent Device row through.
    fn manager() -> RecordingManagerEndpoint {
        RecordingManagerEndpoint::new().with_row(device_row(DEVICE_UID, None))
    }

    struct Fixture {
        ctx: ResourceContext,
        requeue: RecordingRequeue,
    }

    fn fixture(row: StoredDesiredResource) -> Fixture {
        fixture_with(row, manager())
    }

    fn fixture_with(row: StoredDesiredResource, manager: RecordingManagerEndpoint) -> Fixture {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let requeue = RecordingRequeue::default();
        let ctx = ResourceContext::new(
            row,
            binding_spec_decoder(),
            Arc::new(manager),
            Arc::new(requeue.clone()),
            effects_tx,
            notify_tx,
        );
        Fixture { ctx, requeue }
    }

    async fn driver(effects: Arc<FakeAttachmentEffects>) -> Box<dyn DynResourceDriver> {
        DeviceBindingDriverFactory::new(DeviceBindingDriverArgs {
            facets: effects.facet_set(),
        })
        .create(&binding_key())
        .await
    }

    // -- validate ------------------------------------------------------------

    /// A malformed spec is refused terminal before anything is driven: bytes
    /// that are not an envelope, a canonical row spec with an unknown field
    /// beside it, a source that is not a `Device`, a consumer that is not a
    /// binding consumer at all, and a claim mode, slot, or function the
    /// contract does not admit.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_a_malformed_spec_before_anything_is_driven() {
        let fake = FakeAttachmentEffects::new();
        let request = |mutations: serde_json::Value| {
            let mut spec = serde_json::json!({
                "deviceRef": "Device/gpu0",
                "executionRef": "Process/render",
                "slot": "gpu0",
                "function": "render",
                "claim": "exclusive",
            });
            let object = spec.as_object_mut().expect("spec object");
            for (field, value) in mutations.as_object().expect("mutations") {
                match value {
                    serde_json::Value::Null => {
                        object.remove(field);
                    }
                    value => {
                        object.insert(field.clone(), value.clone());
                    }
                }
            }
            spec.to_string().into_bytes()
        };

        for (label, spec) in [
            ("bytes that are not an envelope", b"not a binding envelope".to_vec()),
            (
                "a request carrying a device node path",
                request(serde_json::json!({ "deviceNode": "/dev/dri/renderD128" })),
            ),
            (
                "a source that is not a Device",
                request(serde_json::json!({ "deviceRef": "Volume/data" })),
            ),
            (
                "a claim mode the contract does not admit",
                request(serde_json::json!({ "claim": "observe" })),
            ),
            (
                "a consumer that is not a binding consumer at all",
                request(serde_json::json!({ "executionRef": "Device/gpu1" })),
            ),
            (
                "a slot that is not a bounded token",
                request(serde_json::json!({ "slot": "GPU 0" })),
            ),
            (
                "a function that is not a bounded token",
                request(serde_json::json!({ "function": "render/0" })),
            ),
        ] {
            let mut row = binding_row();
            row.spec = spec;
            let mut f = fixture(row);
            let mut d = driver(Arc::clone(&fake)).await;
            let failure = match d.validate(&mut f.ctx).await {
                Ok(()) => panic!("{label} must be refused"),
                Err(failure) => failure,
            };
            assert_eq!(failure.class(), FailureClass::Terminal, "{label}");
            assert_eq!(
                failure.kind(),
                FailureKinds::BINDING_SPEC_INVALID,
                "{label}"
            );
            assert!(
                fake.call_order().is_empty(),
                "{label} drives nothing: {:?}",
                fake.call_order()
            );
        }
    }


    /// A row whose committed source decision never admitted what it declares
    /// is refused before anything is driven: an unread decision would leave
    /// the field inert, and a row the source did not admit is not a row this
    /// driver may realize.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_row_whose_committed_source_decision_does_not_admit_it_is_refused() {
        for (label, source) in [
            (
                "a decision that admits another right",
                serde_json::json!({
                    "admittedRights": ["share"],
                    "arbitration": "shared",
                    "realizedFacets": ["device-attachment"],
                }),
            ),
            (
                "a decision that never claimed device attachment",
                serde_json::json!({
                    "admittedRights": ["exclusive"],
                    "arbitration": "exclusive",
                    "realizedFacets": ["filesystem-presentation"],
                }),
            ),
        ] {
            let fake = FakeAttachmentEffects::new();
            let mut row = binding_row();
            let mut spec: serde_json::Value =
                serde_json::from_slice(&row.spec).expect("fixture spec");
            spec.as_object_mut()
                .expect("spec object")
                .insert("source".to_owned(), source);
            row.spec = spec.to_string().into_bytes();

            let mut f = fixture(row);
            let mut d = driver(Arc::clone(&fake)).await;
            let failure = d
                .validate(&mut f.ctx)
                .await
                .expect_err("an unadmitted row must be refused");
            assert_eq!(failure.class(), FailureClass::Terminal, "{label}");
            assert_eq!(failure.kind(), FailureKinds::DRIVER_REFUSED, "{label}");
            assert_eq!(
                fake.call_order(),
                Vec::<String>::new(),
                "an unadmitted row never reaches the mediation: {label}"
            );
        }
    }

    // -- reconcile: unauthorized and stale attachments ------------------------

    /// An attachment the trusted adapter refuses as unauthorized or stale is
    /// terminal, keeps its stable provider reason in the in-memory status,
    /// and never claims the capability: the row reports itself refused rather
    /// than reporting a device use it could not prove.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_refuses_an_unauthorized_or_stale_attachment() {
        for (refusal, reason) in [
            (ScriptedRefusal::Unauthorized, "device-function-unauthorized"),
            (ScriptedRefusal::Stale, "device-attachment-stale"),
        ] {
            let fake = FakeAttachmentEffects::new();
            fake.refuse_establish(refusal);
            let mut f = fixture(binding_row());
            let mut d = driver(Arc::clone(&fake)).await;

            let failure = d
                .reconcile(&mut f.ctx)
                .await
                .expect_err("a refused attachment is terminal");
            assert_eq!(failure.class(), FailureClass::Terminal, "{reason}");
            assert_eq!(failure.kind(), FailureKinds::DRIVER_REFUSED, "{reason}");
            assert!(
                matches!(
                    f.ctx.status::<BindingDriverStatus>(),
                    Some(BindingDriverStatus::Rejected { reason: reported }) if *reported == reason
                ),
                "the stable provider reason stays visible: {reason}"
            );
            assert_eq!(
                fake.call_order(),
                vec!["establish".to_owned()],
                "a refused row observes nothing further and claims nothing"
            );
            assert!(
                f.ctx.take_status_projection().is_none(),
                "a refused pass publishes no readiness it cannot prove"
            );
            assert!(
                f.requeue.scheduled().is_empty(),
                "a terminal refusal does not requeue"
            );
        }
    }

    /// A claim another live relationship holds, and an operational mediation
    /// failure, both defer retryably instead of failing the row terminal: the
    /// source's release evidence and a later adapter pass can still converge
    /// them.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_defers_a_conflict_and_an_operational_mediation_failure() {
        for (refusal, kind) in [
            (ScriptedRefusal::Conflicted, FailureKinds::DRIVER_NOT_YET),
            (
                ScriptedRefusal::MediationFailed,
                FailureKinds::BINDING_SERVING_EFFECT_FAILED,
            ),
        ] {
            let fake = FakeAttachmentEffects::new();
            fake.refuse_establish(refusal);
            let mut f = fixture(binding_row());
            let mut d = driver(Arc::clone(&fake)).await;
            let failure = d
                .reconcile(&mut f.ctx)
                .await
                .expect_err("a deferred attachment does not converge yet");
            assert_eq!(failure.class(), FailureClass::Retryable);
            assert_eq!(failure.kind(), kind);
            assert!(
                matches!(
                    f.ctx.status::<BindingDriverStatus>(),
                    Some(BindingDriverStatus::Rejected { .. })
                ),
                "the adapter's own reason stays visible while the pass retries"
            );
        }
    }

    // -- the parent Device row ------------------------------------------------

    /// A Device that is not observable yet defers retryably - the row may
    /// simply not be committed yet, and that is not terminal evidence against
    /// the binding - while a Device row that is present under another owner, or
    /// one whose stored spec does not decode, is terminal: the committed rows
    /// cannot converge by retrying.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_parent_device_row_gates_the_binding_without_a_second_authority() {
        let fake = FakeAttachmentEffects::new();

        // Not committed yet.
        let mut f = fixture_with(binding_row(), RecordingManagerEndpoint::new());
        let mut d = driver(Arc::clone(&fake)).await;
        let failure = d
            .reconcile(&mut f.ctx)
            .await
            .expect_err("an absent Device row defers");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_UNAVAILABLE);
        assert_eq!(
            fake.call_order(),
            Vec::<String>::new(),
            "a binding whose Device is not there claims nothing"
        );

        // The manager cannot answer.
        let unanswerable = RecordingManagerEndpoint::new().with_row(device_row(DEVICE_UID, None));
        unanswerable.set_fail_reads(true);
        let mut f = fixture_with(binding_row(), unanswerable);
        let mut d = driver(Arc::clone(&fake)).await;
        let failure = d
            .reconcile(&mut f.ctx)
            .await
            .expect_err("an unanswerable manager defers");
        assert_eq!(
            failure.class(),
            FailureClass::Retryable,
            "an unanswerable plane is never reported as an absent Device"
        );

        // Present, but owned by another resource: terminal, and the row keeps
        // the claim rather than re-parenting itself.
        let foreign = RecordingManagerEndpoint::new().with_row(device_row([0x99; 16], None));
        let mut f = fixture_with(binding_row(), foreign);
        let mut d = driver(Arc::clone(&fake)).await;
        let failure = d
            .reconcile(&mut f.ctx)
            .await
            .expect_err("a foreign Device row is refused");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_OWNER_MISMATCH);

        // Present, but not a usable Device row: terminal, and distinct from
        // the deferral above.
        let mut broken = device_row(DEVICE_UID, None);
        broken.spec = b"not a device spec".to_vec();
        let mut f = fixture_with(binding_row(), RecordingManagerEndpoint::new().with_row(broken));
        let mut d = driver(Arc::clone(&fake)).await;
        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("an undecodable Device row is refused");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_SPEC_INVALID);
        assert_eq!(
            fake.call_order(),
            Vec::<String>::new(),
            "a refused row never reaches the mediation"
        );
    }

    /// The parent Device is a declared dependency edge, registered once per
    /// target: a Device change wakes this actor, and repeated passes do not
    /// accumulate watch entries.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_parent_device_edge_is_watched_once_per_target() {
        let fake = FakeAttachmentEffects::new();
        let manager = manager();
        let mut f = fixture_with(binding_row(), manager.clone());
        let mut d = driver(fake).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile one");
        d.reconcile(&mut f.ctx).await.expect("reconcile two");

        let targets = manager
            .watch_targets()
            .into_iter()
            .map(|key| format!("{}/{}", key.type_name, key.name))
            .collect::<Vec<_>>();
        assert!(
            targets.contains(&"Device/gpu0".to_owned()),
            "the parent Device is the declared dependency edge: {targets:?}"
        );
        assert_eq!(
            targets.iter().filter(|target| *target == "Device/gpu0").count(),
            1,
            "one registration per target: {targets:?}"
        );
    }

    // -- reconcile: the realized attachment -----------------------------------

    /// A realized attachment reports itself serving under its own fence, and a
    /// pass that changed nothing converges; while the port cannot observe the
    /// attachment, the row reports the frozen not-ready reason and keeps
    /// re-checking on the preserved cadence.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_publishes_the_fenced_readiness_of_the_realized_attachment() {
        let fake = FakeAttachmentEffects::new();
        let mut f = fixture(binding_row());
        let mut d = driver(Arc::clone(&fake)).await;

        // First pass realizes the attachment; the port cannot observe it yet.
        assert_eq!(
            d.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        let pending = f
            .ctx
            .take_status_projection()
            .expect("the concluding pass publishes the fenced projection");
        let typed =
            BindingReadiness::from_projection(&pending).expect("the typed fenced projection");
        assert!(!typed.ready(), "no attachment was observed serving");
        assert_eq!(
            typed.reason().map(|code| code.as_str()),
            Some(REASON_NOT_READY),
            "the not-ready projection carries the frozen provider reason"
        );
        let fence = typed.fence().clone();
        assert_eq!(fence.generation().get(), f.ctx.generation());
        assert_eq!(fence.revision().get(), f.ctx.generation());
        assert!(
            fence.matches(
                &ResourceUid::from_bytes(f.ctx.uid()).expect("the row uid is canonical"),
                ResourceGeneration::new(f.ctx.generation()).expect("generation"),
                ZoneRevision::new(f.ctx.generation()),
            ),
            "the fence names this row's own identity"
        );
        assert_eq!(f.requeue.scheduled().len(), 1, "the pass re-checks");
        assert!(matches!(
            f.ctx.status::<BindingDriverStatus>(),
            Some(BindingDriverStatus::Realized {
                converged: false,
                ready: false,
                ..
            })
        ));

        // Second pass: the realization is already in place, so the pass
        // changes nothing on the host and converges.
        d.reconcile(&mut f.ctx).await.expect("reconcile again");
        assert_eq!(f.requeue.scheduled().len(), 2);
        assert!(matches!(
            f.ctx.status::<BindingDriverStatus>(),
            Some(BindingDriverStatus::Realized {
                converged: true,
                ready: false,
                ..
            })
        ));
        assert_eq!(
            fake.realized_count(),
            1,
            "the second pass found the claim in place and attached nothing"
        );

        // Once the port observes the attachment, the row reports ready and
        // stops requeueing.
        fake.make_ready();
        d.reconcile(&mut f.ctx).await.expect("reconcile third");
        assert_eq!(f.requeue.scheduled().len(), 2);
        let serving = f
            .ctx
            .take_status_projection()
            .expect("every pass republishes the projection");
        let typed =
            BindingReadiness::from_projection(&serving).expect("the typed fenced projection");
        assert!(typed.ready());
        assert!(typed.reason().is_none());
        assert!(matches!(
            f.ctx.status::<BindingDriverStatus>(),
            Some(BindingDriverStatus::Realized {
                converged: true,
                ready: true,
                ..
            })
        ));
        // The attachment the pass drove is the committed relationship, so a
        // reader and the graph cannot disagree about which row this is.
        let attachment = match f.ctx.status::<BindingDriverStatus>() {
            Some(BindingDriverStatus::Realized { attachment, .. }) => attachment,
            other => panic!("expected Realized, got {other:?}"),
        };
        assert_eq!(attachment.binding(), &binding_key());
        assert_eq!(
            attachment.claim(),
            DeviceClaimRequest::Exclusive,
            "the realized claim is the committed one"
        );
    }

    // -- recover: re-adoption rather than a second claim ----------------------

    /// A restart re-adopts the exact realization the row declares instead of
    /// attaching the consumer a second time, and a realization that did not
    /// survive is reported missing so the actor drives it again.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn recover_readopts_the_realized_attachment_instead_of_attaching_twice() {
        let fake = FakeAttachmentEffects::new();
        // Pre-restart: the pass realizes the attachment.
        let mut f = fixture(binding_row());
        let mut d = driver(Arc::clone(&fake)).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        assert_eq!(fake.realized_count(), 1);

        // Restart: a fresh driver over the same durable row adopts the
        // realization the mediation still holds, and takes nothing twice.
        let mut restarted = fixture(binding_row());
        let mut d = driver(Arc::clone(&fake)).await;
        assert_eq!(
            d.recover(&mut restarted.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted,
            "the realization survived the restart: adopt it, do not attach again"
        );
        assert_eq!(
            fake.call_order().iter().filter(|entry| *entry == "establish").count(),
            2,
            "adoption asked the mediation, and it reported the claim already held"
        );
        assert_eq!(
            fake.realized_count(),
            1,
            "the consumer was attached exactly once across the restart"
        );
        assert!(matches!(
            restarted.ctx.status::<BindingDriverStatus>(),
            Some(BindingDriverStatus::Recovered { .. })
        ));

        // A restart whose realization did not survive adopts nothing: the pass
        // reports missing so the actor reconciles.
        let lost = FakeAttachmentEffects::new();
        let mut fresh = fixture(binding_row());
        let mut d = driver(Arc::clone(&lost)).await;
        assert_eq!(
            d.recover(&mut fresh.ctx).await.expect("recover"),
            RecoveryOutcome::Missing,
            "no realization survived: the driver reports it missing"
        );
        assert_eq!(lost.realized_count(), 1);
    }

    // -- teardown: the drain gate and the attachment-first release -------------

    /// A consumer that still holds the attachment keeps the durable deleting
    /// mark and the claim: the gate is observed BEFORE anything is released,
    /// so a live consumer is never yanked off a capability.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_held_attachment_blocks_the_teardown_before_anything_is_released() {
        let fake = FakeAttachmentEffects::new();
        let mut f = fixture(binding_row());
        let mut d = driver(Arc::clone(&fake)).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        fake.make_held();

        let failure = d.delete(&mut f.ctx).await.expect_err("the drain gate blocks");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::DRIVER_NOT_YET);
        let order = fake.call_order();
        assert!(
            !order.iter().any(|entry| entry.starts_with("release-")),
            "nothing may be released while the consumer holds it: {order:?}"
        );
        let gate = order
            .iter()
            .position(|entry| entry == "held")
            .expect("the drain gate was observed");
        let realized = order
            .iter()
            .position(|entry| entry == "establish")
            .expect("the realization was established");
        assert!(realized < gate, "the gate is observed on a realized row: {order:?}");
    }

    /// The teardown releases the device slot: once nothing holds the
    /// attachment, the attachment goes first and the slot last, so the
    /// capability becomes assignable again only once no consumer still holds
    /// it. Both releases are idempotent, so a row that never realized its
    /// attachment converges the same way.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn teardown_releases_the_attachment_before_the_device_slot() {
        let fake = FakeAttachmentEffects::new();
        let mut f = fixture(binding_row());
        let mut d = driver(Arc::clone(&fake)).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let before = fake.call_order().len();

        d.delete(&mut f.ctx).await.expect("delete");
        assert_eq!(
            fake.call_order().into_iter().skip(before).collect::<Vec<String>>(),
            vec![
                "held".to_owned(),
                "release-attachment".to_owned(),
                "release-slot".to_owned(),
            ],
            "the drain gate is observed first, then attachment-first / slot-last"
        );
        assert_eq!(fake.released_slot_count(), 1, "the device slot was released");

        // A row that never realized its attachment releases nothing and
        // converges: a release that was never taken is a no-op.
        let mut cold = fixture(binding_row());
        let mut cold_driver = driver(Arc::clone(&fake)).await;
        let before = fake.call_order().len();
        cold_driver.delete(&mut cold.ctx).await.expect("delete");
        assert_eq!(
            fake.call_order().into_iter().skip(before).collect::<Vec<String>>(),
            vec![
                "held".to_owned(),
                "release-attachment".to_owned(),
                "release-slot".to_owned(),
            ],
            "an unheld attachment releases idempotently"
        );
    }

    /// The pre-drain removes the attachment the consumer was holding before
    /// the row's own teardown runs, and a failed removal defers retryably
    /// rather than dropping the claim under a live consumer.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn pre_drain_removes_the_attachment_and_a_failed_removal_defers() {
        let fake = FakeAttachmentEffects::new();
        let mut f = fixture(binding_row());
        let mut d = driver(Arc::clone(&fake)).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let before = fake.call_order().len();
        d.pre_drain(&mut f.ctx).await.expect("pre-drain");
        assert_eq!(
            fake.call_order().into_iter().skip(before).collect::<Vec<String>>(),
            vec!["release-attachment".to_owned()],
            "the pre-drain releases the attachment and nothing else"
        );

        let fake = FakeAttachmentEffects::new();
        fake.set_fail_release_attachment(true);
        let mut f = fixture(binding_row());
        let mut d = driver(Arc::clone(&fake)).await;
        let failure = d
            .pre_drain(&mut f.ctx)
            .await
            .expect_err("a failed release defers");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SERVING_EFFECT_FAILED);
    }

    /// A teardown whose slot release fails defers retryably after the
    /// attachment is already gone: the retry re-drives the slot, not the
    /// attachment.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_failed_slot_release_defers_after_the_attachment_is_gone() {
        let fake = FakeAttachmentEffects::new();
        fake.set_fail_release_slot(true);
        let mut f = fixture(binding_row());
        let mut d = driver(Arc::clone(&fake)).await;
        let failure = d
            .delete(&mut f.ctx)
            .await
            .expect_err("the slot release failed");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SERVING_EFFECT_FAILED);
        let order = fake.call_order();
        assert_eq!(
            order.last().map(String::as_str),
            Some("release-slot"),
            "the attachment went first: {order:?}"
        );
    }
}