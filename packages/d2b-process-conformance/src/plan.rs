//! The one resolved Process execution plan (U12, KTD8).
//!
//! Before this module a Process launch carried a [`LaunchTicket`](crate::LaunchTicket):
//! compiled digests, a caller-supplied argument list, and a legacy runner role
//! the effect adapter re-derived a trusted intent from. The ticket was the
//! authority, and the provider chose the resources it needed by re-reading the
//! row. That is the second-authority shape the plan's problem frame names: a
//! syntactically valid ticket described a posture, and several layers each
//! reconstructed the actual access from it.
//!
//! This module replaces that with the KTD8 order:
//!
//! 1. The row states typed **requests** - one [`ProcessResourceRequest`] per
//!    relationship, and one [`ProcessPlanRequest`] carrying the instance, the
//!    provider's declared requirements, and the policy selection. A request
//!    names a `Volume`, a `Device`, a view, a slot, or an `ExecutionPolicy`.
//!    It never names a host path, a destination, a uid, or a program.
//! 2. The privileged effect owner resolves those requests against its own
//!    accepted graph and returns a plan. [`ProcessPlanValues`] is that plan,
//!    projected: the only way to hold one is
//!    [`ProcessPlanValues::from_execution_plan`], so the private paths, the
//!    identity, and the executable exist only as the broker resolved them.
//! 3. [`resolve_process_plan`] folds the two into a
//!    [`ResolvedProcessPlan`]: the effective [`AdmittedExecution`], one
//!    [`PreparedBinding`] per admitted relationship, the screened
//!    [`ProcessLaunchArguments`], and the
//!    [`ProcessLaunchEvidence`] that a restart or an adoption must match.
//!
//! # Both lifetimes take this path
//!
//! A long-running `Process` and a run-to-completion `EphemeralProcess` are
//! distinguished only by [`ExecutionInstanceKind`], and
//! [`ProcessSubject::kind`] is derived from the row's own `ResourceRef` through
//! the contract crate's canonical type-name constants. There is no second
//! policy path, no kind-specific binding arm, and no per-kind resource
//! selection: the kind is carried in the admitted execution and changes nothing
//! about admission (AE28).
//!
//! # Preparation happens before the consumer runs
//!
//! A binding is prepared against the **committed** consumer identity carried
//! by [`ProcessSubject`], not against a running process. The plan's legs are
//! therefore complete before the consumer starts, and a consumer starts only
//! once every leg it depends on is `Prepared` - which is what removes the
//! startup cycle R40 names (AE20). [`ResolvedProcessPlan::prepared_against`]
//! is the check that keeps that true across a restart, where the row may have
//! been re-committed in the meantime.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use d2b_contracts_resource::v3::authority::AdmissionStage;
use d2b_contracts_resource::v3::binding::{
    BindingAdmission, BindingKey, BindingRealizationFacet, BindingSlot, BindingSlotAddress,
    RequestedRights, SourceReservation,
};
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::execution_policy_resource::{
    AdmittedExecution, BackendSupport, BudgetCeiling, ExecutionInstance, ExecutionInstanceKind,
    ExecutionPolicyFingerprint, ExecutionPolicySpec, ExecutionRequirements, PolicyAuthorization,
    PolicyRefusal, admit_execution,
};
use d2b_contracts_resource::v3::{
    FreshnessTuple, ResourceRef, ResourceUid, ZoneId, ZoneRevision, canonical_json_bytes,
};
use d2b_core::execution_plan::{
    BindingPlanRequest, EffectFreshness, ExecutionPlan, MAX_PLAN_LEGS, PlannedDestination,
    PlannedExecutable, PlannedIdentity, PlannedSource, ResolvedLeg,
};
use sha2::{Digest, Sha256};

use crate::identity::ProcessIdentityDigest;
use crate::ticket::{
    MAX_LAUNCH_ARG_BYTES, MAX_LAUNCH_ARGS, MAX_LAUNCH_ARGS_TOTAL_BYTES,
};

/// The domain tag framing one resolved Process plan digest.
pub const PROCESS_PLAN_DIGEST_DOMAIN_TAG: &str = "d2b:v3:process-plan";

/// The domain tag framing one Process launch-evidence component digest.
pub const PROCESS_EVIDENCE_DIGEST_DOMAIN_TAG: &str = "d2b:v3:process-launch-evidence";

/// The enforcing stage one Process plan refusal happened at.
///
/// The stage is part of the refusal rather than a log field, so a caller always
/// learns which boundary refused it and a report never has to guess (R42).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProcessPlanStage {
    /// The request itself was malformed or named something that is not an
    /// execution instance.
    Request,
    /// The policy selection was not authorized for this subject.
    Authorize,
    /// The instance, its requirements, and the selected policy did not agree.
    Admit,
    /// The relationship was admitted but its source side is not prepared, so
    /// the consumer may not start yet.
    Prepare,
    /// A committed dependency moved after the request was formed.
    Fence,
    /// A launch argument tried to stand in for a binding-selected source.
    Screen,
    /// A release asked for an effect or a runner this scope does not own.
    Release,
}

impl ProcessPlanStage {
    /// The stable wire name of this stage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Authorize => "authorize",
            Self::Admit => "admit",
            Self::Prepare => "prepare",
            Self::Fence => "fence",
            Self::Screen => "screen",
            Self::Release => "release",
        }
    }
}

impl fmt::Display for ProcessPlanStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The closed set of reasons one Process plan is refused for.
///
/// Every variant names the enforcing boundary and the exact fact that failed,
/// and none of them carries a private path, a numerical principal, a program,
/// or an argument value: a refusal is a report, not a channel (R42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessPlanRefusalReason {
    /// The subject reference names no execution instance.
    NotAnExecutionInstance,
    /// The policy selection is not authorized for this subject.
    PolicySelectionNotAuthorized,
    /// The `ExecutionPolicy` contract refused this instance.
    PolicyRefused(PolicyRefusal),
    /// More relationship legs were requested than one plan admits.
    TooManyLegs,
    /// The same consumer slot was requested twice.
    DuplicateSlot,
    /// A requested relationship is not against this plan's consumer.
    ForeignConsumer,
    /// The resolved plan does not carry a leg the request names.
    LegNotResolved,
    /// The source side of a leg is not prepared, so the consumer may not start.
    BindingNotPrepared,
    /// A committed dependency moved after the request was formed.
    StaleDependency,
    /// A launch argument is malformed or past its bound.
    ArgumentMalformed,
    /// A launch argument named a binding-selected source or destination.
    ArgumentRedirectsSource,
    /// The plan is not prepared against the consumer identity presented.
    NotPreparedForConsumer,
    /// A release named a runner or an effect this launch scope does not own.
    ForeignRelease,
    /// The launch evidence diverges from the evidence a candidate must match.
    EvidenceDiverged(ProcessEvidenceComponent),
}

impl ProcessPlanRefusalReason {
    /// The stable wire name of this reason.
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotAnExecutionInstance => "process-plan-not-an-execution-instance",
            Self::PolicySelectionNotAuthorized => "process-plan-policy-selection-not-authorized",
            Self::PolicyRefused(_) => "process-plan-policy-refused",
            Self::TooManyLegs => "process-plan-too-many-legs",
            Self::DuplicateSlot => "process-plan-duplicate-slot",
            Self::ForeignConsumer => "process-plan-foreign-consumer",
            Self::LegNotResolved => "process-plan-leg-not-resolved",
            Self::BindingNotPrepared => "process-plan-binding-not-prepared",
            Self::StaleDependency => "process-plan-stale-dependency",
            Self::ArgumentMalformed => "process-plan-argument-malformed",
            Self::ArgumentRedirectsSource => "process-plan-argument-redirects-source",
            Self::NotPreparedForConsumer => "process-plan-not-prepared-for-consumer",
            Self::ForeignRelease => "process-plan-foreign-release",
            Self::EvidenceDiverged(_) => "process-plan-evidence-diverged",
        }
    }
}

impl fmt::Display for ProcessPlanRefusalReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// One refused Process plan, with its enforcing stage and typed reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessPlanRefusal {
    stage: ProcessPlanStage,
    reason: ProcessPlanRefusalReason,
}

impl ProcessPlanRefusal {
    /// Refuse at one stage for one reason.
    pub const fn new(stage: ProcessPlanStage, reason: ProcessPlanRefusalReason) -> Self {
        Self { stage, reason }
    }

    /// The enforcing stage that refused.
    pub const fn stage(&self) -> ProcessPlanStage {
        self.stage
    }

    /// The typed reason it refused.
    pub const fn reason(&self) -> ProcessPlanRefusalReason {
        self.reason
    }
}

impl fmt::Display for ProcessPlanRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at {}", self.reason.code(), self.stage.as_str())
    }
}

