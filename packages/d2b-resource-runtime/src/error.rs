//! Runtime error taxonomy shared by actors, drivers, and the store (U4).
//!
//! Two layers:
//!
//! - [`ResourceError`]: the cross-surface taxonomy (spec decode, store,
//!   manager RPC, driver failures, deleting conflict). Store errors wrap
//!   verbatim except [`SpecStoreError::ResourceDeleting`], which is re-mapped
//!   to [`ResourceError::DeletingConflict`] so callers handle the deleting
//!   conflict once, by one variant (R10).
//! - [`DriverFailure`]: the structured failure surface of one driver
//!   operation (issue #508). It carries the stage, the registered
//!   [`FailureKind`], the structural verdict ([`DriverVerdict`]:
//!   `NotYet`/`Refused`/`Error`), and the compared values behind the failure
//!   ([`FailureComparison`], redacted where the value is secret). Drivers map
//!   their typed errors onto it at the erased driver boundary; the actor owns
//!   retry/backoff from the class alone (R13: retry state is runtime-only).
//!
//! The structural outcomes are not error strings: a [`DriverVerdict::NotYet`]
//! is a defer (`defer + requeue`, never terminal), a
//! [`DriverVerdict::Refused`] is a decision against the row (terminal), and a
//! [`DriverVerdict::Error`] is an operational failure with the driver's own
//! retry class. One structured failure produces both operator projections -
//! the log line ([`DriverFailure::log_line`]) and the wire layer
//! ([`DriverFailure::wire_layer`]) - from the same [`FailureReport`], so what
//! a test asserts is what an operator reads.
//!
//! Every failure kind is registered in [`FailureKinds`] with a one-line
//! means/likely-cause note. `docs/reference/resource-runtime-failure-kinds.md`
//! is generated from that registry by [`render_failure_kind_reference`] (a
//! test fails when the committed page drifts), never hand-maintained prose.

pub const MODULE_NAME: &str = "error";

use crate::identity::ResourceKey;
use crate::spec_store::SpecStoreError;

/// Which driver operation failed. Closed: no provider detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverOp {
    Validate,
    Recover,
    Reconcile,
    Delete,
}

impl DriverOp {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Recover => "recover",
            Self::Reconcile => "reconcile",
            Self::Delete => "delete",
        }
    }
}

impl std::fmt::Display for DriverOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Closed retry class (mirrors `HandlerErrorClass` in the old runner).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// Transient: the actor requeues with backoff (runtime-only, R13).
    Retryable,
    /// Terminal: no retry; the actor surfaces the failure and stops.
    Terminal,
}

impl FailureClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Retryable => "retryable",
            Self::Terminal => "terminal",
        }
    }
}

impl std::fmt::Display for FailureClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Structured failure detail (issue #508)
// ---------------------------------------------------------------------------

/// The placeholder a redacted compared value renders as. A
/// [`ComparedValue::Redacted`] captures no bytes at all, so no projection can
/// leak the secret it stood for.
pub const REDACTED: &str = "<redacted>";

/// Bounded note length a failure may carry (provider-supplied text). The wire
/// `status.resource` layer is bounded, so failure detail is truncated at a
/// char boundary before it can render.
pub const MAX_FAILURE_NOTE_BYTES: usize = 512;

/// One side of a compared value pair. `Redacted` records that a value
/// existed without capturing it; use it for secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComparedValue {
    /// Safe to render.
    Value(String),
    /// Secret: only the fact that a value was compared is recorded.
    Redacted,
}

impl ComparedValue {
    /// A value safe to render.
    pub fn value(value: impl Into<String>) -> Self {
        Self::Value(value.into())
    }

    /// A secret value: never captured.
    pub const fn redacted() -> Self {
        Self::Redacted
    }

    /// The renderable form; `<redacted>` for secrets.
    pub fn render(&self) -> &str {
        match self {
            Self::Value(value) => value.as_str(),
            Self::Redacted => REDACTED,
        }
    }
}

impl std::fmt::Display for ComparedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.render())
    }
}

/// One compared value pair behind a failure (issue #508): the field or
/// precondition that was compared, and the values on both sides. Construct
/// with [`FailureComparison::redacted`] where the value is a secret; the
/// secret never enters the failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureComparison {
    field: &'static str,
    expected: ComparedValue,
    observed: ComparedValue,
}

impl FailureComparison {
    /// A comparison of two renderable values.
    pub fn new(
        field: &'static str,
        expected: impl Into<String>,
        observed: impl Into<String>,
    ) -> Self {
        Self {
            field,
            expected: ComparedValue::value(expected),
            observed: ComparedValue::value(observed),
        }
    }

    /// A comparison whose values are secret on both sides.
    pub const fn redacted(field: &'static str) -> Self {
        Self {
            field,
            expected: ComparedValue::Redacted,
            observed: ComparedValue::Redacted,
        }
    }

    /// The compared field or precondition.
    pub const fn field(&self) -> &'static str {
        self.field
    }

    /// The value the pass required.
    pub const fn expected(&self) -> &ComparedValue {
        &self.expected
    }

    /// The value the pass observed.
    pub const fn observed(&self) -> &ComparedValue {
        &self.observed
    }
}

/// Driver-side accumulation of structured failure detail (issue #508). Drivers
/// build it where they know the compared values and attach it through
/// [`DriverFailure::with_detail`]; the erased boundary reports it unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailureDetail {
    stage: Option<&'static str>,
    comparisons: Vec<FailureComparison>,
    note: Option<String>,
}

impl FailureDetail {
    /// No detail: the failure is the kind alone.
    pub const fn new() -> Self {
        Self {
            stage: None,
            comparisons: Vec::new(),
            note: None,
        }
    }

    /// Detail whose stage refines the driver operation.
    pub const fn at(stage: &'static str) -> Self {
        Self {
            stage: Some(stage),
            comparisons: Vec::new(),
            note: None,
        }
    }

    /// Attach one compared value pair.
    #[must_use]
    pub fn comparison(mut self, comparison: FailureComparison) -> Self {
        self.comparisons.push(comparison);
        self
    }

