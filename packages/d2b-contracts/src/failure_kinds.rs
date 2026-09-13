//! Driver failure-kind registry (issue #508): the stable lower-kebab codes a
//! driver failure can carry, each with a one-line meaning and a one-line
//! likely cause for operators. It lives in `d2b-contracts` so consumers such
//! as the `d2b` CLI can read a code's operator-facing note without depending
//! on `d2b-resource-runtime`; that crate re-exports these items from its
//! `error` module. `docs/reference/resource-runtime-failure-kinds.md` is
//! rendered from [`FailureKinds::ALL`] by
//! [`render_failure_kind_reference`].

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