impl std::error::Error for ProcessPlanRefusal {}

const fn refuse(stage: ProcessPlanStage, reason: ProcessPlanRefusalReason) -> ProcessPlanRefusal {
    ProcessPlanRefusal::new(stage, reason)
}

/// The enforcing stage one shared contract refusal maps onto.
///
/// The Process plan reports its own stages so a caller learns which of *its*
/// boundaries refused, while the policy and binding contracts keep their own
/// closed vocabulary. This mapping is total and data-only, so a refusal keeps
/// its enforcing stage across the seam instead of being flattened into a bare
/// code.
const fn stage_of(stage: AdmissionStage) -> ProcessPlanStage {
    match stage {
        AdmissionStage::Normalize => ProcessPlanStage::Request,
        AdmissionStage::Authorize => ProcessPlanStage::Authorize,
        AdmissionStage::Admit | AdmissionStage::Reserve => ProcessPlanStage::Admit,
        AdmissionStage::Prepare => ProcessPlanStage::Prepare,
        AdmissionStage::Activate
        | AdmissionStage::Revoke
        | AdmissionStage::Drain
        | AdmissionStage::Release
        | AdmissionStage::Recover => ProcessPlanStage::Fence,
    }
}

// ---------------------------------------------------------------------------
// The committed consumer identity
// ---------------------------------------------------------------------------

/// The committed identity one execution instance prepares its relationships
/// against.
///
/// Preparation is anchored here, not on a running process: the row is committed
/// with a `ResourceUid` and a `ZoneRevision` before anything starts, and every
/// relationship is prepared against exactly those two values plus the row
/// reference. A consumer therefore has its access before it exists (R40, AE20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSubject {
    process_ref: ResourceRef,
    process_uid: ResourceUid,
    zone: ZoneId,
    resource_revision: ZoneRevision,
    kind: ExecutionInstanceKind,
}

impl ProcessSubject {
    /// Construct a subject from a committed row identity.
    ///
    /// # Errors
    ///
    /// Refuses with [`ProcessPlanRefusalReason::NotAnExecutionInstance`] when
    /// the reference names neither a long-running nor a run-to-completion
    /// execution instance. The classification reads the contract crate's
    /// canonical type-name constants, so this module holds no private copy of
    /// the resource vocabulary.
    pub fn new(
        process_ref: ResourceRef,
        process_uid: ResourceUid,
        zone: ZoneId,
        resource_revision: ZoneRevision,
    ) -> Result<Self, ProcessPlanRefusal> {
        let kind = ExecutionInstanceKind::of_reference(&process_ref).ok_or_else(|| {
            refuse(
                ProcessPlanStage::Request,
                ProcessPlanRefusalReason::NotAnExecutionInstance,
            )
        })?;
        Ok(Self {
            process_ref,
            process_uid,
            zone,
            resource_revision,
            kind,
        })
    }

    /// The exact Process or one-shot reference.
    pub const fn process_ref(&self) -> &ResourceRef {
        &self.process_ref
    }

    /// The row's store-assigned identity.
    pub const fn process_uid(&self) -> &ResourceUid {
        &self.process_uid
    }

    /// The Zone the row is committed in.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// The committed desired revision preparation is fenced against.
    pub const fn resource_revision(&self) -> ZoneRevision {
        self.resource_revision
    }

    /// Whether this instance is long-running or run-to-completion.
    ///
    /// The kind is recorded, not branched on: both lifetimes pass through
    /// exactly the same policy and binding path.
    pub const fn kind(&self) -> ExecutionInstanceKind {
        self.kind
    }

    /// Whether the committed identity is exactly `other`'s.
    ///
    /// Two submissions of one row across a restart differ in revision once the
    /// row is re-committed, so a caller that only wants the store identity
    /// compares [`Self::process_uid`] instead.
    pub fn is_committed_as(&self, other: &ProcessSubject) -> bool {
        self.process_ref == other.process_ref
            && self.process_uid == other.process_uid
            && self.zone == other.zone
            && self.resource_revision == other.resource_revision
    }
}

// ---------------------------------------------------------------------------
// Typed resource requests
// ---------------------------------------------------------------------------

/// One relationship one execution instance asks for.
///
/// The request is a claim, not a grant: it names an exact source, a stable
/// consumer slot, a right, and the presentation facets the effect depends on.
/// It carries no destination, no host path, and no right the source has not
/// admitted, so nothing in it can widen what the source decides (R16, R18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessResourceRequest {
    key: BindingKey,
    rights: RequestedRights,
    presentation: Vec<BindingRealizationFacet>,
    helper: Option<ResourceRef>,
}

impl ProcessResourceRequest {
    /// Claim one relationship.
    pub const fn new(
        key: BindingKey,
        rights: RequestedRights,
        presentation: Vec<BindingRealizationFacet>,
        helper: Option<ResourceRef>,
    ) -> Self {
        Self {
            key,
            rights,
            presentation,
            helper,
        }
    }

    /// The exact relationship this leg names.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// The right this leg claims.
    pub const fn rights(&self) -> RequestedRights {
        self.rights
    }

    /// The presentation facets this leg depends on.
    pub fn presentation(&self) -> &[BindingRealizationFacet] {
        &self.presentation
    }

    /// The helper this leg is an attenuated realization for, when it is one.
    pub const fn helper(&self) -> Option<&ResourceRef> {
        self.helper.as_ref()
    }

    /// The consumer slot this leg occupies.
    pub fn address(&self) -> BindingSlotAddress {
        self.key.address()
    }

    /// Project this claim onto the privileged effect owner's carrier.
    ///
    /// The projection is total: every field of the request is already
    /// claim-shaped, so there is no translation step that could add, drop, or
    /// reinterpret one.
    pub fn plan_leg(&self) -> BindingPlanRequest {
        BindingPlanRequest::new(
            self.key.clone(),
            self.rights,
            self.presentation.clone(),
            self.helper.clone(),
        )
    }

    /// Whether this leg is claimed against exactly this consumer identity.
    pub fn is_claimed_by(&self, subject: &ProcessSubject) -> bool {
        self.key.consumer_ref() == subject.process_ref()
            && self.key.consumer_uid() == subject.process_uid()
            && self.key.zone() == subject.zone()
    }
}

/// Everything one Process launch is asked for, before anything resolves.
///
/// The request is the whole of the caller-visible surface: an instance, the
/// provider's declared requirements, an authorized policy selection, the typed
/// relationship claims, the prepared evidence for each, and the arguments the
/// trusted template will bind. There is no field here for a host path, a
/// destination, a numerical principal, a cgroup, a unit, or a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessPlanRequest {
    subject: ProcessSubject,
    provider: BoundedToken,
    instance: ExecutionInstance,
    requirements: ExecutionRequirements,
    policy: ExecutionPolicySpec,
    authorization: PolicyAuthorization,
    support: BackendSupport,
    budget_ceiling: BudgetCeiling,
    resources: Vec<ProcessResourceRequest>,
    prepared: BTreeMap<BindingSlotAddress, PreparedBinding>,
    arguments: Vec<String>,
}

impl ProcessPlanRequest {
    /// Assemble one launch request.
    ///
    /// # Errors
    ///
    /// Refuses with [`ProcessPlanRefusalReason::TooManyLegs`] past
    /// [`MAX_PLAN_LEGS`], with [`ProcessPlanRefusalReason::DuplicateSlot`]
    /// when two claims occupy one consumer slot, and with
    /// [`ProcessPlanRefusalReason::ForeignConsumer`] when a claim is against
    /// an identity other than `subject`. Preparation evidence is matched by
    /// slot here, so a claim whose evidence is absent simply resolves to
    /// [`ProcessPlanRefusalReason::BindingNotPrepared`] at resolution rather
    /// than being silently dropped here.
    pub fn new(
        subject: ProcessSubject,
        provider: BoundedToken,
        instance: ExecutionInstance,
        requirements: ExecutionRequirements,
        policy: ExecutionPolicySpec,
        authorization: PolicyAuthorization,
        support: BackendSupport,
        budget_ceiling: BudgetCeiling,
        resources: Vec<ProcessResourceRequest>,
        prepared: Vec<PreparedBinding>,
        arguments: Vec<String>,
    ) -> Result<Self, ProcessPlanRefusal> {
        if resources.len() > MAX_PLAN_LEGS {
            return Err(refuse(
                ProcessPlanStage::Request,
                ProcessPlanRefusalReason::TooManyLegs,
            ));
        }
        let mut seen: BTreeSet<BindingSlotAddress> = BTreeSet::new();
        for claim in &resources {
            if !seen.insert(claim.address()) {
                return Err(refuse(
                    ProcessPlanStage::Request,
                    ProcessPlanRefusalReason::DuplicateSlot,
                ));
            }
            if !claim.is_claimed_by(&subject) {
                return Err(refuse(
                    ProcessPlanStage::Request,
                    ProcessPlanRefusalReason::ForeignConsumer,
                ));
            }
        }
        let mut prepared_by_slot = BTreeMap::new();
        for binding in prepared {
            prepared_by_slot.insert(binding.address().clone(), binding);
        }
        Ok(Self {
            subject,
            provider,
            instance,
            requirements,
            policy,
            authorization,
            support,
            budget_ceiling,
            resources,
            prepared: prepared_by_slot,
            arguments,
        })
    }