    /// Attach bounded provider-supplied text (already redacted by the
    /// producer). Truncated at [`MAX_FAILURE_NOTE_BYTES`].
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(bound_failure_note(note.into()));
        self
    }

    /// The stage refinement, if any.
    pub const fn stage(&self) -> Option<&'static str> {
        self.stage
    }

    /// The compared value pairs.
    pub fn comparisons(&self) -> &[FailureComparison] {
        &self.comparisons
    }

    /// The bounded provider-supplied text, if any.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Whether this detail carries nothing beyond the kind.
    pub fn is_empty(&self) -> bool {
        self.stage.is_none() && self.comparisons.is_empty() && self.note.is_none()
    }
}

/// The structural outcome of one driver operation (issue #508).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureOutcome {
    /// The pass cannot proceed yet: the world has not reached the state the
    /// operation needs. Defer and requeue; never terminal.
    NotYet,
    /// A decision against this row: retrying cannot change the committed
    /// input (invalid spec, unsupported provider, refused resolution).
    Refused,
    /// An operational failure while the pass was viable, with the driver's
    /// own retry class.
    Error,
}

impl FailureOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotYet => "not-yet",
            Self::Refused => "refused",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for FailureOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The structural verdict of one failed driver operation (issue #508).
///
/// `NotYet` is never terminal and always defers with a requeue; `Refused` is
/// terminal by construction (a decision against the row); `Error` carries the
/// driver's retry class for an operational failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverVerdict {
    /// The operation cannot proceed yet.
    NotYet { because: String },
    /// A decision against this row.
    Refused { why: String },
    /// An operational failure.
    Error { message: String, class: FailureClass },
}

impl DriverVerdict {
    /// The structural outcome tag.
    pub const fn outcome(&self) -> FailureOutcome {
        match self {
            Self::NotYet { .. } => FailureOutcome::NotYet,
            Self::Refused { .. } => FailureOutcome::Refused,
            Self::Error { .. } => FailureOutcome::Error,
        }
    }

    /// The retry class this verdict asserts (`NotYet` is retryable by
    /// construction, `Refused` terminal by construction).
    pub const fn class(&self) -> FailureClass {
        match self {
            Self::NotYet { .. } => FailureClass::Retryable,
            Self::Refused { .. } => FailureClass::Terminal,
            Self::Error { class, .. } => *class,
        }
    }

    /// The human reason: why not yet, why refused, or the error message.
    pub fn reason(&self) -> &str {
        match self {
            Self::NotYet { because } => because.as_str(),
            Self::Refused { why } => why.as_str(),
            Self::Error { message, .. } => message.as_str(),
        }
    }
}

/// One registry entry of the failure-kind registry (issue #508): a stable
/// kebab code, a one-line meaning, and a one-line likely cause. The docs page
/// `docs/reference/resource-runtime-failure-kinds.md` is rendered from
/// [`FailureKinds::ALL`], so the notes are source, never hand-maintained
/// prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureKind {
    code: &'static str,
    means: &'static str,
    likely_cause: &'static str,
}

impl FailureKind {
    /// Define one registry entry.
    pub const fn new(
        code: &'static str,
        means: &'static str,
        likely_cause: &'static str,
    ) -> Self {
        Self {
            code,
            means,
            likely_cause,
        }
    }

    /// The stable lower-kebab code.
    pub const fn code(self) -> &'static str {
        self.code
    }

    /// One-line meaning for an operator.
    pub const fn means(self) -> &'static str {
        self.means
    }

    /// One-line likely cause for an operator.
    pub const fn likely_cause(self) -> &'static str {
        self.likely_cause
    }

    /// Resolve a registered kind by code.
    pub fn from_code(code: &str) -> Option<Self> {
        FailureKinds::ALL
            .iter()
            .copied()
            .find(|kind| kind.code == code)
    }
}

/// The failure-kind registry (issue #508): one const per kind so producers
/// name a kind at compile time, and [`FailureKinds::ALL`] so the docs and the
/// uniqueness test see every entry.
pub struct FailureKinds;

impl FailureKinds {
    // -- Runtime-generic ----------------------------------------------------

    /// A row read could not be answered by its plane.
    pub const ROW_UNAVAILABLE: FailureKind = FailureKind::new(
        "row-unavailable",
        "The manager plane could not answer a row read.",
        "Manager RPC failure, no published plane, or an unusable payload; retry.",
    );
    /// Owned children are still retiring before this row may proceed.
    pub const CHILDREN_DRAINING: FailureKind = FailureKind::new(
        "children-draining",
        "Owned children have not finished their own finalize/delete pass yet.",
        "Child-first teardown; the parent requeues and re-drives the children.",
    );
    /// The realization target session is gone.
    pub const TARGET_UNAVAILABLE: FailureKind = FailureKind::new(
        "target-unavailable",
        "The resource's realization target session is gone.",
        "Target disconnect; the target's own reconnect drives the next attempt.",
    );
    /// Unclassified deferral: the operation is not available yet.
    pub const DRIVER_NOT_YET: FailureKind = FailureKind::new(
        "driver-not-yet",
        "The driver operation cannot proceed yet.",
        "The world has not reached the state the pass needs; defer and requeue.",
    );
    /// Unclassified refusal: the driver decided against the row.
    pub const DRIVER_REFUSED: FailureKind = FailureKind::new(
        "driver-refused",
        "The driver refused this row.",
        "Committed input the pass cannot change; retrying cannot converge.",
    );
    /// Unclassified operational failure.
    pub const DRIVER_ERROR: FailureKind = FailureKind::new(
        "driver-error",
        "An operational driver failure.",
        "A provider effect or store call failed; retry unless terminal evidence exists.",
    );

    // -- Process ------------------------------------------------------------

    /// The durable Process spec did not decode.
    pub const PROCESS_SPEC_INVALID: FailureKind = FailureKind::new(
        "process-spec-invalid",
        "The durable spec did not decode as the closed Process contract.",
        "Stored bytes are not canonical Process/EphemeralProcess JSON.",
    );
    /// The row's launch identity cannot be constructed.
    pub const PROCESS_IDENTITY_INCOMPLETE: FailureKind = FailureKind::new(
        "process-identity-incomplete",
        "The row's launch identity is incomplete or invalid.",
        "Missing or invalid zone, uid, generation, or execution binding inputs.",
    );
    /// The spec selects a Provider this driver does not own.
    pub const PROCESS_PROVIDER_UNSUPPORTED: FailureKind = FailureKind::new(
        "process-provider-unsupported",
        "The spec selects a Provider this driver does not own.",
        "providerRef names a Provider outside the process family this daemon serves.",
    );
    /// The execution target is not drivable in this daemon mode.
    pub const PROCESS_EXECUTION_UNSUPPORTED: FailureKind = FailureKind::new(
        "process-execution-unsupported",
        "The spec's execution target is not drivable in this daemon mode.",
        "The row's executionRef or execution domain is not allowed here.",
    );
    /// The trusted bundle holds no template binding for the ticket.
    pub const PROCESS_TEMPLATE_UNAVAILABLE: FailureKind = FailureKind::new(
        "process-template-unavailable",
        "The trusted bundle holds no template binding for the requested ticket.",
        "The preflight or treated template was never materialized for this row.",
    );
    /// The trusted bundle refused to resolve the launch ticket.
    pub const PROCESS_RESOLUTION_REFUSED: FailureKind = FailureKind::new(
        "process-resolution-refused",
        "The trusted bundle refused to resolve the launch ticket before any launch.",
        "No matching intent, or a wrong execution target, scope, or descriptor posture.",
    );
    /// A Guest-owned process outside the guest VMM chain.
    pub const PROCESS_GUEST_PROCESS_NOT_VMM: FailureKind = FailureKind::new(
        "process-guest-process-not-vmm",
        "The row is a Guest-owned process outside the guest VMM chain.",
        "A projected preflight intent no host-minted ticket can describe.",
    );
    /// The observed process identity is ambiguous.
    pub const PROCESS_IDENTITY_AMBIGUOUS: FailureKind = FailureKind::new(
        "process-identity-ambiguous",
        "The observed process identity is ambiguous; the process is quarantined.",
        "Several candidates matched, or the observation drifted from the ticket (R15).",
    );
    /// A provider process effect failed operatively.
    pub const PROCESS_PROVIDER_EFFECT_FAILED: FailureKind = FailureKind::new(
        "process-provider-effect-failed",
        "A provider process effect failed.",
        "The launch, stop, wait, or finalize call returned an operational error.",
    );
    /// The in-memory restart budget is exhausted.
    pub const PROCESS_START_BUDGET_EXHAUSTED: FailureKind = FailureKind::new(
        "process-start-budget-exhausted",
        "The in-memory restart budget for this row is exhausted.",
        "Repeated restarts inside the window; a spec change or daemon restart resets it.",
    );
    /// Owned process children are still retiring.
    pub const PROCESS_DRAIN_PENDING: FailureKind = FailureKind::new(
        "process-drain-pending",
        "Owned process children are still retiring.",
        "Children must go first; the delete pass requeues and re-drives them.",
    );

    // -- Binding ------------------------------------------------------------

    /// The durable binding spec did not decode.
    pub const BINDING_SPEC_INVALID: FailureKind = FailureKind::new(
        "binding-spec-invalid",
        "The durable spec did not decode as the strict neutral binding contract.",
        "Stored bytes are not canonical VolumeBinding JSON.",
    );
    /// The spec selects a Provider this driver does not own.
    pub const BINDING_PROVIDER_UNSUPPORTED: FailureKind = FailureKind::new(
        "binding-provider-unsupported",
        "The spec selects a Provider this driver does not own.",
        "providerRef is outside the volume-virtiofs binding family.",
    );
    /// The declared parent Volume is owned by another resource.
    pub const BINDING_OWNER_MISMATCH: FailureKind = FailureKind::new(
        "binding-owner-mismatch",
        "The declared parent Volume row is owned by a different resource.",
        "The committed rows disagree on ownership; adopting would silently re-parent.",
    );
    /// The declared parent Volume row is not observable yet.
    pub const BINDING_PARENT_UNAVAILABLE: FailureKind = FailureKind::new(
        "binding-parent-unavailable",
        "The declared parent Volume row is not observable yet.",
        "The parent is not committed yet, or the manager plane could not answer the read.",
    );
    /// The declared parent Volume row is not a usable Volume row.
    pub const BINDING_PARENT_SPEC_INVALID: FailureKind = FailureKind::new(
        "binding-parent-spec-invalid",
        "The declared parent Volume row does not hold a usable Volume row.",
        "The committed parent row's uid or stored spec does not decode as canonical Volume.",
    );
    /// The worker plan could not be derived.
    pub const BINDING_PLAN_DERIVATION_INVALID: FailureKind = FailureKind::new(
        "binding-plan-derivation-invalid",
        "The worker plan could not be derived from the binding and its parent.",
        "View rights or vcpu inputs do not satisfy the plan contract.",
    );
    /// A provider serving effect failed.
    pub const BINDING_SERVING_EFFECT_FAILED: FailureKind = FailureKind::new(
        "binding-serving-effect-failed",
        "A provider serving effect failed.",
        "The bind/unbind call returned an operational error; the pass retries.",
    );
    /// A manager child mutation failed.
    pub const BINDING_CHILD_MUTATION_FAILED: FailureKind = FailureKind::new(
        "binding-child-mutation-failed",
        "A manager child ensure or delete failed.",
        "The manager RPC failed; the manager owns the retry.",
    );

    // -- Volume -------------------------------------------------------------