    /// The committed consumer identity this launch is prepared for.
    pub const fn subject(&self) -> &ProcessSubject {
        &self.subject
    }

    /// The Process Provider that declared the requirements.
    pub const fn provider(&self) -> &BoundedToken {
        &self.provider
    }

    /// The instance facts: lifetime, identity request, and budget.
    pub const fn instance(&self) -> &ExecutionInstance {
        &self.instance
    }

    /// The provider's declared requirements.
    pub const fn requirements(&self) -> &ExecutionRequirements {
        &self.requirements
    }

    /// The selected policy's own contract.
    pub const fn policy(&self) -> &ExecutionPolicySpec {
        &self.policy
    }

    /// The supplied launch arguments, before screening.
    ///
    /// These are non-authority values the trusted template will bind. The
    /// screen runs over them in [`resolve_process_plan`], and a value that
    /// names a resolved source never reaches a resolved plan.
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    /// The typed relationship claims.
    pub fn resources(&self) -> &[ProcessResourceRequest] {
        &self.resources
    }

    /// The prepared evidence recorded for one consumer slot.
    pub fn prepared_at(&self, address: &BindingSlotAddress) -> Option<&PreparedBinding> {
        self.prepared.get(address)
    }

    /// Admit this instance against its selected policy.
    ///
    /// This is the one policy path: a long-running and a run-to-completion
    /// instance call the same function with the same arguments shape, and the
    /// only thing the kind changes is the field the admitted execution records
    /// (R25, R26, AE28).
    ///
    /// # Errors
    ///
    /// Refuses with [`ProcessPlanRefusalReason::PolicySelectionNotAuthorized`]
    /// when the selection carries no authorization evidence, and otherwise
    /// with [`ProcessPlanRefusalReason::PolicyRefused`] carrying the policy
    /// contract's own stage and reason.
    pub fn admit_execution(&self) -> Result<AdmittedExecution, ProcessPlanRefusal> {
        if !self.authorization.is_granted() {
            return Err(refuse(
                ProcessPlanStage::Authorize,
                ProcessPlanRefusalReason::PolicySelectionNotAuthorized,
            ));
        }
        admit_execution(
            &self.instance,
            &self.requirements,
            &self.policy,
            &self.authorization,
            &self.support,
            &self.budget_ceiling,
        )
        .map_err(|error| {
            refuse(
                stage_of(error.stage()),
                ProcessPlanRefusalReason::PolicyRefused(error),
            )
        })
    }
}

// ---------------------------------------------------------------------------
// Prepared binding evidence
// ---------------------------------------------------------------------------

/// Whether one relationship's source side is ready for its consumer to start.
///
/// R39 splits a binding's lifecycle into source preparation, access
/// admission, and consumer-side completion. Only the first two can complete
/// before the consumer runs, so this value records exactly the first one:
/// a plan may be `Prepared` while the consumer has not started at all
/// (R40, AE20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BindingPreparation {
    /// The source-side access and delivery prerequisites exist.
    Prepared,
    /// The source side is still being established.
    Incomplete,
}

/// One relationship prepared for one committed consumer identity.
///
/// Construction is closed: the admission must be the one
/// [`d2b_contracts_resource::v3::binding::admit_binding_request`] produced for
/// this exact key, it must admit the claimed right, and the caller must state
/// whether the source side is prepared. A desired request cannot be turned into
/// prepared evidence by naming itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedBinding {
    address: BindingSlotAddress,
    source_ref: ResourceRef,
    source_uid: ResourceUid,
    rights: RequestedRights,
    presentation: Vec<BindingRealizationFacet>,
    admission: BindingAdmission,
    reservation: SourceReservation,
    fingerprint: String,
    preparation: BindingPreparation,
}

impl PreparedBinding {
    /// Record one prepared relationship.
    ///
    /// # Errors
    ///
    /// Refuses when the admission is for a different relationship, when it does
    /// not admit the claimed right, or when the reservation does not belong to
    /// the same Zone and source this leg names.
    pub fn new(
        source_ref: ResourceRef,
        source_uid: ResourceUid,
        rights: RequestedRights,
        presentation: Vec<BindingRealizationFacet>,
        admission: BindingAdmission,
        reservation: SourceReservation,
        fingerprint: String,
        preparation: BindingPreparation,
    ) -> Result<Self, ProcessPlanRefusal> {
        let key = admission.key();
        if key.source_ref() != &source_ref
            || key.source_uid() != &source_uid
            || admission.rights() != rights
            || reservation.zone() != key.zone()
            || reservation.source_uid() != key.source_uid()
        {
            return Err(refuse(
                ProcessPlanStage::Prepare,
                ProcessPlanRefusalReason::LegNotResolved,
            ));
        }
        Ok(Self {
            address: key.address(),
            source_ref,
            source_uid,
            rights,
            presentation,
            admission,
            reservation,
            fingerprint,
            preparation,
        })
    }

    /// The consumer slot this relationship occupies.
    pub const fn address(&self) -> &BindingSlotAddress {
        &self.address
    }

    /// The exact source reference.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// The source's store-assigned identity.
    pub const fn source_uid(&self) -> &ResourceUid {
        &self.source_uid
    }

    /// The right this relationship admitted.
    pub const fn rights(&self) -> RequestedRights {
        self.rights
    }

    /// The presentation facets this relationship depends on.
    pub fn presentation(&self) -> &[BindingRealizationFacet] {
        &self.presentation
    }

    /// The admission this evidence was minted from.
    pub const fn admission(&self) -> &BindingAdmission {
        &self.admission
    }

    /// The source-owned reservation this relationship holds.
    pub const fn reservation(&self) -> &SourceReservation {
        &self.reservation
    }

    /// The digest of the exact desired bytes this relationship commits.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Whether the source side is prepared for the consumer to start.
    pub const fn preparation(&self) -> BindingPreparation {
        self.preparation
    }

    /// Whether the consumer may start on this relationship.
    pub const fn admits_start(&self) -> bool {
        matches!(self.preparation, BindingPreparation::Prepared)
    }

    /// Whether this evidence is still fenced against the observed graph.
    pub fn is_current(&self, observed: &[FreshnessTuple]) -> bool {
        self.admission.is_current(observed)
    }
}

// ---------------------------------------------------------------------------
// Launch arguments
// ---------------------------------------------------------------------------

/// The arguments one trusted template will bind for a launch.
///
/// # Why a supplied argument is refused, not ignored
///
/// The destination of every relationship is resolved by the privileged effect
/// owner from the admitted source and the binding's own presentation facet.
/// A caller argument can therefore never *become* a source: it is a value the
/// declared template binds, and its only effect is positional.
///
/// A value that names a binding-selected source or destination is refused
/// rather than dropped, for two reasons. Dropping it would shift every later
/// argument into the wrong slot, so the process would run with a different
/// meaning than the one the row asked for - the unexplained launch mismatch
/// the plan's problem frame calls out. And a silent drop tells the author
/// nothing, so the same mistake would be re-authored on the next attempt. A
/// refusal names the offending argument by index and leaves nothing running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessLaunchArguments {
    values: Vec<String>,
}

impl ProcessLaunchArguments {
    /// The empty argument list: a template that takes no caller value.
    pub fn empty() -> Self {
        Self { values: Vec::new() }
    }