    /// The durable Volume spec did not decode.
    pub const VOLUME_SPEC_INVALID: FailureKind = FailureKind::new(
        "volume-spec-invalid",
        "The durable spec did not decode as the closed Volume contract.",
        "Stored bytes are not canonical Volume JSON.",
    );
    /// The spec selects a Provider this driver does not own.
    pub const VOLUME_PROVIDER_UNSUPPORTED: FailureKind = FailureKind::new(
        "volume-provider-unsupported",
        "The spec selects a Provider this driver does not own.",
        "providerRef is outside the volume-local family this daemon serves.",
    );
    /// A provider layout effect failed.
    pub const VOLUME_LAYOUT_EFFECT_FAILED: FailureKind = FailureKind::new(
        "volume-layout-effect-failed",
        "A provider layout effect failed.",
        "The layout call returned an operational error; the pass retries.",
    );
    /// The provider layout report is not Ready.
    pub const VOLUME_LAYOUT_NOT_READY: FailureKind = FailureKind::new(
        "volume-layout-not-ready",
        "The provider layout report is not Ready yet.",
        "The layout is Degraded or Pending; requeue instead of respawning the effect.",
    );
    /// A manager child mutation failed.
    pub const VOLUME_CHILD_MUTATION_FAILED: FailureKind = FailureKind::new(
        "volume-child-mutation-failed",
        "A manager child ensure or delete failed.",
        "The manager RPC failed; the manager owns the retry.",
    );
    /// The derived Volume child set is invalid.
    pub const VOLUME_CHILD_DERIVATION_INVALID: FailureKind = FailureKind::new(
        "volume-child-derivation-invalid",
        "The derived child set does not satisfy the Volume contract.",
        "The spec's derived children are malformed or not owned by this row.",
    );

    // -- Endpoint -----------------------------------------------------------

    /// The durable Endpoint spec did not decode.
    pub const ENDPOINT_SPEC_INVALID: FailureKind = FailureKind::new(
        "endpoint-spec-invalid",
        "The durable spec did not decode as the closed Endpoint contract.",
        "Stored bytes are not canonical Endpoint JSON.",
    );
    /// The spec is an Endpoint shape this driver does not realize.
    pub const ENDPOINT_SHAPE_UNSUPPORTED: FailureKind = FailureKind::new(
        "endpoint-shape-unsupported",
        "The spec is an Endpoint shape this driver does not realize.",
        "Class, transport, visibility, or producer ref outside the realized set.",
    );
    /// A provider socket effect failed.
    pub const ENDPOINT_SOCKET_EFFECT_FAILED: FailureKind = FailureKind::new(
        "endpoint-socket-effect-failed",
        "A provider socket effect failed.",
        "The socket realize/remove call returned an operational error.",
    );
    /// Owned endpoint children are still retiring.
    pub const ENDPOINT_DRAIN_PENDING: FailureKind = FailureKind::new(
        "endpoint-drain-pending",
        "Owned endpoint children are still retiring.",
        "Children must go first; the delete pass requeues and re-drives them.",
    );

    // -- Guest --------------------------------------------------------------

    /// The durable Guest spec did not decode.
    pub const GUEST_SPEC_INVALID: FailureKind = FailureKind::new(
        "guest-spec-invalid",
        "The durable spec did not decode or names a Provider outside the row's type.",
        "Stored bytes are not canonical Guest JSON for the selected Provider.",
    );
    /// A manager child mutation failed.
    pub const GUEST_CHILD_MUTATION: FailureKind = FailureKind::new(
        "guest-child-mutation",
        "A manager child ensure or delete failed.",
        "The manager RPC failed; the manager owns the retry.",
    );
    /// The Guest Provider path could not serve the call.
    pub const GUEST_PROVIDER_UNAVAILABLE: FailureKind = FailureKind::new(
        "guest-provider-unavailable",
        "The Guest Provider path is temporarily unavailable.",
        "Provider call, manager plane, or row read failed; retry.",
    );
    /// A Guest teardown stage is still progressing.
    pub const GUEST_FINALIZE_PENDING: FailureKind = FailureKind::new(
        "guest-finalize-pending",
        "A Guest Provider teardown stage is still progressing.",
        "Cleanup has not converged; the owner is re-entered on the next pass.",
    );

    // -- Core ---------------------------------------------------------------

    /// The durable core spec did not decode.
    pub const CORE_SPEC_INVALID: FailureKind = FailureKind::new(
        "core-spec-invalid",
        "The stored spec envelope did not decode, or the row identity is not a contract reference.",
        "Stored bytes are not the JSON spec object the core types store.",
    );
    /// A dependency read failed.
    pub const CORE_DEPENDENCY_READ_FAILED: FailureKind = FailureKind::new(
        "core-dependency-read-failed",
        "A dependency or owned-child read failed.",
        "Manager RPC failure or a missing child row; retry.",
    );
    /// Owned core children are still retiring.
    pub const CORE_DRAIN_PENDING: FailureKind = FailureKind::new(
        "core-drain-pending",
        "Owned core children are still retiring.",
        "Children must go first; the delete pass requeues and re-drives them.",
    );

    // -- System core --------------------------------------------------------

    /// The durable Host/User spec did not decode.
    pub const SYSTEM_CORE_SPEC_INVALID: FailureKind = FailureKind::new(
        "system-core-spec-invalid",
        "The durable spec did not decode as the closed Host/User contract.",
        "Stored bytes are not canonical Host/User JSON for an admitted Provider.",
    );
    /// The Host observation failed.
    pub const SYSTEM_CORE_HOST_OBSERVATION_FAILED: FailureKind = FailureKind::new(
        "system-core-host-observation-failed",
        "The Host observation could not be read.",
        "The provider's host status read failed; retry.",
    );
    /// User discovery failed.
    pub const SYSTEM_CORE_USER_DISCOVERY_FAILED: FailureKind = FailureKind::new(
        "system-core-user-discovery-failed",
        "User or group discovery failed.",
        "The host's user database could not be read; retry.",
    );
    /// Owned system-core children are still retiring.
    pub const SYSTEM_CORE_DRAIN_PENDING: FailureKind = FailureKind::new(
        "system-core-drain-pending",
        "Owned system-core children are still retiring.",
        "Children must go first; the delete pass requeues and re-drives them.",
    );

    /// Every registered kind. The uniqueness test and the generated reference
    /// page read this list; producers name kinds through the consts above.
    pub const ALL: &'static [FailureKind] = &[
        Self::ROW_UNAVAILABLE,
        Self::CHILDREN_DRAINING,
        Self::TARGET_UNAVAILABLE,
        Self::DRIVER_NOT_YET,
        Self::DRIVER_REFUSED,
        Self::DRIVER_ERROR,
        Self::PROCESS_SPEC_INVALID,
        Self::PROCESS_IDENTITY_INCOMPLETE,
        Self::PROCESS_PROVIDER_UNSUPPORTED,
        Self::PROCESS_EXECUTION_UNSUPPORTED,
        Self::PROCESS_TEMPLATE_UNAVAILABLE,
        Self::PROCESS_RESOLUTION_REFUSED,
        Self::PROCESS_GUEST_PROCESS_NOT_VMM,
        Self::PROCESS_IDENTITY_AMBIGUOUS,
        Self::PROCESS_PROVIDER_EFFECT_FAILED,
        Self::PROCESS_START_BUDGET_EXHAUSTED,
        Self::PROCESS_DRAIN_PENDING,
        Self::BINDING_SPEC_INVALID,
        Self::BINDING_PROVIDER_UNSUPPORTED,
        Self::BINDING_OWNER_MISMATCH,
        Self::BINDING_PARENT_UNAVAILABLE,
        Self::BINDING_PARENT_SPEC_INVALID,
        Self::BINDING_PLAN_DERIVATION_INVALID,
        Self::BINDING_SERVING_EFFECT_FAILED,
        Self::BINDING_CHILD_MUTATION_FAILED,
        Self::VOLUME_SPEC_INVALID,
        Self::VOLUME_PROVIDER_UNSUPPORTED,
        Self::VOLUME_LAYOUT_EFFECT_FAILED,
        Self::VOLUME_LAYOUT_NOT_READY,
        Self::VOLUME_CHILD_MUTATION_FAILED,
        Self::VOLUME_CHILD_DERIVATION_INVALID,
        Self::ENDPOINT_SPEC_INVALID,
        Self::ENDPOINT_SHAPE_UNSUPPORTED,
        Self::ENDPOINT_SOCKET_EFFECT_FAILED,
        Self::ENDPOINT_DRAIN_PENDING,
        Self::GUEST_SPEC_INVALID,
        Self::GUEST_CHILD_MUTATION,
        Self::GUEST_PROVIDER_UNAVAILABLE,
        Self::GUEST_FINALIZE_PENDING,
        Self::CORE_SPEC_INVALID,
        Self::CORE_DEPENDENCY_READ_FAILED,
        Self::CORE_DRAIN_PENDING,
        Self::SYSTEM_CORE_SPEC_INVALID,
        Self::SYSTEM_CORE_HOST_OBSERVATION_FAILED,
        Self::SYSTEM_CORE_USER_DISCOVERY_FAILED,
        Self::SYSTEM_CORE_DRAIN_PENDING,
    ];
}

/// The structured failure of one driver operation (issue #508): the operation
/// and refined stage, the registered kind, the structural verdict, the
/// compared values, and bounded provider text.
///
/// One value produces both operator projections - [`Self::log_line`] and
/// [`Self::wire_layer`] - through [`Self::report`], so the log an operator
/// reads and the status a test asserts cannot drift.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.log_line())]
pub struct DriverFailure {
    op: DriverOp,
    stage: &'static str,
    kind: FailureKind,
    verdict: DriverVerdict,
    comparisons: Vec<FailureComparison>,
    note: Option<String>,
}

impl DriverFailure {
    /// A `NotYet` failure: defer and requeue, never terminal.
    pub fn not_yet(op: DriverOp, kind: FailureKind) -> Self {
        Self::new(
            op,
            kind,
            DriverVerdict::NotYet {
                because: kind.means().to_owned(),
            },
        )
    }

    /// A `Refused` failure: a terminal decision against the row.
    pub fn refused(op: DriverOp, kind: FailureKind) -> Self {
        Self::new(
            op,
            kind,
            DriverVerdict::Refused {
                why: kind.means().to_owned(),
            },
        )
    }

    /// An operational `Error` with the driver's retry class.
    pub fn error(op: DriverOp, kind: FailureKind, class: FailureClass) -> Self {
        Self::new(
            op,
            kind,
            DriverVerdict::Error {
                message: kind.means().to_owned(),
                class,
            },
        )
    }

    /// A `NotYet` with a caller-specific reason.
    pub fn not_yet_because(op: DriverOp, kind: FailureKind, because: impl Into<String>) -> Self {
        Self::new(
            op,
            kind,
            DriverVerdict::NotYet {
                because: because.into(),
            },
        )
    }

    /// A `Refused` with a caller-specific reason.
    pub fn refused_because(op: DriverOp, kind: FailureKind, why: impl Into<String>) -> Self {
        Self::new(
            op,
            kind,
            DriverVerdict::Refused { why: why.into() },
        )
    }

    /// An operational `Error` with a caller-specific message.
    pub fn error_because(
        op: DriverOp,
        kind: FailureKind,
        class: FailureClass,
        message: impl Into<String>,
    ) -> Self {
        Self::new(
            op,
            kind,
            DriverVerdict::Error {
                message: message.into(),
                class,
            },
        )
    }

    fn new(op: DriverOp, kind: FailureKind, verdict: DriverVerdict) -> Self {
        Self {
            op,
            stage: op.as_str(),
            kind,
            verdict,
            comparisons: Vec::new(),
            note: None,
        }
    }

    /// A generic retryable classification. Prefer [`Self::not_yet`] with a
    /// registered kind: a named failure diagnoses, this shorthand only defers.
    pub fn retryable(op: DriverOp) -> Self {
        Self::not_yet(op, FailureKinds::DRIVER_NOT_YET)
    }

    /// A generic terminal classification. Prefer [`Self::refused`] with a
    /// registered kind.
    pub fn terminal(op: DriverOp) -> Self {
        Self::refused(op, FailureKinds::DRIVER_REFUSED)
    }

    /// Refine the stage beyond the driver operation (e.g. `recover/adopt`).
    #[must_use]
    pub fn at(mut self, stage: &'static str) -> Self {
        self.stage = stage;
        self
    }

    /// Attach one compared value pair.
    #[must_use]
    pub fn with_comparison(mut self, comparison: FailureComparison) -> Self {
        self.comparisons.push(comparison);
        self
    }