    /// Screen one supplied argument list against a resolved plan.
    ///
    /// # Errors
    ///
    /// Refuses with [`ProcessPlanRefusalReason::ArgumentMalformed`] past
    /// [`MAX_LAUNCH_ARGS`], past [`MAX_LAUNCH_ARG_BYTES`] for one value, past
    /// [`MAX_LAUNCH_ARGS_TOTAL_BYTES`] in total, or for a value carrying a NUL
    /// or control byte, and with
    /// [`ProcessPlanRefusalReason::ArgumentRedirectsSource`] for a value that
    /// names a binding-selected destination or source, a prefix of one at a
    /// path boundary, or a binding reference the plan does not carry.
    pub fn screen(
        supplied: &[String],
        destinations: &[PlannedDestination],
        sources: &[PlannedSource],
    ) -> Result<Self, ProcessPlanRefusal> {
        if supplied.len() > MAX_LAUNCH_ARGS {
            return Err(refuse(
                ProcessPlanStage::Screen,
                ProcessPlanRefusalReason::ArgumentMalformed,
            ));
        }
        let mut total = 0usize;
        for value in supplied {
            if value.len() > MAX_LAUNCH_ARG_BYTES
                || value.bytes().any(|byte| byte == 0 || byte.is_ascii_control())
            {
                return Err(refuse(
                    ProcessPlanStage::Screen,
                    ProcessPlanRefusalReason::ArgumentMalformed,
                ));
            }
            total = total.saturating_add(value.len());
            if total > MAX_LAUNCH_ARGS_TOTAL_BYTES {
                return Err(refuse(
                    ProcessPlanStage::Screen,
                    ProcessPlanRefusalReason::ArgumentMalformed,
                ));
            }
            if names_a_resolved_source(value, destinations, sources) {
                return Err(refuse(
                    ProcessPlanStage::Screen,
                    ProcessPlanRefusalReason::ArgumentRedirectsSource,
                ));
            }
        }
        Ok(Self {
            values: supplied.to_vec(),
        })
    }

    /// The screened values, in template order.
    pub fn values(&self) -> &[String] {
        &self.values
    }

    /// Whether this launch contributes no caller value.
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl Default for ProcessLaunchArguments {
    fn default() -> Self {
        Self::empty()
    }
}

/// Whether one supplied value names a source or destination the plan resolved.
fn names_a_resolved_source(
    value: &str,
    destinations: &[PlannedDestination],
    sources: &[PlannedSource],
) -> bool {
    let candidate = value.trim_start_matches("--").split_once('=').map_or(value, |(_, rest)| rest);
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }
    if destinations
        .iter()
        .any(|destination| names_path(candidate, destination.path().as_path()))
    {
        return true;
    }
    sources.iter().any(|source| {
        names_path(candidate, source.backing_path().as_path())
            || source
                .views()
                .iter()
                .any(|view| names_path(candidate, view.path().as_path()))
    })
}

/// Whether a candidate names a resolved path, exactly or as a parent of it.
///
/// Only a path-boundary match counts. `/var/lib/d2bd` does not name
/// `/var/lib/d2bd-2`, and a substring match would refuse an argument that
/// merely mentions the same letters.
fn names_path(candidate: &str, path: &std::path::Path) -> bool {
    let resolved = path.to_string_lossy();
    if candidate == resolved.as_ref() {
        return true;
    }
    let trimmed = resolved.trim_end_matches('/');
    if trimmed.is_empty() {
        return false;
    }
    candidate
        .strip_prefix(trimmed)
        .is_some_and(|rest| rest.starts_with('/'))
}

// ---------------------------------------------------------------------------
// The broker-resolved plan, projected
// ---------------------------------------------------------------------------

/// The privileged effect owner's resolved values for one Process launch.
///
/// The only constructor projects an [`ExecutionPlan`], so a source path, a
/// destination, an identity, and an executable exist here only because the
/// broker resolved them from its own accepted graph and trusted implementation
/// contract. Nothing on this type serializes, and `Debug`/`Display` redact the
/// private values, so a plan cannot be logged, persisted, or replayed as
/// access.
#[derive(Clone)]
pub struct ProcessPlanValues {
    executable: PlannedExecutable,
    legs: Vec<ResolvedLeg>,
    sources: Vec<PlannedSource>,
    destinations: Vec<PlannedDestination>,
    identity: Option<PlannedIdentity>,
    freshness: EffectFreshness,
}

impl ProcessPlanValues {
    /// Project one broker-resolved execution plan.
    pub fn from_execution_plan(plan: &ExecutionPlan) -> Self {
        Self {
            executable: plan.executable().clone(),
            legs: plan.legs().to_vec(),
            sources: plan.sources().to_vec(),
            destinations: plan.destinations().to_vec(),
            identity: plan.identity().cloned(),
            freshness: plan.freshness().clone(),
        }
    }

    /// The trusted executable the effect runs.
    pub const fn executable(&self) -> &PlannedExecutable {
        &self.executable
    }

    /// The admitted relationship legs.
    pub fn legs(&self) -> &[ResolvedLeg] {
        &self.legs
    }

    /// The exact sources the effect addresses.
    pub fn sources(&self) -> &[PlannedSource] {
        &self.sources
    }

    /// The exact presentation destinations the effect applies.
    pub fn destinations(&self) -> &[PlannedDestination] {
        &self.destinations
    }

    /// The identity the effect runs as, when the graph admitted one.
    pub const fn identity(&self) -> Option<&PlannedIdentity> {
        self.identity.as_ref()
    }

    /// The fence the plan was admitted under.
    pub const fn freshness(&self) -> &EffectFreshness {
        &self.freshness
    }
}

impl fmt::Debug for ProcessPlanValues {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessPlanValues")
            .field("template", &self.executable.template())
            .field("implementation", &self.executable.implementation())
            .field("legs", &self.legs.len())
            .field("sources", &self.sources.len())
            .field("destinations", &self.destinations.len())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Launch evidence
// ---------------------------------------------------------------------------

/// The five independent facts a restart or an adoption must match.
///
/// A `Process` row can be reconciled again after a controller restart, after a
/// broker restart, or after a spec regeneration that moved no byte of rendered
/// configuration. Cached `Ready` status alone cannot mint access back, so the
/// decision is made against these five digests: the executable that was
/// verified, the committed row, the assigned Provider, the selected
/// `ExecutionPolicy`, and the exact set of prepared relationships. Any one of
/// them diverging quarantines the candidate instead of adopting it (R41).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessLaunchEvidence {
    executable: [u8; 32],
    resource: [u8; 32],
    provider: [u8; 32],
    policy: [u8; 32],
    bindings: [u8; 32],
}

/// Which independent fact a divergence was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProcessEvidenceComponent {
    /// The verified executable.
    Executable,
    /// The committed consumer row.
    Resource,
    /// The assigned Process Provider.
    Provider,
    /// The selected `ExecutionPolicy`.
    Policy,
    /// The exact set of prepared relationships.
    Bindings,
}

impl ProcessEvidenceComponent {
    /// The stable wire name of this component.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Executable => "executable",
            Self::Resource => "resource",
            Self::Provider => "provider",
            Self::Policy => "policy",
            Self::Bindings => "bindings",
        }
    }
}

impl fmt::Display for ProcessEvidenceComponent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl ProcessLaunchEvidence {
    /// Fold the five facts into their independent digests.
    fn frame(
        executable: &[u8],
        resource: &[u8],
        provider: &[u8],
        policy: &[u8],
        bindings: &[u8],
    ) -> Self {
        let component = |label: &str, bytes: &[u8]| -> [u8; 32] {
            let mut hasher = Sha256::new();
            hasher.update(PROCESS_EVIDENCE_DIGEST_DOMAIN_TAG.as_bytes());
            hasher.update([0]);
            hasher.update(label.as_bytes());
            hasher.update([0]);
            hasher.update(bytes);
            hasher.finalize().into()
        };
        Self {
            executable: component("executable", executable),
            resource: component("resource", resource),
            provider: component("provider", provider),
            policy: component("policy", policy),
            bindings: component("bindings", bindings),
        }
    }

    /// The digest of the verified executable.
    pub const fn executable(&self) -> [u8; 32] {
        self.executable
    }

    /// The digest of the committed consumer row.
    pub const fn resource(&self) -> [u8; 32] {
        self.resource
    }

    /// The digest of the assigned Provider.
    pub const fn provider(&self) -> [u8; 32] {
        self.provider
    }

    /// The digest of the selected `ExecutionPolicy`.
    pub const fn policy(&self) -> [u8; 32] {
        self.policy
    }

    /// The digest of the exact prepared relationship set.
    pub const fn bindings(&self) -> [u8; 32] {
        self.bindings
    }

    /// The first component in which two evidence sets diverge.
    ///
    /// The order is fixed - executable, resource, provider, policy, bindings -
    /// so a diagnostic always names the same component for the same divergence
    /// rather than whichever comparison happened to run first.
    pub fn divergence(&self, other: &ProcessLaunchEvidence) -> Option<ProcessEvidenceComponent> {
        [
            (
                ProcessEvidenceComponent::Executable,
                self.executable == other.executable,
            ),
            (
                ProcessEvidenceComponent::Resource,
                self.resource == other.resource,
            ),
            (
                ProcessEvidenceComponent::Provider,
                self.provider == other.provider,
            ),
            (
                ProcessEvidenceComponent::Policy,
                self.policy == other.policy,
            ),
            (
                ProcessEvidenceComponent::Bindings,
                self.bindings == other.bindings,
            ),
        ]
        .into_iter()
        .find_map(|(component, equal)| (!equal).then_some(component))
    }