    /// Attach bounded provider-supplied text (already redacted).
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(bound_failure_note(note.into()));
        self
    }

    /// Attach the detail a driver accumulated: stage, comparisons, note.
    #[must_use]
    pub fn with_detail(mut self, detail: FailureDetail) -> Self {
        if let Some(stage) = detail.stage {
            self.stage = stage;
        }
        self.comparisons.extend(detail.comparisons);
        if let Some(note) = detail.note {
            self.note = Some(note);
        }
        self
    }

    /// The failed operation.
    pub const fn op(&self) -> DriverOp {
        self.op
    }

    /// The stage: the operation, refined where the driver named a finer one.
    pub const fn stage(&self) -> &'static str {
        self.stage
    }

    /// The registered failure kind.
    pub const fn kind(&self) -> FailureKind {
        self.kind
    }

    /// The structural verdict.
    pub const fn verdict(&self) -> &DriverVerdict {
        &self.verdict
    }

    /// The structural outcome tag.
    pub const fn outcome(&self) -> FailureOutcome {
        self.verdict.outcome()
    }

    /// The human reason the verdict carries.
    pub fn reason(&self) -> &str {
        self.verdict.reason()
    }

    /// The retry class (R13 owns retry policy from this alone).
    pub const fn class(&self) -> FailureClass {
        self.verdict.class()
    }

    /// Whether the actor must defer with a requeue: every `NotYet` (never
    /// terminal by construction) and every retryable `Error`.
    pub const fn defers(&self) -> bool {
        matches!(self.verdict, DriverVerdict::NotYet { .. })
            || matches!(self.class(), FailureClass::Retryable)
    }

    /// The compared value pairs behind the failure.
    pub fn comparisons(&self) -> &[FailureComparison] {
        &self.comparisons
    }

    /// The bounded provider-supplied text, if any.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The canonical projection both operator surfaces render (issue #508).
    pub fn report(&self) -> FailureReport {
        FailureReport {
            code: self.kind.code(),
            means: self.kind.means(),
            likely_cause: self.kind.likely_cause(),
            operation: self.op,
            stage: self.stage,
            outcome: self.verdict.outcome(),
            reason: self.verdict.reason().to_owned(),
            retryable: self.defers(),
            comparisons: self.comparisons.clone(),
            note: self.note.clone(),
        }
    }

    /// The one-line operator log line: the same structured detail as
    /// [`Self::wire_layer`].
    pub fn log_line(&self) -> String {
        self.report().log_line()
    }

    /// The `status.resource.driverFailure` wire object: the same structured
    /// detail as [`Self::log_line`].
    pub fn wire_layer(&self) -> serde_json::Value {
        self.report().wire_layer()
    }
}

/// The canonical projection of one [`DriverFailure`] (issue #508): the log
/// line and the wire layer are pure functions of this value, so the two
/// operator surfaces cannot diverge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureReport {
    code: &'static str,
    means: &'static str,
    likely_cause: &'static str,
    operation: DriverOp,
    stage: &'static str,
    outcome: FailureOutcome,
    reason: String,
    retryable: bool,
    comparisons: Vec<FailureComparison>,
    note: Option<String>,
}

impl FailureReport {
    /// The stable failure-kind code.
    pub const fn code(&self) -> &'static str {
        self.code
    }

    /// One-line meaning of the kind (registry note).
    pub const fn means(&self) -> &'static str {
        self.means
    }

    /// One-line likely cause of the kind (registry note).
    pub const fn likely_cause(&self) -> &'static str {
        self.likely_cause
    }

    /// The failed operation.
    pub const fn operation(&self) -> DriverOp {
        self.operation
    }

    /// The refined stage.
    pub const fn stage(&self) -> &'static str {
        self.stage
    }

    /// The structural outcome tag.
    pub const fn outcome(&self) -> FailureOutcome {
        self.outcome
    }

    /// The human reason.
    pub fn reason(&self) -> &str {
        self.reason.as_str()
    }

    /// Whether the actor defers and requeues.
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    /// The compared value pairs.
    pub fn comparisons(&self) -> &[FailureComparison] {
        &self.comparisons
    }

    /// The bounded provider-supplied text, if any.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The operator log line.
    pub fn log_line(&self) -> String {
        let mut line = format!(
            "driver {} {} kind={} stage={} retryable={}: {}",
            self.operation,
            self.outcome,
            self.code,
            self.stage,
            self.retryable,
            self.reason,
        );
        for comparison in &self.comparisons {
            line.push_str(&format!(
                "; {}: expected={} observed={}",
                comparison.field(),
                comparison.expected(),
                comparison.observed(),
            ));
        }
        if let Some(note) = &self.note {
            line.push_str(&format!("; note={note}"));
        }
        line
    }

    /// The `status.resource.driverFailure` wire object. `operation` keeps the
    /// established PascalCase spelling and `retryable` stays first-class for
    /// the readers that gate child retries on it.
    pub fn wire_layer(&self) -> serde_json::Value {
        let comparisons: Vec<serde_json::Value> = self
            .comparisons
            .iter()
            .map(|comparison| {
                serde_json::json!({
                    "field": comparison.field(),
                    "expected": comparison.expected().render(),
                    "observed": comparison.observed().render(),
                })
            })
            .collect();
        let mut layer = serde_json::json!({
            "code": self.code,
            "operation": format!("{:?}", self.operation),
            "stage": self.stage,
            "outcome": self.outcome.as_str(),
            "reason": self.reason,
            "retryable": self.retryable,
            "comparisons": comparisons,
        });
        if let Some(note) = &self.note {
            layer["note"] = serde_json::Value::String(note.clone());
        }
        layer
    }
}

/// Truncate provider-supplied text to [`MAX_FAILURE_NOTE_BYTES`] on a char
/// boundary.
fn bound_failure_note(note: String) -> String {
    if note.len() <= MAX_FAILURE_NOTE_BYTES {
        return note;
    }
    let mut end = MAX_FAILURE_NOTE_BYTES;
    while !note.is_char_boundary(end) {
        end -= 1;
    }
    let mut bounded = note;
    bounded.truncate(end);
    bounded.push_str("...");
    bounded
}