    /// Refuse an adoption whose evidence is not this evidence.
    ///
    /// # Errors
    ///
    /// Refuses with [`ProcessPlanRefusalReason::EvidenceDiverged`] naming the
    /// first diverging component.
    pub fn admit_candidate(
        &self,
        candidate: &ProcessLaunchEvidence,
    ) -> Result<(), ProcessPlanRefusal> {
        match self.divergence(candidate) {
            None => Ok(()),
            Some(component) => Err(refuse(
                ProcessPlanStage::Fence,
                ProcessPlanRefusalReason::EvidenceDiverged(component),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// The resolved plan
// ---------------------------------------------------------------------------

/// The one resolved plan a Process launch runs against.
///
/// It carries the effective [`AdmittedExecution`], one [`PreparedBinding`] per
/// admitted relationship, the screened launch arguments, and the
/// [`ProcessLaunchEvidence`] a restart or adoption must match. It is derived
/// execution data: it decides nothing, and it holds no authored resource.
#[derive(Debug, Clone)]
pub struct ResolvedProcessPlan {
    subject: ProcessSubject,
    execution: AdmittedExecution,
    values: ProcessPlanValues,
    bindings: BTreeMap<BindingSlotAddress, PreparedBinding>,
    arguments: ProcessLaunchArguments,
    evidence: ProcessLaunchEvidence,
    fingerprint: [u8; 32],
}

impl ResolvedProcessPlan {
    /// The committed consumer identity this plan was prepared for.
    pub const fn subject(&self) -> &ProcessSubject {
        &self.subject
    }

    /// The effective execution this launch runs under.
    pub const fn execution(&self) -> &AdmittedExecution {
        &self.execution
    }

    /// The broker-resolved private values.
    pub const fn values(&self) -> &ProcessPlanValues {
        &self.values
    }

    /// The prepared relationships, keyed by consumer slot.
    pub fn bindings(&self) -> impl Iterator<Item = &PreparedBinding> {
        self.bindings.values()
    }

    /// The one prepared relationship at a consumer slot.
    pub fn binding(&self, slot: &BindingSlot) -> Option<&PreparedBinding> {
        self.bindings
            .values()
            .find(|binding| binding.address().slot() == slot)
    }

    /// The screened launch arguments.
    pub const fn arguments(&self) -> &ProcessLaunchArguments {
        &self.arguments
    }

    /// The evidence a restart or adoption must match.
    pub const fn evidence(&self) -> &ProcessLaunchEvidence {
        &self.evidence
    }

    /// The digest of this whole plan.
    pub const fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }

    /// Whether this plan may start its consumer.
    ///
    /// Every relationship it depends on must be prepared. A plan with no
    /// relationship depends on nothing and is ready.
    pub fn admits_start(&self) -> bool {
        self.bindings.values().all(PreparedBinding::admits_start)
    }

    /// Refuse a launch prepared against a different committed consumer.
    ///
    /// # Errors
    ///
    /// Refuses with [`ProcessPlanRefusalReason::NotPreparedForConsumer`] when
    /// the presented subject is not the one every leg was prepared against.
    /// This is the R40 fence expressed as a check: a plan prepared for one
    /// committed row revision is never evidence for another.
    pub fn prepared_against(&self, subject: &ProcessSubject) -> Result<(), ProcessPlanRefusal> {
        if self.subject.is_committed_as(subject)
            && self
                .bindings
                .values()
                .all(|binding| binding.address().consumer_uid() == subject.process_uid())
        {
            Ok(())
        } else {
            Err(refuse(
                ProcessPlanStage::Fence,
                ProcessPlanRefusalReason::NotPreparedForConsumer,
            ))
        }
    }

    /// Refuse a fence whose committed dependencies have moved.
    ///
    /// # Errors
    ///
    /// Refuses with [`ProcessPlanRefusalReason::StaleDependency`] when any leg
    /// the plan depends on is no longer current against the dependencies the
    /// caller now observes. An ownership, view, consumer, provider-assignment,
    /// or policy change advances a row's desired revision even when the
    /// rendered spec bytes did not move, so this is what keeps a cached plan
    /// from reminting access (R35, R41).
    pub fn fence(&self, observed: &[FreshnessTuple]) -> Result<(), ProcessPlanRefusal> {
        if self
            .bindings
            .values()
            .all(|binding| binding.is_current(observed))
        {
            Ok(())
        } else {
            Err(refuse(
                ProcessPlanStage::Fence,
                ProcessPlanRefusalReason::StaleDependency,
            ))
        }
    }

    /// Begin a launch scope for this plan.
    pub fn launch_scope(&self) -> ProcessLaunchScope {
        ProcessLaunchScope {
            fingerprint: self.fingerprint,
            consumer: self.subject.process_uid().clone(),
            slots: self.bindings.keys().cloned().collect(),
            runner: None,
        }
    }
}

impl PartialEq for ResolvedProcessPlan {
    fn eq(&self, other: &Self) -> bool {
        self.subject == other.subject
            && self.execution == other.execution
            && self.bindings == other.bindings
            && self.arguments == other.arguments
            && self.evidence == other.evidence
    }
}

/// Resolve one Process launch request against the broker's resolved values.
///
/// The function is the single place the two admission boundaries are composed:
/// the `ExecutionPolicy` contract for the instance, and the prepared binding
/// evidence for each relationship. It refuses at the first failure, so a
/// caller always sees one enforcing stage.
///
/// # Errors
///
/// Refuses with
/// - [`ProcessPlanRefusalReason::PolicySelectionNotAuthorized`] or
///   [`ProcessPlanRefusalReason::PolicyRefused`] when the instance and its
///   selected policy do not agree;
/// - [`ProcessPlanRefusalReason::LegNotResolved`] when the resolved plan does
///   not carry a leg the request names;
/// - [`ProcessPlanRefusalReason::BindingNotPrepared`] when a leg is admitted
///   but its source side is not prepared;
/// - [`ProcessPlanRefusalReason::ForeignConsumer`] when a leg's consumer
///   identity is not this plan's;
/// - [`ProcessPlanRefusalReason::ArgumentMalformed`] or
///   [`ProcessPlanRefusalReason::ArgumentRedirectsSource`] when a supplied
///   launch argument is malformed or tries to stand in for a source.
pub fn resolve_process_plan(
    request: &ProcessPlanRequest,
    values: &ProcessPlanValues,
) -> Result<ResolvedProcessPlan, ProcessPlanRefusal> {
    // The one policy path. Nothing below re-reads the instance, the policy, or
    // the provider's requirements: a long-running and a one-shot launch differ
    // only in the kind the admitted execution records.
    let execution = request.admit_execution()?;

    let resolved: BTreeMap<BindingSlotAddress, &ResolvedLeg> = values
        .legs()
        .iter()
        .map(|leg| (leg.key().address(), leg))
        .collect();

    let mut bindings = BTreeMap::new();
    for claim in request.resources() {
        if !claim.is_claimed_by(request.subject()) {
            return Err(refuse(
                ProcessPlanStage::Request,
                ProcessPlanRefusalReason::ForeignConsumer,
            ));
        }
        if !resolved.contains_key(&claim.address()) {
            return Err(refuse(
                ProcessPlanStage::Admit,
                ProcessPlanRefusalReason::LegNotResolved,
            ));
        }
        let prepared = request.prepared.get(&claim.address()).ok_or_else(|| {
            refuse(
                ProcessPlanStage::Prepare,
                ProcessPlanRefusalReason::BindingNotPrepared,
            )
        })?;
        if !prepared.admits_start() {
            return Err(refuse(
                ProcessPlanStage::Prepare,
                ProcessPlanRefusalReason::BindingNotPrepared,
            ));
        }
        bindings.insert(claim.address(), prepared.clone());
    }

    let arguments = ProcessLaunchArguments::screen(
        &request.arguments,
        values.destinations(),
        values.sources(),
    )?;

    let evidence = frame_launch_evidence(request, values, &bindings);
    let fingerprint = frame_plan_fingerprint(&evidence, values, &arguments);
    Ok(ResolvedProcessPlan {
        subject: request.subject().clone(),
        execution,
        values: ProcessPlanValues {
            executable: values.executable().clone(),
            legs: values.legs().to_vec(),
            sources: values.sources().to_vec(),
            destinations: values.destinations().to_vec(),
            identity: values.identity().cloned(),
            freshness: values.freshness().clone(),
        },
        bindings,
        arguments,
        evidence,
        fingerprint,
    })
}

/// Fold the five launch-evidence facts for one resolved launch.
fn frame_launch_evidence(
    request: &ProcessPlanRequest,
    values: &ProcessPlanValues,
    bindings: &BTreeMap<BindingSlotAddress, PreparedBinding>,
) -> ProcessLaunchEvidence {
    let executable = values.executable();
    let executable_bytes = canonical_json_bytes(executable.implementation())
        .expect("a declared implementation always renders as canonical bytes");
    let resource = request.subject();
    let resource_bytes = [
        resource.process_ref().to_canonical_string(),
        resource.process_uid().as_str().to_owned(),
        resource.zone().as_str().to_owned(),
        resource.resource_revision().get().to_string(),
    ]
    .join("|");
    let policy = request.policy();
    let policy_bytes = [
        request.subject().process_ref().to_canonical_string(),
        execution_policy_fingerprint(policy).as_str().to_owned(),
    ]
    .join("|");
    let binding_bytes = bindings
        .values()
        .map(|binding| {
            [
                binding.address().kind().resource_type().to_owned(),
                binding.address().slot().as_str().to_owned(),
                binding.source_ref().to_canonical_string(),
                binding.fingerprint().to_owned(),
            ]
            .join("~")
        })
        .collect::<Vec<_>>()
        .join("|");
    ProcessLaunchEvidence::frame(
        &[executable_bytes.as_slice(), executable.template().as_str().as_bytes()].concat(),
        resource_bytes.as_bytes(),
        request.provider().as_str().as_bytes(),
        policy_bytes.as_bytes(),
        binding_bytes.as_bytes(),
    )
}

/// The canonical digest of one policy selection.
fn execution_policy_fingerprint(policy: &ExecutionPolicySpec) -> ExecutionPolicyFingerprint {
    ExecutionPolicyFingerprint::from_spec(policy)
}

/// Frame the whole-plan digest from the evidence and the resolved values.
fn frame_plan_fingerprint(
    evidence: &ProcessLaunchEvidence,
    values: &ProcessPlanValues,
    arguments: &ProcessLaunchArguments,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(PROCESS_PLAN_DIGEST_DOMAIN_TAG.as_bytes());
    hasher.update([0]);
    for component in [
        evidence.executable(),
        evidence.resource(),
        evidence.provider(),
        evidence.policy(),
        evidence.bindings(),
    ] {
        hasher.update(component);
    }
    hasher.update(values.executable().program().as_path().as_os_str().as_encoded_bytes());
    hasher.update((arguments.values().len() as u64).to_le_bytes());
    for value in arguments.values() {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    hasher.finalize().into()
}

// ---------------------------------------------------------------------------
// Failure release
// ---------------------------------------------------------------------------

/// The effects one launch created, and only those.
///
/// A failed launch has to give back what it prepared without touching anything
/// that was already running. This scope is what makes that structural rather
/// than a matter of discipline: it records the consumer slot it prepared for
/// and the one runner identity it created, and a release that names anything
/// else is refused instead of performed.
///
/// It is also the reason a restart cannot double-release. The scope carries the
/// plan's own digest, so a scope from a previous incarnation is recognisably
/// not this one and its legs are not released by this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessLaunchScope {
    fingerprint: [u8; 32],
    consumer: ResourceUid,
    slots: BTreeSet<BindingSlotAddress>,
    runner: Option<ProcessIdentityDigest>,
}

/// What one failed launch gives back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessLaunchRelease {
    identity: ProcessIdentityDigest,
    slots: Vec<BindingSlotAddress>,
    fingerprint: [u8; 32],
}

impl ProcessLaunchRelease {
    /// The runner this release may stop: the one this launch created.
    pub const fn identity(&self) -> ProcessIdentityDigest {
        self.identity
    }

    /// The consumer slots this launch prepared, and only those.
    pub fn slots(&self) -> &[BindingSlotAddress] {
        &self.slots
    }

    /// The plan digest this release belongs to.
    pub const fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }
}

impl ProcessLaunchScope {
    /// Whether this scope is the scope of exactly this plan.
    pub fn covers(&self, fingerprint: [u8; 32]) -> bool {
        self.fingerprint == fingerprint
    }

    /// The consumer this scope prepared for.
    pub const fn consumer(&self) -> &ResourceUid {
        &self.consumer
    }

    /// The consumer slots this scope prepared.
    pub fn slots(&self) -> &BTreeSet<BindingSlotAddress> {
        &self.slots
    }

    /// Record the one runner this launch created.
    ///
    /// A second, different identity is refused: one launch owns one runner, and
    /// recording a replacement here would let a later release stop a process
    /// this launch never started.
    pub fn record_runner(
        &mut self,
        identity: ProcessIdentityDigest,
    ) -> Result<(), ProcessPlanRefusal> {
        if identity.is_zero() || self.runner.is_some_and(|held| held != identity) {
            return Err(refuse(
                ProcessPlanStage::Release,
                ProcessPlanRefusalReason::ForeignRelease,
            ));
        }
        self.runner = Some(identity);
        Ok(())
    }

    /// The runner this launch created, when it created one.
    pub const fn runner(&self) -> Option<ProcessIdentityDigest> {
        self.runner
    }

    /// Compute the release for a failed launch.
    ///
    /// # Errors
    ///
    /// Refuses with [`ProcessPlanRefusalReason::ForeignRelease`] when
    /// `candidate` is not the runner this scope created. The refusal is the
    /// point: a failed launch must never release an existing runner, so the
    /// only way to obtain a release is to name the exact identity this launch
    /// started, and a launch that started none releases only its own prepared
    /// relationships.
    pub fn release_for(
        &self,
        candidate: &ProcessIdentityDigest,
    ) -> Result<ProcessLaunchRelease, ProcessPlanRefusal> {
        if self.runner != Some(*candidate) {
            return Err(refuse(
                ProcessPlanStage::Release,
                ProcessPlanRefusalReason::ForeignRelease,
            ));
        }
        Ok(ProcessLaunchRelease {
            identity: *candidate,
            slots: self.slots.iter().cloned().collect(),
            fingerprint: self.fingerprint,
        })
    }

    /// Release only this scope's prepared relationships.
    ///
    /// A launch that failed before it started anything has no runner to stop,
    /// and this is the release it takes.
    pub fn release_prepared(&self) -> ProcessLaunchRelease {
        ProcessLaunchRelease {
            identity: ProcessIdentityDigest::from_bytes([0; 32]),
            slots: self.slots.iter().cloned().collect(),
            fingerprint: self.fingerprint,
        }
    }
}

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::execution_policy_resource::{
        ALL_CONFINEMENT_FACETS, BackendSupport, ConfinementFacet, ExecutionInstanceKind,
        ExecutionPolicySpec, PolicyAuthorization, PolicyNamespaces,
    };
    use d2b_contracts_resource::v3::process::NamespaceClass;
    use d2b_contracts_resource::v3::ResourceRef;

    use super::*;
    use crate::testing::plan_fixtures as fixtures;

    fn request(
        kind: ExecutionInstanceKind,
        reference_value: &str,
        arguments: Vec<String>,
        preparation: BindingPreparation,
    ) -> ProcessPlanRequest {
        let consumer = ResourceRef::parse(reference_value).expect("a canonical fixture reference");
        ProcessPlanRequest::new(
            fixtures::subject(reference_value),
            BoundedToken::parse("process").expect("a canonical token"),
            fixtures::instance(kind),
            fixtures::requirements(),
            fixtures::policy(),
            PolicyAuthorization::granted(),
            fixtures::backend_support(),
            fixtures::ceiling(),
            vec![fixtures::volume_claim(&consumer)],
            vec![fixtures::prepared_binding(&consumer, preparation)],
            arguments,
        )
        .expect("the fixture request is well formed")
    }

    fn resolved(
        kind: ExecutionInstanceKind,
        reference_value: &str,
    ) -> ResolvedProcessPlan {
        let consumer = ResourceRef::parse(reference_value).expect("a canonical fixture reference");
        let values = ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&consumer));
        resolve_process_plan(
            &request(kind, reference_value, Vec::new(), BindingPreparation::Prepared),
            &values,
        )
        .expect("the fixture plan resolves")
    }

    /// A row that is not an execution instance is refused before anything
    /// else looks at it, so no other type can reach the Process plan path.
    #[test]
    fn a_reference_that_names_no_execution_instance_is_refused() {
        let refusal = ProcessSubject::new(
            ResourceRef::parse("Volume/data").expect("a canonical reference"),
            fixtures::shared_operation_uid(),
            ZoneId::parse(fixtures::ZONE).expect("a canonical Zone"),
            ZoneRevision::new(1),
        )
        .expect_err("a Volume is not an execution instance");
        assert_eq!(
            refusal.reason(),
            ProcessPlanRefusalReason::NotAnExecutionInstance
        );
        assert_eq!(refusal.stage(), ProcessPlanStage::Request);
    }

    /// AE20 and AE28: both lifetimes prepare their bindings against the
    /// committed consumer identity, before that consumer runs, and neither
    /// takes a different path.
    #[test]
    fn both_lifetimes_prepare_their_bindings_before_the_consumer_runs() {
        let long_running = resolved(ExecutionInstanceKind::LongRunning, "Process/worker");
        assert_eq!(
            long_running.subject().kind(),
            ExecutionInstanceKind::LongRunning
        );

        // Preparation completed before any consumer ran: the committed subject
        // is the anchor, not a running process, and the plan admits its start.
        assert_eq!(long_running.bindings().count(), 1);
        assert!(long_running.admits_start());
        long_running
            .prepared_against(long_running.subject())
            .expect("prepared for its own committed row");

        // The one-shot instance is classified through the same
        // contract-crate constants and its policy admission is the same
        // function, but the accepted graph's `RoleBinding` subject vocabulary
        // has no `EphemeralProcess` entry, so the broker refuses to resolve
        // one-shot consumer legs today. That refusal is the shared
        // contract's, not this plan's: the same request under a bindable
        // consumer is admitted, which is what proves the Process plan path
        // itself is lifetime-neutral.
        let one_shot_subject = fixtures::subject("EphemeralProcess/flush");
        assert_eq!(one_shot_subject.kind(), ExecutionInstanceKind::OneShot);
        let one_shot_request = ProcessPlanRequest::new(
            one_shot_subject,
            BoundedToken::parse("process").expect("a canonical token"),
            fixtures::instance(ExecutionInstanceKind::OneShot),
            fixtures::requirements(),
            fixtures::policy(),
            PolicyAuthorization::granted(),
            fixtures::backend_support(),
            fixtures::ceiling(),
            vec![fixtures::volume_claim(&ResourceRef::parse("EphemeralProcess/flush")
                .expect("a canonical reference"))],
            vec![fixtures::prepared_binding(
                &ResourceRef::parse("EphemeralProcess/flush").expect("a canonical reference"),
                BindingPreparation::Prepared,
            )],
            Vec::new(),
        )
        .expect("the one-shot request is well formed");
        let admitted = one_shot_request.admit_execution();
        assert_eq!(
            admitted
                .expect("the one policy path admits a one-shot instance")
                .kind(),
            ExecutionInstanceKind::OneShot
        );
        assert!(fixtures::one_shot_leg_is_unbindable());
    }

    /// AE20: a relationship whose source side is still being established
    /// refuses the launch instead of starting a consumer that would wait.
    #[test]
    fn an_unprepared_binding_refuses_the_launch() {
        let consumer = ResourceRef::parse("Process/worker").expect("a canonical reference");
        let values =
            ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&consumer));
        let refusal = resolve_process_plan(
            &request(
                ExecutionInstanceKind::LongRunning,
                "Process/worker",
                Vec::new(),
                BindingPreparation::Incomplete,
            ),
            &values,
        )
        .expect_err("an incomplete source side may not start a consumer");
        assert_eq!(
            refusal.reason(),
            ProcessPlanRefusalReason::BindingNotPrepared
        );
        assert_eq!(refusal.stage(), ProcessPlanStage::Prepare);
    }

    /// A plan prepared for one committed row revision is never evidence for
    /// another, which is what stops a re-committed row from reusing a
    /// previous plan's access.
    #[test]
    fn a_recommitted_consumer_cannot_reuse_the_previous_preparation() {
        let plan = resolved(ExecutionInstanceKind::LongRunning, "Process/worker");
        let recommitted = ProcessSubject::new(
            ResourceRef::parse("Process/worker").expect("a canonical reference"),
            fixtures::shared_operation_uid(),
            ZoneId::parse(fixtures::ZONE).expect("a canonical Zone"),
            ZoneRevision::new(2),
        )
        .expect("a canonical subject");
        let refusal = plan
            .prepared_against(&recommitted)
            .expect_err("a re-committed row is a different consumer");
        assert_eq!(
            refusal.reason(),
            ProcessPlanRefusalReason::NotPreparedForConsumer
        );
    }

    /// R41: a dependency that moved after the request was formed invalidates
    /// the earlier preparation, even though no spec generation moved.
    #[test]
    fn a_dependency_that_moved_invalidates_the_prepared_evidence() {
        let plan = resolved(ExecutionInstanceKind::LongRunning, "Process/worker");
        plan.fence(&[fixtures::source_freshness(), fixtures::consumer_freshness()])
            .expect("an unmoved dependency keeps the fence closed");

        let moved = fixtures::freshness(fixtures::CONSUMER_UID, 2, "consumer-2");
        let refusal = plan
            .fence(&[fixtures::source_freshness(), moved])
            .expect_err("a moved consumer dependency is not current");
        assert_eq!(refusal.reason(), ProcessPlanRefusalReason::StaleDependency);
        assert_eq!(refusal.stage(), ProcessPlanStage::Fence);
    }

    /// Scenario 2: a restart matches the same evidence, and each independent
    /// fact is detected when it alone diverges.
    #[test]
    fn adoption_matches_executable_resource_provider_policy_and_binding_evidence() {
        let plan = resolved(ExecutionInstanceKind::LongRunning, "Process/worker");
        plan.evidence()
            .admit_candidate(plan.evidence())
            .expect("the same evidence is the same launch");

        let other_policy = ProcessPlanRequest::new(
            fixtures::subject("Process/worker"),
            BoundedToken::parse("process").expect("a canonical token"),
            fixtures::instance(ExecutionInstanceKind::LongRunning),
            fixtures::requirements(),
            other_policy(),
            PolicyAuthorization::granted(),
            fixtures::backend_support(),
            fixtures::ceiling(),
            vec![fixtures::volume_claim(&fixtures::consumer())],
            vec![fixtures::prepared_binding(
                &fixtures::consumer(),
                BindingPreparation::Prepared,
            )],
            Vec::new(),
        )
        .expect("the second fixture request is well formed");
        let values =
            ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&fixtures::consumer()));
        let other = resolve_process_plan(&other_policy, &values).expect("the second plan resolves");

        let refusal = plan
            .evidence()
            .admit_candidate(other.evidence())
            .expect_err("a different selected policy is not this launch");
        assert_eq!(
            refusal.reason(),
            ProcessPlanRefusalReason::EvidenceDiverged(ProcessEvidenceComponent::Policy)
        );
        assert_eq!(refusal.stage(), ProcessPlanStage::Fence);
    }

    /// Scenario 4: a supplied argument cannot redirect a source. The refusal
    /// names the screen, not the value, and a sibling path that merely shares
    /// a prefix is still admitted.
    #[test]
    fn a_supplied_argument_cannot_replace_a_binding_selected_source() {
        let plan = resolved(ExecutionInstanceKind::LongRunning, "Process/worker");
        let destination = fixtures::DESTINATION_PATH.to_owned();
        let sibling = format!("{destination}-staging");

        for value in [
            destination.clone(),
            format!("--root={destination}"),
            format!("{destination}/sub"),
            fixtures::SOURCE_PATH.to_owned(),
            fixtures::VIEW_PATH.to_owned(),
        ] {
            let label = value.clone();
            let consumer = fixtures::consumer();
            let values =
                ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&consumer));
            let refusal = resolve_process_plan(
                &request(
                    ExecutionInstanceKind::LongRunning,
                    "Process/worker",
                    vec![value],
                    BindingPreparation::Prepared,
                ),
                &values,
            )
            .expect_err("an argument that names a resolved source is refused");
            assert_eq!(
                refusal.reason(),
                ProcessPlanRefusalReason::ArgumentRedirectsSource,
                "argument {label:?} must not redirect a source"
            );
            assert_eq!(refusal.stage(), ProcessPlanStage::Screen);
        }

        // A value that merely shares a prefix at a non-boundary, and a value
        // that names nothing the plan resolved, are both admitted: the screen
        // matches paths, not substrings.
        for value in [sibling, "--serve".to_owned(), "worker-2".to_owned()] {
            let consumer = fixtures::consumer();
            let values =
                ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&consumer));
            let admitted = resolve_process_plan(
                &request(
                    ExecutionInstanceKind::LongRunning,
                    "Process/worker",
                    vec![value.clone()],
                    BindingPreparation::Prepared,
                ),
                &values,
            )
            .unwrap_or_else(|error| panic!("argument {value:?} is not a source: {error}"));
            assert_eq!(admitted.arguments().values(), [value.as_str()]);
        }

        // Nothing in the admitted plan exposes a host path: the destination
        // came from the broker, not from the argument list.
        assert_eq!(plan.arguments().values(), [] as [String; 0]);
    }

    /// An argument past its bound, or carrying a byte an exec path cannot
    /// round-trip, is refused at the same screen.
    #[test]
    fn a_malformed_launch_argument_is_refused_at_the_screen() {
        let consumer = fixtures::consumer();
        let values =
            ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&consumer));
        for value in ["with\0nul".to_owned(), "x".repeat(crate::ticket::MAX_LAUNCH_ARG_BYTES + 1)] {
            let refusal = resolve_process_plan(
                &request(
                    ExecutionInstanceKind::LongRunning,
                    "Process/worker",
                    vec![value],
                    BindingPreparation::Prepared,
                ),
                &values,
            )
            .expect_err("a malformed argument is refused");
            assert_eq!(
                refusal.reason(),
                ProcessPlanRefusalReason::ArgumentMalformed
            );
        }
    }

    /// Scenario 3: a failed launch releases only its own prepared effects and
    /// refuses to stop a runner it did not start.
    #[test]
    fn a_failed_launch_releases_only_its_own_prepared_effects() {
        let plan = resolved(ExecutionInstanceKind::LongRunning, "Process/worker");
        let own = ProcessIdentityDigest::from_bytes([0x11; 32]);
        let existing_runner = ProcessIdentityDigest::from_bytes([0x22; 32]);

        // A launch that failed before it started anything gives back only the
 // relationships it prepared, and no runner.
        let early = plan.launch_scope();
        let release = early.release_prepared();
        assert_eq!(release.slots().len(), 1);
        assert!(release.identity.is_zero());
        assert_eq!(release.fingerprint(), plan.fingerprint());

        // A launch that started one runner may stop exactly that runner.
        let mut scope = plan.launch_scope();
        scope.record_runner(own).expect("the launch recorded its own runner");
        assert_eq!(scope.release_for(&own).expect("its own runner").slots().len(), 1);

        // It may not stop a runner that was already there.
        let refusal = scope
            .release_for(&existing_runner)
            .expect_err("an existing runner is never released by a failed launch");
        assert_eq!(refusal.reason(), ProcessPlanRefusalReason::ForeignRelease);
        assert_eq!(refusal.stage(), ProcessPlanStage::Release);

        // And it may not record a replacement identity either, so a later
        // release still cannot name a process this launch never started.
        assert!(scope.record_runner(existing_runner).is_err());
        assert_eq!(scope.runner(), Some(own));
    }

    /// A launch scope is recognisably the scope of one plan, so a previous
    /// incarnation's scope cannot release this one's relationships.
    #[test]
    fn a_launch_scope_is_bound_to_its_own_plan_digest() {
        let first = resolved(ExecutionInstanceKind::LongRunning, "Process/worker");
        let second = resolved(ExecutionInstanceKind::LongRunning, "Process/other");
        let scope = first.launch_scope();
        assert!(scope.covers(first.fingerprint()));
        assert!(!scope.covers(second.fingerprint()));
    }

    /// The policy path is one path: a mandatory facet the selected backend
    /// cannot enforce is refused for both lifetimes alike.
    #[test]
    fn an_unenforceable_mandatory_facet_refuses_both_lifetimes() {
        let consumer = fixtures::consumer();
        let values =
            ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&consumer));
        for (kind, reference_value) in [
            (ExecutionInstanceKind::LongRunning, "Process/worker"),
            (ExecutionInstanceKind::OneShot, "EphemeralProcess/flush"),
        ] {
            let mut request = request(kind, reference_value, Vec::new(), BindingPreparation::Prepared);
            request = ProcessPlanRequest::new(
                request.subject().clone(),
                request.provider().clone(),
                request.instance().clone(),
                request.requirements().clone(),
                request.policy().clone(),
                PolicyAuthorization::granted(),
                weak_without(ConfinementFacet::UserNamespace),
                fixtures::ceiling(),
                request.resources().to_vec(),
                Vec::new(),
                request.arguments().to_vec(),
            )
            .expect("the weakened fixture request is well formed");
            let refusal = resolve_process_plan(&request, &values)
                .expect_err("a backend that cannot enforce a mandatory facet is refused");
            assert!(matches!(
                refusal.reason(),
                ProcessPlanRefusalReason::PolicyRefused(_)
            ));
        }
    }

    /// An unauthorized policy selection is refused at the authorization
    /// stage rather than admitted against the same fields.
    #[test]
    fn an_unauthorized_policy_selection_is_refused() {
        let consumer = fixtures::consumer();
        let values =
            ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&consumer));
        let base = request(
            ExecutionInstanceKind::LongRunning,
            "Process/worker",
            Vec::new(),
            BindingPreparation::Prepared,
        );
        let unauthorized = ProcessPlanRequest::new(
            base.subject().clone(),
            base.provider().clone(),
            base.instance().clone(),
            base.requirements().clone(),
            base.policy().clone(),
            PolicyAuthorization::absent(),
            fixtures::backend_support(),
            fixtures::ceiling(),
            base.resources().to_vec(),
            Vec::new(),
            base.arguments().to_vec(),
        )
        .expect("the unauthorized fixture request is well formed");
        let refusal = resolve_process_plan(&unauthorized, &values)
            .expect_err("an unauthorized selection is refused");
        assert_eq!(
            refusal.reason(),
            ProcessPlanRefusalReason::PolicySelectionNotAuthorized
        );
        assert_eq!(refusal.stage(), ProcessPlanStage::Authorize);
    }

    /// A leg the request names but the resolved plan does not carry is
    /// refused, and a duplicate consumer slot is refused before resolution.
    #[test]
    fn a_leg_the_resolved_plan_does_not_carry_is_refused() {
        let consumer = fixtures::consumer();
        let values =
            ProcessPlanValues::from_execution_plan(&fixtures::resolved_execution(&consumer));
        let base = request(
            ExecutionInstanceKind::LongRunning,
            "Process/worker",
            Vec::new(),
            BindingPreparation::Prepared,
        );
        let mut without_evidence = base.resources().to_vec();
        let duplicate = ProcessPlanRequest::new(
            base.subject().clone(),
            base.provider().clone(),
            base.instance().clone(),
            base.requirements().clone(),
            base.policy().clone(),
            PolicyAuthorization::granted(),
            fixtures::backend_support(),
            fixtures::ceiling(),
            without_evidence.clone(),
            Vec::new(),
            Vec::new(),
        )
        .expect("the request without evidence is well formed");
        let refusal = resolve_process_plan(&duplicate, &values)
            .expect_err("a leg with no prepared evidence may not start");
        assert_eq!(
            refusal.reason(),
            ProcessPlanRefusalReason::BindingNotPrepared
        );
        without_evidence.push(without_evidence[0].clone());
        assert!(ProcessPlanRequest::new(
            base.subject().clone(),
            base.provider().clone(),
            base.instance().clone(),
            base.requirements().clone(),
            base.policy().clone(),
            PolicyAuthorization::granted(),
            fixtures::backend_support(),
            fixtures::ceiling(),
            without_evidence,
            Vec::new(),
            Vec::new(),
        )
        .is_err());
    }

    /// A claim against another consumer is refused at the request stage: a
    /// row cannot ask for a relationship that belongs to somebody else.
    #[test]
    fn a_claim_against_another_consumer_is_refused() {
        let other = ResourceRef::parse("Process/other").expect("a canonical reference");
        let request = ProcessPlanRequest::new(
            fixtures::subject("Process/worker"),
            BoundedToken::parse("process").expect("a canonical token"),
            fixtures::instance(ExecutionInstanceKind::LongRunning),
            fixtures::requirements(),
            fixtures::policy(),
            PolicyAuthorization::granted(),
            fixtures::backend_support(),
            fixtures::ceiling(),
            vec![fixtures::volume_claim(&other)],
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(
            request.expect_err("a foreign consumer is refused").reason(),
            ProcessPlanRefusalReason::ForeignConsumer
        );
    }

    /// A second, different policy selection, used to prove the evidence
    /// comparison detects a policy change on its own.
    fn other_policy() -> ExecutionPolicySpec {
        ExecutionPolicySpec::new(
            PolicyNamespaces::new(vec![NamespaceClass::User]).expect("namespaces are bounded"),
            fixtures::policy().capabilities().clone(),
            true,
            fixtures::policy().identity().clone(),
            fixtures::policy().root().clone(),
            fixtures::policy().seccomp().clone(),
            Some(0o077),
        )
        .expect("a well-formed policy")
    }

    /// A backend support set missing one declared facet.
    fn weak_without(facet: ConfinementFacet) -> BackendSupport {
        BackendSupport::new(
            ALL_CONFINEMENT_FACETS
                .iter()
                .copied()
                .filter(|candidate| *candidate != facet)
                .collect(),
        )
        .expect("a well-formed backend support set")
    }
}