/// Render `docs/reference/resource-runtime-failure-kinds.md` from
/// [`FailureKinds::ALL`]. The committed page is generated, never
/// hand-maintained: the drift test renders this and compares.
pub fn render_failure_kind_reference() -> String {
    let mut rendered = String::from(
        "# Driver failure kinds\n\
         \n\
         Generated from `d2b_resource_runtime::error::FailureKinds::ALL` by\n\
         `render_failure_kind_reference`; the `d2b-resource-runtime` unit test\n\
         `failure_kind_reference_doc_matches_the_registry` fails when this page\n\
         drifts. Regenerate with\n\
         `cargo test -p d2b-resource-runtime --lib -- --ignored regenerate_failure_kind_reference`.\n\
         \n\
         A failing driver operation reports one of these codes with its verdict\n\
         (`not-yet` defers and requeues, `refused` is terminal, `error` carries\n\
         the driver's retry class), the stage, and the compared values behind the\n\
         failure.\n\
         \n\
         | code | means | likely cause |\n\
         | --- | --- | --- |\n",
    );
    for kind in FailureKinds::ALL {
        rendered.push_str(&format!(
            "| `{}` | {} | {} |\n",
            kind.code(),
            markdown_cell(kind.means()),
            markdown_cell(kind.likely_cause()),
        ));
    }
    rendered
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', "<br>")
}

/// Cross-surface runtime error taxonomy.
#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    /// The stored spec envelope could not be decoded into the concrete
    /// driver spec type: the wired decode hook failed, or the decoded type
    /// did not match the requested one.
    #[error("spec decode for {key}: {source}")]
    SpecDecode {
        key: ResourceKey,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Spec store failure, wrapped verbatim (the manager translates store
    /// results for drivers; `ResourceDeleting` never reaches this variant).
    #[error(transparent)]
    Store(SpecStoreError),
    /// Mutation admission rejected the mutation at the manager boundary
    /// (U3). This is the admission point that replaces the consciously cut
    /// redb `SealedMutation` seal (KTD2): U8/U10 wire the real subject
    /// resolution (API caller, bundle identity, owning resource).
    #[error("admission denied for {principal} on {zone}/{type_name}/{name}: {reason}")]
    AdmissionDenied {
        principal: String,
        zone: String,
        type_name: String,
        name: String,
        reason: String,
    },
    /// No provider factory is registered for the resource type (or the
    /// driver could not be produced): the spec row stays committed and the
    /// resource recovers on the next manager restart or Ensure.
    #[error("provider for resource type {type_name}: {message}")]
    Provider {
        type_name: String,
        message: String,
    },
     /// A call routed to the manager actor failed: its channel closed, the
     /// request was dropped, or the manager rejected it.
     #[error("manager rpc: {0}")]
     ManagerRpc(String),
    /// A driver reported a structured failure (issue #508).
    #[error(transparent)]
    Driver(#[from] DriverFailure),
    /// Operation against a resource whose durable `deleting` mark is set
    /// (R10). Mapped from `SpecStoreError::ResourceDeleting`.
    #[error("resource {zone}/{type_name}/{name} is marked deleting")]
    DeletingConflict {
        zone: String,
        type_name: String,
        name: String,
    },
    /// An owned child has not finished its own finalize/delete pass yet
    /// (F3 child-first teardown). Retryable by construction: the caller's
    /// next pass re-drives the children and observes their retirement.
    #[error("owned children of {zone}/{type_name}/{name} are still draining")]
    ChildrenDraining {
        zone: String,
        type_name: String,
        name: String,
    },
}

impl From<crate::provider::ProviderDirectoryError> for ResourceError {
    fn from(error: crate::provider::ProviderDirectoryError) -> Self {
        match error {
            crate::provider::ProviderDirectoryError::UnknownType(type_name) => {
                Self::Provider { type_name: type_name.to_string(), message: "no provider factory registered".to_string() }
            }
            crate::provider::ProviderDirectoryError::DuplicateType(type_name) => {
                Self::Provider { type_name: type_name.to_string(), message: "duplicate provider registration".to_string() }
            }
        }
    }
}

impl From<SpecStoreError> for ResourceError {
    fn from(error: SpecStoreError) -> Self {
        match error {
            SpecStoreError::ResourceDeleting { zone, type_name, name } => {
                Self::DeletingConflict { zone, type_name, name }
            }
            other => Self::Store(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleting_store_error_maps_to_deleting_conflict() {
        let error: ResourceError = SpecStoreError::ResourceDeleting {
            zone: "z".into(),
            type_name: "Volume".into(),
            name: "data".into(),
        }
        .into();
        match error {
            ResourceError::DeletingConflict { zone, type_name, name } => {
                assert_eq!(zone, "z");
                assert_eq!(type_name, "Volume");
                assert_eq!(name, "data");
            }
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[test]
    fn other_store_errors_wrap_as_store() {
        let error: ResourceError = SpecStoreError::NotFound {
            zone: "z".into(),
            type_name: "Volume".into(),
            name: "data".into(),
        }
        .into();
        assert!(matches!(error, ResourceError::Store(_)));
    }

    #[test]
    fn retryable_shorthand_defers_and_terminal_shorthand_refuses() {
        let retryable = DriverFailure::retryable(DriverOp::Reconcile);
        assert_eq!(retryable.class(), FailureClass::Retryable);
        assert_eq!(retryable.op(), DriverOp::Reconcile);
        assert_eq!(retryable.outcome(), FailureOutcome::NotYet);
        assert!(retryable.defers());
        let terminal = DriverFailure::terminal(DriverOp::Delete);
        assert_eq!(terminal.class(), FailureClass::Terminal);
        assert_eq!(terminal.op(), DriverOp::Delete);
        assert_eq!(terminal.outcome(), FailureOutcome::Refused);
        assert!(!terminal.defers());
        // Through the taxonomy the structured failure is preserved verbatim.
        let error: ResourceError = terminal.clone().into();
        assert!(matches!(
            error,
            ResourceError::Driver(f)
                if f.class() == FailureClass::Terminal && f.kind().code() == "driver-refused"
        ));
    }

    #[test]
    fn not_yet_is_never_terminal_and_always_defers() {
        for op in [DriverOp::Validate, DriverOp::Recover, DriverOp::Reconcile, DriverOp::Delete] {
            let failure = DriverFailure::not_yet(op, FailureKinds::PROCESS_DRAIN_PENDING);
            assert_eq!(failure.class(), FailureClass::Retryable, "{op}");
            assert_eq!(failure.outcome(), FailureOutcome::NotYet, "{op}");
            assert!(failure.defers(), "{op} must defer with a requeue");
        }
        let refused = DriverFailure::refused(DriverOp::Reconcile, FailureKinds::PROCESS_SPEC_INVALID);
        assert_eq!(refused.class(), FailureClass::Terminal);
        assert!(!refused.defers());
        let transient = DriverFailure::error(
            DriverOp::Reconcile,
            FailureKinds::PROCESS_PROVIDER_EFFECT_FAILED,
            FailureClass::Retryable,
        );
        assert!(transient.defers(), "a retryable error defers too");
        let terminal = DriverFailure::error(
            DriverOp::Validate,
            FailureKinds::PROCESS_SPEC_INVALID,
            FailureClass::Terminal,
        );
        assert!(!terminal.defers());
    }

    #[test]
    fn log_line_and_wire_layer_project_the_same_structured_detail() {
        let failure = DriverFailure::refused(DriverOp::Validate, FailureKinds::ENDPOINT_SHAPE_UNSUPPORTED)
            .at("validate/plan")
            .with_comparison(FailureComparison::new(
                "endpoint.realization",
                "virtiofsd-unix",
                "unrealized",
            ))
            .with_note("provider refused the endpoint shape");
        let report = failure.report();
        let line = failure.log_line();
        let wire = failure.wire_layer();

        // Every structured field is readable in both projections.
        for expected in [
            report.code(),
            report.operation().as_str(),
            report.stage(),
            report.outcome().as_str(),
            report.reason(),
        ] {
            assert!(line.contains(expected), "log line misses {expected:?}: {line}");
        }
        assert!(line.contains("endpoint.realization"));
        assert!(line.contains("virtiofsd-unix"));
        assert!(line.contains("unrealized"));
        assert!(line.contains("provider refused the endpoint shape"));

        assert_eq!(wire["code"], report.code());
        assert_eq!(wire["operation"], format!("{:?}", report.operation()));
        assert_eq!(wire["stage"], report.stage());
        assert_eq!(wire["outcome"], report.outcome().as_str());
        assert_eq!(wire["reason"], report.reason());
        assert_eq!(wire["retryable"], report.retryable());
        assert_eq!(wire["comparisons"][0]["field"], "endpoint.realization");
        assert_eq!(wire["comparisons"][0]["expected"], "virtiofsd-unix");
        assert_eq!(wire["comparisons"][0]["observed"], "unrealized");
        assert_eq!(wire["note"], "provider refused the endpoint shape");

        // The Display is the operator log line, not a second spelling.
        assert_eq!(failure.to_string(), line);
    }

    #[test]
    fn redacted_comparisons_capture_no_secret_bytes() {
        let failure = DriverFailure::refused(DriverOp::Recover, FailureKinds::PROCESS_RESOLUTION_REFUSED)
            .with_comparison(FailureComparison::redacted("launch.ticket"));
        let line = failure.log_line();
        let wire = failure.wire_layer();
        assert!(line.contains("launch.ticket: expected=<redacted> observed=<redacted>"));
        assert_eq!(wire["comparisons"][0]["expected"], REDACTED);
        assert_eq!(wire["comparisons"][0]["observed"], REDACTED);
    }

    #[test]
    fn failure_notes_are_bounded_on_a_char_boundary() {
        let long: String = "é".repeat(MAX_FAILURE_NOTE_BYTES);
        let failure = DriverFailure::refused(DriverOp::Reconcile, FailureKinds::PROCESS_SPEC_INVALID)
            .with_note(long);
        let note = failure.note().expect("note");
        assert!(note.len() <= MAX_FAILURE_NOTE_BYTES + 3);
        assert!(note.ends_with("..."));
        assert!(note.is_char_boundary(note.len()));
    }

    #[test]
    fn every_registered_kind_is_unique_kebab_with_both_notes() {
        let mut codes: Vec<&str> = FailureKinds::ALL.iter().map(|kind| kind.code()).collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total, "duplicate failure-kind code in the registry");
        for kind in FailureKinds::ALL {
            let code = kind.code();
            assert!(!code.is_empty(), "empty code");
            assert!(
                code.split('-').all(|part| {
                    !part.is_empty()
                        && part.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
                }),
                "code {code:?} is not lower-kebab"
            );
            assert!(!kind.means().is_empty(), "{code} has no means note");
            assert!(!kind.likely_cause().is_empty(), "{code} has no likely-cause note");
            assert_eq!(
                FailureKind::from_code(code).map(FailureKind::code),
                Some(code),
                "{code} must resolve from the registry"
            );
        }
        assert_eq!(FailureKind::from_code("not-registered"), None);
    }

    /// The committed reference page is a rendering of the registry, not
    /// hand-maintained prose. Regenerate with
    /// `cargo test -p d2b-resource-runtime --lib -- --ignored regenerate_failure_kind_reference`.
    #[test]
    fn failure_kind_reference_doc_matches_the_registry() {
        let path = reference_doc_path();
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert_eq!(
            committed,
            render_failure_kind_reference(),
            "{} is stale: regenerate it from FailureKinds::ALL",
            path.display()
        );
    }

    #[test]
    #[ignore = "regenerates the committed reference doc from the registry"]
    fn regenerate_failure_kind_reference() {
        let path = reference_doc_path();
        std::fs::write(&path, render_failure_kind_reference())
            .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
    }

    /// Locate the committed reference page at runtime: Bazel rejects compiled
    /// output that embeds the build-time manifest path, and the test runs from
    /// the workspace root there while `cargo test` starts in the crate
    /// directory, so walk up from both candidate bases.
    fn reference_doc_path() -> std::path::PathBuf {
        const RELATIVE: &str = "docs/reference/resource-runtime-failure-kinds.md";
        let mut bases = Vec::new();
        if let Some(manifest) = std::env::var_os("CARGO_MANIFEST_DIR") {
            bases.push(std::path::PathBuf::from(manifest));
        }
        if let Ok(current_dir) = std::env::current_dir() {
            bases.push(current_dir);
        }
        for mut base in bases {
            loop {
                let candidate = base.join(RELATIVE);
                if candidate.is_file() {
                    return candidate;
                }
                if !base.pop() {
                    break;
                }
            }
        }
        panic!("{RELATIVE} is not discoverable from CARGO_MANIFEST_DIR or the working directory");
    }
}
